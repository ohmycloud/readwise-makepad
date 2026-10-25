//! Shelf management reuses the native Db operations, not a second book store.
use crate::routes;
use readwise_core::books::models::BookShelfItem;
use serde::Deserialize;
use topcoat::{
    Result,
    asset::{Asset, asset},
    context::Cx,
    router::{content::Json, error::bad_request, route},
    view::{View, component, view},
};

pub const SCRIPT: Asset = asset!("./shelf.js");

pub fn status(book: &BookShelfItem) -> String {
    let name = match book.status.as_str() {
        "reading" => "在读",
        "completed" => "读完",
        _ => "未读",
    };
    if book.progress_total > 0 {
        let percent =
            (book.progress_current as i128 * 100 / book.progress_total as i128).clamp(0, 100);
        format!("{name} · {percent}%")
    } else {
        name.into()
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum BookAction {
    Rename { title: String },
    WantToRead { enabled: bool },
    Tags { tags: Vec<String> },
    Complete,
    Delete,
}

#[route(POST "/api/books/{book_id}")]
async fn manage_book(cx: &Cx, Json(action): Json<BookAction>) -> Result<Json<serde_json::Value>> {
    let id = routes::book_id(cx)?;
    let result = routes::state(cx)
        .call(move |library| {
            library.book(&id)?.ok_or("书籍不存在")?;
            match action {
                BookAction::Rename { title } => {
                    library.db.rename_book(&id, &title)?;
                }
                BookAction::WantToRead { enabled } => {
                    library.db.set_book_want_to_read(&id, enabled)?
                }
                BookAction::Tags { tags } => {
                    library.db.set_book_tags(&id, &tags)?;
                }
                BookAction::Complete => library.db.mark_book_completed(&id)?,
                BookAction::Delete => {
                    library.delete_book(&id)?;
                    return Ok(serde_json::json!({"deleted": true}));
                }
            }
            let book = library.book(&id)?.ok_or("书籍不存在")?;
            // Never expose private absolute file paths in API responses.
            Ok(serde_json::json!({
                "title": book.title, "status": status(&book),
                "want_to_read": book.want_to_read, "tags": book.tags,
            }))
        })
        .await
        .map_err(|error| bad_request(error.to_string()))?;
    Ok(Json(result))
}

#[component]
pub async fn dialogs() -> Result<impl View> {
    Ok(view! {
        <dialog
            id="book-menu"
            class="panel-dialog shelf-menu"
            aria-labelledby="book-menu-title"
        >
            <div class="dialog-heading">
                <h2 id="book-menu-title"></h2>
                <button type="button" class="button quiet" data-close="">
                    "关闭"
                </button>
            </div>
            <div class="shelf-menu-actions">
                <button type="button" class="button" data-book-action="share">
                    "分享"
                </button>
                <button type="button" class="button" data-book-action="want_to_read">
                    "加入欲读清单"
                </button>
                <hr>
                <button type="button" class="button" data-book-action="tags">
                    "添加到藏书"
                </button>
                <button type="button" class="button" data-book-action="complete">
                    "标记为已读完"
                </button>
                <button type="button" class="button" data-book-action="rename">
                    "重新命名"
                </button>
                <button type="button" class="button danger" data-book-action="delete">
                    "删除"
                </button>
            </div>
            <p class="shelf-error" role="status" aria-live="polite"></p>
        </dialog>
        <dialog
            id="book-editor"
            class="panel-dialog"
            aria-labelledby="book-editor-title"
        >
            <form id="book-editor-form">
                <div class="dialog-heading">
                    <h2 id="book-editor-title">"重新命名"</h2>
                </div>
                <p id="book-editor-book" class="muted"></p>
                <label for="book-editor-input" id="book-editor-label">"书名"</label>
                <input
                    id="book-editor-input"
                    class="shelf-input"
                    type="text"
                    required=(true)
                    maxlength="600"
                >
                <p id="book-editor-hint" class="muted small"></p>
                <p class="shelf-error" role="status" aria-live="polite"></p>
                <div class="shelf-dialog-actions">
                    <button type="button" class="button" data-close="">"取消"</button>
                    <button type="submit" class="button primary">"保存"</button>
                </div>
            </form>
        </dialog>
        <dialog
            id="book-delete"
            class="panel-dialog"
            aria-labelledby="book-delete-title"
        >
            <form id="book-delete-form">
                <div class="dialog-heading">
                    <h2 id="book-delete-title">"删除书籍？"</h2>
                </div>
                <p id="book-delete-name"></p>
                <p>
                    "阅读进度和书内高亮会一并删除，已保存的摘录笔记保留。不会删除应用外的原始 EPUB / PDF。此操作无法撤销。"
                </p>
                <p class="shelf-error" role="status" aria-live="polite"></p>
                <div class="shelf-dialog-actions">
                    <button type="button" class="button" data-close="">"取消"</button>
                    <button type="submit" class="button danger">"确认删除"</button>
                </div>
            </form>
        </dialog>
    })
}
