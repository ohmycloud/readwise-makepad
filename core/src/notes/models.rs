use sqlx::Row;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: String,
    pub book_id: Option<String>,
    pub book_title: Option<String>,
    pub book_author: Option<String>,
    pub title: String,
    pub content: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Note {
    pub(crate) fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, String> {
        let book_meta: Option<String> = row
            .try_get("book_meta")
            .map_err(|e| format!("读取笔记书籍信息失败: {}", e))?;
        let (book_title, book_author) = book_meta
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .map(|meta| {
                (
                    meta.get("title")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned),
                    meta.get("author")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned),
                )
            })
            .unwrap_or((None, None));

        Ok(Self {
            id: row
                .try_get("id")
                .map_err(|e| format!("读取笔记 id 失败: {}", e))?,
            book_id: row
                .try_get("book_id")
                .map_err(|e| format!("读取笔记 book_id 失败: {}", e))?,
            book_title,
            book_author,
            title: row
                .try_get::<Option<String>, _>("title")
                .map_err(|e| format!("读取笔记标题失败: {}", e))?
                .unwrap_or_default(),
            content: row
                .try_get::<Option<String>, _>("content")
                .map_err(|e| format!("读取笔记内容失败: {}", e))?
                .unwrap_or_default(),
            created_at: row
                .try_get("created_at")
                .map_err(|e| format!("读取笔记创建时间失败: {}", e))?,
            updated_at: row
                .try_get("updated_at")
                .map_err(|e| format!("读取笔记更新时间失败: {}", e))?,
        })
    }
}
