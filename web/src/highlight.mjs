// One continuous background BELOW the browser's text, never a patchwork around
// estimated native font boxes. Native Selection/Range, glyphs, source nodes and
// UTF-8 offsets remain untouched. Opaque inline/code backgrounds get the same
// paint as a CSS background image, also below their text.
const colours = ["#f2c43859", "#7cc66f59", "#6aa8f059", "#f084ac59"];
const ns = "http://www.w3.org/2000/svg";
const rect = r => ({ left: r.left, top: r.top, right: r.right, bottom: r.bottom });
export function intersect(a, b) {
  const r = { left: Math.max(a.left, b.left), top: Math.max(a.top, b.top), right: Math.min(a.right, b.right), bottom: Math.min(a.bottom, b.bottom) };
  return r.right > r.left && r.bottom > r.top ? r : null;
}

// Rectangle subtraction is only for testing complete coverage of an element's
// background, never for approximating a browser font or glyph paint box.
export function subtract(boxes, hole) {
  return boxes.flatMap(box => {
    const cut = intersect(box, hole);
    if (!cut) return [box];
    return [{ ...box, bottom: cut.top }, { ...box, top: cut.bottom },
      { left: box.left, right: cut.left, top: cut.top, bottom: cut.bottom },
      { left: cut.right, right: box.right, top: cut.top, bottom: cut.bottom }].filter(r => r.right > r.left && r.bottom > r.top);
  });
}
const pathData = (rectangles, origin) => rectangles.map(r => `M${r.left - origin.left},${r.top - origin.top}h${r.right - r.left}v${r.bottom - r.top}h${r.left - r.right}Z`).join("");
const hasBackground = colour => colour && colour !== "transparent" && colour !== "rgba(0, 0, 0, 0)";

// The inline border/padding box can extend beyond its font and line boxes.
// Extend horizontal padding only at a selected logical code boundary: selecting
// a suffix must not also colour an unselected prefix (and vice versa).
export function inlineFragment(text, box, start, end) {
  return { left: start ? Math.min(text.left, box.left) : text.left, right: end ? Math.max(text.right, box.right) : text.right,
    top: Math.min(text.top, box.top), bottom: Math.max(text.bottom, box.bottom) };
}

// Each call is ONE flow box and ONE column: never bridge paragraph margins,
// table cells, code scrollers or the central gutter of a two-column spread.
export function lineBands(fragments) {
  const rows = [];
  for (const f of [...fragments].sort((a, b) => a.top - b.top || a.left - b.left)) {
    const centre = f.centre ?? (f.top + f.bottom) / 2;
    let row = rows.find(r => Math.min(r.inkBottom, f.bottom) > Math.max(r.inkTop, f.top)
      && Math.abs(r.centre - centre) < Math.max(r.font, f.font) * .6);
    if (!row) { row = { centre, left: f.left, right: f.right, inkTop: f.top, inkBottom: f.bottom, height: f.height, font: f.font }; rows.push(row); }
    else { row.left = Math.min(row.left, f.left); row.right = Math.max(row.right, f.right); row.inkTop = Math.min(row.inkTop, f.top); row.inkBottom = Math.max(row.inkBottom, f.bottom); row.height = Math.max(row.height, f.height); row.font = Math.max(row.font, f.font); }
  }
  rows.sort((a, b) => a.inkTop - b.inkTop);
  const bands = rows.map(r => {
    const height = r.height;
    return { left: r.left, right: r.right, top: Math.min(r.inkTop, r.centre - height / 2), bottom: Math.max(r.inkBottom, r.centre + height / 2) };
  });
  for (let i = 1; i < rows.length; i++) {
    const previous = rows[i - 1], current = rows[i];
    if (current.inkTop - previous.inkTop > Math.max(previous.height, current.height) * 2.1) continue;
    if (Math.min(previous.right, current.right) <= Math.max(previous.left, current.left)) continue;
    const middle = (previous.inkBottom + current.inkTop) / 2;
    bands[i - 1].bottom = middle;
    bands[i].top = middle;
  }
  return bands;
}

export function createHighlightPainter({ article }) {
  const supported = Boolean(globalThis.CSS?.highlights && globalThis.Highlight);
  if (!supported) return { setSaved() {}, setSelection() {} };
  const viewport = document.querySelector("#reader-viewport"), toolbar = document.querySelector(".reader-toolbar");
  const main = article.closest(".reader-main");
  const svg = document.createElementNS(ns, "svg");
  svg.classList.add("reader-highlight-gaps");
  svg.setAttribute("shape-rendering", "crispEdges"); svg.setAttribute("aria-hidden", "true"); svg.setAttribute("focusable", "false");
  // A negative layer inside the main's stacking context is above the paper but
  // below all source content, including the paged viewport's paint containment.
  main.append(svg);
  let backgroundNodes = new Map();
  const colourCache = new Map();
  const rgba = colour => {
    if (!colourCache.has(colour)) {
      if (/^#[\da-f]{6}(?:[\da-f]{2})?$/i.test(colour)) {
        const values = colour.slice(1).match(/../g).map(value => parseInt(value, 16));
        if (values.length === 3) values.push(255);
        colourCache.set(colour, values); return values;
      }
      const canvas = document.createElement("canvas"); canvas.width = canvas.height = 1;
      const context = canvas.getContext("2d"); context.fillStyle = colour; context.fillRect(0, 0, 1, 1);
      colourCache.set(colour, [...context.getImageData(0, 0, 1, 1).data]);
    }
    return colourCache.get(colour);
  };
  function restoreBackground(node) {
    node.removeAttribute("data-highlight-background"); node.style.removeProperty("--rw-highlight-background"); node.style.removeProperty("--rw-highlight-base");
  }
  function clearBackgrounds() {
    for (const node of backgroundNodes.keys()) restoreBackground(node);
    backgroundNodes = new Map();
  }
  // Fixed stacking order must agree with the four CSS highlight layers, even
  // when a user selects before the initial saved-mark request has completed.
  for (let color = 0; color < 4; color++) CSS.highlights.set(`rw-${color}`, new Highlight());
  let saved = [], captured = null, frame = 0;
  const schedule = () => { if (!frame) frame = requestAnimationFrame(draw); };
  function liveRange() {
    const selection = getSelection();
    if (selection?.rangeCount === 1 && !selection.isCollapsed) {
      const range = selection.getRangeAt(0);
      if (article.contains(range.startContainer) && article.contains(range.endContainer)) return range;
    }
    // Retain the quoted selection while a toolbar input or dialog owns focus.
    return document.activeElement?.closest("#selection-toolbar,dialog") ? captured : null;
  }
  function geometry(range, base, columnWidth, layoutScale, styles, clips, backgrounds) {
    const style = node => { if (!styles.has(node)) styles.set(node, getComputedStyle(node)); return styles.get(node); };
    function clipFor(element) {
      if (element === article) return base;
      if (clips.has(element)) return clips.get(element);
      const parent = element.parentElement;
      const bounds = { ...clipFor(parent) }, css = style(element);
      if (/(auto|scroll|hidden|clip)/.test(css.overflowX + css.overflowY)) {
        const r = element.getBoundingClientRect();
        // Formula wrappers have no borders/padding. clientHeight/Width round
        // their em sizes to integers and can cut a real device-pixel strip off
        // a fraction/root at fractional DPR or after restored scrolling.
        if (element.matches('.typst-math')) {
          bounds.left=Math.max(bounds.left,r.left); bounds.right=Math.min(bounds.right,r.right);
          bounds.top=Math.max(bounds.top,r.top); bounds.bottom=Math.min(bounds.bottom,r.bottom);
          clips.set(element,bounds); return bounds;
        }
        if (/(auto|scroll|hidden|clip)/.test(css.overflowX)) { bounds.left = Math.max(bounds.left, r.left + element.clientLeft); bounds.right = Math.min(bounds.right, r.left + element.clientLeft + element.clientWidth); }
        if (/(auto|scroll|hidden|clip)/.test(css.overflowY)) { bounds.top = Math.max(bounds.top, r.top + element.clientTop); bounds.bottom = Math.min(bounds.bottom, r.top + element.clientTop + element.clientHeight); }
      }
      clips.set(element, bounds); return bounds;
    }
    const groups = new Map(), inlineBoxes = new WeakMap(), formulas = new Map();
    function inlineBorder(parent, raw) {
      const inline = parent.closest("code,.word");
      if (!inline || inline.closest("pre")) return raw;
      if (!inlineBoxes.has(inline)) {
        const walker = document.createTreeWalker(inline, NodeFilter.SHOW_TEXT);
        let first, last;
        while (walker.nextNode()) if (walker.currentNode.length) { first ||= walker.currentNode; last = walker.currentNode; }
        inlineBoxes.set(inline, { boxes: [...inline.getClientRects()], start: first && range.comparePoint(first, 0) === 0, end: last && range.comparePoint(last, last.length) === 0 });
      }
      const { boxes, start, end } = inlineBoxes.get(inline);
      // A wrapped inline has multiple fragments, sometimes on different columns
      // at the same Y. Match both axes and the closest vertical centre.
      const box = boxes.filter(box => intersect(box, raw)).sort((a, b) =>
        Math.abs(a.top + a.bottom - raw.top - raw.bottom) - Math.abs(b.top + b.bottom - raw.top - raw.bottom))[0];
      return box ? inlineFragment(raw, box, start, end) : raw;
    }
    function visit(node) {
      if (!range.intersectsNode(node)) return;
      const parent = node.parentElement, css = style(parent);
      if (css.visibility !== "visible" || css.display === "none") return;
      const part = document.createRange(); part.selectNodeContents(node);
      if (node === range.startContainer) part.setStart(node, range.startOffset);
      if (node === range.endContainer) part.setEnd(node, range.endOffset);
      if (part.collapsed) return;
      const math = parent.closest('.typst-math');
      const owner = parent.closest('[data-math-owner]')?.dataset.mathOwner;
      const formula = math || (owner && article.querySelector(`[data-math-id="${owner}"]`));
      if (formula) {
        // A positioned source font box is NOT a mathematical ink box. Fractions,
        // radicals and limits are one atomic visual unit, including displaced
        // legacy source proxies. Standalone/display math stays atomic; inline
        // math selected WITH prose must share that prose's uniform line band.
        if (!formulas.has(formula)) {
          const box = formula.querySelector('.math-canvas').getBoundingClientRect();
          const clipped = intersect(box, clipFor(formula));
          if (clipped) formulas.set(formula, clipped);
        }
        return;
      }
      const clip = clipFor(parent), font = parseFloat(css.fontSize) * layoutScale;
      const height = (parseFloat(css.lineHeight) || parseFloat(css.fontSize) * 1.2) * layoutScale;
      const flow = parent.closest("p,h1,h2,h3,h4,h5,h6,li,pre,td,th,figcaption,blockquote,[data-block]");
      for (const raw of part.getClientRects()) {
        // A line's leading/padding can remain visible after its glyph box has
        // scrolled just outside the viewport. Do not drop that background strip.
        if (!intersect({ left: raw.left - font, right: raw.right + font, top: raw.top - height, bottom: raw.bottom + height }, clip)) continue;
        for (let node = parent; node && node !== article; node = node.parentElement) if (hasBackground(backgroundNodes.get(node) || style(node).backgroundColor)) backgrounds.add(node);
        if (!flow || parent.closest("rt,rp")) continue;
        const decorated = inlineBorder(parent, raw), left = Math.max(decorated.left, clip.left), right = Math.min(decorated.right, clip.right);
        if (right <= left) continue;
        const column = Math.floor(((left + right) / 2 - viewport.getBoundingClientRect().left) / columnWidth);
        if (!groups.has(flow)) groups.set(flow, new Map());
        const columns = groups.get(flow);
        if (!columns.has(column)) columns.set(column, { fragments: [], clip });
        columns.get(column).fragments.push({ ...rect(decorated), left, right, font, height });
      }
    }
    const root = range.commonAncestorContainer;
    if (root.nodeType === Node.TEXT_NODE) visit(root);
    else {
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
        acceptNode(node) {
          if (!range.intersectsNode(node)) return NodeFilter.FILTER_REJECT;
          return node.nodeType === Node.TEXT_NODE ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_SKIP;
        },
      });
      while (walker.nextNode()) visit(walker.currentNode);
    }
    const bands = [];
    for (const [formula, box] of formulas) {
      const flow = formula.closest('p,h1,h2,h3,h4,h5,h6,li,td,th,figcaption,blockquote,[data-block]');
      const column = Math.floor(((box.left + box.right) / 2 - viewport.getBoundingClientRect().left) / columnWidth);
      const group = groups.get(flow)?.get(column);
      if (formula.matches('.math-display') || !group) { bands.push(box); continue; }
      const css = style(formula), font = parseFloat(css.fontSize) * layoutScale;
      const baseline = formula.getBoundingClientRect().bottom + (parseFloat(css.verticalAlign) || 0) * layoutScale;
      const height = (parseFloat(style(flow).lineHeight) || font * 1.2 / layoutScale) * layoutScale;
      group.fragments.push({ ...box, font, height, centre: baseline - font * .3 });
    }
    for (const columns of groups.values()) for (const group of columns.values()) {
      for (const band of lineBands(group.fragments)) {
        const clipped = intersect(band, group.clip);
        if (clipped) bands.push(clipped);
      }
    }
    return bands;
  }
  function paintBackgrounds(nodes, layers) {
    const next = new Map();
    for (const node of nodes) {
      const boxes = [...node.getClientRects()].filter(r => r.width && r.height);
      // Inline CSS backgrounds use one unbroken strip, sliced across line/column
      // fragments. Block backgrounds have one box. Match that native coordinate
      // space instead of adding wrappers or replacing source text.
      const width = boxes.reduce((sum, r) => sum + r.width, 0), height = Math.max(0, ...boxes.map(r => r.height));
      if (!width || !height) continue;
      const original = backgroundNodes.get(node) || getComputedStyle(node).backgroundColor, baseColour = rgba(original);
      const paths = [], uniformColours = []; let uniform = true;
      for (const [colour, rectangles] of layers) {
        let offset = 0; const pieces = [];
        for (const box of boxes) {
          for (const r of rectangles) {
            const clipped = intersect(r, box);
            if (clipped) pieces.push({ left: clipped.left - box.left + offset, right: clipped.right - box.left + offset, top: clipped.top - box.top, bottom: clipped.bottom - box.top });
          }
          offset += box.width;
        }
        if (pieces.length) {
          paths.push(`<path fill="${colour}" d="${pathData(pieces, { left: 0, top: 0 })}"/>`);
          const covered = boxes.every(box => {
            let remaining = [rect(box)];
            for (const r of rectangles) remaining = subtract(remaining, r);
            return remaining.every(r => (r.right - r.left) * (r.bottom - r.top) < .000001);
          });
          if (covered) uniformColours.push(colour); else uniform = false;
        }
      }
      if (!paths.length) continue;
      let value = "none", base = "transparent";
      if (uniform) {
        // A single native background colour avoids an extra texture-edge clip
        // around a completely selected code/word panel, even at fractional zoom.
        let rgb = baseColour.slice(0, 3);
        for (const colour of uniformColours) { const ink = rgba(colour), alpha = ink[3] / 255; rgb = rgb.map((c, i) => Math.round(c * (1 - alpha) + ink[i] * alpha)); }
        base = `rgba(${rgb.join(",")},${baseColour[3] / 255})`;
      } else {
        // Bake the original background and highlight into ONE image, not two
        // separately antialiased layers. Preserve the original background alpha.
        const image = `<svg xmlns="${ns}" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" preserveAspectRatio="none" shape-rendering="crispEdges"><g opacity="${baseColour[3] / 255}"><rect width="100%" height="100%" fill="rgb(${baseColour.slice(0, 3).join(",")})"/>${paths.join("")}</g></svg>`;
        value = `url("data:image/svg+xml,${encodeURIComponent(image)}")`;
      }
      if (node.style.getPropertyValue("--rw-highlight-background") !== value) node.style.setProperty("--rw-highlight-background", value);
      if (node.style.getPropertyValue("--rw-highlight-base") !== base) node.style.setProperty("--rw-highlight-base", base);
      node.setAttribute("data-highlight-background", ""); next.set(node, original);
    }
    for (const node of backgroundNodes.keys()) if (!next.has(node)) restoreBackground(node);
    backgroundNodes = next;
  }
  function draw() {
    frame = 0;
    const live = liveRange();
    if (live) CSS.highlights.set("rw-selection", new Highlight(live));
    else CSS.highlights.delete("rw-selection");
    article.toggleAttribute("data-selection-paint", Boolean(live));
    // Do not leave a stationary gap projection over a moving/clipped page.
    if (article.getAnimations().some(animation => animation.playState === "running")) { svg.replaceChildren(); clearBackgrounds(); schedule(); return; }
    const visual = globalThis.visualViewport;
    const screen = { left: visual?.offsetLeft || 0, top: Math.max(visual?.offsetTop || 0, toolbar.getBoundingClientRect().bottom), right: (visual?.offsetLeft || 0) + (visual?.width || innerWidth), bottom: (visual?.offsetTop || 0) + (visual?.height || innerHeight) };
    // Paint in the scrolling document's coordinate space, not in a fixed screen
    // overlay that needs JS to chase compositor scrolling. In scroll mode keep
    // the complete bands (including offscreen lines) so newly exposed text is
    // already painted even while the main thread is busy. Paged views still
    // clip to their fixed viewport; nested formula/code scrollers keep clipFor.
    const base = main.dataset.turnMode === "scroll" ? rect(viewport.getBoundingClientRect()) : intersect(screen, viewport.getBoundingClientRect());
    if (!base) { svg.replaceChildren(); clearBackgrounds(); return; }
    const css = getComputedStyle(article), columns = parseInt(css.columnCount) || 1;
    const layoutScale = viewport.getBoundingClientRect().width / parseFloat(getComputedStyle(viewport).width);
    const columnWidth = (viewport.getBoundingClientRect().width + (parseFloat(css.columnGap) || 0) * layoutScale) / columns;
    const styles = new WeakMap(), clips = new WeakMap(), layers = new Map(), backgrounds = new Set();
    const entries = [...saved, ...(live ? [{ range: live, colour: css.getPropertyValue("--selection-ink").trim() || "#7fb2a688" }] : [])];
    const origin = svg.getBoundingClientRect();
    svg.setAttribute("viewBox", `0 0 ${origin.width} ${origin.height}`);
    for (const { range, colour } of entries) {
      const rectangles = geometry(range, base, columnWidth, layoutScale, styles, clips, backgrounds);
      if (!layers.has(colour)) layers.set(colour, []);
      layers.get(colour).push(...rectangles);
    }
    svg.replaceChildren(...[...layers].map(([colour, rectangles]) => {
      const path = document.createElementNS(ns, "path"); path.setAttribute("fill", colour); path.setAttribute("d", pathData(rectangles, origin)); return path;
    }));
    paintBackgrounds(backgrounds, layers);
  }
  document.addEventListener("selectionchange", schedule);
  document.addEventListener("scroll", event => {
    // The document and its underlay move together without a repaint. Only an
    // internal scroller changes geometry relative to the underlay.
    if (event.target !== document) schedule();
  }, { passive: true, capture: true });
  window.addEventListener("resize", schedule);
  visualViewport?.addEventListener("resize", schedule); visualViewport?.addEventListener("scroll", schedule);
  for (const event of ["readwise-page", "readwise-layout"]) document.addEventListener(event, schedule);
  document.addEventListener("readwise-preferences", () => { clearBackgrounds(); schedule(); });
  document.fonts.addEventListener("loadingdone", schedule);
  new ResizeObserver(schedule).observe(viewport);
  new MutationObserver(records => {
    if (records.some(record => ["data-known", "data-gloss-enabled"].includes(record.attributeName))) clearBackgrounds();
    schedule();
  }).observe(article, { subtree: true, attributes: true, attributeFilter: ["style", "class", "data-known", "data-gloss-enabled", "data-page", "data-turn-mode", "data-columns"] });
  return {
    setSaved(records) {
      saved = [...records].sort((a, b) => a.color - b.color).map(({ range, color }) => ({ range, colour: colours[color] }));
      for (let color = 0; color < 4; color++) CSS.highlights.set(`rw-${color}`, new Highlight(...records.filter(r => r.color === color).map(r => r.range)));
      schedule();
    },
    setSelection(range) { captured = range; schedule(); },
  };
}
