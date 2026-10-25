// Real Chromium regression, against an isolated server/database and owned EPUB.
// npm ci --prefix web/tests && npm test --prefix web/tests
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawn, execFileSync } = require("node:child_process");
const { chromium } = require(process.env.READWISE_PLAYWRIGHT_MODULE || "playwright-core");

const root = path.resolve(__dirname, "../..");
const binary = process.env.READWISE_WEB_BINARY || path.join(root, "target/debug/readwise-web");
const artifacts = path.join(root, "target/web-qa/browser");
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "readwise-web-"));
fs.mkdirSync(artifacts, { recursive: true });
const fixture = path.join(temporary, "林间低语.epub");
execFileSync("python3", [path.join(__dirname, "fixture.py"), fixture]);
let server;
let browser;
let serverLog = "";
const failures = [];

async function start() {
  return new Promise((resolve, reject) => {
    server = spawn(binary, [], { cwd: root, env: { ...process.env, READWISE_WEB_BIND: "127.0.0.1:0", READWISE_WEB_DATA_DIR: path.join(temporary, "data") }, stdio: ["ignore", "pipe", "pipe"] });
    const timer = setTimeout(() => reject(new Error("Web server startup timed out")), 25000);
    let startupLog = "";
    const log = chunk => {
      const text = chunk.toString();
      serverLog += text;
      startupLog += text;
      const match = startupLog.match(/ReadWise Web: (http:\/\/[^\s]+)\n/);
      if (match) { clearTimeout(timer); resolve(match[1]); }
    };
    server.stdout.on("data", log);
    server.stderr.on("data", log);
    server.on("error", error => { clearTimeout(timer); reject(error); });
    server.once("exit", code => { clearTimeout(timer); if (code) reject(new Error(`Server exited: ${code}`)); });
  });
}

async function stop() {
  if (!server || server.exitCode !== null) return;
  const child = server;
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => { child.kill("SIGKILL"); reject(new Error("Graceful shutdown timed out")); }, 10000);
    child.once("exit", (code, signal) => { clearTimeout(timer); code === 0 ? resolve() : reject(new Error(`Shutdown failed: ${code}/${signal}`)); });
    child.kill("SIGINT");
  });
  server = null;
}

async function run() {
  let base = await start();
  const executablePath = process.env.READWISE_CHROMIUM || (process.platform === "darwin" ? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" : undefined);
  browser = await chromium.launch({ headless: true, ...(executablePath ? { executablePath } : {}) });
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const page = await context.newPage();
  page.on("pageerror", error => failures.push(error.message));
  const getPreferences = () => page.request.get(`${base}/api/preferences`).then(r => r.json());
  const settle = () => page.waitForTimeout(250);
  const positions = () => page.evaluate(() => ({
    scroll: scrollY,
    blocks: [...document.querySelectorAll("#reading-content [data-block]")].map(node => {
      const r = node.getBoundingClientRect(); return [r.x, r.y, r.width, r.height];
    }),
    words: [...document.querySelectorAll("#reading-content .word")].slice(0, 20).map(node => {
      const r = node.getBoundingClientRect(); return [r.x, r.y, r.width, r.height];
    }),
  }));
  const ruby = lemma => page.locator(`#reading-content ruby[data-lemma="${lemma}"]`);
  const visibleIndex = lemma => page.evaluate(lemma => [...document.querySelectorAll(`#reading-content ruby[data-lemma="${lemma}"]`)].findIndex(node => {
    const r = node.getBoundingClientRect(); return r.top > 110 && r.bottom < innerHeight - 50;
  }), lemma);

  await page.goto(base);
  await page.locator('body[data-browser-ready="true"]').waitFor();
  await page.request.post(`${base}/api/preferences`, { headers: { "x-readwise-request": "1" }, data: { ...await getPreferences(), turn_mode: "scroll", columns: "one" } });
  await page.getByText("从一本书开始", { exact: true }).waitFor();
  await page.locator("#book-upload").setInputFiles(fixture);
  await page.locator('#import-results [data-state="success"] a').waitFor({ timeout: 30000 });
  assert.equal(fs.readdirSync(path.join(temporary, "data/uploads")).length, 0);
  await page.locator('#import-results [data-state="success"] a').click();
  await page.locator("#reading-content").waitFor();
  await page.evaluate(() => document.fonts.ready);
  await settle();
  const bookId = await page.locator("#reading-content").getAttribute("data-book-id");
  assert.ok(await ruby("solitude").count() > 10);
  const count = await page.locator("#reading-content ruby").count();
  const image = page.locator('#reading-content img');
  await image.evaluate(node => node.decode());
  assert.equal(await image.evaluate(node => node.naturalWidth), 1);
  console.log("PASS: streamed Chinese EPUB import, real chapter, ruby and private image route");

  const chapterLink = page.locator('.reader-block a', { hasText: 'Other chapter' });
  const assertLinkStyle = async () => {
    assert.deepEqual(await chapterLink.evaluate(node => {
      const style = getComputedStyle(node);
      return { decoration: style.textDecorationLine, color: style.color };
    }), { decoration: 'none', color: 'rgb(69, 107, 117)' });
  };
  await assertLinkStyle();
  await chapterLink.scrollIntoViewIfNeeded();
  await chapterLink.hover();
  await assertLinkStyle();
  await chapterLink.focus();
  await assertLinkStyle();
  assert.notEqual(await chapterLink.evaluate(node => getComputedStyle(node).outlineStyle), 'none');
  await page.mouse.down();
  await assertLinkStyle();
  await page.mouse.up();
  await page.waitForURL(`**/read/${bookId}/1`);
  await page.goBack();
  await page.evaluate(() => scrollTo(0, 0));
  await settle();
  console.log("PASS: reader links keep their color and keyboard focus, without underlines in normal/hover/focus/active states; real click still navigates");

  await ruby("solitude").first().click();
  await page.locator("#word-card[open]").waitFor();
  await page.evaluate(() => { window.__qaRuby = document.querySelector('#reading-content ruby[data-lemma="solitude"]'); });
  const before = await positions();
  await page.getByRole("button", { name: "标为已认识", exact: true }).click();
  await page.waitForFunction(() => document.querySelector('#reading-content ruby[data-lemma="solitude"]').dataset.known === "true");
  await settle();
  assert.equal(await page.locator("#word-card").getAttribute("open"), null);
  assert.equal(await ruby("solitude").first().locator("rt").evaluate(node => getComputedStyle(node).visibility), "hidden");
  assert.equal(await page.evaluate(() => window.__qaRuby === document.querySelector('#reading-content ruby[data-lemma="solitude"]')), true);
  assert.deepEqual(await positions(), before);
  assert.equal(await page.locator("#reading-content ruby").count(), count);
  console.log("PASS: mark known closes card, hides rt in place, keeps nodes/line geometry/scroll");

  await page.evaluate(() => scrollTo(0, 2500));
  await settle();
  const index = await visibleIndex("murmur");
  assert.ok(index > 0, "subsequent screen must use a later ruby occurrence");
  await ruby("murmur").nth(index).click();
  const laterBefore = await positions();
  await page.getByRole("button", { name: "标为已认识", exact: true }).click();
  await settle();
  assert.deepEqual(await positions(), laterBefore);
  await page.screenshot({ path: path.join(artifacts, "reader-desktop.png") });
  await page.getByRole("link", { name: "下一章 →", exact: true }).click();
  await page.waitForURL(`**/read/${bookId}/1`);
  await settle();
  assert.equal(await ruby("solitude").first().getAttribute("data-known"), "true");
  await page.goBack();
  await settle();
  assert.equal(await ruby("murmur").first().getAttribute("data-known"), "true");
  assert.equal(await ruby("murmur").first().locator("rt").evaluate(node => getComputedStyle(node).visibility), "hidden");
  assert.equal(await page.locator("#reading-content ruby").count(), count);
  console.log("PASS: later-screen mark known and return from next chapter preserve annotation slots");

  // A moved/held gesture must not open a word card; a genuine tap afterwards must.
  await page.evaluate(() => scrollTo(0, 0));
  await settle();
  const forest = ruby("forest").first();
  const box = await forest.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.waitForTimeout(550);
  await page.mouse.up();
  assert.equal(await page.locator("#word-card").getAttribute("open"), null);
  await page.evaluate(() => getSelection().removeAllRanges());
  await page.route("**/api/known", route => route.request().method() === "POST" ? route.abort() : route.continue());
  await forest.click();
  await page.locator("#word-card[open]").waitFor();
  const failureBefore = await positions();
  await page.getByRole("button", { name: "标为已认识", exact: true }).click();
  await page.waitForFunction(() => document.querySelector('#reading-content ruby[data-lemma="forest"]').dataset.known === "false");
  assert.deepEqual(await positions(), failureBefore);
  await page.unroute("**/api/known");
  console.log("PASS: long hold is not a tap; failed known save restores state without relayout");

  // Reader selection, not a word-card click. Includes multiple ruby slots,
  // Chinese/emoji and a paragraph boundary; none of the rt text may be copied.
  await context.grantPermissions(["clipboard-read", "clipboard-write"], { origin: base });
  const selectParagraphs = async target => target.evaluate(() => {
    const paragraphs = [...document.querySelectorAll(".reader-block p")].slice(0, 2);
    const expected = paragraphs.map(node => {
      const clone = node.cloneNode(true);
      clone.querySelectorAll("rt, rp").forEach(rt => rt.remove());
      return clone.textContent;
    }).join("\n\n");
    const range = document.createRange();
    range.setStart(paragraphs[0].firstChild, 0);
    range.setEnd(paragraphs[1].lastChild, paragraphs[1].lastChild.length);
    getSelection().removeAllRanges(); getSelection().addRange(range);
    return expected;
  });
  const openUtilities = async target => { if (!await target.locator("#selection-more").isVisible()) await target.locator("#more-selection").click(); };
  const selectionBefore = await positions();
  const excerptText = await selectParagraphs(page);
  await page.locator("#selection-toolbar").waitFor({ state: "visible" });
  assert.deepEqual(await positions(), selectionBefore);
  assert.equal(await page.locator("#word-card").getAttribute("open"), null);
  await openUtilities(page);
  await page.getByRole("button", { name: "复制", exact: true }).click();
  await page.getByText("已复制选中文本", { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => navigator.clipboard.readText()), excerptText);
  await page.route("**/api/excerpts", route => route.abort());
  await page.getByRole("button", { name: "保存摘录", exact: true }).click();
  await page.getByText(/摘录保存失败/).waitFor();
  assert.equal(await page.locator("#selection-toolbar").isVisible(), true);
  assert.equal(await page.evaluate(() => getSelection().isCollapsed), false);
  await page.unroute("**/api/excerpts");
  let releaseSave;
  let excerptRequests = 0;
  const heldSave = new Promise(resolve => { releaseSave = resolve; });
  await page.route("**/api/excerpts", async route => { excerptRequests++; await heldSave; await route.continue(); });
  const excerptSaved = page.waitForResponse(r => r.url().endsWith("/api/excerpts") && r.request().method() === "POST");
  await page.getByRole("button", { name: "保存摘录", exact: true }).click();
  await selectParagraphs(page);
  await settle();
  assert.equal(await page.locator("#save-excerpt").isDisabled(), true, "selection updates must not re-enable an in-flight save");
  await page.locator("#save-excerpt").evaluate(button => button.click());
  await page.evaluate(() => {
    const paragraph = document.querySelectorAll(".reader-block p")[2];
    const range = document.createRange(); range.selectNodeContents(paragraph);
    getSelection().removeAllRanges(); getSelection().addRange(range);
  });
  await settle();
  releaseSave();
  assert.equal((await excerptSaved).status(), 200);
  await page.unroute("**/api/excerpts");
  assert.equal(excerptRequests, 1);
  await page.getByText("已保存，可在阅读页的「摘录」查看", { exact: true }).waitFor();
  assert.equal(await page.locator("#selection-toolbar").isVisible(), true);
  assert.equal(await page.evaluate(() => getSelection().isCollapsed), false, "a slow save must preserve a different, newer selection");
  await page.keyboard.press("Escape");
  await page.locator("#selection-toolbar").waitFor({ state: "hidden" });
  assert.deepEqual(await positions(), selectionBefore);
  await page.getByRole("link", { name: "摘录", exact: true }).click();
  await page.locator(".excerpt-entry").waitFor();
  assert.equal(await page.locator(".excerpt-entry").count(), 1);
  assert.equal(await page.locator(".excerpt-text").textContent(), `${excerptText}\n\n—— The Forest`);
  await page.getByRole("link", { name: "← 返回阅读", exact: true }).click();
  await page.locator("#reading-content").waitFor();
  await settle();
  await page.evaluate(() => scrollTo(0, 0));
  await settle();

  // Actual mouse drag must open the toolbar only on release, not the word card.
  const dragPoints = await page.evaluate(() => {
    const paragraphs = [...document.querySelectorAll(".reader-block p")];
    const first = document.createRange(); first.selectNodeContents(paragraphs[0].firstChild);
    const last = document.createRange(); last.selectNodeContents(paragraphs[1].lastChild);
    const a = first.getClientRects()[0], b = [...last.getClientRects()].at(-1);
    return { x1: a.left + 1, y1: a.top + a.height / 2, x2: b.right - 1, y2: b.top + b.height / 2 };
  });
  await page.mouse.move(dragPoints.x1, dragPoints.y1);
  await page.mouse.down();
  await page.mouse.move(dragPoints.x2, dragPoints.y2, { steps: 12 });
  assert.equal(await page.locator("#selection-toolbar").isVisible(), false);
  await page.mouse.up();
  await page.locator("#selection-toolbar").waitFor({ state: "visible" });
  assert.equal(await page.locator("#word-card").getAttribute("open"), null);
  await page.screenshot({ path: path.join(artifacts, "selection-desktop.png") });
  await page.keyboard.press("Escape");
  await page.locator("#selection-toolbar").waitFor({ state: "hidden" });
  await page.evaluate(() => {
    const node = document.querySelector(".reader-block p").firstChild;
    getSelection().collapse(node, 0);
    getSelection().modify("extend", "forward", "character");
  });
  await page.locator("#selection-toolbar").waitFor({ state: "visible" });
  await page.keyboard.press("Escape");
  console.log("PASS: real drag/Selection API, keyboard dismiss, overlay geometry, ruby-free copy, excerpt save/failure and persisted list");

  const code = page.locator("#reading-content pre code").first();
  await code.scrollIntoViewIfNeeded();
  const source = 'fn main() {\n    // 森林 🌿\n    let forest = "<script>&";\n    println!("{}", forest);\n}\n';
  assert.equal(await code.textContent(), source);
  assert.equal(await code.locator("ruby").count(), 0);
  const colours = await code.locator("span").evaluateAll(nodes => [...new Set(nodes.map(node => getComputedStyle(node).color))]);
  assert.ok(colours.length >= 3, "keywords, strings and comments must have distinct syntax colours");
  await code.evaluate(node => { const range = document.createRange(); range.selectNodeContents(node); getSelection().removeAllRanges(); getSelection().addRange(range); });
  await page.locator("#selection-toolbar").waitFor({ state: "visible" });
  await openUtilities(page);
  await page.getByRole("button", { name: "复制", exact: true }).click();
  await page.getByText("已复制选中文本", { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => navigator.clipboard.readText()), source);
  await page.screenshot({ path: path.join(artifacts, "syntax-desktop.png") });
  await page.keyboard.press("Escape");
  await page.evaluate(() => scrollTo(0, 0));
  await settle();
  console.log("PASS: server-rendered Rust syntax colours, escaped source/indentation and lossless code copy");

  await page.getByRole("button", { name: "阅读外观", exact: true }).click();
  await page.locator('#tab-annotations').click();
  await page.getByRole("button", { name: "注音背景：奶黄", exact: true }).click();
  assert.equal(await page.locator(".annotation-preview rt").first().evaluate(node => getComputedStyle(node).backgroundColor), "rgb(255, 233, 168)");
  await page.getByRole("button", { name: "词背景：薄荷", exact: true }).click();
  const originalURL = page.url();
  const response = page.waitForResponse(r => r.url().endsWith("/api/preferences") && r.request().method() === "POST");
  await page.getByRole("button", { name: "应用并保存", exact: true }).click();
  assert.equal((await response).status(), 200);
  await page.waitForFunction(() => !document.querySelector("#appearance-dialog").open);
  assert.equal(page.url(), originalURL);
  assert.equal(await page.locator("#reading-content").evaluate(node => node.style.getPropertyValue("--gloss-background")), "#FFE9A8");
  assert.equal((await getPreferences()).word_background, 2);
  assert.equal(await page.locator("#reading-content ruby").count(), count);

  await page.setViewportSize({ width: 320, height: 568 });
  await page.getByRole("button", { name: "阅读外观", exact: true }).click();
  const swatchY = await page.locator('[data-field="gloss_background"] .swatch').evaluateAll(nodes => nodes.map(node => node.getBoundingClientRect().y));
  assert.ok(Math.max(...swatchY) - Math.min(...swatchY) < 1, "all four swatches must remain on one row");
  assert.equal(await page.getByRole("button", { name: "注音背景：无", exact: true }).getAttribute("data-none"), "true");
  await page.getByRole("button", { name: "注音背景：雾蓝", exact: true }).click();
  await page.locator('#tab-text').click();
  await page.locator('[name="font"]').selectOption("times_new_roman");
  await page.screenshot({ path: path.join(artifacts, "appearance-portrait.png") });
  await page.setViewportSize({ width: 568, height: 320 });
  assert.equal(await page.locator('[data-field="gloss_background"] [data-slot="3"]').getAttribute("data-on"), "true");
  assert.equal(await page.locator('[name="font"]').inputValue(), "times_new_roman");
  await page.locator("#appearance-dialog").hover();
  await page.mouse.wheel(0, 1600);
  await settle();
  await page.locator('[data-reset-settings="text"]').scrollIntoViewIfNeeded();
  assert.equal(await page.getByRole("button", { name: "应用并保存", exact: true }).isVisible(), true);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  const dialogBox = await page.locator("#appearance-dialog").boundingBox();
  const lastOptionBox = await page.locator('[data-reset-settings="text"]').boundingBox();
  const saveBox = await page.getByRole("button", { name: "应用并保存", exact: true }).boundingBox();
  assert.ok(dialogBox.width <= 568 && dialogBox.height <= 320);
  assert.ok(lastOptionBox.y > dialogBox.y + 50 && lastOptionBox.y + lastOptionBox.height <= saveBox.y, "last option must be reachable above fixed actions");
  await page.screenshot({ path: path.join(artifacts, "appearance-landscape.png") });
  await page.getByRole("button", { name: "取消", exact: true }).click();
  assert.equal((await getPreferences()).gloss_background, 1);
  assert.equal((await getPreferences()).english_level, "A1");
  assert.equal((await getPreferences()).font, "original");
  await page.getByRole("button", { name: "阅读外观", exact: true }).click();
  await page.locator('#tab-annotations').click();
  assert.equal(await page.getByRole("button", { name: "注音背景：奶黄", exact: true }).getAttribute("data-on"), "true");
  await page.getByRole("button", { name: "取消", exact: true }).click();
  console.log("PASS: Topcoat colour signals, explicit persistence, 320px portrait/short landscape, rotation draft and cancel");

  await page.setViewportSize({ width: 1280, height: 900 });
  await page.evaluate(() => scrollTo(0, 0));
  await settle();
  await ruby("forest").first().click();
  await page.getByRole("button", { name: "加入词汇本", exact: true }).click();
  await page.waitForFunction(() => !document.querySelector("#word-card").open);
  await page.goto(`${base}/vocabulary`);
  await page.locator(".vocabulary-entry h2", { hasText: "forest" }).waitFor();
  await page.goto(`${base}/settings`);
  await page.waitForFunction(() => document.querySelectorAll(".swatch[data-on=true]").length === 4);
  assert.equal(await page.getByRole("button", { name: "注音背景：奶黄", exact: true }).getAttribute("data-on"), "true");
  await page.screenshot({ path: path.join(artifacts, "settings-desktop.png") });
  console.log("PASS: vocabulary stored in real database, settings and aA share persisted slots");

  await page.goto(`${base}/read/${bookId}/0`);
  await settle();
  await page.evaluate(() => scrollTo(0, 4200));
  await page.waitForTimeout(1200);
  const location = JSON.parse(execFileSync("python3", ["-c", "import json,sqlite3,sys; c=sqlite3.connect(sys.argv[1]); print(json.dumps(c.execute('select location from book_status where book_id=?',(sys.argv[2],)).fetchone()[0]))", path.join(temporary, "data/database/app.db"), bookId], { encoding: "utf8" }));
  assert.match(location, /^web:v1:0:[1-9][0-9]*:[0-9]+$/);
  await page.goto(base);
  await stop();
  base = await start();
  await page.goto(`${base}/read/${bookId}`);
  await settle();
  assert.equal(await page.evaluate(() => scrollY > 1000), true);
  assert.equal(await ruby("solitude").first().getAttribute("data-known"), "true");
  assert.equal((await getPreferences()).gloss_background, 1);
  const persistedExcerpt = await page.request.get(`${base}/excerpts/${bookId}`);
  assert.ok((await persistedExcerpt.text()).includes("data-note-id="));
  console.log("PASS: graceful stop/restart restores source anchor, known words, appearance and saved excerpts");

  const touchContext = await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
  const touchPage = await touchContext.newPage();
  touchPage.on("pageerror", error => failures.push(error.message));
  await touchPage.goto(`${base}/read/${bookId}/0`);
  await touchPage.waitForTimeout(400);
  await touchPage.evaluate(() => scrollTo(0, 0));
  const touchWord = touchPage.locator('#reading-content ruby[data-lemma="forest"]').first();
  const touchBox = await touchWord.boundingBox();
  const x = touchBox.x + touchBox.width / 2;
  const y = touchBox.y + touchBox.height / 2;
  const cdp = await touchContext.newCDPSession(touchPage);
  await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ id: 1, x, y }] });
  await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [{ id: 1, x, y: y + 35 }] });
  await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [{ id: 1, x, y }] });
  await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await touchPage.waitForTimeout(150);
  assert.equal(await touchPage.locator("#word-card").getAttribute("open"), null);
  await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ id: 1, x, y }] });
  await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ id: 1, x, y }, { id: 2, x: x + 12, y: y + 12 }] });
  await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await touchPage.waitForTimeout(150);
  assert.equal(await touchPage.locator("#word-card").getAttribute("open"), null);
  await touchPage.evaluate(() => getSelection().removeAllRanges());
  await touchWord.tap();
  await touchPage.locator("#word-card[open]").waitFor();
  await touchPage.getByRole("button", { name: "关闭词卡", exact: true }).tap();
  await touchPage.setViewportSize({ width: 320, height: 568 });
  await selectParagraphs(touchPage);
  await touchPage.locator("#selection-toolbar").waitFor({ state: "visible" });
  for (const size of [{ width: 320, height: 568 }, { width: 568, height: 320 }]) {
    await touchPage.setViewportSize(size);
    await touchPage.waitForTimeout(150);
    const toolbar = await touchPage.locator("#selection-toolbar").boundingBox();
    assert.ok(toolbar && toolbar.x >= 0 && toolbar.y >= 0 && toolbar.x + toolbar.width <= size.width && toolbar.y + toolbar.height <= size.height, "selection toolbar must remain within the rotated mobile viewport");
  }
  await touchPage.screenshot({ path: path.join(artifacts, "selection-mobile-landscape.png") });
  await openUtilities(touchPage);
  await touchPage.getByRole("button", { name: "取消选文", exact: true }).tap();
  await touchPage.locator("#selection-toolbar").waitFor({ state: "hidden" });
  await touchPage.goto(base);
  const widths = await touchPage.locator('.app-header nav a').evaluateAll(nodes => nodes.map(node => node.getBoundingClientRect().width));
  assert.ok(Math.max(...widths) - Math.min(...widths) < 1);
  assert.equal(await touchPage.locator('.app-header nav svg').count(), 4);
  await touchContext.close();
  console.log("PASS: emulated touch move-return/multitouch reject word clicks; genuine tap and equal-width icon navigation work");
  assert.deepEqual(failures, [], "browser runtime errors");
  console.log(`Screenshots: ${artifacts}`);
}

run().catch(error => { console.error(error); process.exitCode = 1; }).finally(async () => {
  if (browser) await browser.close();
  try { await stop(); } catch (error) { console.error(error); process.exitCode = 1; }
  fs.writeFileSync(path.join(artifacts, "server.log"), serverLog);
  fs.rmSync(temporary, { recursive: true, force: true });
});
