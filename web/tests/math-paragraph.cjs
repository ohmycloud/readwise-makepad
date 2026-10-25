// Full PARAGRAPH regression: uniform row edges and no white line between rows,
// not just a filled formula rectangle. Source nodes/anchors stay untouched.
const assert=require('node:assert/strict');
const path=require('node:path');
module.exports=async function({page,selector,label,artifacts}) {
  const block=page.locator(selector);
  await page.evaluate(()=>getSelection().removeAllRanges());
  await block.evaluate(n=>n.scrollIntoView({block:'center'}));await page.waitForTimeout(180);
  const measure=()=>block.evaluate(n=>{
    const bounds=n.getBoundingClientRect(),font=parseFloat(getComputedStyle(n).fontSize),leading=parseFloat(getComputedStyle(n).lineHeight),rows=[];
    const w=document.createTreeWalker(n,NodeFilter.SHOW_TEXT);
    while(w.nextNode()) {
      const t=w.currentNode;if(t.parentElement.closest('rt,rp,svg,.typst-math,.math-remote-source'))continue;
      for(let i=0;i<t.length;i++)if(t.data[i].trim()) {
        const range=document.createRange();range.setStart(t,i);range.setEnd(t,i+1);const r=range.getBoundingClientRect();if(!r.width)continue;
        let row=rows.find(row=>Math.abs(row.center-(r.top+r.bottom)/2)<1);
        if(!row){row={center:(r.top+r.bottom)/2,left:r.left,right:r.right,top:r.top,bottom:r.bottom};rows.push(row);}
        row.left=Math.min(row.left,r.left);row.right=Math.max(row.right,r.right);row.top=Math.min(row.top,r.top);row.bottom=Math.max(row.bottom,r.bottom);
      }
    }
    for(const math of n.querySelectorAll('.typst-math:not(.math-display)')) {
      const r=math.querySelector('.math-canvas').getBoundingClientRect(),baseline=math.getBoundingClientRect().bottom+parseFloat(getComputedStyle(math).verticalAlign);
      const row=rows.toSorted((a,b)=>Math.abs(a.center-baseline+font*.3)-Math.abs(b.center-baseline+font*.3))[0];
      if(row){row.left=Math.min(row.left,r.left);row.right=Math.max(row.right,r.right);row.top=Math.min(row.top,r.top);row.bottom=Math.max(row.bottom,r.bottom);}
    }
    rows.sort((a,b)=>a.center-b.center);
    const bands=rows.map(r=>({...r,top:Math.min(r.top,r.center-leading/2),bottom:Math.max(r.bottom,r.center+leading/2)}));
    for(let i=1;i<rows.length;i++){const middle=(rows[i-1].bottom+rows[i].top)/2;bands[i-1].bottom=middle;bands[i].top=middle;}
    return {box:{x:Math.floor(bounds.x),y:Math.floor(bounds.y),width:bounds.width,height:bounds.height},dpr:devicePixelRatio,math:[...n.querySelectorAll('.math-canvas')].map(n=>{const r=n.getBoundingClientRect();return {left:r.left,right:r.right,top:r.top,bottom:r.bottom};}),bands,font,paper:getComputedStyle(document.body).backgroundColor,ink:getComputedStyle(n).color,selection:getComputedStyle(document.querySelector('#reading-content')).getPropertyValue('--selection-ink').trim()};
  });
  let metrics=await measure();
  assert(metrics.bands.length>=2,`${label}: genuinely multi-line paragraph`);
  let baseline=await block.screenshot({path:path.join(artifacts,`${label}-before.png`)});
  const assertPixels=async(kind,colour,saved)=>{
    const after=saved||await block.screenshot({path:path.join(artifacts,`${label}-${kind}.png`)});
    const result=await page.evaluate(async({a,b,m,colour})=>{
      const decode=async b=>{const im=new Image();im.src='data:image/png;base64,'+b;await im.decode();const c=document.createElement('canvas');c.width=im.width;c.height=im.height;const ctx=c.getContext('2d');ctx.drawImage(im,0,0);return {w:im.width,h:im.height,data:ctx.getImageData(0,0,im.width,im.height).data};};
      const rgb=s=>s.match(/[\d.]+/g).slice(0,3).map(Number),paper=rgb(m.paper),glyph=rgb(m.ink);
      const c=document.createElement('canvas').getContext('2d');c.fillStyle=colour;c.fillRect(0,0,1,1);const pen=c.getImageData(0,0,1,1).data;
      const before=await decode(a),after=await decode(b),sx=m.dpr,sy=m.dpr,fail=[];let count=0,ink=0;
      for(const band of m.bands)for(let y=Math.max(2,Math.ceil((band.top-m.box.y)*sy)+2);y<Math.min(before.h-2,Math.floor((band.bottom-m.box.y)*sy)-2);y++)for(let x=Math.max(2,Math.ceil((band.left-m.box.x)*sx)+2);x<Math.min(before.w-2,Math.floor((band.right-m.box.x)*sx)-2);x++) {
        const i=(y*before.w+x)*4,old=[...before.data.slice(i,i+3)];let expected;
        if(old.every((v,k)=>v===paper[k])){expected=paper.map((v,k)=>Math.round(v*(1-pen[3]/255)+pen[k]*pen[3]/255));count++;}
        else if(old.every((v,k)=>v===glyph[k])&&[-1,0,1].every(dy=>[-1,0,1].every(dx=>glyph.every((v,k)=>before.data[((y+dy)*before.w+x+dx)*4+k]===v)))){expected=old;ink++;}else continue;
        if(expected.some((v,k)=>Math.abs(v-after.data[i+k])>3)&&fail.length<10)fail.push({x,y,expected,actual:[...after.data.slice(i,i+3)]});
      }
      // Explicitly scan the shared horizontal strip BETWEEN every two rows.
      for(let j=1;j<m.bands.length;j++) {
        const p=m.bands[j-1],q=m.bands[j],left=Math.max(p.left,q.left),right=Math.min(p.right,q.right);
        for(let y=Math.ceil((p.bottom-m.box.y)*sy)-1;y<=Math.ceil((p.bottom-m.box.y)*sy)+1;y++)for(let x=Math.ceil((left-m.box.x)*sx)+3;x<Math.floor((right-m.box.x)*sx)-3;x++) {
          if(y<2||y>=before.h-2)continue;const i=(y*before.w+x)*4;if(!paper.every((v,k)=>before.data[i+k]===v))continue;
          const expected=paper.map((v,k)=>Math.round(v*(1-pen[3]/255)+pen[k]*pen[3]/255));count++;
          if(expected.some((v,k)=>Math.abs(v-after.data[i+k])>3)&&fail.length<10)fail.push({seam:true,x,y});
        }
      }
      // SVG may be composited at a different subpixel raster phase after a
      // native selection. Compare integrated glyph coverage, not a thin edge
      // pixel that happened to be opaque in only one rasterisation. This still
      // rejects glyph tint/overpainting, while exact opaque interiors above and
      // all background/seam pixels remain checked individually.
      const bg=paper.map((v,k)=>v*(1-pen[3]/255)+pen[k]*pen[3]/255);
      const coverage=(value,paper)=>Math.max(0,Math.min(1,(paper-value)/(paper-glyph[1])));
      for(const r of m.math) {
        let old=0,now=0;
        for(let y=Math.max(1,Math.ceil((r.top-m.box.y)*sy)+1);y<Math.min(before.h-1,Math.floor((r.bottom-m.box.y)*sy)-1);y++)for(let x=Math.max(1,Math.ceil((r.left-m.box.x)*sx)+1);x<Math.min(before.w-1,Math.floor((r.right-m.box.x)*sx)-1);x++) {
          const i=(y*before.w+x)*4+1;old+=coverage(before.data[i],paper[1]);now+=coverage(after.data[i],bg[1]);
        }
        if(old>1&&Math.abs(old-now)/old>.025)fail.push({glyphCoverage:true,old,now});
      }
      return {count,ink,fail};
    },{a:baseline.toString('base64'),b:after.toString('base64'),m:metrics,colour});
    assert(result.count>1000,`${label}: meaningful paragraph and line-gap scan`);
    if(result.fail.length) require('node:fs').writeFileSync(path.join(artifacts,`${label}-${kind}-metrics.json`),JSON.stringify({metrics,paths:await page.locator('.reader-highlight-gaps path').evaluateAll(ns=>ns.map(n=>({colour:n.getAttribute('fill'),path:n.getAttribute('d')})))},null,2));
    assert.deepEqual(result.fail,[],`${label} ${kind}: uniform edges, continuous between rows, no glyph tint`);
  };
  const nodes=await block.evaluate(n=>{window.__paragraphNodes=[n,...n.querySelectorAll('*')];return window.__paragraphNodes.map(n=>{const r=n.getBoundingClientRect();return [r.x,r.y,r.width,r.height];});});
  // Real drag across the whole paragraph, including its inline formulas.
  const first=metrics.bands[0],last=metrics.bands.at(-1);
  await page.mouse.move(first.left+1,first.center);await page.mouse.down();
  await page.mouse.move(last.right-1,last.center,{steps:35});await page.mouse.up();
  await page.locator('#selection-toolbar').waitFor({state:'visible'});await page.waitForTimeout(160);
  await page.locator('#selection-toolbar').evaluate(n=>n.style.visibility='hidden');
  assert.deepEqual(await block.evaluate(n=>[n,...n.querySelectorAll('*')].map(n=>{const r=n.getBoundingClientRect();return [r.x,r.y,r.width,r.height];})),nodes,`${label}: drag preserves exact source and formula geometry`);
  await assertPixels('selected',metrics.selection);
  await page.locator('#selection-toolbar').evaluate(n=>n.style.visibility='');
  await page.locator('[data-marker="0"]').click();await page.locator('#selection-toolbar').waitFor({state:'hidden'});await page.waitForTimeout(160);
  await assertPixels('yellow','#f2c43859');
  assert(await block.evaluate(n=>[n,...n.querySelectorAll('*')].every((v,i)=>v===window.__paragraphNodes[i])));
  assert.deepEqual(await block.evaluate(n=>[n,...n.querySelectorAll('*')].map(n=>{const r=n.getBoundingClientRect();return [r.x,r.y,r.width,r.height];})),nodes);
  await page.reload();await page.locator('body[data-browser-ready="true"]').waitFor();await block.evaluate(n=>n.scrollIntoView({block:'center'}));await page.waitForTimeout(160);
  metrics=await measure();const reloaded=await block.screenshot({path:path.join(artifacts,`${label}-reloaded.png`)});
  await block.evaluate(n=>{const r=document.createRange();r.selectNodeContents(n);getSelection().removeAllRanges();getSelection().addRange(r);});
  await page.locator('#selection-toolbar').waitFor({state:'visible'});await page.locator('#toggle-highlight').click();await page.locator('#selection-toolbar').waitFor({state:'hidden'});await page.waitForTimeout(160);
  baseline=await block.screenshot({path:path.join(artifacts,`${label}-removed.png`)});
  await assertPixels('reloaded','#f2c43859',reloaded);
  console.log(`PASS: ${label}: full paragraph drag/save/reload/remove, uniform formula/prose edges and every line seam`);
};
