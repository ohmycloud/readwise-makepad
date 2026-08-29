use crate::ruby_word::RubyWordAction;
use crate::sample_chapter;
use makepad_ai::{
    AiBackend, AiEvent, AiRequest, BackendConfig, Message, OpenAiBackend, RequestId, StreamDelta,
};
use makepad_widgets::makepad_platform::event::{TouchPoint, TouchState};
use makepad_widgets::*;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock, RwLock};
use std::time::Instant;

type AiSettings = readwise_core::ai::AiSettings;
type AppLanguage = readwise_core::settings::AppLanguage;
type LocalAiSettings = readwise_core::local_ai::LocalAiSettings;
type LocalAiSession = readwise_core::local_ai::LocalAiSession;
type ReaderFont = readwise_core::settings::ReaderFont;
type ReaderSpacing = readwise_core::settings::ReaderSpacing;
type ReaderSettings = readwise_core::settings::ReaderSettings;
type ReaderTheme = readwise_core::settings::ReaderTheme;
type StatisticsSnapshot = readwise_core::statistics::models::StatisticsSnapshot;
type VocabEntry = readwise_core::vocab::models::VocabBookEntry;

app_main!(App);

const PARA_COUNT: usize = 200;
const COMPACT_LAYOUT_BREAKPOINT: f64 = 720.0;
const READER_SWIPE_MIN_DISTANCE: f64 = 64.0;
const READER_SWIPE_MAX_DURATION: f64 = 0.9;
const READER_SWIPE_AXIS_RATIO: f64 = 1.35;
const READER_SWIPE_VERTICAL_CANCEL: f64 = 28.0;

fn uses_compact_layout(size: DVec2) -> bool {
    size.x > 0.0 && size.y > 0.0 && size.x < COMPACT_LAYOUT_BREAKPOINT
}

fn should_show_mobile_nav(compact_layout: bool, virtual_keyboard_open: bool) -> bool {
    compact_layout && !virtual_keyboard_open
}

fn word_range_at(text: &str, char_index: usize) -> Option<(usize, usize)> {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return None;
    }
    let index = char_index.min(chars.len() - 1);
    let selected = chars[index];
    if selected.is_whitespace() {
        return None;
    }
    let is_cjk = |ch: char| {
        matches!(
            ch as u32,
            0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff
        )
    };
    if is_cjk(selected) {
        return Some((index, index + 1));
    }
    let is_word_char = |ch: char| ch.is_alphanumeric() || matches!(ch, '\'' | '’' | '-');
    if !is_word_char(selected) {
        return Some((index, index + 1));
    }
    let mut start = index;
    while start > 0 && is_word_char(chars[start - 1]) {
        start -= 1;
    }
    let mut end = index + 1;
    while end < chars.len() && is_word_char(chars[end]) {
        end += 1;
    }
    Some((start, end))
}

fn should_keep_started_reading_session(
    invalidated: bool,
    app_in_foreground: bool,
    on_reader_page: bool,
    is_current_book: bool,
) -> bool {
    !invalidated && app_in_foreground && on_reader_page && is_current_book
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReaderSwipeDirection {
    Previous,
    Next,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum AppPage {
    Library,
    #[default]
    Reader,
    Chat,
    Skills,
    Vocab,
    Statistics,
    Settings,
}

impl AppPage {
    fn live_id(self) -> LiveId {
        match self {
            Self::Library => id!(page_library),
            Self::Reader => id!(page_reader),
            Self::Chat => id!(page_chat),
            Self::Skills => id!(page_skills),
            Self::Vocab => id!(page_vocab),
            Self::Statistics => id!(page_statistics),
            Self::Settings => id!(page_settings),
        }
    }
}

fn reader_swipe_direction(start: DVec2, end: DVec2, elapsed: f64) -> Option<ReaderSwipeDirection> {
    let delta = end - start;
    if !(0.0..=READER_SWIPE_MAX_DURATION).contains(&elapsed)
        || delta.x.abs() < READER_SWIPE_MIN_DISTANCE
        || delta.x.abs() < delta.y.abs() * READER_SWIPE_AXIS_RATIO
    {
        return None;
    }
    Some(if delta.x < 0.0 {
        ReaderSwipeDirection::Next
    } else {
        ReaderSwipeDirection::Previous
    })
}

#[derive(Default)]
struct ReaderSwipeGesture {
    origin: Option<(u64, DVec2, f64)>,
    cancelled: bool,
}

impl ReaderSwipeGesture {
    fn begin(&mut self, uid: u64, abs: DVec2, time: f64) {
        self.origin = Some((uid, abs, time));
        self.cancelled = false;
    }

    fn cancel(&mut self) {
        self.origin = None;
        self.cancelled = true;
    }

    fn update(&mut self, touch: &TouchPoint, event_time: f64) -> Option<ReaderSwipeDirection> {
        let Some((uid, start, start_time)) = self.origin else {
            return None;
        };
        if touch.uid != uid {
            self.cancel();
            return None;
        }
        let delta = touch.abs - start;
        if touch.state == TouchState::Move
            && delta.y.abs() > READER_SWIPE_VERTICAL_CANCEL
            && delta.y.abs() > delta.x.abs()
        {
            self.cancelled = true;
        }
        if touch.state != TouchState::Stop {
            return None;
        }
        self.origin = None;
        if self.cancelled {
            return None;
        }
        reader_swipe_direction(start, touch.abs, event_time - start_time)
    }
}

static PARAGRAPHS: LazyLock<RwLock<Vec<String>>> =
    LazyLock::new(|| RwLock::new(sample_chapter::build_paragraphs(PARA_COUNT)));

/// 全局数据库句柄：后台线程初始化后写入，各处只读使用。
static DB: OnceLock<readwise_core::Db> = OnceLock::new();
static APP_STARTED_AT: OnceLock<Instant> = OnceLock::new();
static INITIAL_LIBRARY_READY_LOGGED: AtomicBool = AtomicBool::new(false);

fn startup_elapsed_ms() -> Option<u128> {
    APP_STARTED_AT
        .get()
        .map(Instant::elapsed)
        .map(|d| d.as_millis())
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
static LOCAL_AI_MANAGER: LazyLock<std::sync::Mutex<readwise_core::local_ai::LocalAiManager>> =
    LazyLock::new(|| std::sync::Mutex::new(readwise_core::local_ai::LocalAiManager::default()));
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
static LOCAL_AI_CANCEL: AtomicBool = AtomicBool::new(false);

/// 书架数据（UI 侧只读渲染，后台线程写）。
static SHELF: LazyLock<RwLock<Vec<ShelfItem>>> = LazyLock::new(|| RwLock::new(Vec::new()));

/// 书架筛选后的 PortalList 投影。筛选变化时通过 revision 切换等价模板，
/// 避免较短结果集复用上一筛选留下的卡片。
static SHELF_VISIBLE: LazyLock<RwLock<ListProjection<ShelfItem>>> =
    LazyLock::new(|| RwLock::new(ListProjection::default()));

/// PortalList 数据投影。revision 变化时列表切换等价模板，确保 Makepad 不会把
/// 相同 item id 的旧卡片继续留在新的数据集合里。
struct ListProjection<T> {
    revision: usize,
    entries: Vec<T>,
}

impl<T> Default for ListProjection<T> {
    fn default() -> Self {
        Self {
            revision: 0,
            entries: Vec::new(),
        }
    }
}

impl<T> ListProjection<T> {
    fn replace(&mut self, entries: Vec<T>) {
        self.revision = self.revision.wrapping_add(1).max(1);
        self.entries = entries;
    }

    fn entry_for_item_id(&self, item_id: usize) -> Option<&T> {
        self.entries.get(item_id)
    }
}

/// 当前书的笔记（后台线程刷新，NoteList 只读渲染）。
static NOTES: LazyLock<RwLock<ListProjection<readwise_core::notes::models::Note>>> =
    LazyLock::new(|| RwLock::new(ListProjection::default()));

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChatRole {
    User,
    Assistant,
}

#[derive(Clone)]
struct ChatMessage {
    role: ChatRole,
    text: String,
}

#[derive(Default)]
struct ChatData {
    messages: Vec<ChatMessage>,
    streaming_text: String,
    is_streaming: bool,
}

static CHAT_DATA: LazyLock<RwLock<ChatData>> = LazyLock::new(|| RwLock::new(ChatData::default()));

/// AI 技能 / Prompt 模板（后台刷新，SkillList 与发送请求只读）。
static SKILLS: LazyLock<RwLock<ListProjection<readwise_core::skills::models::Skill>>> =
    LazyLock::new(|| RwLock::new(ListProjection::default()));

/// 生词本当前筛选结果。与笔记、技能共享同一种有版本的列表投影。
static VOCAB_VISIBLE: LazyLock<RwLock<ListProjection<VocabEntry>>> =
    LazyLock::new(|| RwLock::new(ListProjection::default()));

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum VocabFilter {
    #[default]
    All,
    Learning,
    Mastered,
}

impl VocabFilter {
    fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Learning,
            2 => Self::Mastered,
            _ => Self::All,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => tr("all"),
            Self::Learning => tr("learning"),
            Self::Mastered => tr("mastered"),
        }
    }

    fn accepts(self, entry: &VocabEntry) -> bool {
        match self {
            Self::All => true,
            Self::Learning => entry.status == "learning",
            Self::Mastered => entry.status == "mastered",
        }
    }
}

#[derive(Debug)]
enum VocabAction {
    Loaded(Vec<VocabEntry>),
    Updated {
        entries: Vec<VocabEntry>,
        message: String,
        reviewed_lemma: Option<String>,
    },
    Removed {
        entries: Vec<VocabEntry>,
        message: String,
    },
    Error(String),
}

#[derive(Debug)]
enum StatisticsAction {
    Loaded(StatisticsSnapshot),
    Error(String),
}

#[derive(Debug)]
enum ReadingSessionAction {
    Started { id: String, book_id: String },
    Error(String),
}

fn spawn_load_statistics() {
    std::thread::spawn(|| {
        let Some(db) = DB.get() else { return };
        match db.statistics_snapshot() {
            Ok(snapshot) => Cx::post_action(StatisticsAction::Loaded(snapshot)),
            Err(error) => Cx::post_action(StatisticsAction::Error(error)),
        }
    });
}

fn spawn_start_reading_session(book_id: String) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        match db.start_reading_session(&book_id) {
            Ok(id) => Cx::post_action(ReadingSessionAction::Started { id, book_id }),
            Err(error) => Cx::post_action(ReadingSessionAction::Error(error)),
        }
    });
}

fn spawn_checkpoint_reading_session(id: String) {
    std::thread::spawn(move || {
        if let Some(db) = DB.get() {
            if let Err(error) = db.checkpoint_reading_session(&id) {
                ::log::error!("{error}");
            }
        }
    });
}

fn spawn_finish_reading_session(id: String, load_statistics: bool) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        if let Err(error) = db.finish_reading_session(&id) {
            Cx::post_action(ReadingSessionAction::Error(error));
            return;
        }
        if load_statistics {
            match db.statistics_snapshot() {
                Ok(snapshot) => Cx::post_action(StatisticsAction::Loaded(snapshot)),
                Err(error) => Cx::post_action(StatisticsAction::Error(error)),
            }
        }
    });
}

fn spawn_load_vocab() {
    std::thread::spawn(|| {
        let Some(db) = DB.get() else { return };
        match db.vocab_book_list() {
            Ok(entries) => Cx::post_action(VocabAction::Loaded(entries)),
            Err(error) => Cx::post_action(VocabAction::Error(error)),
        }
    });
}

fn spawn_set_vocab_status(lemma: String, status: &'static str) {
    let english = app_language() == AppLanguage::English;
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        if let Err(error) = db.vocab_book_set_status(&lemma, status) {
            Cx::post_action(VocabAction::Error(error));
            return;
        }
        match db.vocab_book_list() {
            Ok(entries) => Cx::post_action(VocabAction::Updated {
                entries,
                message: if status == "mastered" {
                    if english {
                        format!("{lemma} marked as mastered")
                    } else {
                        format!("{lemma} 已标为掌握")
                    }
                } else if english {
                    format!("{lemma} moved back to Learning")
                } else {
                    format!("{lemma} 已回到学习中")
                },
                reviewed_lemma: None,
            }),
            Err(error) => Cx::post_action(VocabAction::Error(error)),
        }
    });
}

fn spawn_review_vocab(lemma: String, status: &'static str) {
    let english = app_language() == AppLanguage::English;
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        if let Err(error) = db.vocab_book_review_with_status(&lemma, status) {
            Cx::post_action(VocabAction::Error(error));
            return;
        }
        match db.vocab_book_list() {
            Ok(entries) => Cx::post_action(VocabAction::Updated {
                entries,
                message: if status == "mastered" {
                    if english {
                        format!("{lemma}: Got it ✓")
                    } else {
                        format!("{lemma}：这次认识 ✓")
                    }
                } else if english {
                    format!("{lemma}: Keep learning")
                } else {
                    format!("{lemma}：继续学习")
                },
                reviewed_lemma: Some(lemma),
            }),
            Err(error) => Cx::post_action(VocabAction::Error(error)),
        }
    });
}

fn spawn_remove_vocab(lemma: String) {
    let english = app_language() == AppLanguage::English;
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        if let Err(error) = db.vocab_book_remove(&lemma) {
            Cx::post_action(VocabAction::Error(error));
            return;
        }
        match db.vocab_book_list() {
            Ok(entries) => Cx::post_action(VocabAction::Removed {
                entries,
                message: if english {
                    format!("Removed {lemma} from Vocabulary")
                } else {
                    format!("已从生词本移除 {lemma}")
                },
            }),
            Err(error) => Cx::post_action(VocabAction::Error(error)),
        }
    });
}

#[derive(Debug)]
enum SkillAction {
    Loaded(Vec<readwise_core::skills::models::Skill>),
    Saved(Vec<readwise_core::skills::models::Skill>),
    Toggled(Vec<readwise_core::skills::models::Skill>),
    Deleted {
        skills: Vec<readwise_core::skills::models::Skill>,
        id: String,
    },
    Error(String),
}

fn spawn_load_skills() {
    std::thread::spawn(|| {
        let Some(db) = DB.get() else { return };
        match db.list_skills() {
            Ok(skills) => Cx::post_action(SkillAction::Loaded(skills)),
            Err(error) => Cx::post_action(SkillAction::Error(error)),
        }
    });
}

fn spawn_save_skill(id: Option<String>, name: String, content: String, is_active: bool) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        let result = match id {
            Some(id) => db.update_skill(&id, &name, &content, is_active).map(|_| ()),
            None => db.create_skill(&name, &content, is_active).map(|_| ()),
        };
        if let Err(error) = result {
            Cx::post_action(SkillAction::Error(error));
            return;
        }
        match db.list_skills() {
            Ok(skills) => Cx::post_action(SkillAction::Saved(skills)),
            Err(error) => Cx::post_action(SkillAction::Error(error)),
        }
    });
}

fn spawn_toggle_skill(id: String, is_active: bool) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        if let Err(error) = db.set_skill_active(&id, is_active) {
            Cx::post_action(SkillAction::Error(error));
            return;
        }
        match db.list_skills() {
            Ok(skills) => Cx::post_action(SkillAction::Toggled(skills)),
            Err(error) => Cx::post_action(SkillAction::Error(error)),
        }
    });
}

fn spawn_delete_skill(id: String) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        if let Err(error) = db.delete_skill(&id) {
            Cx::post_action(SkillAction::Error(error));
            return;
        }
        match db.list_skills() {
            Ok(skills) => Cx::post_action(SkillAction::Deleted { skills, id }),
            Err(error) => Cx::post_action(SkillAction::Error(error)),
        }
    });
}

fn active_system_prompt() -> String {
    let prompts = SKILLS
        .read()
        .unwrap()
        .entries
        .iter()
        .filter(|skill| skill.is_system && skill.is_active)
        .map(|skill| skill.content.trim())
        .filter(|content| !content.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if prompts.is_empty() {
        "You are a concise reading assistant. Explain English text clearly, and answer in the user's language."
            .to_string()
    } else {
        prompts
    }
}

enum AiWireOutcome {
    NonStreamComplete(String),
    Failure(String),
}

#[derive(Debug, PartialEq, Eq)]
enum AiSseUpdate {
    ReasoningStarted,
    Text(String),
    Done,
    Failure(String),
}

/// `HttpStreamChunk` 是任意网络分片，不保证恰好落在一个 SSE/UTF-8 边界上。
/// makepad_ai 当前逐 chunk 解析会丢掉被拆开的 JSON，因此在 App 层保留增量缓冲。
#[derive(Default)]
struct AiSseDecoder {
    pending: Vec<u8>,
    saw_reasoning: bool,
    saw_content: bool,
}

impl AiSseDecoder {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn push(&mut self, bytes: &[u8]) -> Vec<AiSseUpdate> {
        self.pending.extend_from_slice(bytes);
        let mut updates = Vec::new();
        while let Some((frame_end, delimiter_len)) = sse_frame_boundary(&self.pending) {
            let mut consumed: Vec<u8> = self.pending.drain(..frame_end + delimiter_len).collect();
            consumed.truncate(frame_end);
            let Ok(frame) = String::from_utf8(consumed) else {
                updates.push(AiSseUpdate::Failure(
                    "AI 返回了无效的 UTF-8 流数据".to_string(),
                ));
                continue;
            };
            let payload = frame
                .lines()
                .filter_map(|line| line.trim_end_matches('\r').strip_prefix("data:"))
                .map(|data| data.strip_prefix(' ').unwrap_or(data))
                .collect::<Vec<_>>()
                .join("\n");
            if payload.is_empty() {
                continue;
            }
            if payload == "[DONE]" {
                updates.push(AiSseUpdate::Done);
                continue;
            }
            let json: serde_json::Value = match serde_json::from_str(&payload) {
                Ok(json) => json,
                Err(error) => {
                    updates.push(AiSseUpdate::Failure(format!(
                        "无法解析 AI 流式响应：{error}"
                    )));
                    continue;
                }
            };
            updates.extend(self.decode_json(&json));
        }
        updates
    }

    fn finish(&mut self) -> Vec<AiSseUpdate> {
        if self.pending.is_empty() {
            return Vec::new();
        }
        if let Ok(text) = std::str::from_utf8(&self.pending) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(text.trim()) {
                self.pending.clear();
                return self.decode_json(&json);
            }
        }
        self.pending.extend_from_slice(b"\n\n");
        self.push(&[])
    }

    fn decode_json(&mut self, json: &serde_json::Value) -> Vec<AiSseUpdate> {
        if let Some(error) = json
            .pointer("/error/message")
            .and_then(|error| error.as_str())
            .or_else(|| json.get("error").and_then(|error| error.as_str()))
        {
            return vec![AiSseUpdate::Failure(error.to_string())];
        }
        let mut updates = Vec::new();
        let reasoning = json
            .pointer("/choices/0/delta/reasoning_content")
            .or_else(|| json.pointer("/choices/0/message/reasoning_content"))
            .and_then(|value| value.as_str())
            .filter(|text| !text.is_empty());
        if reasoning.is_some() && !self.saw_reasoning {
            self.saw_reasoning = true;
            updates.push(AiSseUpdate::ReasoningStarted);
        }
        if let Some(text) = json
            .pointer("/choices/0/delta/content")
            .or_else(|| json.pointer("/choices/0/message/content"))
            .and_then(|value| value.as_str())
            .filter(|text| !text.is_empty())
        {
            self.saw_content = true;
            updates.push(AiSseUpdate::Text(text.to_string()));
        }
        updates
    }
}

fn sse_frame_boundary(bytes: &[u8]) -> Option<(usize, usize)> {
    let lf = bytes.windows(2).position(|window| window == b"\n\n");
    let crlf = bytes.windows(4).position(|window| window == b"\r\n\r\n");
    match (lf, crlf) {
        (Some(lf), Some(crlf)) if lf <= crlf => Some((lf, 2)),
        (Some(_), Some(crlf)) => Some((crlf, 4)),
        (Some(lf), None) => Some((lf, 2)),
        (None, Some(crlf)) => Some((crlf, 4)),
        (None, None) => None,
    }
}

fn response_error_text(status: u16, body: Option<String>) -> String {
    let detail = body
        .as_deref()
        .and_then(|body| serde_json::from_str::<serde_json::Value>(body).ok())
        .and_then(|json| {
            json.pointer("/error/message")
                .or_else(|| json.get("message"))
                .or_else(|| json.get("detail"))
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
        .or_else(|| {
            body.map(|body| {
                let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
                compact.chars().take(240).collect()
            })
        })
        .filter(|detail| !detail.is_empty())
        .unwrap_or_else(|| "服务端未返回错误详情".to_string());
    format!("HTTP {status}：{detail}")
}

fn response_message_content(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|json| {
            json.pointer("/choices/0/message/content")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
}

fn inspect_ai_network_event(event: &Event, request: RequestId) -> Option<AiWireOutcome> {
    let Event::NetworkResponses(responses) = event else {
        return None;
    };
    for response in responses {
        match response {
            NetworkResponse::HttpResponse {
                request_id,
                response,
            } if *request_id == request.0 => {
                let body = response.get_string_body();
                if !(200..300).contains(&response.status_code) {
                    return Some(AiWireOutcome::Failure(response_error_text(
                        response.status_code,
                        body,
                    )));
                }
                let content = body.as_deref().and_then(response_message_content);
                return Some(match content {
                    Some(content) => AiWireOutcome::NonStreamComplete(content),
                    None => AiWireOutcome::Failure(
                        "服务端返回成功，但响应既不是 SSE，也没有 choices[0].message.content"
                            .to_string(),
                    ),
                });
            }
            NetworkResponse::HttpStreamChunk {
                request_id,
                response,
            }
            | NetworkResponse::HttpStreamComplete {
                request_id,
                response,
            } if *request_id == request.0
                && response.status_code != 0
                && !(200..300).contains(&response.status_code) =>
            {
                return Some(AiWireOutcome::Failure(response_error_text(
                    response.status_code,
                    response.get_string_body(),
                )));
            }
            NetworkResponse::HttpError { request_id, error } if *request_id == request.0 => {
                return Some(AiWireOutcome::Failure(error.message.clone()));
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod ai_response_tests {
    use super::*;

    #[test]
    fn extracts_non_stream_reply_and_error_message() {
        assert_eq!(
            response_message_content(r#"{"choices":[{"message":{"content":"hello"}}]}"#),
            Some("hello".to_string())
        );
        assert_eq!(
            response_error_text(
                401,
                Some(r#"{"error":{"message":"invalid key"}}"#.to_string())
            ),
            "HTTP 401：invalid key"
        );
    }

    #[test]
    fn decodes_sse_across_network_and_utf8_boundaries() {
        let stream = concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking\"}}]}\r\n\r\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\n\n",
            "data: [DONE]\n\n"
        )
        .as_bytes();
        let split = stream
            .windows("你好".len())
            .position(|window| window == "你好".as_bytes())
            .unwrap()
            + 1;
        let mut decoder = AiSseDecoder::default();
        let mut updates = decoder.push(&stream[..split]);
        updates.extend(decoder.push(&stream[split..]));
        assert!(updates.contains(&AiSseUpdate::ReasoningStarted));
        assert!(updates.contains(&AiSseUpdate::Text("你好".to_string())));
        assert!(updates.contains(&AiSseUpdate::Done));
        assert!(decoder.saw_content);
    }

    #[test]
    fn surfaces_plain_json_errors_from_a_streaming_request() {
        let mut decoder = AiSseDecoder::default();
        assert!(decoder
            .push(br#"{"error":{"message":"invalid model"}}"#)
            .is_empty());
        assert_eq!(
            decoder.finish(),
            vec![AiSseUpdate::Failure("invalid model".to_string())]
        );
    }
}

#[cfg(test)]
mod responsive_layout_tests {
    use super::*;

    #[test]
    fn compact_layout_uses_phone_width_and_ignores_unknown_geometry() {
        assert!(uses_compact_layout(dvec2(390.0, 844.0)));
        assert!(uses_compact_layout(dvec2(719.0, 900.0)));
        assert!(!uses_compact_layout(dvec2(720.0, 900.0)));
        assert!(!uses_compact_layout(dvec2(1080.0, 720.0)));
        assert!(!uses_compact_layout(dvec2(0.0, 0.0)));
    }

    #[test]
    fn mobile_navigation_stays_hidden_while_the_virtual_keyboard_is_open() {
        assert!(should_show_mobile_nav(true, false));
        assert!(!should_show_mobile_nav(true, true));
        assert!(!should_show_mobile_nav(false, false));
        assert!(!should_show_mobile_nav(false, true));
    }

    #[test]
    fn long_press_selects_a_word_or_one_cjk_character() {
        assert_eq!(word_range_at("read wise", 2), Some((0, 4)));
        assert_eq!(word_range_at("can't stop", 3), Some((0, 5)));
        assert_eq!(word_range_at("阅读", 1), Some((1, 2)));
        assert_eq!(word_range_at("read wise", 4), None);
    }

    #[test]
    fn invalidated_or_background_session_is_finished_instead_of_kept() {
        assert!(should_keep_started_reading_session(false, true, true, true));
        assert!(!should_keep_started_reading_session(true, true, true, true));
        assert!(!should_keep_started_reading_session(
            false, false, true, true
        ));
        assert!(!should_keep_started_reading_session(
            false, true, false, true
        ));
        assert!(!should_keep_started_reading_session(
            false, true, true, false
        ));
    }

    #[test]
    fn horizontal_reader_swipes_have_direction_and_reject_scrolls() {
        assert_eq!(
            reader_swipe_direction(dvec2(300.0, 400.0), dvec2(210.0, 410.0), 0.4),
            Some(ReaderSwipeDirection::Next)
        );
        assert_eq!(
            reader_swipe_direction(dvec2(100.0, 400.0), dvec2(180.0, 390.0), 0.4),
            Some(ReaderSwipeDirection::Previous)
        );
        assert_eq!(
            reader_swipe_direction(dvec2(300.0, 400.0), dvec2(260.0, 300.0), 0.4),
            None
        );
        assert_eq!(
            reader_swipe_direction(dvec2(300.0, 400.0), dvec2(220.0, 400.0), 1.2),
            None
        );
    }
}

#[cfg(test)]
mod list_projection_tests {
    use super::*;

    #[test]
    fn empty_projection_has_no_renderable_item() {
        let projection = ListProjection::<&str>::default();

        assert!(projection.entries.is_empty());
        assert_eq!(projection.entry_for_item_id(0), None);
    }

    #[test]
    fn replacing_and_shrinking_invalidates_stale_item_ids() {
        let mut projection = ListProjection::default();
        projection.replace(vec!["first", "second", "third"]);
        let first_revision = projection.revision;

        projection.replace(vec!["replacement"]);

        assert!(projection.revision > first_revision);
        assert_eq!(projection.entry_for_item_id(0), Some(&"replacement"));
        assert_eq!(projection.entry_for_item_id(1), None);
        assert_eq!(projection.entry_for_item_id(2), None);
    }

    #[test]
    fn replacing_with_empty_never_creates_a_placeholder_entry() {
        let mut projection = ListProjection::default();
        projection.replace(vec!["note"]);
        let populated_revision = projection.revision;

        projection.replace(Vec::new());

        assert!(projection.revision > populated_revision);
        assert!(projection.entries.is_empty());
        assert_eq!(projection.entry_for_item_id(0), None);
    }
}

#[cfg(test)]
mod vocab_filter_tests {
    use super::*;

    fn entry(status: &str) -> VocabEntry {
        VocabEntry {
            lemma: "example".to_string(),
            word: "example".to_string(),
            gloss: None,
            level: None,
            context_sentence: None,
            book_id: None,
            status: status.to_string(),
            review_count: 0,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn radio_index_selects_the_matching_vocab_status() {
        let learning = entry("learning");
        let mastered = entry("mastered");

        assert_eq!(VocabFilter::from_index(0), VocabFilter::All);
        assert_eq!(VocabFilter::from_index(1), VocabFilter::Learning);
        assert_eq!(VocabFilter::from_index(2), VocabFilter::Mastered);
        assert!(VocabFilter::Learning.accepts(&learning));
        assert!(!VocabFilter::Learning.accepts(&mastered));
        assert!(VocabFilter::Mastered.accepts(&mastered));
        assert!(!VocabFilter::Mastered.accepts(&learning));
    }

    #[test]
    fn each_filter_projection_advances_revision_and_replaces_entries() {
        let mut projection = ListProjection::default();
        projection.replace(vec![
            entry("learning"),
            entry("learning"),
            entry("learning"),
        ]);
        let learning_revision = projection.revision;
        assert_eq!(projection.entries.len(), 3);

        projection.replace(vec![entry("mastered"), entry("mastered")]);
        assert!(projection.revision > learning_revision);
        assert_eq!(projection.entries.len(), 2);
        assert_eq!(
            projection
                .entry_for_item_id(0)
                .map(|entry| entry.status.as_str()),
            Some("mastered")
        );
    }
}

#[derive(Clone)]
struct ShelfItem {
    id: String,
    title: String,
    author: String,
    status: String,
    progress_current: i64,
    progress_total: i64,
    tags: Vec<String>,
    cover_path: Option<String>,
    file_path: String,
}

fn shelf_items_for_tag(items: &[ShelfItem], selected: Option<&str>) -> Vec<ShelfItem> {
    items
        .iter()
        .filter(|book| {
            selected.is_none_or(|selected| {
                book.tags
                    .iter()
                    .any(|tag| tag.eq_ignore_ascii_case(selected))
            })
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod shelf_filter_tests {
    use super::*;

    fn book(id: &str, tags: &[&str]) -> ShelfItem {
        ShelfItem {
            id: id.to_string(),
            title: id.to_string(),
            author: String::new(),
            status: "unread".to_string(),
            progress_current: 0,
            progress_total: 0,
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            cover_path: None,
            file_path: String::new(),
        }
    }

    #[test]
    fn shelf_tag_filter_is_case_insensitive_and_allows_empty_results() {
        let items = vec![book("a", &["Fiction"]), book("b", &["Reference"])];
        assert_eq!(shelf_items_for_tag(&items, None).len(), 2);
        assert_eq!(shelf_items_for_tag(&items, Some("fiction"))[0].id, "a");
        assert!(shelf_items_for_tag(&items, Some("missing")).is_empty());
    }
}

/// 当前打开的书（后台线程写入，UI 读）。
static CURRENT_BOOK: LazyLock<RwLock<Option<OpenBook>>> = LazyLock::new(|| RwLock::new(None));

/// 当前打开的书对应的书架项（"标为已认识"后重开重注音用）。
static CURRENT_ITEM: LazyLock<RwLock<Option<ShelfItem>>> = LazyLock::new(|| RwLock::new(None));

/// 释义卡片操作结果（后台 → UI）。
#[derive(Debug)]
enum CardAction {
    VocabAdded(bool),
    MarkedKnown,
}

fn spawn_add_vocab(lemma: String, word: String, gloss: String, book_id: Option<String>) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        let level = readwise_core::vocab::lemma::word_level(&word);
        let gloss = if gloss.trim().is_empty() {
            db.get_word_glosses(std::slice::from_ref(&lemma))
                .ok()
                .and_then(|items| items.into_iter().next())
                .map(|item| item.gloss)
                .filter(|value| !value.trim().is_empty())
        } else {
            Some(gloss)
        };
        let input = readwise_core::vocab::models::VocabBookAddInput {
            lemma,
            word,
            gloss,
            level: Some(level),
            context_sentence: None,
            book_id,
        };
        match db.vocab_book_add(&input) {
            Ok(added) => Cx::post_action(CardAction::VocabAdded(added)),
            Err(e) => ::log::error!("加入生词本失败: {}", e),
        }
    });
}

fn spawn_mark_known(lemma: String) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        if let Err(e) = db.add_known_word(&lemma, "marked") {
            ::log::error!("标记已认识失败: {}", e);
            return;
        }
        // 只处理已经访问并缓存的章节；未访问章节会从 known_words 直接跳过该词。
        if let Some(book) = CURRENT_BOOK.write().unwrap().as_mut() {
            book.known_words.insert(lemma.clone());
            for chapter in book.glossed_chapters.iter_mut().flatten() {
                if chapter.contains("<ruby") {
                    *chapter =
                        readwise_core::vocab::annotate::remove_ruby_for_lemma(chapter, &lemma);
                }
            }
        }
        Cx::post_action(CardAction::MarkedKnown);
    });
}

/// 阅读字号（None = 用主题默认；A+/A- 调整后生效）。
static READER_FONT_SIZE: LazyLock<RwLock<Option<f32>>> = LazyLock::new(|| RwLock::new(None));

/// 选中文字的高亮色槽位：0=青 1=蓝 2=橙 3=黄。
static SELECTION_SLOT: LazyLock<RwLock<usize>> = LazyLock::new(|| RwLock::new(0));

/// 生词注音总开关（关闭 = 原样显示文章）。
static GLOSS_ENABLED: LazyLock<RwLock<bool>> = LazyLock::new(|| RwLock::new(true));

/// 当前 Makepad 主题。script_mod 重跑时从这里选择官方深色/浅色 token。
static READER_THEME: LazyLock<RwLock<ReaderTheme>> =
    LazyLock::new(|| RwLock::new(ReaderTheme::Dark));

/// 应用界面语言；正文内容与书籍元数据保持原文。
static APP_LANGUAGE: LazyLock<RwLock<AppLanguage>> =
    LazyLock::new(|| RwLock::new(AppLanguage::Chinese));

fn app_language() -> AppLanguage {
    *APP_LANGUAGE.read().unwrap()
}

fn tr(key: &'static str) -> &'static str {
    crate::i18n::text(app_language(), key)
}

fn localized(chinese: &'static str, english: &'static str) -> &'static str {
    if app_language() == AppLanguage::English {
        english
    } else {
        chinese
    }
}

/// EPUB 正文字体；应用 UI 与注音释义不跟随此设置。
static READER_FONT: LazyLock<RwLock<ReaderFont>> =
    LazyLock::new(|| RwLock::new(ReaderFont::Standard));

/// EPUB 正文行距；带注音段落始终比普通段落保留更多垂直空间。
static READER_SPACING: LazyLock<RwLock<ReaderSpacing>> =
    LazyLock::new(|| RwLock::new(ReaderSpacing::Default));

static READER_SETTINGS_SAVE_VERSION: AtomicU64 = AtomicU64::new(0);
static READER_SETTINGS_SAVE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

const READER_FONT_DEFAULT: f32 = 11.0;
const READER_FONT_MIN: f32 = 8.0;
const READER_FONT_MAX: f32 = 20.0;

struct OpenBook {
    id: String,
    title: String,
    /// 各章原始 body HTML（已消毒，不含运行时注音）。
    chapters: Vec<String>,
    /// 按访问惰性生成的注音章节；关闭注音时仍直接使用原始章节。
    glossed_chapters: Vec<Option<String>>,
    glosses: HashMap<String, String>,
    known_words: HashSet<String>,
    /// 各章标题（TOC 用）。
    chapter_titles: Vec<String>,
    /// 各章高亮（章下标 → 区间列表）。
    highlights: HashMap<usize, Vec<readwise_core::books::Highlight>>,
}

impl OpenBook {
    fn chapter_html(&mut self, idx: usize, gloss_enabled: bool) -> Option<&str> {
        let raw = self.chapters.get(idx)?;
        if !gloss_enabled || self.glosses.is_empty() {
            return Some(raw);
        }
        if self.glossed_chapters.get(idx)?.is_none() {
            use readwise_core::vocab::annotate::{annotate_html, flatten_lists};
            let flat = flatten_lists(raw);
            self.glossed_chapters[idx] =
                Some(annotate_html(&flat, &self.glosses, &self.known_words));
        }
        self.glossed_chapters[idx].as_deref()
    }
}

#[cfg(test)]
mod lazy_gloss_tests {
    use super::*;

    fn book() -> OpenBook {
        OpenBook {
            id: "book".to_string(),
            title: "Book".to_string(),
            chapters: vec![
                "<p>The solitude.</p>".to_string(),
                "<p>A murmur.</p>".to_string(),
            ],
            glossed_chapters: vec![None, None],
            glosses: HashMap::from([
                ("solitude".to_string(), "孤独".to_string()),
                ("murmur".to_string(), "低语".to_string()),
            ]),
            known_words: HashSet::new(),
            chapter_titles: vec!["One".to_string(), "Two".to_string()],
            highlights: HashMap::new(),
        }
    }

    #[test]
    fn glosses_only_the_requested_chapter_and_keeps_raw_html() {
        let mut book = book();
        let second = book.chapter_html(1, true).unwrap().to_string();
        assert!(second.contains("<ruby>murmur"));
        assert!(book.glossed_chapters[0].is_none());
        assert!(book.glossed_chapters[1].is_some());

        let raw_first = book.chapter_html(0, false).unwrap();
        assert_eq!(raw_first, "<p>The solitude.</p>");
        assert!(book.glossed_chapters[0].is_none());
    }

    #[test]
    fn newly_known_words_are_skipped_by_uncached_chapters() {
        let mut book = book();
        book.known_words.insert("solitude".to_string());
        let first = book.chapter_html(0, true).unwrap();
        assert!(!first.contains("<ruby>solitude"));
    }
}

/// 当前选区（块+字符区间，ChapterList 自跟踪写入，App 读）。
type TextPoint = (usize, usize);
type TextRange = (TextPoint, TextPoint);
static SEL_RANGE: LazyLock<RwLock<Option<TextRange>>> = LazyLock::new(|| RwLock::new(None));

/// 原生 PortalList 选区导出的纯文本（添加笔记时作为引用）。
static SELECTED_TEXT: LazyLock<RwLock<String>> = LazyLock::new(|| RwLock::new(String::new()));

/// 选区菜单覆盖在 ChapterList 上方；菜单按下到抬起期间阻止指针事件继续穿透正文。
static SELECTION_POPUP_POINTER: AtomicBool = AtomicBool::new(false);

/// 选区状态事件（ChapterList → App）。
#[derive(Debug)]
enum SelAction {
    Show,
    Hide,
}

/// 笔记后台操作结果。
#[derive(Debug)]
enum NoteAction {
    Refreshed {
        book_id: String,
        notes: Vec<readwise_core::notes::models::Note>,
        message: Option<String>,
        finish_composer: bool,
    },
    Error(String),
}

fn spawn_load_notes(book_id: String) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        match db.get_notes_by_book(&book_id) {
            Ok(notes) => Cx::post_action(NoteAction::Refreshed {
                book_id,
                notes,
                message: None,
                finish_composer: false,
            }),
            Err(e) => Cx::post_action(NoteAction::Error(e)),
        }
    });
}

fn spawn_create_note(item: ShelfItem, quote: String, extra: String) {
    let english = app_language() == AppLanguage::English;
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        let content = [quote.trim(), extra.trim()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        if let Err(e) = db.create_note(&item.id, &item.title, &item.author, &content) {
            Cx::post_action(NoteAction::Error(e));
            return;
        }
        match db.get_notes_by_book(&item.id) {
            Ok(notes) => Cx::post_action(NoteAction::Refreshed {
                book_id: item.id,
                notes,
                message: Some(if english {
                    "Note saved ✓".to_string()
                } else {
                    "笔记已保存 ✓".to_string()
                }),
                finish_composer: true,
            }),
            Err(e) => Cx::post_action(NoteAction::Error(e)),
        }
    });
}

fn spawn_delete_note(book_id: String, note_id: String) {
    let english = app_language() == AppLanguage::English;
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        if let Err(e) = db.delete_note(&note_id) {
            Cx::post_action(NoteAction::Error(e));
            return;
        }
        match db.get_notes_by_book(&book_id) {
            Ok(notes) => Cx::post_action(NoteAction::Refreshed {
                book_id,
                notes,
                message: Some(if english {
                    "Note deleted".to_string()
                } else {
                    "笔记已删除".to_string()
                }),
                finish_composer: false,
            }),
            Err(e) => Cx::post_action(NoteAction::Error(e)),
        }
    });
}

/// 高亮保存结果。
#[derive(Debug)]
enum HlAction {
    Saved(readwise_core::books::Highlight, usize),
}

fn spawn_add_highlight(book_id: String, chapter: usize, range: TextRange) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        let slot = *crate::mark_span::HL_SLOT.read().unwrap();
        let color = crate::mark_span::HL_COLOR_NAMES[slot.min(3)];
        let ((sb, sc), (eb, ec)) = range;
        match db.add_highlight(&book_id, chapter, (sb, sc, eb, ec), color) {
            Ok(id) => {
                let hl = readwise_core::books::Highlight {
                    id,
                    start_block: sb,
                    start_char: sc,
                    end_block: eb,
                    end_char: ec,
                    color: color.to_string(),
                };
                Cx::post_action(HlAction::Saved(hl, chapter));
            }
            Err(e) => ::log::error!("{}", e),
        }
    });
}

/// 当前章下标（App 写、TocList 读，做高亮）。
static CURRENT_CHAPTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// 打开书的结果（后台 → UI）。
#[derive(Debug)]
enum ReaderAction {
    Opened {
        resume_chapter: usize,
        gloss_count: usize,
        parse_ms: u128,
        initial_gloss_ms: u128,
    },
    OpenError(String),
}

fn selected_single_word(text: &str) -> Option<String> {
    let word = text.trim().trim_matches(|c: char| {
        !c.is_ascii_alphabetic() && c != '\'' && c != '\u{2019}' && c != '-'
    });
    (!word.is_empty()
        && word.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && word
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c == '\'' || c == '\u{2019}' || c == '-'))
    .then(|| word.to_string())
}

fn percent_encode_query(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            use std::fmt::Write;
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

fn speak_text(text: String) {
    std::thread::spawn(move || {
        #[cfg(target_os = "macos")]
        let result = std::process::Command::new("say").arg(&text).status();
        #[cfg(target_os = "windows")]
        let result = std::process::Command::new("PowerShell")
            .args([
                "-NoProfile",
                "-Command",
                "Add-Type -AssemblyName System.Speech; $s=New-Object System.Speech.Synthesis.SpeechSynthesizer; $s.Speak($args[0])",
                "--",
                &text,
            ])
            .status();
        #[cfg(target_os = "linux")]
        let result = std::process::Command::new("spd-say").arg(&text).status();
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        let _ = &text;
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        let result: Result<std::process::ExitStatus, std::io::Error> = Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "当前平台暂不支持系统发音",
        ));
        if let Err(error) = result {
            ::log::error!("发音失败: {}", error);
        }
    });
}

/// 后台加载整本书章节 + 恢复进度。
fn spawn_open_book(item: ShelfItem) {
    *CURRENT_ITEM.write().unwrap() = Some(item.clone());
    std::thread::spawn(move || {
        let open_started = std::time::Instant::now();
        let infos = match readwise_core::books::epub_meta::load_chapters(std::path::Path::new(
            &item.file_path,
        )) {
            Ok(list) => list,
            Err(e) => {
                Cx::post_action(ReaderAction::OpenError(e));
                return;
            }
        };
        let mut chapters = Vec::with_capacity(infos.len());
        let mut chapter_titles = Vec::with_capacity(infos.len());
        for info in infos {
            chapter_titles.push(info.title);
            chapters.push(info.html);
        }
        let parse_ms = open_started.elapsed().as_millis();
        let total = chapters.len();
        let resume = DB
            .get()
            .and_then(|db| db.get_status(&item.id).ok().flatten())
            .and_then(|s| s.location.parse::<usize>().ok())
            .filter(|idx| *idx < total)
            .unwrap_or(0);
        let highlights = DB
            .get()
            .and_then(|db| db.get_highlights_by_chapter(&item.id).ok())
            .unwrap_or_default();
        let (glosses, known_words) = DB
            .get()
            .and_then(|db| db.gloss_context(Some(&item.id)).ok())
            .unwrap_or_default();
        let gloss_count = glosses.len();
        let mut book = OpenBook {
            id: item.id,
            title: item.title,
            glossed_chapters: vec![None; chapters.len()],
            chapters,
            chapter_titles,
            glosses,
            known_words,
            highlights,
        };
        let gloss_started = std::time::Instant::now();
        if *GLOSS_ENABLED.read().unwrap() {
            let _ = book.chapter_html(resume, true);
        }
        let initial_gloss_ms = gloss_started.elapsed().as_millis();
        *CURRENT_BOOK.write().unwrap() = Some(book);
        Cx::post_action(ReaderAction::Opened {
            resume_chapter: resume,
            gloss_count,
            parse_ms,
            initial_gloss_ms,
        });
    });
}

/// 章节 → PortalList 块（含高亮标记注入）。
fn build_chapter_chunks(book: &mut OpenBook, idx: usize) -> Option<Vec<String>> {
    let chapter = book.chapter_html(idx, *GLOSS_ENABLED.read().unwrap())?;
    let mut chunks = readwise_core::books::epub_meta::split_blocks(chapter);
    if let Some(hls) = book.highlights.get(&idx) {
        let mut per_block: HashMap<usize, Vec<(usize, usize, &str)>> = HashMap::new();
        for hl in hls {
            for b in hl.start_block..=hl.end_block.min(chunks.len().saturating_sub(1)) {
                let from = if b == hl.start_block {
                    hl.start_char
                } else {
                    0
                };
                let to = if b == hl.end_block {
                    hl.end_char
                } else {
                    usize::MAX
                };
                per_block
                    .entry(b)
                    .or_default()
                    .push((from, to, hl.color.as_str()));
            }
        }
        for (b, ranges) in per_block {
            if let Some(chunk) = chunks.get_mut(b) {
                *chunk = readwise_core::vocab::annotate::inject_colored_marks(chunk, &ranges);
            }
        }
    }
    Some(chunks)
}

/// 后台保存阅读进度（章节粒度，location 存章节下标）。
fn spawn_save_progress(id: String, chapter_idx: usize, total: usize) {
    std::thread::spawn(move || {
        if let Some(db) = DB.get() {
            let _ = db.update_progress(
                &id,
                (chapter_idx + 1) as i64,
                total as i64,
                &chapter_idx.to_string(),
            );
        }
    });
}

fn shelf_status(language: AppLanguage, status: &str, cur: i64, total: i64) -> String {
    let name = match (language, status) {
        (AppLanguage::English, "reading") => "Reading",
        (AppLanguage::English, "completed") => "Finished",
        (AppLanguage::English, _) => "Unread",
        (_, "reading") => "在读",
        (_, "completed") => "读完",
        _ => "未读",
    };
    if total > 0 {
        format!("{} · {}%", name, (cur * 100 / total).min(100))
    } else {
        name.to_string()
    }
}

/// 书架加载结果（后台 → UI）。
#[derive(Debug)]
enum ShelfAction {
    Loaded {
        items: usize,
        imported: usize,
        import_error: Option<String>,
    },
    TagsSaved {
        items: usize,
    },
    Deleted {
        items: usize,
        title: String,
    },
    TagError(String),
    DeleteError(String),
    Error(String),
}

/// 从 DB 读书架写入静态区，返回本数（后台线程内调用）。
fn refresh_shelf(db: &readwise_core::Db) -> Result<usize, String> {
    let items = db.get_shelf_items()?;
    let ui_items: Vec<ShelfItem> = items
        .iter()
        .map(|b| ShelfItem {
            id: b.id.clone(),
            title: b.title.clone(),
            author: b.author.clone(),
            status: b.status.clone(),
            progress_current: b.progress_current,
            progress_total: b.progress_total,
            tags: b.tags.clone(),
            cover_path: b.cover_abs_path.clone(),
            file_path: b.file_abs_path.clone(),
        })
        .collect();
    let n = ui_items.len();
    *SHELF.write().unwrap() = ui_items;
    Ok(n)
}

fn spawn_set_book_tags(book_id: String, tags: Vec<String>) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        if let Err(error) = db.set_book_tags(&book_id, &tags) {
            Cx::post_action(ShelfAction::TagError(error));
            return;
        }
        match refresh_shelf(db) {
            Ok(items) => Cx::post_action(ShelfAction::TagsSaved { items }),
            Err(error) => Cx::post_action(ShelfAction::TagError(error)),
        }
    });
}

/// 后台加载书架：先扫描导入目录，再读全部书目。
fn spawn_load_shelf() {
    std::thread::spawn(|| {
        let Some(db) = DB.get() else { return };
        let imported = db.scan_import_dir().unwrap_or(0);
        match refresh_shelf(db) {
            Ok(n) => Cx::post_action(ShelfAction::Loaded {
                items: n,
                imported,
                import_error: None,
            }),
            Err(e) => Cx::post_action(ShelfAction::Error(e)),
        }
    });
}

/// 后台删除书籍后刷新书架；删除失败时不再伪装成一次普通刷新。
fn spawn_delete(id: String, title: String) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else {
            Cx::post_action(ShelfAction::DeleteError("数据库尚未就绪".to_string()));
            return;
        };
        if let Err(error) = db.delete_book(&id) {
            Cx::post_action(ShelfAction::DeleteError(error));
            return;
        }
        match refresh_shelf(db) {
            Ok(items) => Cx::post_action(ShelfAction::Deleted { items, title }),
            Err(error) => Cx::post_action(ShelfAction::DeleteError(error)),
        }
    });
}

fn is_epub_path(path: &str) -> bool {
    path.to_lowercase().ends_with(".epub")
}

/// macOS 拖放给的是 URL 百分号编码路径（中文/空格会变 %XX），解码成真实路径。
fn percent_decode_path(s: &str) -> String {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_epub_item(item: &DragItem) -> bool {
    matches!(item, DragItem::FilePath { path, .. } if is_epub_path(path))
}

/// EPUB 文件在后台导入，完成后刷新书架。系统选择器的 import 模式会给应用一份
/// 临时私有副本，成功入库后可以清理；桌面拖放的用户原文件绝不删除。
fn spawn_import(paths: Vec<String>, cleanup_sources: bool) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        let mut imported = 0;
        let mut errors = Vec::new();
        for p in paths {
            match db.import_book(std::path::Path::new(&p)) {
                Ok(_) => {
                    imported += 1;
                    if cleanup_sources {
                        let _ = std::fs::remove_file(&p);
                    }
                }
                Err(e) => {
                    println!("导入失败 {}: {}", p, e);
                    errors.push(e);
                }
            }
        }
        match refresh_shelf(db) {
            Ok(n) => Cx::post_action(ShelfAction::Loaded {
                items: n,
                imported,
                import_error: errors.into_iter().next(),
            }),
            Err(e) => Cx::post_action(ShelfAction::Error(e)),
        }
    });
}

/// 数据库初始化结果（后台线程 → UI）。
#[derive(Debug)]
enum DbAction {
    Ready {
        vocab_count: usize,
        known_count: usize,
        reader_settings: ReaderSettings,
    },
    Failed(String),
}

fn spawn_db_init(platform_data_dir: Option<std::path::PathBuf>) {
    std::thread::spawn(move || {
        let data_dir =
            readwise_core::paths::app_data_dir_from_platform(platform_data_dir.as_deref());
        match readwise_core::Db::open_in(data_dir) {
            Ok(db) => {
                let entries = db.vocab_book_list().unwrap_or_default();
                let known = db.get_known_words().unwrap_or_default();
                let reader_settings = db.load_reader_settings().unwrap_or_default();
                let action = DbAction::Ready {
                    vocab_count: entries.len(),
                    known_count: known.len(),
                    reader_settings,
                };
                let _ = DB.set(db);
                Cx::post_action(action);
            }
            Err(e) => Cx::post_action(DbAction::Failed(e)),
        }
    });
}

#[derive(Debug)]
enum AiStateAction {
    Loaded {
        settings: readwise_core::ai::AiSettings,
        thread_id: Option<String>,
        messages: Vec<readwise_core::threads::models::ThreadMessage>,
    },
    SettingsSaved(readwise_core::ai::AiSettings),
    ThreadSaved(String),
    Error(String),
}

#[derive(Debug)]
enum LocalAiAction {
    Loaded(readwise_core::local_ai::LocalAiSettings),
    Saved(readwise_core::local_ai::LocalAiSettings),
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    Started {
        settings: readwise_core::local_ai::LocalAiSettings,
        session: readwise_core::local_ai::LocalAiSession,
    },
    Stopped,
    StartFailed(String),
    Error(String),
}

#[derive(Debug)]
enum ReaderSettingsAction {
    Saved,
    Error(String),
}

fn current_reader_settings() -> ReaderSettings {
    use crate::ruby_word::*;

    ReaderSettings {
        language: *APP_LANGUAGE.read().unwrap(),
        theme: *READER_THEME.read().unwrap(),
        font: *READER_FONT.read().unwrap(),
        spacing: *READER_SPACING.read().unwrap(),
        font_size: *READER_FONT_SIZE.read().unwrap(),
        gloss_enabled: *GLOSS_ENABLED.read().unwrap(),
        gloss_font_size: *GLOSS_FONT_SIZE.read().unwrap(),
        gloss_color_slot: GLOSS_PRESETS
            .iter()
            .position(|(_, color)| *color == *GLOSS_COLOR.read().unwrap())
            .unwrap_or(0),
        gloss_background_slot: BG_PRESETS
            .iter()
            .position(|(_, color)| *color == *GLOSS_BG.read().unwrap())
            .unwrap_or(0),
        word_color_slot: WORD_COLOR_PRESETS
            .iter()
            .position(|(_, color)| *color == *WORD_COLOR.read().unwrap())
            .unwrap_or(0),
        word_background_slot: BG_PRESETS
            .iter()
            .position(|(_, color)| *color == *WORD_BG.read().unwrap())
            .unwrap_or(0),
        selection_color_slot: *SELECTION_SLOT.read().unwrap(),
        highlight_color_slot: *crate::mark_span::HL_SLOT.read().unwrap(),
    }
}

fn spawn_save_reader_settings(settings: ReaderSettings) {
    let version = READER_SETTINGS_SAVE_VERSION.fetch_add(1, Ordering::Relaxed) + 1;
    std::thread::spawn(move || {
        let Ok(_guard) = READER_SETTINGS_SAVE_LOCK.lock() else {
            Cx::post_action(ReaderSettingsAction::Error(
                "阅读设置保存队列已损坏".to_string(),
            ));
            return;
        };
        if version != READER_SETTINGS_SAVE_VERSION.load(Ordering::Relaxed) {
            return;
        }
        let Some(db) = DB.get() else { return };
        match db.save_reader_settings(&settings) {
            Ok(()) if version == READER_SETTINGS_SAVE_VERSION.load(Ordering::Relaxed) => {
                Cx::post_action(ReaderSettingsAction::Saved);
            }
            Ok(()) => {}
            Err(error) => Cx::post_action(ReaderSettingsAction::Error(error)),
        }
    });
}

fn spawn_load_local_ai_settings() {
    std::thread::spawn(|| {
        let Some(db) = DB.get() else { return };
        match db.load_local_ai_settings() {
            Ok(settings) => Cx::post_action(LocalAiAction::Loaded(settings)),
            Err(error) => Cx::post_action(LocalAiAction::Error(error)),
        }
    });
}

fn spawn_save_local_ai_settings(settings: readwise_core::local_ai::LocalAiSettings) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        match db.save_local_ai_settings(&settings) {
            Ok(()) => Cx::post_action(LocalAiAction::Saved(settings)),
            Err(error) => Cx::post_action(LocalAiAction::Error(error)),
        }
    });
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn spawn_start_local_ai(settings: readwise_core::local_ai::LocalAiSettings) {
    LOCAL_AI_CANCEL.store(false, Ordering::Relaxed);
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        if let Err(error) = db.save_local_ai_settings(&settings) {
            Cx::post_action(LocalAiAction::StartFailed(error));
            return;
        }
        let result = LOCAL_AI_MANAGER
            .lock()
            .map_err(|_| "本地 AI 进程状态锁已损坏".to_string())
            .and_then(|mut manager| manager.start_cancellable(&settings, &LOCAL_AI_CANCEL));
        match result {
            Ok(session) => Cx::post_action(LocalAiAction::Started { settings, session }),
            Err(error) => Cx::post_action(LocalAiAction::StartFailed(error)),
        }
    });
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn spawn_start_local_ai(_settings: readwise_core::local_ai::LocalAiSettings) {
    Cx::post_action(LocalAiAction::StartFailed(
        "移动端不支持启动 llama.cpp 子进程，请使用远程 API".to_string(),
    ));
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn spawn_stop_local_ai() {
    LOCAL_AI_CANCEL.store(true, Ordering::Relaxed);
    std::thread::spawn(|| {
        let result = LOCAL_AI_MANAGER
            .lock()
            .map_err(|_| "本地 AI 进程状态锁已损坏".to_string())
            .and_then(|mut manager| manager.stop());
        match result {
            Ok(()) => Cx::post_action(LocalAiAction::Stopped),
            Err(error) => Cx::post_action(LocalAiAction::Error(error)),
        }
    });
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn spawn_stop_local_ai() {
    Cx::post_action(LocalAiAction::Stopped);
}

fn spawn_load_ai_state() {
    std::thread::spawn(|| {
        let Some(db) = DB.get() else { return };
        let settings = match db.load_ai_settings() {
            Ok(settings) => settings,
            Err(error) => {
                Cx::post_action(AiStateAction::Error(error));
                return;
            }
        };
        match db.latest_thread(None) {
            Ok(thread) => Cx::post_action(AiStateAction::Loaded {
                settings,
                thread_id: thread.as_ref().map(|thread| thread.id.clone()),
                messages: thread.map(|thread| thread.messages).unwrap_or_default(),
            }),
            Err(error) => Cx::post_action(AiStateAction::Error(error)),
        }
    });
}

fn spawn_save_ai_settings(settings: readwise_core::ai::AiSettings) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        match db.save_ai_settings(&settings) {
            Ok(()) => Cx::post_action(AiStateAction::SettingsSaved(settings)),
            Err(error) => Cx::post_action(AiStateAction::Error(error)),
        }
    });
}

fn spawn_save_chat(thread_id: Option<String>, messages: Vec<ChatMessage>) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        if messages.is_empty() {
            return;
        }
        let stored: Vec<_> = messages
            .iter()
            .map(|message| readwise_core::threads::models::ThreadMessage {
                role: match message.role {
                    ChatRole::User => readwise_core::threads::models::ThreadRole::User,
                    ChatRole::Assistant => readwise_core::threads::models::ThreadRole::Assistant,
                },
                content: message.text.clone(),
            })
            .collect();
        let title = messages
            .iter()
            .find(|message| message.role == ChatRole::User)
            .map(|message| preview_text(&message.text, 32))
            .unwrap_or_else(|| "新对话".to_string());
        match db.save_thread(thread_id.as_deref(), None, &title, "{}", &stored) {
            Ok(thread) => Cx::post_action(AiStateAction::ThreadSaved(thread.id)),
            Err(error) => Cx::post_action(AiStateAction::Error(error)),
        }
    });
}

script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*

    mod.widgets.BookShelfCard = RoundedView{
        width: Fill height: 94
        new_batch: true
        flow: Right spacing: 14 padding: 12
        align: Align{ y: 0.5 }
        draw_bg.color: theme.color_bg_container
        draw_bg.border_radius: 8.0
        cover := View{
            width: 44 height: 56
            flow: Overlay
            cover_ph := SolidView{
                width: Fill height: Fill
                align: Align{ x: 0.5 y: 0.5 }
                draw_bg.color: theme.color_highlight
                draw_bg.border_radius: 4.0
                cover_letter := Label{
                    text: ""
                    draw_text.text_style.font_size: 20
                }
            }
            cover_img := Image{
                width: Fill height: Fill
                fit: ImageFit.Biggest
            }
        }
        info := View{
            width: Fill height: Fit
            flow: Down spacing: 4
            card_title := Label{
                width: Fill
                text: ""
                draw_text.text_style.font_size: 14
            }
            card_author := Label{
                width: Fill
                text: ""
                draw_text.color: theme.color_label_inner_inactive
            }
            card_status := Label{
                width: Fill
                text: ""
                draw_text.color: theme.color_highlight
                draw_text.text_style.font_size: 9
            }
            card_tags := Label{
                width: Fill
                text: ""
                draw_text.color: theme.color_label_inner_inactive
                draw_text.text_style.font_size: 8.5
            }
        }
        read_btn := ButtonFlat{ text: "阅读" }
        tag_btn := ButtonFlatter{ text: "标签" }
        del_btn := ButtonFlat{ text: "删除" }
    }

    mod.widgets.BookShelfBase = #(BookShelf::register_widget(vm))
    mod.widgets.BookShelf = set_type_default() do mod.widgets.BookShelfBase{
        width: Fill height: Fill
        list := PortalList{
            width: Fill height: Fill
            padding: Inset{ left: 24 right: 24 top: 8 bottom: 24 }
            spacing: 10
            scroll_bar: ScrollBar{}
            Card := mod.widgets.BookShelfCard{}
            CardAlternate := mod.widgets.BookShelfCard{}
        }
    }

    mod.widgets.TocListBase = #(TocList::register_widget(vm))
    mod.widgets.TocList = set_type_default() do mod.widgets.TocListBase{
        width: Fill height: Fill
        active_bg: theme.color_bg_highlight_inline
        inactive_bg: theme.color_bg_app
        list := PortalList{
            width: Fill height: Fill
            padding: Inset{ left: 8 right: 8 top: 8 bottom: 8 }
            spacing: 2
            scroll_bar: ScrollBar{}
            TocItem := SolidView{
                width: Fill height: Fit
                draw_bg.color: theme.color_bg_app
                toc_item_btn := ButtonFlat{
                    width: Fill
                    text: ""
                }
            }
        }
    }

    let NoteCardTemplate = RoundedView{
        width: Fill height: Fit
        new_batch: true
        padding: 10
        flow: Down spacing: 6
        draw_bg.color: theme.color_bg_container
        draw_bg.border_radius: 7.0
        note_title := Label{
            width: Fill
            text: ""
            draw_text.text_style.font_size: 11
        }
        note_preview := Label{
            width: Fill
            text: ""
            draw_text.color: theme.color_label_inner_inactive
            draw_text.text_style.font_size: 9
        }
        note_actions := View{
            width: Fill height: Fit
            flow: Right
            align: Align{x: 1.0 y: 0.5}
            note_delete_btn := ButtonFlatter{ text: "删除" }
        }
    }

    mod.widgets.NoteListBase = #(NoteList::register_widget(vm))
    mod.widgets.NoteList = set_type_default() do mod.widgets.NoteListBase{
        width: Fill height: Fill
        list := PortalList{
            width: Fill height: Fill
            spacing: 8
            scroll_bar: ScrollBar{}
            NoteCard := NoteCardTemplate{}
            NoteCardAlternate := NoteCardTemplate{}
        }
    }

    let SkillCardTemplate = RoundedView{
        width: Fill height: Fit
        flow: Down spacing: 7 padding: 10
        new_batch: true
        draw_bg.color: theme.color_bg_container
        draw_bg.border_radius: 8.0
        skill_top := View{
            width: Fill height: Fit
            flow: Right spacing: 8
            align: Align{y: 0.5}
            skill_name := Label{
                width: Fill height: Fit
                text: ""
                draw_text.text_style.font_size: 11
            }
            skill_kind := Label{
                width: Fit height: Fit
                text: ""
                draw_text.color: theme.color_highlight
                draw_text.text_style.font_size: 8.5
            }
        }
        skill_preview := Label{
            width: Fill height: Fit
            text: ""
            draw_text.color: theme.color_label_inner_inactive
            draw_text.text_style.font_size: 9
        }
        skill_actions := View{
            width: Fill height: Fit
            flow: Right spacing: 4
            align: Align{y: 0.5}
            skill_active := CheckBox{text: "启用"}
            skill_use := ButtonFlatter{text: "下次提问使用"}
            skill_edit := ButtonFlatter{text: "编辑"}
            skill_delete := ButtonFlatter{text: "删除"}
        }
    }

    mod.widgets.ChatListBase = #(ChatList::register_widget(vm))
    mod.widgets.ChatList = set_type_default() do mod.widgets.ChatListBase{
        width: Fill height: Fill
        list := PortalList{
            width: Fill height: Fill
            auto_tail: true
            smooth_tail: true
            spacing: 8
            padding: Inset{left: 20 right: 20 top: 12 bottom: 12}
            scroll_bar: ScrollBar{}
            User := RoundedView{
                width: Fill height: Fit
                margin: Inset{left: 90}
                padding: 12
                new_batch: true
                draw_bg.color: theme.color_bg_highlight_inline
                draw_bg.border_radius: 9.0
                message := Label{
                    width: Fill height: Fit
                    text: ""
                    draw_text.text_style.font_size: 11
                }
            }
            Assistant := RoundedView{
                width: Fill height: Fit
                margin: Inset{right: 90}
                padding: 12
                new_batch: true
                draw_bg.color: theme.color_bg_container
                draw_bg.border_radius: 9.0
                message := Label{
                    width: Fill height: Fit
                    text: ""
                    draw_text.text_style.font_size: 11
                }
            }
        }
    }

    mod.widgets.SkillListBase = #(SkillList::register_widget(vm))
    mod.widgets.SkillList = set_type_default() do mod.widgets.SkillListBase{
        width: Fill height: Fill
        list := PortalList{
            width: Fill height: Fill
            spacing: 8
            padding: Inset{right: 8 bottom: 8}
            scroll_bar: ScrollBar{}
            SkillCard := SkillCardTemplate{}
            SkillCardAlternate := SkillCardTemplate{}
        }
    }

    let VocabCardTemplate = RoundedView{
        width: Fill height: Fit
        flow: Down spacing: 7 padding: 12
        new_batch: true
        draw_bg.color: theme.color_bg_container
        draw_bg.border_radius: 8.0
        vocab_card_top := View{
            width: Fill height: Fit
            flow: Right spacing: 10
            align: Align{y: 0.5}
            vocab_word := Label{
                width: Fill height: Fit
                text: ""
                draw_text.text_style.font_size: 15
            }
            vocab_status := Label{
                width: Fit height: Fit
                text: ""
                draw_text.color: theme.color_highlight
                draw_text.text_style.font_size: 9
            }
        }
        vocab_gloss := Label{
            width: Fill height: Fit
            text: ""
            draw_text.color: theme.color_text
            draw_text.text_style.font_size: 11
        }
        vocab_context := Label{
            width: Fill height: Fit
            text: ""
            draw_text.color: theme.color_label_inner_inactive
            draw_text.text_style.font_size: 9
        }
        vocab_card_bottom := View{
            width: Fill height: Fit
            flow: Right spacing: 6
            align: Align{y: 0.5}
            vocab_meta := Label{
                width: Fill height: Fit
                text: ""
                draw_text.color: theme.color_label_inner_disabled
                draw_text.text_style.font_size: 8.5
            }
            vocab_speak := ButtonFlatter{text: "发音"}
            vocab_review := ButtonFlatter{text: "复习"}
            vocab_toggle := ButtonFlatter{text: "标为掌握"}
            vocab_remove := ButtonFlatter{text: "移除"}
        }
    }

    mod.widgets.VocabListBase = #(VocabList::register_widget(vm))
    mod.widgets.VocabList = set_type_default() do mod.widgets.VocabListBase{
        width: Fill height: Fill
        list := PortalList{
            width: Fill height: Fill
            spacing: 8
            padding: Inset{right: 8 bottom: 16}
            scroll_bar: ScrollBar{}
            VocabCard := VocabCardTemplate{}
            VocabCardAlternate := VocabCardTemplate{}
        }
    }

    mod.widgets.StatsBarsBase = #(StatsBars::register_widget(vm))
    mod.widgets.StatsBars = set_type_default() do mod.widgets.StatsBarsBase{
        width: Fill height: 190
        draw_bg.color: theme.color_bg_container
        draw_grid.color: theme.color_chart_grid
        draw_bar.color: theme.color_chart_reading
    }

    mod.widgets.ChapterListBase = #(ChapterList::register_widget(vm))
    mod.widgets.ChapterList = set_type_default() do mod.widgets.ChapterListBase{
        width: Fill height: Fill
        font_default_size: theme.font_size_p
        font_standard_normal: theme.font_regular{}
        font_standard_italic: theme.font_italic{}
        font_standard_bold: theme.font_bold{}
        font_standard_bold_italic: theme.font_bold_italic{}
        font_monospace: theme.font_code{}
        list := PortalList{
            width: Fill height: Fill
            selectable: true
            padding: Inset{ left: 32 right: 32 top: 8 bottom: 32 }
            spacing: 6
            scroll_bar: ScrollBar{}
            Para := View{
                width: Fill height: Fit
                para_html := Html{
                    width: Fill height: Fit
                    selectable: true
                    a.color: theme.color_text
                    a.hover_color: theme.color_text_hover
                    a.pressed_color: theme.color_text_focus
                    img := mod.widgets.ChapterImage{}
                    mark := mod.widgets.MarkSpan{}
                }
            }
            GlossPara := View{
                width: Fill height: Fit
                para_html := Html{
                    width: Fill height: Fit
                    selectable: true
                    padding: Inset{ top: 22 }
                    text_style_normal.line_spacing: 2.2
                    text_style_bold.line_spacing: 2.2
                    text_style_italic.line_spacing: 2.2
                    text_style_fixed.line_spacing: 2.2
                    paragraph_margin.bottom: 1.4
                    heading_margin.bottom: 1.1
                    a.color: theme.color_text
                    a.hover_color: theme.color_text_hover
                    a.pressed_color: theme.color_text_focus
                    ruby := mod.widgets.RubyWord{}
                    img := mod.widgets.ChapterImage{}
                    mark := mod.widgets.MarkSpan{}
                }
            }
        }
    }

    startup() do #(App::script_component(vm)){
        swatch_inactive: theme.color_bg_container
        ui: Root{
            main_window := Window{
                window.title: "ReadWise"
                window.inner_size: vec2(1080, 720)
                body +: {
                    flow: Overlay
                    app_content := View{
                        width: Fill height: Fill
                        flow: Right
                        sidebar := SolidView{
                        width: 150 height: Fill
                        flow: Down padding: 12 spacing: 8
                        draw_bg.color: theme.color_bg_app
                        app_title := Label{
                            text: "ReadWise"
                            draw_text.text_style.font_size: 15
                        }
                        nav_library := ButtonFlat{ width: Fill text: "书库" }
                        nav_reader := ButtonFlat{ width: Fill text: "阅读" }
                        nav_chat := ButtonFlat{ width: Fill text: "AI 对话" }
                        nav_skills := ButtonFlat{ width: Fill text: "AI 技能" }
                        nav_vocab := ButtonFlat{ width: Fill text: "生词本" }
                        nav_statistics := ButtonFlat{ width: Fill text: "统计" }
                        nav_settings := ButtonFlat{ width: Fill text: "设置" }
                        sidebar_fill := View{ width: Fill height: Fill }
                        db_status := Label{
                            width: Fill
                            text: "数据库初始化中…"
                            draw_text.text_style.font_size: 8.5
                        }
                    }
                    main_column := View{
                        width: Fill height: Fill
                        flow: Down
                        mobile_safe_top := SolidView{
                            visible: false
                            width: Fill height: 0
                            draw_bg.color: theme.color_bg_app
                        }
                        pages := PageFlip{
                            width: Fill height: Fill
                            active_page: @page_reader

                        page_library := View{
                            width: Fill height: Fill
                            flow: Down
                            lib_header := View{
                                width: Fill height: Fit
                                padding: Inset{ left: 24 right: 24 top: 20 bottom: 8 }
                                flow: Down spacing: 4
                                lib_title_row := View{
                                    width: Fill height: Fit
                                    flow: Right spacing: 10
                                    align: Align{y: 0.5}
                                    library_title := Label{
                                        width: Fill
                                        text: "书库"
                                        draw_text.text_style.font_size: 18
                                    }
                                    import_epub_btn := ButtonFlat{
                                        visible: false
                                        text: "导入 EPUB"
                                    }
                                }
                                library_hint := Label{
                                    width: Fill
                                    text: "把 EPUB 拖进窗口，或放入导入目录后重启。"
                                    draw_text.color: theme.color_label_inner_inactive
                                    draw_text.text_style.font_size: 9
                                }
                                library_filter_row := View{
                                    width: Fill height: Fit
                                    flow: Right spacing: 8
                                    align: Align{y: 0.5}
                                    library_tag_filter_label := Label{
                                        text: "标签筛选"
                                        draw_text.color: theme.color_label_inner_inactive
                                        draw_text.text_style.font_size: 9
                                    }
                                    library_tag_filter := DropDown{
                                        width: 180 height: 32
                                        labels: ["全部标签"]
                                        selected_item: 0
                                    }
                                    library_filter_summary := Label{
                                        width: Fill
                                        text: ""
                                        draw_text.color: theme.color_label_inner_inactive
                                        draw_text.text_style.font_size: 9
                                    }
                                }
                            }
                            library_body := View{
                                width: Fill height: Fill
                                flow: Overlay
                                library_empty := View{
                                    visible: true
                                    width: Fill height: Fill
                                    flow: Down spacing: 8
                                    align: Center
                                    library_empty_title := Label{
                                        text: "书架为空"
                                        draw_text.text_style.font_size: 14
                                    }
                                    library_empty_hint := Label{
                                        text: "拖入 EPUB 文件，或放进 ReadWise/import 目录"
                                        draw_text.color: theme.color_label_inner_inactive
                                        draw_text.text_style.font_size: 9
                                    }
                                }
                                book_shelf := mod.widgets.BookShelf{visible: false}
                            }
                        }

                        page_reader := View{
                            width: Fill height: Fill
                            flow: Down
                            header := View{
                                width: Fill height: Fit
                                padding: 12 spacing: 12
                                align: Align{ y: 0.5 }
                                toc_btn := ButtonFlat{ text: "目录" }
                                notes_btn := ButtonFlat{ text: "笔记" }
                                reader_ai_btn := ButtonFlat{ text: "AI" }
                                title := Label{
                                    text: "阅读 · 示例章节"
                                    draw_text.text_style.font_size: 15
                                }
                                spacer := View{ width: Fill height: Fit }
                                status := Label{ text: "" }
                                chapter_pos := Label{
                                    text: ""
                                    draw_text.color: theme.color_label_inner_inactive
                                    draw_text.text_style.font_size: 9.5
                                }
                            }
                            reader_body := View{
                                width: Fill height: Fill
                                flow: Overlay
                                reader_columns := View{
                                    width: Fill height: Fill
                                    flow: Right
                                    toc_panel := SolidView{
                                    visible: false
                                    width: 260 height: Fill
                                    flow: Down
                                    draw_bg.color: theme.color_bg_app
                                    toc_list := mod.widgets.TocList{}
                                    }
                                    chapter_list := mod.widgets.ChapterList{}
                                    note_panel := SolidView{
                                    visible: false
                                    width: 320 height: Fill
                                    new_batch: true
                                    flow: Down spacing: 10 padding: 12
                                    draw_bg.color: theme.color_bg_app
                                    note_header := View{
                                        width: Fill height: Fit
                                        flow: Right spacing: 8
                                        align: Align{y: 0.5}
                                        note_panel_title := Label{
                                            width: Fill
                                            text: "笔记"
                                            draw_text.text_style.font_size: 14
                                        }
                                        note_close_btn := ButtonFlatter{ text: "关闭" }
                                    }
                                    note_composer := RoundedView{
                                        visible: false
                                        width: Fill height: Fit
                                        new_batch: true
                                        flow: Down spacing: 8 padding: 10
                                        draw_bg.color: theme.color_bg_container
                                        draw_bg.border_radius: 8.0
                                        note_quote_title := Label{
                                            text: "引用"
                                            draw_text.color: theme.color_highlight
                                            draw_text.text_style.font_size: 9
                                        }
                                        note_quote := Label{
                                            width: Fill
                                            text: ""
                                            draw_text.color: theme.color_text
                                            draw_text.text_style.font_size: 9.5
                                        }
                                        note_extra := TextInput{
                                            width: Fill height: 38
                                            empty_text: "补充内容（可选，回车保存）"
                                        }
                                        note_compose_actions := View{
                                            width: Fill height: Fit
                                            flow: Right spacing: 8
                                            align: Align{x: 1.0 y: 0.5}
                                            note_cancel_btn := ButtonFlatter{ text: "取消" }
                                            note_save_btn := ButtonFlat{ text: "保存" }
                                        }
                                    }
                                    note_hint := Label{
                                        width: Fill
                                        text: "选中文本后可添加笔记"
                                        draw_text.color: theme.color_label_inner_inactive
                                        draw_text.text_style.font_size: 9
                                    }
                                    note_empty := View{
                                        visible: true
                                        width: Fill height: 110
                                        flow: Down spacing: 7
                                        align: Center
                                        note_empty_title := Label{
                                            text: "还没有笔记"
                                            draw_text.text_style.font_size: 12
                                        }
                                        note_empty_hint := Label{
                                            text: "选中文本，点击「笔记」创建第一条。"
                                            draw_text.color: theme.color_label_inner_inactive
                                            draw_text.text_style.font_size: 8.5
                                        }
                                    }
                                    note_list := mod.widgets.NoteList{visible: false}
                                    }
                                    reader_ai_panel := SolidView{
                                    visible: false
                                    width: 360 height: Fill
                                    new_batch: true
                                    flow: Down spacing: 10 padding: 12
                                    draw_bg.color: theme.color_bg_app
                                    reader_ai_header := View{
                                        width: Fill height: Fit
                                        flow: Right spacing: 8
                                        align: Align{y: 0.5}
                                        reader_ai_title := Label{
                                            width: Fill
                                            text: "阅读 AI"
                                            draw_text.text_style.font_size: 14
                                        }
                                        reader_ai_close := ButtonFlatter{text: "关闭"}
                                    }
                                    reader_chat_status := Label{
                                        width: Fill height: Fit
                                        text: "选择文字后点“问 AI”，或在下方直接提问"
                                        draw_text.color: theme.color_label_inner_inactive
                                        draw_text.text_style.font_size: 9
                                    }
                                    reader_chat_list := mod.widgets.ChatList{}
                                    reader_ai_composer := View{
                                        width: Fill height: Fit
                                        flow: Down spacing: 8
                                        reader_chat_input := TextInput{
                                            width: Fill height: 42
                                            empty_text: "继续追问，按回车发送"
                                        }
                                        reader_ai_actions := View{
                                            width: Fill height: Fit
                                            flow: Right spacing: 6
                                            reader_chat_send := Button{width: 64 text: "发送"}
                                            reader_chat_cancel := ButtonFlat{
                                                visible: false
                                                width: 64 text: "停止"
                                            }
                                            reader_chat_clear := ButtonFlatter{width: 72 text: "新对话"}
                                        }
                                    }
                                    }
                                }
                                sel_bar := View{
                                    visible: false
                                    width: Fill height: Fill
                                    flow: Down
                                    align: Align{x: 0.5 y: 1.0}
                                    padding: Inset{left: 18 right: 18 bottom: 18}
                                    selection_popup := RoundedView{
                                        width: 520 height: Fit
                                        new_batch: true
                                        padding: Inset{left: 12 right: 12 top: 9 bottom: 9}
                                        flow: Down spacing: 5
                                        draw_bg.color: theme.color_bg_container
                                        draw_bg.border_radius: 10.0
                                        selection_actions := ScrollXView{
                                            width: Fill height: 38
                                            flow: Right spacing: 6
                                            align: Align{y: 0.5}
                                            copy_sel_btn := ButtonFlat{text: "复制"}
                                            note_from_sel_btn := ButtonFlat{text: "笔记"}
                                            vocab_from_sel_btn := ButtonFlat{text: "生词"}
                                            translate_sel_btn := ButtonFlat{text: "翻译"}
                                            ask_ai_sel_btn := ButtonFlat{text: "问 AI"}
                                            speak_sel_btn := ButtonFlat{text: "发音"}
                                            hl_btn := ButtonFlat{text: "高亮"}
                                            sel_close_btn := ButtonFlatter{text: "关闭"}
                                        }
                                        sel_hint := Label{
                                            width: Fill
                                            text: ""
                                            draw_text.color: theme.color_label_inner_inactive
                                            draw_text.text_style.font_size: 8.5
                                        }
                                    }
                                }
                            }
                            word_card := RoundedView{
                                visible: false
                                width: Fill height: Fit
                                margin: Inset{ left: 16 right: 16 bottom: 12 top: 4 }
                                padding: 14
                                flow: Down spacing: 10
                                draw_bg.color: theme.color_bg_container
                                draw_bg.border_radius: 10.0
                                card_head := View{
                                    width: Fill height: Fit
                                    spacing: 14
                                    align: Align{ y: 0.5 }
                                    card_word := Label{
                                        text: ""
                                        draw_text.text_style.font_size: 17
                                    }
                                    card_gloss := Label{
                                        width: Fill
                                        text: ""
                                        draw_text.color: theme.color_highlight
                                        draw_text.text_style.font_size: 13
                                    }
                                    close_card_btn := ButtonFlat{ text: "关闭" }
                                }
                                card_foot := View{
                                    width: Fill height: Fit
                                    spacing: 14
                                    align: Align{ y: 0.5 }
                                    add_vocab_btn := ButtonFlat{ text: "加入生词本" }
                                    know_btn := ButtonFlat{ text: "标为已认识" }
                                    card_hint := Label{
                                        width: Fill
                                        text: ""
                                        draw_text.color: theme.color_label_inner_inactive
                                        draw_text.text_style.font_size: 9
                                    }
                                }
                            }
                        }

                        page_vocab := View{
                            width: Fill height: Fill
                            flow: Down padding: 24 spacing: 12
                            vocab_header := View{
                                width: Fill height: Fit
                                flow: Right spacing: 12
                                align: Align{y: 0.5}
                                vocab_title := Label{
                                    width: Fill height: Fit
                                    text: "生词本"
                                    draw_text.text_style.font_size: 18
                                }
                                vocab_start_review := Button{text: "开始复习"}
                            }
                            vocab_summary := Label{
                                width: Fill height: Fit
                                text: "等待数据库…"
                                draw_text.color: theme.color_label_inner_inactive
                                draw_text.text_style.font_size: 9
                            }
                            vocab_toolbar := View{
                                width: Fill height: Fit
                                flow: Right spacing: 8
                                align: Align{y: 0.5}
                                vocab_search := TextInput{
                                    width: Fill height: 38
                                    empty_text: "搜索单词、原形或释义"
                                }
                                vocab_filter_all := RadioButtonTabFlat{text: "全部"}
                                vocab_filter_learning := RadioButtonTabFlat{text: "学习中"}
                                vocab_filter_mastered := RadioButtonTabFlat{text: "已掌握"}
                            }
                            vocab_filter_hint := Label{
                                width: Fill height: Fit
                                text: "当前筛选：全部"
                                draw_text.color: theme.color_highlight
                                draw_text.text_style.font_size: 8.5
                            }
                            vocab_review_panel := RoundedView{
                                visible: false
                                width: Fill height: Fit
                                flow: Down spacing: 9 padding: 14
                                new_batch: true
                                draw_bg.color: theme.color_bg_highlight_inline
                                draw_bg.border_radius: 10.0
                                vocab_review_top := View{
                                    width: Fill height: Fit
                                    flow: Right spacing: 10
                                    align: Align{y: 0.5}
                                    vocab_review_progress := Label{
                                        width: Fill height: Fit
                                        text: ""
                                        draw_text.color: theme.color_highlight
                                        draw_text.text_style.font_size: 9
                                    }
                                    vocab_review_close := ButtonFlatter{text: "结束复习"}
                                }
                                vocab_review_word := Label{
                                    width: Fill height: Fit
                                    text: ""
                                    draw_text.text_style.font_size: 22
                                }
                                vocab_review_context := Label{
                                    width: Fill height: Fit
                                    text: ""
                                    draw_text.color: theme.color_label_inner_inactive
                                    draw_text.text_style.font_size: 10
                                }
                                vocab_review_gloss := Label{
                                    visible: false
                                    width: Fill height: Fit
                                    text: ""
                                    draw_text.color: theme.color_text
                                    draw_text.text_style.font_size: 13
                                }
                                vocab_review_actions := View{
                                    width: Fill height: Fit
                                    flow: Right spacing: 8
                                    align: Align{y: 0.5}
                                    vocab_review_speak := ButtonFlatter{text: "发音"}
                                    vocab_review_reveal := ButtonFlat{text: "显示释义"}
                                    vocab_review_again := ButtonFlat{
                                        visible: false
                                        text: "继续学习"
                                    }
                                    vocab_review_known := Button{
                                        visible: false
                                        text: "认识了"
                                    }
                                }
                            }
                            vocab_status_hint := Label{
                                width: Fill height: Fit
                                text: ""
                                draw_text.color: theme.color_label_inner_inactive
                                draw_text.text_style.font_size: 9
                            }
                            vocab_empty := View{
                                width: Fill height: 120
                                visible: true
                                flow: Down spacing: 7
                                align: Center
                                vocab_empty_title := Label{
                                    width: Fit height: Fit
                                    text: "生词本为空"
                                    draw_text.text_style.font_size: 13
                                }
                                vocab_empty_hint := Label{
                                    width: Fit height: Fit
                                    text: "去阅读页选择一个英文单词加入生词本。"
                                    draw_text.color: theme.color_label_inner_inactive
                                    draw_text.text_style.font_size: 9
                                }
                            }
                            vocab_list := mod.widgets.VocabList{visible: false}
                        }

                        page_statistics := ScrollYView{
                            width: Fill height: Fill
                            flow: Down padding: 24 spacing: 14
                            statistics_header := View{
                                width: Fill height: Fit
                                flow: Right spacing: 10
                                align: Align{y: 0.5}
                                statistics_title := Label{
                                    width: Fill height: Fit
                                    text: "阅读统计"
                                    draw_text.text_style.font_size: 18
                                }
                                statistics_refresh := ButtonFlat{text: "刷新"}
                            }
                            statistics_status := Label{
                                width: Fill height: Fit
                                text: "正在读取统计数据…"
                                draw_text.color: theme.color_label_inner_inactive
                                draw_text.text_style.font_size: 9
                            }
                            statistics_summary := View{
                                width: Fill height: Fit
                                flow: Right spacing: 10
                                stat_today_card := RoundedView{
                                    width: Fill height: 88
                                    new_batch: true
                                    flow: Down spacing: 5 padding: 12
                                    draw_bg.color: theme.color_bg_container
                                    draw_bg.border_radius: 9.0
                                    stat_today_title := Label{text: "今日阅读" draw_text.color: theme.color_label_inner_inactive draw_text.text_style.font_size: 9}
                                    stat_today := Label{text: "0 分钟" draw_text.text_style.font_size: 18}
                                }
                                stat_total_card := RoundedView{
                                    width: Fill height: 88
                                    new_batch: true
                                    flow: Down spacing: 5 padding: 12
                                    draw_bg.color: theme.color_bg_container
                                    draw_bg.border_radius: 9.0
                                    stat_total_title := Label{text: "累计阅读" draw_text.color: theme.color_label_inner_inactive draw_text.text_style.font_size: 9}
                                    stat_total := Label{text: "0 分钟" draw_text.text_style.font_size: 18}
                                }
                                stat_days_card := RoundedView{
                                    width: Fill height: 88
                                    new_batch: true
                                    flow: Down spacing: 5 padding: 12
                                    draw_bg.color: theme.color_bg_container
                                    draw_bg.border_radius: 9.0
                                    stat_days_title := Label{text: "阅读天数" draw_text.color: theme.color_label_inner_inactive draw_text.text_style.font_size: 9}
                                    stat_days := Label{text: "0 天" draw_text.text_style.font_size: 18}
                                }
                                stat_books_card := RoundedView{
                                    width: Fill height: 88
                                    new_batch: true
                                    flow: Down spacing: 5 padding: 12
                                    draw_bg.color: theme.color_bg_container
                                    draw_bg.border_radius: 9.0
                                    stat_books_title := Label{text: "读完书籍" draw_text.color: theme.color_label_inner_inactive draw_text.text_style.font_size: 9}
                                    stat_books := Label{text: "0 / 0" draw_text.text_style.font_size: 18}
                                }
                            }
                            reading_chart_card := RoundedView{
                                width: Fill height: Fit
                                new_batch: true
                                flow: Down spacing: 10 padding: 14
                                draw_bg.color: theme.color_bg_container
                                draw_bg.border_radius: 10.0
                                reading_chart_title := Label{text: "最近 7 天阅读时长" draw_text.text_style.font_size: 13}
                                reading_chart_hint := Label{
                                    width: Fill height: Fit
                                    text: "每天 0 分钟"
                                    draw_text.color: theme.color_label_inner_inactive
                                    draw_text.text_style.font_size: 9
                                }
                                reading_chart := mod.widgets.StatsBars{}
                                reading_chart_labels := View{
                                    width: Fill height: Fit
                                    flow: Right spacing: 4
                                    stat_day_0 := Label{width: Fill text: "" draw_text.color: theme.color_label_inner_inactive draw_text.text_style.font_size: 8}
                                    stat_day_1 := Label{width: Fill text: "" draw_text.color: theme.color_label_inner_inactive draw_text.text_style.font_size: 8}
                                    stat_day_2 := Label{width: Fill text: "" draw_text.color: theme.color_label_inner_inactive draw_text.text_style.font_size: 8}
                                    stat_day_3 := Label{width: Fill text: "" draw_text.color: theme.color_label_inner_inactive draw_text.text_style.font_size: 8}
                                    stat_day_4 := Label{width: Fill text: "" draw_text.color: theme.color_label_inner_inactive draw_text.text_style.font_size: 8}
                                    stat_day_5 := Label{width: Fill text: "" draw_text.color: theme.color_label_inner_inactive draw_text.text_style.font_size: 8}
                                    stat_day_6 := Label{width: Fill text: "今天" draw_text.color: theme.color_label_inner_inactive draw_text.text_style.font_size: 8}
                                }
                            }
                            vocab_chart_card := RoundedView{
                                width: Fill height: Fit
                                new_batch: true
                                flow: Down spacing: 10 padding: 14
                                draw_bg.color: theme.color_bg_container
                                draw_bg.border_radius: 10.0
                                vocab_chart_title := Label{text: "词汇学习" draw_text.text_style.font_size: 13}
                                vocab_chart_hint := Label{
                                    width: Fill height: Fit
                                    text: "生词 0 · 累计复习 0 次"
                                    draw_text.color: theme.color_label_inner_inactive
                                    draw_text.text_style.font_size: 9
                                }
                                vocab_chart := mod.widgets.StatsBars{
                                    height: 150
                                    draw_bar.color: theme.color_chart_vocab
                                }
                                vocab_chart_labels := View{
                                    width: Fill height: Fit
                                    flow: Right spacing: 4
                                    vocab_chart_learning := Label{width: Fill text: "学习中" draw_text.color: theme.color_label_inner_inactive draw_text.text_style.font_size: 8}
                                    vocab_chart_mastered := Label{width: Fill text: "已掌握" draw_text.color: theme.color_label_inner_inactive draw_text.text_style.font_size: 8}
                                    vocab_chart_reviews := Label{width: Fill text: "累计复习" draw_text.color: theme.color_label_inner_inactive draw_text.text_style.font_size: 8}
                                }
                            }
                        }

                        page_chat := View{
                            width: Fill height: Fill
                            flow: Down
                            chat_header := View{
                                width: Fill height: Fit
                                flow: Down spacing: 4
                                padding: Inset{left: 24 right: 24 top: 20 bottom: 8}
                                chat_title := Label{ text: "AI 对话" draw_text.text_style.font_size: 18 }
                                chat_status := Label{
                                    width: Fill
                                    text: "正在读取 AI 设置…"
                                    draw_text.color: theme.color_label_inner_inactive
                                    draw_text.text_style.font_size: 9
                                }
                                chat_skill_row := View{
                                    width: Fill height: Fit
                                    flow: Right spacing: 8
                                    align: Align{y: 0.5}
                                    chat_skill_status := Label{
                                        width: Fill height: Fit
                                        text: "未选择技能模板"
                                        draw_text.color: theme.color_highlight
                                        draw_text.text_style.font_size: 9
                                    }
                                    chat_skill_clear := ButtonFlatter{
                                        visible: false
                                        text: "取消模板"
                                    }
                                }
                            }
                            chat_list := mod.widgets.ChatList{}
                            chat_composer := View{
                                width: Fill height: Fit
                                flow: Right spacing: 8
                                padding: Inset{left: 20 right: 20 top: 8 bottom: 18}
                                align: Align{y: 0.5}
                                chat_input := TextInput{
                                    width: Fill height: 42
                                    empty_text: "输入问题，按回车发送"
                                }
                                chat_send := Button{width: 72 height: 42 text: "发送"}
                                chat_cancel := ButtonFlat{
                                    visible: false
                                    width: 72 height: 42 text: "停止"
                                }
                                chat_clear := ButtonFlatter{width: 72 height: 42 text: "新对话"}
                            }
                        }

                        page_skills := View{
                            width: Fill height: Fill
                            flow: Down
                            skills_header := View{
                                width: Fill height: Fit
                                flow: Right spacing: 10
                                padding: Inset{left: 24 right: 24 top: 20 bottom: 8}
                                align: Align{y: 0.5}
                                skills_heading := View{
                                    width: Fill height: Fit
                                    flow: Down spacing: 4
                                    skills_title := Label{
                                        text: "AI 技能"
                                        draw_text.text_style.font_size: 18
                                    }
                                    skills_status := Label{
                                        width: Fill height: Fit
                                        text: "正在读取技能…"
                                        draw_text.color: theme.color_label_inner_inactive
                                        draw_text.text_style.font_size: 9
                                    }
                                }
                                skill_new := Button{text: "新建模板"}
                            }
                            skills_body := View{
                                width: Fill height: Fill
                                flow: Right spacing: 14
                                padding: Inset{left: 24 right: 24 bottom: 20}
                                skills_left := RoundedView{
                                    width: 330 height: Fill
                                    padding: 10
                                    new_batch: true
                                    draw_bg.color: theme.color_bg_app
                                    draw_bg.border_radius: 9.0
                                    skill_list := mod.widgets.SkillList{}
                                }
                                skill_editor := RoundedView{
                                    width: Fill height: Fill
                                    flow: Down spacing: 9 padding: 14
                                    new_batch: true
                                    draw_bg.color: theme.color_bg_container
                                    draw_bg.border_radius: 9.0
                                    skill_editor_title := Label{
                                        width: Fill height: Fit
                                        text: "新建技能"
                                        draw_text.text_style.font_size: 14
                                    }
                                    skill_name_title := Label{
                                        text: "名称"
                                        draw_text.color: theme.color_label_inner_inactive
                                        draw_text.text_style.font_size: 9
                                    }
                                    skill_name_input := TextInput{
                                        width: Fill height: 40
                                        empty_text: "例如：逐句精读"
                                    }
                                    skill_active_edit := CheckBox{
                                        text: "启用这个技能"
                                        active: true
                                    }
                                    skill_prompt_title := Label{
                                        text: "Prompt 模板（Markdown）"
                                        draw_text.color: theme.color_label_inner_inactive
                                        draw_text.text_style.font_size: 9
                                    }
                                    skill_content_input := TextInput{
                                        width: Fill height: Fill
                                        flow: Flow.Right{wrap: true}
                                        is_multiline: true
                                        submit_on_enter: false
                                        empty_text: "写下模型执行此任务时应遵循的目标、步骤和输出格式…"
                                        scroll_bar: ScrollBar{}
                                    }
                                    skill_editor_hint := Label{
                                        width: Fill height: Fit
                                        text: "自定义技能可用于下一次 AI 提问；启用的系统技能会自动作为全局系统提示词。"
                                        draw_text.color: theme.color_label_inner_inactive
                                        draw_text.text_style.font_size: 8.5
                                    }
                                    skill_editor_actions := View{
                                        width: Fill height: Fit
                                        flow: Right spacing: 8
                                        align: Align{x: 1.0 y: 0.5}
                                        skill_cancel := ButtonFlat{text: "清空"}
                                        skill_save := Button{text: "保存"}
                                    }
                                }
                            }
                        }

                            page_settings := ScrollYView{
                            width: Fill height: Fill
                            flow: Down padding: 24 spacing: 12
                            settings_title := Label{ text: "设置" draw_text.text_style.font_size: 18 }
                            settings_reading_heading := Label{
                                text: "阅读"
                                draw_text.text_style.font_size: 13
                                draw_text.color: theme.color_label_inner_inactive
                            }
                            language_row := View{
                                width: Fill height: Fit
                                flow: Right spacing: 12
                                align: Align{y: 0.5}
                                language_label := Label{text: "界面语言"}
                                language_chinese := RadioButtonTabFlat{text: "中文"}
                                language_english := RadioButtonTabFlat{text: "英文"}
                            }
                            theme_row := View{
                                width: Fill height: Fit
                                flow: Right spacing: 12
                                align: Align{y: 0.5}
                                theme_label := Label{text: "外观主题"}
                                theme_dark := RadioButtonTabFlat{text: "深色"}
                                theme_light := RadioButtonTabFlat{text: "浅色"}
                            }
                            font_family_row := View{
                                width: Fill height: Fit
                                flow: Right spacing: 12
                                align: Align{y: 0.5}
                                font_family_label := Label{text: "正文字体"}
                                reader_font_standard := RadioButtonTabFlat{text: "标准"}
                                reader_font_monospace := RadioButtonTabFlat{text: "等宽"}
                            }
                            line_spacing_row := View{
                                width: Fill height: Fit
                                flow: Right spacing: 12
                                align: Align{y: 0.5}
                                line_spacing_label := Label{text: "正文行距"}
                                reader_spacing_default := RadioButtonTabFlat{text: "默认"}
                                reader_spacing_comfortable := RadioButtonTabFlat{text: "舒适"}
                                reader_spacing_relaxed := RadioButtonTabFlat{text: "宽松"}
                            }
                            font_row := View{
                                width: Fill height: Fit
                                spacing: 12
                                align: Align{ y: 0.5 }
                                font_size_title := Label{ text: "正文字号" }
                                font_dec := ButtonFlat{ text: "A-" }
                                font_size_label := Label{ text: "默认" }
                                font_inc := ButtonFlat{ text: "A+" }
                            }
                            gloss_enable_row := View{
                                width: Fill height: Fit
                                spacing: 12
                                align: Align{ y: 0.5 }
                                gloss_enable := CheckBox{ text: "生词注音（关闭则原样显示文章）" }
                            }
                            gloss_row := View{
                                width: Fill height: Fit
                                spacing: 12
                                align: Align{ y: 0.5 }
                                gloss_size_title := Label{ text: "注音字号" }
                                gloss_dec := ButtonFlat{ text: "A-" }
                                gloss_size_label := Label{ text: "7.5" }
                                gloss_inc := ButtonFlat{ text: "A+" }
                            }
                            gloss_color_row := View{
                                width: Fill height: Fit
                                spacing: 12
                                align: Align{ y: 0.5 }
                                gloss_color_title := Label{ text: "注音字色" }
                                gc0 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "青" } }
                                gc1 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "蓝" } }
                                gc2 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "橙" } }
                                gc3 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "灰" } }
                            }
                            gloss_bg_row := View{
                                width: Fill height: Fit
                                spacing: 12
                                align: Align{ y: 0.5 }
                                gloss_bg_title := Label{ text: "注音背景" }
                                gb0 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "无" } }
                                gb1 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "深灰" } }
                                gb2 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "深青" } }
                                gb3 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "深蓝" } }
                            }
                            word_color_row := View{
                                width: Fill height: Fit
                                spacing: 12
                                align: Align{ y: 0.5 }
                                word_color_title := Label{ text: "词字色" }
                                wc0 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "默认" } }
                                wc1 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "青" } }
                                wc2 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "蓝" } }
                                wc3 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "橙" } }
                            }
                            word_bg_row := View{
                                width: Fill height: Fit
                                spacing: 12
                                align: Align{ y: 0.5 }
                                word_bg_title := Label{ text: "词背景" }
                                wb0 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "无" } }
                                wb1 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "深灰" } }
                                wb2 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "深青" } }
                                wb3 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "深蓝" } }
                            }
                            settings_text_heading := Label{
                                text: "文本"
                                draw_text.text_style.font_size: 13
                                draw_text.color: theme.color_label_inner_inactive
                            }
                            sel_color_row := View{
                                width: Fill height: Fit
                                spacing: 12
                                align: Align{ y: 0.5 }
                                selection_color_title := Label{ text: "选中颜色" }
                                sc0 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "青" } }
                                sc1 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "蓝" } }
                                sc2 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "橙" } }
                                sc3 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "黄" } }
                            }
                            hl_color_row := View{
                                width: Fill height: Fit
                                spacing: 12
                                align: Align{ y: 0.5 }
                                highlight_color_title := Label{ text: "高亮颜色" }
                                hc0 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "黄" } }
                                hc1 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "青" } }
                                hc2 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "蓝" } }
                                hc3 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: theme.color_bg_container b := ButtonFlat{ text: "橙" } }
                            }
                            settings_hint := Label{
                                width: Fill
                                text: "阅读设置与外观主题更改后自动保存在本机。"
                                draw_text.color: theme.color_label_inner_inactive
                                draw_text.text_style.font_size: 9
                            }
                            ai_settings_heading := Label{
                                text: "AI（OpenAI 兼容接口）"
                                draw_text.text_style.font_size: 13
                                draw_text.color: theme.color_label_inner_inactive
                            }
                            ai_endpoint := TextInput{
                                width: Fill height: 38
                                empty_text: "API 根地址或完整 /chat/completions 地址"
                            }
                            ai_model := TextInput{
                                width: Fill height: 38
                                empty_text: "模型名称（必填）"
                            }
                            ai_api_key := TextInput{
                                width: Fill height: 38
                                is_password: true
                                empty_text: "API Key（本地免鉴权服务可留空）"
                            }
                            ai_settings_actions := View{
                                width: Fill height: Fit
                                spacing: 12
                                ai_save := Button{ text: "保存 AI 设置" }
                                reload_button := Button{ text: "重载示例章节" }
                            }
                            ai_settings_status := Label{
                                width: Fill height: Fit
                                text: "密钥保存在本机应用数据库中。"
                                draw_text.color: theme.color_label_inner_inactive
                                draw_text.text_style.font_size: 9
                            }
                            local_ai_section := View{
                                visible: false
                                width: Fill height: Fit
                                flow: Down spacing: 9
                                local_ai_heading := Label{
                                    text: "本地 AI（桌面 llama.cpp）"
                                    draw_text.text_style.font_size: 13
                                    draw_text.color: theme.color_label_inner_inactive
                                }
                                local_ai_description := Label{
                                    width: Fill height: Fit
                                    text: "填写已安装的 llama-server 与 GGUF 文件路径。服务只监听 127.0.0.1；启动后聊天会自动切换到本地模型。"
                                    draw_text.color: theme.color_label_inner_inactive
                                    draw_text.text_style.font_size: 9
                                }
                                local_ai_binary := TextInput{
                                    width: Fill height: 38
                                    empty_text: "llama-server 路径，或 PATH 中的 llama-server"
                                }
                                local_ai_model_path := TextInput{
                                    width: Fill height: 38
                                    empty_text: "GGUF 模型完整路径"
                                }
                                local_ai_numbers := View{
                                    width: Fill height: Fit
                                    flow: Right spacing: 10
                                    align: Align{y: 0.5}
                                    local_ai_port_label := Label{text: "端口"}
                                    local_ai_port := TextInput{
                                        width: 72 height: 38
                                        is_numeric_only: true
                                        text: "8080"
                                    }
                                    local_ai_context_label := Label{text: "上下文"}
                                    local_ai_context := TextInput{
                                        width: 82 height: 38
                                        is_numeric_only: true
                                        text: "4096"
                                    }
                                    local_ai_threads_label := Label{text: "线程（0=自动）"}
                                    local_ai_threads := TextInput{
                                        width: 62 height: 38
                                        is_numeric_only: true
                                        text: "0"
                                    }
                                    local_ai_gpu_layers_label := Label{text: "GPU 层"}
                                    local_ai_gpu_layers := TextInput{
                                        width: 62 height: 38
                                        is_numeric_only: true
                                        text: "0"
                                    }
                                }
                                local_ai_auto_start := CheckBox{
                                    text: "应用启动时自动加载本地模型"
                                }
                                local_ai_actions := View{
                                    width: Fill height: Fit
                                    flow: Right spacing: 10
                                    local_ai_start := Button{text: "启动并使用"}
                                    local_ai_save := ButtonFlat{text: "仅保存设置"}
                                    local_ai_stop := ButtonFlatter{
                                        visible: false
                                        text: "停止本地服务"
                                    }
                                }
                                local_ai_status := Label{
                                    width: Fill height: Fit
                                    text: "本地服务未启动。macOS 可通过 brew install llama.cpp 安装。"
                                    draw_text.color: theme.color_label_inner_inactive
                                    draw_text.text_style.font_size: 9
                                }
                            }
                            }
                        }
                        mobile_nav := SolidView{
                            visible: false
                            width: Fill height: Fit
                            new_batch: true
                            flow: Down
                            padding: Inset{left: 4 right: 4 top: 5 bottom: 5}
                            draw_bg.color: theme.color_bg_app
                            mobile_nav_scroll := ScrollXView{
                                width: Fill height: 44
                                flow: Right spacing: 4
                                mobile_nav_library := ButtonFlat{width: 58 text: "书库"}
                                mobile_nav_reader := ButtonFlat{width: 58 text: "阅读"}
                                mobile_nav_chat := ButtonFlat{width: 66 text: "AI 对话"}
                                mobile_nav_vocab := ButtonFlat{width: 66 text: "生词本"}
                                mobile_nav_statistics := ButtonFlat{width: 58 text: "统计"}
                                mobile_nav_skills := ButtonFlat{width: 66 text: "AI 技能"}
                                mobile_nav_settings := ButtonFlat{width: 58 text: "设置"}
                            }
                        }
                    }
                    }
                    library_tag_modal := Modal{
                content +: {
                    width: 360 height: Fit
                    tag_modal_card := RoundedView{
                        width: Fill height: Fit
                        new_batch: true
                        flow: Down spacing: 12 padding: 22
                        draw_bg.color: theme.color_bg_container
                        draw_bg.border_radius: 10.0
                        library_tag_modal_title := Label{
                            width: Fill
                            text: "编辑标签"
                            draw_text.text_style.font_size: 15
                        }
                        library_tag_input := TextInput{
                            width: Fill height: 40
                            empty_text: "例如：小说, 技术, 待读"
                        }
                        library_tag_hint := Label{
                            width: Fill height: Fit
                            text: "使用逗号分隔；每本书最多 8 个标签，每个最多 24 个字符。"
                            draw_text.color: theme.color_label_inner_inactive
                            draw_text.text_style.font_size: 9
                        }
                        library_tag_status := Label{
                            width: Fill height: Fit
                            text: ""
                            draw_text.color: theme.color_highlight
                            draw_text.text_style.font_size: 9
                        }
                        library_tag_actions := View{
                            width: Fill height: Fit
                            flow: Right spacing: 8
                            align: Align{x: 1.0 y: 0.5}
                            library_tag_cancel := ButtonFlat{text: "取消"}
                            library_tag_save := Button{text: "保存"}
                        }
                    }
                }
                    }
                    library_delete_modal := Modal{
                        content +: {
                            width: 380 height: Fit
                            library_delete_card := RoundedView{
                                width: Fill height: Fit
                                new_batch: true
                                flow: Down spacing: 14 padding: 22
                                draw_bg.color: theme.color_bg_container
                                draw_bg.border_radius: 10.0
                                library_delete_modal_title := Label{
                                    width: Fill
                                    text: "删除书籍"
                                    draw_text.text_style.font_size: 15
                                }
                                library_delete_modal_message := Label{
                                    width: Fill height: Fit
                                    text: ""
                                    draw_text.color: theme.color_label_inner_inactive
                                    draw_text.text_style.font_size: 10
                                }
                                library_delete_actions := View{
                                    width: Fill height: Fit
                                    flow: Right spacing: 8
                                    align: Align{x: 1.0 y: 0.5}
                                    library_delete_cancel := ButtonFlat{text: "取消"}
                                    library_delete_confirm := Button{text: "确认删除"}
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// 轻量 GPU 柱状图。矩形由 Makepad 的 DrawColor shader 批量绘制，
/// 数据和坐标仍在 Rust 侧保持可测试、可更新。
#[derive(Script, ScriptHook, Widget)]
struct StatsBars {
    #[uid]
    uid: WidgetUid,
    #[source]
    source: ScriptObjectRef,
    #[walk]
    walk: Walk,
    #[redraw]
    #[live]
    draw_bg: DrawColor,
    #[live]
    draw_grid: DrawColor,
    #[live]
    draw_bar: DrawColor,
    #[rust]
    values: Vec<f64>,
}

impl StatsBars {
    fn set_values(&mut self, cx: &mut Cx, values: Vec<f64>) {
        self.values = values;
        self.draw_bg.redraw(cx);
    }
}

impl Widget for StatsBars {
    fn handle_event(&mut self, _cx: &mut Cx, _event: &Event, _scope: &mut Scope) {}

    fn draw_walk(&mut self, cx: &mut Cx2d, _scope: &mut Scope, walk: Walk) -> DrawStep {
        let rect = cx.walk_turtle(walk);
        self.draw_bg.draw_abs(cx, rect);
        let padding = 12.0;
        let plot = Rect {
            pos: rect.pos + dvec2(padding, padding),
            size: rect.size - dvec2(padding * 2.0, padding * 2.0),
        };
        if plot.size.x <= 1.0 || plot.size.y <= 1.0 {
            return DrawStep::done();
        }
        for fraction in [0.25, 0.5, 0.75, 1.0] {
            self.draw_grid.draw_abs(
                cx,
                Rect {
                    pos: plot.pos + dvec2(0.0, plot.size.y * (1.0 - fraction)),
                    size: dvec2(plot.size.x, 1.0),
                },
            );
        }
        if self.values.is_empty() {
            return DrawStep::done();
        }
        let maximum = self.values.iter().copied().fold(0.0_f64, f64::max).max(1.0);
        let slot = plot.size.x / self.values.len() as f64;
        let bar_width = (slot * 0.56).max(3.0);
        for (index, value) in self.values.iter().enumerate() {
            let height =
                (plot.size.y * (*value / maximum)).max(if *value > 0.0 { 2.0 } else { 0.0 });
            self.draw_bar.draw_abs(
                cx,
                Rect {
                    pos: plot.pos
                        + dvec2(
                            slot * index as f64 + (slot - bar_width) * 0.5,
                            plot.size.y - height,
                        ),
                    size: dvec2(bar_width, height),
                },
            );
        }
        DrawStep::done()
    }
}

#[derive(Script, ScriptHook, Widget)]
struct ChapterList {
    #[deref]
    view: View,
    #[live]
    font_default_size: f32,
    #[live]
    font_standard_normal: TextStyle,
    #[live]
    font_standard_italic: TextStyle,
    #[live]
    font_standard_bold: TextStyle,
    #[live]
    font_standard_bold_italic: TextStyle,
    #[live]
    font_monospace: TextStyle,
    /// 选区自跟踪（PortalList 未暴露区间 API，用公开的命中接口镜像；
    /// 仅镜像记录，绘制仍由原生选区完成）。
    #[rust]
    sel_anchor: Option<(usize, usize)>,
    #[rust]
    selecting: bool,
    #[rust]
    last_abs: Option<DVec2>,
    #[rust]
    touch_selection_uid: Option<u64>,
    #[rust]
    compact_layout: bool,
}

impl ChapterList {
    fn set_compact_layout(&mut self, cx: &mut Cx, compact_layout: bool) {
        if self.compact_layout == compact_layout {
            return;
        }
        self.compact_layout = compact_layout;
        let list = self.view.portal_list(cx, ids!(list));
        if let Some(mut inner) = list.borrow_mut() {
            inner.selectable = !compact_layout;
            inner.clear_selection(cx);
        }
        self.selecting = false;
        self.touch_selection_uid = None;
        *SEL_RANGE.write().unwrap() = None;
        SELECTED_TEXT.write().unwrap().clear();
        Cx::post_action(SelAction::Hide);
        self.view.redraw(cx);
    }

    /// 命中可见块的 (块id, 字符下标)。点位在视口外或块间隙时吸附到
    /// 最近边界，与 PortalList 原生选择规则保持一致。
    fn hit_block_char(&self, cx: &Cx, abs: DVec2) -> Option<(usize, usize)> {
        let list = self.view.portal_list(cx, ids!(list));
        let first = list.first_id();
        let vis = list.visible_items() + 1;
        let list_rect = list.area().rect(cx);
        let mut visible = Vec::new();
        for id in first..first + vis {
            let Some((_, widget)) = list.get_item(id) else {
                continue;
            };
            let rect = widget.area().rect(cx);
            if rect.size.y > 0.0 {
                visible.push((id, widget, rect));
            }
        }
        visible.sort_by_key(|(id, _, _)| *id);
        let (first_id, _, _) = visible.first()?;
        let (last_id, last_widget, _) = visible.last()?;
        if abs.y < list_rect.pos.y {
            return Some((*first_id, 0));
        }
        if abs.y > list_rect.pos.y + list_rect.size.y {
            return Some((*last_id, last_widget.selection_text_len()));
        }
        for (id, widget, rect) in &visible {
            if rect.contains(abs) {
                return widget
                    .selection_point_to_char_index(cx, abs)
                    .map(|char_idx| (*id, char_idx));
            }
        }
        if let Some((id, widget, _)) = visible
            .iter()
            .rev()
            .find(|(_, _, rect)| rect.pos.y + rect.size.y <= abs.y)
        {
            return Some((*id, widget.selection_text_len()));
        }
        Some((*first_id, 0))
    }

    fn set_cursor(&self, cursor: (usize, usize)) {
        let Some(anchor) = self.sel_anchor else {
            return;
        };
        let range = if cursor < anchor {
            (cursor, anchor)
        } else {
            (anchor, cursor)
        };
        *SEL_RANGE.write().unwrap() = Some(range);
    }

    /// PortalList 会复用/回收离屏 item，因此每次滚动与重绘都要把全局区间
    /// 重新投影到当前可见 widget，而不能依赖上一次的原生选择状态。
    fn apply_visible_selection(&self, cx: &mut Cx) {
        let range = *SEL_RANGE.read().unwrap();
        let list = self.view.portal_list(cx, ids!(list));
        let first = list.first_id();
        let vis = list.visible_items() + 1;
        for id in first..first + vis {
            let Some((_, widget)) = list.get_item(id) else {
                continue;
            };
            match range {
                None => widget.selection_clear(),
                Some((start, end)) if id < start.0 || id > end.0 => widget.selection_clear(),
                Some((start, end)) if id == start.0 && id == end.0 => {
                    widget.selection_set(start.1, end.1)
                }
                Some((start, _)) if id == start.0 => {
                    widget.selection_set(start.1, widget.selection_text_len())
                }
                Some((_, end)) if id == end.0 => widget.selection_set(0, end.1),
                Some(_) => widget.selection_select_all(),
            }
            widget.redraw(cx);
        }
        list.redraw(cx);
    }

    fn extend_after_scroll(&mut self, cx: &mut Cx) {
        let Some(abs) = self.last_abs else { return };
        if let Some(hit) = self.hit_block_char(cx, abs) {
            self.set_cursor(hit);
            self.apply_visible_selection(cx);
            return;
        }

        // 新的可见项会在下一帧才绑定；先按视口比例估算块，再在后续 MouseMove
        // 中用精确字形命中修正。这样按住鼠标滚轮时选区会立即跨块延伸。
        let list = self.view.portal_list(cx, ids!(list));
        let first = list.first_id();
        let visible = list.visible_items().max(1);
        let rect = list.area().rect(cx);
        let fraction = ((abs.y - rect.pos.y) / rect.size.y.max(1.0)).clamp(0.0, 0.999);
        let max_id = PARAGRAPHS.read().unwrap().len().saturating_sub(1);
        let id = (first + (fraction * visible as f64) as usize).min(max_id);
        let len = PARAGRAPHS
            .read()
            .unwrap()
            .get(id)
            .map(|block| {
                readwise_core::vocab::annotate::rendered_text(block)
                    .chars()
                    .count()
            })
            .unwrap_or(0);
        let char_idx = self
            .sel_anchor
            .map(|anchor| if id >= anchor.0 { len } else { 0 })
            .unwrap_or(0);
        self.set_cursor((id, char_idx));
        self.apply_visible_selection(cx);
    }

    fn finish_selection(&mut self) {
        self.selecting = false;
        self.touch_selection_uid = None;
        let range = *SEL_RANGE.read().unwrap();
        let non_empty = range.map(|(a, b)| a != b).unwrap_or(false);
        if let Some(range) = range.filter(|(a, b)| a != b) {
            *SELECTED_TEXT.write().unwrap() =
                readwise_core::vocab::annotate::rendered_text_for_range(
                    &PARAGRAPHS.read().unwrap(),
                    range,
                );
        }
        if std::env::var("SEL_DEBUG").is_ok() {
            eprintln!("[SEL] up: range={:?} show={}", range, non_empty);
        }
        Cx::post_action(if non_empty {
            SelAction::Show
        } else {
            SelAction::Hide
        });
    }

    fn track_selection(&mut self, cx: &mut Cx, event: &Event) {
        match event {
            Event::MouseDown(e) => {
                let list = self.view.portal_list(cx, ids!(list));
                if !list.area().rect(cx).contains(e.abs) {
                    return;
                }
                if std::env::var("SEL_DEBUG").is_ok() {
                    eprintln!("[SEL] down abs={:?}", e.abs);
                }
                *SEL_RANGE.write().unwrap() = None;
                SELECTED_TEXT.write().unwrap().clear();
                self.selecting = false;
                self.sel_anchor = None;
                self.last_abs = Some(e.abs);
                Cx::post_action(SelAction::Hide);
                let first = list.first_id();
                let vis = list.visible_items() + 1;
                let on_interactive = (first..first + vis).any(|id| {
                    list.get_item(id)
                        .map(|(_, w)| w.find_interactive_widget_from_point(cx, e.abs).is_some())
                        .unwrap_or(false)
                });
                if !on_interactive {
                    if let Some(hit) = self.hit_block_char(cx, e.abs) {
                        self.sel_anchor = Some(hit);
                        self.selecting = true;
                    }
                }
                if std::env::var("SEL_DEBUG").is_ok() {
                    eprintln!(
                        "[SEL] down: interactive={} anchor={:?}",
                        on_interactive, self.sel_anchor
                    );
                }
            }
            Event::MouseMove(e) => {
                if self.selecting {
                    self.last_abs = Some(e.abs);
                    if let Some(cur) = self.hit_block_char(cx, e.abs) {
                        self.set_cursor(cur);
                        self.apply_visible_selection(cx);
                    }
                }
            }
            Event::Scroll(_) if self.selecting => self.extend_after_scroll(cx),
            Event::MouseUp(_) if self.selecting => {
                self.finish_selection();
            }
            Event::LongPress(event) if self.compact_layout => {
                let list = self.view.portal_list(cx, ids!(list));
                if !list.area().rect(cx).contains(event.abs) {
                    return;
                }
                let Some((block_id, char_index)) = self.hit_block_char(cx, event.abs) else {
                    return;
                };
                let Some((start, end)) =
                    PARAGRAPHS
                        .read()
                        .unwrap()
                        .get(block_id)
                        .and_then(|paragraph| {
                            let text = readwise_core::vocab::annotate::rendered_text(paragraph);
                            word_range_at(&text, char_index)
                        })
                else {
                    return;
                };
                self.sel_anchor = Some((block_id, start));
                self.selecting = true;
                self.touch_selection_uid = Some(event.uid);
                self.last_abs = Some(event.abs);
                *SEL_RANGE.write().unwrap() = Some(((block_id, start), (block_id, end)));
                SELECTED_TEXT.write().unwrap().clear();
                Cx::post_action(SelAction::Hide);
                self.apply_visible_selection(cx);
            }
            Event::TouchUpdate(event) if self.touch_selection_uid.is_some() => {
                let uid = self.touch_selection_uid.unwrap();
                let Some(touch) = event.touches.iter().find(|touch| touch.uid == uid) else {
                    return;
                };
                match touch.state {
                    TouchState::Move => {
                        self.last_abs = Some(touch.abs);
                        if let Some(cursor) = self.hit_block_char(cx, touch.abs) {
                            self.set_cursor(cursor);
                            self.apply_visible_selection(cx);
                        }
                    }
                    TouchState::Stop => self.finish_selection(),
                    TouchState::Start | TouchState::Stable => {}
                }
            }
            _ => {}
        }
    }
}

impl Widget for ChapterList {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let paragraphs = PARAGRAPHS.read().unwrap();
        let reader_font = *READER_FONT.read().unwrap();
        let reader_spacing = *READER_SPACING.read().unwrap();
        let reader_font_size = READER_FONT_SIZE
            .read()
            .unwrap()
            .unwrap_or(self.font_default_size);
        let selection_color = Vec4f::from_u32(match *SELECTION_SLOT.read().unwrap() {
            1 => 0x4a90_d988,
            2 => 0xe887_5a88,
            3 => 0xd9c9_6a88,
            _ => 0x7fb2_a688,
        });
        let selection_range = *SEL_RANGE.read().unwrap();
        let normal_family = if reader_font == ReaderFont::Monospace {
            self.font_monospace.font_family.clone()
        } else {
            self.font_standard_normal.font_family.clone()
        };
        let italic_family = if reader_font == ReaderFont::Monospace {
            self.font_monospace.font_family.clone()
        } else {
            self.font_standard_italic.font_family.clone()
        };
        let bold_family = if reader_font == ReaderFont::Monospace {
            self.font_monospace.font_family.clone()
        } else {
            self.font_standard_bold.font_family.clone()
        };
        let bold_italic_family = if reader_font == ReaderFont::Monospace {
            self.font_monospace.font_family.clone()
        } else {
            self.font_standard_bold_italic.font_family.clone()
        };

        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_portal_list().borrow_mut() {
                list.set_item_range(cx, 0, paragraphs.len());
                while let Some(item_id) = list.next_visible_item(cx) {
                    let Some(para) = paragraphs.get(item_id) else {
                        continue;
                    };
                    let has_gloss = para.contains("<ruby");
                    let template = if has_gloss { id!(GlossPara) } else { id!(Para) };
                    let line_spacing = if has_gloss {
                        reader_spacing.gloss_line_spacing()
                    } else {
                        reader_spacing.plain_line_spacing()
                    };
                    let item = list.item(cx, item_id, template);
                    let html = item.widget(cx, ids!(para_html));
                    if let Some(mut inner) = html.borrow_mut::<Html>() {
                        if inner.body.as_ref() != para {
                            inner.set_text(cx, para);
                        }
                        inner.text_flow.selectable = !self.compact_layout;
                        inner.text_flow.font_size = reader_font_size;
                        inner.text_flow.draw_selection.color = selection_color;
                        inner.text_flow.text_style_normal.font_family = normal_family.clone();
                        inner.text_flow.text_style_italic.font_family = italic_family.clone();
                        inner.text_flow.text_style_bold.font_family = bold_family.clone();
                        inner.text_flow.text_style_bold_italic.font_family =
                            bold_italic_family.clone();
                        inner.text_flow.text_style_normal.line_spacing = line_spacing;
                        inner.text_flow.text_style_italic.line_spacing = line_spacing;
                        inner.text_flow.text_style_bold.line_spacing = line_spacing;
                        inner.text_flow.text_style_bold_italic.line_spacing = line_spacing;
                        inner.text_flow.text_style_fixed.line_spacing = line_spacing;
                    }
                    if let Some((start, end)) = selection_range {
                        if item_id < start.0 || item_id > end.0 {
                            item.selection_clear();
                        } else if item_id == start.0 && item_id == end.0 {
                            item.selection_set(start.1, end.1);
                        } else if item_id == start.0 {
                            item.selection_set(start.1, item.selection_text_len());
                        } else if item_id == end.0 {
                            item.selection_set(0, end.1);
                        } else {
                            item.selection_select_all();
                        }
                    } else {
                        item.selection_clear();
                    }
                    item.draw_all_unscoped(cx);
                }
            }
        }

        DrawStep::done()
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        if SELECTION_POPUP_POINTER.load(Ordering::Relaxed)
            && matches!(
                event,
                Event::MouseDown(_) | Event::MouseMove(_) | Event::MouseUp(_)
            )
        {
            return;
        }
        if self.touch_selection_uid.is_some() && matches!(event, Event::TouchUpdate(_)) {
            let released = matches!(event, Event::TouchUpdate(event) if event.touches.iter().any(|touch| {
                Some(touch.uid) == self.touch_selection_uid && touch.state == TouchState::Stop
            }));
            self.track_selection(cx, event);
            // Let PortalList observe the final release so its pre-long-press drag
            // state cannot leak into the next gesture. Selection moves stay local.
            if released {
                self.view.handle_event(cx, event, scope);
            }
            return;
        }
        self.view.handle_event(cx, event, scope);
        self.track_selection(cx, event);
    }
}

#[derive(Script, ScriptHook, Widget)]
struct TocList {
    #[deref]
    view: View,
    #[live]
    active_bg: Vec4f,
    #[live]
    inactive_bg: Vec4f,
}

impl Widget for TocList {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let guard = CURRENT_BOOK.read().unwrap();
        let titles: &[String] = guard
            .as_ref()
            .map(|b| b.chapter_titles.as_slice())
            .unwrap_or(&[]);
        let current = CURRENT_CHAPTER.load(std::sync::atomic::Ordering::Relaxed);

        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_portal_list().borrow_mut() {
                list.set_item_range(cx, 0, titles.len());
                while let Some(item_id) = list.next_visible_item(cx) {
                    let Some(title) = titles.get(item_id) else {
                        continue;
                    };
                    let item = list.item(cx, item_id, id!(TocItem));
                    let text = format!("{}. {}", item_id + 1, title);
                    let mut item_mut = item.clone();
                    item.button(cx, ids!(toc_item_btn)).set_text(cx, &text);
                    if item_id == current {
                        let color = self.active_bg;
                        script_apply_eval!(cx, item_mut, {
                            draw_bg.color: #(color)
                        });
                    } else {
                        let color = self.inactive_bg;
                        script_apply_eval!(cx, item_mut, {
                            draw_bg.color: #(color)
                        });
                    }
                    item.draw_all_unscoped(cx);
                }
            }
        }
        DrawStep::done()
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        self.view.handle_event(cx, event, scope);
    }
}

#[derive(Script, ScriptHook, Widget)]
struct NoteList {
    #[deref]
    view: View,
    #[rust]
    projection_revision: usize,
    #[rust]
    alternate_template: bool,
}

fn preview_text(text: &str, max_chars: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = compact.chars();
    let preview: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}…", preview)
    } else {
        preview
    }
}

impl Widget for NoteList {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let projection = NOTES.read().unwrap();
        let language = *APP_LANGUAGE.read().unwrap();
        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_portal_list().borrow_mut() {
                list.set_item_range(cx, 0, projection.entries.len());
                if self.projection_revision != projection.revision {
                    list.set_first_id_and_scroll(0, 0.0);
                    self.projection_revision = projection.revision;
                    self.alternate_template = !self.alternate_template;
                }
                let template = if self.alternate_template {
                    id!(NoteCardAlternate)
                } else {
                    id!(NoteCard)
                };
                while let Some(item_id) = list.next_visible_item(cx) {
                    let Some(note) = projection.entry_for_item_id(item_id) else {
                        continue;
                    };
                    let item = list.item(cx, item_id, template);
                    item.label(cx, ids!(note_title)).set_text(cx, &note.title);
                    item.label(cx, ids!(note_preview))
                        .set_text(cx, &preview_text(&note.content, 120));
                    item.widget(cx, ids!(note_delete_btn))
                        .set_text(cx, crate::i18n::text(language, "delete"));
                    item.draw_all_unscoped(cx);
                }
            }
        }
        DrawStep::done()
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        self.view.handle_event(cx, event, scope);
    }
}

#[derive(Script, ScriptHook, Widget)]
struct ChatList {
    #[deref]
    view: View,
}

impl Widget for ChatList {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let data = CHAT_DATA.read().unwrap();
        let language = *APP_LANGUAGE.read().unwrap();
        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_portal_list().borrow_mut() {
                let message_count = data.messages.len();
                list.set_item_range(cx, 0, message_count + usize::from(data.is_streaming));
                while let Some(item_id) = list.next_visible_item(cx) {
                    if data.is_streaming && item_id == message_count {
                        let item = list.item(cx, item_id, id!(Assistant));
                        let text = if data.streaming_text.is_empty() {
                            if language == AppLanguage::English {
                                "Thinking…"
                            } else {
                                "思考中…"
                            }
                        } else {
                            &data.streaming_text
                        };
                        item.label(cx, ids!(message)).set_text(cx, text);
                        item.draw_all_unscoped(cx);
                        continue;
                    }
                    let Some(message) = data.messages.get(item_id) else {
                        continue;
                    };
                    let template = match message.role {
                        ChatRole::User => id!(User),
                        ChatRole::Assistant => id!(Assistant),
                    };
                    let item = list.item(cx, item_id, template);
                    item.label(cx, ids!(message)).set_text(cx, &message.text);
                    item.draw_all_unscoped(cx);
                }
            }
        }
        DrawStep::done()
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        self.view.handle_event(cx, event, scope);
    }
}

#[derive(Script, ScriptHook, Widget)]
struct SkillList {
    #[deref]
    view: View,
    #[rust]
    projection_revision: usize,
    #[rust]
    alternate_template: bool,
}

impl Widget for SkillList {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let projection = SKILLS.read().unwrap();
        let language = *APP_LANGUAGE.read().unwrap();
        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_portal_list().borrow_mut() {
                list.set_item_range(cx, 0, projection.entries.len());
                if self.projection_revision != projection.revision {
                    list.set_first_id_and_scroll(0, 0.0);
                    self.projection_revision = projection.revision;
                    self.alternate_template = !self.alternate_template;
                }
                let template = if self.alternate_template {
                    id!(SkillCardAlternate)
                } else {
                    id!(SkillCard)
                };
                while let Some(item_id) = list.next_visible_item(cx) {
                    let Some(skill) = projection.entry_for_item_id(item_id) else {
                        continue;
                    };
                    let item = list.item(cx, item_id, template);
                    item.label(cx, ids!(skill_name)).set_text(cx, &skill.name);
                    item.label(cx, ids!(skill_kind)).set_text(
                        cx,
                        if skill.is_system {
                            if language == AppLanguage::English {
                                "System"
                            } else {
                                "系统"
                            }
                        } else {
                            if language == AppLanguage::English {
                                "Custom"
                            } else {
                                "自定义"
                            }
                        },
                    );
                    item.label(cx, ids!(skill_preview))
                        .set_text(cx, &preview_text(&skill.content, 100));
                    item.check_box(cx, ids!(skill_active)).set_active(
                        cx,
                        skill.is_active,
                        Animate::No,
                    );
                    item.widget(cx, ids!(skill_use))
                        .set_visible(cx, !skill.is_system && skill.is_active);
                    item.widget(cx, ids!(skill_delete))
                        .set_visible(cx, !skill.is_system);
                    item.widget(cx, ids!(skill_active))
                        .set_text(cx, crate::i18n::text(language, "enable"));
                    item.widget(cx, ids!(skill_use))
                        .set_text(cx, crate::i18n::text(language, "use_next"));
                    item.widget(cx, ids!(skill_edit))
                        .set_text(cx, crate::i18n::text(language, "edit"));
                    item.widget(cx, ids!(skill_delete))
                        .set_text(cx, crate::i18n::text(language, "delete"));
                    item.draw_all_unscoped(cx);
                }
            }
        }
        DrawStep::done()
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        self.view.handle_event(cx, event, scope);
    }
}

#[derive(Script, ScriptHook, Widget)]
struct VocabList {
    #[deref]
    view: View,
    #[rust]
    projection_revision: usize,
    #[rust]
    alternate_template: bool,
}

impl Widget for VocabList {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let projection = VOCAB_VISIBLE.read().unwrap();
        let language = *APP_LANGUAGE.read().unwrap();
        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_portal_list().borrow_mut() {
                list.set_item_range(cx, 0, projection.entries.len());
                if self.projection_revision != projection.revision {
                    list.set_first_id_and_scroll(0, 0.0);
                    self.projection_revision = projection.revision;
                    self.alternate_template = !self.alternate_template;
                }
                let template = if self.alternate_template {
                    id!(VocabCardAlternate)
                } else {
                    id!(VocabCard)
                };
                while let Some(item_id) = list.next_visible_item(cx) {
                    let Some(entry) = projection.entry_for_item_id(item_id) else {
                        continue;
                    };
                    let item = list.item(cx, item_id, template);
                    let heading = if entry.word.eq_ignore_ascii_case(&entry.lemma) {
                        entry.word.clone()
                    } else {
                        format!("{} · {}", entry.word, entry.lemma)
                    };
                    item.label(cx, ids!(vocab_word)).set_text(cx, &heading);
                    item.label(cx, ids!(vocab_status)).set_text(
                        cx,
                        if entry.status == "mastered" {
                            crate::i18n::text(language, "mastered")
                        } else {
                            crate::i18n::text(language, "learning")
                        },
                    );
                    item.label(cx, ids!(vocab_gloss)).set_text(
                        cx,
                        entry
                            .gloss
                            .as_deref()
                            .filter(|gloss| !gloss.trim().is_empty())
                            .unwrap_or(if language == AppLanguage::English {
                                "No definition"
                            } else {
                                "暂无释义"
                            }),
                    );
                    let context = entry
                        .context_sentence
                        .as_deref()
                        .filter(|context| !context.trim().is_empty());
                    item.widget(cx, ids!(vocab_context))
                        .set_visible(cx, context.is_some());
                    item.label(cx, ids!(vocab_context))
                        .set_text(cx, context.unwrap_or(""));
                    let level = entry
                        .level
                        .filter(|level| *level > 0)
                        .map(|level| format!("L{level}"))
                        .unwrap_or_else(|| {
                            if language == AppLanguage::English {
                                "Unlisted".to_string()
                            } else {
                                "词表外".to_string()
                            }
                        });
                    item.label(cx, ids!(vocab_meta)).set_text(
                        cx,
                        &if language == AppLanguage::English {
                            format!("{level} · Reviewed {} times", entry.review_count)
                        } else {
                            format!("{level} · 已复习 {} 次", entry.review_count)
                        },
                    );
                    item.button(cx, ids!(vocab_toggle)).set_text(
                        cx,
                        if entry.status == "mastered" {
                            crate::i18n::text(language, "keep_learning")
                        } else {
                            crate::i18n::text(language, "mark_mastered")
                        },
                    );
                    item.widget(cx, ids!(vocab_speak))
                        .set_text(cx, crate::i18n::text(language, "speak"));
                    item.widget(cx, ids!(vocab_review))
                        .set_text(cx, crate::i18n::text(language, "review"));
                    item.widget(cx, ids!(vocab_remove))
                        .set_text(cx, crate::i18n::text(language, "remove"));
                    item.widget(cx, ids!(vocab_card_bottom))
                        .set_visible(cx, true);
                    item.draw_all_unscoped(cx);
                }
            }
        }
        DrawStep::done()
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        self.view.handle_event(cx, event, scope);
    }
}

#[derive(Script, ScriptHook, Widget)]
struct BookShelf {
    #[deref]
    view: View,
    /// item_id → 已加载的封面路径（PortalList 复用 item，路径变了才重载）。
    #[rust]
    loaded_covers: HashMap<usize, Option<String>>,
    #[rust]
    projection_revision: usize,
    #[rust]
    alternate_template: bool,
}

impl Widget for BookShelf {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let shelf = SHELF_VISIBLE.read().unwrap();
        let language = *APP_LANGUAGE.read().unwrap();

        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_portal_list().borrow_mut() {
                list.set_item_range(cx, 0, shelf.entries.len());
                if self.projection_revision != shelf.revision {
                    list.set_first_id_and_scroll(0, 0.0);
                    self.projection_revision = shelf.revision;
                    self.alternate_template = !self.alternate_template;
                    self.loaded_covers.clear();
                }
                let template = if self.alternate_template {
                    id!(CardAlternate)
                } else {
                    id!(Card)
                };
                while let Some(item_id) = list.next_visible_item(cx) {
                    let Some(book) = shelf.entry_for_item_id(item_id) else {
                        continue;
                    };
                    let letter = book
                        .title
                        .chars()
                        .next()
                        .map(|c| c.to_string())
                        .unwrap_or_default();
                    let item = list.item(cx, item_id, template);
                    item.label(cx, ids!(cover_letter)).set_text(cx, &letter);
                    item.label(cx, ids!(card_title)).set_text(cx, &book.title);
                    item.label(cx, ids!(card_author)).set_text(cx, &book.author);
                    item.label(cx, ids!(card_status)).set_text(
                        cx,
                        &shelf_status(
                            language,
                            &book.status,
                            book.progress_current,
                            book.progress_total,
                        ),
                    );
                    item.label(cx, ids!(card_tags)).set_text(
                        cx,
                        &if book.tags.is_empty() {
                            crate::i18n::text(language, "no_tags").to_string()
                        } else {
                            book.tags
                                .iter()
                                .map(|tag| format!("#{tag}"))
                                .collect::<Vec<_>>()
                                .join("  ")
                        },
                    );
                    item.widget(cx, ids!(read_btn))
                        .set_text(cx, crate::i18n::text(language, "reader"));
                    item.widget(cx, ids!(tag_btn))
                        .set_text(cx, crate::i18n::text(language, "edit_tags"));
                    item.widget(cx, ids!(del_btn))
                        .set_text(cx, crate::i18n::text(language, "delete"));
                    if self.loaded_covers.get(&item_id) != Some(&book.cover_path) {
                        let img = item.image(cx, ids!(cover_img));
                        match &book.cover_path {
                            Some(path) => {
                                let p = std::path::Path::new(path);
                                if let Err(e) = img.load_image_file_by_path(cx, p) {
                                    println!("封面加载失败 {}: {:?}", path, e);
                                    img.set_texture(cx, None);
                                }
                            }
                            None => img.set_texture(cx, None),
                        }
                        self.loaded_covers.insert(item_id, book.cover_path.clone());
                    }
                    item.draw_all_unscoped(cx);
                }
            }
        }

        DrawStep::done()
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        self.view.handle_event(cx, event, scope);
    }
}

#[derive(Script, ScriptHook)]
pub struct App {
    #[source]
    source: ScriptObjectRef,
    #[live]
    ui: WidgetRef,
    #[live]
    swatch_inactive: Vec4f,
    #[rust]
    reload_count: usize,
    #[rust]
    ai_backend: Option<OpenAiBackend>,
    #[rust]
    ai_request: Option<RequestId>,
    #[rust]
    ai_stream: AiSseDecoder,
    #[rust]
    ai_timeout: Timer,
    #[rust]
    ai_settings: AiSettings,
    #[rust]
    local_ai_settings: LocalAiSettings,
    #[rust]
    local_ai_session: Option<LocalAiSession>,
    #[rust]
    local_ai_poll_timer: Timer,
    #[rust]
    chat_thread_id: Option<String>,
    #[rust]
    editing_skill_id: Option<String>,
    /// 仅应用到下一条用户消息的自定义技能。
    #[rust]
    pending_skill_id: Option<String>,
    /// 书库标签筛选与模态编辑状态只由 App 持有，卡片保持纯投影。
    #[rust]
    library_tag_filter: Option<String>,
    #[rust]
    editing_book_tags_id: Option<String>,
    /// 待用户确认删除的 (book_id, title)。实际磁盘与数据库操作只在确认后开始。
    #[rust]
    pending_delete_book: Option<(String, String)>,
    #[rust]
    vocab_entries: Vec<VocabEntry>,
    #[rust]
    vocab_filter: VocabFilter,
    #[rust]
    vocab_search: String,
    #[rust]
    vocab_review_queue: Vec<String>,
    #[rust]
    vocab_review_index: usize,
    #[rust]
    vocab_review_pending: bool,
    #[rust]
    reading_session_id: Option<String>,
    #[rust]
    reading_session_pending: bool,
    #[rust]
    reading_session_invalidated: bool,
    #[rust]
    reading_checkpoint_timer: Timer,
    #[rust]
    statistics_after_pending_session: bool,
    #[rust]
    chapter_idx: usize,
    #[rust]
    on_reader_page: bool,
    #[rust]
    active_page: AppPage,
    #[rust]
    pending_theme_reapply: bool,
    #[rust]
    toc_open: bool,
    #[rust]
    note_open: bool,
    #[rust]
    reader_ai_open: bool,
    #[rust]
    compact_layout: bool,
    #[rust]
    responsive_layout_initialized: bool,
    #[rust]
    virtual_keyboard_open: bool,
    #[rust]
    safe_area_insets: SafeAreaInsets,
    #[rust]
    app_in_foreground: bool,
    #[rust]
    reader_swipe: ReaderSwipeGesture,
    #[rust]
    note_quote_text: String,
    /// 释义卡片当前展示的 (word, lemma, gloss)。
    #[rust]
    card: Option<(String, String, String)>,
}

impl App {
    fn activate_page(&mut self, cx: &mut Cx, page: AppPage) {
        self.ui
            .page_flip(cx, ids!(pages))
            .set_active_page(cx, page.live_id());
        self.active_page = page;
        self.on_reader_page = page == AppPage::Reader;
    }

    fn available_shelf_tags() -> Vec<String> {
        let shelf = SHELF.read().unwrap();
        let mut tags = shelf
            .iter()
            .flat_map(|book| book.tags.iter().cloned())
            .collect::<Vec<_>>();
        tags.sort_by_key(|tag| tag.to_lowercase());
        tags.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        tags
    }

    fn refresh_shelf_projection(&mut self, cx: &mut Cx) {
        let tags = Self::available_shelf_tags();
        if self
            .library_tag_filter
            .as_ref()
            .is_some_and(|selected| !tags.iter().any(|tag| tag.eq_ignore_ascii_case(selected)))
        {
            self.library_tag_filter = None;
        }

        let all = SHELF.read().unwrap().clone();
        let visible = shelf_items_for_tag(&all, self.library_tag_filter.as_deref());
        let visible_count = visible.len();
        let total = all.len();
        SHELF_VISIBLE.write().unwrap().replace(visible);

        let mut labels = Vec::with_capacity(tags.len() + 1);
        labels.push(tr("all_tags").to_string());
        labels.extend(tags.iter().cloned());
        let selected_index = self
            .library_tag_filter
            .as_ref()
            .and_then(|selected| {
                tags.iter()
                    .position(|tag| tag.eq_ignore_ascii_case(selected))
                    .map(|index| index + 1)
            })
            .unwrap_or(0);
        let dropdown = self.ui.drop_down(cx, ids!(library_tag_filter));
        dropdown.set_labels(cx, labels);
        dropdown.set_selected_item(cx, selected_index);

        let summary = if app_language() == AppLanguage::English {
            format!("Showing {visible_count} of {total} books")
        } else {
            format!("显示 {visible_count} / {total} 本")
        };
        self.ui
            .label(cx, ids!(library_filter_summary))
            .set_text(cx, &summary);

        let is_filtered_empty = total > 0 && visible_count == 0;
        self.ui.label(cx, ids!(library_empty_title)).set_text(
            cx,
            if is_filtered_empty {
                tr("no_tag_matches")
            } else {
                tr("library_empty")
            },
        );
        self.ui.label(cx, ids!(library_empty_hint)).set_text(
            cx,
            if is_filtered_empty {
                tr("no_tag_matches_hint")
            } else {
                tr("library_empty_hint")
            },
        );
        self.ui
            .widget(cx, ids!(library_empty))
            .set_visible(cx, visible_count == 0);
        self.ui
            .widget(cx, ids!(book_shelf))
            .set_visible(cx, visible_count > 0);
        self.ui.widget(cx, ids!(book_shelf)).redraw(cx);
        self.ui.widget(cx, ids!(page_library)).redraw(cx);
    }

    fn set_library_tag_filter(&mut self, cx: &mut Cx, index: usize) {
        self.library_tag_filter = if index == 0 {
            None
        } else {
            Self::available_shelf_tags().get(index - 1).cloned()
        };
        self.refresh_shelf_projection(cx);
    }

    fn open_library_tag_editor(&mut self, cx: &mut Cx, book: &ShelfItem) {
        self.editing_book_tags_id = Some(book.id.clone());
        self.ui.label(cx, ids!(library_tag_modal_title)).set_text(
            cx,
            &if app_language() == AppLanguage::English {
                format!("Edit Tags · {}", book.title)
            } else {
                format!("编辑标签 · {}", book.title)
            },
        );
        self.ui
            .text_input(cx, ids!(library_tag_input))
            .set_text(cx, &book.tags.join(", "));
        self.ui.label(cx, ids!(library_tag_status)).set_text(cx, "");
        self.ui.modal(cx, ids!(library_tag_modal)).open(cx);
    }

    fn save_library_tag_editor(&mut self, cx: &mut Cx) {
        let Some(book_id) = self.editing_book_tags_id.clone() else {
            return;
        };
        let input = self.ui.text_input(cx, ids!(library_tag_input)).text();
        let raw_tags = input
            .split([',', '，', ';', '；'])
            .map(str::to_string)
            .collect::<Vec<_>>();
        let tags = match readwise_core::books::normalize_book_tags(&raw_tags) {
            Ok(tags) => tags,
            Err(error) => {
                self.ui.label(cx, ids!(library_tag_status)).set_text(
                    cx,
                    &if app_language() == AppLanguage::English {
                        "Use up to 8 tags, each no longer than 24 characters.".to_string()
                    } else {
                        error
                    },
                );
                return;
            }
        };
        self.ui
            .label(cx, ids!(library_tag_status))
            .set_text(cx, localized("正在保存标签…", "Saving tags…"));
        spawn_set_book_tags(book_id, tags);
    }

    fn open_delete_book_confirmation(&mut self, cx: &mut Cx, book: &ShelfItem) {
        self.pending_delete_book = Some((book.id.clone(), book.title.clone()));
        let message = if app_language() == AppLanguage::English {
            format!(
                "Delete “{}” from ReadWise? Reading progress, notes, and highlights will also be removed. The original EPUB outside the app is not affected.",
                book.title
            )
        } else {
            format!(
                "确定要从 ReadWise 删除《{}》吗？阅读进度、笔记和高亮也会一并删除。应用外的原始 EPUB 不受影响。",
                book.title
            )
        };
        self.ui
            .label(cx, ids!(library_delete_modal_message))
            .set_text(cx, &message);
        self.ui.modal(cx, ids!(library_delete_modal)).open(cx);
    }

    fn cancel_delete_book_confirmation(&mut self, cx: &mut Cx) {
        self.pending_delete_book = None;
        self.ui.modal(cx, ids!(library_delete_modal)).close(cx);
    }

    fn confirm_delete_book(&mut self, cx: &mut Cx) {
        let Some((book_id, title)) = self.pending_delete_book.take() else {
            self.ui.modal(cx, ids!(library_delete_modal)).close(cx);
            return;
        };
        self.ui.modal(cx, ids!(library_delete_modal)).close(cx);
        self.ui.label(cx, ids!(library_hint)).set_text(
            cx,
            &if app_language() == AppLanguage::English {
                format!("Deleting “{title}”…")
            } else {
                format!("正在删除《{title}》…")
            },
        );
        spawn_delete(book_id, title);
    }

    fn sync_theme_controls(&self, cx: &mut Cx, theme: ReaderTheme) {
        self.ui.radio_button(cx, ids!(theme_dark)).set_active(
            cx,
            theme == ReaderTheme::Dark,
            Animate::No,
        );
        self.ui.radio_button(cx, ids!(theme_light)).set_active(
            cx,
            theme == ReaderTheme::Light,
            Animate::No,
        );
    }

    fn sync_app_language_controls(&self, cx: &mut Cx, language: AppLanguage) {
        self.ui.radio_button(cx, ids!(language_chinese)).set_active(
            cx,
            language == AppLanguage::Chinese,
            Animate::No,
        );
        self.ui.radio_button(cx, ids!(language_english)).set_active(
            cx,
            language == AppLanguage::English,
            Animate::No,
        );
    }

    fn apply_ui_language(&self, cx: &mut Cx, language: AppLanguage) {
        macro_rules! set_text {
            ($id:ident, $key:literal) => {
                self.ui
                    .widget(cx, ids!($id))
                    .set_text(cx, crate::i18n::text(language, $key));
            };
        }
        macro_rules! set_empty_text {
            ($id:ident, $key:literal) => {
                self.ui
                    .text_input(cx, ids!($id))
                    .set_empty_text(cx, crate::i18n::text(language, $key).to_string());
            };
        }

        set_text!(nav_library, "library");
        set_text!(nav_reader, "reader");
        set_text!(nav_chat, "chat");
        set_text!(nav_skills, "skills");
        set_text!(nav_vocab, "vocabulary");
        set_text!(nav_statistics, "statistics");
        set_text!(nav_settings, "settings");
        set_text!(mobile_nav_library, "library");
        set_text!(mobile_nav_reader, "reader");
        set_text!(mobile_nav_chat, "chat");
        set_text!(mobile_nav_skills, "skills");
        set_text!(mobile_nav_vocab, "vocabulary");
        set_text!(mobile_nav_statistics, "statistics");
        set_text!(mobile_nav_settings, "settings");

        set_text!(library_title, "library");
        set_text!(import_epub_btn, "import_epub");
        set_text!(library_hint, "library_hint");
        set_text!(library_empty_title, "library_empty");
        set_text!(library_empty_hint, "library_empty_hint");
        set_text!(library_tag_filter_label, "tag_filter");
        set_text!(library_tag_modal_title, "edit_tags_title");
        set_text!(library_tag_hint, "tag_editor_hint");
        set_text!(library_tag_cancel, "cancel");
        set_text!(library_tag_save, "save");
        set_empty_text!(library_tag_input, "tag_input_placeholder");
        set_text!(library_delete_modal_title, "delete_book_title");
        set_text!(library_delete_cancel, "cancel");
        set_text!(library_delete_confirm, "delete_book_confirm");

        set_text!(toc_btn, "contents");
        set_text!(notes_btn, "note");
        set_text!(note_panel_title, "note");
        set_text!(note_close_btn, "close");
        set_text!(note_quote_title, "quote");
        set_text!(note_cancel_btn, "cancel");
        set_text!(note_save_btn, "save");
        set_text!(note_hint, "note_select_hint");
        set_text!(note_empty_title, "no_notes");
        set_text!(note_empty_hint, "no_notes_hint");
        set_text!(reader_ai_title, "reader_ai");
        set_text!(reader_ai_close, "close");
        if self.ai_request.is_none() {
            set_text!(reader_chat_status, "reader_ai_hint");
        }
        set_text!(reader_chat_send, "send");
        set_text!(reader_chat_cancel, "stop");
        set_text!(reader_chat_clear, "new_chat");
        set_text!(copy_sel_btn, "copy");
        set_text!(note_from_sel_btn, "note");
        set_text!(vocab_from_sel_btn, "word");
        set_text!(translate_sel_btn, "translate");
        set_text!(ask_ai_sel_btn, "ask_ai");
        set_text!(speak_sel_btn, "speak");
        set_text!(hl_btn, "highlight");
        set_text!(sel_close_btn, "close");
        set_text!(close_card_btn, "close");
        set_text!(add_vocab_btn, "add_vocabulary");
        set_text!(know_btn, "mark_known");

        set_text!(vocab_title, "vocabulary");
        set_text!(vocab_start_review, "start_review");
        set_text!(vocab_filter_all, "all");
        set_text!(vocab_filter_learning, "learning");
        set_text!(vocab_filter_mastered, "mastered");
        set_text!(vocab_review_close, "end_review");
        set_text!(vocab_review_speak, "speak");
        set_text!(vocab_review_reveal, "show_definition");
        set_text!(vocab_review_again, "keep_learning");
        set_text!(vocab_review_known, "known");

        set_text!(statistics_title, "reading_statistics");
        set_text!(statistics_refresh, "refresh");
        set_text!(stat_today_title, "today_reading");
        set_text!(stat_total_title, "total_reading");
        set_text!(stat_days_title, "reading_days");
        set_text!(stat_books_title, "finished_books");
        set_text!(reading_chart_title, "last_7_days");
        set_text!(vocab_chart_title, "vocabulary_learning");
        set_text!(vocab_chart_learning, "learning");
        set_text!(vocab_chart_mastered, "mastered");
        set_text!(vocab_chart_reviews, "total_reviews");

        set_text!(chat_title, "chat");
        set_text!(chat_send, "send");
        set_text!(chat_cancel, "stop");
        set_text!(chat_clear, "new_chat");
        set_text!(chat_skill_clear, "cancel_template");
        set_text!(skills_title, "skills");
        set_text!(skill_new, "new_skill");
        set_text!(skill_editor_title, "new_skill_title");
        set_text!(skill_name_title, "name");
        set_text!(skill_active_edit, "enable_skill");
        set_text!(skill_prompt_title, "prompt_template");
        set_text!(skill_cancel, "clear");
        set_text!(skill_save, "save");
        set_empty_text!(note_extra, "note_placeholder");
        set_empty_text!(reader_chat_input, "reader_chat_placeholder");
        set_empty_text!(vocab_search, "vocab_search_placeholder");
        set_empty_text!(chat_input, "chat_placeholder");
        set_empty_text!(skill_name_input, "skill_name_placeholder");
        set_empty_text!(skill_content_input, "skill_prompt_placeholder");
        set_empty_text!(ai_endpoint, "api_endpoint_placeholder");
        set_empty_text!(ai_model, "model_placeholder");
        set_empty_text!(ai_api_key, "api_key_placeholder");
        set_empty_text!(local_ai_binary, "local_binary_placeholder");
        set_empty_text!(local_ai_model_path, "local_model_placeholder");

        set_text!(settings_title, "settings");
        set_text!(settings_reading_heading, "reading_section");
        set_text!(language_label, "language");
        set_text!(language_chinese, "chinese");
        set_text!(language_english, "english");
        set_text!(theme_label, "appearance_theme");
        set_text!(theme_dark, "dark");
        set_text!(theme_light, "light");
        set_text!(font_family_label, "body_font");
        set_text!(reader_font_standard, "standard");
        set_text!(reader_font_monospace, "monospace");
        set_text!(line_spacing_label, "line_spacing");
        set_text!(reader_spacing_default, "default");
        set_text!(reader_spacing_comfortable, "comfortable");
        set_text!(reader_spacing_relaxed, "relaxed");
        set_text!(font_size_title, "font_size");
        set_text!(gloss_enable, "gloss_enabled");
        set_text!(gloss_size_title, "gloss_size");
        set_text!(gloss_color_title, "gloss_color");
        set_text!(gloss_bg_title, "gloss_background");
        set_text!(word_color_title, "word_color");
        set_text!(word_bg_title, "word_background");
        set_text!(settings_text_heading, "text_section");
        set_text!(selection_color_title, "selection_color");
        set_text!(highlight_color_title, "highlight_color");
        set_text!(ai_settings_heading, "ai_settings");
        set_text!(ai_save, "save_ai_settings");
        set_text!(reload_button, "reload_sample");
        set_text!(ai_settings_status, "api_key_local_hint");
        set_text!(local_ai_heading, "local_ai");
        set_text!(local_ai_description, "local_ai_hint");
        set_text!(local_ai_port_label, "port");
        set_text!(local_ai_context_label, "context");
        set_text!(local_ai_threads_label, "threads_auto");
        set_text!(local_ai_gpu_layers_label, "gpu_layers");
        set_text!(local_ai_auto_start, "local_auto_start");
        set_text!(local_ai_start, "start_local_ai");
        set_text!(local_ai_save, "save_local_ai");
        set_text!(local_ai_stop, "stop_local_ai");
        if self.local_ai_session.is_none() {
            set_text!(local_ai_status, "local_not_running");
        }
        if READER_FONT_SIZE.read().unwrap().is_none() {
            set_text!(font_size_label, "default");
        }

        for (id, key) in [
            (id!(gc0), "cyan"),
            (id!(gc1), "blue"),
            (id!(gc2), "orange"),
            (id!(gc3), "gray"),
            (id!(gb0), "none"),
            (id!(gb1), "dark_gray"),
            (id!(gb2), "dark_cyan"),
            (id!(gb3), "dark_blue"),
            (id!(wc0), "default"),
            (id!(wc1), "cyan"),
            (id!(wc2), "blue"),
            (id!(wc3), "orange"),
            (id!(wb0), "none"),
            (id!(wb1), "dark_gray"),
            (id!(wb2), "dark_cyan"),
            (id!(wb3), "dark_blue"),
            (id!(sc0), "cyan"),
            (id!(sc1), "blue"),
            (id!(sc2), "orange"),
            (id!(sc3), "yellow"),
            (id!(hc0), "yellow"),
            (id!(hc1), "cyan"),
            (id!(hc2), "blue"),
            (id!(hc3), "orange"),
        ] {
            self.ui
                .widget(cx, &[id, id!(b)])
                .set_text(cx, crate::i18n::text(language, key));
        }
        self.ui
            .label(cx, ids!(settings_hint))
            .set_text(cx, crate::i18n::text(language, "reader_settings_saved"));
    }

    fn set_app_language(&mut self, cx: &mut Cx, language: AppLanguage) {
        if app_language() == language {
            self.sync_app_language_controls(cx, language);
            return;
        }
        *APP_LANGUAGE.write().unwrap() = language;
        self.sync_app_language_controls(cx, language);
        self.apply_ui_language(cx, language);
        self.refresh_shelf_projection(cx);
        self.refresh_vocab_projection(cx);
        self.refresh_pending_skill_indicator(cx);
        if self.active_page == AppPage::Statistics {
            spawn_load_statistics();
        }
        self.persist_reader_settings(cx);
        self.ui.redraw(cx);
    }

    fn sync_reader_font_controls(&self, cx: &mut Cx, font: ReaderFont) {
        self.ui
            .radio_button(cx, ids!(reader_font_standard))
            .set_active(cx, font == ReaderFont::Standard, Animate::No);
        self.ui
            .radio_button(cx, ids!(reader_font_monospace))
            .set_active(cx, font == ReaderFont::Monospace, Animate::No);
    }

    fn set_reader_font(&mut self, cx: &mut Cx, font: ReaderFont) {
        if *READER_FONT.read().unwrap() == font {
            self.sync_reader_font_controls(cx, font);
            return;
        }
        *READER_FONT.write().unwrap() = font;
        self.sync_reader_font_controls(cx, font);
        self.refresh_current_chapter(cx);
        self.persist_reader_settings(cx);
    }

    fn sync_reader_spacing_controls(&self, cx: &mut Cx, spacing: ReaderSpacing) {
        self.ui
            .radio_button(cx, ids!(reader_spacing_default))
            .set_active(cx, spacing == ReaderSpacing::Default, Animate::No);
        self.ui
            .radio_button(cx, ids!(reader_spacing_comfortable))
            .set_active(cx, spacing == ReaderSpacing::Comfortable, Animate::No);
        self.ui
            .radio_button(cx, ids!(reader_spacing_relaxed))
            .set_active(cx, spacing == ReaderSpacing::Relaxed, Animate::No);
    }

    fn set_reader_spacing(&mut self, cx: &mut Cx, spacing: ReaderSpacing) {
        if *READER_SPACING.read().unwrap() == spacing {
            self.sync_reader_spacing_controls(cx, spacing);
            return;
        }
        *READER_SPACING.write().unwrap() = spacing;
        self.sync_reader_spacing_controls(cx, spacing);
        self.refresh_current_chapter(cx);
        self.persist_reader_settings(cx);
    }

    fn set_reader_theme(&mut self, cx: &mut Cx, theme: ReaderTheme) {
        if *READER_THEME.read().unwrap() == theme {
            self.sync_theme_controls(cx, theme);
            return;
        }
        *READER_THEME.write().unwrap() = theme;
        self.sync_theme_controls(cx, theme);
        self.persist_reader_settings(cx);
        self.apply_runtime_theme(cx, theme);
    }

    fn apply_runtime_theme(&mut self, cx: &mut Cx, theme: ReaderTheme) {
        if theme == ReaderTheme::Light {
            script_eval!(cx, {
                mod.theme.splat(mod.readwise_theme_light)
            });
        } else {
            script_eval!(cx, {
                mod.theme.splat(mod.readwise_theme_dark)
            });
        }
        self.pending_theme_reapply = true;
        cx.request_script_reapply();
    }

    /// ScriptReapply 不销毁 widget 树；这里只恢复可能由脚本默认值重写的
    /// 页面、响应式布局和投影视图，业务数据仍由 Rust 状态持有。
    fn restore_after_script_reload(&mut self, cx: &mut Cx) {
        self.activate_page(cx, self.active_page);
        self.responsive_layout_initialized = false;
        self.update_responsive_layout(
            cx,
            cx.display_context.screen_size,
            cx.display_context.safe_area_insets,
        );
        self.apply_reader_settings(cx, current_reader_settings());
        self.sync_reader_panel_layout(cx);

        self.refresh_shelf_projection(cx);

        let notes_are_empty = NOTES.read().unwrap().entries.is_empty();
        self.ui
            .widget(cx, ids!(note_empty))
            .set_visible(cx, notes_are_empty);
        self.ui
            .widget(cx, ids!(note_list))
            .set_visible(cx, !notes_are_empty);
        self.ui
            .widget(cx, ids!(note_composer))
            .set_visible(cx, !self.note_quote_text.is_empty());

        self.refresh_vocab_projection(cx);
        self.refresh_pending_skill_indicator(cx);
        self.set_chat_streaming_controls(cx, self.ai_request.is_some());
        if self.active_page == AppPage::Statistics {
            spawn_load_statistics();
        }
        if self.pending_theme_reapply {
            self.ui.label(cx, ids!(settings_hint)).set_text(
                cx,
                localized(
                    "主题已应用；更改会自动保存在本机。",
                    "Theme applied; changes are saved on this device.",
                ),
            );
            self.pending_theme_reapply = false;
        }
        self.ui.redraw(cx);
    }

    fn track_reader_swipe(&mut self, cx: &mut Cx, event: &Event) -> Option<ReaderSwipeDirection> {
        match event {
            Event::LongPress(_) | Event::SelectionHandleDrag(_) => {
                self.reader_swipe.cancel();
                None
            }
            Event::TouchUpdate(event) => {
                if event.touches.len() != 1 {
                    self.reader_swipe.cancel();
                    return None;
                }
                let touch = &event.touches[0];
                if touch.state == TouchState::Start {
                    let chapter_rect = self
                        .ui
                        .widget(cx, ids!(chapter_list))
                        .area()
                        .clipped_rect(cx);
                    let can_begin = self.compact_layout
                        && !self.virtual_keyboard_open
                        && self.on_reader_page
                        && CURRENT_BOOK.read().unwrap().is_some()
                        && !self.toc_open
                        && !self.note_open
                        && !self.reader_ai_open
                        && SEL_RANGE.read().unwrap().is_none()
                        && SELECTED_TEXT.read().unwrap().is_empty()
                        && chapter_rect.contains(touch.abs);
                    if can_begin {
                        self.reader_swipe.begin(touch.uid, touch.abs, event.time);
                    } else {
                        self.reader_swipe.cancel();
                    }
                    return None;
                }
                self.reader_swipe.update(touch, event.time)
            }
            _ => None,
        }
    }

    fn sync_mobile_chrome(&self, cx: &mut Cx) {
        self.ui.widget(cx, ids!(mobile_nav)).set_visible(
            cx,
            should_show_mobile_nav(self.compact_layout, self.virtual_keyboard_open),
        );
    }

    fn handle_virtual_keyboard(&mut self, cx: &mut Cx, event: &VirtualKeyboardEvent) {
        let open = match event {
            VirtualKeyboardEvent::WillShow { .. } | VirtualKeyboardEvent::DidShow { .. } => true,
            // iOS emits WillHide before DidHide. Keep the bottom navigation hidden
            // throughout that animation; Android currently emits DidHide only.
            VirtualKeyboardEvent::WillHide { .. } => return,
            VirtualKeyboardEvent::DidHide { .. } => false,
        };
        if self.virtual_keyboard_open == open {
            return;
        }
        self.virtual_keyboard_open = open;
        if open {
            self.reader_swipe.cancel();
        }
        self.sync_mobile_chrome(cx);
        self.ui.redraw(cx);
    }

    fn sync_reader_panel_layout(&self, cx: &mut Cx) {
        let panel_open = self.toc_open || self.note_open || self.reader_ai_open;
        if let Some(mut chapter_list) = self
            .ui
            .widget(cx, ids!(chapter_list))
            .borrow_mut::<ChapterList>()
        {
            chapter_list.set_compact_layout(cx, self.compact_layout);
        }
        self.ui
            .widget(cx, ids!(chapter_list))
            .set_visible(cx, !self.compact_layout || !panel_open);
        self.ui
            .widget(cx, ids!(toc_panel))
            .set_visible(cx, self.toc_open);
        self.ui
            .widget(cx, ids!(note_panel))
            .set_visible(cx, self.note_open);
        self.ui
            .widget(cx, ids!(reader_ai_panel))
            .set_visible(cx, self.reader_ai_open);

        let mut toc_panel = self.ui.widget(cx, ids!(toc_panel));
        let mut note_panel = self.ui.widget(cx, ids!(note_panel));
        let mut reader_ai_panel = self.ui.widget(cx, ids!(reader_ai_panel));
        let mut selection_popup = self.ui.widget(cx, ids!(selection_popup));
        if self.compact_layout {
            let fill = Size::fill();
            script_apply_eval!(cx, toc_panel, {width: #(fill)});
            script_apply_eval!(cx, note_panel, {width: #(fill)});
            script_apply_eval!(cx, reader_ai_panel, {width: #(fill)});
            script_apply_eval!(cx, selection_popup, {width: #(fill)});
        } else {
            script_apply_eval!(cx, toc_panel, {width: 260});
            script_apply_eval!(cx, note_panel, {width: 320});
            script_apply_eval!(cx, reader_ai_panel, {width: 360});
            script_apply_eval!(cx, selection_popup, {width: 520});
        }
    }

    fn update_responsive_layout(
        &mut self,
        cx: &mut Cx,
        size: DVec2,
        safe_area_insets: SafeAreaInsets,
    ) {
        if size.x <= 0.0 || size.y <= 0.0 {
            return;
        }
        let compact = uses_compact_layout(size);
        let layout_changed = !self.responsive_layout_initialized || compact != self.compact_layout;
        let safe_area_changed = safe_area_insets != self.safe_area_insets;
        if !layout_changed && !safe_area_changed {
            return;
        }

        self.responsive_layout_initialized = true;
        self.compact_layout = compact;
        self.safe_area_insets = safe_area_insets;
        self.ui.widget(cx, ids!(sidebar)).set_visible(cx, !compact);
        self.sync_mobile_chrome(cx);
        self.ui
            .widget(cx, ids!(mobile_safe_top))
            .set_visible(cx, compact && safe_area_insets.top > 0.0);
        self.ui.widget(cx, ids!(title)).set_visible(cx, !compact);
        self.ui.widget(cx, ids!(status)).set_visible(cx, !compact);

        let safe_top = safe_area_insets.top;
        let mobile_nav_padding = Inset {
            left: 4.0,
            right: 4.0,
            top: 5.0,
            bottom: safe_area_insets.bottom + 5.0,
        };
        let mut mobile_safe_top = self.ui.widget(cx, ids!(mobile_safe_top));
        let mut mobile_nav = self.ui.widget(cx, ids!(mobile_nav));
        script_apply_eval!(cx, mobile_safe_top, {height: #(safe_top)});
        script_apply_eval!(cx, mobile_nav, {padding: #(mobile_nav_padding)});
        self.sync_reader_panel_layout(cx);
        self.ui.redraw(cx);
    }

    fn format_reading_duration(seconds: i64) -> String {
        let english = app_language() == AppLanguage::English;
        if seconds > 0 && seconds < 60 {
            return if english { "< 1 min" } else { "< 1 分钟" }.to_string();
        }
        let minutes = seconds.max(0) / 60;
        if minutes < 60 {
            if english {
                format!("{minutes} min")
            } else {
                format!("{minutes} 分钟")
            }
        } else {
            let hours = minutes / 60;
            let remainder = minutes % 60;
            if remainder == 0 {
                if english {
                    format!("{hours} hr")
                } else {
                    format!("{hours} 小时")
                }
            } else if english {
                format!("{hours} hr {remainder} min")
            } else {
                format!("{hours} 小时 {remainder} 分")
            }
        }
    }

    fn start_reading_timer(&mut self, cx: &mut Cx) {
        if !self.app_in_foreground
            || !self.on_reader_page
            || self.reading_session_id.is_some()
            || self.reading_session_pending
        {
            return;
        }
        let book_id = CURRENT_BOOK
            .read()
            .unwrap()
            .as_ref()
            .map(|book| book.id.clone());
        let Some(book_id) = book_id else { return };
        self.reading_session_pending = true;
        self.reading_session_invalidated = false;
        self.ui.label(cx, ids!(status)).set_text(
            cx,
            if app_language() == AppLanguage::English {
                "Tracking reading time…"
            } else {
                "阅读计时中…"
            },
        );
        spawn_start_reading_session(book_id);
    }

    fn stop_reading_timer(&mut self, cx: &mut Cx, load_statistics: bool) {
        let was_pending = self.reading_session_pending;
        if was_pending {
            self.reading_session_invalidated = true;
        }
        cx.stop_timer(self.reading_checkpoint_timer);
        self.reading_checkpoint_timer = Timer::default();
        if let Some(id) = self.reading_session_id.take() {
            spawn_finish_reading_session(id, load_statistics);
        } else if load_statistics {
            if was_pending {
                self.statistics_after_pending_session = true;
            } else {
                spawn_load_statistics();
            }
        }
    }

    fn apply_statistics(&self, cx: &mut Cx, snapshot: &StatisticsSnapshot) {
        let english = app_language() == AppLanguage::English;
        self.ui.label(cx, ids!(stat_today)).set_text(
            cx,
            &Self::format_reading_duration(snapshot.today_reading_seconds),
        );
        self.ui.label(cx, ids!(stat_total)).set_text(
            cx,
            &Self::format_reading_duration(snapshot.total_reading_seconds),
        );
        self.ui.label(cx, ids!(stat_days)).set_text(
            cx,
            &if english {
                format!("{} days", snapshot.active_days)
            } else {
                format!("{} 天", snapshot.active_days)
            },
        );
        self.ui.label(cx, ids!(stat_books)).set_text(
            cx,
            &format!("{} / {}", snapshot.completed_books, snapshot.total_books),
        );

        let reading_values = snapshot
            .daily_reading
            .iter()
            .map(|day| day.duration_seconds as f64 / 60.0)
            .collect::<Vec<_>>();
        if let Some(mut chart) = self
            .ui
            .widget(cx, ids!(reading_chart))
            .borrow_mut::<StatsBars>()
        {
            chart.set_values(cx, reading_values);
        }
        let daily_peak = snapshot
            .daily_reading
            .iter()
            .map(|day| day.duration_seconds)
            .max()
            .unwrap_or(0);
        let total_duration = Self::format_reading_duration(
            snapshot
                .daily_reading
                .iter()
                .map(|day| day.duration_seconds)
                .sum(),
        );
        let peak_duration = Self::format_reading_duration(daily_peak);
        let reading_hint = if english {
            format!("7-day total {total_duration} · Daily peak {peak_duration}")
        } else {
            format!("7 天合计 {total_duration} · 单日最高 {peak_duration}")
        };
        self.ui
            .label(cx, ids!(reading_chart_hint))
            .set_text(cx, &reading_hint);
        for (index, day) in snapshot.daily_reading.iter().enumerate() {
            let label = if index + 1 == snapshot.daily_reading.len() {
                if english { "Today" } else { "今天" }.to_string()
            } else {
                day.date.get(5..).unwrap_or(&day.date).replace('-', "/")
            };
            let id = match index {
                0 => ids!(stat_day_0),
                1 => ids!(stat_day_1),
                2 => ids!(stat_day_2),
                3 => ids!(stat_day_3),
                4 => ids!(stat_day_4),
                5 => ids!(stat_day_5),
                _ => ids!(stat_day_6),
            };
            self.ui.label(cx, id).set_text(cx, &label);
        }

        if let Some(mut chart) = self
            .ui
            .widget(cx, ids!(vocab_chart))
            .borrow_mut::<StatsBars>()
        {
            chart.set_values(
                cx,
                vec![
                    snapshot.vocab_learning as f64,
                    snapshot.vocab_mastered as f64,
                    snapshot.total_reviews as f64,
                ],
            );
        }
        let vocab_hint = if english {
            format!(
                "Words {} · Learning {} · Mastered {} · Reviews {}",
                snapshot.vocab_total,
                snapshot.vocab_learning,
                snapshot.vocab_mastered,
                snapshot.total_reviews
            )
        } else {
            format!(
                "生词 {} · 学习中 {} · 已掌握 {} · 累计复习 {} 次",
                snapshot.vocab_total,
                snapshot.vocab_learning,
                snapshot.vocab_mastered,
                snapshot.total_reviews
            )
        };
        self.ui
            .label(cx, ids!(vocab_chart_hint))
            .set_text(cx, &vocab_hint);
        self.ui.label(cx, ids!(statistics_status)).set_text(
            cx,
            if english {
                "Statistics updated · Reading time is saved every 30 seconds"
            } else {
                "统计已更新 · 阅读页停留时间每 30 秒自动保存"
            },
        );
        self.ui.widget(cx, ids!(page_statistics)).redraw(cx);
    }

    fn text_input_has_focus(&self, cx: &Cx) -> bool {
        [
            ids!(note_extra),
            ids!(reader_chat_input),
            ids!(chat_input),
            ids!(ai_endpoint),
            ids!(ai_model),
            ids!(ai_api_key),
            ids!(local_ai_binary),
            ids!(local_ai_model_path),
            ids!(local_ai_port),
            ids!(local_ai_context),
            ids!(local_ai_threads),
            ids!(local_ai_gpu_layers),
            ids!(skill_name_input),
            ids!(skill_content_input),
            ids!(vocab_search),
            ids!(library_tag_input),
        ]
        .into_iter()
        .any(|path| self.ui.text_input(cx, path).key_focus(cx))
    }

    fn refresh_vocab_projection(&self, cx: &mut Cx) {
        let english = app_language() == AppLanguage::English;
        let query = self.vocab_search.trim().to_lowercase();
        let visible = self
            .vocab_entries
            .iter()
            .filter(|entry| self.vocab_filter.accepts(entry))
            .filter(|entry| {
                query.is_empty()
                    || entry.word.to_lowercase().contains(&query)
                    || entry.lemma.to_lowercase().contains(&query)
                    || entry
                        .gloss
                        .as_deref()
                        .is_some_and(|gloss| gloss.to_lowercase().contains(&query))
                    || entry
                        .context_sentence
                        .as_deref()
                        .is_some_and(|context| context.to_lowercase().contains(&query))
            })
            .cloned()
            .collect::<Vec<_>>();
        let total = self.vocab_entries.len();
        let learning = self
            .vocab_entries
            .iter()
            .filter(|entry| entry.status == "learning")
            .count();
        let mastered = total.saturating_sub(learning);
        let visible_count = visible.len();
        VOCAB_VISIBLE.write().unwrap().replace(visible);
        let summary = if english {
            format!(
                "{total} words · Learning {learning} · Mastered {mastered} · Showing {visible_count}"
            )
        } else {
            format!(
                "共 {total} 词 · 学习中 {learning} · 已掌握 {mastered} · 当前显示 {visible_count}"
            )
        };
        self.ui
            .label(cx, ids!(vocab_summary))
            .set_text(cx, &summary);
        self.ui.label(cx, ids!(vocab_filter_hint)).set_text(
            cx,
            &if english {
                format!("Filter: {}", self.vocab_filter.label())
            } else {
                format!("当前筛选：{}", self.vocab_filter.label())
            },
        );
        for (button, filter) in [
            (
                self.ui.radio_button(cx, ids!(vocab_filter_all)),
                VocabFilter::All,
            ),
            (
                self.ui.radio_button(cx, ids!(vocab_filter_learning)),
                VocabFilter::Learning,
            ),
            (
                self.ui.radio_button(cx, ids!(vocab_filter_mastered)),
                VocabFilter::Mastered,
            ),
        ] {
            button.set_active(cx, self.vocab_filter == filter, Animate::No);
        }
        self.ui
            .widget(cx, ids!(vocab_start_review))
            .set_visible(cx, learning > 0);
        let has_visible_entries = visible_count > 0;
        let empty_title = if total == 0 {
            if english {
                "Vocabulary is empty"
            } else {
                "生词本为空"
            }
            .to_string()
        } else if !query.is_empty() {
            if english {
                format!("No matches for “{}”", self.vocab_search.trim())
            } else {
                format!("没有找到“{}”", self.vocab_search.trim())
            }
        } else if english {
            format!("No words in {}", self.vocab_filter.label())
        } else {
            format!("“{}”中还没有单词", self.vocab_filter.label())
        };
        let empty_hint = if total == 0 {
            if english {
                "Select an English word in the reader and add it to Vocabulary."
            } else {
                "去阅读页选择一个英文单词加入生词本。"
            }
            .to_string()
        } else if english {
            format!("{total} words remain; adjust the search or filter.")
        } else {
            format!("生词本中仍有 {total} 个单词，请调整搜索或筛选条件。")
        };
        self.ui
            .label(cx, ids!(vocab_empty_title))
            .set_text(cx, &empty_title);
        self.ui
            .label(cx, ids!(vocab_empty_hint))
            .set_text(cx, &empty_hint);
        self.ui
            .widget(cx, ids!(vocab_empty))
            .set_visible(cx, !has_visible_entries);
        let vocab_widget = self.ui.widget(cx, ids!(vocab_list));
        vocab_widget.set_visible(cx, has_visible_entries);
        let list = vocab_widget.portal_list(cx, ids!(list));
        list.redraw(cx);
        vocab_widget.redraw(cx);
        self.ui.widget(cx, ids!(page_vocab)).redraw(cx);
    }

    fn replace_vocab_entries(&mut self, cx: &mut Cx, entries: Vec<VocabEntry>) {
        self.vocab_entries = entries;
        self.refresh_vocab_projection(cx);
    }

    fn set_vocab_filter(&mut self, cx: &mut Cx, filter: VocabFilter) {
        self.vocab_filter = filter;
        self.refresh_vocab_projection(cx);
    }

    fn current_review_entry(&self) -> Option<VocabEntry> {
        let lemma = self.vocab_review_queue.get(self.vocab_review_index)?;
        self.vocab_entries
            .iter()
            .find(|entry| &entry.lemma == lemma)
            .cloned()
    }

    fn start_vocab_review(&mut self, cx: &mut Cx, queue: Vec<String>) {
        let english = app_language() == AppLanguage::English;
        if self.vocab_review_pending {
            self.ui.label(cx, ids!(vocab_status_hint)).set_text(
                cx,
                if english {
                    "Wait for the current review result to be saved"
                } else {
                    "请等待当前复习结果保存完成"
                },
            );
            return;
        }
        if queue.is_empty() {
            self.ui.label(cx, ids!(vocab_status_hint)).set_text(
                cx,
                if english {
                    "No words are due for review"
                } else {
                    "当前没有需要复习的生词"
                },
            );
            return;
        }
        self.vocab_review_queue = queue;
        self.vocab_review_index = 0;
        self.vocab_review_pending = false;
        self.show_vocab_review_card(cx);
    }

    fn show_vocab_review_card(&mut self, cx: &mut Cx) {
        let english = app_language() == AppLanguage::English;
        let Some(entry) = self.current_review_entry() else {
            self.finish_vocab_review(
                cx,
                if english {
                    "Review complete ✓"
                } else {
                    "本轮复习完成 ✓"
                },
            );
            return;
        };
        let progress = if english {
            format!(
                "Review {}/{} · Reviewed {} times",
                self.vocab_review_index + 1,
                self.vocab_review_queue.len(),
                entry.review_count
            )
        } else {
            format!(
                "复习 {}/{} · 此前复习 {} 次",
                self.vocab_review_index + 1,
                self.vocab_review_queue.len(),
                entry.review_count
            )
        };
        self.ui
            .label(cx, ids!(vocab_review_progress))
            .set_text(cx, &progress);
        self.ui
            .label(cx, ids!(vocab_review_word))
            .set_text(cx, &entry.word);
        self.ui.label(cx, ids!(vocab_review_context)).set_text(
            cx,
            entry.context_sentence.as_deref().unwrap_or(if english {
                "Recall the meaning before revealing the definition."
            } else {
                "先回忆含义，再显示释义。"
            }),
        );
        self.ui.label(cx, ids!(vocab_review_gloss)).set_text(
            cx,
            entry
                .gloss
                .as_deref()
                .filter(|gloss| !gloss.trim().is_empty())
                .unwrap_or(if english {
                    "No definition yet"
                } else {
                    "暂无释义"
                }),
        );
        self.ui
            .widget(cx, ids!(vocab_review_panel))
            .set_visible(cx, true);
        self.ui
            .widget(cx, ids!(vocab_review_gloss))
            .set_visible(cx, false);
        self.ui
            .widget(cx, ids!(vocab_review_reveal))
            .set_visible(cx, true);
        self.ui
            .widget(cx, ids!(vocab_review_again))
            .set_visible(cx, false);
        self.ui
            .widget(cx, ids!(vocab_review_known))
            .set_visible(cx, false);
        self.ui.redraw(cx);
    }

    fn reveal_vocab_review(&self, cx: &mut Cx) {
        self.ui
            .widget(cx, ids!(vocab_review_gloss))
            .set_visible(cx, true);
        self.ui
            .widget(cx, ids!(vocab_review_reveal))
            .set_visible(cx, false);
        self.ui
            .widget(cx, ids!(vocab_review_again))
            .set_visible(cx, true);
        self.ui
            .widget(cx, ids!(vocab_review_known))
            .set_visible(cx, true);
        self.ui.redraw(cx);
    }

    fn submit_vocab_review(&mut self, cx: &mut Cx, status: &'static str) {
        if self.vocab_review_pending {
            return;
        }
        let Some(entry) = self.current_review_entry() else {
            return;
        };
        self.vocab_review_pending = true;
        self.ui
            .widget(cx, ids!(vocab_review_again))
            .set_visible(cx, false);
        self.ui
            .widget(cx, ids!(vocab_review_known))
            .set_visible(cx, false);
        self.ui.label(cx, ids!(vocab_status_hint)).set_text(
            cx,
            if app_language() == AppLanguage::English {
                "Saving review result…"
            } else {
                "正在记录复习结果…"
            },
        );
        spawn_review_vocab(entry.lemma, status);
    }

    fn advance_vocab_review(&mut self, cx: &mut Cx) {
        self.vocab_review_pending = false;
        self.vocab_review_index += 1;
        self.show_vocab_review_card(cx);
    }

    fn finish_vocab_review(&mut self, cx: &mut Cx, message: &str) {
        self.vocab_review_queue.clear();
        self.vocab_review_index = 0;
        self.vocab_review_pending = false;
        self.ui
            .widget(cx, ids!(vocab_review_panel))
            .set_visible(cx, false);
        self.ui
            .label(cx, ids!(vocab_status_hint))
            .set_text(cx, message);
        self.ui.redraw(cx);
    }

    fn apply_local_ai_settings_to_ui(
        &self,
        cx: &mut Cx,
        settings: &readwise_core::local_ai::LocalAiSettings,
    ) {
        self.ui
            .text_input(cx, ids!(local_ai_binary))
            .set_text(cx, &settings.binary_path);
        self.ui
            .text_input(cx, ids!(local_ai_model_path))
            .set_text(cx, &settings.model_path);
        self.ui
            .text_input(cx, ids!(local_ai_port))
            .set_text(cx, &settings.port.to_string());
        self.ui
            .text_input(cx, ids!(local_ai_context))
            .set_text(cx, &settings.context_size.to_string());
        self.ui
            .text_input(cx, ids!(local_ai_threads))
            .set_text(cx, &settings.threads.to_string());
        self.ui
            .text_input(cx, ids!(local_ai_gpu_layers))
            .set_text(cx, &settings.gpu_layers.to_string());
        self.ui.check_box(cx, ids!(local_ai_auto_start)).set_active(
            cx,
            settings.auto_start,
            Animate::No,
        );
    }

    fn local_ai_settings_from_ui(
        &self,
        cx: &Cx,
    ) -> Result<readwise_core::local_ai::LocalAiSettings, String> {
        fn number<T: std::str::FromStr>(value: String, label: &str) -> Result<T, String> {
            value
                .trim()
                .parse()
                .map_err(|_| format!("{label}必须是有效整数"))
        }

        Ok(readwise_core::local_ai::LocalAiSettings {
            binary_path: self
                .ui
                .text_input(cx, ids!(local_ai_binary))
                .text()
                .trim()
                .to_string(),
            model_path: self
                .ui
                .text_input(cx, ids!(local_ai_model_path))
                .text()
                .trim()
                .to_string(),
            port: number(self.ui.text_input(cx, ids!(local_ai_port)).text(), "端口")?,
            context_size: number(
                self.ui.text_input(cx, ids!(local_ai_context)).text(),
                "上下文长度",
            )?,
            threads: number(
                self.ui.text_input(cx, ids!(local_ai_threads)).text(),
                "线程数",
            )?,
            gpu_layers: number(
                self.ui.text_input(cx, ids!(local_ai_gpu_layers)).text(),
                "GPU 层数",
            )?,
            auto_start: self.ui.check_box(cx, ids!(local_ai_auto_start)).active(cx),
        })
    }

    fn set_local_ai_running(&self, cx: &mut Cx, running: bool) {
        self.ui
            .widget(cx, ids!(local_ai_start))
            .set_visible(cx, !running);
        self.ui
            .widget(cx, ids!(local_ai_stop))
            .set_visible(cx, running);
    }

    fn clear_skill_editor(&mut self, cx: &mut Cx) {
        self.editing_skill_id = None;
        self.ui
            .label(cx, ids!(skill_editor_title))
            .set_text(cx, tr("new_skill_title"));
        self.ui
            .text_input(cx, ids!(skill_name_input))
            .set_text(cx, "");
        self.ui
            .text_input(cx, ids!(skill_name_input))
            .set_is_read_only(cx, false);
        self.ui
            .text_input(cx, ids!(skill_content_input))
            .set_text(cx, "");
        self.ui
            .check_box(cx, ids!(skill_active_edit))
            .set_active(cx, true, Animate::No);
        self.ui
            .label(cx, ids!(skill_editor_hint))
            .set_text(cx, tr("skill_editor_hint"));
    }

    fn edit_skill(&mut self, cx: &mut Cx, skill: &readwise_core::skills::models::Skill) {
        self.editing_skill_id = Some(skill.id.clone());
        self.ui.label(cx, ids!(skill_editor_title)).set_text(
            cx,
            if skill.is_system {
                if app_language() == AppLanguage::English {
                    "Edit System Skill"
                } else {
                    "编辑系统技能"
                }
            } else if app_language() == AppLanguage::English {
                "Edit Skill"
            } else {
                "编辑技能"
            },
        );
        self.ui
            .text_input(cx, ids!(skill_name_input))
            .set_text(cx, &skill.name);
        self.ui
            .text_input(cx, ids!(skill_name_input))
            .set_is_read_only(cx, skill.is_system);
        self.ui
            .text_input(cx, ids!(skill_content_input))
            .set_text(cx, &skill.content);
        self.ui
            .check_box(cx, ids!(skill_active_edit))
            .set_active(cx, skill.is_active, Animate::No);
        self.ui.label(cx, ids!(skill_editor_hint)).set_text(
            cx,
            if skill.is_system {
                tr("system_skill_hint")
            } else {
                tr("custom_skill_hint")
            },
        );
    }

    fn refresh_pending_skill_indicator(&self, cx: &mut Cx) {
        let selected = self.pending_skill_id.as_ref().and_then(|id| {
            SKILLS
                .read()
                .unwrap()
                .entries
                .iter()
                .find(|skill| &skill.id == id && skill.is_active && !skill.is_system)
                .map(|skill| skill.name.clone())
        });
        match selected {
            Some(name) => {
                let status = if app_language() == AppLanguage::English {
                    format!("Next message will use: {name}")
                } else {
                    format!("下一条消息将使用技能：{name}")
                };
                self.ui
                    .label(cx, ids!(chat_skill_status))
                    .set_text(cx, &status);
                self.ui
                    .widget(cx, ids!(chat_skill_clear))
                    .set_visible(cx, true);
            }
            None => {
                self.ui
                    .label(cx, ids!(chat_skill_status))
                    .set_text(cx, tr("no_skill_selected"));
                self.ui
                    .widget(cx, ids!(chat_skill_clear))
                    .set_visible(cx, false);
            }
        }
    }

    fn use_skill_next(&mut self, cx: &mut Cx, skill: &readwise_core::skills::models::Skill) {
        if skill.is_system || !skill.is_active {
            return;
        }
        self.pending_skill_id = Some(skill.id.clone());
        self.refresh_pending_skill_indicator(cx);
        self.activate_page(cx, AppPage::Chat);
        self.set_chat_status(
            cx,
            &if app_language() == AppLanguage::English {
                format!("Selected “{}”; enter the content to process", skill.name)
            } else {
                format!("已选择「{}」，请输入要处理的内容", skill.name)
            },
        );
        self.ui.redraw(cx);
    }

    fn stop_ai_timeout(&mut self, cx: &mut Cx) {
        if self.ai_timeout.0 != 0 {
            cx.stop_timer(self.ai_timeout);
            self.ai_timeout = Timer::default();
        }
    }

    fn decode_ai_stream_event(&mut self, event: &Event, request: RequestId) -> Vec<AiSseUpdate> {
        let Event::NetworkResponses(responses) = event else {
            return Vec::new();
        };
        let mut updates = Vec::new();
        for response in responses {
            match response {
                NetworkResponse::HttpStreamChunk {
                    request_id,
                    response,
                } if *request_id == request.0 => {
                    if let Some(bytes) = response.get_body() {
                        updates.extend(self.ai_stream.push(bytes));
                    }
                }
                NetworkResponse::HttpStreamComplete { request_id, .. }
                    if *request_id == request.0 =>
                {
                    updates.extend(self.ai_stream.finish());
                }
                _ => {}
            }
        }
        updates
    }

    fn set_chat_status(&self, cx: &mut Cx, text: &str) {
        self.ui.label(cx, ids!(chat_status)).set_text(cx, text);
        self.ui
            .label(cx, ids!(reader_chat_status))
            .set_text(cx, text);
    }

    fn set_chat_streaming_controls(&self, cx: &mut Cx, visible: bool) {
        self.ui
            .widget(cx, ids!(chat_cancel))
            .set_visible(cx, visible);
        self.ui
            .widget(cx, ids!(reader_chat_cancel))
            .set_visible(cx, visible);
    }

    fn set_reader_ai_visible(&mut self, cx: &mut Cx, visible: bool) {
        self.reader_ai_open = visible;
        if visible {
            self.toc_open = false;
            self.note_open = false;
        }
        self.sync_reader_panel_layout(cx);
        self.ui.redraw(cx);
    }

    fn configure_ai_backend(&mut self, cx: &mut Cx) {
        if self.ai_request.is_some() {
            self.cancel_chat(cx);
        }
        let (endpoint, model, api_key, is_local) = if let Some(session) = &self.local_ai_session {
            (
                session.endpoint(),
                readwise_core::local_ai::LOCAL_MODEL_ALIAS.to_string(),
                String::new(),
                true,
            )
        } else {
            let endpoint =
                match readwise_core::ai::normalize_chat_endpoint(&self.ai_settings.endpoint) {
                    Ok(endpoint) => endpoint,
                    Err(error) => {
                        self.ai_backend = None;
                        self.set_chat_status(cx, &error);
                        return;
                    }
                };
            (
                endpoint,
                self.ai_settings.model.trim().to_string(),
                self.ai_settings.api_key.clone(),
                false,
            )
        };
        if model.is_empty() {
            self.ai_backend = None;
            self.set_chat_status(cx, tr("model_name_required"));
            return;
        }
        self.ai_backend = Some(OpenAiBackend::new(BackendConfig::OpenAI {
            api_key,
            model: model.clone(),
            base_url: Some(endpoint.clone()),
            reasoning_effort: None,
        }));
        let model_kind = if is_local {
            if app_language() == AppLanguage::English {
                "Local model"
            } else {
                "本地模型"
            }
        } else if app_language() == AppLanguage::English {
            "Remote model"
        } else {
            "远程模型"
        };
        self.set_chat_status(
            cx,
            &if app_language() == AppLanguage::English {
                format!("{model_kind} ready · {model} · {endpoint}")
            } else {
                format!("{model_kind}已就绪 · {model} · {endpoint}")
            },
        );
    }

    fn redraw_chat(&self, cx: &mut Cx) {
        let main_widget = self.ui.widget(cx, ids!(chat_list));
        let reader_widget = self.ui.widget(cx, ids!(reader_chat_list));
        let main_list = main_widget.portal_list(cx, ids!(list));
        let reader_list = reader_widget.portal_list(cx, ids!(list));
        let items = {
            let data = CHAT_DATA.read().unwrap();
            data.messages.len() + usize::from(data.is_streaming)
        };
        if items > 0 {
            main_list.set_tail_range(true);
            main_list.set_first_id_and_scroll(items - 1, 0.0);
            reader_list.set_tail_range(true);
            reader_list.set_first_id_and_scroll(items - 1, 0.0);
        }
        main_widget.redraw(cx);
        reader_widget.redraw(cx);
    }

    fn send_chat(&mut self, cx: &mut Cx) {
        let text = self.ui.text_input(cx, ids!(chat_input)).text();
        if self.send_chat_text(cx, text) {
            self.ui.text_input(cx, ids!(chat_input)).set_text(cx, "");
        }
    }

    fn send_reader_chat(&mut self, cx: &mut Cx) {
        let text = self.ui.text_input(cx, ids!(reader_chat_input)).text();
        if self.send_chat_text(cx, text) {
            self.ui
                .text_input(cx, ids!(reader_chat_input))
                .set_text(cx, "");
        }
    }

    fn send_chat_text(&mut self, cx: &mut Cx, text: String) -> bool {
        if self.ai_request.is_some() {
            self.set_chat_status(cx, tr("answer_in_progress"));
            return false;
        }
        if text.trim().is_empty() {
            return false;
        }
        if self.ai_backend.is_none() {
            self.set_chat_status(cx, tr("ai_not_configured"));
            return false;
        }
        if self.chat_thread_id.is_none() {
            self.chat_thread_id = Some(readwise_core::threads::new_thread_id());
        }

        let pending_skill = self.pending_skill_id.as_ref().and_then(|id| {
            SKILLS
                .read()
                .unwrap()
                .entries
                .iter()
                .find(|skill| &skill.id == id && skill.is_active && !skill.is_system)
                .cloned()
        });
        let system_prompt = active_system_prompt();
        let visible_text = text.trim().to_string();
        let request_text = pending_skill.as_ref().map_or_else(
            || visible_text.clone(),
            |skill| {
                if app_language() == AppLanguage::English {
                    format!(
                        "Follow the skill template below when completing the request.\n\n[Skill: {}]\n{}\n\n[User request]\n{}",
                        skill.name, skill.content, visible_text
                    )
                } else {
                    format!(
                        "请严格按照下面的技能模板完成用户请求。\n\n【技能：{}】\n{}\n\n【用户请求】\n{}",
                        skill.name, skill.content, visible_text
                    )
                }
            },
        );

        let request_messages = {
            let mut data = CHAT_DATA.write().unwrap();
            data.messages.push(ChatMessage {
                role: ChatRole::User,
                text: visible_text,
            });
            data.streaming_text.clear();
            data.is_streaming = true;
            data.messages
                .iter()
                .enumerate()
                .map(|(index, message)| match message.role {
                    ChatRole::User if index + 1 == data.messages.len() => {
                        Message::user(&request_text)
                    }
                    ChatRole::User => Message::user(&message.text),
                    ChatRole::Assistant => Message::assistant(&message.text),
                })
                .collect()
        };
        self.ai_stream.reset();
        let backend = self.ai_backend.as_mut().unwrap();
        self.ai_request = Some(backend.send_request(
            cx,
            AiRequest {
                messages: request_messages,
                system_prompt: Some(system_prompt),
                ..Default::default()
            },
        ));
        if pending_skill.is_some() {
            self.pending_skill_id = None;
            self.refresh_pending_skill_indicator(cx);
        }
        self.ai_timeout = cx.start_timeout(60.0);
        self.set_chat_streaming_controls(cx, true);
        self.set_chat_status(cx, tr("generating"));
        self.redraw_chat(cx);
        true
    }

    fn cancel_chat(&mut self, cx: &mut Cx) {
        self.stop_ai_timeout(cx);
        let Some(request_id) = self.ai_request.take() else {
            return;
        };
        if let Some(backend) = &mut self.ai_backend {
            backend.cancel_request(cx, request_id);
        }
        let messages = {
            let mut data = CHAT_DATA.write().unwrap();
            let partial = std::mem::take(&mut data.streaming_text);
            if !partial.is_empty() {
                data.messages.push(ChatMessage {
                    role: ChatRole::Assistant,
                    text: partial,
                });
            }
            data.is_streaming = false;
            data.messages.clone()
        };
        spawn_save_chat(self.chat_thread_id.clone(), messages);
        self.set_chat_streaming_controls(cx, false);
        self.set_chat_status(cx, tr("answer_stopped"));
        self.ai_stream.reset();
        self.redraw_chat(cx);
    }

    fn clear_chat(&mut self, cx: &mut Cx) {
        self.cancel_chat(cx);
        *CHAT_DATA.write().unwrap() = ChatData::default();
        self.chat_thread_id = None;
        self.set_chat_status(cx, tr("new_chat_ready"));
        self.ui.redraw(cx);
    }

    fn finish_chat(&mut self, cx: &mut Cx, fallback_text: String) {
        self.stop_ai_timeout(cx);
        let messages = {
            let mut data = CHAT_DATA.write().unwrap();
            let mut answer = std::mem::take(&mut data.streaming_text);
            if answer.is_empty() {
                answer = fallback_text;
            }
            if !answer.is_empty() {
                data.messages.push(ChatMessage {
                    role: ChatRole::Assistant,
                    text: answer,
                });
            }
            data.is_streaming = false;
            data.messages.clone()
        };
        self.ai_request = None;
        self.ai_stream.reset();
        self.set_chat_streaming_controls(cx, false);
        self.set_chat_status(cx, tr("answer_saving"));
        spawn_save_chat(self.chat_thread_id.clone(), messages);
        self.redraw_chat(cx);
    }

    fn fail_chat(&mut self, cx: &mut Cx, error: String) {
        self.stop_ai_timeout(cx);
        if let Some(request_id) = self.ai_request.take() {
            if let Some(backend) = &mut self.ai_backend {
                backend.cancel_request(cx, request_id);
            }
        }
        let messages = {
            let mut data = CHAT_DATA.write().unwrap();
            let partial = std::mem::take(&mut data.streaming_text);
            if !partial.is_empty() {
                data.messages.push(ChatMessage {
                    role: ChatRole::Assistant,
                    text: partial,
                });
            }
            data.is_streaming = false;
            data.messages.clone()
        };
        self.set_chat_streaming_controls(cx, false);
        self.set_chat_status(
            cx,
            &if app_language() == AppLanguage::English {
                format!("Request failed: {error}")
            } else {
                format!("请求失败：{error}")
            },
        );
        self.ai_stream.reset();
        spawn_save_chat(self.chat_thread_id.clone(), messages);
        self.redraw_chat(cx);
    }

    /// 切换到指定章节：按 </p> 切块喂给 PortalList，更新标题/页码，保存进度，滚回顶部。
    fn set_chapter(&mut self, cx: &mut Cx, idx: usize) {
        let (id, title, total, chunks) = {
            let mut guard = CURRENT_BOOK.write().unwrap();
            let Some(book) = guard.as_mut() else { return };
            if idx >= book.chapters.len() {
                return;
            }
            let Some(chunks) = build_chapter_chunks(book, idx) else {
                return;
            };
            (
                book.id.clone(),
                book.title.clone(),
                book.chapters.len(),
                chunks,
            )
        };
        self.clear_reader_selection(cx);
        self.close_note_composer(cx);
        self.chapter_idx = idx;
        CURRENT_CHAPTER.store(idx, std::sync::atomic::Ordering::Relaxed);
        *PARAGRAPHS.write().unwrap() = chunks;
        self.ui
            .label(cx, ids!(title))
            .set_text(cx, &format!("《{}》", title));
        self.ui
            .label(cx, ids!(chapter_pos))
            .set_text(cx, &format!("{}/{}", idx + 1, total));
        self.ui
            .widget(cx, ids!(chapter_list))
            .portal_list(cx, ids!(list))
            .set_first_id_and_scroll(0, 0.0);
        spawn_save_progress(id, idx, total);
        self.ui.redraw(cx);
    }

    /// 章内进度标签：`第x/y章 · z%`（z 按已读到的块估算）。
    fn update_progress_label(&mut self, cx: &mut Cx) {
        let total_ch = {
            let guard = CURRENT_BOOK.read().unwrap();
            let Some(book) = guard.as_ref() else { return };
            book.chapters.len()
        };
        let list = self
            .ui
            .widget(cx, ids!(chapter_list))
            .portal_list(cx, ids!(list));
        let blocks = PARAGRAPHS.read().unwrap().len().max(1);
        let read_to = (list.first_id() + list.visible_items()).min(blocks);
        let pct = read_to * 100 / blocks;
        let text = format!("{}/{} · {}%", self.chapter_idx + 1, total_ch, pct);
        self.ui.label(cx, ids!(chapter_pos)).set_text(cx, &text);
    }

    /// 视口翻屏；到底/到顶时跨章。
    fn page_scroll(&mut self, cx: &mut Cx, down: bool) {
        if CURRENT_BOOK.read().unwrap().is_none() {
            return;
        }
        let list = self
            .ui
            .widget(cx, ids!(chapter_list))
            .portal_list(cx, ids!(list));
        let total = PARAGRAPHS.read().unwrap().len();
        let first = list.first_id();
        let vis = list.visible_items().max(1);
        let step = vis.saturating_sub(1).max(1);
        if down {
            if first + vis >= total {
                let idx = self.chapter_idx + 1;
                self.set_chapter(cx, idx);
            } else {
                list.set_first_id_and_scroll((first + step).min(total - 1), 0.0);
                self.ui.redraw(cx);
            }
        } else if first == 0 {
            if self.chapter_idx > 0 {
                let idx = self.chapter_idx - 1;
                self.set_chapter(cx, idx);
            }
        } else {
            list.set_first_id_and_scroll(first.saturating_sub(step), 0.0);
            self.ui.redraw(cx);
        }
        self.update_progress_label(cx);
    }

    /// 阅读页快捷键：空格/PgDn/→ 下一屏，PgUp/← 上一屏，Shift+←/→ 跨章，Home 回章首。
    fn handle_reader_key(&mut self, cx: &mut Cx, ke: &KeyEvent) {
        if !self.on_reader_page
            || CURRENT_BOOK.read().unwrap().is_none()
            || self.text_input_has_focus(cx)
        {
            return;
        }
        let shift = ke.modifiers.shift;
        match ke.key_code {
            KeyCode::Space | KeyCode::PageDown => self.page_scroll(cx, true),
            KeyCode::PageUp => self.page_scroll(cx, false),
            KeyCode::ArrowRight if shift => {
                let idx = self.chapter_idx + 1;
                self.set_chapter(cx, idx);
            }
            KeyCode::ArrowLeft if shift => {
                if self.chapter_idx > 0 {
                    let idx = self.chapter_idx - 1;
                    self.set_chapter(cx, idx);
                }
            }
            KeyCode::ArrowRight => self.page_scroll(cx, true),
            KeyCode::ArrowLeft => self.page_scroll(cx, false),
            KeyCode::Home => {
                self.ui
                    .widget(cx, ids!(chapter_list))
                    .portal_list(cx, ids!(list))
                    .set_first_id_and_scroll(0, 0.0);
                self.ui.redraw(cx);
                self.update_progress_label(cx);
            }
            _ => {}
        }
    }

    /// 调整阅读字号（clamp 到 [8, 20]），设置页标签同步。
    fn adjust_font(&mut self, cx: &mut Cx, delta: f32) {
        let new_size = {
            let mut guard = READER_FONT_SIZE.write().unwrap();
            let cur = guard.unwrap_or(READER_FONT_DEFAULT);
            let v = (cur + delta).clamp(READER_FONT_MIN, READER_FONT_MAX);
            *guard = Some(v);
            v
        };
        self.ui
            .label(cx, ids!(font_size_label))
            .set_text(cx, &format!("{:.0}", new_size));
        self.ui.redraw(cx);
        self.persist_reader_settings(cx);
    }

    /// 把一个色板格子的底色刷成预设色（apply 片段只认字面量，逐色枚举）。
    fn paint_swatch(&mut self, cx: &mut Cx, cell: &[LiveId], slot: usize) {
        let mut w = self.ui.widget(cx, cell);
        match slot {
            1 => script_apply_eval!(cx, w, { draw_bg.color: #x7fb2a6 }),
            2 => script_apply_eval!(cx, w, { draw_bg.color: #x62b6ff }),
            3 => script_apply_eval!(cx, w, { draw_bg.color: #xe8875a }),
            4 => script_apply_eval!(cx, w, { draw_bg.color: #x9a9a9a }),
            5 => script_apply_eval!(cx, w, { draw_bg.color: #x5c5c5c }),
            6 => script_apply_eval!(cx, w, { draw_bg.color: #x4d4d4d }),
            7 => script_apply_eval!(cx, w, { draw_bg.color: #x294f45 }),
            8 => script_apply_eval!(cx, w, { draw_bg.color: #x293d61 }),
            9 => script_apply_eval!(cx, w, { draw_bg.color: #xd9c96a }),
            _ => {
                let color = self.swatch_inactive;
                script_apply_eval!(cx, w, { draw_bg.color: #(color) });
            }
        }
    }

    /// 按当前配置刷新四行色板的选中态。
    /// slot 约定：0=未选底灰 1=青 2=蓝 3=橙 4=灰 5=中性选中 6=深灰 7=深青 8=深蓝
    fn refresh_swatches(&mut self, cx: &mut Cx) {
        use crate::ruby_word::*;
        let color_slot = [1usize, 2, 3, 4];
        let bg_slot = [5usize, 6, 7, 8];

        let gloss = *GLOSS_COLOR.read().unwrap();
        let cells = [ids!(gc0), ids!(gc1), ids!(gc2), ids!(gc3)];
        for (i, cell) in cells.into_iter().enumerate() {
            let slot = if GLOSS_PRESETS[i].1 == gloss {
                color_slot[i]
            } else {
                0
            };
            self.paint_swatch(cx, cell, slot);
        }

        let gbg = *GLOSS_BG.read().unwrap();
        let cells = [ids!(gb0), ids!(gb1), ids!(gb2), ids!(gb3)];
        for (i, cell) in cells.into_iter().enumerate() {
            let slot = if BG_PRESETS[i].1 == gbg {
                bg_slot[i]
            } else {
                0
            };
            self.paint_swatch(cx, cell, slot);
        }

        let wcol = *WORD_COLOR.read().unwrap();
        let cells = [ids!(wc0), ids!(wc1), ids!(wc2), ids!(wc3)];
        let wc_slot = [5usize, 1, 2, 3];
        for (i, cell) in cells.into_iter().enumerate() {
            let slot = if WORD_COLOR_PRESETS[i].1 == wcol {
                wc_slot[i]
            } else {
                0
            };
            self.paint_swatch(cx, cell, slot);
        }

        let wbg = *WORD_BG.read().unwrap();
        let cells = [ids!(wb0), ids!(wb1), ids!(wb2), ids!(wb3)];
        for (i, cell) in cells.into_iter().enumerate() {
            let slot = if BG_PRESETS[i].1 == wbg {
                bg_slot[i]
            } else {
                0
            };
            self.paint_swatch(cx, cell, slot);
        }

        let sel = *SELECTION_SLOT.read().unwrap();
        let cells = [ids!(sc0), ids!(sc1), ids!(sc2), ids!(sc3)];
        let sel_slot = [1usize, 2, 3, 9];
        for (i, cell) in cells.into_iter().enumerate() {
            let slot = if i == sel { sel_slot[i] } else { 0 };
            self.paint_swatch(cx, cell, slot);
        }

        let hl = *crate::mark_span::HL_SLOT.read().unwrap();
        let cells = [ids!(hc0), ids!(hc1), ids!(hc2), ids!(hc3)];
        let hl_slot = [9usize, 1, 2, 3];
        for (i, cell) in cells.into_iter().enumerate() {
            let slot = if i == hl { hl_slot[i] } else { 0 };
            self.paint_swatch(cx, cell, slot);
        }
    }

    fn apply_reader_settings(&mut self, cx: &mut Cx, settings: ReaderSettings) {
        use crate::ruby_word::*;

        let settings = settings.normalized();
        let theme_changed = *READER_THEME.read().unwrap() != settings.theme;
        *APP_LANGUAGE.write().unwrap() = settings.language;
        *READER_THEME.write().unwrap() = settings.theme;
        *READER_FONT.write().unwrap() = settings.font;
        *READER_SPACING.write().unwrap() = settings.spacing;
        *READER_FONT_SIZE.write().unwrap() = settings.font_size;
        *GLOSS_ENABLED.write().unwrap() = settings.gloss_enabled;
        *GLOSS_FONT_SIZE.write().unwrap() = settings.gloss_font_size;
        *GLOSS_COLOR.write().unwrap() = GLOSS_PRESETS[settings.gloss_color_slot].1;
        *GLOSS_BG.write().unwrap() = BG_PRESETS[settings.gloss_background_slot].1;
        *WORD_COLOR.write().unwrap() = WORD_COLOR_PRESETS[settings.word_color_slot].1;
        *WORD_BG.write().unwrap() = BG_PRESETS[settings.word_background_slot].1;
        *SELECTION_SLOT.write().unwrap() = settings.selection_color_slot;
        *crate::mark_span::HL_SLOT.write().unwrap() = settings.highlight_color_slot;

        self.ui.label(cx, ids!(font_size_label)).set_text(
            cx,
            &settings
                .font_size
                .map(|value| format!("{value:.0}"))
                .unwrap_or_else(|| tr("default").to_string()),
        );
        self.ui
            .label(cx, ids!(gloss_size_label))
            .set_text(cx, &format!("{:.1}", settings.gloss_font_size));
        self.ui.check_box(cx, ids!(gloss_enable)).set_active(
            cx,
            settings.gloss_enabled,
            Animate::No,
        );
        self.sync_theme_controls(cx, settings.theme);
        self.sync_app_language_controls(cx, settings.language);
        self.apply_ui_language(cx, settings.language);
        self.sync_reader_font_controls(cx, settings.font);
        self.sync_reader_spacing_controls(cx, settings.spacing);
        self.set_gloss_rows_visible(cx, settings.gloss_enabled);
        self.refresh_swatches(cx);

        self.refresh_current_chapter(cx);
        if theme_changed {
            self.apply_runtime_theme(cx, settings.theme);
        }
    }

    fn persist_reader_settings(&mut self, cx: &mut Cx) {
        self.ui.label(cx, ids!(settings_hint)).set_text(
            cx,
            localized("正在保存阅读设置…", "Saving reading settings…"),
        );
        spawn_save_reader_settings(current_reader_settings());
    }

    /// 用当前章内容重建 PortalList 块（保滚动位置：块数不变时 first_id 不动）。
    fn refresh_current_chapter(&mut self, cx: &mut Cx) {
        let chunks = {
            let mut guard = CURRENT_BOOK.write().unwrap();
            guard
                .as_mut()
                .and_then(|b| build_chapter_chunks(b, self.chapter_idx))
        };
        if let Some(chunks) = chunks {
            *PARAGRAPHS.write().unwrap() = chunks;
        }
        self.ui.redraw(cx);
    }

    fn clear_reader_selection(&mut self, cx: &mut Cx) {
        *SEL_RANGE.write().unwrap() = None;
        SELECTED_TEXT.write().unwrap().clear();
        self.ui.widget(cx, ids!(sel_bar)).set_visible(cx, false);
        let list = self
            .ui
            .widget(cx, ids!(chapter_list))
            .portal_list(cx, ids!(list));
        if let Some(mut inner) = list.borrow_mut() {
            inner.clear_selection(cx);
        };
    }

    fn replace_notes(&self, cx: &mut Cx, notes: Vec<readwise_core::notes::models::Note>) {
        let is_empty = notes.is_empty();
        NOTES.write().unwrap().replace(notes);
        self.ui
            .widget(cx, ids!(note_empty))
            .set_visible(cx, is_empty);
        let note_list = self.ui.widget(cx, ids!(note_list));
        note_list.set_visible(cx, !is_empty);
        note_list
            .portal_list(cx, ids!(list))
            .set_first_id_and_scroll(0, 0.0);
        note_list.redraw(cx);
    }

    fn open_note_composer(&mut self, cx: &mut Cx) {
        let quote = SELECTED_TEXT.read().unwrap().trim().to_string();
        if quote.is_empty() {
            return;
        }
        self.note_quote_text = quote;
        self.note_open = true;
        self.toc_open = false;
        self.reader_ai_open = false;
        self.sync_reader_panel_layout(cx);
        self.ui
            .widget(cx, ids!(note_composer))
            .set_visible(cx, true);
        self.ui
            .label(cx, ids!(note_quote))
            .set_text(cx, &self.note_quote_text);
        self.ui.text_input(cx, ids!(note_extra)).set_text(cx, "");
        self.ui.label(cx, ids!(note_hint)).set_text(
            cx,
            localized(
                "补充内容可留空；保存后引用会一并进入笔记。",
                "Extra text is optional; the quote is saved with the note.",
            ),
        );
        self.ui.widget(cx, ids!(sel_bar)).set_visible(cx, false);
        self.ui.redraw(cx);
    }

    fn close_note_composer(&mut self, cx: &mut Cx) {
        self.note_quote_text.clear();
        self.ui
            .widget(cx, ids!(note_composer))
            .set_visible(cx, false);
        self.ui.text_input(cx, ids!(note_extra)).set_text(cx, "");
        self.ui.redraw(cx);
    }

    fn save_note_from_composer(&mut self, cx: &mut Cx, extra: String) {
        let Some(item) = CURRENT_ITEM.read().unwrap().clone() else {
            self.ui.label(cx, ids!(note_hint)).set_text(
                cx,
                localized("请先从书库打开一本书", "Open a book from the Library first"),
            );
            return;
        };
        if self.note_quote_text.trim().is_empty() && extra.trim().is_empty() {
            return;
        }
        self.ui
            .label(cx, ids!(note_hint))
            .set_text(cx, localized("保存中…", "Saving…"));
        spawn_create_note(item, self.note_quote_text.clone(), extra);
    }

    /// 注音相关配置行的显隐（跟随总开关）。
    fn set_gloss_rows_visible(&mut self, cx: &mut Cx, visible: bool) {
        for row in [
            ids!(gloss_row),
            ids!(gloss_color_row),
            ids!(gloss_bg_row),
            ids!(word_color_row),
            ids!(word_bg_row),
        ] {
            self.ui.widget(cx, row).set_visible(cx, visible);
        }
    }

    /// 注音总开关切换：保留原始章节，打开时只惰性生成当前章注音。
    fn toggle_gloss(&mut self, cx: &mut Cx, on: bool) {
        *GLOSS_ENABLED.write().unwrap() = on;
        self.set_gloss_rows_visible(cx, on);
        self.refresh_current_chapter(cx);
        self.ui.redraw(cx);
        self.persist_reader_settings(cx);
    }

    /// 四行色板的点击分发。
    fn handle_swatch_rows(&mut self, cx: &mut Cx, actions: &Actions) {
        use crate::ruby_word::*;
        let mut changed = false;
        let gc = [ids!(gc0.b), ids!(gc1.b), ids!(gc2.b), ids!(gc3.b)];
        for (i, cell) in gc.into_iter().enumerate() {
            if self.ui.button(cx, cell).clicked(actions) {
                *GLOSS_COLOR.write().unwrap() = GLOSS_PRESETS[i].1;
                changed = true;
            }
        }
        let gb = [ids!(gb0.b), ids!(gb1.b), ids!(gb2.b), ids!(gb3.b)];
        for (i, cell) in gb.into_iter().enumerate() {
            if self.ui.button(cx, cell).clicked(actions) {
                *GLOSS_BG.write().unwrap() = BG_PRESETS[i].1;
                changed = true;
            }
        }
        let wc = [ids!(wc0.b), ids!(wc1.b), ids!(wc2.b), ids!(wc3.b)];
        for (i, cell) in wc.into_iter().enumerate() {
            if self.ui.button(cx, cell).clicked(actions) {
                *WORD_COLOR.write().unwrap() = WORD_COLOR_PRESETS[i].1;
                changed = true;
            }
        }
        let wb = [ids!(wb0.b), ids!(wb1.b), ids!(wb2.b), ids!(wb3.b)];
        for (i, cell) in wb.into_iter().enumerate() {
            if self.ui.button(cx, cell).clicked(actions) {
                *WORD_BG.write().unwrap() = BG_PRESETS[i].1;
                changed = true;
            }
        }
        let sc = [ids!(sc0.b), ids!(sc1.b), ids!(sc2.b), ids!(sc3.b)];
        for (i, cell) in sc.into_iter().enumerate() {
            if self.ui.button(cx, cell).clicked(actions) {
                *SELECTION_SLOT.write().unwrap() = i;
                changed = true;
            }
        }
        let hc = [ids!(hc0.b), ids!(hc1.b), ids!(hc2.b), ids!(hc3.b)];
        for (i, cell) in hc.into_iter().enumerate() {
            if self.ui.button(cx, cell).clicked(actions) {
                *crate::mark_span::HL_SLOT.write().unwrap() = i;
                changed = true;
            }
        }
        if changed {
            self.refresh_swatches(cx);
            self.ui.redraw(cx);
            self.persist_reader_settings(cx);
        }
    }

    /// 调整注音字号（clamp 到 [6, 12]）。
    fn adjust_gloss_font(&mut self, cx: &mut Cx, delta: f32) {
        let new_size = {
            let mut guard = crate::ruby_word::GLOSS_FONT_SIZE.write().unwrap();
            *guard = (*guard + delta).clamp(6.0, 12.0);
            *guard
        };
        self.ui
            .label(cx, ids!(gloss_size_label))
            .set_text(cx, &format!("{:.1}", new_size));
        self.ui.redraw(cx);
        self.persist_reader_settings(cx);
    }
}

impl MatchEvent for App {
    fn handle_startup(&mut self, _cx: &mut Cx) {
        self.app_in_foreground = true;
    }

    fn handle_foreground(&mut self, cx: &mut Cx) {
        self.app_in_foreground = true;
        self.start_reading_timer(cx);
    }

    fn handle_resume(&mut self, cx: &mut Cx) {
        self.app_in_foreground = true;
        self.start_reading_timer(cx);
    }

    fn handle_background(&mut self, cx: &mut Cx) {
        self.app_in_foreground = false;
        self.stop_reading_timer(cx, false);
    }

    fn handle_pause(&mut self, cx: &mut Cx) {
        self.app_in_foreground = false;
        self.stop_reading_timer(cx, false);
    }

    fn handle_timer(&mut self, cx: &mut Cx, event: &TimerEvent) {
        if self.reading_checkpoint_timer.is_timer(event).is_some() {
            if let Some(id) = self.reading_session_id.clone() {
                spawn_checkpoint_reading_session(id);
            }
        }
        if self.ai_request.is_some() && self.ai_timeout.is_timer(event).is_some() {
            self.fail_chat(
                cx,
                localized(
                    "请求超过 60 秒仍未完成，请检查网络或接口设置",
                    "The request did not finish within 60 seconds; check the network and API settings",
                )
                .to_string(),
            );
        }
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        if self.local_ai_session.is_some() && self.local_ai_poll_timer.is_timer(event).is_some() {
            let exit = LOCAL_AI_MANAGER
                .lock()
                .map_err(|_| "本地 AI 进程状态锁已损坏".to_string())
                .and_then(|mut manager| manager.poll_exit());
            match exit {
                Ok(Some(message)) => {
                    self.local_ai_session = None;
                    cx.stop_timer(self.local_ai_poll_timer);
                    self.local_ai_poll_timer = Timer::default();
                    self.set_local_ai_running(cx, false);
                    self.ui.label(cx, ids!(local_ai_status)).set_text(
                        cx,
                        &if app_language() == AppLanguage::English {
                            format!("{message}; chat switched back to the remote API")
                        } else {
                            format!("{message}，聊天已恢复远程 API")
                        },
                    );
                    self.configure_ai_backend(cx);
                }
                Ok(None) => {}
                Err(error) => self
                    .ui
                    .label(cx, ids!(local_ai_status))
                    .set_text(cx, &error),
            }
        }
    }

    fn handle_shutdown(&mut self, _cx: &mut Cx) {
        if let Some(id) = self.reading_session_id.take() {
            if let Some(db) = DB.get() {
                let _ = db.finish_reading_session(&id);
            }
        }
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            LOCAL_AI_CANCEL.store(true, Ordering::Relaxed);
            if let Ok(mut manager) = LOCAL_AI_MANAGER.lock() {
                let _ = manager.stop();
            }
        }
    }

    fn handle_actions(&mut self, cx: &mut Cx, actions: &Actions) {
        for action in actions {
            #[cfg(target_os = "ios")]
            if let Some(import_action) =
                action.downcast_ref::<crate::mobile_import::IosImportAction>()
            {
                match import_action {
                    crate::mobile_import::IosImportAction::Picked(paths) => {
                        self.ui.label(cx, ids!(library_hint)).set_text(
                            cx,
                            localized("正在导入所选 EPUB…", "Importing selected EPUB files…"),
                        );
                        spawn_import(paths.clone(), true);
                    }
                    crate::mobile_import::IosImportAction::Cancelled => {
                        self.ui
                            .label(cx, ids!(library_hint))
                            .set_text(cx, localized("已取消导入", "Import cancelled"));
                    }
                    crate::mobile_import::IosImportAction::Error(error) => {
                        self.ui.label(cx, ids!(library_hint)).set_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!("Import failed: {error}")
                            } else {
                                format!("导入失败：{error}")
                            },
                        );
                    }
                }
                continue;
            }
            if let Some(db_action) = action.downcast_ref::<DbAction>() {
                match db_action {
                    DbAction::Ready {
                        vocab_count,
                        known_count,
                        reader_settings,
                    } => {
                        if let Some(elapsed) = startup_elapsed_ms() {
                            ::log::info!("Startup: database and settings ready in {elapsed}ms");
                        }
                        let status = if app_language() == AppLanguage::English {
                            format!(
                                "Database ready\nVocabulary {} · Known {}",
                                vocab_count, known_count
                            )
                        } else {
                            format!(
                                "数据库就绪\n生词本 {} · 已认识 {}",
                                vocab_count, known_count
                            )
                        };
                        self.ui.label(cx, ids!(db_status)).set_text(cx, &status);
                        let summary = if app_language() == AppLanguage::English {
                            format!("{vocab_count} words; loading list…")
                        } else {
                            format!("共 {vocab_count} 词，正在加载列表…")
                        };
                        self.ui
                            .label(cx, ids!(vocab_summary))
                            .set_text(cx, &summary);
                        self.apply_reader_settings(cx, reader_settings.clone());
                        self.ui.label(cx, ids!(settings_hint)).set_text(
                            cx,
                            localized(
                                "阅读设置已从本机恢复，更改后会自动保存。",
                                "Reading settings restored; changes are saved automatically.",
                            ),
                        );
                        spawn_load_shelf();
                        spawn_load_ai_state();
                        spawn_load_skills();
                        spawn_load_local_ai_settings();
                        spawn_load_vocab();
                    }
                    DbAction::Failed(e) => {
                        let status = if app_language() == AppLanguage::English {
                            format!("Database failed:\n{e}")
                        } else {
                            format!("数据库失败:\n{e}")
                        };
                        self.ui.label(cx, ids!(db_status)).set_text(cx, &status);
                    }
                }
                continue;
            }
            if let Some(settings_action) = action.downcast_ref::<ReaderSettingsAction>() {
                match settings_action {
                    ReaderSettingsAction::Saved => {
                        self.ui.label(cx, ids!(settings_hint)).set_text(
                            cx,
                            localized(
                                "阅读设置已自动保存在本机。",
                                "Reading settings saved on this device.",
                            ),
                        );
                    }
                    ReaderSettingsAction::Error(error) => {
                        self.ui.label(cx, ids!(settings_hint)).set_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!("Failed to save reading settings: {error}")
                            } else {
                                format!("阅读设置保存失败：{error}")
                            },
                        );
                    }
                }
                continue;
            }
            if let Some(session_action) = action.downcast_ref::<ReadingSessionAction>() {
                match session_action {
                    ReadingSessionAction::Started { id, book_id } => {
                        self.reading_session_pending = false;
                        let invalidated = std::mem::take(&mut self.reading_session_invalidated);
                        let is_current_book = CURRENT_BOOK
                            .read()
                            .unwrap()
                            .as_ref()
                            .is_some_and(|book| book.id == *book_id);
                        if should_keep_started_reading_session(
                            invalidated,
                            self.app_in_foreground,
                            self.on_reader_page,
                            is_current_book,
                        ) {
                            self.statistics_after_pending_session = false;
                            self.reading_session_id = Some(id.clone());
                            self.reading_checkpoint_timer = cx.start_interval(30.0);
                            self.ui.label(cx, ids!(status)).set_text(
                                cx,
                                localized("阅读时长已开始记录", "Reading time tracking started"),
                            );
                        } else {
                            let load_statistics = self.statistics_after_pending_session;
                            self.statistics_after_pending_session = false;
                            spawn_finish_reading_session(id.clone(), load_statistics);
                            if self.app_in_foreground && self.on_reader_page && is_current_book {
                                self.start_reading_timer(cx);
                            }
                        }
                    }
                    ReadingSessionAction::Error(error) => {
                        self.reading_session_pending = false;
                        self.reading_session_invalidated = false;
                        if self.statistics_after_pending_session {
                            self.statistics_after_pending_session = false;
                            spawn_load_statistics();
                        }
                        ::log::error!("{error}");
                        if self.on_reader_page {
                            self.ui.label(cx, ids!(status)).set_text(
                                cx,
                                &if app_language() == AppLanguage::English {
                                    format!("Reading timer failed: {error}")
                                } else {
                                    format!("阅读计时失败：{error}")
                                },
                            );
                        }
                    }
                }
                continue;
            }
            if let Some(statistics_action) = action.downcast_ref::<StatisticsAction>() {
                match statistics_action {
                    StatisticsAction::Loaded(snapshot) => self.apply_statistics(cx, snapshot),
                    StatisticsAction::Error(error) => {
                        self.ui.label(cx, ids!(statistics_status)).set_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!("Failed to load statistics: {error}")
                            } else {
                                format!("统计加载失败：{error}")
                            },
                        )
                    }
                }
                continue;
            }
            if let Some(skill_action) = action.downcast_ref::<SkillAction>() {
                match skill_action {
                    SkillAction::Loaded(skills) => {
                        let count = skills.len();
                        SKILLS.write().unwrap().replace(skills.clone());
                        self.ui.label(cx, ids!(skills_status)).set_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!(
                                    "{count} skills · System skills apply automatically; custom skills can be used for the next request"
                                )
                            } else {
                                format!(
                                    "共 {count} 个技能 · 系统技能自动生效，自定义技能可用于下一次提问"
                                )
                            },
                        );
                    }
                    SkillAction::Saved(skills) => {
                        SKILLS.write().unwrap().replace(skills.clone());
                        let pending_still_available =
                            self.pending_skill_id.as_ref().is_some_and(|id| {
                                skills.iter().any(|skill| {
                                    &skill.id == id && skill.is_active && !skill.is_system
                                })
                            });
                        if !pending_still_available {
                            self.pending_skill_id = None;
                        }
                        self.refresh_pending_skill_indicator(cx);
                        self.clear_skill_editor(cx);
                        self.ui.label(cx, ids!(skills_status)).set_text(
                            cx,
                            if app_language() == AppLanguage::English {
                                "Skill saved ✓"
                            } else {
                                "技能已保存 ✓"
                            },
                        );
                    }
                    SkillAction::Toggled(skills) => {
                        SKILLS.write().unwrap().replace(skills.clone());
                        let pending_still_available =
                            self.pending_skill_id.as_ref().is_some_and(|id| {
                                skills.iter().any(|skill| {
                                    &skill.id == id && skill.is_active && !skill.is_system
                                })
                            });
                        if !pending_still_available {
                            self.pending_skill_id = None;
                        }
                        self.refresh_pending_skill_indicator(cx);
                        self.ui.label(cx, ids!(skills_status)).set_text(
                            cx,
                            if app_language() == AppLanguage::English {
                                "Skill status updated ✓"
                            } else {
                                "技能状态已更新 ✓"
                            },
                        );
                    }
                    SkillAction::Deleted { skills, id } => {
                        SKILLS.write().unwrap().replace(skills.clone());
                        if self.editing_skill_id.as_deref() == Some(id.as_str()) {
                            self.clear_skill_editor(cx);
                        }
                        if self.pending_skill_id.as_deref() == Some(id.as_str()) {
                            self.pending_skill_id = None;
                            self.refresh_pending_skill_indicator(cx);
                        }
                        self.ui.label(cx, ids!(skills_status)).set_text(
                            cx,
                            if app_language() == AppLanguage::English {
                                "Skill deleted"
                            } else {
                                "技能已删除"
                            },
                        );
                    }
                    SkillAction::Error(error) => {
                        self.ui.label(cx, ids!(skills_status)).set_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!("Skill operation failed: {error}")
                            } else {
                                format!("技能操作失败：{error}")
                            },
                        );
                    }
                }
                self.ui.redraw(cx);
                continue;
            }
            if let Some(vocab) = action.downcast_ref::<VocabAction>() {
                match vocab {
                    VocabAction::Loaded(entries) => {
                        self.replace_vocab_entries(cx, entries.clone());
                        self.ui.label(cx, ids!(vocab_status_hint)).set_text(
                            cx,
                            if app_language() == AppLanguage::English {
                                "Vocabulary loaded"
                            } else {
                                "生词本已加载"
                            },
                        );
                    }
                    VocabAction::Updated {
                        entries,
                        message,
                        reviewed_lemma,
                    } => {
                        self.replace_vocab_entries(cx, entries.clone());
                        self.ui
                            .label(cx, ids!(vocab_status_hint))
                            .set_text(cx, message);
                        let is_current_review = reviewed_lemma.as_ref().is_some_and(|lemma| {
                            self.vocab_review_queue
                                .get(self.vocab_review_index)
                                .is_some_and(|current| current == lemma)
                        });
                        if is_current_review {
                            self.advance_vocab_review(cx);
                        }
                    }
                    VocabAction::Removed { entries, message } => {
                        let was_reviewing = !self.vocab_review_queue.is_empty();
                        let current_lemma = self
                            .vocab_review_queue
                            .get(self.vocab_review_index)
                            .cloned();
                        self.replace_vocab_entries(cx, entries.clone());
                        self.vocab_review_queue.retain(|lemma| {
                            self.vocab_entries.iter().any(|entry| &entry.lemma == lemma)
                        });
                        if let Some(current_lemma) = current_lemma {
                            self.vocab_review_index = self
                                .vocab_review_queue
                                .iter()
                                .position(|lemma| lemma == &current_lemma)
                                .unwrap_or_else(|| {
                                    self.vocab_review_index
                                        .min(self.vocab_review_queue.len().saturating_sub(1))
                                });
                        }
                        if was_reviewing {
                            if self.vocab_review_queue.is_empty() {
                                self.finish_vocab_review(
                                    cx,
                                    if app_language() == AppLanguage::English {
                                        "Review ended"
                                    } else {
                                        "本轮复习已结束"
                                    },
                                );
                            } else {
                                self.show_vocab_review_card(cx);
                            }
                        }
                        self.ui
                            .label(cx, ids!(vocab_status_hint))
                            .set_text(cx, message);
                    }
                    VocabAction::Error(error) => {
                        self.vocab_review_pending = false;
                        if !self.vocab_review_queue.is_empty() {
                            self.show_vocab_review_card(cx);
                        }
                        self.ui.label(cx, ids!(vocab_status_hint)).set_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!("Vocabulary operation failed: {error}")
                            } else {
                                format!("生词本操作失败：{error}")
                            },
                        );
                    }
                }
                self.ui.redraw(cx);
                continue;
            }
            if let Some(local_ai) = action.downcast_ref::<LocalAiAction>() {
                match local_ai {
                    LocalAiAction::Loaded(settings) => {
                        self.local_ai_settings = settings.clone();
                        self.apply_local_ai_settings_to_ui(cx, settings);
                        self.ui
                            .widget(cx, ids!(local_ai_section))
                            .set_visible(cx, readwise_core::local_ai::LOCAL_AI_SUPPORTED);
                        if settings.auto_start
                            && readwise_core::local_ai::LOCAL_AI_SUPPORTED
                            && !settings.model_path.trim().is_empty()
                        {
                            self.ui.label(cx, ids!(local_ai_status)).set_text(
                                cx,
                                localized(
                                    "正在自动加载本地模型…",
                                    "Loading the local model automatically…",
                                ),
                            );
                            self.set_local_ai_running(cx, true);
                            spawn_start_local_ai(settings.clone());
                        }
                    }
                    LocalAiAction::Saved(settings) => {
                        self.local_ai_settings = settings.clone();
                        self.apply_local_ai_settings_to_ui(cx, settings);
                        self.ui.label(cx, ids!(local_ai_status)).set_text(
                            cx,
                            localized("本地 AI 设置已保存 ✓", "Local AI settings saved ✓"),
                        );
                    }
                    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
                    LocalAiAction::Started { settings, session } => {
                        self.local_ai_settings = settings.clone();
                        self.local_ai_session = Some(session.clone());
                        self.apply_local_ai_settings_to_ui(cx, settings);
                        self.set_local_ai_running(cx, true);
                        if self.local_ai_poll_timer.0 != 0 {
                            cx.stop_timer(self.local_ai_poll_timer);
                        }
                        self.local_ai_poll_timer = cx.start_interval(2.0);
                        self.ui.label(cx, ids!(local_ai_status)).set_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!(
                                    "Local model running · PID {} · Port {} · Chat switched to the local service",
                                    session.pid, session.port
                                )
                            } else {
                                format!(
                                    "本地模型运行中 · PID {} · 端口 {} · 聊天已切换到本地服务",
                                    session.pid, session.port
                                )
                            },
                        );
                        self.configure_ai_backend(cx);
                    }
                    LocalAiAction::Stopped => {
                        if self.local_ai_poll_timer.0 != 0 {
                            cx.stop_timer(self.local_ai_poll_timer);
                            self.local_ai_poll_timer = Timer::default();
                        }
                        self.local_ai_session = None;
                        self.set_local_ai_running(cx, false);
                        self.ui.label(cx, ids!(local_ai_status)).set_text(
                            cx,
                            localized(
                                "本地服务已停止，聊天已恢复远程 API",
                                "Local service stopped; chat switched back to the remote API",
                            ),
                        );
                        self.configure_ai_backend(cx);
                    }
                    LocalAiAction::StartFailed(error) => {
                        self.set_local_ai_running(cx, self.local_ai_session.is_some());
                        self.ui.label(cx, ids!(local_ai_status)).set_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!("Failed to start local model: {error}")
                            } else {
                                format!("本地模型启动失败：{error}")
                            },
                        );
                    }
                    LocalAiAction::Error(error) => {
                        self.ui.label(cx, ids!(local_ai_status)).set_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!("Local AI operation failed: {error}")
                            } else {
                                format!("本地 AI 操作失败：{error}")
                            },
                        );
                    }
                }
                self.ui.redraw(cx);
                continue;
            }
            if let Some(ai_state) = action.downcast_ref::<AiStateAction>() {
                match ai_state {
                    AiStateAction::Loaded {
                        settings,
                        thread_id,
                        messages,
                    } => {
                        self.ai_settings = settings.clone();
                        self.chat_thread_id = thread_id.clone();
                        self.ui
                            .text_input(cx, ids!(ai_endpoint))
                            .set_text(cx, &settings.endpoint);
                        self.ui
                            .text_input(cx, ids!(ai_model))
                            .set_text(cx, &settings.model);
                        self.ui
                            .text_input(cx, ids!(ai_api_key))
                            .set_text(cx, &settings.api_key);
                        *CHAT_DATA.write().unwrap() = ChatData {
                            messages: messages
                                .iter()
                                .map(|message| ChatMessage {
                                    role: match message.role {
                                        readwise_core::threads::models::ThreadRole::User => {
                                            ChatRole::User
                                        }
                                        readwise_core::threads::models::ThreadRole::Assistant => {
                                            ChatRole::Assistant
                                        }
                                    },
                                    text: message.content.clone(),
                                })
                                .collect(),
                            ..Default::default()
                        };
                        self.configure_ai_backend(cx);
                        self.ui.redraw(cx);
                    }
                    AiStateAction::SettingsSaved(settings) => {
                        self.ai_settings = settings.clone();
                        self.ui
                            .text_input(cx, ids!(ai_endpoint))
                            .set_text(cx, &settings.endpoint);
                        self.configure_ai_backend(cx);
                        self.ui
                            .label(cx, ids!(ai_settings_status))
                            .set_text(cx, localized("AI 设置已保存 ✓", "AI settings saved ✓"));
                    }
                    AiStateAction::ThreadSaved(thread_id) => {
                        if self.chat_thread_id.as_deref() == Some(thread_id.as_str()) {
                            self.set_chat_status(cx, tr("answer_saved"));
                        }
                    }
                    AiStateAction::Error(error) => {
                        self.ui.label(cx, ids!(ai_settings_status)).set_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!("AI data operation failed: {error}")
                            } else {
                                format!("AI 数据操作失败：{error}")
                            },
                        );
                        self.set_chat_status(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!("AI data operation failed: {error}")
                            } else {
                                format!("AI 数据操作失败：{error}")
                            },
                        );
                    }
                }
                continue;
            }
            if let Some(card) = action.downcast_ref::<CardAction>() {
                match card {
                    CardAction::VocabAdded(added) => {
                        let hint = if *added {
                            localized("已加入生词本 ✓", "Added to Vocabulary ✓")
                        } else {
                            localized("生词本里已有这个词", "This word is already in Vocabulary")
                        };
                        self.ui.label(cx, ids!(card_hint)).set_text(cx, hint);
                        self.ui.label(cx, ids!(sel_hint)).set_text(cx, hint);
                        if *added {
                            spawn_load_vocab();
                        }
                    }
                    CardAction::MarkedKnown => {
                        self.card = None;
                        self.ui.widget(cx, ids!(word_card)).set_visible(cx, false);
                        self.refresh_current_chapter(cx);
                    }
                }
                continue;
            }
            if let Some(sel) = action.downcast_ref::<SelAction>() {
                let show = matches!(sel, SelAction::Show);
                self.ui.widget(cx, ids!(sel_bar)).set_visible(cx, show);
                if show {
                    let text = SELECTED_TEXT.read().unwrap();
                    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
                    let preview: String = compact.chars().take(72).collect();
                    let suffix = if compact.chars().count() > 72 {
                        "…"
                    } else {
                        ""
                    };
                    self.ui.label(cx, ids!(sel_hint)).set_text(
                        cx,
                        &if app_language() == AppLanguage::English {
                            format!("Selected: {preview}{suffix}")
                        } else {
                            format!("已选择：{preview}{suffix}")
                        },
                    );
                }
                self.ui.redraw(cx);
                continue;
            }
            if let Some(HlAction::Saved(hl, chapter)) = action.downcast_ref::<HlAction>() {
                if let Some(book) = CURRENT_BOOK.write().unwrap().as_mut() {
                    book.highlights
                        .entry(*chapter)
                        .or_default()
                        .push(hl.clone());
                }
                self.clear_reader_selection(cx);
                self.refresh_current_chapter(cx);
                continue;
            }
            if let Some(note) = action.downcast_ref::<NoteAction>() {
                match note {
                    NoteAction::Refreshed {
                        book_id,
                        notes,
                        message,
                        finish_composer,
                    } => {
                        let is_current = CURRENT_BOOK
                            .read()
                            .unwrap()
                            .as_ref()
                            .map(|book| book.id.as_str() == book_id.as_str())
                            .unwrap_or(false);
                        if !is_current {
                            continue;
                        }
                        let count = notes.len();
                        self.replace_notes(cx, notes.clone());
                        let text = message.clone().unwrap_or_else(|| {
                            if app_language() == AppLanguage::English {
                                format!("{count} notes")
                            } else {
                                format!("共 {count} 条笔记")
                            }
                        });
                        self.ui.label(cx, ids!(note_hint)).set_text(cx, &text);
                        if *finish_composer {
                            self.close_note_composer(cx);
                            self.clear_reader_selection(cx);
                        }
                        self.ui.redraw(cx);
                    }
                    NoteAction::Error(e) => {
                        self.ui.label(cx, ids!(note_hint)).set_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!("Note operation failed: {e}")
                            } else {
                                format!("笔记操作失败：{e}")
                            },
                        );
                    }
                }
                continue;
            }
            if let Some(reader) = action.downcast_ref::<ReaderAction>() {
                match reader {
                    ReaderAction::Opened {
                        resume_chapter,
                        gloss_count,
                        parse_ms,
                        initial_gloss_ms,
                    } => {
                        ::log::info!(
                            "Opened EPUB: parse={}ms, initial chapter gloss={}ms, lazy remaining chapters",
                            parse_ms,
                            initial_gloss_ms
                        );
                        self.set_chapter(cx, *resume_chapter);
                        if *GLOSS_ENABLED.read().unwrap() && *gloss_count == 0 {
                            self.ui.label(cx, ids!(status)).set_text(
                                cx,
                                localized(
                                    "注音未显示：释义库为空（需生成或导入词义数据）",
                                    "Glosses are hidden because the definition library is empty",
                                ),
                            );
                        }
                        if let Some(book_id) = CURRENT_BOOK
                            .read()
                            .unwrap()
                            .as_ref()
                            .map(|book| book.id.clone())
                        {
                            spawn_load_notes(book_id);
                        }
                        self.start_reading_timer(cx);
                    }
                    ReaderAction::OpenError(e) => {
                        self.ui.label(cx, ids!(title)).set_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!("Failed to open: {e}")
                            } else {
                                format!("打开失败: {e}")
                            },
                        );
                    }
                }
                continue;
            }
            if let Some(shelf) = action.downcast_ref::<ShelfAction>() {
                match shelf {
                    ShelfAction::Loaded {
                        items,
                        imported,
                        import_error,
                    } => {
                        if !INITIAL_LIBRARY_READY_LOGGED.swap(true, Ordering::Relaxed) {
                            if let Some(elapsed) = startup_elapsed_ms() {
                                ::log::info!("Startup: library projection ready in {elapsed}ms");
                            }
                        }
                        self.refresh_shelf_projection(cx);
                        let hint = if let Some(error) = import_error {
                            if *imported > 0 {
                                if app_language() == AppLanguage::English {
                                    format!(
                                        "{items} books ({imported} added; some files failed: {error})"
                                    )
                                } else {
                                    format!(
                                        "共 {items} 本（新增 {imported} 本，另有文件失败：{error}）"
                                    )
                                }
                            } else if app_language() == AppLanguage::English {
                                format!("Import failed: {error} · {items} books")
                            } else {
                                format!("导入失败：{error} · 当前共 {items} 本")
                            }
                        } else if *imported > 0 {
                            if app_language() == AppLanguage::English {
                                format!("{items} books ({imported} added)")
                            } else {
                                format!("共 {items} 本（本次新增 {imported} 本）")
                            }
                        } else if cfg!(target_os = "ios") {
                            if app_language() == AppLanguage::English {
                                format!("{items} books · Choose Import EPUB to add files")
                            } else {
                                format!("共 {items} 本 · 点击“导入 EPUB”从文件中添加")
                            }
                        } else if app_language() == AppLanguage::English {
                            format!("{items} books · Drop an EPUB or place it in the import folder")
                        } else {
                            format!("共 {items} 本 · 拖入 EPUB 或放进 import 目录添加")
                        };
                        self.ui.label(cx, ids!(library_hint)).set_text(cx, &hint);
                        self.ui.redraw(cx);
                    }
                    ShelfAction::TagsSaved { items } => {
                        self.editing_book_tags_id = None;
                        self.ui.modal(cx, ids!(library_tag_modal)).close(cx);
                        self.refresh_shelf_projection(cx);
                        self.ui.label(cx, ids!(library_hint)).set_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!("Tags saved · {items} books")
                            } else {
                                format!("标签已保存 · 共 {items} 本")
                            },
                        );
                    }
                    ShelfAction::TagError(e) => {
                        self.ui.label(cx, ids!(library_tag_status)).set_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!("Failed to save tags: {e}")
                            } else {
                                format!("标签保存失败：{e}")
                            },
                        );
                    }
                    ShelfAction::Deleted { items, title } => {
                        self.refresh_shelf_projection(cx);
                        self.ui.label(cx, ids!(library_hint)).set_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!("Deleted “{title}” · {items} books")
                            } else {
                                format!("已删除《{title}》 · 共 {items} 本")
                            },
                        );
                    }
                    ShelfAction::DeleteError(e) => {
                        self.ui.label(cx, ids!(library_hint)).set_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!("Failed to delete book: {e}")
                            } else {
                                format!("删除书籍失败：{e}")
                            },
                        );
                    }
                    ShelfAction::Error(e) => {
                        self.ui.label(cx, ids!(library_hint)).set_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!("Failed to load library: {e}")
                            } else {
                                format!("书架加载失败: {e}")
                            },
                        );
                    }
                }
                continue;
            }
            let Some(action) = action.as_widget_action() else {
                continue;
            };
            if let HtmlLinkAction::Clicked { url, .. } = action.cast() {
                if let Some(word) = url.strip_prefix("w:") {
                    let status = if app_language() == AppLanguage::English {
                        format!("Word: {word}")
                    } else {
                        format!("点词：{word}")
                    };
                    self.ui.label(cx, ids!(status)).set_text(cx, &status);
                }
            }
            if let RubyWordAction::Clicked { word, gloss } = action.cast() {
                let word = word
                    .trim_end_matches(['.', ',', ';', ':', '!', '?'])
                    .to_string();
                let lemma = readwise_core::vocab::lemma::lemmatize(&word);
                self.ui.label(cx, ids!(card_word)).set_text(cx, &word);
                self.ui.label(cx, ids!(card_gloss)).set_text(cx, &gloss);
                self.ui.label(cx, ids!(card_hint)).set_text(cx, "");
                self.ui.widget(cx, ids!(word_card)).set_visible(cx, true);
                self.card = Some((word, lemma, gloss));
                self.ui.redraw(cx);
            }
        }
        if let Some(index) = self
            .ui
            .drop_down(cx, ids!(library_tag_filter))
            .selected(actions)
        {
            self.set_library_tag_filter(cx, index);
        }
        if self
            .ui
            .button(cx, ids!(library_tag_cancel))
            .clicked(actions)
            || self
                .ui
                .modal(cx, ids!(library_tag_modal))
                .dismissed(actions)
        {
            self.editing_book_tags_id = None;
            self.ui.modal(cx, ids!(library_tag_modal)).close(cx);
        }
        if self.ui.button(cx, ids!(library_tag_save)).clicked(actions)
            || self
                .ui
                .text_input(cx, ids!(library_tag_input))
                .returned(actions)
                .is_some()
        {
            self.save_library_tag_editor(cx);
        }
        if self
            .ui
            .button(cx, ids!(library_delete_cancel))
            .clicked(actions)
            || self
                .ui
                .modal(cx, ids!(library_delete_modal))
                .dismissed(actions)
        {
            self.cancel_delete_book_confirmation(cx);
        }
        if self
            .ui
            .button(cx, ids!(library_delete_confirm))
            .clicked(actions)
        {
            self.confirm_delete_book(cx);
        }

        let shelf_list = self
            .ui
            .widget(cx, ids!(book_shelf))
            .portal_list(cx, ids!(list));
        for (item_id, item) in shelf_list.items_with_actions(actions) {
            let book = SHELF_VISIBLE
                .read()
                .unwrap()
                .entry_for_item_id(item_id)
                .cloned();
            let Some(book) = book else { continue };
            if item.button(cx, ids!(del_btn)).clicked(actions) {
                self.open_delete_book_confirmation(cx, &book);
            }
            if item.button(cx, ids!(tag_btn)).clicked(actions) {
                self.open_library_tag_editor(cx, &book);
            }
            if item.button(cx, ids!(read_btn)).clicked(actions) {
                self.replace_notes(cx, Vec::new());
                self.ui.label(cx, ids!(title)).set_text(
                    cx,
                    &if app_language() == AppLanguage::English {
                        format!("Loading “{}”…", book.title)
                    } else {
                        format!("《{}》加载中…", book.title)
                    },
                );
                self.activate_page(cx, AppPage::Reader);
                spawn_open_book(book);
            }
        }
        if self.ui.button(cx, ids!(import_epub_btn)).clicked(actions) {
            #[cfg(target_os = "ios")]
            match crate::mobile_import::open_epub_picker() {
                Ok(()) => self.ui.label(cx, ids!(library_hint)).set_text(
                    cx,
                    localized("请从“文件”中选择 EPUB…", "Select EPUB files in Files…"),
                ),
                Err(error) => self.ui.label(cx, ids!(library_hint)).set_text(
                    cx,
                    &if app_language() == AppLanguage::English {
                        format!("Could not open the file picker: {error}")
                    } else {
                        format!("无法打开文件选择器：{error}")
                    },
                ),
            }
        }
        let skill_list = self
            .ui
            .widget(cx, ids!(skill_list))
            .portal_list(cx, ids!(list));
        for (item_id, item) in skill_list.items_with_actions(actions) {
            let skill = SKILLS.read().unwrap().entry_for_item_id(item_id).cloned();
            let Some(skill) = skill else { continue };
            if let Some(is_active) = item.check_box(cx, ids!(skill_active)).changed(actions) {
                self.ui
                    .label(cx, ids!(skills_status))
                    .set_text(cx, localized("正在更新技能状态…", "Updating skill status…"));
                spawn_toggle_skill(skill.id.clone(), is_active);
            }
            if item.button(cx, ids!(skill_use)).clicked(actions) {
                self.use_skill_next(cx, &skill);
            }
            if item.button(cx, ids!(skill_edit)).clicked(actions) {
                self.edit_skill(cx, &skill);
            }
            if item.button(cx, ids!(skill_delete)).clicked(actions) && !skill.is_system {
                self.ui
                    .label(cx, ids!(skills_status))
                    .set_text(cx, localized("正在删除技能…", "Deleting skill…"));
                spawn_delete_skill(skill.id.clone());
            }
        }
        let vocab_list = self
            .ui
            .widget(cx, ids!(vocab_list))
            .portal_list(cx, ids!(list));
        for (item_id, item) in vocab_list.items_with_actions(actions) {
            let entry = VOCAB_VISIBLE
                .read()
                .unwrap()
                .entry_for_item_id(item_id)
                .cloned();
            let Some(entry) = entry else { continue };
            if item.button(cx, ids!(vocab_speak)).clicked(actions) {
                speak_text(entry.word.clone());
                self.ui.label(cx, ids!(vocab_status_hint)).set_text(
                    cx,
                    &if app_language() == AppLanguage::English {
                        format!("Speaking {}…", entry.word)
                    } else {
                        format!("正在朗读 {}…", entry.word)
                    },
                );
            }
            if item.button(cx, ids!(vocab_review)).clicked(actions) {
                self.start_vocab_review(cx, vec![entry.lemma.clone()]);
            }
            if item.button(cx, ids!(vocab_toggle)).clicked(actions) {
                let status = if entry.status == "mastered" {
                    "learning"
                } else {
                    "mastered"
                };
                self.ui.label(cx, ids!(vocab_status_hint)).set_text(
                    cx,
                    localized("正在更新学习状态…", "Updating learning status…"),
                );
                spawn_set_vocab_status(entry.lemma.clone(), status);
            }
            if item.button(cx, ids!(vocab_remove)).clicked(actions) {
                self.ui
                    .label(cx, ids!(vocab_status_hint))
                    .set_text(cx, localized("正在移除生词…", "Removing word…"));
                spawn_remove_vocab(entry.lemma.clone());
            }
        }
        if self.ui.button(cx, ids!(nav_library)).clicked(actions)
            || self
                .ui
                .button(cx, ids!(mobile_nav_library))
                .clicked(actions)
        {
            if self.on_reader_page {
                self.stop_reading_timer(cx, false);
            }
            self.activate_page(cx, AppPage::Library);
        }
        if self.ui.button(cx, ids!(nav_reader)).clicked(actions)
            || self.ui.button(cx, ids!(mobile_nav_reader)).clicked(actions)
        {
            self.activate_page(cx, AppPage::Reader);
            self.start_reading_timer(cx);
        }
        if self.ui.button(cx, ids!(nav_chat)).clicked(actions)
            || self.ui.button(cx, ids!(mobile_nav_chat)).clicked(actions)
        {
            if self.on_reader_page {
                self.stop_reading_timer(cx, false);
            }
            self.activate_page(cx, AppPage::Chat);
        }
        if self.ui.button(cx, ids!(nav_skills)).clicked(actions)
            || self.ui.button(cx, ids!(mobile_nav_skills)).clicked(actions)
        {
            if self.on_reader_page {
                self.stop_reading_timer(cx, false);
            }
            self.activate_page(cx, AppPage::Skills);
        }
        if self.ui.button(cx, ids!(nav_vocab)).clicked(actions)
            || self.ui.button(cx, ids!(mobile_nav_vocab)).clicked(actions)
        {
            if self.on_reader_page {
                self.stop_reading_timer(cx, false);
            }
            self.activate_page(cx, AppPage::Vocab);
            spawn_load_vocab();
        }
        if self.ui.button(cx, ids!(nav_statistics)).clicked(actions)
            || self
                .ui
                .button(cx, ids!(mobile_nav_statistics))
                .clicked(actions)
        {
            if self.on_reader_page {
                self.stop_reading_timer(cx, true);
            } else {
                spawn_load_statistics();
            }
            self.activate_page(cx, AppPage::Statistics);
            self.ui
                .label(cx, ids!(statistics_status))
                .set_text(cx, localized("正在更新统计数据…", "Updating statistics…"));
        }
        if self.ui.button(cx, ids!(nav_settings)).clicked(actions)
            || self
                .ui
                .button(cx, ids!(mobile_nav_settings))
                .clicked(actions)
        {
            if self.on_reader_page {
                self.stop_reading_timer(cx, false);
            }
            self.activate_page(cx, AppPage::Settings);
        }
        if self
            .ui
            .button(cx, ids!(statistics_refresh))
            .clicked(actions)
        {
            self.ui
                .label(cx, ids!(statistics_status))
                .set_text(cx, localized("正在更新统计数据…", "Updating statistics…"));
            spawn_load_statistics();
        }
        if self.ui.button(cx, ids!(skill_new)).clicked(actions)
            || self.ui.button(cx, ids!(skill_cancel)).clicked(actions)
        {
            self.clear_skill_editor(cx);
        }
        if self.ui.button(cx, ids!(skill_save)).clicked(actions) {
            let name = self.ui.text_input(cx, ids!(skill_name_input)).text();
            let content = self.ui.text_input(cx, ids!(skill_content_input)).text();
            if name.trim().is_empty() {
                self.ui.label(cx, ids!(skills_status)).set_text(
                    cx,
                    localized("技能名称不能为空", "Skill name cannot be empty"),
                );
            } else if content.trim().is_empty() {
                self.ui.label(cx, ids!(skills_status)).set_text(
                    cx,
                    localized("技能内容不能为空", "Skill prompt cannot be empty"),
                );
            } else {
                let is_active = self.ui.check_box(cx, ids!(skill_active_edit)).active(cx);
                self.ui
                    .label(cx, ids!(skills_status))
                    .set_text(cx, localized("正在保存技能…", "Saving skill…"));
                spawn_save_skill(self.editing_skill_id.clone(), name, content, is_active);
            }
        }
        if self.ui.button(cx, ids!(chat_skill_clear)).clicked(actions) {
            self.pending_skill_id = None;
            self.refresh_pending_skill_indicator(cx);
        }
        if let Some(search) = self.ui.text_input(cx, ids!(vocab_search)).changed(actions) {
            self.vocab_search = search;
            self.refresh_vocab_projection(cx);
        }
        if let Some(index) = self
            .ui
            .radio_button_set(
                cx,
                ids_array!(
                    vocab_filter_all,
                    vocab_filter_learning,
                    vocab_filter_mastered
                ),
            )
            .selected(cx, actions)
        {
            self.set_vocab_filter(cx, VocabFilter::from_index(index));
        }
        if self
            .ui
            .button(cx, ids!(vocab_start_review))
            .clicked(actions)
        {
            let queue = self
                .vocab_entries
                .iter()
                .filter(|entry| entry.status == "learning")
                .map(|entry| entry.lemma.clone())
                .collect();
            self.start_vocab_review(cx, queue);
        }
        if self
            .ui
            .button(cx, ids!(vocab_review_reveal))
            .clicked(actions)
        {
            self.reveal_vocab_review(cx);
        }
        if self
            .ui
            .button(cx, ids!(vocab_review_again))
            .clicked(actions)
        {
            self.submit_vocab_review(cx, "learning");
        }
        if self
            .ui
            .button(cx, ids!(vocab_review_known))
            .clicked(actions)
        {
            self.submit_vocab_review(cx, "mastered");
        }
        if self
            .ui
            .button(cx, ids!(vocab_review_speak))
            .clicked(actions)
        {
            if let Some(entry) = self.current_review_entry() {
                speak_text(entry.word);
            }
        }
        if self
            .ui
            .button(cx, ids!(vocab_review_close))
            .clicked(actions)
        {
            self.finish_vocab_review(cx, localized("已结束本轮复习", "Review ended"));
        }
        if let Some(index) = self
            .ui
            .radio_button_set(cx, ids_array!(language_chinese, language_english))
            .selected(cx, actions)
        {
            self.set_app_language(
                cx,
                if index == 0 {
                    AppLanguage::Chinese
                } else {
                    AppLanguage::English
                },
            );
        }
        if let Some(index) = self
            .ui
            .radio_button_set(cx, ids_array!(theme_dark, theme_light))
            .selected(cx, actions)
        {
            self.set_reader_theme(
                cx,
                if index == 0 {
                    ReaderTheme::Dark
                } else {
                    ReaderTheme::Light
                },
            );
        }
        if let Some(index) = self
            .ui
            .radio_button_set(cx, ids_array!(reader_font_standard, reader_font_monospace))
            .selected(cx, actions)
        {
            self.set_reader_font(
                cx,
                if index == 0 {
                    ReaderFont::Standard
                } else {
                    ReaderFont::Monospace
                },
            );
        }
        if let Some(index) = self
            .ui
            .radio_button_set(
                cx,
                ids_array!(
                    reader_spacing_default,
                    reader_spacing_comfortable,
                    reader_spacing_relaxed
                ),
            )
            .selected(cx, actions)
        {
            self.set_reader_spacing(
                cx,
                match index {
                    1 => ReaderSpacing::Comfortable,
                    2 => ReaderSpacing::Relaxed,
                    _ => ReaderSpacing::Default,
                },
            );
        }
        if self.ui.button(cx, ids!(font_inc)).clicked(actions) {
            self.adjust_font(cx, 1.0);
        }
        if self.ui.button(cx, ids!(font_dec)).clicked(actions) {
            self.adjust_font(cx, -1.0);
        }
        if self.ui.button(cx, ids!(gloss_inc)).clicked(actions) {
            self.adjust_gloss_font(cx, 0.5);
        }
        if self.ui.button(cx, ids!(gloss_dec)).clicked(actions) {
            self.adjust_gloss_font(cx, -0.5);
        }
        self.handle_swatch_rows(cx, actions);
        if let Some(on) = self.ui.check_box(cx, ids!(gloss_enable)).changed(actions) {
            self.toggle_gloss(cx, on);
        }
        if self.ui.button(cx, ids!(hl_btn)).clicked(actions) {
            let range = *SEL_RANGE.read().unwrap();
            let book_id = CURRENT_BOOK.read().unwrap().as_ref().map(|b| b.id.clone());
            if let (Some(range), Some(book_id)) = (range, book_id) {
                spawn_add_highlight(book_id, self.chapter_idx, range);
            }
        }
        if self.ui.button(cx, ids!(note_from_sel_btn)).clicked(actions) {
            self.open_note_composer(cx);
        }
        if self.ui.button(cx, ids!(copy_sel_btn)).clicked(actions) {
            let text = SELECTED_TEXT.read().unwrap().trim().to_string();
            if !text.is_empty() {
                cx.copy_to_clipboard(&text);
                self.ui
                    .label(cx, ids!(sel_hint))
                    .set_text(cx, localized("已复制到剪贴板 ✓", "Copied ✓"));
            }
        }
        if self
            .ui
            .button(cx, ids!(vocab_from_sel_btn))
            .clicked(actions)
        {
            let selected = SELECTED_TEXT.read().unwrap().clone();
            if let Some(word) = selected_single_word(&selected) {
                let lemma = readwise_core::vocab::lemma::lemmatize(&word);
                let book_id = CURRENT_ITEM.read().unwrap().as_ref().map(|i| i.id.clone());
                self.ui
                    .label(cx, ids!(sel_hint))
                    .set_text(cx, localized("加入中…", "Adding…"));
                spawn_add_vocab(lemma, word, String::new(), book_id);
            } else {
                self.ui.label(cx, ids!(sel_hint)).set_text(
                    cx,
                    localized(
                        "加入生词仅支持选择一个英文单词",
                        "Select one English word to add it to Vocabulary",
                    ),
                );
            }
        }
        if self.ui.button(cx, ids!(translate_sel_btn)).clicked(actions) {
            let text = SELECTED_TEXT.read().unwrap().trim().to_string();
            if !text.is_empty() {
                let url = format!(
                    "https://translate.google.com/?sl=en&tl=zh-CN&text={}&op=translate",
                    percent_encode_query(&text)
                );
                cx.open_url(&url, OpenUrlInPlace::No);
                self.ui.label(cx, ids!(sel_hint)).set_text(
                    cx,
                    localized("已在浏览器中打开翻译", "Translation opened in the browser"),
                );
            }
        }
        if self.ui.button(cx, ids!(ask_ai_sel_btn)).clicked(actions) {
            let selected = SELECTED_TEXT.read().unwrap().trim().to_string();
            if !selected.is_empty() {
                let prompt = if app_language() == AppLanguage::English {
                    format!(
                        "Explain the following English passage and identify important words and expressions:\n\n{selected}"
                    )
                } else {
                    format!("请解释下面这段英文，并指出重要词语和表达：\n\n{selected}")
                };
                self.clear_reader_selection(cx);
                self.note_open = false;
                self.ui.widget(cx, ids!(note_panel)).set_visible(cx, false);
                self.set_reader_ai_visible(cx, true);
                if !self.send_chat_text(cx, prompt.clone()) {
                    self.ui
                        .text_input(cx, ids!(reader_chat_input))
                        .set_text(cx, &prompt);
                }
            }
        }
        if self.ui.button(cx, ids!(speak_sel_btn)).clicked(actions) {
            let text = SELECTED_TEXT.read().unwrap().trim().to_string();
            if !text.is_empty() {
                speak_text(text);
                self.ui
                    .label(cx, ids!(sel_hint))
                    .set_text(cx, localized("正在发音…", "Speaking…"));
            }
        }
        if self.ui.button(cx, ids!(sel_close_btn)).clicked(actions) {
            self.clear_reader_selection(cx);
            self.ui.redraw(cx);
        }
        if self.ui.button(cx, ids!(note_save_btn)).clicked(actions) {
            let extra = self.ui.text_input(cx, ids!(note_extra)).text();
            self.save_note_from_composer(cx, extra);
        }
        if let Some((extra, _)) = self.ui.text_input(cx, ids!(note_extra)).returned(actions) {
            self.save_note_from_composer(cx, extra);
        }
        if self.ui.button(cx, ids!(note_cancel_btn)).clicked(actions) {
            self.close_note_composer(cx);
        }
        if self.ui.button(cx, ids!(note_close_btn)).clicked(actions) {
            self.note_open = false;
            self.close_note_composer(cx);
            self.sync_reader_panel_layout(cx);
        }
        if self.ui.button(cx, ids!(close_card_btn)).clicked(actions) {
            self.card = None;
            self.ui.widget(cx, ids!(word_card)).set_visible(cx, false);
            self.ui.redraw(cx);
        }
        if self.ui.button(cx, ids!(add_vocab_btn)).clicked(actions) {
            if let Some((word, lemma, gloss)) = self.card.clone() {
                let book_id = CURRENT_ITEM.read().unwrap().as_ref().map(|i| i.id.clone());
                self.ui
                    .label(cx, ids!(card_hint))
                    .set_text(cx, localized("加入中…", "Adding…"));
                spawn_add_vocab(lemma, word, gloss, book_id);
            }
        }
        if self.ui.button(cx, ids!(know_btn)).clicked(actions) {
            if let Some((_, lemma, _)) = self.card.clone() {
                self.ui
                    .label(cx, ids!(card_hint))
                    .set_text(cx, localized("标记中…", "Marking…"));
                spawn_mark_known(lemma);
            }
        }
        if self.ui.button(cx, ids!(toc_btn)).clicked(actions) {
            self.toc_open = !self.toc_open;
            if self.toc_open {
                self.note_open = false;
                self.reader_ai_open = false;
            }
            self.sync_reader_panel_layout(cx);
            self.ui.redraw(cx);
        }
        if self.ui.button(cx, ids!(notes_btn)).clicked(actions) {
            self.note_open = !self.note_open;
            if self.note_open {
                self.toc_open = false;
                self.reader_ai_open = false;
            }
            self.sync_reader_panel_layout(cx);
            self.ui.redraw(cx);
        }
        if self.ui.button(cx, ids!(reader_ai_btn)).clicked(actions) {
            let visible = !self.reader_ai_open;
            self.set_reader_ai_visible(cx, visible);
        }
        if self.ui.button(cx, ids!(reader_ai_close)).clicked(actions) {
            self.set_reader_ai_visible(cx, false);
        }
        let toc_list = self
            .ui
            .widget(cx, ids!(toc_list))
            .portal_list(cx, ids!(list));
        for (item_id, item) in toc_list.items_with_actions(actions) {
            if item.button(cx, ids!(toc_item_btn)).clicked(actions) {
                if CURRENT_BOOK.read().unwrap().is_some() {
                    self.set_chapter(cx, item_id);
                    self.toc_open = false;
                    self.sync_reader_panel_layout(cx);
                    self.ui.redraw(cx);
                }
            }
        }
        let note_list = self
            .ui
            .widget(cx, ids!(note_list))
            .portal_list(cx, ids!(list));
        for (item_id, item) in note_list.items_with_actions(actions) {
            if item.button(cx, ids!(note_delete_btn)).clicked(actions) {
                let note_id = NOTES
                    .read()
                    .unwrap()
                    .entry_for_item_id(item_id)
                    .map(|note| note.id.clone());
                let book_id = CURRENT_BOOK.read().unwrap().as_ref().map(|b| b.id.clone());
                if let (Some(book_id), Some(note_id)) = (book_id, note_id) {
                    self.ui
                        .label(cx, ids!(note_hint))
                        .set_text(cx, localized("删除中…", "Deleting…"));
                    spawn_delete_note(book_id, note_id);
                }
            }
        }
        if self.ui.button(cx, ids!(chat_send)).clicked(actions)
            || self
                .ui
                .text_input(cx, ids!(chat_input))
                .returned(actions)
                .is_some()
        {
            self.send_chat(cx);
        }
        if self.ui.button(cx, ids!(reader_chat_send)).clicked(actions)
            || self
                .ui
                .text_input(cx, ids!(reader_chat_input))
                .returned(actions)
                .is_some()
        {
            self.send_reader_chat(cx);
        }
        if self.ui.button(cx, ids!(chat_cancel)).clicked(actions)
            || self
                .ui
                .button(cx, ids!(reader_chat_cancel))
                .clicked(actions)
            || self.ui.text_input(cx, ids!(chat_input)).escaped(actions)
            || self
                .ui
                .text_input(cx, ids!(reader_chat_input))
                .escaped(actions)
        {
            self.cancel_chat(cx);
        }
        if self.ui.button(cx, ids!(chat_clear)).clicked(actions)
            || self.ui.button(cx, ids!(reader_chat_clear)).clicked(actions)
        {
            self.clear_chat(cx);
        }
        if self.ui.button(cx, ids!(local_ai_save)).clicked(actions) {
            match self.local_ai_settings_from_ui(cx) {
                Ok(settings) => {
                    self.ui.label(cx, ids!(local_ai_status)).set_text(
                        cx,
                        localized("正在保存本地 AI 设置…", "Saving local AI settings…"),
                    );
                    spawn_save_local_ai_settings(settings);
                }
                Err(error) => self
                    .ui
                    .label(cx, ids!(local_ai_status))
                    .set_text(cx, &error),
            }
        }
        if self.ui.button(cx, ids!(local_ai_start)).clicked(actions) {
            match self.local_ai_settings_from_ui(cx) {
                Ok(settings) => match settings.validate() {
                    Ok(()) => {
                        self.ui.label(cx, ids!(local_ai_status)).set_text(
                            cx,
                            localized(
                                "正在加载 GGUF 模型，首次启动可能需要几分钟…",
                                "Loading the GGUF model; the first start may take a few minutes…",
                            ),
                        );
                        self.set_local_ai_running(cx, true);
                        spawn_start_local_ai(settings);
                    }
                    Err(error) => self
                        .ui
                        .label(cx, ids!(local_ai_status))
                        .set_text(cx, &error),
                },
                Err(error) => self
                    .ui
                    .label(cx, ids!(local_ai_status))
                    .set_text(cx, &error),
            }
        }
        if self.ui.button(cx, ids!(local_ai_stop)).clicked(actions) {
            self.ui.label(cx, ids!(local_ai_status)).set_text(
                cx,
                localized("正在停止本地服务…", "Stopping local service…"),
            );
            spawn_stop_local_ai();
        }
        if self.ui.button(cx, ids!(ai_save)).clicked(actions) {
            let endpoint_input = self.ui.text_input(cx, ids!(ai_endpoint)).text();
            let endpoint = readwise_core::ai::normalize_chat_endpoint(&endpoint_input);
            let settings = endpoint.map(|endpoint| readwise_core::ai::AiSettings {
                endpoint,
                model: self
                    .ui
                    .text_input(cx, ids!(ai_model))
                    .text()
                    .trim()
                    .to_string(),
                api_key: self.ui.text_input(cx, ids!(ai_api_key)).text(),
            });
            if let Err(error) = &settings {
                self.ui
                    .label(cx, ids!(ai_settings_status))
                    .set_text(cx, error);
            } else if let Ok(settings) = settings {
                if settings.model.is_empty() {
                    self.ui.label(cx, ids!(ai_settings_status)).set_text(
                        cx,
                        localized("模型名称不能为空", "Model name cannot be empty"),
                    );
                    return;
                }
                self.ui
                    .label(cx, ids!(ai_settings_status))
                    .set_text(cx, localized("正在保存…", "Saving…"));
                spawn_save_ai_settings(settings);
            }
        }
        if self.ui.button(cx, ids!(reload_button)).clicked(actions) {
            self.reload_count += 1;
            *PARAGRAPHS.write().unwrap() = sample_chapter::build_paragraphs(PARA_COUNT);
            let status = if app_language() == AppLanguage::English {
                format!("Rendered {} times", self.reload_count)
            } else {
                format!("已重新渲染 {} 次", self.reload_count)
            };
            self.ui.label(cx, ids!(status)).set_text(cx, &status);
            self.ui.redraw(cx);
        }
    }
}

impl AppMain for App {
    fn script_mod(vm: &mut ScriptVm) -> ScriptValue {
        makepad_widgets::theme_mod(vm);
        script_eval!(vm, {
            mod.themes.dark = mod.themes.dark{
                color_chart_grid: #xFFFFFF18
                color_chart_reading: #x62B6FF
                color_chart_vocab: #x7FB2A6
            }
            mod.themes.light = mod.themes.light{
                color_chart_grid: #x00000018
                color_chart_reading: #x236EAD
                color_chart_vocab: #x397F70
            }
        });
        if *READER_THEME.read().unwrap() == ReaderTheme::Light {
            script_eval!(vm, {
                mod.theme = mod.themes.light
            });
        } else {
            script_eval!(vm, {
                mod.theme = mod.themes.dark
            });
        }
        script_eval!(vm, {
            mod.readwise_theme_dark = mod.themes.dark{}
            mod.readwise_theme_light = mod.themes.light{}
        });
        makepad_widgets::widgets_mod(vm);
        crate::ruby_word::script_mod(vm);
        crate::chapter_image::script_mod(vm);
        crate::mark_span::script_mod(vm);
        self::script_mod(vm)
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        let reader_swipe = self.track_reader_swipe(cx, event);
        if let Event::MouseDown(e) = event {
            let popup_captures_pointer = SEL_RANGE.read().unwrap().is_some()
                && self
                    .ui
                    .widget(cx, ids!(selection_popup))
                    .area()
                    .clipped_rect(cx)
                    .contains(e.abs);
            SELECTION_POPUP_POINTER.store(popup_captures_pointer, Ordering::Relaxed);
        }
        let wire_outcome = self
            .ai_request
            .and_then(|request| inspect_ai_network_event(event, request));
        let stream_updates = if let Some(request) = self.ai_request {
            self.decode_ai_stream_event(event, request)
        } else {
            Vec::new()
        };
        let ai_events = self
            .ai_backend
            .as_mut()
            .map(|backend| backend.handle_event(cx, event))
            .unwrap_or_default();
        if let Some(outcome) = wire_outcome {
            match outcome {
                AiWireOutcome::NonStreamComplete(text) => {
                    if let Some(request_id) = self.ai_request {
                        if let Some(backend) = &mut self.ai_backend {
                            backend.cancel_request(cx, request_id);
                        }
                    }
                    self.finish_chat(cx, text);
                }
                AiWireOutcome::Failure(error) => self.fail_chat(cx, error),
            }
        } else {
            let mut stream_failure = None;
            let mut stream_text = String::new();
            let mut reasoning_started = false;
            for update in stream_updates {
                match update {
                    AiSseUpdate::ReasoningStarted => {
                        reasoning_started = true;
                    }
                    AiSseUpdate::Text(text) => {
                        stream_text.push_str(&text);
                    }
                    AiSseUpdate::Done => {}
                    AiSseUpdate::Failure(error) => {
                        stream_failure.get_or_insert(error);
                    }
                }
            }
            if !stream_text.is_empty() {
                CHAT_DATA
                    .write()
                    .unwrap()
                    .streaming_text
                    .push_str(&stream_text);
                self.set_chat_status(cx, tr("outputting_answer"));
                self.redraw_chat(cx);
            } else if reasoning_started {
                self.set_chat_status(cx, tr("model_thinking"));
            }
            if let Some(error) = stream_failure {
                self.fail_chat(cx, error);
            } else {
                for ai_event in ai_events {
                    match ai_event {
                        AiEvent::StreamDelta {
                            request_id,
                            delta: StreamDelta::TextDelta { text },
                        } if self.ai_request == Some(request_id) && !self.ai_stream.saw_content => {
                            CHAT_DATA.write().unwrap().streaming_text.push_str(&text);
                            self.redraw_chat(cx);
                        }
                        AiEvent::Complete {
                            request_id,
                            response,
                        } if self.ai_request == Some(request_id) => {
                            let fallback = response.message.text();
                            let has_text = !fallback.is_empty()
                                || !CHAT_DATA.read().unwrap().streaming_text.is_empty();
                            if has_text {
                                self.finish_chat(cx, fallback);
                            } else {
                                let error = if self.ai_stream.saw_reasoning {
                                    localized(
                                        "模型完成了思考，但没有返回可显示的正文（可能达到输出上限）",
                                        "The model finished reasoning but returned no visible answer (it may have reached the output limit)",
                                    )
                                } else {
                                    localized(
                                        "服务端完成了请求，但没有返回文本",
                                        "The server completed the request but returned no text",
                                    )
                                };
                                self.fail_chat(cx, error.to_string());
                            }
                        }
                        AiEvent::Error { request_id, error }
                            if self.ai_request == Some(request_id) =>
                        {
                            self.fail_chat(cx, error);
                        }
                        _ => {}
                    }
                }
            }
        }
        match event {
            Event::ScriptReapply => self.restore_after_script_reload(cx),
            Event::Startup => {
                let _ = APP_STARTED_AT.set(Instant::now());
                self.update_responsive_layout(
                    cx,
                    cx.display_context.screen_size,
                    cx.display_context.safe_area_insets,
                );
                self.ui
                    .widget(cx, ids!(import_epub_btn))
                    .set_visible(cx, cfg!(target_os = "ios"));
                if cfg!(target_os = "ios") {
                    self.ui.label(cx, ids!(library_hint)).set_text(
                        cx,
                        localized(
                            "点击“导入 EPUB”，从“文件”App 中选择书籍。",
                            "Choose Import EPUB to select books from Files.",
                        ),
                    );
                    self.ui.label(cx, ids!(library_empty_hint)).set_text(
                        cx,
                        localized(
                            "点击右上角“导入 EPUB”添加书籍",
                            "Choose Import EPUB in the top-right corner to add books",
                        ),
                    );
                }
                spawn_db_init(cx.get_data_dir().map(std::path::PathBuf::from));
            }
            Event::WindowGeomChange(event) => self.update_responsive_layout(
                cx,
                event.new_geom.inner_size,
                event.new_geom.safe_area_insets,
            ),
            Event::VirtualKeyboard(event) => self.handle_virtual_keyboard(cx, event),
            Event::KeyDown(ke) => self.handle_reader_key(cx, ke),
            // 滚轮/触控板滚动后刷新章内百分比
            Event::Scroll(_) => {
                if self.on_reader_page && CURRENT_BOOK.read().unwrap().is_some() {
                    self.update_progress_label(cx);
                }
            }
            // 拖入悬停：声明接受拷贝，否则系统不触发 Drop。
            Event::Drag(de) => {
                if de.items.iter().any(is_epub_item) {
                    *de.response.lock().unwrap() = DragResponse::Copy;
                }
            }
            // 松手放下：收集 epub 路径后台导入。
            Event::Drop(de) => {
                let paths: Vec<String> = de
                    .items
                    .iter()
                    .filter_map(|it| match it {
                        DragItem::FilePath { path, .. } if is_epub_path(path) => {
                            Some(percent_decode_path(path))
                        }
                        _ => None,
                    })
                    .collect();
                if !paths.is_empty() {
                    spawn_import(paths, false);
                }
            }
            _ => {}
        }
        self.match_event(cx, event);
        self.ui.handle_event(cx, event, &mut Scope::empty());
        if let Some(direction) = reader_swipe {
            let selection_is_clear =
                SEL_RANGE.read().unwrap().is_none() && SELECTED_TEXT.read().unwrap().is_empty();
            if self.compact_layout
                && !self.virtual_keyboard_open
                && self.on_reader_page
                && !self.toc_open
                && !self.note_open
                && !self.reader_ai_open
                && selection_is_clear
            {
                self.page_scroll(cx, direction == ReaderSwipeDirection::Next);
            }
        }
        if matches!(event, Event::MouseUp(_)) {
            SELECTION_POPUP_POINTER.store(false, Ordering::Relaxed);
        }
    }
}
