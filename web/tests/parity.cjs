// Native-parity regression. Owned books, Web DB, fake read-only desktop DB and
// a loopback OpenAI mock only: never inspect real desktop data or paid services.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const http = require("node:http");
const { spawn, execFileSync } = require("node:child_process");
const { chromium } = require(process.env.READWISE_PLAYWRIGHT_MODULE || "playwright-core");
const root = path.resolve(__dirname, "../..");
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "readwise-parity-"));
const artifacts = path.join(root, "target/web-qa/parity");
fs.mkdirSync(artifacts, { recursive: true });
const fixture = path.join(temporary, "森林.epub");
execFileSync("python3", [path.join(__dirname, "fixture.py"), fixture]);
const desktop = path.join(temporary, "desktop/database/app.db");
execFileSync("python3", ["-c", `import sqlite3,sys,pathlib
p=pathlib.Path(sys.argv[1]);p.parent.mkdir(parents=True)
c=sqlite3.connect(p);c.execute('create table app_settings(key text primary key,value text,updated_at integer)')
v={'language':'en','english_level':'B2','theme':'light','background':'sage','turn_mode':'slide','columns':'two','font':'seravek','spacing':'1.83','font_size':'13.25','gloss_font_size':'9.55','text_width':'wide','justify_text':'1','indent_paragraphs':'1','gloss_color_slot':'2','gloss_background_slot':'3','word_color_slot':'1','word_background_slot':'2','selection_color_slot':'6','highlight_color_slot':'6'}
c.executemany('insert into app_settings values (?,?,0)', [('reader.'+k,v) for k,v in v.items()]);c.commit();c.close()`, desktop]);
const desktopBefore = fs.readFileSync(desktop);
let server, browser, mock, base, logs = "";
const failures = [], aiRequests = [];
async function start() {
  return new Promise((resolve, reject) => {
    server = spawn(process.env.READWISE_WEB_BINARY || path.join(root, "target/debug/readwise-web"), [], { cwd: root, env: { ...process.env, READWISE_WEB_BIND: "127.0.0.1:0", READWISE_WEB_DATA_DIR: path.join(temporary, "web"), READWISE_WEB_DESKTOP_DATA_DIR: path.dirname(path.dirname(desktop)) }, stdio: ["ignore", "pipe", "pipe"] });
    const timer = setTimeout(() => reject(new Error("Startup timed out")), 20000);
    let startup = "";
    const log = chunk => { logs += chunk; startup += chunk; const match = startup.match(/ReadWise Web: (http:\/\/[^\s]+)\n/); if (match) { clearTimeout(timer); resolve(match[1]); } };
    server.stdout.on("data", log); server.stderr.on("data", log); server.on("error", reject);
  });
}
async function stop() {
  if (!server || server.exitCode !== null) return;
  await new Promise((resolve, reject) => { const timer = setTimeout(() => { server.kill("SIGKILL"); reject(new Error("Shutdown timed out")); }, 10000); server.once("exit", code => { clearTimeout(timer); code === 0 ? resolve() : reject(new Error(`Exit ${code}`)); }); server.kill("SIGINT"); });
  server = null;
}
async function run() {
  mock = http.createServer(async (request, response) => {
    const chunks = []; for await (const chunk of request) chunks.push(chunk);
    aiRequests.push({ body: JSON.parse(Buffer.concat(chunks)), authorization: request.headers.authorization, url: request.url });
    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify({ choices: [{ message: { content: "QA answer: 原文解释 <script>not executable</script>" } }] }));
  });
  await new Promise(resolve => mock.listen(0, "127.0.0.1", resolve));
  base = await start();
  const executablePath = process.env.READWISE_CHROMIUM || (process.platform === "darwin" ? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" : undefined);
  browser = await chromium.launch({ headless: true, ...(executablePath ? { executablePath } : {}) });
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, permissions: ["clipboard-read", "clipboard-write"] });
  const page = await context.newPage();
  page.on("pageerror", e => failures.push(e.message));
  page.on("console", message => { if (message.type() === "error" && !message.text().startsWith("Failed to load resource")) failures.push(message.text()); });
  const ready = async () => {
    try { await page.locator('body[data-browser-ready="true"]').waitFor({ timeout: 10000 }); }
    catch (error) { console.error("Browser initialization:", failures, await page.locator("body").innerText()); throw error; }
  };
  const settle = async () => page.waitForTimeout(300);
  const preferences = () => page.request.get(`${base}/api/preferences`).then(r => r.json());
  const get = route => page.request.get(`${base}${route}`).then(r => r.json());
  const geometry = () => page.evaluate(() => ({ page: document.querySelector("#reading-content").dataset.page, scroll: scrollY, horizontal: scrollX, blocks: [...document.querySelectorAll("[data-block]")].map(node => { const r = node.getBoundingClientRect(); return [r.x,r.y,r.width,r.height]; }) }));
  const utilities = async () => { if (!await page.locator("#selection-more").isVisible()) await page.locator("#more-selection").click(); };
  const selectText = async (single = false) => {
    const text = await page.evaluate(single => {
      const paragraphs = [...document.querySelectorAll(".reader-block p")].slice(0, 2);
      const range = document.createRange();
      if (single) range.selectNodeContents(paragraphs[0].querySelector(".word"));
      else { range.setStart(paragraphs[0].firstChild, 0); range.setEnd(paragraphs[1].lastChild, paragraphs[1].lastChild.length); }
      getSelection().removeAllRanges(); getSelection().addRange(range);
      const clone = range.cloneContents(); clone.querySelectorAll("rt,rp").forEach(node => node.remove());
      return single ? clone.textContent : paragraphs.map(node => { const copy = node.cloneNode(true); copy.querySelectorAll("rt,rp").forEach(rt => rt.remove()); return copy.textContent; }).join("\n\n");
    }, single);
    await page.locator("#selection-toolbar").waitFor({ state: "visible" }); return text;
  };

  await page.goto(`${base}/settings`); await ready();
  const original = await preferences();
  for (const field of ["language", "english_level", "theme", "background", "turn_mode", "columns", "font", "spacing", "font_size", "gloss_size"]) assert.ok(await page.locator(`[name="${field}"]`).count(), field);
  await page.locator("[data-import-desktop]").click();
  await page.getByText("已载入桌面设置草稿，点击应用后保存", { exact: true }).waitFor();
  assert.equal(await page.locator('[name="font"]').inputValue(), "seravek");
  assert.deepEqual(await preferences(), original, "loading a desktop draft must not save it");
  await page.goto(base); await page.goto(`${base}/settings`); await ready();
  assert.equal(await page.locator('[name="font"]').inputValue(), original.font);
  await page.locator("[data-import-desktop]").click();
  await page.getByText("已载入桌面设置草稿，点击应用后保存", { exact: true }).waitFor();
  await page.locator("[data-save-appearance]").click();
  await page.waitForFunction(() => document.documentElement.lang === "en");
  await page.getByRole("heading", { name: "Reading Settings", exact: true }).waitFor();
  const imported = await preferences();
  assert.equal(imported.background, "sage"); assert.equal(imported.theme, "light"); assert.equal(imported.font_size, 26.5);
  assert.equal(imported.font, "seravek"); assert.equal(imported.turn_mode, "slide"); assert.equal(imported.columns, "two");
  assert.equal(imported.spacing, 1.83); assert.ok(Math.abs(imported.gloss_size - 13.37) < 0.001);
  assert.equal(imported.gloss_background, 3); assert.equal(imported.selection_color_slot, 6); assert.equal(imported.highlight_color_slot, 6);
  assert.deepEqual(fs.readFileSync(desktop), desktopBefore, "desktop database bytes must remain unchanged");
  await page.screenshot({ path: path.join(artifacts, "global-settings-english.png"), fullPage: true });
  await page.locator('[name="language"][value="zh-CN"]').check();
  await page.locator('[name="english_level"]').selectOption("A1");
  await page.locator("[data-save-appearance]").click(); await page.waitForFunction(() => document.documentElement.lang === "zh-CN");
  console.log("PASS: full global controls, read-only desktop draft/apply/discard, native units, English/Chinese and no desktop writes");

  await page.goto(base); await ready(); await page.locator("#book-upload").setInputFiles(fixture);
  await page.locator('#import-results [data-state="success"] a').waitFor(); await page.locator('#import-results [data-state="success"] a').click(); await ready(); await settle();
  const bookId = await page.locator("#reading-content").getAttribute("data-book-id");
  const readingURL = `${base}/read/${bookId}/0`;
  assert.equal(await page.locator("#reading-content").getAttribute("data-turn-mode"), "slide");
  assert.equal(await page.locator("#reading-content").evaluate(node => getComputedStyle(node).columnCount), "2");
  assert.ok(Number(await page.locator("#reading-content").getAttribute("data-pages")) > 5);
  await page.evaluate(() => { window.__article = document.querySelector("#reading-content"); });
  await page.locator("#next-page").click(); await page.waitForFunction(() => document.querySelector("#reading-content").dataset.page === "1"); await settle();
  const visibleRuby = await page.locator('ruby[data-lemma="solitude"]').evaluateAll(nodes => nodes.findIndex(node => { const r = node.getBoundingClientRect(), v = document.querySelector("#reader-viewport").getBoundingClientRect(); return r.left >= v.left && r.right <= v.right && r.top > v.top && r.bottom < v.bottom; }));
  assert.ok(visibleRuby >= 0);
  const rubyPoint = await page.locator('ruby[data-lemma="solitude"]').nth(visibleRuby).evaluate(node => { const r = node.getBoundingClientRect(); const x = r.x + r.width / 2, y = r.y + r.height / 2; return { x, y, hit: document.elementFromPoint(x, y)?.closest('ruby') === node }; });
  assert.equal(rubyPoint.hit, true, "later-page word must be physically hittable");
  await page.mouse.click(rubyPoint.x, rubyPoint.y); await page.locator("#word-card[open]").waitFor();
  const beforeKnown = await geometry(); await page.locator("#mark-known").click(); await settle();
  assert.deepEqual(await geometry(), beforeKnown); assert.equal(await page.evaluate(() => window.__article === document.querySelector("#reading-content")), true);
  await page.locator("#previous-page").click(); await settle();
  assert.equal(await page.locator("#reading-content").getAttribute("data-page"), "0");
  await page.screenshot({ path: path.join(artifacts, "reader-paged.png") });
  console.log("PASS: real two-column page turns and later-page mark-known keep DOM, pagination and geometry");

  const selected = await selectText();
  assert.equal(await page.locator("#selection-toolbar").evaluate(node => getComputedStyle(node).backgroundColor), "rgb(48, 38, 23)");
  for (const name of ["问 AI", "解释", "释义"]) assert.equal(await page.getByRole("button", { name, exact: true }).isVisible(), true);
  assert.equal(await page.locator(".selection-colour").count(), 4);
  await page.screenshot({ path: path.join(artifacts, "native-toolbar.png") });
  const beforeHighlight = await geometry();
  await page.locator('[data-marker="3"]').click(); await page.waitForFunction(() => document.querySelector('[data-marker="3"]').getAttribute("aria-pressed") === "true");
  assert.equal((await preferences()).highlight_color_slot, 7);
  await page.locator("#selection-toolbar").waitFor({ state: "hidden" });
  await page.waitForFunction(() => CSS.highlights.get("rw-3")?.size === 1);
  assert.deepEqual(await geometry(), beforeHighlight);
  const persisted = await get(`/api/highlights/${bookId}/0`); assert.equal(persisted.length, 1); assert.equal(persisted[0].color, 3);
  await page.reload(); await ready(); await settle(); await page.waitForFunction(() => CSS.highlights.get("rw-3")?.size === 1);
  await selectText(); await page.locator("#toggle-highlight").click(); await page.locator("#selection-toolbar").waitFor({ state: "hidden" });
  assert.equal((await get(`/api/highlights/${bookId}/0`)).length, 0);
  await selectText(); await page.route("**/api/highlights", route => route.abort()); await page.locator("#toggle-highlight").click();
  await page.locator("#selection-hint").filter({ hasText: "高亮保存失败" }).waitFor();
  assert.equal(await page.evaluate(() => getSelection().isCollapsed), false);
  await page.unroute("**/api/highlights"); await page.locator("#toggle-highlight").click(); await page.locator("#selection-toolbar").waitFor({ state: "hidden" });
  console.log("PASS: native-style toolbar, persisted colour, paint-only cross-block highlights, reload/remove and failed-save selection preservation");

  for (const colour of [0, 1, 2, 3, 3]) {
    await selectText(); const snapshot = await geometry();
    await page.locator(`[data-marker="${colour}"]`).click(); await page.locator("#selection-toolbar").waitFor({ state: "hidden" }); await settle();
    const marks = await get(`/api/highlights/${bookId}/0`);
    assert.equal(marks.length, 1); assert.equal(marks[0].color, colour);
    assert.equal((await preferences()).highlight_color_slot, colour+4);
    assert.equal(await page.evaluate(colour => CSS.highlights.get(`rw-${colour}`).size, colour), 1);
    assert.deepEqual(await geometry(), snapshot);
  }
  // Preference failure and annotation failure both retain the original pink
  // mark and the quote. A successful preference is not a false saved highlight.
  for (const endpoint of ["marker", "highlights/apply"]) {
    await selectText(); await page.route(`**/api/${endpoint}`, route => route.abort());
    await page.locator('[data-marker="1"]').click();
    await page.locator("#selection-hint").filter({ hasText: "高亮保存失败" }).waitFor();
    await page.waitForFunction(() => !document.querySelector('[data-marker="1"]').disabled);
    assert.equal(await page.evaluate(() => getSelection().isCollapsed), false);
    const unchanged = await get(`/api/highlights/${bookId}/0`); assert.equal(unchanged.length, 1); assert.equal(unchanged[0].color, 3);
    await page.unroute(`**/api/${endpoint}`);
  }
  await page.locator('[data-marker="1"]').click(); await page.locator("#selection-toolbar").waitFor({ state: "hidden" });
  assert.equal((await get(`/api/highlights/${bookId}/0`))[0].color, 1);

  // Lock all annotation controls before the first await. A slow response must
  // apply the captured old quote once, without closing a newer selection.
  let releaseApply, seenApply, applyCalls = 0;
  const gate = new Promise(resolve => { releaseApply = resolve; }), seen = new Promise(resolve => { seenApply = resolve; });
  await page.route("**/api/highlights/apply", async route => { applyCalls++; seenApply(); await gate; await route.continue(); });
  try {
    await selectText(); const oldMark = (await get(`/api/highlights/${bookId}/0`))[0];
    await page.locator('[data-marker="2"]').click(); await seen;
    assert.equal(await page.locator('[data-marker],#toggle-highlight,[data-selection-utility=remove]').evaluateAll(nodes => nodes.every(node => node.disabled)), true);
    await page.locator('[data-marker="2"]').evaluate(node => { node.click(); node.click(); });
    const newQuote = await selectText(true); await settle(); assert.equal(applyCalls, 1);
    const response = page.waitForResponse(r => r.url().endsWith("/api/highlights/apply") && r.status() === 200);
    releaseApply(); await response; await page.waitForFunction(() => !document.querySelector('[data-marker="2"]').disabled); await settle();
    const mark = (await get(`/api/highlights/${bookId}/0`))[0];
    assert.equal(mark.color, 2); assert.deepEqual(mark.start, oldMark.start); assert.deepEqual(mark.end, oldMark.end);
    assert.equal(await page.locator("#selection-toolbar").isVisible(), true);
    assert.equal(await page.evaluate(() => { const clone = getSelection().getRangeAt(0).cloneContents(); clone.querySelectorAll("rt,rp").forEach(node => node.remove()); return clone.textContent; }), newQuote);
  } finally { releaseApply(); await page.unroute("**/api/highlights/apply"); }
  await page.keyboard.press("Escape");
  console.log("PASS: all swatches immediately apply/recolour without toggling off or duplicating; failures preserve marks/selection, controls reject duplicate clicks and slow saves retain newer selections");

  await selectText(); await page.locator("#compose-note").click(); await page.locator("#note-dialog[open]").waitFor();
  assert.equal(await page.locator("#note-quote").textContent(), selected);
  await page.locator("#note-comment").fill("Cancelled draft"); await page.locator('#note-dialog [data-close]').click();
  assert.equal((await page.request.get(`${base}/excerpts/${bookId}`).then(r => r.text())).includes("data-note-id="), false);
  await selectText(); await page.locator("#compose-note").click(); await page.locator("#note-comment").fill("QA thoughts 中文 🌿");
  await page.setViewportSize({ width: 568, height: 320 }); await settle();
  assert.equal(await page.locator("#note-comment").inputValue(), "QA thoughts 中文 🌿");
  await page.locator('#note-form button[type=submit]').scrollIntoViewIfNeeded(); await page.locator('#note-form button[type=submit]').click();
  await page.locator("#note-dialog").waitFor({ state: "hidden" });
  await page.setViewportSize({ width: 1280, height: 900 }); await settle();
  await page.goto(`${base}/excerpts/${bookId}`); await ready(); assert.ok((await page.locator(".excerpt-text").textContent()).includes("QA thoughts 中文 🌿"));
  console.log("PASS: editable note composer, cancel does not save, rotation retains draft, final action reachable and note is durable");

  await page.goto(`${base}/settings`); await ready();
  await page.locator('#ai-settings-form [name="endpoint"]').fill(`http://127.0.0.1:${mock.address().port}`);
  await page.locator('#ai-settings-form [name="model"]').fill("qa-model");
  await page.locator('#ai-settings-form [name="api_key"]').fill("qa-private-key");
  await page.locator('#ai-settings-form button[type=submit]').click(); await page.getByText("AI 设置已保存", { exact: true }).waitFor();
  const config = await get("/api/ai/settings"); assert.equal(config.has_api_key, true); assert.equal(config.api_key, undefined);
  assert.equal((await page.content()).includes("qa-private-key"), false);
  await page.goto(readingURL); await ready(); await settle();
  await selectText(); await page.getByRole("button", { name: "解释", exact: true }).click();
  await page.locator("#ai-answer").filter({ hasText: "QA answer" }).waitFor();
  assert.equal(await page.locator("#ai-answer script").count(), 0);
  assert.equal(aiRequests.length, 1); assert.equal(aiRequests[0].body.model, "qa-model"); assert.equal(aiRequests[0].authorization, "Bearer qa-private-key");
  assert.ok(aiRequests[0].body.messages[1].content.includes(`<selection>\n${selected}\n</selection>`));
  await page.locator('#ai-dialog [data-close]').click(); await selectText(); await page.locator("#ask-selection").click();
  await page.locator("#selection-question").fill("Why is this expression used?"); await page.locator('#selection-question-form button[type=submit]').click();
  await page.locator("#ai-answer").filter({ hasText: "QA answer" }).waitFor(); assert.equal(aiRequests.length, 2); assert.ok(aiRequests[1].body.messages[1].content.includes("Why is this expression used?"));
  await page.locator('#ai-dialog [data-close]').click();
  await selectText(); await utilities(); await page.locator('[data-selection-utility="related"]').click();
  await page.locator("#search-results a").first().waitFor(); assert.ok(await page.locator("#search-results a").count() > 1);
  await page.locator("#search-results a").last().click();
  await page.waitForFunction(() => Number(document.querySelector("#reading-content").dataset.page) > 0);
  await page.goto(`${readingURL}#web-block-0`); await ready(); await settle();
  await selectText(); await utilities(); await page.locator('[data-selection-utility="quote"]').click();
  assert.ok((await page.evaluate(() => navigator.clipboard.readText())).includes("——《林间低语 · Web 回归》"));
  await page.keyboard.press("Escape"); await selectText(true); await utilities();
  await page.locator('[data-selection-utility="vocabulary"]').click(); await page.locator("#selection-hint").filter({ hasText: "已加入词汇本" }).waitFor();
  await page.keyboard.press("Escape");
  console.log("PASS: configured OpenAI-compatible Explain/custom question against loopback mock, secret redaction, related search, citation and vocabulary actions");

  await page.getByRole("button", { name: "阅读外观", exact: true }).click(); await page.locator("#tab-theme").click();
  assert.equal(await page.locator("#appearance-theme").isVisible(), true);
  const old = await preferences(); const bodyPaper = await page.evaluate(() => document.body.style.getPropertyValue("--paper"));
  await page.locator('[name="background"][value="night"]').check();
  assert.equal(await page.evaluate(() => document.body.style.getPropertyValue("--paper")), bodyPaper, "draft must not repaint the reader");
  await page.screenshot({ path: path.join(artifacts, "aa-theme.png") });
  await page.locator('#appearance-dialog [data-close]').click(); assert.equal((await preferences()).background, old.background);
  await page.getByRole("button", { name: "阅读外观", exact: true }).click(); await page.locator("#tab-text").click();
  await page.locator('[name="font"]').selectOption("times_new_roman");
  await page.locator('[name="font_size"]').fill("28"); await page.locator('[name="spacing"]').fill("2.15");
  await page.locator('[name="text_width"][value="narrow"]').check(); await page.locator('[name="turn_mode"][value="curl"]').check();
  await page.screenshot({ path: path.join(artifacts, "aa-text.png") });
  await page.locator("[data-save-appearance]").click(); await page.locator("#appearance-dialog").waitFor({ state: "hidden" }); await settle();
  assert.equal((await preferences()).font, "times_new_roman"); assert.equal((await preferences()).turn_mode, "curl");
  const articleIdentity = await page.evaluate(() => { window.__stableArticle = document.querySelector("#reading-content"); return true; }); assert.equal(articleIdentity, true);
  await page.locator("#next-page").click(); await settle(); await page.locator("#next-page").click(); await settle();
  await page.evaluate(bookId => {
    const location = JSON.parse(sessionStorage.getItem(`readwise:${bookId}:0`));
    const block = document.querySelector(`[data-block="${location.block}"]`), walker = document.createTreeWalker(block, NodeFilter.SHOW_TEXT), encoder = new TextEncoder();
    let remaining = location.offset;
    while (walker.nextNode()) {
      const node = walker.currentNode; if (node.parentElement.closest("rt,rp")) continue;
      const bytes = encoder.encode(node.data).length;
      if (remaining <= bytes) {
        let offset = 0, used = 0; for (const char of node.data) { if (used >= remaining) break; used += encoder.encode(char).length; offset += char.length; }
        const range = document.createRange(); range.setStart(node, offset); range.collapse(true); window.__landmark = range; break;
      }
      remaining -= bytes;
    }
  }, bookId);
  const landmarkVisible = () => page.evaluate(() => { const r = window.__landmark.getBoundingClientRect(), v = document.querySelector("#reader-viewport").getBoundingClientRect(); return r.left >= v.left - 2 && r.left <= v.right + 2 && r.top >= v.top - 2 && r.top <= v.bottom + 2; });
  assert.equal(await landmarkVisible(), true);
  await page.setViewportSize({ width: 320, height: 568 }); await settle(); assert.equal(await page.locator("#reading-content").evaluate(node => getComputedStyle(node).columnCount), "1"); assert.equal((await preferences()).columns, "two");
  assert.equal(await landmarkVisible(), true, "portrait reflow keeps the current source anchor visible");
  await page.setViewportSize({ width: 568, height: 320 }); await settle();
  assert.equal(await landmarkVisible(), true, "short landscape reflow keeps the source anchor visible");
  await page.getByRole("button", { name: "阅读外观", exact: true }).click(); await page.locator("#tab-annotations").click();
  await page.getByRole("button", { name: "词背景：雾蓝", exact: true }).click();
  await page.setViewportSize({ width: 320, height: 568 }); await settle();
  assert.equal(await page.getByRole("button", { name: "词背景：雾蓝", exact: true }).getAttribute("data-on"), "true");
  await page.locator('[data-reset-settings="annotations"]').scrollIntoViewIfNeeded();
  await page.screenshot({ path: path.join(artifacts, "aa-annotations-mobile.png") });
  await page.locator('#appearance-dialog [data-close]').click(); assert.equal((await preferences()).word_background, imported.word_background);
  assert.equal(await page.evaluate(() => window.__stableArticle === document.querySelector("#reading-content")), true);
  await page.setViewportSize({ width: 1280, height: 900 }); await settle();
  await page.goto(`${base}/read/${bookId}/1`); await ready(); await settle();
  const source = await page.locator("#reading-content pre code").textContent();
  const lightCode = await page.locator("#reading-content pre span").evaluateAll(nodes => nodes.map(node => getComputedStyle(node).color));
  await page.evaluate(() => { window.__code = document.querySelector("#reading-content pre code"); });
  await page.getByRole("button", { name: "阅读外观", exact: true }).click(); await page.locator("#tab-theme").click();
  await page.locator('[name="background"][value="night"]').check(); await page.locator("[data-save-appearance]").click();
  await page.locator("#appearance-dialog").waitFor({ state: "hidden" }); await settle();
  assert.equal(await page.locator("#reading-content pre").evaluate(node => getComputedStyle(node).backgroundColor), "rgb(43, 48, 54)");
  assert.notDeepEqual(await page.locator("#reading-content pre span").evaluateAll(nodes => nodes.map(node => getComputedStyle(node).color)), lightCode);
  assert.equal(await page.evaluate(() => document.querySelector("#reading-content pre code") === window.__code), true);
  assert.equal(await page.locator("#reading-content pre code").textContent(), source);
  await page.screenshot({ path: path.join(artifacts, "code-night.png") });
  await page.setViewportSize({ width: 568, height: 320 }); await settle();
  for (let i = 0; i < 20; i++) {
    if (await page.locator("#reading-content pre").evaluate(node => { const r = node.getBoundingClientRect(), v = document.querySelector("#reader-viewport").getBoundingClientRect(); return r.left >= v.left - 1 && r.right <= v.right + 1; })) break;
    await page.locator("#next-page").click(); await settle();
  }
  const codeBounds = await page.locator("#reading-content pre").evaluate(node => {
    const r = node.getBoundingClientRect(), v = document.querySelector("#reader-viewport").getBoundingClientRect(); node.scrollTop = node.scrollHeight;
    return { fits: r.top >= v.top - 1 && r.bottom <= v.bottom + 1, canScroll: node.scrollTop > 0 };
  });
  assert.equal(codeBounds.fits, true); assert.equal(codeBounds.canScroll, true, "short-page long code must remain accessible through its own scroller");
  for (const selector of ["#page-navigation", ".chapter-navigation"]) assert.equal(await page.locator(selector).evaluate(node => { const r = node.getBoundingClientRect(); return r.top >= 0 && r.bottom <= innerHeight; }), true);
  await page.screenshot({ path: path.join(artifacts, "code-short-landscape.png") });
  await page.setViewportSize({ width: 1280, height: 900 }); await settle();
  console.log("PASS: theme changes restyle code without replacing it; short paged code scrolls internally and both navigation rows stay in viewport");

  const touchContext = await browser.newContext({ viewport: { width: 390, height: 740 }, isMobile: true, hasTouch: true });
  const touchPage = await touchContext.newPage(); touchPage.on("pageerror", error => failures.push(error.message));
  await touchPage.goto(readingURL); await touchPage.locator('body[data-browser-ready="true"]').waitFor(); await touchPage.waitForTimeout(350);
  const startPage = Number(await touchPage.locator("#reading-content").getAttribute("data-page"));
  const touchPoint = await touchPage.locator("#reading-content ruby").evaluateAll(nodes => {
    const v = document.querySelector("#reader-viewport").getBoundingClientRect();
    const node = nodes.find(node => { const r = node.getBoundingClientRect(); return r.left > 140 && r.right < v.right && r.top > v.top && r.bottom < v.bottom; });
    if (!node) return null; const r = node.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
  });
  assert.ok(touchPoint);
  const cdp = await touchContext.newCDPSession(touchPage);
  await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ ...touchPoint, id: 1 }] });
  for (const dx of [45, 90, 135]) { await touchPage.waitForTimeout(25); await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [{ x: touchPoint.x - dx, y: touchPoint.y, id: 1 }] }); }
  await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await touchPage.waitForFunction(expected => Number(document.querySelector("#reading-content").dataset.page) === expected, startPage + 1);
  assert.equal(await touchPage.locator("#word-card[open]").count(), 0);
  await touchContext.close();
  console.log("PASS: emulated horizontal touch swipe beginning on a ruby word turns one page without opening the word card");
  await page.goto(`${base}/settings`); await ready(); assert.equal(await page.locator('[name="font"]').inputValue(), "times_new_roman"); assert.equal(await page.locator('[name="turn_mode"]:checked').inputValue(), "curl");
  assert.deepEqual(fs.readFileSync(desktop), desktopBefore);
  console.log("PASS: all three aA tabs, theme/text preview and persistence, font list, narrow/short rotation, cancel and unified global settings");
  const finalMarks = await get(`/api/highlights/${bookId}/0`); assert.equal(finalMarks.length, 1);
  await context.close(); await stop(); base = await start();
  const restarted = await browser.newPage({ viewport: { width: 1280, height: 900 } }); restarted.on("pageerror", error => failures.push(error.message));
  await restarted.goto(`${base}/read/${bookId}/0`); await restarted.locator('body[data-browser-ready="true"]').waitFor();
  await restarted.waitForFunction(colour => CSS.highlights.get(`rw-${colour}`)?.size === 1, finalMarks[0].color);
  const restoredPreferences = await restarted.request.get(`${base}/api/preferences`).then(response => response.json());
  assert.equal(restoredPreferences.font, "times_new_roman"); assert.equal(restoredPreferences.turn_mode, "curl"); assert.equal(restoredPreferences.background, "night");
  assert.ok((await restarted.request.get(`${base}/excerpts/${bookId}`).then(response => response.text())).includes("QA thoughts 中文 🌿"));
  const restoredAI = await restarted.request.get(`${base}/api/ai/settings`).then(response => response.json());
  assert.equal(restoredAI.has_api_key, true); assert.equal(restoredAI.api_key, undefined);
  assert.deepEqual(fs.readFileSync(desktop), desktopBefore);
  console.log("PASS: real stop/restart retains Web highlights, notes, full preferences and AI config without changing desktop data");
  assert.deepEqual(failures, []);
  console.log(`Parity screenshots: ${artifacts}`);
}
run().catch(error => { console.error(error); process.exitCode = 1; }).finally(async () => {
  if (browser) await browser.close();
  try { await stop(); } catch (error) { console.error(error); process.exitCode = 1; }
  if (mock) await new Promise(resolve => mock.close(resolve));
  fs.writeFileSync(path.join(artifacts, "server.log"), logs);
  fs.rmSync(temporary, { recursive: true, force: true });
});
