// CSS multicolumn pagination: paint/translate the SAME article, never rebuild it.
export function createPager({ article, textNodes, textPoint, byteOffset, pointRect, save }) {
  const viewport = document.querySelector("#reader-viewport");
  const main = document.querySelector("[data-reader-main]");
  const navigation = document.querySelector("#page-navigation");
  const previous = document.querySelector("#previous-page");
  const next = document.querySelector("#next-page");
  const indicator = document.querySelector("#page-indicator");
  let preferences = JSON.parse(article.dataset.preferences);
  let index = 0, count = 1, stride = 1, anchor = null, resizing, animationTimer;
  let touch = null;
  const active = () => preferences.turn_mode !== "scroll";
  const pointFor = (node, offset) => {
    const block = (node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement).closest("[data-block]");
    if (!block) return null;
    const nodes = textNodes(block);
    if (!nodes.includes(node)) { node = nodes[0]; offset = 0; }
    return node ? { chapter: Number(article.dataset.chapter), block: Number(block.dataset.block), offset: byteOffset(nodes, node, offset), screenY: viewport.getBoundingClientRect().top } : null;
  };
  function capture() {
    if (!active()) return null;
    const box = viewport.getBoundingClientRect();
    for (const dy of [12, 30, 60]) {
      const x = box.left + 10, y = box.top + dy;
      let node, offset;
      if (document.caretPositionFromPoint) { const point = document.caretPositionFromPoint(x, y); node = point?.offsetNode; offset = point?.offset; }
      else { const range = document.caretRangeFromPoint?.(x, y); node = range?.startContainer; offset = range?.startOffset; }
      if (node && article.contains(node)) { const result = pointFor(node, offset); if (result) return result; }
    }
    for (const block of article.querySelectorAll("[data-block]")) {
      for (const node of textNodes(block)) {
        const range = document.createRange(); range.selectNodeContents(node);
        if ([...range.getClientRects()].some(r => r.right > box.left && r.left < box.right && r.bottom > box.top && r.top < box.bottom)) return pointFor(node, 0);
      }
    }
    return anchor;
  }
  function paint(animate = false) {
    article.classList.toggle("turn-slide", animate && preferences.turn_mode === "slide");
    article.classList.toggle("turn-curl", animate && preferences.turn_mode === "curl");
    article.style.transform = active() ? `translate3d(${-index * stride}px,0,0)` : "";
    indicator.textContent = `${index + 1} / ${count}`;
    previous.disabled = index === 0; next.disabled = index >= count - 1;
    article.dataset.page = String(index); article.dataset.pages = String(count);
    clearTimeout(animationTimer);
    if (animate) animationTimer = setTimeout(() => article.classList.remove("turn-slide", "turn-curl"), 220);
  }
  function measure() {
    const width = viewport.clientWidth;
    article.style.setProperty("--page-height", `${Math.max(40, viewport.clientHeight - 12)}px`);
    stride = width + 36;
    count = active() ? Math.max(1, Math.ceil((article.scrollWidth + 36 - 2) / stride)) : 1;
    index = Math.min(index, count - 1);
  }
  function restore(location) {
    if (!active() || !location) return false;
    const block = article.querySelector(`[data-block="${location.block}"]`);
    const point = block && textPoint(block, location.offset);
    const rect = pointRect(point);
    if (!rect) return true;
    const x = rect.left - viewport.getBoundingClientRect().left + index * stride;
    index = Math.max(0, Math.min(count - 1, Math.floor((x + 2) / stride)));
    paint(false); anchor = location;
    return true;
  }
  function configure(prefs, location = anchor) {
    preferences = prefs;
    article.dataset.turnMode = prefs.turn_mode; article.dataset.columns = prefs.columns;
    main.dataset.turnMode = prefs.turn_mode;
    main.dataset.textWidth = prefs.text_width;
    main.style.setProperty("--margin-scale", String(1 + prefs.margin / 100));
    navigation.hidden = !active();
    if (active()) window.scrollTo(0, 0);
    if (!active()) { index = 0; article.style.transform = ""; }
    measure(); paint();
    if (active() && location) restore(location);
  }
  async function turn(delta) {
    if (!active() || document.querySelector("dialog[open]") || getSelection()?.isCollapsed === false) return;
    const target = Math.max(0, Math.min(count - 1, index + delta));
    if (target === index) return;
    index = target; paint(true);
    await new Promise(resolve => setTimeout(resolve, matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : 200));
    anchor = capture(); save();
    document.dispatchEvent(new Event("readwise-page"));
  }
  previous.addEventListener("click", () => turn(-1)); next.addEventListener("click", () => turn(1));
  document.addEventListener("keydown", event => {
    if (!active() || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || event.target.closest("input,textarea,select,button,[contenteditable=true],#selection-toolbar,dialog,.typst-math") || getSelection()?.isCollapsed === false) return;
    if (event.key === "ArrowRight" || event.key === "ArrowLeft") { event.preventDefault(); turn(event.key === "ArrowRight" ? 1 : -1); }
  });
  viewport.addEventListener("pointerdown", event => {
    if (event.target.closest(".typst-math")) { touch = null; return; }
    if (event.pointerType === "touch" && event.isPrimary) touch = { id: event.pointerId, x: event.clientX, y: event.clientY, time: performance.now() };
    else if (event.pointerType === "touch") touch = null;
  }, { passive: true });
  viewport.addEventListener("pointerup", event => {
    const gesture = touch; touch = null;
    if (!gesture || gesture.id !== event.pointerId || performance.now() - gesture.time > 500 || event.target.closest("pre,table,a,button,.typst-math")) return;
    const dx = event.clientX - gesture.x, dy = event.clientY - gesture.y;
    if (Math.abs(dx) > 60 && Math.abs(dx) > Math.abs(dy) * 2) turn(dx < 0 ? 1 : -1);
  }, { passive: true });
  viewport.addEventListener("pointercancel", () => { touch = null; });
  window.addEventListener("resize", () => {
    clearTimeout(resizing);
    resizing = setTimeout(() => { configure(preferences, anchor); document.dispatchEvent(new Event("readwise-layout")); }, 100);
  });
  configure(preferences);
  return { active, capture, restore, configure, refreshAnchor() { anchor = capture(); } };
}
