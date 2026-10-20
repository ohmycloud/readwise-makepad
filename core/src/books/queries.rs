use super::models::*;
use super::{parse_meta, supported_format};
use sqlx::{Row, SqlitePool};
use std::fs;
use std::path::{Path, PathBuf};

/// Roll back only the newly allocated private directory, never a user's source.
struct ImportDirectory {
    path: PathBuf,
    committed: bool,
}

impl Drop for ImportDirectory {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn cover_ext(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "jpg",
    }
}

/// 导入 EPUB / PDF：Rust 侧解析元数据 → 复制文件与封面到数据目录 → 入库。
/// 源文件不删（拖放/导入目录的是用户原文件）。返回新书。
pub async fn import_book(
    pool: &SqlitePool,
    data_dir: &Path,
    src: &Path,
) -> Result<SimpleBook, String> {
    let format =
        supported_format(src).ok_or_else(|| format!("仅支持 EPUB 和 PDF：{}", src.display()))?;
    let meta = parse_meta(src)?;
    let id = uuid::Uuid::new_v4().to_string();
    let file_size = fs::metadata(src).map(|m| m.len() as i64).unwrap_or(0);

    let book_dir = data_dir.join("books").join(&id);
    fs::create_dir_all(&book_dir).map_err(|e| format!("创建书目录失败: {}", e))?;
    let mut directory = ImportDirectory {
        path: book_dir,
        committed: false,
    };

    let epub_rel = format!("books/{}/book.{format}", id);
    fs::copy(src, data_dir.join(&epub_rel)).map_err(|e| format!("复制书籍失败: {}", e))?;
    if format == "pdf" {
        // Validate text extraction before reporting import success, and freeze
        // the source coordinates used by selection / highlights / ruby.
        super::pdf::load_imported_pages(&data_dir.join(&epub_rel))?;
    }

    let cover_rel = if let Some((bytes, mime)) = &meta.cover {
        let rel = format!("books/{}/cover.{}", id, cover_ext(mime));
        fs::write(data_dir.join(&rel), bytes).map_err(|e| format!("保存封面失败: {}", e))?;
        Some(rel)
    } else {
        None
    };

    let now = chrono::Utc::now().timestamp_millis();
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| format!("开启事务失败: {}", e))?;

    sqlx::query(
        r#"INSERT INTO books (id, title, author, format, file_path, cover_path, file_size, language, tags, created_at, updated_at)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(&id)
    .bind(&meta.title)
    .bind(&meta.author)
    .bind(format)
    .bind(&epub_rel)
    .bind(&cover_rel)
    .bind(file_size)
    .bind(&meta.language)
    .bind(None::<String>)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("插入书籍失败: {}", e))?;

    sqlx::query(
        r#"INSERT INTO book_status (book_id, status, progress_current, progress_total, location, created_at, updated_at)
           VALUES (?, 'unread', 0, 0, '', ?, ?)"#,
    )
    .bind(&id)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("创建书籍状态失败: {}", e))?;

    tx.commit()
        .await
        .map_err(|e| format!("提交事务失败: {}", e))?;
    directory.committed = true;

    Ok(SimpleBook {
        id,
        title: meta.title,
        author: meta.author,
        format: format.to_string(),
        file_path: epub_rel,
        cover_path: cover_rel,
        file_size,
        language: meta.language,
        tags: None,
        created_at: now,
        updated_at: now,
    })
}

/// 书架列表：books LEFT JOIN book_status，封面/文件拼成绝对路径供 UI 直接用。
pub async fn get_shelf_items(
    pool: &SqlitePool,
    data_dir: &Path,
) -> Result<Vec<BookShelfItem>, String> {
    let rows = sqlx::query(
        r#"SELECT b.id, b.title, b.author, b.file_path, b.cover_path, b.tags, b.created_at,
                  COALESCE(s.status, 'unread') AS status,
                  COALESCE(s.progress_current, 0) AS progress_current,
                  COALESCE(s.progress_total, 0) AS progress_total,
                  s.last_read_at,
                  s.metadata AS status_metadata
           FROM books b LEFT JOIN book_status s ON b.id = s.book_id
           ORDER BY b.created_at DESC"#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("获取书架失败: {}", e))?;

    let abs = |rel: Option<String>| -> Option<String> {
        rel.map(|r| data_dir.join(r).to_string_lossy().into_owned())
    };

    Ok(rows
        .iter()
        .map(|row| {
            let file_path: String = row.get("file_path");
            let tags = row
                .get::<Option<String>, _>("tags")
                .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
                .unwrap_or_default();
            let want_to_read = row
                .get::<Option<String>, _>("status_metadata")
                .and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok())
                .and_then(|value| value.get("want_to_read").and_then(|value| value.as_bool()))
                .unwrap_or(false);
            BookShelfItem {
                id: row.get("id"),
                title: row.get("title"),
                author: row.get("author"),
                cover_abs_path: abs(row.get("cover_path")),
                file_abs_path: data_dir.join(file_path).to_string_lossy().into_owned(),
                status: row.get("status"),
                progress_current: row.get("progress_current"),
                progress_total: row.get("progress_total"),
                last_read_at: row.get("last_read_at"),
                want_to_read,
                tags,
                created_at: row.get("created_at"),
            }
        })
        .collect())
}

/// 标签在保存前统一清理：去空白、大小写不敏感去重，并限制数量与长度。
pub fn normalize_book_tags(tags: &[String]) -> Result<Vec<String>, String> {
    const MAX_TAGS: usize = 8;
    const MAX_TAG_CHARS: usize = 24;

    let mut normalized = Vec::new();
    for raw in tags {
        let tag = raw.trim();
        if tag.is_empty() {
            continue;
        }
        if tag.chars().any(char::is_control) {
            return Err("标签不能包含控制字符".to_string());
        }
        if tag.chars().count() > MAX_TAG_CHARS {
            return Err(format!("标签不能超过 {MAX_TAG_CHARS} 个字符：{tag}"));
        }
        if normalized
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(tag))
        {
            continue;
        }
        normalized.push(tag.to_string());
        if normalized.len() > MAX_TAGS {
            return Err(format!("每本书最多设置 {MAX_TAGS} 个标签"));
        }
    }
    Ok(normalized)
}

/// 原子更新一本书的标签，同时补齐全局标签目录。
pub async fn set_book_tags(
    pool: &SqlitePool,
    book_id: &str,
    tags: &[String],
) -> Result<Vec<String>, String> {
    let tags = normalize_book_tags(tags)?;
    let now = chrono::Utc::now().timestamp_millis();
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| format!("开启标签事务失败: {e}"))?;

    for tag in &tags {
        sqlx::query(
            "INSERT INTO tags (id, name, color, created_at, updated_at)
             VALUES (?, ?, NULL, ?, ?)
             ON CONFLICT(name) DO UPDATE SET updated_at = excluded.updated_at",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(tag)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("保存标签失败: {e}"))?;
    }

    let encoded = if tags.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&tags).map_err(|e| format!("编码标签失败: {e}"))?)
    };
    let result = sqlx::query("UPDATE books SET tags = ?, updated_at = ? WHERE id = ?")
        .bind(encoded)
        .bind(now)
        .bind(book_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("更新书籍标签失败: {e}"))?;
    if result.rows_affected() == 0 {
        return Err("书籍不存在".to_string());
    }
    tx.commit()
        .await
        .map_err(|e| format!("提交标签事务失败: {e}"))?;
    Ok(tags)
}

pub fn normalize_book_title(title: &str) -> Result<String, String> {
    const MAX_TITLE_CHARS: usize = 300;
    let title = title.trim();
    if title.is_empty() {
        return Err("书名不能为空".to_string());
    }
    if title.chars().any(char::is_control) {
        return Err("书名不能包含控制字符".to_string());
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(format!("书名不能超过 {MAX_TITLE_CHARS} 个字符"));
    }
    Ok(title.to_string())
}

/// 只修改应用中的展示书名，不改写用户原始 EPUB 文件。
pub async fn rename_book(pool: &SqlitePool, book_id: &str, title: &str) -> Result<String, String> {
    let title = normalize_book_title(title)?;
    let now = chrono::Utc::now().timestamp_millis();
    let result = sqlx::query("UPDATE books SET title = ?, updated_at = ? WHERE id = ?")
        .bind(&title)
        .bind(now)
        .bind(book_id)
        .execute(pool)
        .await
        .map_err(|error| format!("重命名书籍失败: {error}"))?;
    if result.rows_affected() == 0 {
        return Err("书籍不存在".to_string());
    }
    Ok(title)
}

/// “欲读清单”独立于 unread/reading/completed，存入 status.metadata，
/// 这样加入清单不会破坏真实阅读进度。
pub async fn set_book_want_to_read(
    pool: &SqlitePool,
    book_id: &str,
    want_to_read: bool,
) -> Result<(), String> {
    let metadata = sqlx::query_scalar::<_, Option<String>>(
        "SELECT metadata FROM book_status WHERE book_id = ?",
    )
    .bind(book_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("读取书籍清单状态失败: {error}"))?
    .ok_or_else(|| "书籍不存在".to_string())?;
    let mut object = metadata
        .and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    object.insert(
        "want_to_read".to_string(),
        serde_json::Value::Bool(want_to_read),
    );
    let metadata =
        serde_json::to_string(&object).map_err(|error| format!("编码书籍清单状态失败: {error}"))?;
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query("UPDATE book_status SET metadata = ?, updated_at = ? WHERE book_id = ?")
        .bind(metadata)
        .bind(now)
        .bind(book_id)
        .execute(pool)
        .await
        .map_err(|error| format!("保存书籍清单状态失败: {error}"))?;
    Ok(())
}

pub async fn mark_book_completed(pool: &SqlitePool, book_id: &str) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();
    let result = sqlx::query(
        "UPDATE book_status
         SET status = 'completed',
             progress_current = CASE
                 WHEN progress_total > 0 THEN progress_total
                 ELSE progress_current
             END,
             completed_at = ?, updated_at = ?
         WHERE book_id = ?",
    )
    .bind(now)
    .bind(now)
    .bind(book_id)
    .execute(pool)
    .await
    .map_err(|error| format!("标记书籍已读完失败: {error}"))?;
    if result.rows_affected() == 0 {
        return Err("书籍不存在".to_string());
    }
    Ok(())
}

/// 删除书籍：先删磁盘目录，再删 DB（外键级联清 status 等）。
pub async fn delete_book(pool: &SqlitePool, data_dir: &Path, id: &str) -> Result<(), String> {
    let book_dir = data_dir.join("books").join(id);
    if book_dir.exists() {
        fs::remove_dir_all(&book_dir).map_err(|e| format!("删除书籍文件失败: {}", e))?;
    }
    let result = sqlx::query("DELETE FROM books WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| format!("删除书籍失败: {}", e))?;
    if result.rows_affected() == 0 {
        return Err("书籍不存在".to_string());
    }
    Ok(())
}

/// 高亮区间（块下标 + 块内 UTF-8 字节下标，与 TextFlow 选区 API 一致）。
#[derive(Debug, Clone)]
pub struct Highlight {
    pub id: String,
    pub start_block: usize,
    pub start_char: usize,
    pub end_block: usize,
    pub end_char: usize,
    pub color: String,
}

#[derive(Debug, Clone)]
pub struct Bookmark {
    pub id: String,
    pub location: String,
    pub label: String,
    pub created_at: i64,
}

pub async fn get_bookmarks(pool: &SqlitePool, book_id: &str) -> Result<Vec<Bookmark>, String> {
    let rows = sqlx::query(
        "SELECT id, cfi, text, created_at FROM book_notes \
         WHERE book_id = ? AND type = 'bookmark' ORDER BY created_at DESC",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("读取书签失败: {e}"))?;
    Ok(rows
        .iter()
        .map(|row| Bookmark {
            id: row.get("id"),
            location: row.get("cfi"),
            label: row.get::<Option<String>, _>("text").unwrap_or_default(),
            created_at: row.get("created_at"),
        })
        .collect())
}

pub async fn add_bookmark(
    pool: &SqlitePool,
    book_id: &str,
    location: &str,
    label: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query(
        "INSERT INTO book_notes (id, book_id, type, cfi, text, note, created_at, updated_at) \
         VALUES (?, ?, 'bookmark', ?, ?, '', ?, ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(book_id)
    .bind(location)
    .bind(label)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| format!("保存书签失败: {e}"))?;
    Ok(())
}

pub async fn delete_bookmark(
    pool: &SqlitePool,
    book_id: &str,
    bookmark_id: &str,
) -> Result<(), String> {
    sqlx::query("DELETE FROM book_notes WHERE id = ? AND book_id = ? AND type = 'bookmark'")
        .bind(bookmark_id)
        .bind(book_id)
        .execute(pool)
        .await
        .map_err(|e| format!("删除书签失败: {e}"))?;
    Ok(())
}

const PACK: i64 = 1_000_000;

/// 新增一条高亮（book_notes: type=annotation, style=highlight）。
pub async fn add_highlight(
    pool: &SqlitePool,
    book_id: &str,
    chapter: usize,
    range: (usize, usize, usize, usize),
    color: &str,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();
    let (sb, sc, eb, ec) = range;
    sqlx::query(
        r#"INSERT INTO book_notes (id, book_id, type, cfi, text, style, color, section_id, norm_start, norm_end, note, created_at, updated_at)
           VALUES (?, ?, 'annotation', '', NULL, 'highlight', ?, ?, ?, ?, '', ?, ?)"#,
    )
    .bind(&id)
    .bind(book_id)
    .bind(color)
    .bind(chapter.to_string())
    .bind(sb as i64 * PACK + sc as i64)
    .bind(eb as i64 * PACK + ec as i64)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| format!("保存高亮失败: {}", e))?;
    Ok(id)
}

/// 全书高亮，按章节分组。
pub async fn get_highlights_by_chapter(
    pool: &SqlitePool,
    book_id: &str,
) -> Result<std::collections::HashMap<usize, Vec<Highlight>>, String> {
    let rows = sqlx::query(
        "SELECT id, section_id, norm_start, norm_end, color FROM book_notes
         WHERE book_id = ? AND type = 'annotation' AND style = 'highlight'",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("读取高亮失败: {}", e))?;

    let mut map: std::collections::HashMap<usize, Vec<Highlight>> =
        std::collections::HashMap::new();
    for row in rows.iter() {
        let section: String = row.get("section_id");
        let Ok(chapter) = section.parse::<usize>() else {
            continue;
        };
        let ns: i64 = row.get::<Option<i64>, _>("norm_start").unwrap_or(0);
        let ne: i64 = row.get::<Option<i64>, _>("norm_end").unwrap_or(0);
        map.entry(chapter).or_default().push(Highlight {
            id: row.get("id"),
            start_block: (ns / PACK) as usize,
            start_char: (ns % PACK) as usize,
            end_block: (ne / PACK) as usize,
            end_char: (ne % PACK) as usize,
            color: row.get::<Option<String>, _>("color").unwrap_or_default(),
        });
    }
    Ok(map)
}

/// 按 id 批量删除一本书的高亮。书 id 和高亮类型一起限制删除范围，
/// 避免误删其他书籍的笔记或注释。
pub async fn delete_highlights(
    pool: &SqlitePool,
    book_id: &str,
    ids: &[String],
) -> Result<usize, String> {
    if ids.is_empty() {
        return Ok(0);
    }
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| format!("开始删除高亮事务失败: {e}"))?;
    let mut deleted = 0usize;
    for id in ids {
        deleted += sqlx::query(
            "DELETE FROM book_notes
             WHERE id = ? AND book_id = ? AND type = 'annotation' AND style = 'highlight'",
        )
        .bind(id)
        .bind(book_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("删除高亮失败: {e}"))?
        .rows_affected() as usize;
    }
    tx.commit()
        .await
        .map_err(|e| format!("提交删除高亮事务失败: {e}"))?;
    Ok(deleted)
}

/// 读取一本书的阅读状态（恢复进度用）。
pub async fn get_status(pool: &SqlitePool, id: &str) -> Result<Option<BookStatus>, String> {
    let row = sqlx::query("SELECT * FROM book_status WHERE book_id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("读取阅读状态失败: {}", e))?;
    match row {
        Some(r) => Ok(Some(
            BookStatus::from_db_row(&r).map_err(|e| format!("解析阅读状态失败: {}", e))?,
        )),
        None => Ok(None),
    }
}

/// 更新阅读进度与位置（M3 阅读器保存进度用）。
pub async fn update_progress(
    pool: &SqlitePool,
    id: &str,
    current: i64,
    total: i64,
    location: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();
    let status = if total > 0 && current >= total {
        "completed"
    } else if current > 0 {
        "reading"
    } else {
        "unread"
    };
    sqlx::query(
        r#"UPDATE book_status
           SET progress_current = ?, progress_total = ?, location = ?, status = ?, last_read_at = ?, updated_at = ?
           WHERE book_id = ?"#,
    )
    .bind(current)
    .bind(total)
    .bind(location)
    .bind(status)
    .bind(now)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| format!("更新进度失败: {}", e))?;
    Ok(())
}

/// 扫描"导入目录"里的 EPUB / PDF，导入尚未入库的（按文件名判断粗略去重靠 import 内 uuid，
/// 这里用已入库标题+大小近似跳过重复导入）。返回新导入数量。
pub async fn scan_import_dir(
    pool: &SqlitePool,
    data_dir: &Path,
    import_dir: &Path,
) -> Result<usize, String> {
    if !import_dir.is_dir() {
        return Ok(0);
    }
    let existing: Vec<(String, i64)> = sqlx::query("SELECT title, file_size FROM books")
        .fetch_all(pool)
        .await
        .map_err(|e| format!("读取已有书籍失败: {}", e))?
        .iter()
        .map(|r| (r.get::<String, _>("title"), r.get::<i64, _>("file_size")))
        .collect();

    let mut count = 0;
    let entries = fs::read_dir(import_dir).map_err(|e| format!("读取导入目录失败: {}", e))?;
    for entry in entries.flatten() {
        let path: PathBuf = entry.path();
        if supported_format(&path).is_none() {
            continue;
        }
        let size = fs::metadata(&path).map(|m| m.len() as i64).unwrap_or(0);
        if let Ok(meta) = parse_meta(&path) {
            if existing.iter().any(|(t, s)| *t == meta.title && *s == size) {
                continue;
            }
        }
        if import_book(pool, data_dir, &path).await.is_ok() {
            // 导入目录是"投递箱"：成功入库后移除源文件，
            // 否则删书后重启会被再次扫描导入。
            let _ = fs::remove_file(&path);
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod import_tests {
    use super::*;

    #[test]
    fn failed_pdf_transaction_removes_only_private_import_files() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let pool = runtime
            .block_on(SqlitePool::connect("sqlite::memory:"))
            .unwrap();
        runtime
            .block_on(sqlx::raw_sql(include_str!("../schema.sql")).execute(&pool))
            .unwrap();
        runtime
            .block_on(sqlx::query("DROP TABLE book_status").execute(&pool))
            .unwrap();
        let root = std::env::temp_dir().join(format!("readwise-import-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.pdf");
        fs::write(&source, super::super::pdf::tests::fixture()).unwrap();
        assert!(runtime
            .block_on(import_book(&pool, &root, &source))
            .is_err());
        assert!(source.is_file());
        assert_eq!(fs::read_dir(root.join("books")).unwrap().count(), 0);
        let count: i64 = runtime
            .block_on(sqlx::query_scalar("SELECT COUNT(*) FROM books").fetch_one(&pool))
            .unwrap();
        assert_eq!(count, 0);
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod highlight_tests {
    use super::*;

    fn pool() -> (tokio::runtime::Runtime, SqlitePool) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let pool = runtime
            .block_on(SqlitePool::connect("sqlite::memory:"))
            .unwrap();
        runtime
            .block_on(
                sqlx::query(
                    "CREATE TABLE book_notes (
                        id TEXT PRIMARY KEY NOT NULL,
                        book_id TEXT NOT NULL,
                        type TEXT NOT NULL,
                        cfi TEXT NOT NULL,
                        text TEXT,
                        style TEXT,
                        color TEXT,
                        section_id TEXT,
                        norm_start INTEGER,
                        norm_end INTEGER,
                        note TEXT NOT NULL,
                        created_at INTEGER NOT NULL,
                        updated_at INTEGER NOT NULL
                    )",
                )
                .execute(&pool),
            )
            .unwrap();
        (runtime, pool)
    }

    #[test]
    fn deleting_highlights_is_scoped_to_book_type_and_style() {
        let (runtime, pool) = pool();
        let first = runtime
            .block_on(add_highlight(&pool, "book-1", 0, (0, 1, 0, 4), "yellow"))
            .unwrap();
        let second = runtime
            .block_on(add_highlight(&pool, "book-1", 0, (1, 0, 1, 3), "blue"))
            .unwrap();
        let other_book = runtime
            .block_on(add_highlight(&pool, "book-2", 0, (0, 0, 0, 2), "teal"))
            .unwrap();

        let deleted = runtime
            .block_on(delete_highlights(
                &pool,
                "book-1",
                &[first.clone(), other_book.clone()],
            ))
            .unwrap();
        assert_eq!(deleted, 1);

        let remaining = runtime
            .block_on(get_highlights_by_chapter(&pool, "book-1"))
            .unwrap();
        assert_eq!(remaining[&0].len(), 1);
        assert_eq!(remaining[&0][0].id, second);
        assert_eq!(
            runtime
                .block_on(get_highlights_by_chapter(&pool, "book-2"))
                .unwrap()[&0][0]
                .id,
            other_book
        );
    }

    #[test]
    fn bookmarks_are_scoped_to_their_book_and_do_not_mix_with_highlights() {
        let (runtime, pool) = pool();
        runtime
            .block_on(add_bookmark(&pool, "book-1", "v3:2:4:30", "Chapter 3"))
            .unwrap();
        runtime
            .block_on(add_bookmark(&pool, "book-2", "v3:0:1:0", "Other"))
            .unwrap();
        runtime
            .block_on(add_highlight(&pool, "book-1", 2, (4, 0, 4, 5), "yellow"))
            .unwrap();
        let bookmarks = runtime.block_on(get_bookmarks(&pool, "book-1")).unwrap();
        assert_eq!(bookmarks.len(), 1);
        assert_eq!(bookmarks[0].location, "v3:2:4:30");
        assert_eq!(bookmarks[0].label, "Chapter 3");
        runtime
            .block_on(delete_bookmark(&pool, "book-1", &bookmarks[0].id))
            .unwrap();
        assert!(runtime
            .block_on(get_bookmarks(&pool, "book-1"))
            .unwrap()
            .is_empty());
        assert_eq!(
            runtime
                .block_on(get_bookmarks(&pool, "book-2"))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            runtime
                .block_on(get_highlights_by_chapter(&pool, "book-1"))
                .unwrap()[&2]
                .len(),
            1
        );
    }
}

#[cfg(test)]
mod library_mutation_tests {
    use super::*;

    fn pool() -> (tokio::runtime::Runtime, SqlitePool) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let pool = runtime
            .block_on(SqlitePool::connect("sqlite::memory:"))
            .unwrap();
        runtime
            .block_on(
                sqlx::query(
                    "CREATE TABLE books (
                        id TEXT PRIMARY KEY NOT NULL,
                        title TEXT NOT NULL,
                        author TEXT,
                        file_path TEXT NOT NULL,
                        cover_path TEXT,
                        tags TEXT,
                        created_at INTEGER NOT NULL,
                        updated_at INTEGER NOT NULL
                    );
                    CREATE TABLE book_status (
                        book_id TEXT PRIMARY KEY NOT NULL,
                        status TEXT NOT NULL,
                        progress_current INTEGER DEFAULT 0,
                        progress_total INTEGER DEFAULT 0,
                        location TEXT,
                        last_read_at INTEGER,
                        started_at INTEGER,
                        completed_at INTEGER,
                        metadata TEXT,
                        created_at INTEGER NOT NULL,
                        updated_at INTEGER NOT NULL
                    );",
                )
                .execute(&pool),
            )
            .unwrap();
        runtime
            .block_on(
                sqlx::query(
                    "INSERT INTO books
                     (id, title, author, file_path, cover_path, tags, created_at, updated_at)
                     VALUES ('book-1', 'Old title', 'Author', 'book.epub', NULL, NULL, 1, 1);
                     INSERT INTO book_status
                     (book_id, status, progress_current, progress_total, location, metadata, created_at, updated_at)
                     VALUES ('book-1', 'reading', 3, 10, '', '{\"preserved\":true}', 1, 1);",
                )
                .execute(&pool),
            )
            .unwrap();
        (runtime, pool)
    }

    #[test]
    fn rename_want_to_read_and_finish_preserve_independent_book_state() {
        let (runtime, pool) = pool();
        let title = runtime
            .block_on(rename_book(&pool, "book-1", "  A much shorter title  "))
            .unwrap();
        assert_eq!(title, "A much shorter title");

        runtime
            .block_on(set_book_want_to_read(&pool, "book-1", true))
            .unwrap();
        let shelf = runtime
            .block_on(get_shelf_items(&pool, Path::new("/data")))
            .unwrap();
        assert_eq!(shelf[0].title, title);
        assert!(shelf[0].want_to_read);
        let metadata: String = runtime
            .block_on(
                sqlx::query_scalar("SELECT metadata FROM book_status WHERE book_id = 'book-1'")
                    .fetch_one(&pool),
            )
            .unwrap();
        let metadata: serde_json::Value = serde_json::from_str(&metadata).unwrap();
        assert_eq!(metadata["preserved"], true);

        runtime
            .block_on(mark_book_completed(&pool, "book-1"))
            .unwrap();
        let row = runtime
            .block_on(
                sqlx::query(
                    "SELECT status, progress_current, progress_total, completed_at
                 FROM book_status WHERE book_id = 'book-1'",
                )
                .fetch_one(&pool),
            )
            .unwrap();
        assert_eq!(row.get::<String, _>("status"), "completed");
        assert_eq!(row.get::<i64, _>("progress_current"), 10);
        assert_eq!(row.get::<i64, _>("progress_total"), 10);
        assert!(row.get::<Option<i64>, _>("completed_at").is_some());
    }

    #[test]
    fn rename_rejects_blank_and_control_characters() {
        assert!(normalize_book_title("   ").is_err());
        assert!(normalize_book_title("bad\ntitle").is_err());
    }
}
