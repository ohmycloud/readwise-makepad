use crate::ruby_word::RubyWordAction;
use crate::sample_chapter;
use makepad_ai::{
    AiBackend, AiEvent, AiRequest, BackendConfig, Message, OpenAiBackend, RequestId, StreamDelta,
};
use makepad_widgets::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, OnceLock, RwLock};

type AiSettings = readwise_core::ai::AiSettings;

app_main!(App);

const PARA_COUNT: usize = 200;

static PARAGRAPHS: LazyLock<RwLock<Vec<String>>> =
    LazyLock::new(|| RwLock::new(sample_chapter::build_paragraphs(PARA_COUNT)));

/// 全局数据库句柄：后台线程初始化后写入，各处只读使用。
static DB: OnceLock<readwise_core::Db> = OnceLock::new();

/// 书架数据（UI 侧只读渲染，后台线程写）。
static SHELF: LazyLock<RwLock<Vec<ShelfItem>>> = LazyLock::new(|| RwLock::new(Vec::new()));

/// 当前书的笔记（后台线程刷新，NoteList 只读渲染）。
static NOTES: LazyLock<RwLock<Vec<readwise_core::notes::models::Note>>> =
    LazyLock::new(|| RwLock::new(Vec::new()));

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
static SKILLS: LazyLock<RwLock<Vec<readwise_core::skills::models::Skill>>> =
    LazyLock::new(|| RwLock::new(Vec::new()));

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

#[derive(Clone)]
struct ShelfItem {
    id: String,
    title: String,
    author: String,
    status_line: String,
    cover_path: Option<String>,
    file_path: String,
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
        let input = readwise_core::vocab::models::VocabBookAddInput {
            lemma,
            word,
            gloss: Some(gloss),
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
        // 原地消注：内存里剥掉该词的 <ruby>，不重载不跳转
        if let Some(book) = CURRENT_BOOK.write().unwrap().as_mut() {
            for ch in book.chapters.iter_mut() {
                if ch.contains("<ruby") {
                    *ch = readwise_core::vocab::annotate::remove_ruby_for_lemma(ch, &lemma);
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

const READER_FONT_DEFAULT: f32 = 11.0;
const READER_FONT_MIN: f32 = 8.0;
const READER_FONT_MAX: f32 = 20.0;

struct OpenBook {
    id: String,
    title: String,
    /// 各章 body HTML（已消毒）。
    chapters: Vec<String>,
    /// 各章标题（TOC 用）。
    chapter_titles: Vec<String>,
    /// 各章高亮（章下标 → 区间列表）。
    highlights: HashMap<usize, Vec<readwise_core::books::Highlight>>,
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
                message: Some("笔记已保存 ✓".to_string()),
                finish_composer: true,
            }),
            Err(e) => Cx::post_action(NoteAction::Error(e)),
        }
    });
}

fn spawn_delete_note(book_id: String, note_id: String) {
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
                message: Some("笔记已删除".to_string()),
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
    NOTES.write().unwrap().clear();
    std::thread::spawn(move || {
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
        // 注音：总开关开启且有释义数据才注入（老项目 GlossAnnotator 的等价物）
        let mut gloss_count = 0usize;
        if *GLOSS_ENABLED.read().unwrap() {
            if let Some(db) = DB.get() {
                if let Ok((glosses, known)) = db.gloss_context(Some(&item.id)) {
                    gloss_count = glosses.len();
                    if !glosses.is_empty() {
                        use readwise_core::vocab::annotate::{annotate_html, flatten_lists};
                        for ch in chapters.iter_mut() {
                            let flat = flatten_lists(ch);
                            *ch = annotate_html(&flat, &glosses, &known);
                        }
                    }
                }
            }
        }
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
        *CURRENT_BOOK.write().unwrap() = Some(OpenBook {
            id: item.id,
            title: item.title,
            chapters,
            chapter_titles,
            highlights,
        });
        Cx::post_action(ReaderAction::Opened {
            resume_chapter: resume,
            gloss_count,
        });
    });
}

/// 章节 → PortalList 块（含高亮标记注入）。
fn build_chapter_chunks(book: &OpenBook, idx: usize) -> Option<Vec<String>> {
    let chapter = book.chapters.get(idx)?;
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

fn status_cn(status: &str, cur: i64, total: i64) -> String {
    let name = match status {
        "reading" => "在读",
        "completed" => "读完",
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
    Loaded { items: usize, imported: usize },
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
            status_line: status_cn(&b.status, b.progress_current, b.progress_total),
            cover_path: b.cover_abs_path.clone(),
            file_path: b.file_abs_path.clone(),
        })
        .collect();
    let n = ui_items.len();
    *SHELF.write().unwrap() = ui_items;
    Ok(n)
}

/// 后台加载书架：先扫描导入目录，再读全部书目。
fn spawn_load_shelf() {
    std::thread::spawn(|| {
        let Some(db) = DB.get() else { return };
        let imported = db.scan_import_dir().unwrap_or(0);
        match refresh_shelf(db) {
            Ok(n) => Cx::post_action(ShelfAction::Loaded { items: n, imported }),
            Err(e) => Cx::post_action(ShelfAction::Error(e)),
        }
    });
}

/// 后台删除书籍后刷新书架。
fn spawn_delete(id: String) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        if let Err(e) = db.delete_book(&id) {
            println!("删除失败 {}: {}", id, e);
        }
        match refresh_shelf(db) {
            Ok(n) => Cx::post_action(ShelfAction::Loaded {
                items: n,
                imported: 0,
            }),
            Err(e) => Cx::post_action(ShelfAction::Error(e)),
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

/// 拖入的 epub 文件在后台导入，完成后刷新书架。
fn spawn_import(paths: Vec<String>) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        for p in paths {
            if let Err(e) = db.import_book(std::path::Path::new(&p)) {
                //log::error!("导入失败 {}: {}", p, e);
                println!("导入失败 {}: {}", p, e);
            }
        }
        match refresh_shelf(db) {
            Ok(n) => Cx::post_action(ShelfAction::Loaded {
                items: n,
                imported: 0,
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
        vocab_preview: String,
    },
    Failed(String),
}

fn spawn_db_init() {
    std::thread::spawn(|| match readwise_core::Db::open() {
        Ok(db) => {
            let entries = db.vocab_book_list().unwrap_or_default();
            let known = db.get_known_words().unwrap_or_default();
            let vocab_preview = entries
                .iter()
                .take(50)
                .map(|e| {
                    format!(
                        "{}  —  {}   [{} · 复习{}次]",
                        e.word,
                        e.gloss.as_deref().unwrap_or("(无释义)"),
                        e.status,
                        e.review_count
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let action = DbAction::Ready {
                vocab_count: entries.len(),
                known_count: known.len(),
                vocab_preview,
            };
            let _ = DB.set(db);
            Cx::post_action(action);
        }
        Err(e) => Cx::post_action(DbAction::Failed(e)),
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

    mod.widgets.BookShelfBase = #(BookShelf::register_widget(vm))
    mod.widgets.BookShelf = set_type_default() do mod.widgets.BookShelfBase{
        width: Fill height: Fill
        list := PortalList{
            width: Fill height: Fill
            padding: Inset{ left: 24 right: 24 top: 8 bottom: 24 }
            spacing: 10
            scroll_bar: ScrollBar{}
            Card := RoundedView{
                width: Fill height: 76
                flow: Right spacing: 14 padding: 12
                align: Align{ y: 0.5 }
                draw_bg.color: #x2a2a2a
                draw_bg.border_radius: 8.0
                cover := View{
                    width: 44 height: 52
                    flow: Overlay
                    cover_ph := SolidView{
                        width: Fill height: Fill
                        align: Align{ x: 0.5 y: 0.5 }
                        draw_bg.color: #x1f6f63
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
                    flow: Down spacing: 5
                    card_title := Label{
                        width: Fill
                        text: ""
                        draw_text.text_style.font_size: 14
                    }
                    card_author := Label{
                        width: Fill
                        text: ""
                        draw_text.color: #x9a9a9a
                    }
                    card_status := Label{
                        width: Fill
                        text: ""
                        draw_text.color: #x7fb2a6
                        draw_text.text_style.font_size: 9
                    }
                }
                read_btn := ButtonFlat{ text: "阅读" }
                del_btn := ButtonFlat{ text: "删除" }
            }
        }
    }

    mod.widgets.TocListBase = #(TocList::register_widget(vm))
    mod.widgets.TocList = set_type_default() do mod.widgets.TocListBase{
        width: Fill height: Fill
        list := PortalList{
            width: Fill height: Fill
            padding: Inset{ left: 8 right: 8 top: 8 bottom: 8 }
            spacing: 2
            scroll_bar: ScrollBar{}
            TocItem := SolidView{
                width: Fill height: Fit
                draw_bg.color: #x232323
                toc_item_btn := ButtonFlat{
                    width: Fill
                    text: ""
                }
            }
        }
    }

    mod.widgets.NoteListBase = #(NoteList::register_widget(vm))
    mod.widgets.NoteList = set_type_default() do mod.widgets.NoteListBase{
        width: Fill height: Fill
        list := PortalList{
            width: Fill height: Fill
            spacing: 8
            scroll_bar: ScrollBar{}
            NoteCard := RoundedView{
                width: Fill height: Fit
                new_batch: true
                padding: 10
                flow: Down spacing: 6
                draw_bg.color: #x292929
                draw_bg.border_radius: 7.0
                note_title := Label{
                    width: Fill
                    text: ""
                    draw_text.text_style.font_size: 11
                }
                note_preview := Label{
                    width: Fill
                    text: ""
                    draw_text.color: #xaaaaaa
                    draw_text.text_style.font_size: 9
                }
                note_actions := View{
                    width: Fill height: Fit
                    flow: Right
                    align: Align{x: 1.0 y: 0.5}
                    note_delete_btn := ButtonFlatter{ text: "删除" }
                }
            }
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
                draw_bg.color: #x24433e
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
                draw_bg.color: #x292929
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
            SkillCard := RoundedView{
                width: Fill height: Fit
                flow: Down spacing: 7 padding: 10
                new_batch: true
                draw_bg.color: #x292929
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
                        draw_text.color: #x7fb2a6
                        draw_text.text_style.font_size: 8.5
                    }
                }
                skill_preview := Label{
                    width: Fill height: Fit
                    text: ""
                    draw_text.color: #xaaaaaa
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
        }
    }

    mod.widgets.ChapterListBase = #(ChapterList::register_widget(vm))
    mod.widgets.ChapterList = set_type_default() do mod.widgets.ChapterListBase{
        width: Fill height: Fill
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
                    a.color: #xc9c9c9
                    a.hover_color: #x62b6ff
                    a.pressed_color: #x2a8fe0
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
                    a.color: #xc9c9c9
                    a.hover_color: #x62b6ff
                    a.pressed_color: #x2a8fe0
                    ruby := mod.widgets.RubyWord{}
                    img := mod.widgets.ChapterImage{}
                    mark := mod.widgets.MarkSpan{}
                }
            }
        }
    }

    startup() do #(App::script_component(vm)){
        ui: Root{
            main_window := Window{
                window.title: "ReadWise"
                window.inner_size: vec2(1080, 720)
                body +: {
                    flow: Right
                    sidebar := SolidView{
                        width: 150 height: Fill
                        flow: Down padding: 12 spacing: 8
                        draw_bg.color: #x1f1f1f
                        app_title := Label{
                            text: "ReadWise"
                            draw_text.text_style.font_size: 15
                        }
                        nav_library := ButtonFlat{ width: Fill text: "书库" }
                        nav_reader := ButtonFlat{ width: Fill text: "阅读" }
                        nav_chat := ButtonFlat{ width: Fill text: "AI 对话" }
                        nav_skills := ButtonFlat{ width: Fill text: "AI 技能" }
                        nav_vocab := ButtonFlat{ width: Fill text: "生词本" }
                        nav_settings := ButtonFlat{ width: Fill text: "设置" }
                        sidebar_fill := View{ width: Fill height: Fill }
                        db_status := Label{
                            width: Fill
                            text: "数据库初始化中…"
                            draw_text.text_style.font_size: 8.5
                        }
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
                                Label{ text: "书库" draw_text.text_style.font_size: 18 }
                                library_hint := Label{
                                    width: Fill
                                    text: "把 EPUB 拖进窗口，或放入导入目录后重启。"
                                    draw_text.color: #x9a9a9a
                                    draw_text.text_style.font_size: 9
                                }
                            }
                            book_shelf := mod.widgets.BookShelf{}
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
                                    draw_text.color: #x8a8a8a
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
                                    draw_bg.color: #x232323
                                    toc_list := mod.widgets.TocList{}
                                    }
                                    chapter_list := mod.widgets.ChapterList{}
                                    note_panel := SolidView{
                                    visible: false
                                    width: 320 height: Fill
                                    new_batch: true
                                    flow: Down spacing: 10 padding: 12
                                    draw_bg.color: #x232323
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
                                        draw_bg.color: #x2d2d2d
                                        draw_bg.border_radius: 8.0
                                        Label{
                                            text: "引用"
                                            draw_text.color: #x7fb2a6
                                            draw_text.text_style.font_size: 9
                                        }
                                        note_quote := Label{
                                            width: Fill
                                            text: ""
                                            draw_text.color: #xc9c9c9
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
                                        draw_text.color: #x8a8a8a
                                        draw_text.text_style.font_size: 9
                                    }
                                    note_list := mod.widgets.NoteList{}
                                    }
                                    reader_ai_panel := SolidView{
                                    visible: false
                                    width: 360 height: Fill
                                    new_batch: true
                                    flow: Down spacing: 10 padding: 12
                                    draw_bg.color: #x232323
                                    reader_ai_header := View{
                                        width: Fill height: Fit
                                        flow: Right spacing: 8
                                        align: Align{y: 0.5}
                                        Label{
                                            width: Fill
                                            text: "阅读 AI"
                                            draw_text.text_style.font_size: 14
                                        }
                                        reader_ai_close := ButtonFlatter{text: "关闭"}
                                    }
                                    reader_chat_status := Label{
                                        width: Fill height: Fit
                                        text: "选择文字后点“问 AI”，或在下方直接提问"
                                        draw_text.color: #x8f8f8f
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
                                        width: Fit height: Fit
                                        new_batch: true
                                        padding: Inset{left: 12 right: 12 top: 9 bottom: 9}
                                        flow: Down spacing: 5
                                        draw_bg.color: #x303030
                                        draw_bg.border_radius: 10.0
                                        selection_actions := View{
                                            width: Fit height: Fit
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
                                            draw_text.color: #x9a9a9a
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
                                draw_bg.color: #x2e2e2e
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
                                        draw_text.color: #x7fb2a6
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
                                        draw_text.color: #x9a9a9a
                                        draw_text.text_style.font_size: 9
                                    }
                                }
                            }
                        }

                        page_vocab := View{
                            width: Fill height: Fill
                            flow: Down padding: 24 spacing: 12
                            vocab_title := Label{ text: "生词本" draw_text.text_style.font_size: 18 }
                            vocab_summary := Label{ width: Fill text: "等待数据库…" }
                            vocab_scroll := ScrollYView{
                                width: Fill height: Fill
                                vocab_list := Label{ width: Fill text: "" }
                            }
                        }

                        page_chat := View{
                            width: Fill height: Fill
                            flow: Down
                            chat_header := View{
                                width: Fill height: Fit
                                flow: Down spacing: 4
                                padding: Inset{left: 24 right: 24 top: 20 bottom: 8}
                                Label{ text: "AI 对话" draw_text.text_style.font_size: 18 }
                                chat_status := Label{
                                    width: Fill
                                    text: "正在读取 AI 设置…"
                                    draw_text.color: #x8f8f8f
                                    draw_text.text_style.font_size: 9
                                }
                                chat_skill_row := View{
                                    width: Fill height: Fit
                                    flow: Right spacing: 8
                                    align: Align{y: 0.5}
                                    chat_skill_status := Label{
                                        width: Fill height: Fit
                                        text: "未选择技能模板"
                                        draw_text.color: #x7fb2a6
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
                                    Label{
                                        text: "AI 技能"
                                        draw_text.text_style.font_size: 18
                                    }
                                    skills_status := Label{
                                        width: Fill height: Fit
                                        text: "正在读取技能…"
                                        draw_text.color: #x8f8f8f
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
                                    draw_bg.color: #x232323
                                    draw_bg.border_radius: 9.0
                                    skill_list := mod.widgets.SkillList{}
                                }
                                skill_editor := RoundedView{
                                    width: Fill height: Fill
                                    flow: Down spacing: 9 padding: 14
                                    new_batch: true
                                    draw_bg.color: #x252525
                                    draw_bg.border_radius: 9.0
                                    skill_editor_title := Label{
                                        width: Fill height: Fit
                                        text: "新建技能"
                                        draw_text.text_style.font_size: 14
                                    }
                                    Label{
                                        text: "名称"
                                        draw_text.color: #xaaaaaa
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
                                    Label{
                                        text: "Prompt 模板（Markdown）"
                                        draw_text.color: #xaaaaaa
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
                                        draw_text.color: #x8f8f8f
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
                            Label{ text: "设置" draw_text.text_style.font_size: 18 }
                            Label{
                                text: "阅读"
                                draw_text.text_style.font_size: 13
                                draw_text.color: #x9a9a9a
                            }
                            font_row := View{
                                width: Fill height: Fit
                                spacing: 12
                                align: Align{ y: 0.5 }
                                Label{ text: "正文字号" }
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
                                Label{ text: "注音字号" }
                                gloss_dec := ButtonFlat{ text: "A-" }
                                gloss_size_label := Label{ text: "7.5" }
                                gloss_inc := ButtonFlat{ text: "A+" }
                            }
                            gloss_color_row := View{
                                width: Fill height: Fit
                                spacing: 12
                                align: Align{ y: 0.5 }
                                Label{ text: "注音字色" }
                                gc0 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "青" } }
                                gc1 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "蓝" } }
                                gc2 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "橙" } }
                                gc3 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "灰" } }
                            }
                            gloss_bg_row := View{
                                width: Fill height: Fit
                                spacing: 12
                                align: Align{ y: 0.5 }
                                Label{ text: "注音背景" }
                                gb0 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "无" } }
                                gb1 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "深灰" } }
                                gb2 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "深青" } }
                                gb3 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "深蓝" } }
                            }
                            word_color_row := View{
                                width: Fill height: Fit
                                spacing: 12
                                align: Align{ y: 0.5 }
                                Label{ text: "词字色" }
                                wc0 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "默认" } }
                                wc1 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "青" } }
                                wc2 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "蓝" } }
                                wc3 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "橙" } }
                            }
                            word_bg_row := View{
                                width: Fill height: Fit
                                spacing: 12
                                align: Align{ y: 0.5 }
                                Label{ text: "词背景" }
                                wb0 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "无" } }
                                wb1 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "深灰" } }
                                wb2 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "深青" } }
                                wb3 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "深蓝" } }
                            }
                            Label{
                                text: "文本"
                                draw_text.text_style.font_size: 13
                                draw_text.color: #x9a9a9a
                            }
                            sel_color_row := View{
                                width: Fill height: Fit
                                spacing: 12
                                align: Align{ y: 0.5 }
                                Label{ text: "选中颜色" }
                                sc0 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "青" } }
                                sc1 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "蓝" } }
                                sc2 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "橙" } }
                                sc3 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "黄" } }
                            }
                            hl_color_row := View{
                                width: Fill height: Fit
                                spacing: 12
                                align: Align{ y: 0.5 }
                                Label{ text: "高亮颜色" }
                                hc0 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "黄" } }
                                hc1 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "青" } }
                                hc2 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "蓝" } }
                                hc3 := RoundedView{ width: Fit height: Fit padding: 2 draw_bg.border_radius: 6.0 draw_bg.color: #x2a2a2a b := ButtonFlat{ text: "橙" } }
                            }
                            settings_hint := Label{
                                width: Fill
                                text: "字体、注音颜色、深浅主题等 M8 补齐。"
                                draw_text.color: #x9a9a9a
                                draw_text.text_style.font_size: 9
                            }
                            Label{
                                text: "AI（OpenAI 兼容接口）"
                                draw_text.text_style.font_size: 13
                                draw_text.color: #x9a9a9a
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
                                draw_text.color: #x8f8f8f
                                draw_text.text_style.font_size: 9
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Script, ScriptHook, Widget)]
struct ChapterList {
    #[deref]
    view: View,
    /// 选区自跟踪（PortalList 未暴露区间 API，用公开的命中接口镜像；
    /// 仅镜像记录，绘制仍由原生选区完成）。
    #[rust]
    sel_anchor: Option<(usize, usize)>,
    #[rust]
    selecting: bool,
    #[rust]
    last_abs: Option<DVec2>,
}

impl ChapterList {
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
                self.selecting = false;
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
            _ => {}
        }
    }
}

impl Widget for ChapterList {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let paragraphs = PARAGRAPHS.read().unwrap();

        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_portal_list().borrow_mut() {
                list.set_item_range(cx, 0, paragraphs.len());
                while let Some(item_id) = list.next_visible_item(cx) {
                    let Some(para) = paragraphs.get(item_id) else {
                        continue;
                    };
                    let template = if para.contains("<ruby") {
                        id!(GlossPara)
                    } else {
                        id!(Para)
                    };
                    let item = list.item(cx, item_id, template);
                    let mut html = item.widget(cx, ids!(para_html));
                    html.set_text(cx, para);
                    if let Some(fs) = *READER_FONT_SIZE.read().unwrap() {
                        script_apply_eval!(cx, html, {
                            font_size: #(fs)
                        });
                    }
                    match *SELECTION_SLOT.read().unwrap() {
                        1 => script_apply_eval!(cx, html, { draw_selection.color: #x4a90d988 }),
                        2 => script_apply_eval!(cx, html, { draw_selection.color: #xe8875a88 }),
                        3 => script_apply_eval!(cx, html, { draw_selection.color: #xd9c96a88 }),
                        _ => script_apply_eval!(cx, html, { draw_selection.color: #x7fb2a688 }),
                    }
                    if let Some((start, end)) = *SEL_RANGE.read().unwrap() {
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
        self.view.handle_event(cx, event, scope);
        self.track_selection(cx, event);
    }
}

#[derive(Script, ScriptHook, Widget)]
struct TocList {
    #[deref]
    view: View,
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
                list.set_item_range(cx, 0, titles.len().max(1));
                while let Some(item_id) = list.next_visible_item(cx) {
                    let item = list.item(cx, item_id, id!(TocItem));
                    let text = match titles.get(item_id) {
                        Some(t) => format!("{}. {}", item_id + 1, t),
                        None => "（未打开书）".to_string(),
                    };
                    let mut item_mut = item.clone();
                    item.button(cx, ids!(toc_item_btn)).set_text(cx, &text);
                    if item_id == current {
                        script_apply_eval!(cx, item_mut, {
                            draw_bg.color: #x31554b
                        });
                    } else {
                        script_apply_eval!(cx, item_mut, {
                            draw_bg.color: #x232323
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
        let notes = NOTES.read().unwrap();
        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_portal_list().borrow_mut() {
                list.set_item_range(cx, 0, notes.len().max(1));
                while let Some(item_id) = list.next_visible_item(cx) {
                    let item = list.item(cx, item_id, id!(NoteCard));
                    if let Some(note) = notes.get(item_id) {
                        item.label(cx, ids!(note_title)).set_text(cx, &note.title);
                        item.label(cx, ids!(note_preview))
                            .set_text(cx, &preview_text(&note.content, 120));
                        item.widget(cx, ids!(note_actions)).set_visible(cx, true);
                    } else {
                        item.label(cx, ids!(note_title)).set_text(cx, "还没有笔记");
                        item.label(cx, ids!(note_preview))
                            .set_text(cx, "选中文本，点击「添加笔记」创建第一条。");
                        item.widget(cx, ids!(note_actions)).set_visible(cx, false);
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
struct ChatList {
    #[deref]
    view: View,
}

impl Widget for ChatList {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let data = CHAT_DATA.read().unwrap();
        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_portal_list().borrow_mut() {
                let message_count = data.messages.len();
                list.set_item_range(cx, 0, message_count + usize::from(data.is_streaming));
                while let Some(item_id) = list.next_visible_item(cx) {
                    if data.is_streaming && item_id == message_count {
                        let item = list.item(cx, item_id, id!(Assistant));
                        let text = if data.streaming_text.is_empty() {
                            "思考中…"
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
}

impl Widget for SkillList {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let skills = SKILLS.read().unwrap();
        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_portal_list().borrow_mut() {
                list.set_item_range(cx, 0, skills.len().max(1));
                while let Some(item_id) = list.next_visible_item(cx) {
                    let item = list.item(cx, item_id, id!(SkillCard));
                    if let Some(skill) = skills.get(item_id) {
                        item.label(cx, ids!(skill_name)).set_text(cx, &skill.name);
                        item.label(cx, ids!(skill_kind)).set_text(
                            cx,
                            if skill.is_system {
                                "系统"
                            } else {
                                "自定义"
                            },
                        );
                        item.label(cx, ids!(skill_preview))
                            .set_text(cx, &preview_text(&skill.content, 100));
                        item.check_box(cx, ids!(skill_active)).set_active(
                            cx,
                            skill.is_active,
                            Animate::No,
                        );
                        item.widget(cx, ids!(skill_actions)).set_visible(cx, true);
                        item.widget(cx, ids!(skill_use))
                            .set_visible(cx, !skill.is_system && skill.is_active);
                        item.widget(cx, ids!(skill_delete))
                            .set_visible(cx, !skill.is_system);
                    } else {
                        item.label(cx, ids!(skill_name)).set_text(cx, "还没有技能");
                        item.label(cx, ids!(skill_kind)).set_text(cx, "");
                        item.label(cx, ids!(skill_preview))
                            .set_text(cx, "点击「新建模板」创建第一个 Prompt 模板。");
                        item.widget(cx, ids!(skill_actions)).set_visible(cx, false);
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
struct BookShelf {
    #[deref]
    view: View,
    /// item_id → 已加载的封面路径（PortalList 复用 item，路径变了才重载）。
    #[rust]
    loaded_covers: HashMap<usize, Option<String>>,
}

impl Widget for BookShelf {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let shelf = SHELF.read().unwrap();

        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_portal_list().borrow_mut() {
                if shelf.is_empty() {
                    list.set_item_range(cx, 0, 1);
                    while let Some(item_id) = list.next_visible_item(cx) {
                        let item = list.item(cx, item_id, id!(Card));
                        item.label(cx, ids!(cover_letter)).set_text(cx, "书");
                        item.label(cx, ids!(card_title)).set_text(cx, "书架为空");
                        item.label(cx, ids!(card_author))
                            .set_text(cx, "拖入 EPUB 文件，或放进 ReadWise/import 目录");
                        item.label(cx, ids!(card_status)).set_text(cx, "");
                        item.draw_all_unscoped(cx);
                    }
                } else {
                    list.set_item_range(cx, 0, shelf.len());
                    while let Some(item_id) = list.next_visible_item(cx) {
                        let Some(book) = shelf.get(item_id) else {
                            continue;
                        };
                        let letter = book
                            .title
                            .chars()
                            .next()
                            .map(|c| c.to_string())
                            .unwrap_or_default();
                        let item = list.item(cx, item_id, id!(Card));
                        item.label(cx, ids!(cover_letter)).set_text(cx, &letter);
                        item.label(cx, ids!(card_title)).set_text(cx, &book.title);
                        item.label(cx, ids!(card_author)).set_text(cx, &book.author);
                        item.label(cx, ids!(card_status))
                            .set_text(cx, &book.status_line);
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
    chat_thread_id: Option<String>,
    #[rust]
    editing_skill_id: Option<String>,
    /// 仅应用到下一条用户消息的自定义技能。
    #[rust]
    pending_skill_id: Option<String>,
    #[rust]
    chapter_idx: usize,
    #[rust]
    on_reader_page: bool,
    #[rust]
    toc_open: bool,
    #[rust]
    note_open: bool,
    #[rust]
    reader_ai_open: bool,
    #[rust]
    note_quote_text: String,
    /// 释义卡片当前展示的 (word, lemma, gloss)。
    #[rust]
    card: Option<(String, String, String)>,
}

impl App {
    fn text_input_has_focus(&self, cx: &Cx) -> bool {
        [
            ids!(note_extra),
            ids!(reader_chat_input),
            ids!(chat_input),
            ids!(ai_endpoint),
            ids!(ai_model),
            ids!(ai_api_key),
            ids!(skill_name_input),
            ids!(skill_content_input),
        ]
        .into_iter()
        .any(|path| self.ui.text_input(cx, path).key_focus(cx))
    }

    fn clear_skill_editor(&mut self, cx: &mut Cx) {
        self.editing_skill_id = None;
        self.ui
            .label(cx, ids!(skill_editor_title))
            .set_text(cx, "新建技能");
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
        self.ui.label(cx, ids!(skill_editor_hint)).set_text(
            cx,
            "自定义技能可用于下一次 AI 提问；启用的系统技能会自动作为全局系统提示词。",
        );
    }

    fn edit_skill(&mut self, cx: &mut Cx, skill: &readwise_core::skills::models::Skill) {
        self.editing_skill_id = Some(skill.id.clone());
        self.ui.label(cx, ids!(skill_editor_title)).set_text(
            cx,
            if skill.is_system {
                "编辑系统技能"
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
                "系统技能启用后自动作为所有 AI 对话的系统提示词；系统技能不可删除。"
            } else {
                "保存后可从左侧点击「下次提问使用」，模板只应用一次。"
            },
        );
    }

    fn refresh_pending_skill_indicator(&self, cx: &mut Cx) {
        let selected = self.pending_skill_id.as_ref().and_then(|id| {
            SKILLS
                .read()
                .unwrap()
                .iter()
                .find(|skill| &skill.id == id && skill.is_active && !skill.is_system)
                .map(|skill| skill.name.clone())
        });
        match selected {
            Some(name) => {
                self.ui
                    .label(cx, ids!(chat_skill_status))
                    .set_text(cx, &format!("下一条消息将使用技能：{}", name));
                self.ui
                    .widget(cx, ids!(chat_skill_clear))
                    .set_visible(cx, true);
            }
            None => {
                self.ui
                    .label(cx, ids!(chat_skill_status))
                    .set_text(cx, "未选择技能模板");
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
        self.ui
            .page_flip(cx, ids!(pages))
            .set_active_page(cx, id!(page_chat));
        self.on_reader_page = false;
        self.set_chat_status(cx, &format!("已选择「{}」，请输入要处理的内容", skill.name));
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
            self.ui.widget(cx, ids!(toc_panel)).set_visible(cx, false);
        }
        self.ui
            .widget(cx, ids!(reader_ai_panel))
            .set_visible(cx, visible);
        self.ui.redraw(cx);
    }

    fn configure_ai_backend(&mut self, cx: &mut Cx) {
        if self.ai_request.is_some() {
            self.cancel_chat(cx);
        }
        let endpoint = match readwise_core::ai::normalize_chat_endpoint(&self.ai_settings.endpoint)
        {
            Ok(endpoint) => endpoint,
            Err(error) => {
                self.ai_backend = None;
                self.set_chat_status(cx, &error);
                return;
            }
        };
        let model = self.ai_settings.model.trim();
        if model.is_empty() {
            self.ai_backend = None;
            self.set_chat_status(cx, "请先到设置页填写模型名称");
            return;
        }
        self.ai_backend = Some(OpenAiBackend::new(BackendConfig::OpenAI {
            api_key: self.ai_settings.api_key.clone(),
            model: model.to_string(),
            base_url: Some(endpoint.clone()),
            reasoning_effort: None,
        }));
        self.set_chat_status(
            cx,
            &format!("已就绪 · {} · {}", self.ai_settings.model.trim(), endpoint),
        );
    }

    fn redraw_chat(&self, cx: &mut Cx) {
        let main_list = self
            .ui
            .widget(cx, ids!(chat_list))
            .portal_list(cx, ids!(list));
        let reader_list = self
            .ui
            .widget(cx, ids!(reader_chat_list))
            .portal_list(cx, ids!(list));
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
        self.ui.redraw(cx);
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
            self.set_chat_status(cx, "上一条回答仍在生成，可先停止");
            return false;
        }
        if text.trim().is_empty() {
            return false;
        }
        if self.ai_backend.is_none() {
            self.set_chat_status(cx, "AI 尚未配置，请先到设置页保存接口信息");
            return false;
        }
        if self.chat_thread_id.is_none() {
            self.chat_thread_id = Some(readwise_core::threads::new_thread_id());
        }

        let pending_skill = self.pending_skill_id.as_ref().and_then(|id| {
            SKILLS
                .read()
                .unwrap()
                .iter()
                .find(|skill| &skill.id == id && skill.is_active && !skill.is_system)
                .cloned()
        });
        let system_prompt = active_system_prompt();
        let visible_text = text.trim().to_string();
        let request_text = pending_skill.as_ref().map_or_else(
            || visible_text.clone(),
            |skill| {
                format!(
                    "请严格按照下面的技能模板完成用户请求。\n\n【技能：{}】\n{}\n\n【用户请求】\n{}",
                    skill.name, skill.content, visible_text
                )
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
        self.set_chat_status(cx, "正在生成…");
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
        self.set_chat_status(cx, "已停止，已保留当前内容");
        self.ai_stream.reset();
        self.redraw_chat(cx);
    }

    fn clear_chat(&mut self, cx: &mut Cx) {
        self.cancel_chat(cx);
        *CHAT_DATA.write().unwrap() = ChatData::default();
        self.chat_thread_id = None;
        self.set_chat_status(cx, "新对话已就绪");
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
        self.set_chat_status(cx, "回答完成 · 正在保存对话");
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
        self.set_chat_status(cx, &format!("请求失败：{error}"));
        self.ai_stream.reset();
        spawn_save_chat(self.chat_thread_id.clone(), messages);
        self.redraw_chat(cx);
    }

    /// 切换到指定章节：按 </p> 切块喂给 PortalList，更新标题/页码，保存进度，滚回顶部。
    fn set_chapter(&mut self, cx: &mut Cx, idx: usize) {
        let (id, title, total, chunks) = {
            let guard = CURRENT_BOOK.read().unwrap();
            let Some(book) = guard.as_ref() else { return };
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
            _ => script_apply_eval!(cx, w, { draw_bg.color: #x2a2a2a }),
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

    /// 用当前章内容重建 PortalList 块（保滚动位置：块数不变时 first_id 不动）。
    fn refresh_current_chapter(&mut self, cx: &mut Cx) {
        let chunks = {
            let guard = CURRENT_BOOK.read().unwrap();
            guard
                .as_ref()
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

    fn open_note_composer(&mut self, cx: &mut Cx) {
        let quote = SELECTED_TEXT.read().unwrap().trim().to_string();
        if quote.is_empty() {
            return;
        }
        self.note_quote_text = quote;
        self.note_open = true;
        self.ui.widget(cx, ids!(note_panel)).set_visible(cx, true);
        self.ui
            .widget(cx, ids!(note_composer))
            .set_visible(cx, true);
        self.ui
            .label(cx, ids!(note_quote))
            .set_text(cx, &self.note_quote_text);
        self.ui.text_input(cx, ids!(note_extra)).set_text(cx, "");
        self.ui
            .label(cx, ids!(note_hint))
            .set_text(cx, "补充内容可留空；保存后引用会一并进入笔记。");
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
            self.ui
                .label(cx, ids!(note_hint))
                .set_text(cx, "请先从书库打开一本书");
            return;
        };
        if self.note_quote_text.trim().is_empty() && extra.trim().is_empty() {
            return;
        }
        self.ui.label(cx, ids!(note_hint)).set_text(cx, "保存中…");
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

    /// 注音总开关切换：关→原地剥掉全部注音；开→重开当前书注音。
    fn toggle_gloss(&mut self, cx: &mut Cx, on: bool) {
        *GLOSS_ENABLED.write().unwrap() = on;
        self.set_gloss_rows_visible(cx, on);
        if on {
            let item = CURRENT_ITEM.read().unwrap().clone();
            if let Some(item) = item {
                self.ui
                    .label(cx, ids!(title))
                    .set_text(cx, &format!("《{}》注音中…", item.title));
                spawn_open_book(item);
            }
        } else {
            if let Some(book) = CURRENT_BOOK.write().unwrap().as_mut() {
                for ch in book.chapters.iter_mut() {
                    if ch.contains("<ruby") {
                        *ch = readwise_core::vocab::annotate::remove_all_ruby(ch);
                    }
                }
            }
            self.refresh_current_chapter(cx);
        }
        self.ui.redraw(cx);
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
    }
}

impl MatchEvent for App {
    fn handle_timer(&mut self, cx: &mut Cx, event: &TimerEvent) {
        if self.ai_request.is_some() && self.ai_timeout.is_timer(event).is_some() {
            self.fail_chat(
                cx,
                "请求超过 60 秒仍未完成，请检查网络或接口设置".to_string(),
            );
        }
    }

    fn handle_actions(&mut self, cx: &mut Cx, actions: &Actions) {
        for action in actions {
            if let Some(db_action) = action.downcast_ref::<DbAction>() {
                match db_action {
                    DbAction::Ready {
                        vocab_count,
                        known_count,
                        vocab_preview,
                    } => {
                        let status = format!(
                            "数据库就绪\n生词本 {} · 已认识 {}",
                            vocab_count, known_count
                        );
                        self.ui.label(cx, ids!(db_status)).set_text(cx, &status);
                        let summary = format!("共 {} 词（预览前 50 条）", vocab_count);
                        self.ui
                            .label(cx, ids!(vocab_summary))
                            .set_text(cx, &summary);
                        let list = if vocab_preview.is_empty() {
                            "生词本为空——去阅读页点注音词收藏（M4 打通）。".to_string()
                        } else {
                            vocab_preview.clone()
                        };
                        self.ui.label(cx, ids!(vocab_list)).set_text(cx, &list);
                        self.refresh_swatches(cx);
                        let gloss_on = *GLOSS_ENABLED.read().unwrap();
                        self.ui.check_box(cx, ids!(gloss_enable)).set_active(
                            cx,
                            gloss_on,
                            Animate::No,
                        );
                        self.set_gloss_rows_visible(cx, gloss_on);
                        spawn_load_shelf();
                        spawn_load_ai_state();
                        spawn_load_skills();
                    }
                    DbAction::Failed(e) => {
                        let status = format!("数据库失败:\n{}", e);
                        self.ui.label(cx, ids!(db_status)).set_text(cx, &status);
                    }
                }
                continue;
            }
            if let Some(skill_action) = action.downcast_ref::<SkillAction>() {
                match skill_action {
                    SkillAction::Loaded(skills) => {
                        let count = skills.len();
                        *SKILLS.write().unwrap() = skills.clone();
                        self.ui.label(cx, ids!(skills_status)).set_text(
                            cx,
                            &format!(
                                "共 {} 个技能 · 系统技能自动生效，自定义技能可用于下一次提问",
                                count
                            ),
                        );
                    }
                    SkillAction::Saved(skills) => {
                        *SKILLS.write().unwrap() = skills.clone();
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
                        self.ui
                            .label(cx, ids!(skills_status))
                            .set_text(cx, "技能已保存 ✓");
                    }
                    SkillAction::Toggled(skills) => {
                        *SKILLS.write().unwrap() = skills.clone();
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
                        self.ui
                            .label(cx, ids!(skills_status))
                            .set_text(cx, "技能状态已更新 ✓");
                    }
                    SkillAction::Deleted { skills, id } => {
                        *SKILLS.write().unwrap() = skills.clone();
                        if self.editing_skill_id.as_deref() == Some(id.as_str()) {
                            self.clear_skill_editor(cx);
                        }
                        if self.pending_skill_id.as_deref() == Some(id.as_str()) {
                            self.pending_skill_id = None;
                            self.refresh_pending_skill_indicator(cx);
                        }
                        self.ui
                            .label(cx, ids!(skills_status))
                            .set_text(cx, "技能已删除");
                    }
                    SkillAction::Error(error) => {
                        self.ui
                            .label(cx, ids!(skills_status))
                            .set_text(cx, &format!("技能操作失败：{}", error));
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
                            .set_text(cx, "AI 设置已保存 ✓");
                    }
                    AiStateAction::ThreadSaved(thread_id) => {
                        if self.chat_thread_id.as_deref() == Some(thread_id.as_str()) {
                            self.set_chat_status(cx, "回答完成 · 对话已保存 ✓");
                        }
                    }
                    AiStateAction::Error(error) => {
                        self.ui
                            .label(cx, ids!(ai_settings_status))
                            .set_text(cx, &format!("AI 数据操作失败：{}", error));
                        self.set_chat_status(cx, &format!("AI 数据操作失败：{}", error));
                    }
                }
                continue;
            }
            if let Some(card) = action.downcast_ref::<CardAction>() {
                match card {
                    CardAction::VocabAdded(added) => {
                        let hint = if *added {
                            "已加入生词本 ✓"
                        } else {
                            "生词本里已有这个词"
                        };
                        self.ui.label(cx, ids!(card_hint)).set_text(cx, hint);
                        self.ui.label(cx, ids!(sel_hint)).set_text(cx, hint);
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
                    self.ui
                        .label(cx, ids!(sel_hint))
                        .set_text(cx, &format!("已选择：{}{}", preview, suffix));
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
                        *NOTES.write().unwrap() = notes.clone();
                        let text = message
                            .clone()
                            .unwrap_or_else(|| format!("共 {} 条笔记", count));
                        self.ui.label(cx, ids!(note_hint)).set_text(cx, &text);
                        if *finish_composer {
                            self.close_note_composer(cx);
                            self.clear_reader_selection(cx);
                        }
                        self.ui.redraw(cx);
                    }
                    NoteAction::Error(e) => {
                        self.ui
                            .label(cx, ids!(note_hint))
                            .set_text(cx, &format!("笔记操作失败：{}", e));
                    }
                }
                continue;
            }
            if let Some(reader) = action.downcast_ref::<ReaderAction>() {
                match reader {
                    ReaderAction::Opened {
                        resume_chapter,
                        gloss_count,
                    } => {
                        self.set_chapter(cx, *resume_chapter);
                        if *GLOSS_ENABLED.read().unwrap() && *gloss_count == 0 {
                            self.ui
                                .label(cx, ids!(status))
                                .set_text(cx, "注音未显示：释义库为空（需生成或导入词义数据）");
                        }
                        if let Some(book_id) = CURRENT_BOOK
                            .read()
                            .unwrap()
                            .as_ref()
                            .map(|book| book.id.clone())
                        {
                            spawn_load_notes(book_id);
                        }
                    }
                    ReaderAction::OpenError(e) => {
                        self.ui
                            .label(cx, ids!(title))
                            .set_text(cx, &format!("打开失败: {}", e));
                    }
                }
                continue;
            }
            if let Some(shelf) = action.downcast_ref::<ShelfAction>() {
                match shelf {
                    ShelfAction::Loaded { items, imported } => {
                        let hint = if *imported > 0 {
                            format!("共 {} 本（导入目录新增 {} 本）", items, imported)
                        } else {
                            format!("共 {} 本 · 拖入 EPUB 或放进 import 目录添加", items)
                        };
                        self.ui.label(cx, ids!(library_hint)).set_text(cx, &hint);
                        self.ui.redraw(cx);
                    }
                    ShelfAction::Error(e) => {
                        self.ui
                            .label(cx, ids!(library_hint))
                            .set_text(cx, &format!("书架加载失败: {}", e));
                    }
                }
                continue;
            }
            let Some(action) = action.as_widget_action() else {
                continue;
            };
            if let HtmlLinkAction::Clicked { url, .. } = action.cast() {
                if let Some(word) = url.strip_prefix("w:") {
                    let status = format!("点词：{}", word);
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
        let shelf_list = self
            .ui
            .widget(cx, ids!(book_shelf))
            .portal_list(cx, ids!(list));
        let pages = self.ui.page_flip(cx, ids!(pages));
        for (item_id, item) in shelf_list.items_with_actions(actions) {
            if item.button(cx, ids!(del_btn)).clicked(actions) {
                let book_id = SHELF.read().unwrap().get(item_id).map(|b| b.id.clone());
                if let Some(id) = book_id {
                    spawn_delete(id);
                }
            }
            if item.button(cx, ids!(read_btn)).clicked(actions) {
                let book = SHELF.read().unwrap().get(item_id).cloned();
                if let Some(book) = book {
                    self.ui
                        .label(cx, ids!(title))
                        .set_text(cx, &format!("《{}》加载中…", book.title));
                    pages.set_active_page(cx, id!(page_reader));
                    self.on_reader_page = true;
                    spawn_open_book(book);
                }
            }
        }
        let skill_list = self
            .ui
            .widget(cx, ids!(skill_list))
            .portal_list(cx, ids!(list));
        for (item_id, item) in skill_list.items_with_actions(actions) {
            let skill = SKILLS.read().unwrap().get(item_id).cloned();
            let Some(skill) = skill else { continue };
            if let Some(is_active) = item.check_box(cx, ids!(skill_active)).changed(actions) {
                self.ui
                    .label(cx, ids!(skills_status))
                    .set_text(cx, "正在更新技能状态…");
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
                    .set_text(cx, "正在删除技能…");
                spawn_delete_skill(skill.id.clone());
            }
        }
        if self.ui.button(cx, ids!(nav_library)).clicked(actions) {
            pages.set_active_page(cx, id!(page_library));
            self.on_reader_page = false;
        }
        if self.ui.button(cx, ids!(nav_reader)).clicked(actions) {
            pages.set_active_page(cx, id!(page_reader));
            self.on_reader_page = true;
        }
        if self.ui.button(cx, ids!(nav_chat)).clicked(actions) {
            pages.set_active_page(cx, id!(page_chat));
            self.on_reader_page = false;
        }
        if self.ui.button(cx, ids!(nav_skills)).clicked(actions) {
            pages.set_active_page(cx, id!(page_skills));
            self.on_reader_page = false;
        }
        if self.ui.button(cx, ids!(nav_vocab)).clicked(actions) {
            pages.set_active_page(cx, id!(page_vocab));
            self.on_reader_page = false;
        }
        if self.ui.button(cx, ids!(nav_settings)).clicked(actions) {
            pages.set_active_page(cx, id!(page_settings));
            self.on_reader_page = false;
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
                self.ui
                    .label(cx, ids!(skills_status))
                    .set_text(cx, "技能名称不能为空");
            } else if content.trim().is_empty() {
                self.ui
                    .label(cx, ids!(skills_status))
                    .set_text(cx, "技能内容不能为空");
            } else {
                let is_active = self.ui.check_box(cx, ids!(skill_active_edit)).active(cx);
                self.ui
                    .label(cx, ids!(skills_status))
                    .set_text(cx, "正在保存技能…");
                spawn_save_skill(self.editing_skill_id.clone(), name, content, is_active);
            }
        }
        if self.ui.button(cx, ids!(chat_skill_clear)).clicked(actions) {
            self.pending_skill_id = None;
            self.refresh_pending_skill_indicator(cx);
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
                    .set_text(cx, "已复制到剪贴板 ✓");
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
                self.ui.label(cx, ids!(sel_hint)).set_text(cx, "加入中…");
                spawn_add_vocab(lemma, word, String::new(), book_id);
            } else {
                self.ui
                    .label(cx, ids!(sel_hint))
                    .set_text(cx, "加入生词仅支持选择一个英文单词");
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
                self.ui
                    .label(cx, ids!(sel_hint))
                    .set_text(cx, "已在浏览器中打开翻译");
            }
        }
        if self.ui.button(cx, ids!(ask_ai_sel_btn)).clicked(actions) {
            let selected = SELECTED_TEXT.read().unwrap().trim().to_string();
            if !selected.is_empty() {
                let prompt = format!("请解释下面这段英文，并指出重要词语和表达：\n\n{}", selected);
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
                self.ui.label(cx, ids!(sel_hint)).set_text(cx, "正在发音…");
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
            self.ui.widget(cx, ids!(note_panel)).set_visible(cx, false);
            self.close_note_composer(cx);
        }
        if self.ui.button(cx, ids!(close_card_btn)).clicked(actions) {
            self.card = None;
            self.ui.widget(cx, ids!(word_card)).set_visible(cx, false);
            self.ui.redraw(cx);
        }
        if self.ui.button(cx, ids!(add_vocab_btn)).clicked(actions) {
            if let Some((word, lemma, gloss)) = self.card.clone() {
                let book_id = CURRENT_ITEM.read().unwrap().as_ref().map(|i| i.id.clone());
                self.ui.label(cx, ids!(card_hint)).set_text(cx, "加入中…");
                spawn_add_vocab(lemma, word, gloss, book_id);
            }
        }
        if self.ui.button(cx, ids!(know_btn)).clicked(actions) {
            if let Some((_, lemma, _)) = self.card.clone() {
                self.ui.label(cx, ids!(card_hint)).set_text(cx, "标记中…");
                spawn_mark_known(lemma);
            }
        }
        if self.ui.button(cx, ids!(toc_btn)).clicked(actions) {
            self.toc_open = !self.toc_open;
            self.ui
                .widget(cx, ids!(toc_panel))
                .set_visible(cx, self.toc_open);
            self.ui.redraw(cx);
        }
        if self.ui.button(cx, ids!(notes_btn)).clicked(actions) {
            self.note_open = !self.note_open;
            if self.note_open {
                self.set_reader_ai_visible(cx, false);
            }
            self.ui
                .widget(cx, ids!(note_panel))
                .set_visible(cx, self.note_open);
            self.ui.redraw(cx);
        }
        if self.ui.button(cx, ids!(reader_ai_btn)).clicked(actions) {
            let visible = !self.reader_ai_open;
            if visible {
                self.note_open = false;
                self.ui.widget(cx, ids!(note_panel)).set_visible(cx, false);
            }
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
                    self.ui.widget(cx, ids!(toc_panel)).set_visible(cx, false);
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
                let note_id = NOTES.read().unwrap().get(item_id).map(|n| n.id.clone());
                let book_id = CURRENT_BOOK.read().unwrap().as_ref().map(|b| b.id.clone());
                if let (Some(book_id), Some(note_id)) = (book_id, note_id) {
                    self.ui.label(cx, ids!(note_hint)).set_text(cx, "删除中…");
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
                    self.ui
                        .label(cx, ids!(ai_settings_status))
                        .set_text(cx, "模型名称不能为空");
                    return;
                }
                self.ui
                    .label(cx, ids!(ai_settings_status))
                    .set_text(cx, "正在保存…");
                spawn_save_ai_settings(settings);
            }
        }
        if self.ui.button(cx, ids!(reload_button)).clicked(actions) {
            self.reload_count += 1;
            *PARAGRAPHS.write().unwrap() = sample_chapter::build_paragraphs(PARA_COUNT);
            let status = format!("已重新渲染 {} 次", self.reload_count);
            self.ui.label(cx, ids!(status)).set_text(cx, &status);
            self.ui.redraw(cx);
        }
    }
}

impl AppMain for App {
    fn script_mod(vm: &mut ScriptVm) -> ScriptValue {
        makepad_widgets::theme_mod(vm);
        script_eval!(vm, {
            mod.theme = mod.themes.dark
        });
        makepad_widgets::widgets_mod(vm);
        crate::ruby_word::script_mod(vm);
        crate::chapter_image::script_mod(vm);
        crate::mark_span::script_mod(vm);
        self::script_mod(vm)
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
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
            for update in stream_updates {
                match update {
                    AiSseUpdate::ReasoningStarted => {
                        self.set_chat_status(cx, "模型正在思考…");
                        self.redraw_chat(cx);
                    }
                    AiSseUpdate::Text(text) => {
                        CHAT_DATA.write().unwrap().streaming_text.push_str(&text);
                        self.set_chat_status(cx, "正在输出回答…");
                        self.redraw_chat(cx);
                    }
                    AiSseUpdate::Done => {}
                    AiSseUpdate::Failure(error) => {
                        stream_failure.get_or_insert(error);
                    }
                }
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
                                    "模型完成了思考，但没有返回可显示的正文（可能达到输出上限）"
                                } else {
                                    "服务端完成了请求，但没有返回文本"
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
            Event::Startup => spawn_db_init(),
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
                    spawn_import(paths);
                }
            }
            _ => {}
        }
        self.match_event(cx, event);
        self.ui.handle_event(cx, event, &mut Scope::empty());
        if matches!(event, Event::MouseUp(_)) {
            SELECTION_POPUP_POINTER.store(false, Ordering::Relaxed);
        }
    }
}
