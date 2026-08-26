use super::models::Note;
use sqlx::SqlitePool;

fn note_title(content: &str) -> String {
    let compact = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = compact.chars();
    let title: String = chars.by_ref().take(50).collect();
    if chars.next().is_some() {
        format!("{}…", title)
    } else {
        title
    }
}

pub async fn create_note(
    pool: &SqlitePool,
    book_id: &str,
    book_title: &str,
    book_author: &str,
    content: &str,
) -> Result<Note, String> {
    let content = content.trim();
    if content.is_empty() {
        return Err("笔记内容不能为空".to_string());
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();
    let title = note_title(content);
    let book_meta = serde_json::json!({"title": book_title, "author": book_author}).to_string();

    sqlx::query(
        "INSERT INTO notes (id, book_id, book_meta, title, content, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(book_id)
    .bind(book_meta)
    .bind(&title)
    .bind(content)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| format!("创建笔记失败: {}", e))?;

    Ok(Note {
        id,
        book_id: Some(book_id.to_string()),
        book_title: Some(book_title.to_string()),
        book_author: Some(book_author.to_string()),
        title,
        content: content.to_string(),
        created_at: now,
        updated_at: now,
    })
}

pub async fn update_note(pool: &SqlitePool, id: &str, content: &str) -> Result<Note, String> {
    let content = content.trim();
    if content.is_empty() {
        return Err("笔记内容不能为空".to_string());
    }
    let title = note_title(content);
    let now = chrono::Utc::now().timestamp_millis();
    let result =
        sqlx::query("UPDATE notes SET title = ?, content = ?, updated_at = ? WHERE id = ?")
            .bind(&title)
            .bind(content)
            .bind(now)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| format!("更新笔记失败: {}", e))?;
    if result.rows_affected() == 0 {
        return Err("笔记不存在".to_string());
    }
    get_note(pool, id)
        .await?
        .ok_or_else(|| "更新后读取笔记失败".to_string())
}

pub async fn get_note(pool: &SqlitePool, id: &str) -> Result<Option<Note>, String> {
    let row = sqlx::query(
        "SELECT id, book_id, book_meta, title, content, created_at, updated_at
         FROM notes WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("读取笔记失败: {}", e))?;
    row.as_ref().map(Note::from_row).transpose()
}

pub async fn get_notes_by_book(pool: &SqlitePool, book_id: &str) -> Result<Vec<Note>, String> {
    let rows = sqlx::query(
        "SELECT id, book_id, book_meta, title, content, created_at, updated_at
         FROM notes WHERE book_id = ? ORDER BY updated_at DESC",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("读取书籍笔记失败: {}", e))?;
    rows.iter().map(Note::from_row).collect()
}

pub async fn delete_note(pool: &SqlitePool, id: &str) -> Result<(), String> {
    let result = sqlx::query("DELETE FROM notes WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| format!("删除笔记失败: {}", e))?;
    if result.rows_affected() == 0 {
        return Err("笔记不存在".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool() -> (tokio::runtime::Runtime, SqlitePool) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let pool = rt.block_on(SqlitePool::connect("sqlite::memory:")).unwrap();
        rt.block_on(
            sqlx::query(
                "CREATE TABLE notes (
                    id TEXT PRIMARY KEY NOT NULL,
                    book_id TEXT,
                    book_meta TEXT,
                    title TEXT,
                    content TEXT,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                )",
            )
            .execute(&pool),
        )
        .unwrap();
        (rt, pool)
    }

    #[test]
    fn note_crud_keeps_unicode_and_book_meta() {
        let (rt, pool) = pool();
        let note = rt
            .block_on(create_note(
                &pool,
                "book-1",
                "The Book",
                "A. Writer",
                "引用\n\n补充内容",
            ))
            .unwrap();
        assert_eq!(note.book_title.as_deref(), Some("The Book"));
        assert_eq!(note.content, "引用\n\n补充内容");

        let updated = rt
            .block_on(update_note(&pool, &note.id, "新的内容 with English"))
            .unwrap();
        assert_eq!(updated.content, "新的内容 with English");
        assert_eq!(
            rt.block_on(get_notes_by_book(&pool, "book-1"))
                .unwrap()
                .len(),
            1
        );

        rt.block_on(delete_note(&pool, &note.id)).unwrap();
        assert!(rt
            .block_on(get_notes_by_book(&pool, "book-1"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn note_rejects_blank_content_and_truncates_title_by_chars() {
        let (rt, pool) = pool();
        assert!(rt
            .block_on(create_note(&pool, "book-1", "Book", "Author", "  \n "))
            .is_err());
        let long = "你".repeat(51);
        let note = rt
            .block_on(create_note(&pool, "book-1", "Book", "Author", &long))
            .unwrap();
        assert_eq!(note.title.chars().count(), 51);
        assert!(note.title.ends_with('…'));
    }
}
