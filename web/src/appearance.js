const palette = { original: ["#FFFFFF", "#202020"], quiet: ["#F2E3C6", "#3B3025"], paper: ["#F7F4EA", "#2B2925"], sepia: ["#F1E6CF", "#33271B"], sage: ["#E7EFE3", "#27342B"], snow: ["#FFFFFF", "#141518"], night: ["#20252B", "#D8D4CA"], theme: ["#D9D9D9", "#333333"] };
const fonts = { canela: "Canela,Georgia,serif", original: "Georgia,'Songti SC','Noto Serif CJK SC',serif", athelas: "Athelas,Georgia,serif", avenir_next: "'Avenir Next',Avenir,sans-serif", charter: "Charter,'Bitstream Charter',serif", georgia: "Georgia,serif", iowan: "'Iowan Old Style',Palatino,serif", palatino: "Palatino,'Palatino Linotype',serif", proxima_nova: "'Proxima Nova',Arial,sans-serif", publico: "Publico,Georgia,serif", san_francisco: "-apple-system,BlinkMacSystemFont,Arial,sans-serif", new_york: "'New York',ui-serif,Georgia,serif", seravek: "Seravek,Arial,sans-serif", times_new_roman: "'Times New Roman',Times,serif" };
const pens = ["#7FB2A688", "#4A90D988", "#E8875A88", "#D9C96A88", "#C1401B88", "#2C58C988", "#3B7A3D88", "#4B4A4788"];
const geometry = ["font", "font_size", "gloss_size", "spacing", "text_width", "justify_text", "indent_paragraphs", "columns", "turn_mode", "bold_text", "character_spacing", "word_spacing", "margin"];
const dictionary = {
  "注音颜色": "Annotation Color", "自定义配色": "Custom Colours", "内嵌取色卡片": "Inline Colour Picker", "收起取色卡片": "Collapse Colour Picker", "饱和度与明度": "Saturation and Brightness", "色相": "Hue", "十六进制": "Hex", "十六进制颜色": "Hex Colour", "恢复预设": "Use Preset", "推荐色": "Suggested Colours", "我的搭配": "My Combinations", "♡ 收藏这套配色": "♡ Favorite this combination", "喜欢的配色可以收藏在这里。": "Keep your favorite combinations here.", "自定义颜色和收藏在点击「应用并保存」后保存。": "Custom colours and favorites are saved only with Apply & Save.",
  "列表": "List", "网格": "Grid", "书架视图": "Library View", "分享": "Share", "加入欲读清单": "Add to Want to Read", "添加到藏书": "Add to Collection", "标记为已读完": "Mark as Finished", "重新命名": "Rename", "删除": "Delete", "书名": "Book title", "保存": "Save", "删除书籍？": "Delete book?", "确认删除": "Confirm Delete", "阅读进度和书内高亮会一并删除，已保存的摘录笔记保留。不会删除应用外的原始 EPUB / PDF。此操作无法撤销。": "Reading progress and book highlights will be deleted; saved excerpt notes are kept. Original EPUB / PDF files outside the app are not affected. This cannot be undone.",
  "阅读": "Read", "书架": "Library", "词汇": "Vocabulary", "我": "Me", "我与设置": "Me & Settings", "阅读设置": "Reading Settings", "设置": "Settings", "让文字呈现你喜欢的样子。": "Make reading your own.",
  "主题": "Theme", "文本": "Text", "注音": "Annotations", "阅读外观": "Reading Appearance", "界面语言": "Interface Language", "中文": "Chinese", "英文": "English", "英文水平": "English Level", "C3-自定义": "C3 - Custom",
  "外观主题": "Appearance", "深色": "Dark", "浅色": "Light", "阅读背景": "Reading Background", "原色": "Original", "静谧": "Quiet", "纸白": "Paper", "米黄": "Sepia", "浅绿": "Calm", "雪白": "Snow", "夜间": "Night", "跟随主题": "Follow Theme",
  "笔颜色": "Pen", "朱红": "Vermilion", "钴蓝": "Cobalt", "苔绿": "Moss", "石墨": "Graphite", "高亮颜色": "Highlight Color", "黄": "Yellow", "绿": "Green", "粉": "Pink", "经典色（保留）": "Classic colour (preserved)",
  "正文字体": "Font", "正文字号": "Text Size", "正文行距": "Line Spacing", "正文宽度": "Text Width", "窄": "Narrow", "适中": "Even", "宽": "Wide", "对齐": "Alignment", "左对齐": "Left", "两端对齐": "Justified", "段落": "Paragraphs", "段间留白": "Spaced", "首行缩进": "Indented", "正文分栏": "Columns", "单列": "One", "双列": "Two", "自动": "Auto", "翻页方式": "Page Turn", "仿真翻页": "Page Curl", "左右滑动": "Slide", "上下滚动": "Scroll",
  "显示注音": "Show Annotations", "注音字号": "Annotation Size", "注音字色": "Annotation Color", "注音背景": "Annotation Background", "词字色": "Word Color", "词背景": "Word Background", "青": "Teal", "蓝": "Blue", "橙": "Orange", "灰": "Gray", "无": "None", "默认": "Default", "奶黄": "Cream", "薄荷": "Mint", "雾蓝": "Mist Blue",
  "↶ 重置主题设置": "↶ Reset Theme", "↶ 重置文本设置": "↶ Reset Text", "↶ 重置注音设置": "↶ Reset Annotations", "应用并保存": "Apply & Save", "取消": "Cancel", "关闭": "Close", "阅读设置已保存": "Reading settings saved",
  "载入桌面阅读设置": "Load Desktop Reading Settings", "只读载入此电脑的桌面设置为草稿，点击应用后保存到 Web；不会修改桌面数据库或同步书籍。": "Load a read-only snapshot from this computer. Apply to save to Web. Desktop data and books are not changed.",
  "水平越高，注音越少；C3 仅注音有自定义释义的导入词，已认识词不注音。": "Higher levels show fewer annotations. C3 uses imported custom glosses; known words are hidden.", "字体使用本机已安装版本；不可用时使用同类后备字体。": "Uses locally installed fonts, with matching fallbacks when unavailable.", "窄屏自动回退单栏；上下滚动采用单栏，分栏用于分页模式。分页可使用底部按钮或左右方向键，选文时不拦截方向键。": "Columns apply to paged reading; narrow screens and scrolling use one column. Use page buttons or arrow keys; selection keys are not intercepted.",
  "颜色即时预览，点击应用后保存。修改英语水平会重新加载当前章节；其他外观修改不重载正文。": "Preview changes, then apply to save. Only an English-level change reloads the chapter.",
  "问 AI": "Ask AI", "解释": "Explain", "释义": "Define", "复制": "Copy", "保存摘录": "Save Excerpt", "生词": "Vocabulary", "翻译": "Translate", "发音": "Speak", "移除高亮": "Remove Highlight", "添加笔记": "Add Note", "保存笔记": "Save Note", "我的想法": "My Thoughts", "定义与用法": "Definition & Usage", "事实核查": "Fact Check", "查找书中相关内容": "Find Related in Book", "提出反驳": "Counterpoint", "复制引文及出处": "Copy Quote & Source", "翻译与事实核查会在外部网站打开选文。": "Translation and fact-checking open your selection on an external website.",
  "为什么用这个词？": "Why this word?", "这个词源自哪里？": "Its etymology?", "用它造个句子": "Use it in a sentence", "阅读助教": "Reading Tutor", "配置 AI 接口": "Configure AI", "书内搜索": "Search in Book", "关于这段选文提问…": "Ask about this passage…", "写下想法（可选）": "Your thoughts (optional)",
  "AI 设置": "AI Settings", "接口地址": "Endpoint", "模型": "Model", "清除已保存密钥": "Clear Saved Key", "保存 AI 设置": "Save AI Settings", "留空保留已保存的密钥": "Leave blank to keep the saved key", "选文的问 AI、解释和释义共用 OpenAI 兼容接口。仅在点击发送时将选文发送到你配置的服务。": "Ask AI, Explain and Define use your OpenAI-compatible service. Selected text is sent only when you submit a request.",
  "密钥已保存（不回传浏览器）": "Key saved (never sent back to the browser)", "尚未保存密钥；本机无鉴权模型可留空": "No key saved; leave blank for a local service without authentication", "AI 设置已保存": "AI settings saved", "Web 预览版 · Topcoat 0.10 · 本机单用户。数据与原生客户端默认独立，暂不提供云同步。": "Web preview · Topcoat 0.10 · Single user, loopback only. Desktop data is separate; cloud sync is not provided.",
  "更多选文操作": "More Selection Actions", "添加或移除高亮": "Add or Remove Highlight", "添加高亮": "Add Highlight", "取消选文": "Dismiss Selection", "向 AI 提问": "Ask AI a Question", "发送问题": "Send Question", "返回书架": "Back to Library", "关闭词卡": "Close Word Card", "选文操作": "Selection Actions", "阅读设置分区": "Reader Settings Sections", "主题设置": "Theme Settings", "文本设置": "Text Settings", "注音设置": "Annotation Settings",
  "词汇本": "Vocabulary", "把遇见的词留下来": "Keep the words you meet", "阅读时点击带注音的单词，选择「加入词汇本」。": "Tap an annotated word while reading and choose Add to Vocabulary.", "个收藏词 ·": "saved words ·", "个已认识词": "known words", "已认识词（": "Known words (", "）": ")", "移除": "Remove", "移出词汇本": "Remove from Vocabulary", "阅读记录": "Reading History", "← 返回我与设置": "← Back to Me & Settings", "来自当前 Web 数据库的真实统计。": "Real statistics from the current Web database.", "藏书": "Books", "在读": "Reading", "词汇收藏": "Saved Words", "笔记": "Notes", "本阶段尚未接入 Web 阅读计时及已有笔记编辑；不会用模拟数据代替。": "Reading timers and editing existing notes are not available yet. No statistics are simulated.", "跳到正文": "Skip to Content", "主导航": "Main Navigation", "正文翻页": "Page Navigation", "章节目录": "Table of Contents", "从一本书开始": "Start with a book", "本书，慢慢读。": "books. Take your time.", "＋ 导入书籍": "+ Import Books", "支持 EPUB / PDF，每本最大 200 MB。文件保存在当前 Web 服务的数据目录。": "EPUB / PDF, up to 200 MB per book. Files are stored in this Web server's data directory.", "导入书籍后，即可阅读、显示生词注音，并建立自己的词汇本。": "Import a book to start reading with annotations and build your vocabulary.", "继续阅读": "Continue Reading", "开始阅读": "Start Reading", "选中正文后，使用悬浮工具栏保存摘录。": "Select text and use the floating toolbar to save an excerpt.", "已载入桌面设置草稿，点击应用后保存": "Desktop settings loaded as a draft. Apply to save.", "已加入词汇本": "Added to vocabulary", "已保存摘录": "Excerpt saved", "已保存笔记": "Note saved", "已添加高亮": "Highlight added", "已移除高亮": "Highlight removed", "已标为认识": "Marked as known",
  "← 书架": "← Library", "目录": "Contents", "摘录": "Excerpts", "摘录笔记": "Excerpt Notes", "← 返回阅读": "← Back to Reading", "← 上一页": "← Previous Page", "下一页 →": "Next Page →", "← 上一章": "← Previous Chapter", "下一章 →": "Next Chapter →", "本书完": "End of Book", "标为已认识": "Mark as Known", "恢复注音": "Restore Annotation", "加入词汇本": "Add to Vocabulary", "阅读记录 →": "Reading History →",
};

export async function initializeAppearance({ $, $$, api, toast, article, captureAnchor, restoreAnchor, getPager, channel }) {
  const { annotationProperties, createCustomColours } = await import(document.body.dataset.coloursSrc);
  let current = JSON.parse(document.body.dataset.preferences);
  const originals = new WeakMap();
  const t = (message, fallback) => {
    if (current.language !== "en") return message;
    if (Object.hasOwn(dictionary, message)) return dictionary[message];
    const parts = message.split("：");
    return parts.length === 2 && Object.hasOwn(dictionary, parts[0]) && Object.hasOwn(dictionary, parts[1]) ? `${dictionary[parts[0]]}: ${dictionary[parts[1]]}` : (fallback || message);
  };
  function localize() {
    document.documentElement.lang = current.language;
    document.documentElement.style.colorScheme = document.body.style.colorScheme || current.theme;
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    while (walker.nextNode()) {
      const node = walker.currentNode;
      if (node.parentElement.closest("script,style,svg,#reading-content,[data-preview],.book-card h2,.book-card p,.cover-title,.reader-book-title,.chapter-navigation > span:nth-child(2),#toc-dialog nav a,.vocabulary-entry > div,.known-list li span,#card-word,#card-gloss,.excerpt-text,#ai-quote,#ai-answer,#note-quote,#search-results,[data-live-message]")) continue;
      const original = originals.get(node) || node.textContent;
      if (Object.hasOwn(dictionary, original.trim())) { originals.set(node, original); node.textContent = original.replace(original.trim(), t(original.trim())); }
    }
    for (const node of $$("[data-live-message]")) node.textContent = t(node.dataset.liveMessage);
    if (article) {
      const kicker = $(".chapter-kicker", article), p = current.language === "en" ? `Chapter ${Number(article.dataset.chapter) + 1} / ${article.dataset.total}` : `第 ${Number(article.dataset.chapter) + 1} / ${article.dataset.total} 章`;
      if (kicker && kicker.textContent !== p) kicker.textContent = p;
    }
    for (const node of $$("[placeholder],[aria-label],[title]")) {
      if (node.closest("#reading-content")) continue;
      for (const attr of ["placeholder", "aria-label", "title"]) {
        const key = `data-original-${attr}`;
        if (node.hasAttribute(attr)) {
          const original = node.getAttribute(key) || node.getAttribute(attr);
          if (t(original) !== original || node.hasAttribute(key)) { node.setAttribute(key, original); node.setAttribute(attr, t(original)); }
        }
      }
    }
  }
  function liveMessage(selector, message) {
    const node = $(selector); node.dataset.liveMessage = message; node.textContent = t(message);
  }
  function textStyle(target, p) {
    const vars = { "--font-size": `${p.font_size}px`, "--gloss-size": `${p.gloss_size}px`, "--reader-font": fonts[p.font], "--reader-leading": p.spacing, "--reader-align": p.justify_text ? "justify" : "left", "--reader-indent": p.indent_paragraphs ? "2em" : "0", "--paragraph-gap": p.indent_paragraphs ? "0" : "0.9em", "--reader-weight": p.bold_text ? 600 : 400, "--letter-spacing": `${p.character_spacing}em`, "--word-spacing": `${p.word_spacing}em`, "--selection-ink": pens[p.selection_color_slot] };
    for (const [key, value] of Object.entries({ ...vars, ...annotationProperties(p) })) target.style.setProperty(key, value);
    target.dataset.glossEnabled = String(p.gloss_enabled);
  }
  function sceneStyle(target, p, reader) {
    const [paper, ink] = reader ? p.background === "theme" && p.theme === "dark" ? ["#4D4D4D", "#D9D9D9"] : palette[p.background] : p.theme === "dark" ? ["#252729", "#E8E4DC"] : ["#FAF7F0", "#302F2A"];
    const dark = reader ? p.background === "night" || p.background === "theme" && p.theme === "dark" : p.theme === "dark";
    const vars = { "--paper": paper, "--ink": ink, "--panel": dark ? "#303439" : "#FFFDF8", "--muted": dark ? "#B4AD9E" : "#7A776D", "--line": dark ? "#4B4A46" : "#E7E1D5", "--accent": dark ? "#D6B17D" : "#9C5039" };
    for (const [key, value] of Object.entries(vars)) target.style.setProperty(key, value);
    target.style.colorScheme = dark ? "dark" : "light";
  }
  function values(root) {
    const result = JSON.parse(root.dataset.draftSeed || root.dataset.initial);
    for (const node of $$("input[name],select[name]", root)) {
      if (node.type === "radio" && !node.checked || node.type === "range" && !node.dataset.edited) continue;
      result[node.name] = typeof result[node.name] === "boolean" ? node.type === "checkbox" ? node.checked : node.value === "true" : typeof result[node.name] === "number" ? Number(node.value) : node.value;
    }
    for (const field of ["gloss_color", "gloss_background", "word_color", "word_background"]) {
      const selected = $(`[data-field="${field}"] [data-on="true"]`, root);
      if (selected) result[field] = Number(selected.dataset.slot);
    }
    Object.assign(result, root._colourEditor?.getDraft());
    return result;
  }
  function preview(root) {
    const p = values(root);
    root._colourEditor?.refresh(p);
    for (const target of $$("[data-preview]", root)) { textStyle(target, p); sceneStyle(target, p, true); }
    for (const node of $$("input[type=range]", root)) {
      const output = $(`[data-output="${node.name}"]`, root);
      if (output) output.textContent = String(Math.round(Number(p[node.name]) / Number(node.dataset.scale || 1) * 100) / 100);
    }
  }
  function draft(root, p) {
    for (const node of $$("input[name],select[name]", root)) {
      delete node.dataset.edited;
      if (node.type === "radio") node.checked = node.value === String(p[node.name]);
      else if (node.type === "checkbox") node.checked = p[node.name];
      else node.value = p[node.name];
    }
    // Preserve fields without visible controls in this form (e.g. global-only).
    root.dataset.draftSeed = JSON.stringify(p);
    const selectPresets = () => {
      for (const field of ["gloss_color", "gloss_background", "word_color", "word_background"]) $(`[data-field="${field}"] [data-slot="${p[field]}"]`, root).click();
    };
    if (root._colourEditor) root._colourEditor.setDraft(p, selectPresets);
    else selectPresets();
    preview(root);
  }
  async function applySaved(p, { notify = false } = {}) {
    const old = current;
    const changedGeometry = geometry.some(key => p[key] !== old[key]);
    const anchor = changedGeometry ? captureAnchor() : null;
    current = p; document.body.dataset.preferences = JSON.stringify(p);
    sceneStyle(document.body, p, Boolean(article || document.querySelector(".excerpts-page")));
    if (article) {
      textStyle(article, p);
      article.dataset.preferences = JSON.stringify(p);
      if (changedGeometry) {
        getPager()?.configure(p, anchor);
        await document.fonts.ready;
        await new Promise(resolve => requestAnimationFrame(resolve));
        if (anchor) restoreAnchor(anchor);
      }
    }
    for (const root of $$("[data-appearance]")) {
      // An open draft survives cross-tab notifications; cancellation uses latest.
      root.dataset.initial = JSON.stringify(p);
      if (!root.closest("dialog[open]")) draft(root, p);
    }
    localize();
    document.dispatchEvent(new CustomEvent("readwise-preferences", { detail: p }));
    if (notify) channel?.postMessage("preferences");
    return old;
  }
  // Dynamic asset imports can beat Topcoat's hydration on a cold direct load.
  // Wait for the four signal projections instead of reading missing attributes.
  await Promise.all($$("[data-appearance]").map(root => new Promise((resolve, reject) => {
    if ($$('[data-field] [data-on="true"]', root).length === 4) { resolve(); return; }
    const timer = setTimeout(() => { observer.disconnect(); reject(new Error("注音信号初始化超时")); }, 6000);
    const observer = new MutationObserver(() => {
      if ($$('[data-field] [data-on="true"]', root).length === 4) { clearTimeout(timer); observer.disconnect(); resolve(); }
    });
    observer.observe(root, { subtree: true, attributes: true, attributeFilter: ["data-on"] });
  })));
  for (const root of $$("[data-appearance]")) {
    root._colourEditor = createCustomColours(root, { t, read: () => values(root), changed: () => preview(root) });
    draft(root, JSON.parse(root.dataset.initial));
    new MutationObserver(() => preview(root)).observe(root, { subtree: true, attributes: true, attributeFilter: ["data-on"] });
    // HTML range inputs round imported fractional values to their step. Keep the
    // canonical draft until that slider is actually edited; colour-only saves
    // must not silently change font geometry or native custom line spacing.
    root.addEventListener("input", event => { if (event.target.matches('input[type="range"]')) event.target.dataset.edited = "true"; preview(root); });
    const tabs = $$('[data-settings-tab]', root);
    function showTab(name, focus = false) {
      for (const tab of tabs) { const selected = tab.dataset.settingsTab === name; tab.setAttribute("aria-selected", String(selected)); tab.tabIndex = selected ? 0 : -1; if (focus && selected) tab.focus(); }
      for (const section of $$("[data-settings-section]", root)) section.hidden = root.dataset.global !== "true" && section.dataset.settingsSection !== name;
    }
    tabs.forEach(tab => tab.addEventListener("click", () => showTab(tab.dataset.settingsTab)));
    $(".appearance-tabs", root)?.addEventListener("keydown", event => {
      if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
      event.preventDefault(); const index = tabs.indexOf(document.activeElement);
      const next = event.key === "Home" ? 0 : event.key === "End" ? 2 : (index + (event.key === "ArrowRight" ? 1 : 2)) % 3;
      showTab(tabs[next].dataset.settingsTab, true);
    });
    for (const button of $$("[data-reset-settings]", root)) button.addEventListener("click", () => {
      const p = values(root), defaults = JSON.parse(root.dataset.defaults);
      const fields = button.dataset.resetSettings === "theme" ? ["background", "selection_color_slot", "highlight_color_slot"] : button.dataset.resetSettings === "text" ? ["font", "font_size", "spacing", "text_width", "justify_text", "indent_paragraphs", "columns", "bold_text", "character_spacing", "word_spacing", "margin"] : ["gloss_enabled", "gloss_size", "gloss_color", "gloss_background", "word_color", "word_background", "custom_colors"];
      for (const field of fields) p[field] = defaults[field];
      const saved = root.dataset.initial; draft(root, p); root.dataset.initial = saved;
    });
    root.closest("dialog")?.addEventListener("close", () => draft(root, current));
    $("[data-import-desktop]", root)?.addEventListener("click", async event => {
      const button = event.currentTarget; button.disabled = true;
      try { const p = await api("/api/preferences/desktop", null, { method: "GET" }); const saved = root.dataset.initial; p.favorites = values(root).favorites; draft(root, p); root.dataset.initial = saved; root.dataset.importedDraft = JSON.stringify(p); toast("已载入桌面设置草稿，点击应用后保存"); }
      catch (error) { toast(error.message); } finally { button.disabled = false; }
    });
    $("[data-save-appearance]", root).addEventListener("click", async event => {
      const button = event.currentTarget; button.disabled = true;
      try {
        if (!root._colourEditor.validate()) throw new Error(t("请输入 #RRGGBB 格式的颜色", "Enter a colour as #RRGGBB"));
        const submitted = values(root);
        const saved = await api("/api/preferences", submitted);
        const old = await applySaved(saved, { notify: true });
        root.closest("dialog")?.close(); delete root.dataset.importedDraft;
        toast("阅读设置已保存");
        if (article && saved.english_level !== old.english_level) { await api("/api/progress", { book_id: article.dataset.bookId, location: captureAnchor() }); location.reload(); }
      } catch (error) { toast(error.message); } finally { button.disabled = false; }
    });
    preview(root);
  }
  channel?.addEventListener("message", async event => {
    if (event.data === "preferences") { try { await applySaved(await api("/api/preferences", null, { method: "GET" })); } catch { /* keep last projection */ } }
  });
  const aiForm = $("#ai-settings-form");
  if (aiForm) {
    api("/api/ai/settings", null, { method: "GET" }).then(p => { aiForm.elements.endpoint.value = p.endpoint; aiForm.elements.model.value = p.model; liveMessage("#ai-config-status", p.has_api_key ? "密钥已保存（不回传浏览器）" : "尚未保存密钥；本机无鉴权模型可留空"); }).catch(error => { liveMessage("#ai-config-status", error.message); });
    aiForm.addEventListener("submit", async event => {
      event.preventDefault(); const button = $("button[type=submit]", aiForm); button.disabled = true;
      try { await api("/api/ai/settings", { endpoint: aiForm.elements.endpoint.value, model: aiForm.elements.model.value, api_key: aiForm.elements.api_key.value || null, clear_key: aiForm.elements.clear_key.checked }); aiForm.elements.api_key.value = ""; aiForm.elements.clear_key.checked = false; liveMessage("#ai-config-status", "AI 设置已保存"); }
      catch (error) { liveMessage("#ai-config-status", error.message); } finally { button.disabled = false; }
    });
  }
  localize();
  return { applySaved, t };
}
