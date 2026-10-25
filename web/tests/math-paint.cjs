// Pixel regression: the WHOLE formula ink box, not merely nonempty SVG paths.
const assert = require('node:assert/strict');
const path = require('node:path');
module.exports = async function checkMathPaint({ page, selector, label, artifacts }) {
  const formula = page.locator(selector), canvas = formula.locator('.math-canvas');
  await page.evaluate(() => getSelection().removeAllRanges());
  await formula.evaluate(n=>n.scrollIntoView({block:'center',inline:'nearest'})); await page.waitForTimeout(160);
  let baseline = await canvas.screenshot({ path: path.join(artifacts, `${label}-before.png`) });
  const colours = await formula.evaluate(node => {
    const rgb = s => s.match(/[\d.]+/g).slice(0,3).map(Number);
    return { paper:rgb(getComputedStyle(document.body).backgroundColor), glyph:rgb(getComputedStyle(node).color), selection:getComputedStyle(document.querySelector('#reading-content')).getPropertyValue('--selection-ink').trim() };
  });
  const source = await formula.locator('.math-source').textContent();
  const geometry = await formula.evaluate(n => { window.__mathNodes=[n,...n.querySelectorAll('*')]; return window.__mathNodes.map(n => {const r=n.getBoundingClientRect();return [r.x,r.y,r.width,r.height];}); });
  const check = async (kind, ink, alpha, captured) => {
    const png = captured || await canvas.screenshot({ path:path.join(artifacts, `${label}-${kind}.png`) });
    const result = await page.evaluate(async ({a,b,paper,glyph,ink,alpha}) => {
      const decode=async data=>{const im=new Image();im.src=`data:image/png;base64,${data}`;await im.decode();const c=document.createElement('canvas');c.width=im.width;c.height=im.height;const ctx=c.getContext('2d');ctx.drawImage(im,0,0);return {w:im.width,h:im.height,data:ctx.getImageData(0,0,im.width,im.height).data};};
      const before=await decode(a),after=await decode(b), mismatches=[]; let backgrounds=0,glyphs=0;
      if(before.w!==after.w||before.h!==after.h)throw new Error('formula geometry changed');
      for(let y=2;y<before.h-2;y++)for(let x=2;x<before.w-2;x++){
        const i=(y*before.w+x)*4,original=[...before.data.slice(i,i+3)];
        let expected;
        if(original.every((c,n)=>c===paper[n])){expected=paper.map((c,n)=>Math.round(c*(1-alpha)+ink[n]*alpha));backgrounds++;}
        else if(original.every((c,n)=>c===glyph[n])){expected=original;glyphs++;}
        else continue; // anti-aliased ink is neither paper nor an opaque glyph
        if(expected.some((c,n)=>Math.abs(c-after.data[i+n])>3)&&mismatches.length<12)mismatches.push({x,y,expected,actual:[...after.data.slice(i,i+3)]});
      }
      return {backgrounds,glyphs,mismatches};
    }, {a:baseline.toString('base64'),b:png.toString('base64'),...colours,ink,alpha});
    assert(result.backgrounds>250 && result.glyphs>10, `${label}: meaningful complete-formula pixel scan`);
    assert.deepEqual(result.mismatches, [], `${label} ${kind}: no unpainted fraction, root, script or limits; glyphs unchanged`);
  };
  // REAL mouse drag, including display equations whose source extraction order
  // jumps from numerator to base to denominator. Native user-select:all is used.
  const box = await canvas.boundingBox();
  await page.mouse.move(box.x+2,box.y+box.height/2); await page.mouse.down();
  await page.mouse.move(box.x+box.width-2,box.y+box.height/2,{steps:18}); await page.mouse.up();
  await page.locator('#selection-toolbar').waitFor({state:'visible'}); await page.waitForTimeout(140);
  await page.locator('#more-selection').click();
  await page.locator('#copy-selection').click();
  await page.getByText('已复制选中文本',{exact:true}).waitFor();
  assert.equal(await page.evaluate(()=>navigator.clipboard.readText()),source,'drag/copy selects the complete formula source, without SVG definitions');
  await page.locator('#selection-toolbar').evaluate(n=>{n.style.visibility='hidden';});
  const selection = colours.selection;
  const colour = await page.evaluate(value=>{const c=document.createElement('canvas');c.width=c.height=1;const ctx=c.getContext('2d');ctx.fillStyle=value;ctx.fillRect(0,0,1,1);return [...ctx.getImageData(0,0,1,1).data];},selection);
  await check('selected',colour.slice(0,3),colour[3]/255);
  await page.locator('#selection-toolbar').evaluate(n=>{n.style.visibility='';});
  await page.locator('[data-marker="0"]').click();
  await page.locator('#selection-toolbar').waitFor({state:'hidden'}); await page.waitForTimeout(140);
  await check('yellow',[242,196,56],89/255);
  assert.equal(await formula.evaluate(n=>{const nodes=[n,...n.querySelectorAll('*')];return nodes.length===window.__mathNodes.length && nodes.every((n,i)=>n===window.__mathNodes[i]);}),true);
  assert.deepEqual(await formula.evaluate(n=>[n,...n.querySelectorAll('*')].map(n=>{const r=n.getBoundingClientRect();return [r.x,r.y,r.width,r.height];})),geometry);
  const vector = await formula.locator('svg').evaluate(n=>n.outerHTML);
  await page.reload(); await page.locator('body[data-browser-ready="true"]').waitFor();
  await formula.evaluate(n=>n.scrollIntoView({block:'center',inline:'nearest'})); await page.waitForTimeout(140);
  assert.equal(await formula.locator('svg').evaluate(n=>n.outerHTML),vector,'reload cannot change mathematical glyphs');
  const reloaded = await canvas.screenshot({path:path.join(artifacts,`${label}-reloaded.png`)});
  // A programmatic partial source selection is expanded to the visual unit for
  // save/remove/copy as well, without modifying any stored coordinate encoding.
  await formula.locator('.math-source').evaluate(node=>{const w=document.createTreeWalker(node,NodeFilter.SHOW_TEXT);w.nextNode();const n=w.currentNode,r=document.createRange();r.setStart(n,0);r.setEnd(n,1);getSelection().removeAllRanges();getSelection().addRange(r);});
  await page.locator('#selection-toolbar').waitFor({state:'visible'});
  await page.locator('#toggle-highlight').click(); await page.locator('#selection-toolbar').waitFor({state:'hidden'}); await page.waitForTimeout(140);
  // Reload can change the fractional page-scroll raster origin. Compare its
  // saved paint with an unmarked baseline at THAT SAME origin, not the old
  // page's differently antialiased ink. This still scans every paper/ink pixel.
  baseline = await canvas.screenshot({path:path.join(artifacts,`${label}-removed.png`)});
  await check('reloaded',[242,196,56],89/255,reloaded);
  assert(await page.locator('.reader-highlight-gaps path').evaluateAll(nodes=>nodes.every(n=>!n.getAttribute('d'))),'removal clears saved and live paint');
  console.log(`PASS: ${label}: real formula drag/copy, complete selection/yellow/reload/remove pixel coverage, unchanged glyphs/nodes/geometry`);
};
