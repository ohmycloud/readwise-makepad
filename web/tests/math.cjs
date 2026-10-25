// Real embedded Typst over PDF glyph geometry. All imports use an isolated DB.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawn } = require('node:child_process');
const { chromium } = require(process.env.READWISE_PLAYWRIGHT_MODULE || 'playwright-core');
const root = path.resolve(__dirname, '../..');
const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'readwise-math-'));
const artifacts = path.join(root, 'target/web-qa/math');
fs.mkdirSync(artifacts, { recursive: true });
const fixture = path.join(__dirname, 'fixtures/equations.pdf');
let server, browser, log = '';
const errors = [];
const checkMathPaint = require('./math-paint.cjs');
const checkMathParagraph = require('./math-paragraph.cjs');
async function checkInlineLeading(page, selector, label, requireWrap = true) {
  const metrics=await page.locator(selector).first().evaluate(math=>{
    const block=math.closest('.reader-block'),style=getComputedStyle(block);
    const font=parseFloat(style.fontSize),leading=parseFloat(style.lineHeight);
    const walker=document.createTreeWalker(block,NodeFilter.SHOW_TEXT),bottoms=[];
    while(walker.nextNode()) {
      const node=walker.currentNode;
      if(node.parentElement.closest('.typst-math,.math-remote-source,rt,rp,svg')) continue;
      for(let i=0;i<node.length;i++) {
        if(!node.data[i].trim()) continue;
        const range=document.createRange();range.setStart(node,i);range.setEnd(node,i+1);
        const r=range.getBoundingClientRect();if(r.width && r.height)bottoms.push(r.bottom);
      }
    }
    const rows=[];
    for(const y of bottoms.sort((a,b)=>a-b))if(!rows.length||y-rows.at(-1)>.75)rows.push(y);
    const gaps=rows.slice(1).map((y,i)=>y-rows[i]);
    const canvas=math.querySelector('.math-canvas'),svg=math.querySelector('svg'),ink=svg.getBBox(),box=svg.viewBox.baseVal;
    return {font,leading,gaps,height:canvas.getBoundingClientRect().height,fontRatio:Math.max(...[...math.querySelectorAll('.math-source>span')].map(s=>parseFloat(getComputedStyle(s).fontSize)))/font,
      contained:ink.x>=box.x-.001&&ink.y>=box.y-.001&&ink.x+ink.width<=box.x+box.width+.001&&ink.y+ink.height<=box.y+box.height+.001};
  });
  assert(!requireWrap || metrics.gaps.length>=1,`${label}: exercise a genuinely wrapped paragraph`);
  assert(Math.max(...metrics.gaps)<=metrics.leading+metrics.font*.25,`${label}: expanded line spacing: ${JSON.stringify(metrics)}`);
  assert(metrics.height/metrics.font<1.65,`${label}: no oversized fraction canvas`);
  assert(metrics.fontRatio<.8,`${label}: original script size must not become a full prose em`);
  assert(metrics.contained,`${label}: all SVG ink remains inside its canvas`);
  console.log(`PASS: ${label}: inline fraction ${(metrics.height/metrics.font).toFixed(3)}em, line steps ${metrics.gaps.map(n=>n.toFixed(2)).join('/')}, leading ${metrics.leading}`);
}
async function start() {
  return new Promise((resolve, reject) => {
    server = spawn(process.env.READWISE_WEB_BINARY || path.join(root, 'target/debug/readwise-web'), [], { cwd: root, env: { ...process.env, READWISE_WEB_BIND: '127.0.0.1:0', READWISE_WEB_DATA_DIR: path.join(temp, 'data') } });
    const timer = setTimeout(() => reject(new Error('startup timeout')), 25000);
    let startup = '';
    const receive = chunk => { log += chunk; startup += chunk; const match = startup.match(/ReadWise Web: (http:\/\/[^\s]+)\n/); if (match) { clearTimeout(timer); resolve(match[1]); } };
    server.stdout.on('data', receive); server.stderr.on('data', receive); server.on('error', reject);
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
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: 1.5 });
  const page = await context.newPage();
  page.on('pageerror', error => errors.push(error.message));
  const ready = async () => { await page.locator('body[data-browser-ready="true"]').waitFor(); await page.waitForTimeout(200); };
  const post = async (route, data) => { const r = await page.request.post(`${base}${route}`, { headers: { 'x-readwise-request': '1' }, data }); assert.equal(r.status(), 200, await r.text()); return r.json(); };
  const importBook = async file => {
    await page.goto(base); await ready(); await page.locator('#book-upload').setInputFiles(file);
    await page.locator('#import-results [data-state="success"] a').waitFor({ timeout: 60000 });
    return (await page.locator('#import-results a').getAttribute('href')).split('/').at(-1);
  };
  await page.goto(base); await ready();
  await post('/api/preferences', { turn_mode: 'scroll', columns: 'one', theme: 'light', background: 'paper', text_width: 'wide' });
  const id = await importBook(fixture);
  await page.goto(`${base}/read/${id}/0`); await ready();
  const formulas = page.locator('.typst-math');
  console.log('Owned fixture projected formulas:', await formulas.count());
  if (await formulas.count() < 2) console.log(await page.locator('.reader-block').allTextContents());
  assert(await page.locator('.math-display').count() >= 1, 'display fraction rendered by Typst');
  assert(await formulas.count() >= 2, 'inline scripts also rendered');
  assert.equal(await formulas.locator('svg text').count(), 0);
  assert(await formulas.locator('svg path').count() > 0);
  assert.equal(await formulas.locator('ruby').count(), 0);
  const source = () => page.locator('.reader-block').evaluateAll(nodes => nodes.map(node => {
    const walker = document.createTreeWalker(node, NodeFilter.SHOW_TEXT, { acceptNode(n) { return n.parentElement.closest('rt,rp,svg') ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT; } });
    let text = ''; while (walker.nextNode()) text += walker.currentNode.textContent; return text;
  }));
  const initial = await source();
  await context.grantPermissions(['clipboard-read','clipboard-write'], { origin: base });
  for (const [selector,label] of [['.math-display','fixture-display'],['.typst-math:not(.math-display)','fixture-inline']]) {
    await checkMathPaint({page,selector:`${selector} >> nth=0`,label,artifacts});
    assert.deepEqual(await source(),initial);
  }
  await page.screenshot({ path: path.join(artifacts, 'fixture-light.png'), fullPage: true });
  // Source remains independently selectable/copyable, without SVG definitions.
  const selected = await page.locator('.math-source').first().evaluate(node => { const r = document.createRange(); r.selectNodeContents(node); getSelection().removeAllRanges(); getSelection().addRange(r); return getSelection().toString(); });
  assert(selected.length > 4 && !selected.includes('<path'));
  await context.grantPermissions(['clipboard-read','clipboard-write'], { origin: base });
  await page.locator('#selection-toolbar').waitFor({ state: 'visible' });
  await page.locator('#more-selection').click();
  await page.locator('#copy-selection').click();
  await page.getByText('已复制选中文本', { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => navigator.clipboard.readText()), selected);
  const saved = page.waitForResponse(r => r.url().endsWith('/api/highlights/apply') && r.status() === 200);
  await page.locator('[data-marker="1"]').click(); await saved;
  await page.locator('#selection-toolbar').waitFor({ state: 'hidden' });
  await page.waitForTimeout(100);
  assert(await page.locator('.reader-highlight-gaps path').count() > 0);
  assert.deepEqual(await source(), initial);
  const geometry = () => page.locator('.reader-block').evaluateAll(nodes => nodes.map(n => { const r = n.getBoundingClientRect(); return [r.x,r.y,r.width,r.height]; }));
  await page.locator('ruby[data-lemma="solitude"]').first().click();
  const before = await geometry();
  await page.locator('#mark-known').click(); await page.waitForFunction(() => !document.querySelector('#word-card').open);
  assert.deepEqual(await geometry(), before);
  assert.deepEqual(await source(), initial);
  await post('/api/preferences', { background: 'night', theme: 'dark' });
  await page.reload(); await ready();
  assert.deepEqual(await source(), initial);
  const svg = formulas.locator('svg').first();
  assert.equal(await svg.evaluate(node => getComputedStyle(node).color), 'rgb(216, 212, 202)');
  await page.screenshot({ path: path.join(artifacts, 'fixture-dark.png'), fullPage: true });
  await page.setViewportSize({ width: 320, height: 640 });
  await post('/api/preferences', { font_size: 40 }); await page.reload(); await ready();
  const wide = page.locator('.math-display').first();
  assert(await wide.evaluate(node => node.scrollWidth > node.clientWidth + 10), 'long math scrolls inside the viewport');
  await wide.focus(); await page.keyboard.press('ArrowRight'); await page.waitForTimeout(200);
  assert(await wide.evaluate(node => node.scrollLeft > 0), 'keyboard scroll reaches overflowing math');
  const clip=await wide.boundingBox();
  assert(await page.locator('.reader-highlight-gaps path').evaluateAll((nodes,clip)=>nodes.filter(n=>n.getAttribute('d')).every(n=>{const r=n.getBoundingClientRect();return r.left>=clip.x-0.1 && r.right<=clip.x+clip.width+0.1;}),clip),'saved math tint follows internal scroll and stays inside its viewport');
  await page.screenshot({ path: path.join(artifacts, 'fixture-320-large.png'), fullPage: true });
  assert.deepEqual(await source(), initial);
  await post('/api/preferences', { font_size: 22 }); await page.reload(); await ready();
  const bounds = await formulas.evaluateAll(nodes => nodes.map(n => { const r = n.getBoundingClientRect(); return [r.left,r.right]; }));
  assert(bounds.every(([left,right]) => left >= 0 && right <= 321));
  await page.screenshot({ path: path.join(artifacts, 'fixture-320.png'), fullPage: true });
  await page.setViewportSize({ width: 640, height: 320 });
  assert.deepEqual(await source(), initial);
  await page.setViewportSize({ width: 1280, height: 900 });
  await post('/api/preferences', { turn_mode: 'slide', columns: 'two' });
  await page.reload(); await ready();
  assert(await formulas.count() >= 2);
  assert.deepEqual(await source(), initial);
  await stop(); base = await start(); await context.grantPermissions(['clipboard-read','clipboard-write'], { origin: base }); await page.goto(`${base}/read/${id}/0`); await ready();
  assert.deepEqual(await source(), initial);
  assert(await formulas.count() >= 2);
  assert.equal((await (await page.request.get(`${base}/api/highlights/${id}/0`)).json()).length, 1);
  console.log('PASS: real Typst fraction/scripts, original source text/copy, known-word continuity, light/dark, narrow/short, pagination and restart');

  const scattered = await importBook(path.join(__dirname,'fixtures/math-scattered.pdf'));
  await post('/api/preferences',{turn_mode:'scroll',columns:'one',background:'paper',theme:'light'});
  await page.goto(`${base}/read/${scattered}/0`); await ready();
  console.log('Scattered fixture source:',await source());
  assert.equal(await page.locator('[data-math-symbols="∑"]').count(),1,'real CMEX summation is painted, not Roman P');
  assert(await page.locator('.math-remote-source').count()>0,'fixture really exercises displaced legacy source');
  assert(await formulas.count()>=3,'both subscripts and summation with limits');
  const scatteredSource=await source();
  const fractionBlock=scatteredSource.findIndex(text=>text.includes('The scale is'));
  assert(fractionBlock>=0);
  const fraction=`[data-block="${fractionBlock}"] .typst-math:not(.math-display)`;
  assert.equal(await page.locator(fraction).count(),1,'owned fixture has a real small-font inline fraction');
  assert(await page.locator(`${fraction} svg path[stroke="currentColor"]`).count()>0,'small-font fraction retains its real overhanging PDF bar');
  for(const [width,height,font_size] of [[360,720,22],[360,720,40],[640,320,24.2],[1280,900,22]]) {
    await page.setViewportSize({width,height});
    await post('/api/preferences',{font_size});await page.reload();await ready();
    await checkInlineLeading(page,fraction,`fraction-${width}-${font_size}`,width<1000);
    assert.deepEqual(await source(),scatteredSource);
  }
  await checkMathPaint({page,selector:fraction,label:'fixture-inline-fraction',artifacts});
  await page.locator(`[data-block="${fractionBlock}"]`).screenshot({path:path.join(artifacts,'fixture-inline-leading.png')});
  await page.setViewportSize({width:640,height:800});
  await checkMathParagraph({page,selector:`[data-block="${fractionBlock}"]`,label:'fixture-paragraph',artifacts});
  await page.setViewportSize({width:1280,height:900});
  const sumId=await page.locator('[data-math-symbols="∑"]').getAttribute('data-math-id');
  await checkMathPaint({page,selector:`[data-math-id="${sumId}"]`,label:'fixture-sum',artifacts});
  assert.deepEqual(await source(),scatteredSource);
  await post('/api/preferences',{background:'night',theme:'dark',font_size:24.2}); await page.reload(); await ready();
  await checkMathPaint({page,selector:`[data-math-id="${sumId}"]`,label:'fixture-sum-dark',artifacts});
  await checkMathPaint({page,selector:fraction,label:'fixture-inline-fraction-dark',artifacts});
  assert.deepEqual(await source(),scatteredSource);
  await post('/api/preferences',{background:'paper',theme:'light',font_size:22}); await page.reload(); await ready();
  await page.screenshot({path:path.join(artifacts,'fixture-scattered.png'),fullPage:true});
  const oldBlock=scatteredSource.findIndex(text=>text.includes('P'));
  const oldOffset=Buffer.byteLength(scatteredSource[oldBlock].split('P')[0]);
  const legacy=await post('/api/highlights',{book_id:scattered,chapter:0,start:{block:oldBlock,offset:oldOffset},end:{block:oldBlock,offset:oldOffset+1},color:2});
  await page.reload(); await ready();
  const sum=page.locator(`[data-math-id="${sumId}"]`);
  await sum.evaluate(n=>n.scrollIntoView({block:'center'})); await page.waitForTimeout(160);
  const inkBox=await sum.locator('.math-canvas').boundingBox();
  const oldPaint=await page.locator('.reader-highlight-gaps path[fill="#6aa8f059"]').boundingBox();
  assert(oldPaint && oldPaint.x<=inkBox.x+0.1 && oldPaint.y<=inkBox.y+0.1 && oldPaint.x+oldPaint.width>=inkBox.x+inkBox.width-0.1 && oldPaint.y+oldPaint.height>=inkBox.y+inkBox.height-0.1,'old displaced P source range paints the entire visual sum');
  assert.deepEqual(await source(),scatteredSource);
  await page.screenshot({path:path.join(artifacts,'fixture-legacy-P-highlight.png'),fullPage:true});
  await post('/api/highlights/remove',{book_id:scattered,chapter:0,ids:[legacy.id]});
  console.log('PASS: pre-existing displaced P coordinate projects onto the complete sum without source migration');

  // Optional local acceptance of the user's PDF, copied through normal import;
  // never opens or modifies a real application database or its original file.
  if (process.env.READWISE_MATH_PDF) {
    const original = process.env.READWISE_MATH_PDF;
    const bytesBefore = fs.readFileSync(original);
    const attention = await importBook(original);
    await post('/api/preferences', { turn_mode: 'scroll', columns: 'one', background: 'paper', theme: 'light' });
    for (const chapter of [3,4,6]) {
      await page.goto(`${base}/read/${attention}/${chapter}`); await ready();
      console.log(`Reference page ${chapter + 1}:`, await formulas.count(), 'formulas;', await page.locator('.math-display').count(), 'display');
      if (chapter === 3) {
        assert.equal(await page.locator('[data-block="6"] .typst-math').count(),3,'two displaced subscripts and radical recovered');
        assert.equal(await page.locator('[data-block="13"] [data-math-symbols="∑"]').count(),1);
        for (const [selector,label] of [['.math-display','attention-display'],['[data-block="10"] .typst-math:has-text("√1")','attention-inline']]) {
          await checkMathPaint({page,selector:`${selector} >> nth=0`,label,artifacts});
        }
        await checkInlineLeading(page,'[data-block="10"] .typst-math:has-text("√1")','attention-inline-leading');
        for(const block of [9,10]) await checkMathParagraph({page,selector:`[data-block="${block}"]`,label:`attention-paragraph-${block}`,artifacts});
        for (const block of [6,9,10,13]) await page.locator(`[data-block="${block}"]`).screenshot({path:path.join(artifacts,`attention-block-${block}.png`)});
      }
      if(chapter===4) {
        const blocks=await source(),index=blocks.findIndex(text=>text.startsWith('Where the projections'));
        assert(index>=0,'reported projection-matrices paragraph exists');
        const prose = blocks.findIndex(text=>text.startsWith('and the memory keys'));
        assert(prose>=0);
        assert.equal(await page.locator(`[data-block="${prose}"] .typst-math`).count(),0,'ordinary "and values" must not be replaced with a displaced dv formula');
        await checkMathParagraph({page,selector:`[data-block="${index}"]`,label:'attention-matrices-paragraph',artifacts});
      }
      const named = page.locator('.math-display').filter({ hasText: {3:'Attention',4:'MultiHead',6:'lrate'}[chapter] }).first();
      assert.equal(await named.count(), 1, 'the reported equation itself is projected');
      await page.screenshot({ path: path.join(artifacts, `attention-page-${chapter + 1}.png`), fullPage: true });
      await named.screenshot({ path: path.join(artifacts, `attention-equation-${chapter + 1}.png`) });
    }
    assert.deepEqual(fs.readFileSync(original), bytesBefore);
    console.log('PASS: screenshot reference PDF pages 4/5/7, original file bytes unchanged');
  }
  assert.deepEqual(errors, []);
}
run().catch(error => { console.error(error); process.exitCode = 1; }).finally(async () => {
  await browser?.close(); await stop(); fs.writeFileSync(path.join(artifacts, 'server.log'), log); fs.rmSync(temp, { recursive: true, force: true });
});
