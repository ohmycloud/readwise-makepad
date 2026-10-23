use serde::{Deserialize, Serialize};
use sqlx::{
    migrate::MigrateDatabase,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    Row, Sqlite, SqlitePool,
};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

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

/// 1 = 老 Tauri 版基线；2 = app_settings + book_notes 高亮位置；3 = C3 词库。
const CURRENT_SCHEMA_VERSION: i64 = 3;

const REQUIRED_SCHEMA: &[(&str, &[&str])] = &[
    (
        "threads",
        &[
            "id",
            "book_id",
            "metadata",
            "title",
            "messages",
            "created_at",
            "updated_at",
        ],
    ),
    ("app_settings", &["key", "value", "updated_at"]),
    (
        "books",
        &[
            "id",
            "title",
            "author",
            "format",
            "file_path",
            "cover_path",
            "file_size",
            "language",
            "tags",
            "created_at",
            "updated_at",
        ],
    ),
    (
        "book_status",
        &[
            "book_id",
            "status",
            "progress_current",
            "progress_total",
            "location",
            "last_read_at",
            "started_at",
            "completed_at",
            "metadata",
            "created_at",
            "updated_at",
        ],
    ),
    (
        "reading_sessions",
        &[
            "id",
            "book_id",
            "started_at",
            "ended_at",
            "duration_seconds",
            "created_at",
            "updated_at",
        ],
    ),
    ("tags", &["id", "name", "color", "created_at", "updated_at"]),
    (
        "notes",
        &[
            "id",
            "book_id",
            "book_meta",
            "title",
            "content",
            "created_at",
            "updated_at",
        ],
    ),
    (
        "book_notes",
        &[
            "id",
            "book_id",
            "type",
            "cfi",
            "text",
            "style",
            "color",
            "section_id",
            "norm_start",
            "norm_end",
            "note",
            "context_before",
            "context_after",
            "created_at",
            "updated_at",
        ],
    ),
    (
        "public_highlight_retry_queue",
        &[
            "id",
            "job_type",
            "payload_json",
            "attempts",
            "last_error",
            "created_at",
            "next_retry_at",
        ],
    ),
    (
        "skills",
        &[
            "id",
            "name",
            "content",
            "is_active",
            "is_system",
            "created_at",
            "updated_at",
        ],
    ),
    ("word_glosses", &["lemma", "gloss", "level", "created_at"]),
    ("book_words", &["book_id", "lemma", "context_sentence"]),
    ("book_glosses", &["book_id", "lemma", "gloss", "created_at"]),
    ("known_words", &["lemma", "source", "created_at"]),
    ("custom_words", &["lemma", "gloss", "source", "created_at"]),
    (
        "vocab_book",
        &[
            "lemma",
            "word",
            "gloss",
            "level",
            "context_sentence",
            "book_id",
            "status",
            "review_count",
            "created_at",
            "updated_at",
        ],
    ),
];

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
        Self::open_in(crate::paths::app_data_dir())
    }

    /// 在调用方解析出的应用数据目录中打开数据库。
    /// 移动端由 Makepad 传入原生沙箱路径，避免依赖 HOME/XDG 环境变量。
    pub fn open_in(data_dir: PathBuf) -> Result<Self, String> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("创建运行时失败: {}", e))?;
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

    /// Load reader text, atomically migrating legacy PDF heading coordinates
    /// before callers restore progress, bookmarks or highlights. Background only.
    pub fn load_book_chapters(
        &self,
        id: &str,
        path: &std::path::Path,
    ) -> Result<books::epub_meta::LoadedChapters, String> {
        if books::supported_format(path) == Some("pdf") {
            self.rt
                .block_on(books::pdf::load_reader_pages(&self.pool, id, path))
        } else {
            books::load_imported_chapters_with_toc(path)
        }
    }

    pub fn get_shelf_items(&self) -> Result<Vec<BookShelfItem>, String> {
        self.rt
            .block_on(books::get_shelf_items(&self.pool, &self.data_dir))
    }

    /// Background-only; emits each repaired cover without reloading the shelf.
    pub fn backfill_pdf_covers(&self, ready: impl FnMut(String, String)) -> Result<usize, String> {
        self.rt.block_on(books::backfill_pdf_covers(
            &self.pool,
            &self.data_dir,
            ready,
        ))
    }

    pub fn delete_book(&self, id: &str) -> Result<(), String> {
        self.rt
            .block_on(books::delete_book(&self.pool, &self.data_dir, id))
    }

    pub fn set_book_tags(&self, id: &str, tags: &[String]) -> Result<Vec<String>, String> {
        self.rt.block_on(books::set_book_tags(&self.pool, id, tags))
    }

    pub fn rename_book(&self, id: &str, title: &str) -> Result<String, String> {
        self.rt.block_on(books::rename_book(&self.pool, id, title))
    }

    pub fn set_book_want_to_read(&self, id: &str, want_to_read: bool) -> Result<(), String> {
        self.rt
            .block_on(books::set_book_want_to_read(&self.pool, id, want_to_read))
    }

    pub fn mark_book_completed(&self, id: &str) -> Result<(), String> {
        self.rt.block_on(books::mark_book_completed(&self.pool, id))
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

    pub fn get_bookmarks(&self, book_id: &str) -> Result<Vec<books::Bookmark>, String> {
        self.rt.block_on(books::get_bookmarks(&self.pool, book_id))
    }

    pub fn add_bookmark(&self, book_id: &str, location: &str, label: &str) -> Result<(), String> {
        self.rt
            .block_on(books::add_bookmark(&self.pool, book_id, location, label))
    }

    pub fn delete_bookmark(&self, book_id: &str, bookmark_id: &str) -> Result<(), String> {
        self.rt
            .block_on(books::delete_bookmark(&self.pool, book_id, bookmark_id))
    }

    pub fn delete_highlights(&self, book_id: &str, ids: &[String]) -> Result<usize, String> {
        self.rt
            .block_on(books::delete_highlights(&self.pool, book_id, ids))
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

    pub fn load_reader_settings(&self) -> Result<crate::settings::ReaderSettings, String> {
        self.rt.block_on(async {
            use sqlx::Row;

            let rows = sqlx::query("SELECT key, value FROM app_settings WHERE key LIKE 'reader.%'")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| format!("读取阅读设置失败: {e}"))?;
            let mut settings = crate::settings::ReaderSettings::default();
            let mut legacy_margin_pixels = None;
            let mut margin_percent = None;
            for row in rows {
                let key: String = row.get("key");
                let value: String = row.get("value");
                match key.as_str() {
                    "reader.language" => {
                        settings.language = crate::settings::AppLanguage::from_str(&value)
                    }
                    "reader.english_level" => {
                        settings.english_level = crate::settings::EnglishLevel::from_str(&value)
                    }
                    "reader.library_view_mode" => {
                        settings.library_view_mode =
                            crate::settings::LibraryViewMode::from_str(&value)
                    }
                    "reader.theme" => {
                        settings.theme = crate::settings::ReaderTheme::from_str(&value)
                    }
                    "reader.background" => {
                        settings.background = crate::settings::ReaderBackground::from_str(&value)
                    }
                    "reader.columns" => {
                        settings.columns = crate::settings::ReaderColumns::from_str(&value)
                    }
                    "reader.turn_mode" => {
                        settings.turn_mode = crate::settings::ReaderTurnMode::from_str(&value)
                    }
                    "reader.font" => settings.font = crate::settings::ReaderFont::from_str(&value),
                    "reader.bold_text" => settings.bold_text = value == "1",
                    "reader.spacing" => {
                        settings.spacing = crate::settings::ReaderSpacing::from_str(&value)
                    }
                    "reader.character_spacing" => {
                        settings.character_spacing =
                            value.parse().unwrap_or(settings.character_spacing)
                    }
                    "reader.word_spacing" => {
                        settings.word_spacing = value.parse().unwrap_or(settings.word_spacing)
                    }
                    "reader.margin" => legacy_margin_pixels = value.parse::<f32>().ok(),
                    "reader.margin_percent" => margin_percent = value.parse::<f32>().ok(),
                    "reader.justify_text" => settings.justify_text = value == "1",
                    "reader.text_width" => {
                        settings.text_width = crate::settings::ReaderTextWidth::from_str(&value)
                    }
                    "reader.indent_paragraphs" => settings.indent_paragraphs = value == "1",
                    "reader.font_size" => settings.font_size = value.parse().ok(),
                    "reader.gloss_enabled" => settings.gloss_enabled = value == "1",
                    "reader.gloss_font_size" => {
                        settings.gloss_font_size =
                            value.parse().unwrap_or(settings.gloss_font_size);
                    }
                    "reader.gloss_color_slot" => {
                        settings.gloss_color_slot =
                            value.parse().unwrap_or(settings.gloss_color_slot);
                    }
                    "reader.gloss_background_slot" => {
                        settings.gloss_background_slot =
                            value.parse().unwrap_or(settings.gloss_background_slot);
                    }
                    "reader.word_color_slot" => {
                        settings.word_color_slot =
                            value.parse().unwrap_or(settings.word_color_slot);
                    }
                    "reader.word_background_slot" => {
                        settings.word_background_slot =
                            value.parse().unwrap_or(settings.word_background_slot);
                    }
                    "reader.selection_color_slot" => {
                        settings.selection_color_slot =
                            value.parse().unwrap_or(settings.selection_color_slot);
                    }
                    "reader.highlight_color_slot" => {
                        settings.highlight_color_slot =
                            value.parse().unwrap_or(settings.highlight_color_slot);
                    }
                    _ => {}
                }
            }
            settings.margin = margin_percent
                .or_else(|| legacy_margin_pixels.map(|pixels| (pixels / 28.0 - 1.0) * 100.0))
                .unwrap_or(settings.margin);
            Ok(settings.normalized())
        })
    }

    pub fn save_reader_settings(
        &self,
        settings: &crate::settings::ReaderSettings,
    ) -> Result<(), String> {
        self.rt.block_on(async {
            let settings = settings.clone().normalized();
            let now = chrono::Utc::now().timestamp_millis();
            let language = settings.language.as_str();
            let library_view_mode = settings.library_view_mode.as_str();
            let theme = settings.theme.as_str();
            let background = settings.background.as_str();
            let columns = settings.columns.as_str();
            let turn_mode = settings.turn_mode.as_str();
            let font = settings.font.as_str();
            let bold_text = if settings.bold_text { "1" } else { "0" };
            let spacing = settings.spacing.as_str();
            let character_spacing = settings.character_spacing.to_string();
            let word_spacing = settings.word_spacing.to_string();
            let margin = settings.margin.to_string();
            let justify_text = if settings.justify_text { "1" } else { "0" };
            let font_size = settings.font_size.map(|value| value.to_string()).unwrap_or_default();
            let gloss_enabled = if settings.gloss_enabled { "1" } else { "0" };
            let gloss_font_size = settings.gloss_font_size.to_string();
            let gloss_color_slot = settings.gloss_color_slot.to_string();
            let gloss_background_slot = settings.gloss_background_slot.to_string();
            let word_color_slot = settings.word_color_slot.to_string();
            let word_background_slot = settings.word_background_slot.to_string();
            let selection_color_slot = settings.selection_color_slot.to_string();
            let highlight_color_slot = settings.highlight_color_slot.to_string();
            let values = [
                ("reader.language", language),
                ("reader.english_level", settings.english_level.as_str()),
                ("reader.library_view_mode", library_view_mode),
                ("reader.theme", theme),
                ("reader.background", background),
                ("reader.columns", columns),
                ("reader.turn_mode", turn_mode),
                ("reader.font", font),
                ("reader.bold_text", bold_text),
                ("reader.spacing", spacing.as_str()),
                ("reader.character_spacing", character_spacing.as_str()),
                ("reader.word_spacing", word_spacing.as_str()),
                ("reader.margin_percent", margin.as_str()),
                ("reader.justify_text", justify_text),
                ("reader.text_width", settings.text_width.as_str()),
                ("reader.indent_paragraphs", if settings.indent_paragraphs { "1" } else { "0" }),
                ("reader.font_size", font_size.as_str()),
                ("reader.gloss_enabled", gloss_enabled),
                ("reader.gloss_font_size", gloss_font_size.as_str()),
                ("reader.gloss_color_slot", gloss_color_slot.as_str()),
                (
                    "reader.gloss_background_slot",
                    gloss_background_slot.as_str(),
                ),
                ("reader.word_color_slot", word_color_slot.as_str()),
                (
                    "reader.word_background_slot",
                    word_background_slot.as_str(),
                ),
                (
                    "reader.selection_color_slot",
                    selection_color_slot.as_str(),
                ),
                (
                    "reader.highlight_color_slot",
                    highlight_color_slot.as_str(),
                ),
            ];
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| format!("开启阅读设置事务失败: {e}"))?;
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
                .map_err(|e| format!("保存阅读设置失败: {e}"))?;
            }
            tx.commit()
                .await
                .map_err(|e| format!("提交阅读设置失败: {e}"))
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

    pub fn get_all_notes(&self) -> Result<Vec<crate::notes::models::Note>, String> {
        self.rt.block_on(crate::notes::get_all_notes(&self.pool))
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

    /// 注音上下文：常规词库与 C3 导入词库分开；已认识词在两种模式下均排除。
    pub fn gloss_context(
        &self,
        book_id: Option<&str>,
    ) -> Result<
        (
            vocab::reader_lexicon::ReaderLexicon,
            std::collections::HashSet<String>,
        ),
        String,
    > {
        self.rt.block_on(async {
            use vocab::reader_lexicon::ReaderLexicon;
            let custom = sqlx::query("SELECT lemma, gloss FROM custom_words")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| format!("读取自定义词库失败: {e}"))?;
            let mut lexicon = ReaderLexicon::builtin();
            let mut custom_members = HashSet::new();
            for row in custom {
                let lemma: String = row.get("lemma");
                let gloss: Option<String> = row.get("gloss");
                let gloss = gloss
                    .filter(|g| !g.trim().is_empty())
                    .or_else(|| lexicon.standard.get(&lemma).cloned());
                custom_members.insert(lemma.clone());
                if let Some(gloss) = gloss {
                    lexicon.custom.insert(lemma, gloss);
                }
            }
            let rows = sqlx::query("SELECT lemma, gloss FROM word_glosses")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| format!("读取全局释义失败: {}", e))?;
            for row in rows {
                let lemma: String = row.get("lemma");
                let gloss: String = row.get("gloss");
                if gloss.trim().is_empty() {
                    continue;
                }
                // Imported-only words belong to C3, but overlap with the
                // built-in dictionary remains available at normal levels.
                if !custom_members.contains(&lemma) || ReaderLexicon::is_builtin(&lemma) {
                    lexicon.standard.insert(lemma.clone(), gloss.clone());
                }
                if custom_members.contains(&lemma) {
                    lexicon.custom.entry(lemma).or_insert(gloss);
                }
            }
            if let Some(id) = book_id {
                for g in vocab::get_book_glosses(&self.pool, id).await? {
                    if lexicon.custom.contains_key(&g.lemma) {
                        lexicon.custom.insert(g.lemma.clone(), g.gloss.clone());
                    }
                    lexicon.standard.insert(g.lemma, g.gloss);
                }
            }
            let known: std::collections::HashSet<String> = vocab::get_known_words(&self.pool)
                .await?
                .into_iter()
                .collect();
            Ok((lexicon, known))
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

    let connect_options = SqliteConnectOptions::from_str(&db_url)
        .map_err(|e| format!("解析数据库地址失败: {}", e))?
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .connect_with(connect_options)
        .await
        .map_err(|e| format!("连接数据库失败: {}", e))?;

    initialize_schema(&pool).await?;

    statistics::recover_open_reading_sessions(&pool).await?;
    initialize_starter_glosses(&pool).await?;

    initialize_default_skills(&pool).await?;

    Ok(pool)
}

async fn initialize_schema(pool: &SqlitePool) -> Result<(), String> {
    reject_newer_schema(pool).await?;
    sqlx::query(include_str!("./schema.sql"))
        .execute(pool)
        .await
        .map_err(|e| format!("初始化 schema 失败: {}", e))?;

    migrate_schema(pool).await?;
    validate_schema(pool).await
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

async fn schema_version(pool: &SqlitePool) -> Result<i64, String> {
    sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(pool)
        .await
        .map_err(|e| format!("读取数据库 schema 版本失败: {}", e))
}

async fn reject_newer_schema(pool: &SqlitePool) -> Result<(), String> {
    let version = schema_version(pool).await?;
    if version > CURRENT_SCHEMA_VERSION {
        return Err(format!(
            "数据库版本 {} 高于当前应用支持的版本 {}，请升级应用后再打开",
            version, CURRENT_SCHEMA_VERSION
        ));
    }
    Ok(())
}

async fn table_columns(pool: &SqlitePool, table: &str) -> Result<HashSet<String>, String> {
    let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await
        .map_err(|e| format!("读取 {table} 表结构失败: {e}"))?;
    Ok(rows
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect())
}

async fn ensure_column(
    pool: &SqlitePool,
    table: &str,
    column: &str,
    declaration: &str,
) -> Result<(), String> {
    if table_columns(pool, table).await?.contains(column) {
        return Ok(());
    }
    sqlx::query(&format!(
        "ALTER TABLE {table} ADD COLUMN {column} {declaration}"
    ))
    .execute(pool)
    .await
    .map_err(|e| format!("迁移 {table}.{column} 失败: {e}"))?;
    Ok(())
}

async fn migrate_schema(pool: &SqlitePool) -> Result<(), String> {
    ensure_column(pool, "book_notes", "section_id", "TEXT").await?;
    ensure_column(pool, "book_notes", "norm_start", "INTEGER").await?;
    ensure_column(pool, "book_notes", "norm_end", "INTEGER").await?;

    sqlx::query(&format!("PRAGMA user_version = {CURRENT_SCHEMA_VERSION}"))
        .execute(pool)
        .await
        .map_err(|e| format!("写入数据库 schema 版本失败: {e}"))?;
    Ok(())
}

async fn validate_schema(pool: &SqlitePool) -> Result<(), String> {
    for (table, required_columns) in REQUIRED_SCHEMA {
        let columns = table_columns(pool, table).await?;
        if columns.is_empty() {
            return Err(format!("数据库结构不完整：缺少 {table} 表"));
        }
        let missing: Vec<_> = required_columns
            .iter()
            .filter(|column| !columns.contains(**column))
            .copied()
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "数据库结构不完整：{table} 缺少列 {}",
                missing.join(", ")
            ));
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

#[cfg(test)]
mod migration_tests {
    use super::*;

    #[test]
    fn custom_import_membership_is_independent_of_difficulty_and_known_status() {
        use crate::settings::EnglishLevel;
        let (rt, pool) = runtime_and_pool();
        rt.block_on(initialize_schema(&pool)).unwrap();
        rt.block_on(async {
            sqlx::query("INSERT INTO custom_words VALUES ('forest','自定义森林','youdao',1), ('customonly','自定义词','youdao',1), ('emptyword',NULL,'youdao',1)")
                .execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO word_glosses VALUES ('customonly','旧全局释义',5,1), ('solitude','孤独',3,1)")
                .execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO known_words VALUES ('forest','marked',1)")
                .execute(&pool).await.unwrap();
        });
        let db = Db {
            rt,
            pool,
            data_dir: Default::default(),
        };
        let (lexicon, known) = db.gloss_context(None).unwrap();
        assert!(known.contains("forest"));
        assert_eq!(
            lexicon.select(EnglishLevel::C3).get("forest").unwrap(),
            "自定义森林"
        );
        assert!(lexicon.select(EnglishLevel::C3).contains_key("customonly"));
        assert!(!lexicon.select(EnglishLevel::A1).contains_key("customonly"));
        assert!(!lexicon.select(EnglishLevel::C3).contains_key("solitude"));
        let html = vocab::annotate::annotate_html(
            "<p>forest customonly solitude.</p>",
            &lexicon.select(EnglishLevel::C3),
            &known,
        );
        assert!(!html.contains("<ruby>forest"));
        assert!(html.contains("<ruby>customonly"));
        assert!(!html.contains("<ruby>solitude"));
    }

    #[test]
    fn english_levels_persist_and_missing_or_invalid_values_default_to_a1() {
        use crate::settings::{AppLanguage, EnglishLevel};
        let (rt, pool) = runtime_and_pool();
        rt.block_on(initialize_schema(&pool)).unwrap();
        let db = Db {
            rt,
            pool,
            data_dir: std::path::PathBuf::new(),
        };
        let mut settings = db.load_reader_settings().unwrap();
        assert_eq!(settings.english_level, EnglishLevel::A1);
        settings.language = AppLanguage::English;
        for level in EnglishLevel::ALL {
            settings.english_level = level;
            db.save_reader_settings(&settings).unwrap();
            assert_eq!(db.load_reader_settings().unwrap(), settings);
        }
        db.rt
            .block_on(
                sqlx::query(
                    "UPDATE app_settings SET value = 'invalid' WHERE key = 'reader.english_level'",
                )
                .execute(&db.pool),
            )
            .unwrap();
        assert_eq!(
            db.load_reader_settings().unwrap().english_level,
            EnglishLevel::A1
        );
    }

    #[test]
    fn reader_turn_modes_persist_without_changing_columns_and_default_to_curl() {
        use crate::settings::{ReaderColumns, ReaderTurnMode};
        let (rt, pool) = runtime_and_pool();
        rt.block_on(initialize_schema(&pool)).unwrap();
        let db = Db {
            rt,
            pool,
            data_dir: std::path::PathBuf::new(),
        };
        let mut settings = db.load_reader_settings().unwrap();
        assert_eq!(settings.turn_mode, ReaderTurnMode::Curl);
        settings.columns = ReaderColumns::Two;
        for mode in [
            ReaderTurnMode::Slide,
            ReaderTurnMode::Scroll,
            ReaderTurnMode::Curl,
        ] {
            settings.turn_mode = mode;
            db.save_reader_settings(&settings).unwrap();
            assert_eq!(db.load_reader_settings().unwrap(), settings);
        }
        db.rt
            .block_on(
                sqlx::query(
                    "UPDATE app_settings SET value = 'invalid' WHERE key = 'reader.turn_mode'",
                )
                .execute(&db.pool),
            )
            .unwrap();
        assert_eq!(
            db.load_reader_settings().unwrap().turn_mode,
            ReaderTurnMode::Curl
        );
    }

    #[test]
    fn reader_column_preference_persists_and_old_databases_default_to_auto() {
        let (rt, pool) = runtime_and_pool();
        rt.block_on(initialize_schema(&pool)).unwrap();
        let db = Db {
            rt,
            pool,
            data_dir: std::path::PathBuf::new(),
        };
        let mut settings = db.load_reader_settings().unwrap();
        assert_eq!(settings.columns, crate::settings::ReaderColumns::Auto);
        for columns in [
            crate::settings::ReaderColumns::One,
            crate::settings::ReaderColumns::Two,
            crate::settings::ReaderColumns::Auto,
        ] {
            settings.columns = columns;
            db.save_reader_settings(&settings).unwrap();
            assert_eq!(db.load_reader_settings().unwrap().columns, columns);
        }
    }

    #[test]
    fn old_and_new_pen_and_highlighter_slots_persist_without_recolouring_notes() {
        let (rt, pool) = runtime_and_pool();
        rt.block_on(initialize_schema(&pool)).unwrap();
        let db = Db {
            rt,
            pool,
            data_dir: std::path::PathBuf::new(),
        };
        let mut settings = db.load_reader_settings().unwrap();
        assert_eq!(settings.highlight_color_slot, 4);
        assert_eq!(settings.selection_color_slot, 0);
        // No settings migration may rewrite an already saved annotation.
        db.rt.block_on(sqlx::query(
            "INSERT INTO books (id, title, author, format, file_path, file_size, language, created_at, updated_at) VALUES ('book', 'Test', '', 'epub', '', 0, 'en', 0, 0)"
        ).execute(&db.pool)).unwrap();
        db.rt.block_on(sqlx::query(
            "INSERT INTO book_notes (id, book_id, type, cfi, style, color, section_id, norm_start, norm_end, note, created_at, updated_at) VALUES ('legacy', 'book', 'annotation', '', 'highlight', 'orange', '0', 0, 3, '', 0, 0)"
        ).execute(&db.pool)).unwrap();
        for slot in 0..8 {
            settings.selection_color_slot = slot;
            settings.highlight_color_slot = slot;
            db.save_reader_settings(&settings).unwrap();
            assert_eq!(db.load_reader_settings().unwrap(), settings);
            let color: String = db
                .rt
                .block_on(
                    sqlx::query_scalar("SELECT color FROM book_notes WHERE id = 'legacy'")
                        .fetch_one(&db.pool),
                )
                .unwrap();
            assert_eq!(color, "orange");
        }
    }

    #[test]
    fn reader_customisation_survives_database_round_trip() {
        let (rt, pool) = runtime_and_pool();
        rt.block_on(initialize_schema(&pool)).unwrap();
        let db = Db {
            rt,
            pool,
            data_dir: std::path::PathBuf::new(),
        };
        let mut settings = db.load_reader_settings().unwrap();
        settings.bold_text = true;
        settings.spacing = crate::settings::ReaderSpacing::Custom(1.7);
        settings.character_spacing = -0.06;
        settings.word_spacing = -0.15;
        settings.margin = 7.0;
        settings.justify_text = true;
        settings.text_width = crate::settings::ReaderTextWidth::Wide;
        settings.indent_paragraphs = true;
        for background in [
            crate::settings::ReaderBackground::Quiet,
            crate::settings::ReaderBackground::Sepia,
            crate::settings::ReaderBackground::Snow,
        ] {
            settings.background = background;
            db.save_reader_settings(&settings).unwrap();
            assert_eq!(db.load_reader_settings().unwrap(), settings);
        }
    }

    #[test]
    fn legacy_reader_layout_values_are_preserved_when_loaded() {
        let (rt, pool) = runtime_and_pool();
        rt.block_on(initialize_schema(&pool)).unwrap();
        rt.block_on(async {
            sqlx::query("INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?)")
                .bind("reader.margin")
                .bind("28")
                .bind(0_i64)
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?)")
                .bind("reader.spacing")
                .bind("comfortable")
                .bind(0_i64)
                .execute(&pool)
                .await
                .unwrap();
        });
        let db = Db {
            rt,
            pool,
            data_dir: std::path::PathBuf::new(),
        };
        let settings = db.load_reader_settings().unwrap();
        assert_eq!(settings.margin, 0.0);
        assert_eq!(settings.text_width, crate::settings::ReaderTextWidth::Even);
        assert!(!settings.indent_paragraphs);
        assert_eq!(settings.spacing.plain_line_spacing(), 1.45);
        let mut settings = settings;
        settings.margin = -5.0;
        db.save_reader_settings(&settings).unwrap();
        assert_eq!(db.load_reader_settings().unwrap().margin, -5.0);
    }

    fn runtime_and_pool() -> (tokio::runtime::Runtime, SqlitePool) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let pool = runtime
            .block_on(
                SqlitePoolOptions::new()
                    .max_connections(1)
                    .connect("sqlite::memory:"),
            )
            .unwrap();
        (runtime, pool)
    }

    #[test]
    fn legacy_schema_is_upgraded_idempotently_without_losing_rows() {
        let (runtime, pool) = runtime_and_pool();
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
                        note TEXT NOT NULL,
                        context_before TEXT,
                        context_after TEXT,
                        created_at INTEGER NOT NULL,
                        updated_at INTEGER NOT NULL
                    );
                    INSERT INTO book_notes (
                        id, book_id, type, cfi, note, created_at, updated_at
                    ) VALUES ('legacy-note', 'legacy-book', 'annotation', '/6/2', 'kept', 1, 1);",
                )
                .execute(&pool),
            )
            .unwrap();

        runtime.block_on(initialize_schema(&pool)).unwrap();
        runtime.block_on(initialize_schema(&pool)).unwrap();

        let columns = runtime
            .block_on(table_columns(&pool, "book_notes"))
            .unwrap();
        assert!(columns.contains("section_id"));
        assert!(columns.contains("norm_start"));
        assert!(columns.contains("norm_end"));
        assert_eq!(
            runtime
                .block_on(
                    sqlx::query_scalar::<_, String>(
                        "SELECT note FROM book_notes WHERE id = 'legacy-note'"
                    )
                    .fetch_one(&pool)
                )
                .unwrap(),
            "kept"
        );
        assert_eq!(
            runtime.block_on(schema_version(&pool)).unwrap(),
            CURRENT_SCHEMA_VERSION
        );
    }

    #[test]
    fn newer_database_is_rejected_before_schema_changes() {
        let (runtime, pool) = runtime_and_pool();
        runtime
            .block_on(sqlx::query("PRAGMA user_version = 999").execute(&pool))
            .unwrap();

        let error = runtime.block_on(initialize_schema(&pool)).unwrap_err();
        assert!(error.contains("999"));
        assert!(runtime
            .block_on(table_columns(&pool, "books"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn book_tags_are_normalized_saved_and_returned_on_the_shelf() {
        let (runtime, pool) = runtime_and_pool();
        runtime.block_on(initialize_schema(&pool)).unwrap();
        runtime
            .block_on(
                sqlx::query(
                    "INSERT INTO books (
                        id, title, author, format, file_path, file_size, language,
                        created_at, updated_at
                    ) VALUES ('book-1', 'Book', 'Author', 'epub', 'book.epub', 1, 'en', 1, 1)",
                )
                .execute(&pool),
            )
            .unwrap();

        let saved = runtime
            .block_on(books::set_book_tags(
                &pool,
                "book-1",
                &[
                    " Fiction ".to_string(),
                    "fiction".to_string(),
                    "Reference".to_string(),
                ],
            ))
            .unwrap();
        assert_eq!(saved, vec!["Fiction", "Reference"]);

        let shelf = runtime
            .block_on(books::get_shelf_items(&pool, std::path::Path::new("/data")))
            .unwrap();
        assert_eq!(shelf.len(), 1);
        assert_eq!(shelf[0].tags, saved);
        assert_eq!(
            runtime
                .block_on(
                    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM tags").fetch_one(&pool)
                )
                .unwrap(),
            2
        );

        let error = runtime
            .block_on(books::set_book_tags(
                &pool,
                "missing",
                &["Fiction".to_string()],
            ))
            .unwrap_err();
        assert!(error.contains("不存在"));
    }
}
