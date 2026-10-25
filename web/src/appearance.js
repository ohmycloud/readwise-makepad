const palette = { original: ["#FFFFFF", "#202020"], quiet: ["#F2E3C6", "#3B3025"], paper: ["#F7F4EA", "#2B2925"], sepia: ["#F1E6CF", "#33271B"], sage: ["#E7EFE3", "#27342B"], snow: ["#FFFFFF", "#141518"], night: ["#20252B", "#D8D4CA"], theme: ["#D9D9D9", "#333333"] };
const fonts = { canela: "Canela,Georgia,serif", original: "Georgia,'Songti SC','Noto Serif CJK SC',serif", athelas: "Athelas,Georgia,serif", avenir_next: "'Avenir Next',Avenir,sans-serif", charter: "Charter,'Bitstream Charter',serif", georgia: "Georgia,serif", iowan: "'Iowan Old Style',Palatino,serif", palatino: "Palatino,'Palatino Linotype',serif", proxima_nova: "'Proxima Nova',Arial,sans-serif", publico: "Publico,Georgia,serif", san_francisco: "-apple-system,BlinkMacSystemFont,Arial,sans-serif", new_york: "'New York',ui-serif,Georgia,serif", seravek: "Seravek,Arial,sans-serif", times_new_roman: "'Times New Roman',Times,serif" };
const pens = ["#7FB2A688", "#4A90D988", "#E8875A88", "#D9C96A88", "#C1401B88", "#2C58C988", "#3B7A3D88", "#4B4A4788"];
const geometry = ["font", "font_size", "gloss_size", "spacing", "text_width", "justify_text", "indent_paragraphs", "columns", "turn_mode", "bold_text", "character_spacing", "word_spacing", "margin"];

export async function initializeAppearance({ $, $$, api, toast, article, captureAnchor, restoreAnchor, getPager, channel }) {
  const { annotationProperties, createCustomColours } = await import(document.body.dataset.coloursSrc);
  const { createI18n } = await import(document.body.dataset.i18nSrc);
  const i18n = createI18n(), { t } = i18n;
  let current = JSON.parse(document.body.dataset.preferences);
  function localize() {
    i18n.localize(current.language);
    document.documentElement.style.colorScheme = document.body.style.colorScheme || current.theme;
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
    // Preserve canonical fractional settings until a slider is actually edited.
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
        if (!root._colourEditor.validate()) throw new Error(t("请输入 #RRGGBB 格式的颜色"));
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
  return { applySaved, t, format: i18n.format };
}
