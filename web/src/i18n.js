// Fluent resolves both catalogs on the server. Only explicit interface markers
// are updated on a saved language change; book text is never traversed.
export function createI18n() {
  const catalogs = JSON.parse(document.body.dataset.uiMessages);
  const t = (message, fallback) => {
    const messages = catalogs[document.documentElement.lang] || catalogs["zh-CN"];
    if (Object.hasOwn(messages, message)) return messages[message];
    const parts = message.split("：");
    if (parts.length === 2 && parts.every(part => Object.hasOwn(messages, part))) {
      return parts.map(part => messages[part]).join(document.documentElement.lang === "en" ? ": " : "：");
    }
    return document.documentElement.lang === "en" ? (fallback || message) : message;
  };
  // Templates are Fluent messages resolved with named placeholder arguments.
  // Substitution is text-only, never HTML or JavaScript evaluation.
  const format = (key, args) => t(key).replace(/\{(\w+)\}/g, (match, name) => {
    if (!Object.hasOwn(args, name)) return match;
    const value = args[name];
    return typeof value === "number" && Number.isFinite(value)
      ? new Intl.NumberFormat(document.documentElement.lang).format(value) : String(value);
  });
  const localize = language => {
    document.documentElement.lang = language;
    for (const node of document.querySelectorAll("[data-i18n],[data-i18n-format],[data-live-message]")) {
      if (node.closest(".reader-block,.excerpt-text,[data-preview],#card-word,#card-gloss")) continue;
      const value = node.hasAttribute("data-i18n-format")
        ? format(node.dataset.i18nFormat, JSON.parse(node.dataset.i18nArgs))
        : t(node.dataset.i18n || node.dataset.liveMessage);
      if (node.textContent !== value) node.textContent = value;
    }
    for (const [attribute, marker] of [["aria-label", "data-i18n-aria-label"], ["placeholder", "data-i18n-placeholder"], ["title", "data-i18n-title-attribute"]]) {
      for (const node of document.querySelectorAll(`[${marker}]`)) {
        if (!node.closest(".reader-block,.excerpt-text,[data-preview]")) node.setAttribute(attribute, t(node.getAttribute(marker)));
      }
    }
    const title = document.querySelector("title[data-i18n-title]");
    if (title) document.title = `${t(title.dataset.i18nTitle)} · ReadWise`;
  };
  return { t, format, localize };
}
