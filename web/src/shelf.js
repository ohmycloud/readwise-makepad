// Browser-only shelf projection; all durable mutations use the native Db facade.
export function initializeShelf({ api, toast }) {
  const shelf = document.querySelector(".shelf-page");
  if (!shelf) return;
  const $ = (s, root = document) => root.querySelector(s);
  const $$ = (s, root = document) => [...root.querySelectorAll(s)];
  const t = (zh, en) => document.documentElement.lang === "en" ? en : zh;
  const menu = $("#book-menu"), editor = $("#book-editor"), deletion = $("#book-delete");
  let selected, origin, editing, busy = false;
  const status = value => value.replace(/^(在读|读完|未读)/, word => t(word, ({ 在读: "Reading", 读完: "Finished", 未读: "Unread" })[word]));
  $$(".book-status", shelf).forEach(node => { node.textContent = status(node.textContent); });
  $$(".book-more", shelf).forEach(node => node.setAttribute("aria-label", t("更多操作：", "More actions: ") + node.closest(".book-card").dataset.title));
  $$(".book-tags", shelf).forEach(node => { if (document.documentElement.lang === "en") node.textContent = node.textContent.replace(/^欲读/, "Want to Read"); });
  // Back/forward cache can restore the old shelf without requesting its new
  // progress. Only the shelf is refreshed; never reload a visible reader.
  window.addEventListener("pageshow", event => { if (event.persisted) location.reload(); });

  function setView(mode) {
    if (!["list", "grid"].includes(mode)) mode = "grid";
    shelf.dataset.view = mode;
    $$("[data-shelf-view]").forEach(button => button.setAttribute("aria-pressed", String(button.dataset.shelfView === mode)));
    try { localStorage.setItem("readwise:shelf-view", mode); } catch { /* Private browsing. */ }
  }
  let mode; try { mode = localStorage.getItem("readwise:shelf-view"); } catch { /* Optional. */ }
  setView(mode);
  $$("[data-shelf-view]").forEach(button => button.addEventListener("click", () => setView(button.dataset.shelfView)));

  // Judge the entire gesture, not merely where pointerup happens. Scrolling
  // out and back, long presses, multiple fingers and inertia stops are not taps.
  let gesture, approved, lastScroll = -Infinity;
  const pointers = new Set();
  window.addEventListener("scroll", () => {
    lastScroll = performance.now();
    if (gesture) gesture.cancelled = true;
  }, { passive: true });
  document.addEventListener("pointerdown", event => {
    pointers.add(event.pointerId);
    approved = null;
    if (pointers.size > 1 && gesture) gesture.cancelled = true;
    const target = event.target.closest(".book-link, .book-more");
    if (!target || !shelf.contains(target) || gesture) return;
    gesture = { id: event.pointerId, target, x: event.clientX, y: event.clientY, time: performance.now(),
      cancelled: pointers.size > 1 || performance.now() - lastScroll < 180 };
  }, { passive: true });
  document.addEventListener("pointermove", event => {
    if (gesture?.id === event.pointerId && Math.hypot(event.clientX - gesture.x, event.clientY - gesture.y) > 9) gesture.cancelled = true;
  }, { passive: true });
  document.addEventListener("pointerup", event => {
    pointers.delete(event.pointerId);
    if (gesture?.id !== event.pointerId) return;
    approved = !gesture.cancelled && performance.now() - gesture.time < 500 && gesture.target.contains(event.target)
      ? { target: gesture.target, time: performance.now() } : null;
    gesture = null;
  }, { passive: true });
  document.addEventListener("pointercancel", event => { pointers.delete(event.pointerId); gesture = null; approved = null; });
  shelf.addEventListener("click", event => {
    const target = event.target.closest(".book-link, .book-more");
    if (!target) return;
    if (event.detail !== 0 && (approved?.target !== target || performance.now() - approved.time > 400)) {
      event.preventDefault(); event.stopImmediatePropagation(); return;
    }
    if (!target.matches(".book-more")) return;
    selected = target.closest(".book-card");
    origin = target;
    $("#book-menu-title").textContent = selected.dataset.title;
    $("[data-book-action='want_to_read']").textContent = selected.dataset.wantToRead === "true"
      ? t("移出欲读清单", "Remove from Want to Read") : t("加入欲读清单", "Add to Want to Read");
    $(".shelf-error", menu).textContent = "";
    menu.showModal();
  }, true);

  function pending(value, dialog) {
    busy = value;
    $$("button, input", dialog).forEach(node => { node.disabled = value; });
    dialog.setAttribute("aria-busy", String(value));
  }
  for (const dialog of [menu, editor, deletion]) {
    dialog.addEventListener("cancel", event => { if (busy) event.preventDefault(); });
    dialog.addEventListener("click", event => {
      if (event.target !== dialog || busy) return;
      const r = dialog.getBoundingClientRect();
      if (event.clientX < r.left || event.clientX > r.right || event.clientY < r.top || event.clientY > r.bottom) dialog.close();
    });
    dialog.addEventListener("close", () => queueMicrotask(() => {
      if (!document.querySelector("dialog[open]")) origin?.focus({ preventScroll: true });
    }));
  }
  function project(book) {
    selected.dataset.title = book.title;
    selected.dataset.wantToRead = String(book.want_to_read);
    selected.dataset.tags = JSON.stringify(book.tags);
    $("h2", selected).textContent = book.title;
    const fallback = $(".cover-title", selected);
    if (fallback) fallback.textContent = book.title;
    $(".book-more", selected).setAttribute("aria-label", t("更多操作：", "More actions: ") + book.title);
    $(".book-status", selected).textContent = status(book.status);
    $(".book-tags", selected).textContent = [book.want_to_read ? t("欲读", "Want to Read") : "", ...book.tags].filter(Boolean).join(" · ");
  }
  async function mutate(action, dialog) {
    if (busy) return;
    const card = selected;
    $(".shelf-error", dialog).textContent = "";
    pending(true, dialog);
    try {
      const result = await api(`/api/books/${card.dataset.bookId}`, action);
      if (result.deleted) {
        card.remove();
        const count = $$(".book-card", shelf).length;
        $("#shelf-count").textContent = t(`${count} 本书，慢慢读。`, `${count} books. Take your time.`);
        // Re-render the server-owned empty state only after confirmed deletion.
        if (!count) location.reload();
        origin = $("[data-shelf-view='grid']");
      } else project(result);
      dialog.close();
      toast(t(result.deleted ? "书籍已删除" : "已保存", result.deleted ? "Book deleted" : "Saved"));
    } catch (error) {
      $(".shelf-error", dialog).textContent = error.message;
    } finally { pending(false, dialog); }
  }
  menu.addEventListener("click", async event => {
    const action = event.target.closest("[data-book-action]")?.dataset.bookAction;
    if (!action || busy) return;
    if (action === "share") {
      try {
        const { title, author } = selected.dataset;
        await navigator.clipboard.writeText(author.trim() ? `${title} — ${author}` : title);
        menu.close(); toast(t("书籍信息已复制，可粘贴到其他应用分享", "Book details copied; paste them into another app to share"));
      } catch { $(".shelf-error", menu).textContent = t("复制失败，请检查浏览器剪贴板权限", "Copy failed; check clipboard permissions"); }
    } else if (action === "want_to_read") {
      await mutate({ action, enabled: selected.dataset.wantToRead !== "true" }, menu);
    } else if (action === "complete") {
      await mutate({ action }, menu);
    } else if (action === "delete") {
      menu.close();
      $("#book-delete-name").textContent = `《${selected.dataset.title}》`;
      $(".shelf-error", deletion).textContent = "";
      deletion.showModal();
      $("[data-close]", deletion).focus();
    } else {
      editing = action;
      const tags = action === "tags";
      $("#book-editor-title").textContent = tags ? t("添加到藏书", "Add to Collection") : t("重新命名", "Rename");
      $("#book-editor-book").textContent = selected.dataset.title;
      $("#book-editor-label").textContent = tags ? t("标签", "Tags") : t("书名", "Book title");
      $("#book-editor-hint").textContent = tags ? t("用逗号或分号分隔；最多 8 个，每个 24 字符。清空可移除全部标签。", "Separate with commas or semicolons; up to 8 tags, 24 characters each. Clear to remove all.") : t("只修改应用内书名，不改写原始文件。最多 300 个字符。", "Only changes the title in the app, not the original file. Up to 300 characters.");
      const input = $("#book-editor-input");
      input.required = !tags;
      input.value = tags ? JSON.parse(selected.dataset.tags).join(", ") : selected.dataset.title;
      $(".shelf-error", editor).textContent = "";
      menu.close(); editor.showModal(); input.focus(); input.select();
    }
  });
  $("#book-editor-form").addEventListener("submit", event => {
    event.preventDefault();
    const value = $("#book-editor-input").value;
    mutate(editing === "tags" ? { action: "tags", tags: value.split(/[,，;；\n]/) } : { action: "rename", title: value }, editor);
  });
  $("#book-delete-form").addEventListener("submit", event => { event.preventDefault(); mutate({ action: "delete" }, deletion); });
}
