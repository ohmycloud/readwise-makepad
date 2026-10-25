// Isolated Chromium regression for desktop/native-parity shelf management.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawn, execFileSync } = require('node:child_process');
const { chromium } = require(process.env.READWISE_PLAYWRIGHT_MODULE || 'playwright-core');
const root = path.resolve(__dirname, '../..');
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'readwise-shelf-'));
const artifacts = path.join(root, 'target/web-qa/shelf');
fs.mkdirSync(artifacts, { recursive: true });
const fixture = path.join(temporary, '原始书籍.epub');
execFileSync('python3', [path.join(__dirname, 'fixture.py'), fixture]);
let server, browser, log = '';
const errors = [];
async function start() {
  return new Promise((resolve, reject) => {
    server = spawn(process.env.READWISE_WEB_BINARY || path.join(root, 'target/debug/readwise-web'), [], { cwd: root, env: { ...process.env, READWISE_WEB_BIND: '127.0.0.1:0', READWISE_WEB_DATA_DIR: path.join(temporary, 'data') } });
    const timer = setTimeout(() => reject(new Error('startup timeout')), 25000);
    let startup = '';
    const receive = chunk => { log += chunk; startup += chunk; const match = startup.match(/ReadWise Web: (http:\/\/[^\s]+)\n/); if (match) { clearTimeout(timer); resolve(match[1]); } };
    server.stdout.on('data', receive); server.stderr.on('data', receive);
    server.on('error', reject);
  });
}
async function stop() {
  if (!server || server.exitCode !== null) return;
  await new Promise(resolve => { server.once('exit', resolve); server.kill('SIGINT'); });
}
async function run() {
  let base = await start();
  const executablePath = process.env.READWISE_CHROMIUM || (process.platform === 'darwin' ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : undefined);
  browser = await chromium.launch({ headless: true, ...(executablePath ? { executablePath } : {}) });
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, permissions: ['clipboard-read', 'clipboard-write'] });
  const page = await context.newPage();
  page.on('pageerror', error => errors.push(error.message));
  const ready = async () => { await page.locator('body[data-browser-ready="true"]').waitFor(); await page.waitForTimeout(250); };
  const more = () => page.locator('.book-more').first();
  const menu = page.locator('#book-menu');
  const open = async action => { await more().click(); await menu.locator(`[data-book-action="${action}"]`).click(); };
  await page.goto(base); await ready();
  await page.locator('#book-upload').setInputFiles(fixture);
  await page.locator('#import-results [data-state="success"] a').waitFor({ timeout: 30000 });
  const href = await page.locator('#import-results a').getAttribute('href');
  const id = href.split('/').at(-1);
  const post = async (route, data) => { const r = await page.request.post(`${base}${route}`, { headers: { 'x-readwise-request': '1' }, data }); assert.equal(r.status(), 200, await r.text()); return r.json(); };
  await post('/api/progress', { book_id: id, location: { chapter: 0, block: 1, offset: 0 } });
  await page.goto(base); await ready();
  assert.match(await page.locator('.book-status').textContent(), /在读 · \d+%/);
  const originalTitle = await page.locator('.book-card h2').textContent();
  await page.screenshot({ path: path.join(artifacts, 'desktop-grid.png') });
  await open('share');
  assert.match(await page.evaluate(() => navigator.clipboard.readText()), new RegExp(originalTitle));
  await open('rename');
  await page.locator('#book-editor-input').fill('取消不保存');
  await page.locator('#book-editor [data-close]').click();
  assert.equal(await page.locator('.book-card h2').textContent(), originalTitle);
  await open('rename');
  await page.locator('#book-editor-input').fill('新书名 <测试>');
  await page.route(`**/api/books/${id}`, route => route.fulfill({ status: 500, body: '模拟保存失败' }));
  await page.locator('#book-editor [type="submit"]').click();
  await page.locator('#book-editor .shelf-error').filter({ hasText: '模拟保存失败' }).waitFor();
  assert.equal(await page.locator('#book-editor-input').inputValue(), '新书名 <测试>');
  await page.unroute(`**/api/books/${id}`);
  await page.locator('#book-editor [type="submit"]').click();
  await page.waitForFunction(() => !document.querySelector('#book-editor').open);
  assert.equal(await page.locator('.book-card h2').textContent(), '新书名 <测试>');
  await open('want_to_read');
  await page.waitForFunction(() => document.querySelector('.book-card').dataset.wantToRead === 'true');
  await open('tags');
  await page.locator('#book-editor-input').fill('技术， Rust;rust');
  await page.locator('#book-editor [type="submit"]').click();
  await page.waitForFunction(() => document.querySelector('.book-card').dataset.tags === '["技术","Rust"]');
  await open('complete');
  await page.waitForFunction(() => document.querySelector('.book-status').textContent === '读完 · 100%');
  await page.locator('[data-shelf-view="list"]').click();
  await page.reload(); await ready();
  assert.equal(await page.locator('.shelf-page').getAttribute('data-view'), 'list');
  assert.equal(await page.locator('.book-card h2').textContent(), '新书名 <测试>');
  await page.screenshot({ path: path.join(artifacts, 'desktop-list.png') });
  console.log('PASS: progress, share, rename/cancel/failure, tags, wishlist, complete, view persistence');

  await page.setViewportSize({ width: 320, height: 640 });
  await open('rename');
  await page.locator('#book-editor-input').fill('旋转保留草稿');
  await page.setViewportSize({ width: 640, height: 320 });
  assert.equal(await page.locator('#book-editor-input').inputValue(), '旋转保留草稿');
  await page.locator('#book-editor [data-close]').click();
  for (const size of [{ width: 320, height: 640 }, { width: 640, height: 320 }]) {
    await page.setViewportSize(size);
    await more().click();
    await menu.locator('[data-book-action="delete"]').scrollIntoViewIfNeeded();
    const r = await menu.boundingBox();
    assert(r.x >= 0 && r.y >= 0 && r.x + r.width <= size.width + 1 && r.y + r.height <= size.height + 1);
    await page.screenshot({ path: path.join(artifacts, `menu-${size.width}.png`) });
    await menu.locator('[data-book-action="delete"]').click();
    await page.locator('#book-delete [data-close]').click();
    assert.equal(await page.locator('.book-card').count(), 1);
  }
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.locator('[data-shelf-view="grid"]').click();
  // Actual pointer moves: returning to the starting point still cannot click.
  const point = await more().boundingBox();
  const x = point.x + point.width / 2, y = point.y + point.height / 2;
  await page.mouse.move(x, y); await page.mouse.down();
  await page.mouse.move(x, y - 35, { steps: 6 }); await page.mouse.move(x, y, { steps: 6 }); await page.mouse.up();
  assert.equal(await menu.evaluate(node => node.open), false);
  await page.mouse.move(x, y); await page.mouse.down(); await page.waitForTimeout(600); await page.mouse.up();
  assert.equal(await menu.evaluate(node => node.open), false);
  await more().click(); await menu.locator('[data-close]').click();
  // Same filter protects opening the reader, not only the more button.
  const link = await page.locator('.book-link').boundingBox();
  await page.mouse.move(link.x + 20, link.y + 20); await page.mouse.down();
  await page.mouse.move(link.x + 60, link.y + 20); await page.mouse.move(link.x + 20, link.y + 20); await page.mouse.up();
  assert.equal(new URL(page.url()).pathname, '/');
  console.log('PASS: narrow/short dialogs, rotation draft, delete cancel, long press/returning drag rejected; next tap works');

  const mobileContext = await browser.newContext({ viewport: { width: 360, height: 640 }, isMobile: true, hasTouch: true });
  const mobile = await mobileContext.newPage();
  mobile.on('pageerror', error => errors.push(error.message));
  await mobile.goto(base);
  await mobile.locator('body[data-browser-ready="true"]').waitFor();
  await mobile.locator('.book-more').scrollIntoViewIfNeeded();
  await mobile.waitForTimeout(250);
  const m = await mobile.locator('.book-more').boundingBox();
  const touch = await mobileContext.newCDPSession(mobile);
  const mx = m.x + 20, my = m.y + 20;
  const sendTouch = (type, points) => touch.send('Input.dispatchTouchEvent', { type, touchPoints: points.map(([x, y, id = 1]) => ({ x, y, id })) });
  await sendTouch('touchStart', [[mx, my]]);
  for (let step = 1; step <= 6; step++) { await sendTouch('touchMove', [[mx, my - step * 8]]); await mobile.waitForTimeout(20); }
  for (let step = 5; step >= 0; step--) { await sendTouch('touchMove', [[mx, my - step * 8]]); await mobile.waitForTimeout(20); }
  await sendTouch('touchEnd', []);
  await mobile.waitForTimeout(350);
  assert.equal(await mobile.locator('#book-menu').evaluate(node => node.open), false);
  assert.equal(new URL(mobile.url()).pathname, '/');
  await mobile.locator('.book-more').scrollIntoViewIfNeeded();
  await mobile.waitForTimeout(300);
  await mobile.locator('.book-more').tap();
  await mobile.locator('#book-menu[open]').waitFor();
  await mobile.locator('#book-menu [data-close]').tap();
  await mobileContext.close();
  await post('/api/preferences', { language: 'en' });
  await page.reload(); await ready();
  assert.match(await page.locator('.book-status').textContent(), /Finished · 100%/);
  await more().click();
  assert.equal(await menu.locator('[data-book-action="rename"]').textContent(), 'Rename');
  await menu.locator('[data-close]').click();
  await post('/api/preferences', { language: 'zh-CN' });
  console.log('PASS: real simulated-touch scrolling does not open a card/menu, following tap works; English menu/progress');

  await stop(); base = await start(); await page.goto(base); await ready();
  assert.equal(await page.locator('.book-card h2').textContent(), '新书名 <测试>');
  await page.locator('.book-link').click(); await page.locator('#reading-content').waitFor();
  await page.goto(base); await ready();
  await open('delete');
  await page.route(`**/api/books/${id}`, route => route.fulfill({ status: 500, body: '模拟删除失败' }));
  await page.locator('#book-delete [type="submit"]').click();
  await page.locator('#book-delete .shelf-error').filter({ hasText: '模拟删除失败' }).waitFor();
  assert.equal(await page.locator('.book-card').count(), 1);
  await page.unroute(`**/api/books/${id}`);
  await page.locator('#book-delete [type="submit"]').click();
  await page.locator('.empty-state').waitFor();
  assert(fs.existsSync(fixture));
  assert.equal((await page.request.get(`${base}/read/${id}/0`)).status(), 404);
  assert.deepEqual(errors, []);
  console.log('PASS: restart persistence, normal opening, deletion failure/retry/last-book empty state, original file retained');
}
run().catch(error => { console.error(error); process.exitCode = 1; }).finally(async () => {
  await browser?.close(); await stop(); fs.writeFileSync(path.join(artifacts, 'server.log'), log); fs.rmSync(temporary, { recursive: true, force: true });
});
