use serde::{Deserialize, Serialize};
use sqlx::{migrate::MigrateDatabase, Sqlite, SqlitePool};
use std::fs;
use std::path::Path;

use crate::books::{self, models::*};
use crate::notes::{self, models::Note};
use crate::skills::{self, models::Skill};
use crate::statistics::{self, models::StatisticsSnapshot};
use crate::threads::{self, models::*};
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
        self.rt
            .block_on(books::import_book(&self.pool, &self.data_dir, src))
    }

    pub fn get_shelf_items(&self) -> Result<Vec<BookShelfItem>, String> {
        self.rt
            .block_on(books::get_shelf_items(&self.pool, &self.data_dir))
    }

    pub fn delete_book(&self, id: &str) -> Result<(), String> {
        self.rt
            .block_on(books::delete_book(&self.pool, &self.data_dir, id))
    }

    pub fn update_progress(
        &self,
        id: &str,
        current: i64,
        total: i64,
        location: &str,
    ) -> Result<(), String> {
        self.rt.block_on(books::update_progress(
            &self.pool, id, current, total, location,
        ))
    }

    pub fn get_status(&self, id: &str) -> Result<Option<BookStatus>, String> {
        self.rt.block_on(books::get_status(&self.pool, id))
    }

    pub fn add_highlight(
        &self,
        book_id: &str,
        chapter: usize,
        range: (usize, usize, usize, usize),
        color: &str,
    ) -> Result<String, String> {
        self.rt.block_on(books::add_highlight(
            &self.pool, book_id, chapter, range, color,
        ))
    }

    pub fn get_highlights_by_chapter(
        &self,
        book_id: &str,
    ) -> Result<std::collections::HashMap<usize, Vec<books::Highlight>>, String> {
        self.rt
            .block_on(books::get_highlights_by_chapter(&self.pool, book_id))
    }

    // —— 阅读统计 ——

    pub fn start_reading_session(&self, book_id: &str) -> Result<String, String> {
        self.rt
            .block_on(statistics::start_reading_session(&self.pool, book_id))
    }

    pub fn checkpoint_reading_session(&self, id: &str) -> Result<(), String> {
        self.rt
            .block_on(statistics::checkpoint_reading_session(&self.pool, id))
    }

    pub fn finish_reading_session(&self, id: &str) -> Result<(), String> {
        self.rt
            .block_on(statistics::finish_reading_session(&self.pool, id))
    }

    pub fn statistics_snapshot(&self) -> Result<StatisticsSnapshot, String> {
        self.rt
            .block_on(statistics::statistics_snapshot(&self.pool))
    }

    // —— AI 设置与对话 ——

    pub fn load_ai_settings(&self) -> Result<crate::ai::AiSettings, String> {
        self.rt.block_on(async {
            use sqlx::Row;

            let rows = sqlx::query(
                "SELECT key, value FROM app_settings WHERE key IN ('ai.endpoint', 'ai.model', 'ai.api_key')",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| format!("读取 AI 设置失败: {}", e))?;
            let mut settings = crate::ai::AiSettings::default();
            for row in rows {
                let key: String = row.get("key");
                let value: String = row.get("value");
                match key.as_str() {
                    "ai.endpoint" => settings.endpoint = value,
                    "ai.model" => settings.model = value,
                    "ai.api_key" => settings.api_key = value,
                    _ => {}
                }
            }
            Ok(settings)
        })
    }

    pub fn save_ai_settings(&self, settings: &crate::ai::AiSettings) -> Result<(), String> {
        self.rt.block_on(async {
            let now = chrono::Utc::now().timestamp_millis();
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| format!("开启 AI 设置事务失败: {}", e))?;
            for (key, value) in [
                ("ai.endpoint", settings.endpoint.as_str()),
                ("ai.model", settings.model.as_str()),
                ("ai.api_key", settings.api_key.as_str()),
            ] {
                sqlx::query(
                    "INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?) \
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                )
                .bind(key)
                .bind(value)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("保存 AI 设置失败: {}", e))?;
            }
            tx.commit()
                .await
                .map_err(|e| format!("提交 AI 设置失败: {}", e))
        })
    }

    pub fn load_local_ai_settings(&self) -> Result<crate::local_ai::LocalAiSettings, String> {
        self.rt.block_on(async {
            use sqlx::Row;

            let rows =
                sqlx::query("SELECT key, value FROM app_settings WHERE key LIKE 'local_ai.%'")
                    .fetch_all(&self.pool)
                    .await
                    .map_err(|e| format!("读取本地 AI 设置失败: {e}"))?;
            let mut settings = crate::local_ai::LocalAiSettings::default();
            for row in rows {
                let key: String = row.get("key");
                let value: String = row.get("value");
                match key.as_str() {
                    "local_ai.binary_path" => settings.binary_path = value,
                    "local_ai.model_path" => settings.model_path = value,
                    "local_ai.port" => {
                        settings.port = value.parse().unwrap_or(settings.port);
                    }
                    "local_ai.context_size" => {
                        settings.context_size = value.parse().unwrap_or(settings.context_size);
                    }
                    "local_ai.threads" => {
                        settings.threads = value.parse().unwrap_or(settings.threads);
                    }
                    "local_ai.gpu_layers" => {
                        settings.gpu_layers = value.parse().unwrap_or(settings.gpu_layers);
                    }
                    "local_ai.auto_start" => settings.auto_start = value == "1",
                    _ => {}
                }
            }
            Ok(settings)
        })
    }

    pub fn save_local_ai_settings(
        &self,
        settings: &crate::local_ai::LocalAiSettings,
    ) -> Result<(), String> {
        self.rt.block_on(async {
            let now = chrono::Utc::now().timestamp_millis();
            let port = settings.port.to_string();
            let context_size = settings.context_size.to_string();
            let threads = settings.threads.to_string();
            let gpu_layers = settings.gpu_layers.to_string();
            let auto_start = if settings.auto_start { "1" } else { "0" };
            let values = [
                ("local_ai.binary_path", settings.binary_path.as_str()),
                ("local_ai.model_path", settings.model_path.as_str()),
                ("local_ai.port", port.as_str()),
                ("local_ai.context_size", context_size.as_str()),
                ("local_ai.threads", threads.as_str()),
                ("local_ai.gpu_layers", gpu_layers.as_str()),
                ("local_ai.auto_start", auto_start),
            ];
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| format!("开启本地 AI 设置事务失败: {e}"))?;
            for (key, value) in values {
                sqlx::query(
                    "INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?) \
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                )
                .bind(key)
                .bind(value)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("保存本地 AI 设置失败: {e}"))?;
            }
            tx.commit()
                .await
                .map_err(|e| format!("提交本地 AI 设置失败: {e}"))
        })
    }

    pub fn latest_thread(&self, book_id: Option<&str>) -> Result<Option<Thread>, String> {
        self.rt
            .block_on(threads::latest_thread(&self.pool, book_id))
    }

    pub fn save_thread(
        &self,
        thread_id: Option<&str>,
        book_id: Option<&str>,
        title: &str,
        metadata: &str,
        messages: &[ThreadMessage],
    ) -> Result<Thread, String> {
        self.rt.block_on(threads::save_thread(
            &self.pool, thread_id, book_id, title, metadata, messages,
        ))
    }

    // —— AI 技能 / Prompt 模板 ——

    pub fn list_skills(&self) -> Result<Vec<Skill>, String> {
        self.rt.block_on(skills::queries::list_skills(&self.pool))
    }

    pub fn create_skill(
        &self,
        name: &str,
        content: &str,
        is_active: bool,
    ) -> Result<Skill, String> {
        self.rt.block_on(skills::queries::create_skill(
            &self.pool, name, content, is_active,
        ))
    }

    pub fn update_skill(
        &self,
        id: &str,
        name: &str,
        content: &str,
        is_active: bool,
    ) -> Result<Skill, String> {
        self.rt.block_on(skills::queries::update_skill(
            &self.pool, id, name, content, is_active,
        ))
    }

    pub fn set_skill_active(&self, id: &str, is_active: bool) -> Result<(), String> {
        self.rt
            .block_on(skills::queries::set_skill_active(&self.pool, id, is_active))
    }

    pub fn delete_skill(&self, id: &str) -> Result<(), String> {
        self.rt
            .block_on(skills::queries::delete_skill(&self.pool, id))
    }

    // —— 笔记 ——

    pub fn create_note(
        &self,
        book_id: &str,
        book_title: &str,
        book_author: &str,
        content: &str,
    ) -> Result<Note, String> {
        self.rt.block_on(notes::create_note(
            &self.pool,
            book_id,
            book_title,
            book_author,
            content,
        ))
    }

    pub fn update_note(&self, id: &str, content: &str) -> Result<Note, String> {
        self.rt
            .block_on(notes::update_note(&self.pool, id, content))
    }

    pub fn get_notes_by_book(&self, book_id: &str) -> Result<Vec<Note>, String> {
        self.rt
            .block_on(notes::get_notes_by_book(&self.pool, book_id))
    }

    pub fn delete_note(&self, id: &str) -> Result<(), String> {
        self.rt.block_on(notes::delete_note(&self.pool, id))
    }

    /// 扫描导入目录，返回新导入数量。
    pub fn scan_import_dir(&self) -> Result<usize, String> {
        let import_dir = self.data_dir.join("import");
        std::fs::create_dir_all(&import_dir).ok();
        self.rt.block_on(books::scan_import_dir(
            &self.pool,
            &self.data_dir,
            &import_dir,
        ))
    }

    pub fn save_book_words(&self, book_id: &str, words: &[BookWord]) -> Result<(), String> {
        self.rt
            .block_on(vocab::save_book_words(&self.pool, book_id, words))
    }

    pub fn get_book_words(&self, book_id: &str) -> Result<Vec<BookWord>, String> {
        self.rt.block_on(vocab::get_book_words(&self.pool, book_id))
    }

    pub fn save_book_glosses(&self, book_id: &str, glosses: &[GlossInput]) -> Result<(), String> {
        self.rt
            .block_on(vocab::save_book_glosses(&self.pool, book_id, glosses))
    }

    pub fn get_book_glosses(&self, book_id: &str) -> Result<Vec<WordGloss>, String> {
        self.rt
            .block_on(vocab::get_book_glosses(&self.pool, book_id))
    }

    pub fn get_word_glosses(&self, lemmas: &[String]) -> Result<Vec<WordGloss>, String> {
        self.rt
            .block_on(vocab::get_word_glosses(&self.pool, lemmas))
    }

    pub fn add_known_word(&self, lemma: &str, source: &str) -> Result<(), String> {
        self.rt
            .block_on(vocab::add_known_word(&self.pool, lemma, source))
    }

    pub fn remove_known_word(&self, lemma: &str) -> Result<(), String> {
        self.rt
            .block_on(vocab::remove_known_word(&self.pool, lemma))
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
        self.rt
            .block_on(vocab::vocab_book_remove(&self.pool, lemma))
    }

    pub fn vocab_book_set_status(&self, lemma: &str, status: &str) -> Result<(), String> {
        self.rt
            .block_on(vocab::vocab_book_set_status(&self.pool, lemma, status))
    }

    pub fn vocab_book_review(&self, lemma: &str) -> Result<(), String> {
        self.rt
            .block_on(vocab::vocab_book_review(&self.pool, lemma))
    }

    pub fn vocab_book_review_with_status(&self, lemma: &str, status: &str) -> Result<(), String> {
        self.rt.block_on(vocab::vocab_book_review_with_status(
            &self.pool, lemma, status,
        ))
    }

    /// 注音上下文：全局释义 +（可选）本书语境释义覆盖，以及已认识词集合。
    pub fn gloss_context(
        &self,
        book_id: Option<&str>,
    ) -> Result<
        (
            std::collections::HashMap<String, String>,
            std::collections::HashSet<String>,
        ),
        String,
    > {
        self.rt.block_on(async {
            let rows = sqlx::query("SELECT lemma, gloss FROM word_glosses")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| format!("读取全局释义失败: {}", e))?;
            let mut glosses: std::collections::HashMap<String, String> = rows
                .iter()
                .map(|r| {
                    use sqlx::Row;
                    (r.get("lemma"), r.get("gloss"))
                })
                .collect();
            if let Some(id) = book_id {
                for g in vocab::get_book_glosses(&self.pool, id).await? {
                    glosses.insert(g.lemma, g.gloss);
                }
            }
            let known: std::collections::HashSet<String> = vocab::get_known_words(&self.pool)
                .await?
                .into_iter()
                .collect();
            Ok((glosses, known))
        })
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
    statistics::recover_open_reading_sessions(&pool).await?;
    initialize_starter_glosses(&pool).await?;

    initialize_default_skills(&pool).await?;

    Ok(pool)
}

/// 新安装或释义缓存近乎为空时，补入离线基础词义。
///
/// 只包含现有词级表的 2-4 级词，避免一级常用词满屏注音；用户/书籍语境释义
/// 仍通过 INSERT OR IGNORE 保持更高优先级。来源与许可见 data/ECDICT-LICENSE.md。
async fn initialize_starter_glosses(pool: &SqlitePool) -> Result<(), String> {
    use sqlx::Row;

    let count: i64 = sqlx::query("SELECT COUNT(*) AS count FROM word_glosses")
        .fetch_one(pool)
        .await
        .map_err(|e| format!("统计释义缓存失败: {}", e))?
        .get("count");
    if count >= 1000 {
        return Ok(());
    }

    let glosses: std::collections::HashMap<String, String> =
        serde_json::from_str(include_str!("./vocab/data/starter-glosses.json"))
            .map_err(|e| format!("解析内置基础释义失败: {}", e))?;
    let now = chrono::Utc::now().timestamp_millis();
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| format!("开启基础释义事务失败: {}", e))?;
    for (lemma, gloss) in &glosses {
        sqlx::query(
            "INSERT OR IGNORE INTO word_glosses (lemma, gloss, level, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(lemma)
        .bind(gloss)
        .bind(crate::vocab::lemma::word_level(lemma))
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("写入基础释义失败: {}", e))?;
    }
    tx.commit()
        .await
        .map_err(|e| format!("提交基础释义失败: {}", e))?;
    log::info!("Initialized {} starter glosses", glosses.len());
    Ok(())
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
    let default_skills: Vec<DefaultSkill> = serde_json::from_str(default_skills_json)
        .map_err(|e| format!("解析默认 skills 失败: {}", e))?;

    log::info!("Initializing {} default skills...", default_skills.len());

    for skill in default_skills {
        let skill_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp_millis();

        sqlx::query(
            r#"
            INSERT OR IGNORE INTO skills
                (id, name, content, is_active, is_system, created_at, updated_at)
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
