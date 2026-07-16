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
    if src.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()) != Some("epub".to_string())
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
    let mut tx = pool.begin().await.map_err(|e| format!("开启事务失败: {}", e))?;

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

    tx.commit().await.map_err(|e| format!("提交事务失败: {}", e))?;

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
        r#"SELECT b.id, b.title, b.author, b.file_path, b.cover_path, b.created_at,
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
            BookShelfItem {
                id: row.get("id"),
                title: row.get("title"),
                author: row.get("author"),
                cover_abs_path: abs(row.get("cover_path")),
                file_abs_path: data_dir.join(file_path).to_string_lossy().into_owned(),
                status: row.get("status"),
                progress_current: row.get("progress_current"),
                progress_total: row.get("progress_total"),
                created_at: row.get("created_at"),
            }
        })
        .collect())
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
        if path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase())
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
