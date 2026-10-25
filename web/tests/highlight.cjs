// Pixel + real browser regression: font boxes alone miss line leading/code pads.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawn, execFileSync } = require("node:child_process");
const { pathToFileURL } = require("node:url");
const { chromium } = require(process.env.READWISE_PLAYWRIGHT_MODULE || "playwright-core");
const root = path.resolve(__dirname, "../..");
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "readwise-highlights-"));
const artifacts = path.join(root, "target/web-qa/highlights"); fs.mkdirSync(artifacts, { recursive: true });
const fixture = path.join(temporary, "Highlight.epub");
execFileSync("python3", [path.join(__dirname, "highlight-fixture.py"), fixture]);
let server, browser, logs = "", base;
const errors = [];
async function start() {
  return new Promise((resolve, reject) => {
    server = spawn(process.env.READWISE_WEB_BINARY || path.join(root, "target/debug/readwise-web"), [], { cwd: root, env: { ...process.env, READWISE_WEB_BIND: "127.0.0.1:0", READWISE_WEB_DATA_DIR: path.join(temporary, "data") }, stdio: ["ignore", "pipe", "pipe"] });
    const timer = setTimeout(() => reject(new Error("Startup timed out")), 25000); let current = "";
    const log = chunk => { logs += chunk; current += chunk; const match = current.match(/ReadWise Web: (http:\/\/[^\s]+)\n/); if (match) { clearTimeout(timer); resolve(match[1]); } };
    server.stdout.on("data", log); server.stderr.on("data", log); server.on("error", reject);
  });
}
async function stop() {
  if (!server || server.exitCode !== null) return;
  await new Promise((resolve, reject) => { const timer = setTimeout(() => { server.kill("SIGKILL"); reject(new Error("Shutdown timed out")); }, 10000); server.once("exit", code => { clearTimeout(timer); code === 0 ? resolve() : reject(new Error(`Exit ${code}`)); }); server.kill("SIGINT"); }); server = null;
}
async function run() {
  const { lineBands, intersect, inlineFragment } = await import(pathToFileURL(path.join(root, "web/src/highlight.mjs")));
  const box = (left, top, right, bottom) => ({ left, top, right, bottom });
  assert.deepEqual(intersect(box(0,0,100,100), box(20,30,60,70)), box(20,30,60,70));
  assert.equal(intersect(box(0,0,100,100), box(110,110,200,200)), null);
  assert.deepEqual(inlineFragment(box(10,4,90,28), box(0,0,100,32), true, true), box(0,0,100,32));
  assert.deepEqual(inlineFragment(box(10,4,90,28), box(0,0,100,32), true, false), box(0,0,90,32));
  assert.deepEqual(inlineFragment(box(10,4,90,28), box(0,0,100,32), false, true), box(10,0,100,32));
  assert.deepEqual(inlineFragment(box(10,4,90,28), box(0,0,100,32), false, false), box(10,0,90,32));
  const fragments = [{ ...box(0,12,100,47), font: 30, height: 60 }, { ...box(110,17,160,46), font: 25.5, height: 51 }, { ...box(170,12,300,47), font: 30, height: 60 }, { ...box(0,85,300,120), font: 30, height: 60 }];
  const bands = lineBands(fragments); assert.equal(bands.length, 2); assert.equal(bands[0].right, 300); assert.ok(bands[0].bottom >= bands[1].top);
  console.log("PASS: clipping, inline boundaries and mixed-font/line-leading geometry units");
  base = await start();
  const executablePath = process.env.READWISE_CHROMIUM || (process.platform === "darwin" ? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" : undefined);
  browser = await chromium.launch({ headless: true, ...(executablePath ? { executablePath } : {}) });
  let page = await browser.newPage({ viewport: { width: 1280, height: 900 } }); page.on("pageerror", error => errors.push(error.message));
  const ready = () => page.locator('body[data-browser-ready="true"]').waitFor();
  const settle = () => page.waitForTimeout(300);
  const prefs = value => page.request.post(`${base}/api/preferences`, { headers: { "x-readwise-request": "1" }, data: value });
  const sample = async (name, points) => {
    const screenshot = await page.screenshot({ path: path.join(artifacts, `${name}.png`), scale: "css" });
    return page.evaluate(async ({ png, points }) => {
      const image = new Image(); image.src = `data:image/png;base64,${png}`; await image.decode();
      const canvas = document.createElement("canvas"); canvas.width = image.width; canvas.height = image.height;
      const context = canvas.getContext("2d"); context.drawImage(image, 0, 0);
      return Object.fromEntries(Object.entries(points).map(([name, point]) => [name, [...context.getImageData(Math.floor(point.x), Math.floor(point.y), 1, 1).data].slice(0, 3)]));
    }, { png: screenshot.toString("base64"), points });
  };
  const blended = (original, ink, alpha) => original.map((c, i) => Math.round(c * (1-alpha) + ink[i]*alpha));
  const near = (actual, expected, name) => actual.forEach((c, i) => assert.ok(Math.abs(c - expected[i]) <= 3, `${name}: ${actual} != ${expected}`));
  const select = async () => {
    await page.evaluate(() => { const range = document.createRange(); range.selectNodeContents(document.querySelector("#content-flow")); getSelection().removeAllRanges(); getSelection().addRange(range); });
    await page.locator("#reading-content[data-selection-paint]").waitFor(); await settle();
  };
  const geometry = () => page.evaluate(() => ({ scrollX, scrollY, blocks: [...document.querySelectorAll("[data-block]")].map(node => { const r = node.getBoundingClientRect(); return [r.x,r.y,r.width,r.height]; }) }));
  const identity = () => page.evaluate(() => {
    const nodes = []; const walk = document.createTreeWalker(document.querySelector("#reading-content"), NodeFilter.SHOW_ALL); while (walk.nextNode()) nodes.push(walk.currentNode);
    return nodes.length === window.__nodes.length && nodes.every((node, i) => node === window.__nodes[i]);
  });
  await page.goto(base); await ready();
  assert.equal((await prefs({ language: "zh-CN", theme: "light", background: "original", english_level: "C3", font: "georgia", font_size: 30, spacing: 2, turn_mode: "scroll", columns: "one", selection_color_slot: 6, text_width: "narrow" })).status(), 200);
  await page.locator("#book-upload").setInputFiles(fixture); await page.locator('#import-results [data-state="success"] a').click(); await ready(); await settle();
  const bookId = await page.locator("#reading-content").getAttribute("data-book-id");
  const url = `${base}/read/${bookId}/0`;
  const points = await page.evaluate(() => {
    const p = document.querySelector("#content-flow"), first = p.firstChild, code = p.querySelector("code").firstChild, after = p.querySelector("code").nextSibling;
    const rectangles = node => { const r = document.createRange(); r.selectNodeContents(node); return [...r.getClientRects()]; };
    const a = rectangles(first)[0], c = rectangles(code)[0], rest = rectangles(after), b = rest.find(r => r.top > a.bottom);
    const space = document.createRange(); space.setStart(first, 3); space.setEnd(first, 4); const s = space.getBoundingClientRect();
    const walker = document.createTreeWalker(document.querySelector("#reading-content"), NodeFilter.SHOW_ALL); window.__nodes = []; while (walker.nextNode()) window.__nodes.push(walker.currentNode);
    const points = { leading: { x: Math.max(a.left, b.left) + 8, y: (a.bottom + b.top) / 2 }, left: { x: (a.right+c.left)/2, y: (c.top+c.bottom)/2 }, right: { x: (c.right+rest[0].left)/2, y: (c.top+c.bottom)/2 }, space: { x: (s.left+s.right)/2, y: (s.top+s.bottom)/2 } };
    for (const [name, left, right] of [["left-edge", a.right-1, c.left+1], ["right-edge", c.right-1, rest[0].left+1]]) {
      for (let x = Math.floor(left); x < Math.ceil(right); x++) points[`${name}-${x}`] = { x: x+.5, y: Math.floor(c.bottom)-2.5 };
    }
    return points;
  });
  const initialGeometry = await geometry(), before = await sample("before", points);
  await select();
  const selected = await sample("selected", points);
  for (const key of Object.keys(points)) near(selected[key], blended(before[key], [59,122,61], 136/255), `selection ${key}`);
  assert.deepEqual(await geometry(), initialGeometry); assert.equal(await identity(), true);
  assert.equal(await page.locator("#content-flow").textContent(), await page.evaluate(() => { const clone = getSelection().getRangeAt(0).cloneContents(); clone.querySelectorAll("rt,rp").forEach(n => n.remove()); return clone.textContent; }));
  console.log("PASS: live selection pixel coverage joins line leading and both inline-code pads, without dark seams or changing source/geometry");
  await page.locator('[data-marker="1"]').click(); await page.locator("#selection-toolbar").waitFor({ state: "hidden" }); await settle();
  const savedMark = (await page.request.get(`${base}/api/highlights/${bookId}/0`).then(r => r.json()))[0];
  const marked = await sample("saved", points);
  for (const key of Object.keys(points)) near(marked[key], blended(before[key], [124,198,111], 89/255), `saved ${key}`);
  assert.deepEqual(await geometry(), initialGeometry); assert.equal(await identity(), true);
  await page.reload(); await ready(); await settle();
  const reloaded = await sample("reloaded", points); for (const key of Object.keys(points)) near(reloaded[key], marked[key], `reload ${key}`);
  await select(); await page.locator("#toggle-highlight").click(); await page.locator("#selection-toolbar").waitFor({ state: "hidden" }); await settle();
  const removed = await sample("removed", points); for (const key of Object.keys(points)) near(removed[key], before[key], `removed ${key}`);
  console.log("PASS: saved marks cover the same gaps, persist after reload, remove cleanly and retain every text node");

  // Partial inline code selection must not colour the unselected prefix/suffix.
  await page.evaluate(() => { const p = document.querySelector("#content-flow"), r = document.createRange(); r.setStart(p.firstChild, 4); r.setEnd(p.querySelector("code").firstChild, 2); getSelection().addRange(r); });
  await settle(); const partial = await sample("partial", points);
  near(partial.space, before.space, "unselected prefix"); near(partial.right, before.right, "unselected code suffix");
  near(partial.left, blended(before.left, [59,122,61], 136/255), "selected left code pad");
  await page.keyboard.press("Escape");
  console.log("PASS: partial code selection paints only the covered boundary, not the unselected prefix or suffix");
  for (const color of [3, 0, 0]) {
    const response = await page.request.post(`${base}/api/highlights`, { headers: { "x-readwise-request": "1" }, data: { book_id: bookId, chapter: 0, start: savedMark.start, end: savedMark.end, color } }); assert.equal(response.status(), 200);
  }
  await page.reload(); await ready(); await settle(); const overlapping = await sample("overlapping", points);
  for (const key of Object.keys(points)) near(overlapping[key], blended(blended(before[key], [242,196,56], 89/255), [240,132,172], 89/255), `overlap ${key}`);
  const ids = await page.request.get(`${base}/api/highlights/${bookId}/0`).then(r => r.json()).then(rows => rows.map(row => row.id));
  assert.equal((await page.request.post(`${base}/api/highlights/remove`, { headers: { "x-readwise-request": "1" }, data: { book_id: bookId, chapter: 0, ids } })).status(), 200);
  await page.reload(); await ready(); await settle();
  console.log("PASS: overlapping colours use the same priority as CSS highlights and duplicate same-colour ranges do not double-darken gaps");

  await prefs({ turn_mode: "slide", columns: "two", text_width: "wide", font_size: 28 });
  await page.goto(`${url}?qa=paged#web-block-0`); await ready(); await settle(); await select();
  await page.locator("#toggle-highlight").click(); await settle();
  await page.locator("#next-page").click(); await settle();
  const pageGeometry = await geometry();
  assert.equal(await page.locator("#reading-content").getAttribute("data-page"), "1");
  assert.equal(await page.locator("#reading-content").evaluate(node => getComputedStyle(node).columnCount), "2");
  const gutter = await page.evaluate(() => { const v = document.querySelector("#reader-viewport").getBoundingClientRect(); return { x: (v.left+v.right)/2, y: v.top+80 }; });
  const gutterColour = await sample("two-columns", { gutter }); near(gutterColour.gutter, [255,255,255], "column gutter must remain clear");
  await page.setViewportSize({ width: 320, height: 568 }); await settle();
  await page.screenshot({ path: path.join(artifacts, "portrait.png") });
  assert.equal(await page.locator("#reading-content").evaluate(node => getComputedStyle(node).columnCount), "1");
  await page.setViewportSize({ width: 568, height: 320 }); await settle();
  const clipped = await page.evaluate(() => {
    const v = document.querySelector("#reader-viewport").getBoundingClientRect();
    return [...document.querySelectorAll(".reader-highlight-gaps path")].every(path => { const r = path.getBoundingClientRect(); return !r.width || r.left >= v.left-1 && r.right <= v.right+1 && r.top >= v.top-1 && r.bottom <= v.bottom+1; });
  }); assert.equal(clipped, true);
  await page.screenshot({ path: path.join(artifacts, "landscape.png") });
  assert.ok(pageGeometry.blocks.length > 0);
  console.log("PASS: later-page two-column projection leaves the gutter clear; rotation clips the paint to the actual reader viewport");
  const remaining = await page.request.get(`${base}/api/highlights/${bookId}/0`).then(r => r.json()).then(rows => rows.map(row => row.id));
  await page.request.post(`${base}/api/highlights/remove`, { headers: { "x-readwise-request": "1" }, data: { book_id: bookId, chapter: 0, ids: remaining } });
  await prefs({ background: "night", turn_mode: "scroll", columns: "one", text_width: "narrow", font_size: 30 });
  await page.close(); page = await browser.newPage({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: 2 }); page.on("pageerror", error => errors.push(error.message));
  await page.goto(`${url}?qa=retina#web-block-0`); await ready(); await settle();
  const darkBefore = await sample("retina-dark-before", points);
  await select(); const darkSelected = await sample("retina-dark-selected", points);
  for (const key of Object.keys(points)) near(darkSelected[key], blended(darkBefore[key], [59,122,61], 136/255), `retina selection ${key}`);
  await page.locator("#toggle-highlight").click(); await page.locator("#selection-toolbar").waitFor({ state: "hidden" }); await settle();
  const darkSaved = await sample("retina-dark-saved", points);
  for (const key of Object.keys(points)) near(darkSaved[key], blended(darkBefore[key], [124,198,111], 89/255), `retina saved ${key}`);
  await page.evaluate(() => scrollBy(0, 100)); await settle();
  const movedPoints = Object.fromEntries(Object.entries(points).map(([key, p]) => [key, { x: p.x, y: p.y-100 }]));
  const scrolled = await sample("retina-dark-scrolled", movedPoints);
  for (const key of Object.keys(points)) near(scrolled[key], darkSaved[key], `scroll projection ${key}`);
  console.log("PASS: DPR 2/night selection and persisted marks have no inline seams, and continuous scrolling keeps pixel coverage aligned");
  await prefs({ turn_mode: "slide" }); await page.goto(`${url}?qa=code`); await ready(); await settle();
  await page.locator("#reading-content pre").evaluate(node => { location.hash = `web-block-${node.closest('[data-block]').dataset.block}`; }); await settle();
  const codePoint = () => page.evaluate(() => {
    const pre = document.querySelector("#reading-content pre"), box = pre.getBoundingClientRect();
    const walker = document.createTreeWalker(pre, NodeFilter.SHOW_TEXT), rows = [];
    while (walker.nextNode()) { const range = document.createRange(); range.selectNodeContents(walker.currentNode); for (const r of range.getClientRects()) if (r.width && r.top > box.top+20 && r.bottom < box.bottom-20) rows.push(r); }
    rows.sort((a,b) => a.top-b.top); return { code: { x: rows[0].left+8, y: (rows[0].bottom+rows.find(r => r.top>rows[0].bottom).top)/2 } };
  });
  const codePixels = await codePoint(), codeBefore = await sample("code-before", codePixels);
  await page.locator("#reading-content pre").evaluate(node => { const range = document.createRange(); range.selectNodeContents(node); getSelection().removeAllRanges(); getSelection().addRange(range); });
  await page.locator("#selection-toolbar").waitFor({ state: "visible" }); await page.locator("#toggle-highlight").click(); await settle();
  const codeMarked = await sample("code-marked", codePixels); near(codeMarked.code, blended(codeBefore.code, [124,198,111], 89/255), "code leading");
  await page.locator("#reading-content pre").evaluate(node => { node.scrollTop = 100; }); await settle();
  const codeScrolled = await sample("code-scrolled", await codePoint()); near(codeScrolled.code, codeMarked.code, "internal code scroll");
  assert.equal(await page.locator(".reader-highlight-gaps path").evaluateAll(paths => { const pre = document.querySelector("#reading-content pre").getBoundingClientRect(); return paths.every(path => { const r = path.getBoundingClientRect(); return !r.width || r.left>=pre.left-1 && r.right<=pre.right+1 && r.top>=pre.top-1 && r.bottom<=pre.bottom+1; }); }), true);
  console.log("PASS: paged code leading stays clipped to its own scroll container and follows internal scrolling");
  // Border/padding boxes, not just the gaps between adjacent text runs. Compact
  // leading and code-only/line-initial runs expose the missing top/bottom/left.
  for (const [dpr, fontSize, spacing] of [[1, 40, 1.1], [2, 40, 1.1], [2, 30, 1.55]]) {
    await prefs({ background: "original", turn_mode: "scroll", columns: "one", font_size: fontSize, spacing });
    await page.close(); page = await browser.newPage({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: dpr }); page.on("pageerror", error => errors.push(error.message));
    await page.goto(`${url}?qa=borders-${dpr}-${fontSize}`); await ready(); await settle();
    for (const name of ["code-leading", "code-alone", "code-break", "code-wrap", "code-nested"]) {
      const selector = `#content-${name}`;
      await page.locator(selector).evaluate(node => node.scrollIntoView({ block: "center" })); await settle();
      const borderPoints = await page.locator(selector).evaluate(p => {
        const points = {};
        [...p.querySelectorAll("code")].forEach((code, ci) => {
          const glyphs = [], walker = document.createTreeWalker(code, NodeFilter.SHOW_TEXT);
          while (walker.nextNode()) { const r = document.createRange(); r.selectNodeContents(walker.currentNode); glyphs.push(...r.getClientRects()); }
          [...code.getClientRects()].forEach((r, ri) => {
            const add = (x, y, side) => {
              if (glyphs.some(g => x >= g.left && x < g.right && y >= g.top && y < g.bottom)) return;
              if (y < 70 || y >= innerHeight-10) return;
              points[`${ci}-${ri}-${side}-${x}-${y}`] = { x, y };
            };
            for (let x = Math.ceil(r.left)+5; x < Math.floor(r.right)-4; x++) { add(x+.5, Math.ceil(r.top)+.5, "top"); add(x+.5, Math.floor(r.bottom)-.5, "bottom"); }
            for (let y = Math.ceil(r.top)+5; y < Math.floor(r.bottom)-4; y++) { add(Math.ceil(r.left)+.5, y+.5, "left"); add(Math.floor(r.right)-.5, y+.5, "right"); }
          });
        });
        return points;
      });
      assert.ok(Object.keys(borderPoints).some(key => key.includes("top"))); assert.ok(Object.keys(borderPoints).some(key => key.includes("bottom")));
      if (name !== "code-wrap") assert.ok(Object.keys(borderPoints).some(key => key.startsWith("0-0-left")));
      const snapshot = await geometry();
      await page.evaluate(() => { window.__nodes = []; const walker = document.createTreeWalker(document.querySelector("#reading-content"), NodeFilter.SHOW_ALL); while (walker.nextNode()) window.__nodes.push(walker.currentNode); });
      const prefix = `border-${name}-${dpr}-${fontSize}`, unmarked = await sample(`${prefix}-before`, borderPoints);
      await page.locator(selector).evaluate(node => { const r = document.createRange(); r.selectNodeContents(node); getSelection().removeAllRanges(); getSelection().addRange(r); });
      await page.locator("#reading-content[data-selection-paint]").waitFor(); await settle();
      await page.locator("#selection-toolbar").evaluate(node => { node.style.visibility = "hidden"; }); // exclude the popup's legitimate shadow from pixel comparisons
      const selected = await sample(`${prefix}-selected`, borderPoints);
      await page.locator("#selection-toolbar").evaluate(node => { node.style.visibility = ""; });
      for (const key of Object.keys(borderPoints)) near(selected[key], blended(unmarked[key], [59,122,61], 136/255), `${prefix} selection ${key}`);
      await page.locator("#toggle-highlight").click(); await page.locator("#selection-toolbar").waitFor({ state: "hidden" }); await settle();
      const marked = await sample(`${prefix}-saved`, borderPoints);
      for (const key of Object.keys(borderPoints)) near(marked[key], blended(unmarked[key], [124,198,111], 89/255), `${prefix} saved ${key}`);
      assert.deepEqual(await geometry(), snapshot); assert.equal(await identity(), true);
      await page.locator(selector).evaluate(node => { const r = document.createRange(); r.selectNodeContents(node); getSelection().addRange(r); }); await settle();
      await page.locator("#toggle-highlight").click(); await page.locator("#selection-toolbar").waitFor({ state: "hidden" }); await settle();
      if (name === "code-alone" && fontSize === 40) for (const suffix of [false, true]) {
        const partialRange = async () => page.locator(selector).evaluate((p, suffix) => {
          const code = p.querySelector("code"), node = code.firstChild, range = document.createRange();
          range.setStart(node, suffix ? 5 : 0); range.setEnd(node, suffix ? node.length : 5);
          getSelection().removeAllRanges(); getSelection().addRange(range);
          const r = range.getBoundingClientRect(), b = code.getBoundingClientRect();
          return { left: suffix ? r.left : b.left, right: suffix ? b.right : r.right };
        }, suffix);
        const bounds = await partialRange(); await settle();
        const check = (colours, ink, alpha, label) => {
          for (const [key, point] of Object.entries(borderPoints)) {
            if (point.x > bounds.left+1 && point.x < bounds.right-1) near(colours[key], blended(unmarked[key], ink, alpha), `${prefix} ${label} selected ${key}`);
            else if (point.x < bounds.left-1 || point.x > bounds.right+1) near(colours[key], unmarked[key], `${prefix} ${label} unselected ${key}`);
          }
        };
        await page.locator("#selection-toolbar").evaluate(node => { node.style.visibility = "hidden"; });
        check(await sample(`${prefix}-${suffix ? "suffix" : "prefix"}-selected`, borderPoints), [59,122,61], 136/255, "partial selection");
        await page.locator("#selection-toolbar").evaluate(node => { node.style.visibility = ""; });
        await page.locator("#toggle-highlight").click(); await page.locator("#selection-toolbar").waitFor({ state: "hidden" }); await settle();
        check(await sample(`${prefix}-${suffix ? "suffix" : "prefix"}-saved`, borderPoints), [124,198,111], 89/255, "partial saved");
        assert.deepEqual(await geometry(), snapshot); assert.equal(await identity(), true);
        await partialRange(); await settle(); await page.locator("#toggle-highlight").click(); await page.locator("#selection-toolbar").waitFor({ state: "hidden" }); await settle();
      }
    }
  }
  console.log("PASS: full inline-code border strips, line-initial/standalone/wrapped/nested code, compact/normal leading and DPR 1/2, with unchanged source geometry");
  assert.deepEqual(errors, []); console.log(`Highlight screenshots: ${artifacts}`);
}
run().catch(error => { console.error(error); process.exitCode = 1; }).finally(async () => {
  if (browser) await browser.close(); try { await stop(); } catch (error) { console.error(error); process.exitCode = 1; }
  fs.writeFileSync(path.join(artifacts, "server.log"), logs); fs.rmSync(temporary, { recursive: true, force: true });
});
