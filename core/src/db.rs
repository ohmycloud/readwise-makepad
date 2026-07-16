use serde::{Deserialize, Serialize};
use sqlx::{migrate::MigrateDatabase, Sqlite, SqlitePool};
use std::fs;
use std::path::Path;

use crate::books::{self, models::*};
use crate::vocab::{self, models::*};

#[derive(Deserialize, Serialize, Debug)]
struct DefaultSkill {
    name: String,
    content: String,
    is_system: bool,
    is_active: bool,
}

/// 同步门面：内部持有 tokio 运行时，UI 侧（Makepad 后台线程）直接调用阻塞方法，
/// 不必在 App 里铺设 async 管线。所有方法在后台线程调用，结果经 Cx::post_action 回 UI。
pub struct Db {
    rt: tokio::runtime::Runtime,
    pool: SqlitePool,
    data_dir: std::path::PathBuf,
}

impl Db {
    /// 打开（必要时创建）应用数据库并初始化 schema。
    pub fn open() -> Result<Self, String> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("创建运行时失败: {}", e))?;
        let data_dir = crate::paths::app_data_dir();
        let pool = rt.block_on(initialize(&data_dir))?;
        Ok(Self { rt, pool, data_dir })
    }

    /// 数据目录（books/ 与 import/ 都在其下）。
    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    // —— 书库 ——

    pub fn import_book(&self, src: &std::path::Path) -> Result<SimpleBook, String> {
        self.rt.block_on(books::import_book(&self.pool, &self.data_dir, src))
    }

    pub fn get_shelf_items(&self) -> Result<Vec<BookShelfItem>, String> {
        self.rt.block_on(books::get_shelf_items(&self.pool, &self.data_dir))
    }

    pub fn delete_book(&self, id: &str) -> Result<(), String> {
        self.rt.block_on(books::delete_book(&self.pool, &self.data_dir, id))
    }

    pub fn update_progress(&self, id: &str, current: i64, total: i64, location: &str) -> Result<(), String> {
        self.rt.block_on(books::update_progress(&self.pool, id, current, total, location))
    }

    /// 扫描导入目录，返回新导入数量。
    pub fn scan_import_dir(&self) -> Result<usize, String> {
        let import_dir = self.data_dir.join("import");
        std::fs::create_dir_all(&import_dir).ok();
        self.rt.block_on(books::scan_import_dir(&self.pool, &self.data_dir, &import_dir))
    }

    pub fn save_book_words(&self, book_id: &str, words: &[BookWord]) -> Result<(), String> {
        self.rt.block_on(vocab::save_book_words(&self.pool, book_id, words))
    }

    pub fn get_book_words(&self, book_id: &str) -> Result<Vec<BookWord>, String> {
        self.rt.block_on(vocab::get_book_words(&self.pool, book_id))
    }

    pub fn save_book_glosses(&self, book_id: &str, glosses: &[GlossInput]) -> Result<(), String> {
        self.rt.block_on(vocab::save_book_glosses(&self.pool, book_id, glosses))
    }

    pub fn get_book_glosses(&self, book_id: &str) -> Result<Vec<WordGloss>, String> {
        self.rt.block_on(vocab::get_book_glosses(&self.pool, book_id))
    }

    pub fn get_word_glosses(&self, lemmas: &[String]) -> Result<Vec<WordGloss>, String> {
        self.rt.block_on(vocab::get_word_glosses(&self.pool, lemmas))
    }

    pub fn add_known_word(&self, lemma: &str, source: &str) -> Result<(), String> {
        self.rt.block_on(vocab::add_known_word(&self.pool, lemma, source))
    }

    pub fn remove_known_word(&self, lemma: &str) -> Result<(), String> {
        self.rt.block_on(vocab::remove_known_word(&self.pool, lemma))
    }

    pub fn get_known_words(&self) -> Result<Vec<String>, String> {
        self.rt.block_on(vocab::get_known_words(&self.pool))
    }

    pub fn vocab_book_add(&self, entry: &VocabBookAddInput) -> Result<bool, String> {
        self.rt.block_on(vocab::vocab_book_add(&self.pool, entry))
    }

    pub fn vocab_book_list(&self) -> Result<Vec<VocabBookEntry>, String> {
        self.rt.block_on(vocab::vocab_book_list(&self.pool))
    }

    pub fn vocab_book_remove(&self, lemma: &str) -> Result<(), String> {
        self.rt.block_on(vocab::vocab_book_remove(&self.pool, lemma))
    }

    pub fn vocab_book_set_status(&self, lemma: &str, status: &str) -> Result<(), String> {
        self.rt.block_on(vocab::vocab_book_set_status(&self.pool, lemma, status))
    }

    pub fn vocab_book_review(&self, lemma: &str) -> Result<(), String> {
        self.rt.block_on(vocab::vocab_book_review(&self.pool, lemma))
    }
}

/// 打开/创建数据库并执行 schema（自老项目 database.rs 迁移，去 tauri 化：
/// 数据目录由调用方传入）。
pub async fn initialize(data_dir: &Path) -> Result<SqlitePool, String> {
    let db_dir = data_dir.join("database");
    fs::create_dir_all(&db_dir).map_err(|e| format!("创建数据目录失败: {}", e))?;

    let db_path = db_dir.join("app.db");
    let db_url = format!("sqlite:{}", db_path.display());

    let is_new_db = !Sqlite::database_exists(&db_url).await.unwrap_or(false);
    if is_new_db {
        Sqlite::create_database(&db_url)
            .await
            .map_err(|e| format!("创建数据库失败: {}", e))?;
        log::info!("Database created at: {}", db_url);
    } else {
        log::info!("Database found at: {}", db_url);
    }

    let pool = SqlitePool::connect(&db_url)
        .await
        .map_err(|e| format!("连接数据库失败: {}", e))?;

    sqlx::query(include_str!("./schema.sql"))
        .execute(&pool)
        .await
        .map_err(|e| format!("初始化 schema 失败: {}", e))?;

    ensure_book_note_columns(&pool).await?;

    if is_new_db {
        initialize_default_skills(&pool).await?;
    }

    Ok(pool)
}

async fn ensure_book_note_columns(pool: &SqlitePool) -> Result<(), String> {
    let statements = [
        "ALTER TABLE book_notes ADD COLUMN section_id TEXT",
        "ALTER TABLE book_notes ADD COLUMN norm_start INTEGER",
        "ALTER TABLE book_notes ADD COLUMN norm_end INTEGER",
    ];

    for statement in statements {
        if let Err(error) = sqlx::query(statement).execute(pool).await {
            let message = error.to_string();
            if !message.contains("duplicate column name") {
                return Err(format!("补充 book_notes 列失败: {}", message));
            }
        }
    }
    Ok(())
}

async fn initialize_default_skills(pool: &SqlitePool) -> Result<(), String> {
    let default_skills_json = include_str!("./default-skills.json");
    let default_skills: Vec<DefaultSkill> =
        serde_json::from_str(default_skills_json).map_err(|e| format!("解析默认 skills 失败: {}", e))?;

    log::info!("Initializing {} default skills...", default_skills.len());

    for skill in default_skills {
        let skill_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp_millis();

        sqlx::query(
            r#"
            INSERT INTO skills (id, name, content, is_active, is_system, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&skill_id)
        .bind(&skill.name)
        .bind(&skill.content)
        .bind(if skill.is_active { 1 } else { 0 })
        .bind(if skill.is_system { 1 } else { 0 })
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .map_err(|e| format!("初始化默认 skill 失败: {}", e))?;
    }

    Ok(())
}
