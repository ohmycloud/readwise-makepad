// Inline, client-side colour drafts. No network request until Apply & Save.
const fields = ["gloss_color", "gloss_background", "word_color", "word_background"];
const titles = ["注音颜色", "注音背景", "词字色", "词背景"];
const presets = [["#7FB2A6", "#62B6FF", "#E8875A", "#9A9A9A"], ["transparent", "#FFE9A8", "#CFE8D5", "#D3E6F7"], ["inherit", "#7FB2A6", "#62B6FF", "#E8875A"], ["transparent", "#FFE9A8", "#CFE8D5", "#D3E6F7"]];
const copy = value => JSON.parse(JSON.stringify(value));
const normal = value => /^#[0-9a-f]{6}$/i.test(value) ? value.toUpperCase() : value;
export function annotationColours(p) {
  return fields.map((field, index) => p.custom_colors?.[index] || presets[index][p[field]]);
}
export function annotationProperties(p) {
  return Object.fromEntries(["--gloss-color", "--gloss-background", "--word-color", "--word-background"].map((key, i) => [key, annotationColours(p)[i]]));
}
function hsv(hex) {
  const rgb = hex.slice(1).match(/../g).map(v => parseInt(v, 16) / 255);
  const max = Math.max(...rgb), min = Math.min(...rgb), d = max - min;
  let h = !d ? 0 : max === rgb[0] ? ((rgb[1] - rgb[2]) / d) % 6 : max === rgb[1] ? (rgb[2] - rgb[0]) / d + 2 : (rgb[0] - rgb[1]) / d + 4;
  return [(h * 60 + 360) % 360, max ? d / max : 0, max];
}
function hex(h, s, v) {
  const f = n => { const k = (n + h / 60) % 6; return Math.round(255 * (v - v * s * Math.max(0, Math.min(k, 4 - k, 1)))).toString(16).padStart(2, '0'); };
  return `#${f(5)}${f(3)}${f(1)}`.toUpperCase();
}
export function createCustomColours(root, { t, read, changed }) {
  const $ = selector => root.querySelector(selector);
  const $$ = selector => [...root.querySelectorAll(selector)];
  const panel = $(".inline-colour-picker"), plane = $(".colour-plane"), hue = $(".colour-hue"), input = $(".colour-hex"), error = $(".colour-error");
  let custom = [null, null, null, null], favorites = [], active = null, restoring = false;
  let h = 160, s = .4, v = .7, listKey = null;
  const schemeKey = colors => JSON.stringify(colors.map(normal));
  function clearError() { error.textContent = ""; input.removeAttribute("aria-invalid"); }
  function paintPicker() {
    plane.style.setProperty("--hue", h);
    plane.style.setProperty("--saturation", `${s * 100}%`);
    plane.style.setProperty("--brightness", `${(1 - v) * 100}%`);
    plane.setAttribute("aria-valuenow", String(Math.round(s * 100)));
    plane.setAttribute("aria-valuetext", `${t("饱和度", "Saturation")} ${Math.round(s * 100)}%, ${t("明度", "Brightness")} ${Math.round(v * 100)}%`);
    hue.value = String(h);
  }
  function syncPicker(p) {
    if (active === null) return;
    const value = annotationColours(p)[active];
    input.value = value.startsWith('#') ? normal(value) : "";
    [h, s, v] = hsv(value.startsWith('#') ? value : active === 2 ? '#55534D' : '#FFFFFF');
    paintPicker(); clearError();
  }
  function choose(value) {
    custom[active] = normal(value); clearError(); changed();
  }
  function fromPicker() { const value = hex(h, s, v); input.value = value; choose(value); paintPicker(); }
  function closePicker() {
    active = null; panel.hidden = true; clearError();
    $$("[data-custom-colour]").forEach(button => button.setAttribute("aria-expanded", "false"));
  }
  function refresh(p) {
    $$("[data-custom-colour]").forEach((button, index) => {
      const dot = button.querySelector(".colour-dot");
      dot.dataset.empty = String(!custom[index]);
      dot.style.setProperty("--colour", custom[index] || "transparent");
      button.setAttribute("aria-pressed", String(Boolean(custom[index])));
      button.setAttribute("aria-label", `${t(titles[index])}${custom[index] ? ` ${custom[index]}` : ` · ${t("自定义", "Custom")}`}`);
      button.title = custom[index] || t("未设置自定义色", "No custom colour");
      const row = $(`[data-field="${fields[index]}"]`);
      row.dataset.customActive = String(Boolean(custom[index]));
      row.querySelectorAll("[data-slot]").forEach(node => node.setAttribute("aria-pressed", String(!custom[index] && node.dataset.on === "true")));
    });
    const current = schemeKey(annotationColours(p));
    const found = favorites.findIndex(scheme => schemeKey(scheme.colors) === current);
    const toggle = $("[data-favorite-toggle]");
    toggle.textContent = found >= 0 ? t("♥ 已收藏，点击取消", "♥ Favorited · click to remove") : t("♡ 收藏这套配色", "♡ Favorite this combination");
    toggle.setAttribute("aria-pressed", String(found >= 0));
    toggle.disabled = found < 0 && favorites.length >= 32;
    toggle.title = favorites.length >= 32 ? t("最多收藏 32 套配色", "Up to 32 combinations") : "";
    $("[data-favorite-count]").textContent = `${favorites.length} / 32`;
    $(".favorite-empty").hidden = favorites.length > 0;
    const nextListKey = JSON.stringify([favorites, t("应用配色", "Apply combination")]);
    if (listKey !== nextListKey) {
      listKey = nextListKey;
      const list = $(".favorite-schemes"); list.replaceChildren();
      favorites.forEach((scheme, index) => {
        const card = document.createElement("div"); card.className = "favorite-scheme";
        const apply = document.createElement("button"); apply.type = "button"; apply.className = "favorite-apply";
        apply.dataset.favoriteIndex = index;
        apply.setAttribute("aria-label", `${t("应用配色", "Apply combination")} ${index + 1}`);
        const sample = document.createElement("span"); sample.className = "favorite-sample";
        const annotation = document.createElement("span"); annotation.textContent = t("注音", "Gloss");
        annotation.style.color = scheme.colors[0]; annotation.style.background = scheme.colors[1];
        const word = document.createElement("b"); word.textContent = "Aa"; word.style.color = scheme.colors[2]; word.style.background = scheme.colors[3];
        sample.append(annotation, word);
        const dots = document.createElement("span"); dots.className = "favorite-dots";
        scheme.colors.forEach(colour => { const dot = document.createElement("i"); dot.className = "colour-dot"; dot.dataset.empty = String(["transparent", "inherit"].includes(colour)); dot.style.setProperty("--colour", colour); dots.append(dot); });
        apply.append(sample, dots);
        const remove = document.createElement("button"); remove.type = "button"; remove.className = "favorite-remove"; remove.dataset.removeFavorite = index; remove.textContent = "×"; remove.setAttribute("aria-label", `${t("删除配色", "Remove combination")} ${index + 1}`);
        card.append(apply, remove); list.append(card);
      });
    }
    $$(".favorite-apply").forEach((button, index) => button.setAttribute("aria-pressed", String(schemeKey(favorites[index].colors) === current)));
  }
  $$("[data-custom-colour]").forEach((button, index) => button.addEventListener("click", () => {
    if (active === index) { closePicker(); return; }
    active = index; panel.hidden = false;
    $$("[data-custom-colour]").forEach((node, i) => node.setAttribute("aria-expanded", String(i === active)));
    $("[data-picker-title]").textContent = t(titles[index]);
    syncPicker(read());
  }));
  $("[data-close-picker]").addEventListener("click", () => { const button = $(`[data-custom-colour="${active}"]`); closePicker(); button?.focus({ preventScroll: true }); });
  root.addEventListener("click", event => {
    const preset = event.target.closest("[data-field] [data-slot]");
    if (!preset || restoring) return;
    const index = fields.indexOf(preset.closest("[data-field]").dataset.field);
    custom[index] = null;
    queueMicrotask(() => { changed(); syncPicker(read()); });
  });
  $("[data-clear-colour]").addEventListener("click", () => { custom[active] = null; changed(); syncPicker(read()); });
  $$("[data-recommend-colour]").forEach(button => button.addEventListener("click", () => { choose(button.dataset.recommendColour); syncPicker(read()); }));
  input.addEventListener("input", () => {
    const value = input.value.trim();
    const expanded = /^#[0-9a-f]{3}$/i.test(value) ? `#${[...value.slice(1)].map(c => c + c).join('')}` : value;
    if (!/^#[0-9a-f]{6}$/i.test(expanded)) {
      input.setAttribute("aria-invalid", "true"); error.textContent = t("请输入 #RRGGBB 格式的颜色", "Enter a colour as #RRGGBB"); return;
    }
    choose(expanded); [h, s, v] = hsv(expanded); paintPicker();
  });
  input.addEventListener("blur", () => { if (!input.hasAttribute("aria-invalid") && custom[active]) input.value = custom[active]; });
  hue.addEventListener("input", () => { h = Number(hue.value); fromPicker(); });
  let pointer = null;
  function move(event) {
    const r = plane.getBoundingClientRect(); s = Math.max(0, Math.min(1, (event.clientX - r.left) / r.width)); v = 1 - Math.max(0, Math.min(1, (event.clientY - r.top) / r.height)); fromPicker();
  }
  plane.addEventListener("pointerdown", event => { if (!event.isPrimary || event.button !== 0) return; pointer = event.pointerId; plane.setPointerCapture(pointer); plane.focus({ preventScroll: true }); move(event); });
  plane.addEventListener("pointermove", event => { if (pointer === event.pointerId) move(event); });
  plane.addEventListener("pointerup", () => { pointer = null; });
  plane.addEventListener("pointercancel", () => { pointer = null; });
  plane.addEventListener("lostpointercapture", () => { pointer = null; });
  plane.addEventListener("keydown", event => {
    if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) return;
    event.preventDefault();
    const step = event.shiftKey ? .1 : .01;
    s = Math.max(0, Math.min(1, s + (event.key === "ArrowRight" ? step : event.key === "ArrowLeft" ? -step : 0)));
    v = Math.max(0, Math.min(1, v + (event.key === "ArrowUp" ? step : event.key === "ArrowDown" ? -step : 0))); fromPicker();
  });
  $("[data-favorite-toggle]").addEventListener("click", () => {
    if (input.hasAttribute("aria-invalid")) { input.focus(); return; }
    const colors = annotationColours(read()).map(normal), key = schemeKey(colors);
    const index = favorites.findIndex(scheme => schemeKey(scheme.colors) === key);
    if (index >= 0) favorites.splice(index, 1);
    else if (favorites.length < 32) favorites.push({ colors });
    else { error.textContent = t("最多收藏 32 套配色", "Up to 32 combinations"); return; }
    changed();
  });
  $(".favorite-schemes").addEventListener("click", event => {
    const remove = event.target.closest("[data-remove-favorite]"), apply = event.target.closest("[data-favorite-index]");
    if (remove) { const index = Number(remove.dataset.removeFavorite); favorites.splice(index, 1); changed(); const next = $$(".favorite-remove")[index] || $$(".favorite-remove").at(-1) || $("[data-favorite-toggle]"); next.focus({ preventScroll: true }); }
    if (apply) {
      const scheme = favorites[Number(apply.dataset.favoriteIndex)];
      restoring = true;
      scheme.colors.forEach((value, index) => {
        const slot = presets[index].findIndex(preset => normal(preset) === normal(value));
        $(`[data-field="${fields[index]}"] [data-slot="${Math.max(slot, 0)}"]`).click();
        custom[index] = value.startsWith('#') ? normal(value) : null;
      });
      restoring = false; changed(); syncPicker(read());
    }
  });
  return {
    getDraft: () => ({ custom_colors: [...custom], favorites: copy(favorites) }),
    setDraft(p, selectPresets) {
      restoring = true; custom = [...(p.custom_colors || [null, null, null, null])]; favorites = copy(p.favorites || []);
      selectPresets(); restoring = false; closePicker();
    },
    refresh,
    validate: () => !input.hasAttribute("aria-invalid"),
  };
}
