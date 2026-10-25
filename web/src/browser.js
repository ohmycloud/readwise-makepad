// Topcoat owns reactive colour signals. This module owns browser-only reader
// capabilities (Selection/Range, dialogs, file streaming and stable anchors).
const $ = (selector, root = document) => root.querySelector(selector);
const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];
const article = $("#reading-content");
const encoder = new TextEncoder();
let toastTimer;
let translate = text => text;
let pager;
const readerReady = article ? import(article.dataset.pagerSrc).then(({ createPager }) => {
  pager = createPager({ article, textNodes, textPoint, byteOffset, pointRect, save: flushProgress });
}) : Promise.resolve();

function toast(message) {
  const element = $("#toast");
  element.textContent = translate(message);
  element.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { element.hidden = true; }, 4500);
}

async function api(path, value, { method = "POST", keepalive = false, signal } = {}) {
  const response = await fetch(path, {
    method, credentials: "same-origin", keepalive, signal,
    headers: { "content-type": "application/json", "x-readwise-request": "1" },
    ...(method !== "GET" ? { body: JSON.stringify(value) } : {}),
  });
  if (!response.ok) {
    const detail = (await response.text()).trim();
    throw new Error(detail && detail.length < 500 && !detail.startsWith("<") ? detail : `请求失败（${response.status}）`);
  }
  return response.json();
}

function textNodes(block) {
  const walker = document.createTreeWalker(block, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      return node.parentElement.closest("rt, rp, script, style, svg")
        ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT;
    },
  });
  const nodes = [];
  while (walker.nextNode()) nodes.push(walker.currentNode);
  return nodes;
}

function byteOffset(nodes, node, offset) {
  let bytes = 0;
  for (const current of nodes) {
    if (current === node) return bytes + encoder.encode(current.textContent.slice(0, offset)).length;
    bytes += encoder.encode(current.textContent).length;
  }
  return 0;
}

function textPoint(block, bytes) {
  const nodes = textNodes(block);
  for (const node of nodes) {
    const length = encoder.encode(node.textContent).length;
    if (bytes <= length) {
      let consumed = 0;
      let offset = 0;
      for (const character of node.textContent) {
        const width = encoder.encode(character).length;
        if (consumed + width > bytes) break;
        consumed += width;
        offset += character.length;
      }
      return { node, offset };
    }
    bytes -= length;
  }
  const node = nodes.at(-1);
  return node ? { node, offset: node.textContent.length } : null;
}

function pointRect(point) {
  if (!point) return null;
  const parent=point.node.parentElement;
  const owner=parent.closest('[data-math-owner]')?.dataset.mathOwner;
  const math=parent.closest('.typst-math') || (owner && article.querySelector(`[data-math-id="${owner}"]`));
  if (math) return math.getBoundingClientRect();
  const range = document.createRange();
  range.setStart(point.node, point.offset);
  range.collapse(true);
  const rect = range.getBoundingClientRect();
  return rect.height ? rect : point.node.parentElement.getBoundingClientRect();
}

function captureAnchor() {
  if (!article) return null;
  if (pager?.active()) return pager.capture();
  const top = $(".reader-toolbar").getBoundingClientRect().bottom + 16;
  const blocks = $$("[data-block]", article);
  const block = blocks.find(block => block.getBoundingClientRect().bottom > top) || blocks.at(-1);
  if (!block) return null;
  const rect = block.getBoundingClientRect();
  const x = Math.min(rect.right - 2, rect.left + 8);
  const y = Math.max(rect.top + 2, top);
  let point;
  if (document.caretPositionFromPoint) {
    const caret = document.caretPositionFromPoint(x, y);
    if (caret) point = { node: caret.offsetNode, offset: caret.offset };
  } else if (document.caretRangeFromPoint) {
    const caret = document.caretRangeFromPoint(x, y);
    if (caret) point = { node: caret.startContainer, offset: caret.startOffset };
  }
  const nodes = textNodes(block);
  if (!point || !nodes.includes(point.node)) point = nodes[0] ? { node: nodes[0], offset: 0 } : null;
  const offset = point ? byteOffset(nodes, point.node, point.offset) : 0;
  return {
    chapter: Number(article.dataset.chapter), block: Number(block.dataset.block), offset,
    screenY: pointRect(point)?.top ?? rect.top,
  };
}

function restoreAnchor(anchor, saved = false) {
  if (!anchor || !article) return;
  if (pager?.restore(anchor)) return;
  const block = $$("[data-block]", article).find(block => Number(block.dataset.block) === anchor.block);
  if (!block) return;
  const rect = pointRect(textPoint(block, anchor.offset));
  const top = rect?.top ?? block.getBoundingClientRect().top;
  const target = saved ? $(".reader-toolbar").getBoundingClientRect().bottom + 16 : anchor.screenY;
  window.scrollBy(0, top - target);
}

let progressTimer;
let pendingProgress;
let progressDrain = null;
let restoring = true;

function flushProgress() {
  if (!article || restoring) return Promise.resolve();
  const anchor = captureAnchor();
  if (!anchor) return Promise.resolve();
  const location = { chapter: anchor.chapter, block: anchor.block, offset: anchor.offset };
  try {
    sessionStorage.setItem(`readwise:${article.dataset.bookId}:${location.chapter}`, JSON.stringify(location));
  } catch { /* Storage may be disabled; durable server saving still works. */ }
  pendingProgress = location;
  if (progressDrain) return progressDrain;
  progressDrain = (async () => {
    try {
      // Serialize writes so a slower old position cannot overwrite a newer one.
      while (pendingProgress) {
        const next = pendingProgress;
        pendingProgress = null;
        await api("/api/progress", { book_id: article.dataset.bookId, location: next }, { keepalive: true });
      }
    } catch {
      toast("阅读位置暂未保存，请检查服务连接");
    } finally { progressDrain = null; }
  })();
  return progressDrain;
}

if (article) {
  const restoreFragment = () => {
    if (!pager?.active() || !location.hash) return;
    let id; try { id = decodeURIComponent(location.hash.slice(1)); } catch { return; }
    const block = document.getElementById(id)?.closest("[data-block]");
    if (block) pager.restore({ chapter: Number(article.dataset.chapter), block: Number(block.dataset.block), offset: 0 });
  };
  window.addEventListener("hashchange", () => { restoreFragment(); pager?.refreshAnchor(); flushProgress(); });
  const initializePosition = async () => {
    await readerReady;
    await document.fonts.ready;
    pager.configure(JSON.parse(article.dataset.preferences));
    await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    if (!location.hash) {
      let anchor = {
        chapter: Number(article.dataset.chapter), block: Number(article.dataset.resumeBlock),
        offset: Number(article.dataset.resumeOffset),
      };
      if (performance.getEntriesByType("navigation")[0]?.type === "back_forward") {
        try { anchor = JSON.parse(sessionStorage.getItem(`readwise:${article.dataset.bookId}:${anchor.chapter}`)) || anchor; } catch { /* stale tab state */ }
      }
      // A new chapter starts at its heading, not below the toolbar.
      if (anchor.block || anchor.offset) restoreAnchor(anchor, true);
    } else restoreFragment();
    pager.refreshAnchor();
    restoring = false;
  };
  initializePosition().catch(() => { restoring = false; });
  window.addEventListener("scroll", () => {
    if (restoring) return;
    clearTimeout(progressTimer);
    progressTimer = setTimeout(flushProgress, 700);
  }, { passive: true });
  document.addEventListener("visibilitychange", () => { if (document.hidden) flushProgress(); });
  window.addEventListener("pagehide", flushProgress);
  window.addEventListener("pageshow", event => {
    if (event.persisted) restoring = false;
    syncKnown();
  });
  document.addEventListener("click", async event => {
    const link = event.target.closest("a[href]");
    if (!link || event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey || link.target || link.hasAttribute("download")) return;
    event.preventDefault();
    // Flush the latest queued anchor before ordinary navigation. Never trap
    // navigation behind a disconnected server; keepalive is then best-effort.
    await Promise.race([flushProgress(), new Promise(resolve => setTimeout(resolve, 1000))]);
    link.closest("dialog")?.close();
    location.assign(link.href);
  });
}

// Known state is paint-only. Both a new server render and browser back/forward
// retain the same ruby + rt nodes and reserve their original geometry.
const pendingKnown = new Set();
const knownVersions = new Map();
const channel = typeof BroadcastChannel === "function" ? new BroadcastChannel("readwise-web") : null;

function setKnown(lemma, known) {
  if (!article) return;
  for (const ruby of $$("ruby[data-lemma]", article)) {
    if (ruby.dataset.lemma === lemma) ruby.dataset.known = String(known);
  }
}

async function syncKnown() {
  if (!article) return;
  try {
    const known = new Set(await api("/api/known", null, { method: "GET" }));
    for (const ruby of $$("ruby[data-lemma]", article)) {
      if (!pendingKnown.has(ruby.dataset.lemma)) ruby.dataset.known = String(known.has(ruby.dataset.lemma));
    }
  } catch { /* Keep the last visible projection while disconnected. */ }
}
channel?.addEventListener("message", event => { if (event.data === "known") syncKnown(); });

let selectedWord;
let gesture;
let approvedTap;
const activePointers = new Set();

document.addEventListener("pointerdown", event => {
  activePointers.add(event.pointerId);
  if (activePointers.size > 1 && gesture) gesture.cancelled = true;
  const ruby = event.target.closest("#reading-content ruby[data-lemma]");
  if (ruby && activePointers.size === 1) {
    gesture = { id: event.pointerId, ruby, x: event.clientX, y: event.clientY, time: performance.now(), cancelled: false };
  }
}, { passive: true });
document.addEventListener("pointermove", event => {
  if (gesture?.id === event.pointerId && Math.hypot(event.clientX - gesture.x, event.clientY - gesture.y) > 8) gesture.cancelled = true;
}, { passive: true });
document.addEventListener("pointerup", event => {
  activePointers.delete(event.pointerId);
  if (gesture?.id === event.pointerId) {
    approvedTap = !gesture.cancelled && performance.now() - gesture.time < 500
      && gesture.ruby.contains(event.target) ? { ruby: gesture.ruby, time: performance.now() } : null;
    gesture = null;
  }
}, { passive: true });
document.addEventListener("pointercancel", event => { activePointers.delete(event.pointerId); gesture = null; approvedTap = null; });

document.addEventListener("click", event => {
  const open = event.target.closest("[data-open]");
  if (open) {
    const dialog = document.getElementById(open.dataset.open);
    dialog.showModal();
    $("button", dialog)?.focus({ preventScroll: true });
    return;
  }
  if (event.target.closest("[data-close]")) { event.target.closest("dialog").close(); return; }
  const ruby = event.target.closest("#reading-content ruby[data-lemma]");
  if (!ruby || window.getSelection()?.isCollapsed === false) return;
  if (event.detail !== 0 && (approvedTap?.ruby !== ruby || performance.now() - approvedTap.time > 400)) return;
  selectedWord = {
    lemma: ruby.dataset.lemma, word: $(".word", ruby).textContent,
    gloss: $("rt", ruby).textContent, known: ruby.dataset.known === "true",
  };
  $("#card-word").textContent = selectedWord.word;
  $("#card-gloss").textContent = selectedWord.gloss;
  $("#mark-known").textContent = selectedWord.known ? "恢复注音" : "标为已认识";
  $("#word-card").showModal();
});
document.addEventListener("keydown", event => {
  if ((event.key === "Enter" || event.key === " ") && event.target.matches("ruby[data-lemma]")) {
    event.preventDefault();
    event.target.click();
  }
});

$("#mark-known")?.addEventListener("click", async () => {
  const { lemma, known } = selectedWord;
  const version = (knownVersions.get(lemma) || 0) + 1;
  knownVersions.set(lemma, version);
  pendingKnown.add(lemma);
  setKnown(lemma, !known);
  $("#word-card").close();
  try {
    await api("/api/known", { lemma, known: !known });
    channel?.postMessage("known");
    toast(known ? "已恢复注音" : "已标为认识");
  } catch {
    if (knownVersions.get(lemma) === version) setKnown(lemma, known);
    toast("保存失败，已恢复原来的注音状态");
  } finally {
    if (knownVersions.get(lemma) === version) pendingKnown.delete(lemma);
  }
});
$("#add-vocabulary")?.addEventListener("click", async event => {
  const button = event.currentTarget;
  button.disabled = true;
  try {
    const result = await api("/api/vocabulary", {
      lemma: selectedWord.lemma, word: selectedWord.word, gloss: selectedWord.gloss,
      bookId: article.dataset.bookId, level: null, contextSentence: null,
    });
    $("#word-card").close();
    toast(result.added ? "已加入词汇本" : "已在词汇本中");
  } catch (error) { toast(error.message); }
  finally { button.disabled = false; }
});

// Selection is a browser Range, not a chapter re-render. Exclude rt/rp when
// extracting text: Selection.toString() can include ruby glosses in Chromium.
const selectionToolbar = $("#selection-toolbar");
let selectedExcerpt = null;
let savingExcerpt = false;
let selectionPointerDown = false;
let selectionFrame;

function hideSelection(clear = false) {
  if (selectionToolbar) selectionToolbar.hidden = true;
  if (clear) {
    selectedExcerpt = null;
    for (const id of ["#selection-more", "#selection-ai"]) { const panel = $(id); if (panel) panel.hidden = true; }
    $("#more-selection")?.setAttribute("aria-expanded", "false");
    document.dispatchEvent(new CustomEvent("readwise-selection", { detail: null }));
  }
}

function readSelection() {
  const selection = getSelection();
  if (!article || !selection || selection.isCollapsed || selection.rangeCount !== 1) return null;
  const range = selection.getRangeAt(0).cloneRange();
  if (!article.contains(range.startContainer) || !article.contains(range.endContainer)) return null;
  // Formula source order can be multi-baseline and is not its visual order.
  // Expand boundary selections to the entire visual unit without moving source
  // nodes or changing the persisted UTF-8 coordinate system.
  for (const start of [true, false]) {
    const node = start ? range.startContainer : range.endContainer;
    const math = (node.nodeType === Node.TEXT_NODE ? node.parentElement : node).closest?.('.typst-math');
    if (math && textNodes(math).some(n => range.intersectsNode(n) && !(n === range.startContainer && range.startOffset === n.length) && !(n === range.endContainer && range.endOffset === 0))) {
      if (start) range.setStartBefore(math); else range.setEndAfter(math);
    }
  }
  const parts = [];
  let firstPoint, lastPoint;
  for (const block of $$("[data-block]", article)) {
    if (!range.intersectsNode(block)) continue;
    let text = "";
    const nodes = textNodes(block);
    for (const node of nodes) {
      if (!range.intersectsNode(node)) continue;
      const start = node === range.startContainer ? range.startOffset : 0;
      const end = node === range.endContainer ? range.endOffset : node.length;
      const part = node.textContent.slice(start, end);
      if (part) {
        firstPoint ||= { block: Number(block.dataset.block), offset: byteOffset(nodes, node, start) };
        lastPoint = { block: Number(block.dataset.block), offset: byteOffset(nodes, node, end) };
        text += part;
      }
    }
    if (text) parts.push(text);
  }
  const text = parts.join("\n\n");
  return text.trim() ? { text, range: range.cloneRange(), start: firstPoint, end: lastPoint } : null;
}

function positionSelection() {
  if (!selectedExcerpt || !selectionToolbar || document.querySelector("dialog[open]")) { hideSelection(); return; }
  const viewport = visualViewport || { offsetLeft: 0, offsetTop: 0, width: innerWidth, height: innerHeight };
  const style = getComputedStyle(selectionToolbar);
  const inset = side => parseFloat(style.getPropertyValue(`--safe-${side}`)) || 0;
  const left = viewport.offsetLeft + inset("left") + 12;
  const right = viewport.offsetLeft + viewport.width - inset("right") - 12;
  const top = Math.max(viewport.offsetTop + inset("top") + 12, $(".reader-toolbar").getBoundingClientRect().bottom + 8);
  const bottom = viewport.offsetTop + viewport.height - inset("bottom") - 12;
  const rects = [...selectedExcerpt.range.getClientRects()].filter(rect => rect.width && rect.height && rect.bottom > top && rect.top < bottom && rect.right > left && rect.left < right);
  if (!rects.length) { hideSelection(); return; }
  selectionToolbar.style.maxWidth = `${Math.max(0, right - left)}px`;
  selectionToolbar.style.maxHeight = `${Math.max(44, bottom - top)}px`;
  selectionToolbar.hidden = false;
  const box = selectionToolbar.getBoundingClientRect();
  const rect = rects[0];
  const x = Math.max(left, Math.min(right - box.width, (rect.left + rect.right - box.width) / 2));
  let y = rect.top - box.height - 10;
  if (y < top) y = rect.bottom + 10;
  selectionToolbar.style.left = `${x}px`;
  selectionToolbar.style.top = `${Math.max(top, Math.min(bottom - box.height, y))}px`;
  selectionToolbar.style.setProperty("--pointer-x", `${Math.max(18, Math.min(box.width - 18, (rect.left + rect.right) / 2 - x))}px`);
  selectionToolbar.dataset.placement = y < rect.top ? "above" : "below";
}

function updateSelection() {
  if (selectionPointerDown) return;
  // Keyboard navigation inside the toolbar must not discard its captured Range.
  if (!selectionToolbar.contains(document.activeElement)) {
    selectedExcerpt = readSelection();
  }
  if (!selectedExcerpt) { hideSelection(true); return; }
  const tooLong = encoder.encode(selectedExcerpt.text).length > 24000;
  $("#save-excerpt").disabled = tooLong || savingExcerpt;
  $("#save-excerpt").title = tooLong ? "摘录不能超过 24000 字节，请缩小选区" : "保存到本书摘录笔记";
  positionSelection();
  document.dispatchEvent(new CustomEvent("readwise-selection", { detail: selectedExcerpt }));
}

function scheduleSelection() {
  cancelAnimationFrame(selectionFrame);
  selectionFrame = requestAnimationFrame(updateSelection);
}

if (article && selectionToolbar) {
  document.addEventListener("selectionchange", scheduleSelection);
  document.addEventListener("pointerdown", event => {
    if (selectionToolbar.contains(event.target)) {
      // Keep native selection painted while clicking, without disabling touch
      // handles, scrolling, keyboard selection or the browser context menu.
      if (event.pointerType === "mouse" && event.button === 0 && event.target.closest("button")) event.preventDefault();
      return;
    }
    selectionPointerDown = article.contains(event.target);
    hideSelection(true);
  });
  document.addEventListener("pointerup", () => { selectionPointerDown = false; scheduleSelection(); });
  document.addEventListener("pointercancel", () => { selectionPointerDown = false; scheduleSelection(); });
  window.addEventListener("scroll", positionSelection, { passive: true });
  document.addEventListener("readwise-page", () => hideSelection(true));
  document.addEventListener("readwise-layout", positionSelection);
  window.addEventListener("resize", scheduleSelection);
  visualViewport?.addEventListener("resize", scheduleSelection);
  visualViewport?.addEventListener("scroll", positionSelection);
  document.addEventListener("keydown", event => {
    if (event.key === "Escape" && !selectionToolbar.hidden) {
      hideSelection(true);
      getSelection()?.removeAllRanges();
    }
  });
  $("#dismiss-selection").addEventListener("click", () => {
    hideSelection(true);
    getSelection()?.removeAllRanges();
  });
  $("#copy-selection").addEventListener("click", async () => {
    const excerpt = selectedExcerpt;
    if (!excerpt) return;
    try {
      await navigator.clipboard.writeText(excerpt.text);
      toast("已复制选中文本");
    } catch { toast("复制失败，请使用浏览器的复制菜单或 Ctrl/Cmd+C"); }
  });
  $("#save-excerpt").addEventListener("click", async event => {
    const excerpt = selectedExcerpt;
    if (!excerpt || event.currentTarget.disabled || savingExcerpt) return;
    const button = event.currentTarget;
    savingExcerpt = true;
    button.disabled = true;
    try {
      await api("/api/excerpts", { book_id: article.dataset.bookId, chapter: Number(article.dataset.chapter), text: excerpt.text });
      // A slow save must not clear a different selection made in the meantime.
      if (selectedExcerpt && selectedExcerpt.range.compareBoundaryPoints(Range.START_TO_START, excerpt.range) === 0
        && selectedExcerpt.range.compareBoundaryPoints(Range.END_TO_END, excerpt.range) === 0) {
        hideSelection(true);
        getSelection()?.removeAllRanges();
      }
      toast("已保存，可在阅读页的「摘录」查看");
    } catch (error) {
      toast(`摘录保存失败：${error.message}，选区已保留`);
    } finally {
      savingExcerpt = false;
      button.disabled = Boolean(selectedExcerpt && encoder.encode(selectedExcerpt.text).length > 24000);
    }
  });
}

Promise.all([
  readerReady,
  import(document.body.dataset.appearanceSrc),
  article ? import(document.body.dataset.selectionSrc) : Promise.resolve(null),
  article ? import(document.body.dataset.highlightSrc) : Promise.resolve(null),
]).then(async ([, appearanceModule, selectionModule, highlightModule]) => {
  const appearance = await appearanceModule.initializeAppearance({ $, $$, api, toast, article, captureAnchor, restoreAnchor, getPager: () => pager, channel });
  translate = appearance.t;
  const shelf = $(".shelf-page");
  if (shelf) (await import(shelf.dataset.shelfSrc)).initializeShelf({ api, toast });
  selectionModule?.initializeSelection({ $, $$, api, toast, article, textNodes, textPoint, positionSelection,
    getExcerpt: () => selectedExcerpt, hideSelection, applySaved: appearance.applySaved, channel,
    painter: highlightModule?.createHighlightPainter({ article }) });
  document.body.dataset.browserReady = "true";
}).catch(error => { console.error(error); toast("阅读交互加载失败，请刷新页面"); });

$("#book-upload")?.addEventListener("change", async event => {
  const input = event.currentTarget;
  const files = [...input.files];
  if (!files.length) return;
  input.disabled = true;
  let imported = 0;
  for (const file of files) {
    const item = document.createElement("li");
    item.textContent = `${file.name}：正在导入…`;
    $("#import-results").append(item);
    try {
      if (file.size > 200 * 1024 * 1024) throw new Error("文件超过 200 MB");
      const data = new FormData();
      data.append("file", file);
      const response = await fetch("/api/import", { method: "POST", headers: { "x-readwise-request": "1" }, body: data });
      if (!response.ok) throw new Error(`导入失败（${response.status}），请确认书籍格式有效`);
      const result = await response.json();
      imported++;
      item.dataset.state = "success";
      item.replaceChildren(document.createTextNode(`${file.name}：已入库 · `));
      const link = document.createElement("a");
      link.href = `/read/${result.id}`;
      link.textContent = `阅读《${result.title}》`;
      item.append(link);
    } catch (error) {
      item.dataset.state = "failed";
      item.textContent = `${file.name}：${error.message}`;
    }
  }
  input.value = "";
  input.disabled = false;
  if (imported) {
    toast(`已导入 ${imported} 本书`);
    const refresh = document.createElement("a");
    refresh.href = "/";
    refresh.className = "button";
    refresh.textContent = "刷新书架";
    $("#import-results").after(refresh);
  }
});

document.addEventListener("click", async event => {
  const remove = event.target.closest("[data-remove-vocabulary]");
  const unmark = event.target.closest("[data-unmark-known]");
  const button = remove || unmark;
  if (!button) return;
  button.disabled = true;
  try {
    if (remove) await api("/api/vocabulary/remove", { lemma: remove.dataset.removeVocabulary });
    else {
      await api("/api/known", { lemma: unmark.dataset.unmarkKnown, known: false });
      channel?.postMessage("known");
    }
    location.reload();
  } catch (error) { toast(error.message); button.disabled = false; }
});
