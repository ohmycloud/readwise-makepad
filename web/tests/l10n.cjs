// Fluent SSR, explicit UI projections, language drafts and reader continuity.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawn, execFileSync } = require('node:child_process');
const { chromium } = require(process.env.READWISE_PLAYWRIGHT_MODULE || 'playwright-core');
const root = path.resolve(__dirname, '../..');
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'readwise-l10n-'));
const fixture = path.join(temporary, '界面与正文.epub');
execFileSync('python3', [path.join(__dirname, 'fixture.py'), fixture]);
execFileSync('python3', ['-c', `import sys,zipfile
p=sys.argv[1]
with zipfile.ZipFile(p) as z: files={n:z.read(n) for n in z.namelist()}
for n in ['OEBPS/one.xhtml','OEBPS/two.xhtml']:
 files[n]=files[n].decode().replace('中文与 emoji','书架 词汇 阅读设置 中文与 emoji').encode()
with zipfile.ZipFile(p,'w',compression=zipfile.ZIP_DEFLATED) as z:
 for n,b in files.items(): z.writestr(n,b)
`, fixture]);
let server, browser, log = '';
const errors = [];
async function start() {
  return new Promise((resolve, reject) => {
    server = spawn(process.env.READWISE_WEB_BINARY || path.join(root, 'target/debug/readwise-web'), [], {
      cwd: root, env: { ...process.env, READWISE_WEB_BIND: '127.0.0.1:0', READWISE_WEB_DATA_DIR: path.join(temporary, 'data') },
    });
    const timer = setTimeout(() => reject(new Error('startup timeout')), 25000);
    const receive = chunk => { log += chunk; const match = log.match(/ReadWise Web: (http:\/\/[^\s]+)\n/); if (match) { clearTimeout(timer); resolve(match[1]); } };
    server.stdout.on('data', receive); server.stderr.on('data', receive); server.on('error', reject);
  });
}
async function stop() {
  if (!server || server.exitCode !== null) return;
  const child = server;
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => { child.kill('SIGKILL'); reject(new Error('shutdown timeout')); }, 10000);
    child.once('exit', () => { clearTimeout(timer); resolve(); }); child.kill('SIGINT');
  });
}
async function run() {
  const base = await start();
  const executablePath = process.env.READWISE_CHROMIUM || (process.platform === 'darwin' ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : undefined);
  browser = await chromium.launch({ headless: true, ...(executablePath ? { executablePath } : {}) });
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, locale: 'zh-CN' });
  const page = await context.newPage();
  page.on('pageerror', error => errors.push(error.message));
  const post = async (url, data) => { const r = await page.request.post(base + url, { headers: { 'x-readwise-request': '1' }, data }); assert.equal(r.status(), 200, await r.text()); return r.json(); };
  const ready = async target => { await target.locator('body[data-browser-ready="true"]').waitFor(); await target.evaluate(() => document.fonts.ready); await target.waitForTimeout(150); };
  await post('/api/preferences', { language: 'en', turn_mode: 'slide', columns: 'two', english_level: 'B1' });
  const noJS = await browser.newContext({ javaScriptEnabled: false, locale: 'zh-CN' });
  const ssr = await noJS.newPage();
  await ssr.goto(`${base}/settings`);
  assert.equal(await ssr.locator('html').getAttribute('lang'), 'en');
  assert.equal(await ssr.locator('html').getAttribute('dir'), 'ltr');
  await ssr.getByRole('heading', { name: 'Reading Settings', exact: true }).waitFor();
  assert.equal(await ssr.locator('[name="api_key"]').getAttribute('placeholder'), 'Leave blank to keep the saved key');
  assert.equal(await ssr.locator('[data-close-picker]').getAttribute('aria-label'), 'Collapse Colour Picker');
  assert.equal(await ssr.title(), 'Me & Settings · ReadWise');
  assert.equal(await ssr.locator('.scene-options [data-i18n]').first().evaluate(node => getComputedStyle(node).borderTopWidth), '0px');
  assert.deepEqual(await ssr.evaluate(() => {
    const catalog = JSON.parse(document.body.dataset.uiMessages).en;
    return [...document.querySelectorAll('[data-i18n]')].map(node => node.dataset.i18n).filter(key => !Object.hasOwn(catalog, key));
  }), [], 'every explicitly marked settings message must have a Fluent translation');
  await ssr.goto(base);
  await ssr.getByRole('heading', { name: 'Library', exact: true }).waitFor();
  assert.match(await ssr.locator('#shelf-count').textContent(), /books\. Take your time\./);
  console.log('PASS: English headings/title/ARIA/placeholders/counts render without JavaScript; saved locale takes priority over browser language');

  await page.goto(base); await ready(page);
  await page.locator('#book-upload').setInputFiles(fixture);
  const imported = page.locator('#import-results [data-state="success"] a');
  await imported.waitFor({ timeout: 30000 });
  const readUrl = await imported.getAttribute('href');
  const bookId = readUrl.split('/')[2];
  await post(`/api/books/${bookId}`, { action: 'rename', title: '书架' });
  await post(`/api/books/${bookId}`, { action: 'tags', tags: ['欲读', '阅读设置'] });
  await page.reload(); await ready(page);
  assert.equal(await page.locator('.book-card h2').textContent(), '书架');
  assert.equal(await page.locator('.book-tags').textContent(), '欲读 · 阅读设置');
  await page.goto(base + readUrl); await ready(page);
  assert.equal(await page.title(), '书架 · ReadWise');
  const bodyPreferences = await page.locator('body').getAttribute('data-preferences');
  assert.equal(await page.locator('#reading-content').getAttribute('data-preferences'), bodyPreferences);
  assert.equal(await page.locator('[data-appearance]').getAttribute('data-initial'), bodyPreferences);
  const ruby = page.locator('#reading-content ruby[data-lemma="solitude"]').first();
  await ruby.click(); await page.locator('#word-card[open]').waitFor();
  await page.locator('#mark-known').click();
  await page.waitForFunction(() => document.querySelector('ruby[data-lemma="solitude"]').dataset.known === 'true');
  assert.equal(await page.locator('#word-card').evaluate(node => node.open), false);
  const snapshot = () => page.locator('#reading-content').evaluate(article => ({
    blocks: [...article.querySelectorAll('.reader-block')].map(node => {
      const r = node.getBoundingClientRect(); return [node.innerHTML, r.x, r.y, r.width, r.height];
    }), page: article.dataset.page, pages: article.dataset.pages,
  }));
  const before = await snapshot();
  await page.evaluate(() => { window.__l10nNodes = [...document.querySelectorAll('.reader-block')]; });
  const editor = await context.newPage(); editor.on('pageerror', error => errors.push(error.message));
  await editor.goto(`${base}/settings`); await ready(editor);
  await editor.locator('[name="language"][value="zh-CN"]').check();
  assert.equal(await editor.locator('html').getAttribute('lang'), 'en');
  await editor.reload(); await ready(editor);
  assert.equal(await editor.locator('[name="language"][value="en"]').isChecked(), true);
  await editor.locator('[name="language"][value="zh-CN"]').check();
  await editor.locator('[data-save-appearance]').click();
  await page.waitForFunction(() => document.documentElement.lang === 'zh-CN');
  await page.waitForTimeout(200);
  assert.deepEqual(await snapshot(), before);
  assert.equal(await page.evaluate(() => window.__l10nNodes.every((node, i) => node === document.querySelectorAll('.reader-block')[i])), true);
  assert.ok((await page.locator('.reader-block').allTextContents()).some(text => text.includes('书架 词汇 阅读设置')));
  assert.equal(await page.locator('#word-card').evaluate(node => node.open), false);
  console.log('PASS: language draft/discard/apply and cross-tab update preserve reader nodes/source/geometry/page/ruby state; user book title and matching source words are never translated');
  await ssr.goto(`${base}/settings`);
  await ssr.getByRole('heading', { name: '阅读设置', exact: true }).waitFor();
  assert.equal(await ssr.title(), '我与设置 · ReadWise');
  assert.deepEqual(errors, []);
  await noJS.close(); await context.close(); await browser.close(); browser = null;
}
run().catch(error => { console.error(error); process.exitCode = 1; }).finally(async () => {
  if (browser) await browser.close(); await stop();
  fs.mkdirSync(path.join(root, 'target/web-qa/l10n'), { recursive: true });
  fs.writeFileSync(path.join(root, 'target/web-qa/l10n/server.log'), log);
  fs.rmSync(temporary, { recursive: true, force: true });
});
