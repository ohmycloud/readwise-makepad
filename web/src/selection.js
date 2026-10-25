// Native toolbar parity, using paint-only CSS Highlights and Web-owned offsets.
export function initializeSelection({ $, $$, api, toast, article, textPoint, positionSelection, getExcerpt, hideSelection, applySaved, channel, painter }) {
  let annotations = [], generation = 0, annotationBusy = false, deferredRefresh = false, noteContext, aiController;
  let marker = Math.max(0, Number(JSON.parse(document.body.dataset.preferences).highlight_color_slot) - 4);
  const supportsHighlights = Boolean(globalThis.CSS?.highlights && globalThis.Highlight);
  const compare = (a, b) => a.block - b.block || a.offset - b.offset;
  const overlaps = (a, b) => compare(a.start, b.end) < 0 && compare(b.start, a.end) < 0;
  const base = { book_id: article.dataset.bookId, chapter: Number(article.dataset.chapter) };
  const hint = message => { const node = $("#selection-hint"); node.textContent = message; node.hidden = !message; positionSelection(); };
  function domRange(annotation) {
    const first = article.querySelector(`[data-block="${annotation.start.block}"]`), last = article.querySelector(`[data-block="${annotation.end.block}"]`);
    if (!first || !last) return null;
    const start = textPoint(first, annotation.start.offset), end = textPoint(last, annotation.end.offset);
    if (!start || !end) return null;
    const range = document.createRange(); range.setStart(start.node, start.offset); range.setEnd(end.node, end.offset); return range;
  }
  function highlightState(excerpt = getExcerpt()) {
    const exists = excerpt && annotations.some(a => overlaps(a, excerpt));
    $("#toggle-highlight").setAttribute("aria-pressed", String(Boolean(exists)));
    $("#toggle-highlight").title = exists ? "移除高亮" : "添加高亮";
  }
  function paint() {
    highlightState();
    if (!supportsHighlights) return;
    painter.setSaved(annotations.map(a => ({ range: domRange(a), color: a.color })).filter(a => a.range));
  }
  async function refresh() {
    if (annotationBusy) { deferredRefresh = true; return; }
    const version = ++generation;
    try { const result = await api(`/api/highlights/${base.book_id}/${base.chapter}`, null, { method: "GET" }); if (version === generation) { annotations = result; paint(); } }
    catch { /* keep existing paint while disconnected */ }
  }
  function markerState() {
    for (const button of $$("[data-marker]")) button.setAttribute("aria-pressed", String(Number(button.dataset.marker) === marker));
  }
  markerState(); refresh();
  document.addEventListener("readwise-preferences", event => { marker = Math.max(0, Math.min(3, event.detail.highlight_color_slot - 4)); markerState(); });
  document.addEventListener("readwise-selection", event => {
    const excerpt = event.detail;
    painter.setSelection(excerpt?.range || null);
    highlightState(excerpt);
  });
  channel?.addEventListener("message", event => { if (event.data === `highlights:${base.book_id}:${base.chapter}`) refresh(); });
  function currentSelection(excerpt) {
    const current = getExcerpt();
    return current && current.range.compareBoundaryPoints(Range.START_TO_START, excerpt.range) === 0
      && current.range.compareBoundaryPoints(Range.END_TO_END, excerpt.range) === 0;
  }
  async function highlight(removeOnly = false, colour = null) {
    // Capture the quote before either asynchronous request. A later selection
    // must neither change this operation's target nor be cleared by its success.
    const excerpt = getExcerpt(); if (!excerpt || annotationBusy) return;
    if (!supportsHighlights) { hint("当前浏览器不支持高亮绘制，请使用新版浏览器"); return; }
    const ids = annotations.filter(a => overlaps(a, excerpt)).map(a => a.id);
    if (removeOnly && !ids.length) { hint("选区没有已保存的高亮"); return; }
    annotationBusy = true; ++generation;
    const controls = $$("[data-marker],#toggle-highlight,[data-selection-utility=remove]");
    controls.forEach(node => { node.disabled = true; }); hint("");
    const removing = colour === null && ids.length > 0;
    try {
      if (colour !== null) {
        const p = await api("/api/marker", { slot: colour + 4 }); await applySaved(p, { notify: true });
        // Applying a swatch is not the pen's add/remove toggle. Replace only the
        // covered colour range, atomically preserving unselected old fragments.
        annotations = await api("/api/highlights/apply", { ...base, start: excerpt.start, end: excerpt.end, color: colour });
      } else if (removing) {
        await api("/api/highlights/remove", { ...base, ids }); annotations = annotations.filter(a => !ids.includes(a.id));
      } else {
        const result = await api("/api/highlights", { ...base, start: excerpt.start, end: excerpt.end, color: marker }); annotations.push(result);
      }
      paint(); channel?.postMessage(`highlights:${base.book_id}:${base.chapter}`);
      if (currentSelection(excerpt)) { hideSelection(true); getSelection()?.removeAllRanges(); }
      toast(removing ? "已移除高亮" : "已保存高亮");
    } catch (error) {
      const message = `高亮保存失败：${error.message}`;
      if (currentSelection(excerpt)) hint(message); else toast(message);
    } finally {
      annotationBusy = false; controls.forEach(node => { node.disabled = false; });
      if (deferredRefresh) { deferredRefresh = false; refresh(); }
    }
  }
  $("#toggle-highlight").addEventListener("click", () => highlight());
  for (const button of $$("[data-marker]")) button.addEventListener("click", () => highlight(false, Number(button.dataset.marker)));
  $("#more-selection").addEventListener("click", () => {
    const panel = $("#selection-more"); panel.hidden = !panel.hidden; $("#selection-ai").hidden = true;
    $("#more-selection").setAttribute("aria-expanded", String(!panel.hidden)); positionSelection();
  });
  $("#ask-selection").addEventListener("click", () => {
    $("#selection-ai").hidden = !$("#selection-ai").hidden; $("#selection-more").hidden = true;
    $("#more-selection").setAttribute("aria-expanded", "false"); positionSelection();
    if (!$("#selection-ai").hidden) $("#selection-question").focus({ preventScroll: true });
  });
  async function ask(question) {
    const excerpt = getExcerpt(); if (!excerpt || !question.trim()) { hint("请先输入问题"); return; }
    aiController?.abort(); aiController = new AbortController(); const controller = aiController;
    $("#ai-quote").textContent = excerpt.text; $("#ai-prompt").textContent = question;
    $("#ai-answer").textContent = "正在请求 AI…"; $("#ai-dialog").showModal();
    try { const result = await api("/api/ai/ask", { ...base, text: excerpt.text, question }, { signal: controller.signal }); if (!controller.signal.aborted) $("#ai-answer").textContent = result.answer; }
    catch (error) { if (!controller.signal.aborted) $("#ai-answer").textContent = `请求失败：${error.message}\n请检查全局设置中的 AI 接口、模型和密钥。`; }
  }
  $("#ai-dialog").addEventListener("close", () => { aiController?.abort(); positionSelection(); });
  $("#selection-question-form").addEventListener("submit", event => { event.preventDefault(); ask($("#selection-question").value); });
  for (const button of $$("[data-selection-question]")) button.addEventListener("click", () => ask(button.dataset.selectionQuestion));
  $("#compose-note").addEventListener("click", () => {
    const excerpt = getExcerpt(); if (!excerpt) return;
    noteContext = excerpt; $("#note-quote").textContent = excerpt.text; $("#note-comment").value = ""; $("#note-status").textContent = "";
    $("#note-dialog").showModal(); $("#note-comment").focus({ preventScroll: true });
  });
  $("#note-form").addEventListener("submit", async event => {
    event.preventDefault(); const button = $("button[type=submit]", event.currentTarget);
    if (!noteContext || button.disabled) return;
    const note = $("#note-comment").value;
    if (new TextEncoder().encode(note).length > 12000) { $("#note-status").textContent = "笔记不得超过 12000 字节"; return; }
    button.disabled = true;
    try { await api("/api/excerpts", { ...base, text: noteContext.text, note }); $("#note-dialog").close(); hideSelection(true); getSelection()?.removeAllRanges(); toast("笔记已保存，可在「摘录」查看"); }
    catch (error) { $("#note-status").textContent = `保存失败：${error.message}`; }
    finally { button.disabled = false; }
  });
  const open = url => window.open(url, "_blank", "noopener,noreferrer");
  for (const button of $$("[data-selection-utility]")) button.addEventListener("click", async () => {
    const excerpt = getExcerpt(); if (!excerpt) return;
    const text = excerpt.text.trim();
    try {
      switch (button.dataset.selectionUtility) {
        case "remove": await highlight(true); break;
        case "quote": {
          const title = $(".reader-book-title").textContent;
          await navigator.clipboard.writeText(`“${text}”\n——《${title}》，第 ${base.chapter + 1} 章，第 ${excerpt.start.block + 1} 段`); hint("已复制引文及出处"); break;
        }
        case "translate": open(`https://translate.google.com/?sl=auto&tl=zh-CN&text=${encodeURIComponent(text)}&op=translate`); break;
        case "fact": open(`https://www.google.com/search?q=${encodeURIComponent(text)}`); break;
        case "web": open(`https://www.google.com/search?q=${encodeURIComponent(`${text} ${$("#selection-question").value}`)}`); break;
        case "vocabulary": {
          if (!/^[A-Za-z]+(?:['’\-][A-Za-z]+)*$/.test(text) || text.length > 80) { hint("加入生词仅支持选择一个英文单词"); break; }
          const result = await api("/api/vocabulary", { lemma: text.toLowerCase(), word: text, gloss: null, bookId: base.book_id, level: null, contextSentence: null }); hint(result.added ? "已加入词汇本" : "已在词汇本中"); break;
        }
        case "speak": {
          if (!globalThis.speechSynthesis) { hint("当前浏览器不支持发音"); break; }
          speechSynthesis.cancel(); const speech = new SpeechSynthesisUtterance(text); speech.lang = /[\u3400-\u9fff]/.test(text) ? "zh-CN" : "en-US"; speech.onerror = () => toast("发音不可用，请检查系统语音"); speechSynthesis.speak(speech); hint("正在发音…"); break;
        }
        case "related": {
          $("#search-results").replaceChildren(); $("#search-status").textContent = "正在搜索…"; $("#search-dialog").showModal();
          const result = await api("/api/book-search", { book_id: base.book_id, query: [...text].slice(0, 100).join("") });
          $("#search-status").textContent = result.results.length ? `找到 ${result.results.length} 条（最多显示 100 条）` : "没有找到相关内容";
          for (const item of result.results) { const link = document.createElement("a"); link.className = "search-result"; link.href = item.href; const heading = document.createElement("strong"); heading.textContent = item.title; const snippet = document.createElement("p"); snippet.textContent = item.text; link.append(heading, snippet); $("#search-results").append(link); }
          break;
        }
      }
    } catch (error) { hint(error.message); if ($("#search-dialog").open) $("#search-status").textContent = error.message; }
  });
}
