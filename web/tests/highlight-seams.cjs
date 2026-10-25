// Scan complete line-boundary strips and code panels at DEVICE resolution.
// Background pixels must receive exactly one tint; opaque glyph pixels must not
// change at all. CSS-sized screenshots can hide one-device-pixel seams.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawn, execFileSync } = require('node:child_process');
const { chromium } = require(process.env.READWISE_PLAYWRIGHT_MODULE || 'playwright-core');
const root = path.resolve(__dirname, '../..');
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'readwise-seams-'));
const artifacts = path.join(root, 'target/web-qa/seams'); fs.mkdirSync(artifacts, { recursive: true });
const fixture = path.join(temporary, 'Seams.epub');
execFileSync('python3', [path.join(__dirname, 'highlight-fixture.py'), fixture]);
let server, browser, logs = '';
const errors = [];
async function start() {
  return new Promise((resolve, reject) => {
    server = spawn(process.env.READWISE_WEB_BINARY || path.join(root, 'target/debug/readwise-web'), [], { cwd: root, env: { ...process.env, READWISE_WEB_BIND: '127.0.0.1:0', READWISE_WEB_DATA_DIR: path.join(temporary, 'data') }, stdio: ['ignore', 'pipe', 'pipe'] });
    const timer = setTimeout(() => reject(new Error('Startup timed out')), 25000);
    const log = chunk => { logs += chunk; const match = logs.match(/ReadWise Web: (http:\/\/[^\s]+)\n/); if (match) { clearTimeout(timer); resolve(match[1]); } };
    server.stdout.on('data', log); server.stderr.on('data', log); server.on('error', reject);
  });
}
async function run() {
  const base = await start();
  const executablePath = process.env.READWISE_CHROMIUM || (process.platform === 'darwin' ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : undefined);
  browser = await chromium.launch({ headless: true, ...(executablePath ? { executablePath } : {}) });
  let page = await browser.newPage();
  const ready = () => page.locator('body[data-browser-ready="true"]').waitFor();
  const settle = () => page.waitForTimeout(250);
  const prefs = value => page.request.post(`${base}/api/preferences`, { headers: { 'x-readwise-request': '1' }, data: value });
  await page.goto(base); await ready();
  await page.locator('#book-upload').setInputFiles(fixture); await page.locator('#import-results [data-state="success"] a').click(); await ready();
  const url = page.url().split('#')[0];
  const profiles = [
    { dpr: 1, font: 24, zoom: 1, words: 0 },
    { dpr: 1.25, font: 24.2, zoom: 1, words: 0 },
    { dpr: 1.5, font: 24.2, zoom: 1, words: 0 },
    { dpr: 2, font: 24, zoom: 1, words: 2 },
    { dpr: 2.5, font: 24.2, zoom: 1, words: 0 },
    { dpr: 2, font: 24.2, zoom: 1.1, words: 2 },
    { dpr: 2, font: 24.2, zoom: 1.25, words: 0 },
    { dpr: 2, font: 24.2, zoom: 1, words: 2, night: true },
  ];
  let backgroundChecks = 0, inkChecks = 0;
  for (const [index, profile] of profiles.entries()) {
    await prefs({ language: 'zh-CN', english_level: 'A1', theme: profile.night ? 'dark' : 'light', background: profile.night ? 'night' : 'original', font: 'georgia', font_size: profile.font, gloss_size: 13.37, spacing: 1.55, turn_mode: 'scroll', columns: 'one', text_width: 'narrow', word_background: profile.words, selection_color_slot: 6, highlight_color_slot: 4 });
    await page.close(); page = await browser.newPage({ viewport: { width: 1200, height: 900 }, deviceScaleFactor: profile.dpr });
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(`${url}?qa=seams-${index}`); await ready();
    if (profile.zoom !== 1) await page.evaluate(zoom => { document.body.style.zoom = zoom; }, profile.zoom);
    await settle();
    for (const name of ['final', 'steps', 'plain']) {
      const selector = `#content-seams-${name}`, label = `${index}-${name}`;
      await page.locator(selector).evaluate(node => node.scrollIntoView({ block: 'center' })); await settle();
      const geometry = () => page.locator(selector).evaluate(p => {
        const walker = document.createTreeWalker(p, NodeFilter.SHOW_ALL), nodes = [];
        while (walker.nextNode()) nodes.push(walker.currentNode);
        window.__seamNodes ||= nodes;
        return { scrollX, scrollY, boxes: [p, ...p.querySelectorAll('*')].flatMap(node => [...node.getClientRects()].map(r => [r.x,r.y,r.width,r.height])) };
      });
      await page.evaluate(() => { window.__seamNodes = null; });
      const beforeGeometry = await geometry();
      const spec = await page.locator(selector).evaluate(p => {
        const colour = value => { const c = document.createElement('canvas'); c.width = c.height = 1; const ctx = c.getContext('2d'); ctx.fillStyle = value; ctx.fillRect(0,0,1,1); return [...ctx.getImageData(0,0,1,1).data].slice(0,3); };
        const palette = [colour(getComputedStyle(document.body).backgroundColor)], ink = [], fragments = [];
        const walker = document.createTreeWalker(p, NodeFilter.SHOW_TEXT);
        while (walker.nextNode()) {
          const node = walker.currentNode, parent = node.parentElement, css = getComputedStyle(parent);
          if (css.visibility !== 'visible' || parent.closest('rt,rp')) continue;
          ink.push(colour(css.color));
          for (let e = parent; e && e !== p.parentElement; e = e.parentElement) {
            const bg = getComputedStyle(e).backgroundColor; if (bg !== 'rgba(0, 0, 0, 0)') palette.push(colour(bg));
          }
          const range = document.createRange(); range.selectNodeContents(node);
          for (const r of range.getClientRects()) if (r.width && r.height) fragments.push({ left:r.left, right:r.right, top:r.top, bottom:r.bottom });
        }
        const rows = [];
        for (const r of fragments.sort((a,b) => a.bottom-b.bottom || a.left-b.left)) {
          let row = rows.find(row => Math.abs(row.bottom-r.bottom) < 10 * Number(document.body.style.zoom || 1));
          if (!row) { row = {...r}; rows.push(row); }
          else { row.left=Math.min(row.left,r.left); row.right=Math.max(row.right,r.right); row.top=Math.min(row.top,r.top); row.bottom=Math.max(row.bottom,r.bottom); }
        }
        rows.sort((a,b) => a.top-b.top);
        const regions = [];
        for (let i=1; i<rows.length; i++) {
          const a=rows[i-1], b=rows[i];
          regions.push({ left:Math.max(a.left,b.left)+2, right:Math.min(a.right,b.right)-2, top:Math.min(a.bottom,b.top)-2, bottom:Math.max(a.bottom,b.top)+2 });
        }
        for (const code of p.querySelectorAll('code')) for (const r of code.getClientRects()) regions.push({ left:r.left+1, right:r.right-1, top:r.top+1, bottom:r.bottom-1 });
        const r=p.getBoundingClientRect();
        return { palette, ink, regions, textBox:{left:r.left,right:r.right,top:r.top,bottom:r.bottom}, width:innerWidth };
      });
      assert.ok(spec.regions.length, label);
      const baseline = await page.screenshot({ path: path.join(artifacts, `${label}-before.png`), scale:'device' });
      const check = async (kind, ink, alpha, coverage = null) => {
        const png = await page.screenshot({ path: path.join(artifacts, `${label}-${kind}.png`), scale:'device' });
        const result = await page.evaluate(async ({baseline, png, spec, ink, alpha, coverage}) => {
          const decode = async png => { const im=new Image(); im.src=`data:image/png;base64,${png}`; await im.decode(); const c=document.createElement('canvas'); c.width=im.width; c.height=im.height; const ctx=c.getContext('2d'); ctx.drawImage(im,0,0); return { width:im.width, height:im.height, pixels:ctx.getImageData(0,0,im.width,im.height).data }; };
          const a=await decode(baseline), b=await decode(png), scale=a.width/spec.width, mismatches=[];
          let backgrounds=0, glyphs=0;
          const same=(rgb, i) => rgb.every((c,n) => a.pixels[i+n]===c);
          const compare=(x,y,expected,type) => { const i=(y*a.width+x)*4; const actual=[...b.pixels.slice(i,i+3)]; if (actual.some((c,n)=>Math.abs(c-expected[n])>3) && mismatches.length<12) mismatches.push({x,y,expected,actual,type}); };
          const scan=(r, visit) => { for(let y=Math.max(0,Math.ceil(r.top*scale)); y<Math.min(a.height,Math.floor(r.bottom*scale)); y++) for(let x=Math.max(0,Math.ceil(r.left*scale)); x<Math.min(a.width,Math.floor(r.right*scale)); x++) visit(x,y,(y*a.width+x)*4); };
          for (const r of spec.regions) scan(r,(x,y,i) => {
            if (!spec.palette.some(rgb=>same(rgb,i))) return;
            const px=(x+.5)/scale, py=(y+.5)/scale;
            // Only the OUTER partial-selection edge has two raster stages
            // (SVG texture + CSS background positioning). Exclude two device
            // pixels there; internal font-box edges/line seams remain scanned.
            const edge=Math.max(1,2/scale);
            if (coverage && (Math.abs(px-coverage.left)<edge || Math.abs(px-coverage.right)<edge || Math.abs(py-coverage.top)<edge || Math.abs(py-coverage.bottom)<edge)) return;
            const covered=!coverage || px>coverage.left && px<coverage.right && py>coverage.top && py<coverage.bottom;
            const expected=[...a.pixels.slice(i,i+3)].map((c,n)=>ink && covered ? Math.round(c*(1-alpha)+ink[n]*alpha) : c);
            backgrounds++; compare(x,y,expected,'background');
          });
          scan(spec.textBox,(x,y,i) => { if (spec.ink.some(rgb=>same(rgb,i))) { glyphs++; compare(x,y,[...a.pixels.slice(i,i+3)],'glyph'); } });
          return { backgrounds,glyphs,mismatches };
        }, { baseline:baseline.toString('base64'), png:png.toString('base64'), spec, ink, alpha, coverage });
        assert.deepEqual(result.mismatches, [], `${label} ${kind} ${JSON.stringify(coverage)}`); assert.ok(result.backgrounds>300); assert.ok(result.glyphs>100);
        backgroundChecks += result.backgrounds; inkChecks += result.glyphs;
      };
      const select = async () => { await page.locator(selector).evaluate(p => { const r=document.createRange(); r.selectNodeContents(p); getSelection().removeAllRanges(); getSelection().addRange(r); }); await page.locator('#selection-toolbar').waitFor({state:'visible'}); await settle(); };
      await select();
      await page.locator('#selection-toolbar').evaluate(node => { node.style.visibility='hidden'; });
      await check('selected',[59,122,61],136/255);
      await page.locator('#selection-toolbar').evaluate(node => { node.style.visibility=''; });
      await page.locator('[data-marker="0"]').click(); await page.locator('#selection-toolbar').waitFor({state:'hidden'}); await settle();
      await check('yellow',[242,196,56],89/255);
      assert.deepEqual(await geometry(),beforeGeometry);
      assert.equal(await page.locator(selector).evaluate(p => { const w=document.createTreeWalker(p,NodeFilter.SHOW_ALL), nodes=[]; while(w.nextNode()) nodes.push(w.currentNode); return nodes.length===window.__seamNodes.length && nodes.every((node,i)=>node===window.__seamNodes[i]); }),true);
      await select(); await page.locator('#toggle-highlight').click(); await page.locator('#selection-toolbar').waitFor({state:'hidden'}); await settle();
      await check('removed',null,0);
      await select(); await page.locator('[data-marker="3"]').click(); await page.locator('#selection-toolbar').waitFor({state:'hidden'}); await settle();
      await check('pink',[240,132,172],89/255);
      if (name === 'plain') {
        // Browser scrolling runs on the compositor while JS can be busy. Hold
        // ALL painter animation frames: saved paint must follow the text
        // without changing paths, chasing scroll events or exposing blank ink.
        const beforeScroll = await page.locator(selector).evaluate(p => {
          const svg = document.querySelector('.reader-highlight-gaps');
          const paths = [...svg.children].map(path => ({ d:path.getAttribute('d'), y:path.getBoundingClientRect().top }));
          window.__savedRAF = window.requestAnimationFrame;
          window.__heldRAF = [];
          window.requestAnimationFrame = callback => { window.__heldRAF.push(callback); return -1; };
          return { y:p.getBoundingClientRect().top, scrollY, paths };
        });
        await page.mouse.move(1150,450);
        await page.mouse.wheel(0, beforeScroll.scrollY > 180 ? -140 : 140);
        await page.waitForTimeout(180);
        const moving = await page.locator(selector).evaluate(p => {
          const svg = document.querySelector('.reader-highlight-gaps');
          return { y:p.getBoundingClientRect().top, paths:[...svg.children].map(path => ({ d:path.getAttribute('d'), y:path.getBoundingClientRect().top })) };
        });
        const delta = moving.y - beforeScroll.y;
        assert.ok(Math.abs(delta) > 80, `${label}: real wheel scroll occurred`);
        assert.deepEqual(moving.paths.map(p=>p.d), beforeScroll.paths.map(p=>p.d), `${label}: no scroll-driven geometry rewrite`);
        for (let i=0; i<moving.paths.length; i++) assert.ok(Math.abs(moving.paths[i].y-beforeScroll.paths[i].y-delta)<.02, `${label}: saved paint follows compositor while JS paint is stalled`);
        await page.screenshot({ path:path.join(artifacts, `${label}-scroll-stalled.png`), scale:'device' });
        await page.evaluate(y => {
          window.requestAnimationFrame = window.__savedRAF;
          for (const callback of window.__heldRAF) requestAnimationFrame(callback);
          scrollTo(0,y);
        }, beforeScroll.scrollY);
        await settle();
        await check('scroll-return',[240,132,172],89/255);
      }
      if (index===3 && name==='plain') {
        const ruby=page.locator(`${selector} ruby[data-lemma="solitude"]`); await ruby.click(); await page.locator('#word-card[open]').waitFor();
        const knownGeometry=await geometry(); await page.locator('#mark-known').click(); await page.locator('#word-card').waitFor({state:'hidden'}); await settle();
        assert.deepEqual(await geometry(),knownGeometry);
        assert.equal(await ruby.locator('.word').getAttribute('data-highlight-background'),null);
        assert.equal(await ruby.locator('rt').evaluate(node=>getComputedStyle(node).visibility),'hidden');
      }
      await select(); await page.locator('#toggle-highlight').click(); await page.locator('#selection-toolbar').waitFor({state:'hidden'}); await settle();
      // Swatches apply immediately; never use them as settings-only controls.
      assert.deepEqual(await geometry(),beforeGeometry);
      if (name==='final') {
        const partial = async () => page.locator(selector).evaluate(p => {
          const code=p.querySelector('code'), text=code.firstChild, r=document.createRange(); r.setStart(text,2); r.setEnd(text,text.length-2);
          getSelection().removeAllRanges(); getSelection().addRange(r);
          const selection=r.getBoundingClientRect(), box=code.getBoundingClientRect();
          const height=Math.max(box.height,parseFloat(getComputedStyle(code).lineHeight)*Number(document.body.style.zoom||1)), centre=(box.top+box.bottom)/2;
          return {left:selection.left,right:selection.right,top:centre-height/2,bottom:centre+height/2};
        });
        const coverage=await partial(); await settle();
        await page.locator('#selection-toolbar').evaluate(node=>{node.style.visibility='hidden';});
        await check('partial-selected',[59,122,61],136/255,coverage);
        await page.locator('#selection-toolbar').evaluate(node=>{node.style.visibility='';});
        await page.locator('[data-marker="0"]').click(); await page.locator('#selection-toolbar').waitFor({state:'hidden'}); await settle();
        await check('partial-yellow',[242,196,56],89/255,coverage);
        await partial(); await settle(); await page.locator('#toggle-highlight').click(); await page.locator('#selection-toolbar').waitFor({state:'hidden'}); await settle();
        assert.deepEqual(await geometry(),beforeGeometry);
      }
      await page.evaluate(() => { window.__idleStyles=0; window.__idleObserver=new MutationObserver(records=>{window.__idleStyles+=records.length;}); window.__idleObserver.observe(document.querySelector('#reading-content'),{subtree:true,attributes:true,attributeFilter:['style']}); });
      await page.waitForTimeout(300);
      assert.equal(await page.evaluate(()=>{window.__idleObserver.disconnect();return window.__idleStyles;}),0,'no self-sustaining paint/style mutation loop');
    }
    console.log(`PASS: full device-pixel strips/code/untinted glyphs, DPR ${profile.dpr}, font ${profile.font}, zoom ${profile.zoom}, ${profile.night?'night':'light'}, word background ${profile.words}`);
  }
  assert.deepEqual(errors,[]);
  console.log(`PASS: ${backgroundChecks} background pixels and ${inkChecks} opaque glyph pixels; selection/yellow/pink/remove, ruby and known-word continuity. Screenshots: ${artifacts}`);
}
run().catch(error=>{console.error(error);process.exitCode=1;}).finally(async()=>{
  if(browser) await browser.close();
  if(server && server.exitCode===null) await new Promise(resolve=>{const timer=setTimeout(()=>server.kill('SIGKILL'),5000);server.once('exit',()=>{clearTimeout(timer);resolve();});server.kill('SIGINT');});
  fs.writeFileSync(path.join(artifacts,'server.log'),logs);fs.rmSync(temporary,{recursive:true,force:true});
});
