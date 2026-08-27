use super::models::{Thread, ThreadMessage};
use sqlx::{Row, SqlitePool};

fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Thread, String> {
    let messages_json: String = row
        .try_get("messages")
        .map_err(|e| format!("读取对话消息失败: {}", e))?;
    let messages =
        serde_json::from_str(&messages_json).map_err(|e| format!("解析对话消息失败: {}", e))?;
    Ok(Thread {
        id: row
            .try_get("id")
            .map_err(|e| format!("读取对话 id 失败: {}", e))?,
        book_id: row
            .try_get("book_id")
            .map_err(|e| format!("读取对话 book_id 失败: {}", e))?,
        metadata: row
            .try_get("metadata")
            .map_err(|e| format!("读取对话元数据失败: {}", e))?,
        title: row
            .try_get("title")
            .map_err(|e| format!("读取对话标题失败: {}", e))?,
        messages,
        created_at: row
            .try_get("created_at")
            .map_err(|e| format!("读取对话创建时间失败: {}", e))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|e| format!("读取对话更新时间失败: {}", e))?,
    })
}

pub async fn latest_thread(
    pool: &SqlitePool,
    book_id: Option<&str>,
) -> Result<Option<Thread>, String> {
    let row = sqlx::query(
        "SELECT id, book_id, metadata, title, messages, created_at, updated_at \
         FROM threads WHERE book_id IS ? ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(book_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("读取最近对话失败: {}", e))?;
    row.as_ref().map(from_row).transpose()
}

pub async fn save_thread(
    pool: &SqlitePool,
    thread_id: Option<&str>,
    book_id: Option<&str>,
    title: &str,
    metadata: &str,
    messages: &[ThreadMessage],
) -> Result<Thread, String> {
    let now = chrono::Utc::now().timestamp_millis();
    let messages_json =
        serde_json::to_string(messages).map_err(|e| format!("序列化对话失败: {}", e))?;
    let id = thread_id
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    sqlx::query(
        "INSERT INTO threads (id, book_id, metadata, title, messages, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET metadata = excluded.metadata, title = excluded.title, \
         messages = excluded.messages, updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(book_id)
    .bind(metadata)
    .bind(title)
    .bind(&messages_json)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| format!("保存对话失败: {}", e))?;

    let row = sqlx::query(
        "SELECT id, book_id, metadata, title, messages, created_at, updated_at \
         FROM threads WHERE id = ?",
    )
    .bind(&id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("读取已保存对话失败: {}", e))?;
    from_row(&row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::threads::models::ThreadRole;

    #[test]
    fn saves_and_reloads_latest_thread() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
            sqlx::query(
                "CREATE TABLE threads (\
                    id TEXT PRIMARY KEY NOT NULL, book_id TEXT, metadata TEXT NOT NULL, \
                    title TEXT NOT NULL, messages TEXT NOT NULL, created_at INTEGER NOT NULL, \
                    updated_at INTEGER NOT NULL\
                )",
            )
            .execute(&pool)
            .await
            .unwrap();

            let first = vec![ThreadMessage {
                role: ThreadRole::User,
                content: "Explain this sentence".into(),
            }];
            let saved = save_thread(&pool, None, None, "Explain this sentence", "{}", &first)
                .await
                .unwrap();
            let second = vec![
                first[0].clone(),
                ThreadMessage {
                    role: ThreadRole::Assistant,
                    content: "Here is the explanation".into(),
                },
            ];
            save_thread(
                &pool,
                Some(&saved.id),
                None,
                "Explain this sentence",
                "{}",
                &second,
            )
            .await
            .unwrap();

            let loaded = latest_thread(&pool, None).await.unwrap().unwrap();
            assert_eq!(loaded.id, saved.id);
            assert_eq!(loaded.messages, second);
        });
    }
}
