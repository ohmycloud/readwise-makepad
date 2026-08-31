use super::epub_meta;
use super::models::*;
use sqlx::{Row, SqlitePool};
use std::fs;
use std::path::{Path, PathBuf};

fn cover_ext(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "jpg",
    }
}

/// 导入一个 EPUB 文件：Rust 侧解析元数据 → 复制文件与封面到数据目录 → 入库。
/// 源文件不删（拖放/导入目录的是用户原文件）。返回新书。
pub async fn import_book(
    pool: &SqlitePool,
    data_dir: &Path,
    src: &Path,
) -> Result<SimpleBook, String> {
    if src
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        != Some("epub".to_string())
    {
        return Err(format!("暂只支持 EPUB：{}", src.display()));
    }

    let meta = epub_meta::parse_meta(src)?;
    let id = uuid::Uuid::new_v4().to_string();
    let file_size = fs::metadata(src).map(|m| m.len() as i64).unwrap_or(0);

    let book_dir = data_dir.join("books").join(&id);
    fs::create_dir_all(&book_dir).map_err(|e| format!("创建书目录失败: {}", e))?;

    let epub_rel = format!("books/{}/book.epub", id);
    fs::copy(src, data_dir.join(&epub_rel)).map_err(|e| format!("复制书籍失败: {}", e))?;

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
           VALUES (?, ?, ?, 'epub', ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(&id)
    .bind(&meta.title)
    .bind(&meta.author)
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

    Ok(SimpleBook {
        id,
        title: meta.title,
        author: meta.author,
        format: "epub".to_string(),
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
                  COALESCE(s.progress_total, 0) AS progress_total
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
            BookShelfItem {
                id: row.get("id"),
                title: row.get("title"),
                author: row.get("author"),
                cover_abs_path: abs(row.get("cover_path")),
                file_abs_path: data_dir.join(file_path).to_string_lossy().into_owned(),
                status: row.get("status"),
                progress_current: row.get("progress_current"),
                progress_total: row.get("progress_total"),
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

/// 扫描"导入目录"里的 epub，导入尚未入库的（按文件名判断粗略去重靠 import 内 uuid，
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
        if path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            != Some("epub".to_string())
        {
            continue;
        }
        let size = fs::metadata(&path).map(|m| m.len() as i64).unwrap_or(0);
        if let Ok(meta) = epub_meta::parse_meta(&path) {
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
}
