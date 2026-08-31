use serde::{Deserialize, Serialize};
use sqlx::Row;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SimpleBook {
    pub id: String,
    pub title: String,
    pub author: String,
    pub format: String,
    pub file_path: String,
    pub cover_path: Option<String>,
    pub file_size: i64,
    pub language: String,
    pub tags: Option<Vec<String>>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl SimpleBook {
    pub fn from_db_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        let tags_str: Option<String> = row.try_get("tags")?;
        let tags = tags_str.and_then(|s| serde_json::from_str(&s).ok());
        Ok(Self {
            id: row.try_get("id")?,
            title: row.try_get("title")?,
            author: row.try_get("author")?,
            format: row.try_get("format")?,
            file_path: row.try_get("file_path")?,
            cover_path: row.try_get("cover_path")?,
            file_size: row.try_get("file_size")?,
            language: row.try_get("language")?,
            tags,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BookStatus {
    pub book_id: String,
    pub status: String, // 'unread' | 'reading' | 'completed'
    pub progress_current: i64,
    pub progress_total: i64,
    pub location: String,
    pub last_read_at: Option<i64>,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl BookStatus {
    pub fn from_db_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            book_id: row.try_get("book_id")?,
            status: row.try_get("status")?,
            progress_current: row.try_get("progress_current")?,
            progress_total: row.try_get("progress_total")?,
            location: row.try_get("location")?,
            last_read_at: row.try_get("last_read_at")?,
            started_at: row.try_get("started_at")?,
            completed_at: row.try_get("completed_at")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

/// 书架列表项：书 + 可选状态（含封面绝对路径，UI 直接用）。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BookShelfItem {
    pub id: String,
    pub title: String,
    pub author: String,
    /// 封面文件的绝对路径（无封面为 None）。
    pub cover_abs_path: Option<String>,
    /// EPUB 文件的绝对路径。
    pub file_abs_path: String,
    pub status: String,
    pub progress_current: i64,
    pub progress_total: i64,
    /// 独立于阅读进度状态的“欲读清单”标记，保存在 book_status.metadata。
    pub want_to_read: bool,
    /// 用户为书籍设置的标签名称，顺序与编辑时一致。
    pub tags: Vec<String>,
    pub created_at: i64,
}
