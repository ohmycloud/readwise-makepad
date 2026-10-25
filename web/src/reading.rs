//! Web-only annotation coordinates. Never put DOM offsets into native book_notes.
use crate::{
    library::State,
    routes::{self, ChapterNo},
    settings::Preferences,
};
use serde::{Deserialize, Serialize};
use sqlx::{
    Row,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{path::Path, time::Duration};
use topcoat::{
    Result,
    context::Cx,
    router::{content::Json, error::bad_request, path_param, route},
};

pub async fn open_storage(directory: &Path) -> Result<sqlx::SqlitePool> {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(directory.join("database/app.db"))
                .create_if_missing(false)
                .busy_timeout(Duration::from_secs(5)),
        )
        .await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS web_highlights (id TEXT PRIMARY KEY, book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE, chapter INTEGER NOT NULL, start_block INTEGER NOT NULL, start_offset INTEGER NOT NULL, end_block INTEGER NOT NULL, end_offset INTEGER NOT NULL, color INTEGER NOT NULL CHECK(color BETWEEN 0 AND 3))").execute(&pool).await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS web_highlights_book ON web_highlights(book_id, chapter)",
    )
    .execute(&pool)
    .await?;
    Ok(pool)
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPoint {
    pub block: usize,
    pub offset: usize,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Annotation {
    pub id: String,
    pub start: TextPoint,
    pub end: TextPoint,
    pub color: usize,
}
#[derive(Deserialize)]
struct HighlightInput {
    book_id: String,
    chapter: usize,
    start: TextPoint,
    end: TextPoint,
    color: usize,
}

async fn validate_chapter(
    state: &State,
    id: String,
    chapter: usize,
    range: Option<(TextPoint, TextPoint)>,
) -> Result<()> {
    uuid::Uuid::parse_str(&id).map_err(|_| bad_request("书籍 ID 无效"))?;
    state
        .call(move |library| {
            let book = library.book(&id)?.ok_or("书籍不存在")?;
            let content = library.content(&book)?;
            let chapter = content.chapters.get(chapter).ok_or("章节不存在")?;
            if let Some((start, end)) = range {
                if start >= end {
                    return Err("选区无效".into());
                }
                for point in [start, end] {
                    let text = chapter.text.get(point.block).ok_or("选区块不存在")?;
                    if point.offset > text.len() || !text.is_char_boundary(point.offset) {
                        return Err("选区偏移无效".into());
                    }
                }
            }
            Ok(())
        })
        .await
        .map_err(|e| bad_request(e.to_string()).into())
}

#[route(GET "/api/highlights/{book_id}/{chapter_no}")]
async fn highlights(cx: &Cx) -> Result<Json<Vec<Annotation>>> {
    let id = routes::book_id(cx)?;
    let chapter = *path_param::<ChapterNo>(cx)?;
    let state = routes::state(cx);
    validate_chapter(&state, id.clone(), chapter, None).await?;
    let rows = sqlx::query("SELECT id,start_block,start_offset,end_block,end_offset,color FROM web_highlights WHERE book_id=? AND chapter=? ORDER BY rowid")
        .bind(id).bind(chapter as i64).fetch_all(&state.storage).await?;
    Ok(Json(rows.into_iter().map(annotation_from_row).collect()))
}

#[route(POST "/api/highlights")]
async fn add_highlight(cx: &Cx, Json(input): Json<HighlightInput>) -> Result<Json<Annotation>> {
    if input.color > 3 {
        return Err(bad_request("高亮颜色无效").into());
    }
    let state = routes::state(cx);
    validate_chapter(
        &state,
        input.book_id.clone(),
        input.chapter,
        Some((input.start, input.end)),
    )
    .await?;
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO web_highlights (id,book_id,chapter,start_block,start_offset,end_block,end_offset,color) VALUES (?,?,?,?,?,?,?,?)")
        .bind(&id).bind(input.book_id).bind(input.chapter as i64).bind(input.start.block as i64).bind(input.start.offset as i64)
        .bind(input.end.block as i64).bind(input.end.offset as i64).bind(input.color as i64).execute(&state.storage).await?;
    Ok(Json(Annotation {
        id,
        start: input.start,
        end: input.end,
        color: input.color,
    }))
}

// A colour swatch applies a colour, unlike the pen's add/remove toggle. Replace
// overlaps in one transaction and retain their portions outside the selection.
#[route(POST "/api/highlights/apply")]
async fn apply_highlight(
    cx: &Cx,
    Json(input): Json<HighlightInput>,
) -> Result<Json<Vec<Annotation>>> {
    if input.color > 3 {
        return Err(bad_request("高亮颜色无效").into());
    }
    let state = routes::state(cx);
    validate_chapter(
        &state,
        input.book_id.clone(),
        input.chapter,
        Some((input.start, input.end)),
    )
    .await?;
    let mut tx = state.storage.begin().await?;
    let rows = sqlx::query("SELECT id,start_block,start_offset,end_block,end_offset,color FROM web_highlights WHERE book_id=? AND chapter=?")
        .bind(&input.book_id).bind(input.chapter as i64).fetch_all(&mut *tx).await?;
    let mut additions = vec![Annotation {
        id: uuid::Uuid::new_v4().to_string(),
        start: input.start,
        end: input.end,
        color: input.color,
    }];
    for old in rows.into_iter().map(annotation_from_row) {
        if old.start >= input.end || old.end <= input.start {
            continue;
        }
        sqlx::query("DELETE FROM web_highlights WHERE id=?")
            .bind(&old.id)
            .execute(&mut *tx)
            .await?;
        if old.start < input.start {
            additions.push(Annotation {
                id: uuid::Uuid::new_v4().to_string(),
                start: old.start,
                end: input.start,
                color: old.color,
            });
        }
        if old.end > input.end {
            additions.push(Annotation {
                id: uuid::Uuid::new_v4().to_string(),
                start: input.end,
                end: old.end,
                color: old.color,
            });
        }
    }
    for item in additions {
        sqlx::query("INSERT INTO web_highlights (id,book_id,chapter,start_block,start_offset,end_block,end_offset,color) VALUES (?,?,?,?,?,?,?,?)")
            .bind(item.id).bind(&input.book_id).bind(input.chapter as i64).bind(item.start.block as i64).bind(item.start.offset as i64)
            .bind(item.end.block as i64).bind(item.end.offset as i64).bind(item.color as i64).execute(&mut *tx).await?;
    }
    let rows = sqlx::query("SELECT id,start_block,start_offset,end_block,end_offset,color FROM web_highlights WHERE book_id=? AND chapter=? ORDER BY start_block,start_offset,end_block,end_offset")
        .bind(&input.book_id).bind(input.chapter as i64).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(rows.into_iter().map(annotation_from_row).collect()))
}

fn annotation_from_row(row: sqlx::sqlite::SqliteRow) -> Annotation {
    Annotation {
        id: row.get("id"),
        start: TextPoint {
            block: row.get::<i64, _>("start_block") as usize,
            offset: row.get::<i64, _>("start_offset") as usize,
        },
        end: TextPoint {
            block: row.get::<i64, _>("end_block") as usize,
            offset: row.get::<i64, _>("end_offset") as usize,
        },
        color: row.get::<i64, _>("color") as usize,
    }
}

#[derive(Deserialize)]
struct RemoveInput {
    book_id: String,
    chapter: usize,
    ids: Vec<String>,
}
#[route(POST "/api/highlights/remove")]
async fn remove_highlights(
    cx: &Cx,
    Json(input): Json<RemoveInput>,
) -> Result<Json<serde_json::Value>> {
    if input.ids.len() > 1000 {
        return Err(bad_request("一次最多删除 1000 条高亮").into());
    }
    let state = routes::state(cx);
    validate_chapter(&state, input.book_id.clone(), input.chapter, None).await?;
    let mut tx = state.storage.begin().await?;
    let mut removed = 0;
    for id in input.ids {
        removed += sqlx::query("DELETE FROM web_highlights WHERE id=? AND book_id=? AND chapter=?")
            .bind(id)
            .bind(&input.book_id)
            .bind(input.chapter as i64)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    }
    tx.commit().await?;
    Ok(Json(serde_json::json!({"removed": removed})))
}

#[derive(Deserialize)]
struct MarkerInput {
    slot: usize,
}
#[route(POST "/api/marker")]
async fn marker(cx: &Cx, Json(input): Json<MarkerInput>) -> Result<Json<Preferences>> {
    if !(4..=7).contains(&input.slot) {
        return Err(bad_request("标记颜色无效").into());
    }
    Ok(Json(
        routes::state(cx)
            .call(move |library| {
                let mut settings = library.db.load_reader_settings()?;
                settings.highlight_color_slot = input.slot;
                library.db.save_reader_settings(&settings)?;
                Preferences::load(&library.db)
            })
            .await?,
    ))
}

#[route(GET "/api/preferences/desktop")]
async fn desktop_preferences() -> Result<Json<Preferences>> {
    let directory = std::env::var_os("READWISE_WEB_DESKTOP_DATA_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(readwise_core::paths::app_data_dir);
    Ok(Json(desktop_snapshot(&directory).await?))
}

async fn desktop_snapshot(directory: &Path) -> Result<Preferences> {
    let path = directory.join("database/app.db");
    if !path.is_file() {
        return Err(bad_request("未找到桌面设置数据库；此操作只在 Web 服务所在电脑读取").into());
    }
    // Never Db::open_in(): opening the native facade would migrate/write it.
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .read_only(true)
                .create_if_missing(false)
                .busy_timeout(Duration::from_secs(2)),
        )
        .await
        .map_err(|_| bad_request("无法只读访问桌面设置"))?;
    let settings = readwise_core::Db::read_reader_settings(&pool).await;
    pool.close().await;
    Ok(Preferences::from(&settings.map_err(bad_request)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn desktop_snapshot_is_read_only_and_never_initializes_native_schema() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("database")).unwrap();
        let path = root.path().join("database/app.db");
        let pool = SqlitePoolOptions::new()
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE app_settings(key TEXT PRIMARY KEY,value TEXT,updated_at INTEGER)",
        )
        .execute(&pool)
        .await
        .unwrap();
        for (key, value) in [
            ("reader.language", "en"),
            ("reader.theme", "light"),
            ("reader.background", "sage"),
            ("reader.font", "seravek"),
            ("reader.spacing", "1.85"),
            ("reader.font_size", "13"),
            ("reader.gloss_font_size", "9.5"),
            ("reader.columns", "two"),
            ("reader.turn_mode", "slide"),
        ] {
            sqlx::query("INSERT INTO app_settings VALUES (?,?,0)")
                .bind(key)
                .bind(value)
                .execute(&pool)
                .await
                .unwrap();
        }
        pool.close().await;
        let before = std::fs::read(&path).unwrap();
        let p = desktop_snapshot(root.path()).await.unwrap();
        assert_eq!(p.language, "en");
        assert_eq!(p.theme, "light");
        assert_eq!(p.background, "sage");
        assert_eq!(p.font, "seravek");
        assert_eq!(p.spacing, 1.85);
        assert_eq!(p.font_size, 26.0);
        assert!((p.gloss_size - 13.3).abs() < 0.001);
        assert_eq!(p.columns, "two");
        assert_eq!(p.turn_mode, "slide");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(
            desktop_snapshot(&root.path().join("missing"))
                .await
                .is_err()
        );
    }
}

#[derive(Deserialize)]
struct SearchInput {
    book_id: String,
    query: String,
}
#[route(POST "/api/book-search")]
async fn book_search(cx: &Cx, Json(input): Json<SearchInput>) -> Result<Json<serde_json::Value>> {
    if input.query.trim().is_empty() || input.query.len() > 400 {
        return Err(bad_request("搜索文本须在 1 至 400 字节之间").into());
    }
    let results = routes::state(cx).call(move |library| {
        let book = library.book(&input.book_id)?.ok_or("书籍不存在")?;
        let content = library.content(&book)?;
        let query = input.query.trim().to_lowercase(); let mut results = Vec::new();
        'chapters: for (chapter, section) in content.chapters.iter().enumerate() {
            for (block, text) in section.text.iter().enumerate() {
                if text.to_lowercase().contains(&query) {
                    results.push(serde_json::json!({"title": section.title, "text": text.chars().take(240).collect::<String>(), "href": format!("/read/{}/{chapter}#web-block-{block}", book.id)}));
                    if results.len() >= 100 { break 'chapters }
                }
            }
        }
        Ok(results)
    }).await.map_err(|e| bad_request(e.to_string()))?;
    Ok(Json(serde_json::json!({"results": results})))
}
