// Diagram-only raster regions: ordinary headings/captions/prose stay real text.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const { spawn } = require('node:child_process');
const { chromium } = require(process.env.READWISE_PLAYWRIGHT_MODULE || 'playwright-core');
const root = path.resolve(__dirname, '../..');
const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'readwise-vector-'));
const artifacts = path.join(root, 'target/web-qa/vector-figure');
fs.mkdirSync(artifacts, { recursive: true });
let server, browser, log = '';
async function start() {
  return new Promise((resolve, reject) => {
    server = spawn(process.env.READWISE_WEB_BINARY || path.join(root, 'target/debug/readwise-web'), [], {
      cwd: root, env: { ...process.env, READWISE_WEB_BIND: '127.0.0.1:0', READWISE_WEB_DATA_DIR: path.join(temp, 'data') },
    });
    const timer = setTimeout(() => reject(new Error('startup timeout')), 25000);
    const receive = chunk => {
      log += chunk;
      const match = log.match(/ReadWise Web: (http:\/\/[^\s]+)\n/g);
      if (match) { clearTimeout(timer); resolve(match.at(-1).split(' ')[2].trim()); }
    };
    server.stdout.on('data', receive); server.stderr.on('data', receive); server.on('error', reject);
  });
}
async function stop() {
  if (server && server.exitCode === null) await new Promise(resolve => { server.once('exit', resolve); server.kill('SIGINT'); });
}
async function run() {
  let base = await start();
  browser = await chromium.launch({ headless: true, ...(process.platform === 'darwin' ? {
    executablePath: process.env.READWISE_CHROMIUM || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  } : {}) });
  const page = await browser.newPage({ viewport: { width: 1280, height: 1000 } });
  const errors = []; page.on('pageerror', e => errors.push(e.message));
  const ready = async () => { await page.locator('body[data-browser-ready="true"]').waitFor(); await page.waitForTimeout(200); };
  const post = async (route, data) => {
    const r = await page.request.post(base + route, { headers: { 'x-readwise-request': '1' }, data });
    assert.equal(r.status(), 200, await r.text()); return r.json();
  };
  const importBook = async file => {
    await page.goto(base); await ready(); await page.locator('#book-upload').setInputFiles(file);
    await page.locator('#import-results [data-state="success"] a').waitFor({ timeout: 60000 });
    return (await page.locator('#import-results a').getAttribute('href')).split('/').at(-1);
  };
  const source = () => page.locator('.reader-block').evaluateAll(nodes => nodes.map(node => {
    const walker = document.createTreeWalker(node, NodeFilter.SHOW_TEXT, {
      acceptNode: n => n.parentElement.closest('rt,rp,svg') ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT,
    });
    let s = ''; while (walker.nextNode()) s += walker.currentNode.data; return s;
  }));
  const visibleText = () => page.locator('#reading-content').evaluate(node => {
    const clone = node.cloneNode(true);
    clone.querySelectorAll('.typst-math,.math-remote-source,rt,rp').forEach(n => n.remove());
    return clone.textContent;
  });
  await page.goto(base); await ready();
  await post('/api/preferences', { turn_mode: 'scroll', columns: 'one', theme: 'light', background: 'paper' });
  const fixture = path.join(__dirname, 'fixtures/vector-figure.pdf'), original = fs.readFileSync(fixture);
  const id = await importBook(fixture);
  await page.goto(`${base}/read/${id}/0`); await ready();
  const initial = await source();
  const findSnapshot = dir => fs.readdirSync(dir, { withFileTypes: true }).flatMap(e => e.isDirectory()
    ? findSnapshot(path.join(dir, e.name)) : e.name === 'pdf-text-v1.json' ? [path.join(dir, e.name)] : []);
  const snapshots = findSnapshot(path.join(temp, 'data')); assert.equal(snapshots.length, 1);
  const snapshot = fs.readFileSync(snapshots[0]);
  await page.locator('.pdf-region-figure img').waitFor();
  const image = await page.locator('.pdf-region-figure img').evaluate(img => {
    const canvas = document.createElement('canvas'); canvas.width = img.naturalWidth; canvas.height = img.naturalHeight;
    canvas.getContext('2d').drawImage(img, 0, 0);
    const data = canvas.getContext('2d').getImageData(0, 0, canvas.width, canvas.height).data;
    let colour = 0, proseInk = 0;
    for (let i = 0; i < data.length; i += 4) {
      const [r, g, b] = data.slice(i, i + 3);
      if (Math.max(r, g, b) - Math.min(r, g, b) > 30) colour++;
      // The fixture's title is green, caption magenta and prose cyan. None of
      // these sentinel inks belongs to the graph (including anti-aliased ink).
      if ((r < 25 && g > 150 && b < 25) || (r > 150 && g < 25 && b > 150) || (r < 25 && g > 130 && b > 130)) proseInk++;
    }
    return { complete: img.complete, w: img.naturalWidth, h: img.naturalHeight, colour, proseInk };
  });
  assert(image.complete && image.w > 100 && image.h > 100 && image.colour > 2000, JSON.stringify(image));
  assert.equal(image.proseInk, 0, 'no title/caption/prose pixels in the diagram bitmap');
  assert.equal(await page.locator('.pdf-region-figure').count(), 1);
  assert.equal(await page.locator('.pdf-region-figure ruby').count(), 0);
  const prose = await visibleText();
  for (const text of ['Diagram overview', 'Figure 1: coloured edges stay visible.', 'Ordinary forest text stays selectable.']) assert(prose.includes(text), text);
  assert(!prose.includes('label0'), 'diagram labels must not duplicate as visible prose');
  assert((await page.locator('.reader-block h1').allTextContents()).join('').includes('Diagram'));
  // Real ruby interaction in a mixed prose/diagram source block must not replace
  // the image, its source nodes, or any layout geometry.
  const forest = page.locator('ruby[data-lemma="forest"]').first();
  await forest.waitFor();
  await page.evaluate(() => {
    window.__figure = document.querySelector('.pdf-region-figure');
    window.__figureRect = JSON.stringify(window.__figure.getBoundingClientRect());
  });
  await forest.click(); await page.locator('#mark-known').click();
  await page.waitForFunction(() => !document.querySelector('#word-card').open);
  assert(await page.evaluate(() => window.__figure === document.querySelector('.pdf-region-figure') && window.__figureRect === JSON.stringify(window.__figure.getBoundingClientRect())));
  assert.deepEqual(await source(), initial);
  for (const viewport of [{ width: 1280, height: 1000 }, { width: 360, height: 740 }, { width: 640, height: 320 }]) {
    await page.setViewportSize(viewport); await page.waitForTimeout(250);
    const rect = await page.locator('.pdf-region-figure img').boundingBox();
    assert(rect.width > 0 && rect.width <= viewport.width && rect.x >= -1, JSON.stringify(rect));
    assert.deepEqual(await source(), initial); assert.equal(await visibleText(), prose);
    await page.screenshot({ path: path.join(artifacts, `figure-${viewport.width}x${viewport.height}.png`), fullPage: true });
  }
  await page.reload(); await ready(); assert.deepEqual(await source(), initial);
  for (const chapter of [1, 2, 4, 5]) {
    await page.goto(`${base}/read/${id}/${chapter}`); await ready();
    assert.equal(await page.locator('.pdf-region-figure').count(), 0, 'ordinary/overlapping/ambiguous text must not be rasterized');
  }
  assert(fs.readFileSync(snapshots[0]).equals(snapshot)); assert(fs.readFileSync(fixture).equals(original));
  await stop(); log = ''; base = await start();
  await page.goto(`${base}/read/${id}/0`); await ready();
  assert.equal(await page.locator('.pdf-region-figure img').count(), 1);
  assert.deepEqual(await source(), initial); assert(fs.readFileSync(snapshots[0]).equals(snapshot));
  if (process.env.READWISE_MATH_PDF) {
    const reference = process.env.READWISE_MATH_PDF, bytes = fs.readFileSync(reference), ref = await importBook(reference);
    for (const chapter of [12, 13, 14]) {
      await page.goto(`${base}/read/${ref}/${chapter}`); await ready();
      assert.equal(await page.locator('.pdf-region-figure img').count(), 1, `reference page ${chapter + 1}`);
      const text = await visibleText(); assert(text.includes(`Figure ${chapter - 9}:`), 'real caption must remain text');
      assert(!await page.locator('.pdf-region-figure').evaluate(n => n.textContent.includes('Figure ')), 'caption is not a hidden image-source proxy');
      if (chapter === 12) { assert(text.includes('Attention') && text.includes('Visualizations')); assert.equal(await page.locator('.reader-block h4').count(), 2); }
      await page.screenshot({ path: path.join(artifacts, `attention-${chapter + 1}.png`), fullPage: true });
    }
    assert(fs.readFileSync(reference).equals(bytes));
  }
  assert.deepEqual(errors, []);
  console.log('PASS: diagram-only pixels; real heading/caption/prose, ruby continuity, unchanged source/snapshot, narrow/landscape, controls and restart');
}
run().catch(e => { console.error(e); console.error(log); process.exitCode = 1; }).finally(async () => {
  await browser?.close(); await stop(); fs.writeFileSync(path.join(artifacts, 'server.log'), log);
  fs.rmSync(temp, { recursive: true, force: true });
});
