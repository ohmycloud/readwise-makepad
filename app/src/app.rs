use crate::chapter_image::ChapterImageAction;
use crate::ruby_word::RubyWordAction;
use crate::sample_chapter;
use makepad_ai::{
    AiBackend, AiEvent, AiRequest, BackendConfig, Message, OpenAiBackend, RequestId, StreamDelta,
};
use makepad_widgets::makepad_draw::text::geom::Point as LayoutPoint;
use makepad_widgets::makepad_draw::text::layouter::LaidoutText;
use makepad_widgets::makepad_draw::text::selection::{
    Cursor as LayoutCursor, Selection as LayoutSelection,
};
use makepad_widgets::makepad_platform::event::{TouchPoint, TouchState};
use makepad_widgets::*;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock, RwLock};
use std::time::Instant;

type AiSettings = readwise_core::ai::AiSettings;
type AppLanguage = readwise_core::settings::AppLanguage;
type LibraryViewMode = readwise_core::settings::LibraryViewMode;
type LocalAiSettings = readwise_core::local_ai::LocalAiSettings;
type LocalAiSession = readwise_core::local_ai::LocalAiSession;
type ReaderBackground = readwise_core::settings::ReaderBackground;
type ReaderFont = readwise_core::settings::ReaderFont;
type ReaderSpacing = readwise_core::settings::ReaderSpacing;
type ReaderSettings = readwise_core::settings::ReaderSettings;
type ReaderTheme = readwise_core::settings::ReaderTheme;
type StatisticsSnapshot = readwise_core::statistics::models::StatisticsSnapshot;
type VocabEntry = readwise_core::vocab::models::VocabBookEntry;

app_main!(App);

const PARA_COUNT: usize = 200;
const COMPACT_LAYOUT_BREAKPOINT: f64 = 720.0;
const MOBILE_LANDSCAPE_COMPACT_HEIGHT: f64 = 600.0;
const READER_SWIPE_MIN_DISTANCE: f64 = 64.0;
const READER_SWIPE_MAX_DURATION: f64 = 0.9;
const READER_SWIPE_AXIS_RATIO: f64 = 1.35;
const READER_SWIPE_VERTICAL_CANCEL: f64 = 28.0;

fn uses_compact_layout_for_platform(size: DVec2, is_mobile: bool) -> bool {
    size.x > 0.0
        && size.y > 0.0
        && (size.x < COMPACT_LAYOUT_BREAKPOINT
            || (is_mobile && size.y < MOBILE_LANDSCAPE_COMPACT_HEIGHT))
}

fn uses_compact_layout(size: DVec2) -> bool {
    uses_compact_layout_for_platform(size, cfg!(any(target_os = "android", target_os = "ios")))
}

fn screen_orientation_changed(old_size: DVec2, new_size: DVec2) -> bool {
    if old_size.x <= 0.0 || old_size.y <= 0.0 || new_size.x <= 0.0 || new_size.y <= 0.0 {
        return false;
    }
    (old_size.x > old_size.y) != (new_size.x > new_size.y)
}

fn should_show_mobile_nav(compact_layout: bool, virtual_keyboard_open: bool) -> bool {
    compact_layout && !virtual_keyboard_open
}

fn library_surface_visibility(visible_count: usize) -> (bool, bool) {
    // Keep the data-owning PortalList mounted even when it is empty. If it is
    // hidden, a responsive rebuild can leave it uninstantiated and later
    // set_visible calls become no-ops until another user action finds it.
    (visible_count == 0, true)
}

fn shelf_grid_columns_for_width(width: f64) -> usize {
    if width >= 1080.0 {
        4
    } else if width >= 760.0 {
        3
    } else {
        2
    }
}

fn shelf_grid_row_count(book_count: usize, columns: usize) -> usize {
    book_count.div_ceil(columns.max(1))
}

fn shelf_grid_book_index(
    row: usize,
    column: usize,
    columns: usize,
    book_count: usize,
) -> Option<usize> {
    let index = row.checked_mul(columns)?.checked_add(column)?;
    (index < book_count).then_some(index)
}

fn shelf_cover_needs_load(
    loaded_covers: &HashMap<WidgetUid, Option<String>>,
    card_uid: WidgetUid,
    cover_path: &Option<String>,
) -> bool {
    // An empty ref cannot be cached safely. A real UID identifies the actual
    // PortalList card instance that owns the image texture, not merely the
    // book currently projected into that card.
    card_uid == WidgetUid(0) || loaded_covers.get(&card_uid) != Some(cover_path)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReaderPageSummary {
    current: usize,
    total: usize,
    pages_left_in_chapter: usize,
}

/// EPUB 是可重排内容，没有出版物固有的 PDF 页码。这里以当前视口一次稳定翻过的
/// 段落数作为页宽，为每章建立同一套虚拟页；页码与翻页目标因此始终一致。
fn reader_page_summary(
    chapter_block_counts: &[usize],
    chapter: usize,
    first_block: usize,
    blocks_per_page: usize,
) -> Option<ReaderPageSummary> {
    if chapter_block_counts.is_empty()
        || chapter >= chapter_block_counts.len()
        || chapter_block_counts.contains(&0)
    {
        return None;
    }
    let span = blocks_per_page.max(1);
    let chapter_pages = |blocks: usize| blocks.div_ceil(span).max(1);
    let pages_before = chapter_block_counts[..chapter]
        .iter()
        .copied()
        .map(chapter_pages)
        .sum::<usize>();
    let pages_in_chapter = chapter_pages(chapter_block_counts[chapter]);
    let page_in_chapter = (first_block / span).min(pages_in_chapter.saturating_sub(1));
    Some(ReaderPageSummary {
        current: pages_before + page_in_chapter + 1,
        total: chapter_block_counts
            .iter()
            .copied()
            .map(chapter_pages)
            .sum(),
        pages_left_in_chapter: pages_in_chapter.saturating_sub(page_in_chapter + 1),
    })
}

fn reader_last_page_start(block_count: usize, blocks_per_page: usize) -> usize {
    let last = block_count.saturating_sub(1);
    let span = blocks_per_page.max(1);
    (last / span) * span
}

fn word_range_at(text: &str, byte_index: usize) -> Option<(usize, usize)> {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return None;
    }
    let mut byte_index = byte_index.min(text.len().saturating_sub(1));
    while byte_index > 0 && !text.is_char_boundary(byte_index) {
        byte_index -= 1;
    }
    let index = text[..byte_index].chars().count();
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
        return Some((byte_index, byte_index + selected.len_utf8()));
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
    let start = text
        .char_indices()
        .nth(start)
        .map(|(offset, _)| offset)
        .unwrap_or(text.len());
    let end = text
        .char_indices()
        .nth(end)
        .map(|(offset, _)| offset)
        .unwrap_or(text.len());
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
    #[default]
    Library,
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

    fn from_persisted(value: usize) -> Self {
        match value {
            1 => Self::Reader,
            2 => Self::Chat,
            3 => Self::Skills,
            4 => Self::Vocab,
            5 => Self::Statistics,
            6 => Self::Settings,
            _ => Self::Library,
        }
    }

    fn persisted(self) -> usize {
        match self {
            Self::Library => 0,
            Self::Reader => 1,
            Self::Chat => 2,
            Self::Skills => 3,
            Self::Vocab => 4,
            Self::Statistics => 5,
            Self::Settings => 6,
        }
    }
}

fn restored_page_after_rebuild(value: usize, has_current_book: bool) -> AppPage {
    let page = AppPage::from_persisted(value);
    if page == AppPage::Reader && !has_current_book {
        AppPage::Library
    } else {
        page
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
static CODE_BLOCKS: LazyLock<RwLock<Vec<Option<String>>>> = LazyLock::new(|| {
    RwLock::new(
        sample_chapter::build_paragraphs(PARA_COUNT)
            .iter()
            .map(|chunk| readwise_core::books::epub_meta::code_block_text(chunk))
            .collect(),
    )
});

fn replace_paragraphs(chunks: Vec<String>) {
    let code_blocks = chunks
        .iter()
        .map(|chunk| readwise_core::books::epub_meta::code_block_text(chunk))
        .collect();
    *CODE_BLOCKS.write().unwrap() = code_blocks;
    *PARAGRAPHS.write().unwrap() = chunks;
}

fn is_table_block(html: &str) -> bool {
    let html = html.trim_start();
    html.get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("<table"))
        && html
            .as_bytes()
            .get(6)
            .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b'>' || *byte == b'/')
}

/// Html/TextFlow inserts a selection-only newline when a top-level paragraph or
/// heading starts. Persistent highlights, notes, and `rendered_text` use the
/// visible source-text coordinate space, so the reader translates this prefix
/// only at the TextFlow boundary.
fn textflow_selection_prefix_len(html: &str) -> usize {
    let trimmed = html.trim_start();
    if !trimmed.starts_with('<') {
        return 0;
    }
    let Some(tag_end) = trimmed.find('>') else {
        return 0;
    };
    let tag = trimmed[1..tag_end]
        .split_ascii_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches('/')
        .to_ascii_lowercase();
    usize::from(matches!(
        tag.as_str(),
        "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
    ))
}

fn needs_unified_reader_hit_test(html: &str) -> bool {
    let mut remaining = html;
    let mut has_fragmented_text = false;
    let mut has_embedded_visual = false;
    while let Some(tag_start) = remaining.find('<') {
        remaining = &remaining[tag_start + 1..];
        let candidate = remaining
            .trim_start()
            .strip_prefix('/')
            .unwrap_or(remaining.trim_start())
            .trim_start();
        let name_end = candidate
            .find(|ch: char| !ch.is_ascii_alphanumeric())
            .unwrap_or(candidate.len());
        let name = &candidate[..name_end];
        let name = name.to_ascii_lowercase();
        if matches!(name.as_str(), "img" | "image" | "svg") {
            // A plain-text layout cannot represent the horizontal space
            // occupied by an inline visual. Keep the widget-aware path for
            // these mixed-media blocks instead of shifting every later index.
            has_embedded_visual = true;
        } else if matches!(
            name.as_str(),
            // Each of these changes TextFlow state or enters a custom/link
            // widget, splitting one visual line into independently laid-out
            // selection segments.
            "a" | "b"
                | "strong"
                | "i"
                | "em"
                | "code"
                | "u"
                | "del"
                | "s"
                | "strike"
                | "sub"
                | "sup"
                | "mark"
                | "ruby"
        ) {
            has_fragmented_text = true;
        }
        let Some(tag_end) = remaining.find('>') else {
            break;
        };
        remaining = &remaining[tag_end + 1..];
    }
    has_fragmented_text && !has_embedded_visual
}

fn selection_range_in_block(
    range: TextRange,
    block_id: usize,
    text_len: usize,
) -> Option<(usize, usize)> {
    let (start, end) = range;
    if block_id < start.0 || block_id > end.0 {
        return None;
    }
    let from = if block_id == start.0 { start.1 } else { 0 }.min(text_len);
    let to = if block_id == end.0 { end.1 } else { text_len }.min(text_len);
    (from < to).then_some((from, to))
}

fn highlight_range_in_block(
    highlight: &readwise_core::books::Highlight,
    block_id: usize,
    text_len: usize,
) -> Option<(usize, usize)> {
    if block_id < highlight.start_block || block_id > highlight.end_block {
        return None;
    }
    let from = if block_id == highlight.start_block {
        highlight.start_char
    } else {
        0
    }
    .min(text_len);
    let to = if block_id == highlight.end_block {
        highlight.end_char
    } else {
        text_len
    }
    .min(text_len);
    (from < to).then_some((from, to))
}

#[cfg(test)]
mod table_block_tests {
    use super::{
        is_table_block, needs_unified_reader_hit_test, selection_range_in_block,
        textflow_selection_prefix_len, visual_selection_row_bounds,
    };

    #[test]
    fn recognizes_only_top_level_table_blocks() {
        assert!(is_table_block("  <TABLE class='data'><tr></tr></TABLE>"));
        assert!(!is_table_block("<p>Table 1</p><table></table>"));
        assert!(!is_table_block("<tableau>not a table</tableau>"));
    }

    #[test]
    fn reader_selection_separates_textflow_prefix_from_source_offsets() {
        assert_eq!(textflow_selection_prefix_len(" <p>words</p>"), 1);
        assert_eq!(
            textflow_selection_prefix_len("<H2 id='title'>Title</H2>"),
            1
        );
        assert_eq!(
            textflow_selection_prefix_len("<blockquote><p>x</p></blockquote>"),
            0
        );
        assert_eq!(textflow_selection_prefix_len("plain text"), 0);
    }

    #[test]
    fn fragmented_inline_runs_use_unified_reader_hit_testing() {
        assert!(needs_unified_reader_hit_test(
            "<p>before <MARK>selected</MARK> after</p>"
        ));
        assert!(needs_unified_reader_hit_test(
            "<p>before <em>italic</em> after</p>"
        ));
        assert!(needs_unified_reader_hit_test(
            "<p>before <ruby t='word'>vocabulary</ruby> after</p>"
        ));
        assert!(needs_unified_reader_hit_test(
            "<p>before <a href='chapter.xhtml'>link</a> after</p>"
        ));
        assert!(!needs_unified_reader_hit_test("<p>ordinary text</p>"));
        assert!(!needs_unified_reader_hit_test(
            "<p>before <image>name</image> after</p>"
        ));
        assert!(!needs_unified_reader_hit_test(
            "<p><em>caption</em><img src='figure.png'/></p>"
        ));
    }

    #[test]
    fn cross_block_selection_projects_to_visible_text_coordinates() {
        let range = ((2, 4), (4, 7));
        assert_eq!(selection_range_in_block(range, 1, 20), None);
        assert_eq!(selection_range_in_block(range, 2, 20), Some((4, 20)));
        assert_eq!(selection_range_in_block(range, 3, 12), Some((0, 12)));
        assert_eq!(selection_range_in_block(range, 4, 20), Some((0, 7)));
        assert_eq!(selection_range_in_block(range, 5, 20), None);
    }

    #[test]
    fn adjacent_selected_rows_share_the_gap_boundary() {
        let first = visual_selection_row_bounds(None, 0.0, 10.0, Some(20.0), 2.0);
        let second = visual_selection_row_bounds(Some(10.0), 20.0, 30.0, None, 2.0);
        assert_eq!(first, (-2.0, 15.0));
        assert_eq!(second, (15.0, 32.0));
        assert_eq!(first.1, second.0);
    }
}

/// 全局数据库句柄：后台线程初始化后写入，各处只读使用。
static DB: OnceLock<readwise_core::Db> = OnceLock::new();
/// Last database counts successfully projected to the UI. Hidden mobile
/// subtrees can be instantiated only after a rotation, so their labels need a
/// process-lifetime source of truth instead of falling back to DSL defaults.
static DB_READY_COUNTS: LazyLock<RwLock<Option<(usize, usize)>>> =
    LazyLock::new(|| RwLock::new(None));
static APP_STARTED_AT: OnceLock<Instant> = OnceLock::new();
static INITIAL_LIBRARY_READY_LOGGED: AtomicBool = AtomicBool::new(false);
/// Process-lifetime navigation state. Android can rebuild Makepad's App while
/// keeping the native process alive during an OEM configuration transition.
static ACTIVE_PAGE: AtomicUsize = AtomicUsize::new(0);

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
    fn compact_layout_keeps_mobile_phones_compact_in_both_orientations() {
        assert!(uses_compact_layout_for_platform(dvec2(390.0, 844.0), true));
        assert!(uses_compact_layout_for_platform(dvec2(844.0, 390.0), true));
        assert!(uses_compact_layout_for_platform(dvec2(719.0, 900.0), true));
        assert!(!uses_compact_layout_for_platform(
            dvec2(768.0, 1024.0),
            true
        ));
        assert!(!uses_compact_layout_for_platform(
            dvec2(1133.0, 744.0),
            true
        ));
        assert!(!uses_compact_layout_for_platform(
            dvec2(844.0, 390.0),
            false
        ));
        assert!(!uses_compact_layout_for_platform(dvec2(0.0, 0.0), true));
    }

    #[test]
    fn mobile_rotation_is_distinct_from_same_orientation_resize() {
        assert!(screen_orientation_changed(
            dvec2(390.0, 844.0),
            dvec2(844.0, 390.0)
        ));
        assert!(!screen_orientation_changed(
            dvec2(390.0, 844.0),
            dvec2(430.0, 932.0)
        ));
        assert!(!screen_orientation_changed(
            dvec2(0.0, 0.0),
            dvec2(844.0, 390.0)
        ));
    }

    #[test]
    fn library_portal_list_stays_mounted_behind_the_empty_state() {
        assert_eq!(library_surface_visibility(0), (true, true));
        assert_eq!(library_surface_visibility(3), (false, true));
    }

    #[test]
    fn mobile_navigation_stays_hidden_while_the_virtual_keyboard_is_open() {
        assert!(should_show_mobile_nav(true, false));
        assert!(!should_show_mobile_nav(true, true));
        assert!(!should_show_mobile_nav(false, false));
        assert!(!should_show_mobile_nav(false, true));
    }

    #[test]
    fn same_process_rebuild_restores_navigation_without_showing_sample_reader() {
        assert_eq!(
            restored_page_after_rebuild(AppPage::Reader.persisted(), true),
            AppPage::Reader
        );
        assert_eq!(
            restored_page_after_rebuild(AppPage::Reader.persisted(), false),
            AppPage::Library
        );
        assert_eq!(
            restored_page_after_rebuild(AppPage::Vocab.persisted(), false),
            AppPage::Vocab
        );
        assert_eq!(
            restored_page_after_rebuild(usize::MAX, true),
            AppPage::Library
        );
    }

    #[test]
    fn long_press_selects_a_word_or_one_cjk_character() {
        assert_eq!(word_range_at("read wise", 2), Some((0, 4)));
        assert_eq!(word_range_at("can't stop", 3), Some((0, 5)));
        assert_eq!(word_range_at("阅读", 3), Some((3, 6)));
        assert_eq!(word_range_at("A—can’t", 7), Some((4, 11)));
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
    want_to_read: bool,
    tags: Vec<String>,
    cover_path: Option<String>,
    file_path: String,
}

const WANT_TO_READ_FILTER: &str = "\0readwise:want-to-read";

fn shelf_items_for_tag(items: &[ShelfItem], selected: Option<&str>) -> Vec<ShelfItem> {
    items
        .iter()
        .filter(|book| {
            selected.is_none_or(|selected| {
                if selected == WANT_TO_READ_FILTER {
                    return book.want_to_read;
                }
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
            want_to_read: false,
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            cover_path: None,
            file_path: String::new(),
        }
    }

    #[test]
    fn shelf_tag_filter_is_case_insensitive_and_allows_empty_results() {
        let mut wanted = book("b", &["Reference"]);
        wanted.want_to_read = true;
        let items = vec![book("a", &["Fiction"]), wanted];
        assert_eq!(shelf_items_for_tag(&items, None).len(), 2);
        assert_eq!(shelf_items_for_tag(&items, Some("fiction"))[0].id, "a");
        assert!(shelf_items_for_tag(&items, Some("missing")).is_empty());
        assert_eq!(
            shelf_items_for_tag(&items, Some(WANT_TO_READ_FILTER))[0].id,
            "b"
        );
    }

    #[test]
    fn shelf_grid_uses_responsive_columns_and_complete_rows() {
        assert_eq!(shelf_grid_columns_for_width(390.0), 2);
        assert_eq!(shelf_grid_columns_for_width(900.0), 3);
        assert_eq!(shelf_grid_columns_for_width(1200.0), 4);
        assert_eq!(shelf_grid_row_count(0, 3), 0);
        assert_eq!(shelf_grid_row_count(7, 3), 3);
        assert_eq!(shelf_grid_book_index(2, 0, 3, 7), Some(6));
        assert_eq!(shelf_grid_book_index(2, 1, 3, 7), None);
    }

    #[test]
    fn filtered_shelf_projection_has_no_entries_after_the_matching_books() {
        let all = vec![
            book("rust-1", &["Rust"]),
            book("other-1", &["Design"]),
            book("rust-2", &["rust"]),
            book("other-2", &["Fiction"]),
            book("other-3", &["History"]),
        ];
        let mut projection = ListProjection::default();
        projection.replace(shelf_items_for_tag(&all, Some("Rust")));

        assert_eq!(projection.entries.len(), 2);
        assert_eq!(
            projection.entry_for_item_id(0).map(|book| &*book.id),
            Some("rust-1")
        );
        assert_eq!(
            projection.entry_for_item_id(1).map(|book| &*book.id),
            Some("rust-2")
        );
        for stale_item_id in 2..5 {
            assert!(projection.entry_for_item_id(stale_item_id).is_none());
        }
    }

    #[test]
    fn cover_cache_is_scoped_to_the_portal_card_instance() {
        let cover = Some("/covers/rust.png".to_string());
        let first_card = WidgetUid(7);
        let replacement_card = WidgetUid(8);
        let mut loaded = HashMap::new();
        loaded.insert(first_card, cover.clone());

        assert!(!shelf_cover_needs_load(&loaded, first_card, &cover));
        assert!(shelf_cover_needs_load(&loaded, replacement_card, &cover));
        assert!(shelf_cover_needs_load(
            &loaded,
            first_card,
            &Some("/covers/other.png".to_string())
        ));
    }
}

#[cfg(test)]
mod reader_pagination_tests {
    use super::*;

    #[test]
    fn pagination_maps_chapters_to_one_global_page_sequence() {
        let counts = [10, 5, 13];
        assert_eq!(
            reader_page_summary(&counts, 1, 3, 4),
            Some(ReaderPageSummary {
                current: 4,
                total: 9,
                pages_left_in_chapter: 1,
            })
        );
        assert_eq!(
            reader_page_summary(&counts, 2, 12, 4),
            Some(ReaderPageSummary {
                current: 9,
                total: 9,
                pages_left_in_chapter: 0,
            })
        );
    }

    #[test]
    fn pagination_waits_until_every_chapter_count_is_known() {
        assert_eq!(reader_page_summary(&[10, 0, 8], 0, 0, 4), None);
        assert_eq!(reader_page_summary(&[], 0, 0, 4), None);
    }

    #[test]
    fn previous_chapter_opens_on_its_last_page_boundary() {
        assert_eq!(reader_last_page_start(13, 4), 12);
        assert_eq!(reader_last_page_start(12, 4), 8);
        assert_eq!(reader_last_page_start(0, 4), 0);
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
        // 只处理已经访问并缓存的章节块；未访问章节会从
        // known_words 直接跳过该词。
        if let Some(book) = CURRENT_BOOK.write().unwrap().as_mut() {
            book.mark_word_known(&lemma);
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

/// 当前 Makepad 明暗偏好。“跟随主题”背景直接使用它，其余阅读背景会选择
/// 对应的场景主题，并保留相同的控件明暗语义。
static READER_THEME: LazyLock<RwLock<ReaderTheme>> =
    LazyLock::new(|| RwLock::new(ReaderTheme::Dark));

/// 书库投影方式。作为本地偏好与其他界面设置一起保存，并在旋转重建后恢复。
static LIBRARY_VIEW_MODE: LazyLock<RwLock<LibraryViewMode>> =
    LazyLock::new(|| RwLock::new(LibraryViewMode::List));

/// 全应用场景背景：正文、导航、窗口标题栏和其余功能页共享同一纸张配色。
static READER_BACKGROUND: LazyLock<RwLock<ReaderBackground>> =
    LazyLock::new(|| RwLock::new(ReaderBackground::Paper));

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

fn reader_palette(background: ReaderBackground, theme: ReaderTheme) -> (Vec4f, Vec4f) {
    let (surface, text) = match background {
        ReaderBackground::FollowTheme if theme == ReaderTheme::Light => (0xd9d9_d9ff, 0x3333_33ff),
        ReaderBackground::FollowTheme => (0x4d4d_4dff, 0xd9d9_d9ff),
        ReaderBackground::Paper => (0xf7f4_eaff, 0x2b29_25ff),
        ReaderBackground::Sepia => (0xf2e3_c6ff, 0x3b30_25ff),
        ReaderBackground::Sage => (0xe7ef_e3ff, 0x2734_2bff),
        ReaderBackground::Night => (0x2025_2bff, 0xd8d4_caff),
    };
    (Vec4f::from_u32(surface), Vec4f::from_u32(text))
}

#[derive(Clone, Copy)]
struct ReaderScenePalette {
    surface: Vec4f,
    text: Vec4f,
    field: Vec4f,
    border: Vec4f,
    muted: Vec4f,
    accent: Vec4f,
    hover: Vec4f,
    pressed: Vec4f,
}

fn reader_scene_palette(background: ReaderBackground, theme: ReaderTheme) -> ReaderScenePalette {
    let (surface, text) = reader_palette(background, theme);
    let values = match background {
        ReaderBackground::Paper => (
            0xe9e4_d7ff,
            0xd5cd_bdff,
            0x716a_60ff,
            0x805d_35ff,
            0x2b29_2514,
            0x2b29_2528,
        ),
        ReaderBackground::Sepia => (
            0xe2cf_acff,
            0xccb9_95ff,
            0x7562_4eff,
            0x8556_2fff,
            0x3b30_2514,
            0x3b30_2528,
        ),
        ReaderBackground::Sage => (
            0xd3df_cfff,
            0xbecd_b8ff,
            0x6070_65ff,
            0x3e68_50ff,
            0x2734_2b14,
            0x2734_2b28,
        ),
        ReaderBackground::Night => (
            0x343b_43ff,
            0x4650_5aff,
            0x9a9b_99ff,
            0xb7c9_a6ff,
            0xffffff14,
            0xffffff28,
        ),
        ReaderBackground::FollowTheme if theme == ReaderTheme::Light => (
            0xc7c7_c7ff,
            0xb9b9_b9ff,
            0x6868_68ff,
            0x365f_8dff,
            0x3333_3314,
            0x3333_3328,
        ),
        ReaderBackground::FollowTheme => (
            0x5959_59ff,
            0x6868_68ff,
            0xaaaa_aaff,
            0xb8c8_dcff,
            0xffffff14,
            0xffffff28,
        ),
    };
    ReaderScenePalette {
        surface,
        text,
        field: Vec4f::from_u32(values.0),
        border: Vec4f::from_u32(values.1),
        muted: Vec4f::from_u32(values.2),
        accent: Vec4f::from_u32(values.3),
        hover: Vec4f::from_u32(values.4),
        pressed: Vec4f::from_u32(values.5),
    }
}

fn navigation_palette_for_page(
    _page: AppPage,
    background: ReaderBackground,
    theme: ReaderTheme,
) -> ReaderScenePalette {
    reader_scene_palette(background, theme)
}

fn apply_reader_button_palette(cx: &mut Cx, mut widget: WidgetRef, palette: ReaderScenePalette) {
    let text = palette.text;
    let field = palette.field;
    let border = palette.border;
    let hover = palette.hover;
    let pressed = palette.pressed;
    script_apply_eval!(cx, widget, {
        draw_text +: {
            color: #(text)
            color_hover: #(text)
            color_down: #(text)
            color_focus: #(text)
        }
        draw_bg +: {
            color: #(field)
            color_hover: #(hover)
            color_down: #(pressed)
            color_focus: #(hover)
            border_color: #(border)
            border_color_hover: #(border)
            border_color_down: #(border)
            border_color_focus: #(border)
        }
    });
}

fn apply_reader_input_palette(cx: &mut Cx, mut widget: WidgetRef, palette: ReaderScenePalette) {
    let text = palette.text;
    let muted = palette.muted;
    let field = palette.field;
    let surface = palette.surface;
    let border = palette.border;
    script_apply_eval!(cx, widget, {
        draw_bg +: {
            color: #(field)
            color_hover: #(field)
            color_focus: #(surface)
            color_down: #(field)
            color_empty: #(field)
            border_color: #(border)
            border_color_hover: #(border)
            border_color_focus: #(border)
            border_color_down: #(border)
            border_color_empty: #(border)
        }
        draw_text +: {
            color: #(text)
            color_hover: #(text)
            color_focus: #(text)
            color_down: #(text)
            color_empty: #(muted)
            color_empty_hover: #(muted)
            color_empty_focus: #(muted)
        }
    });
}

fn apply_reader_surface_palette(cx: &mut Cx, mut widget: WidgetRef, color: Vec4f, border: Vec4f) {
    script_apply_eval!(cx, widget, {
        draw_bg +: {
            color: #(color)
            border_color: #(border)
        }
    });
}

fn apply_reader_label_palette(cx: &mut Cx, mut widget: WidgetRef, color: Vec4f) {
    script_apply_eval!(cx, widget, {
        draw_text.color: #(color)
    });
}

#[cfg(test)]
mod reader_scene_palette_tests {
    use super::*;

    #[test]
    fn scene_surface_and_text_share_the_canonical_reader_palette() {
        for theme in [ReaderTheme::Dark, ReaderTheme::Light] {
            for background in [
                ReaderBackground::FollowTheme,
                ReaderBackground::Paper,
                ReaderBackground::Sepia,
                ReaderBackground::Sage,
                ReaderBackground::Night,
            ] {
                let (surface, text) = reader_palette(background, theme);
                let scene = reader_scene_palette(background, theme);
                assert_eq!(scene.surface, surface);
                assert_eq!(scene.text, text);
            }
        }
    }

    #[test]
    fn floating_field_surface_is_distinct_from_reader_page() {
        for theme in [ReaderTheme::Dark, ReaderTheme::Light] {
            for background in [
                ReaderBackground::FollowTheme,
                ReaderBackground::Paper,
                ReaderBackground::Sepia,
                ReaderBackground::Sage,
                ReaderBackground::Night,
            ] {
                let scene = reader_scene_palette(background, theme);
                assert_ne!(scene.field, scene.surface);
                assert_ne!(scene.border, scene.surface);
            }
        }
    }

    #[test]
    fn every_page_uses_the_same_scene_palette() {
        let expected = reader_palette(ReaderBackground::Sepia, ReaderTheme::Dark).0;
        for page in [
            AppPage::Library,
            AppPage::Reader,
            AppPage::Chat,
            AppPage::Skills,
            AppPage::Vocab,
            AppPage::Statistics,
            AppPage::Settings,
        ] {
            assert_eq!(
                navigation_palette_for_page(page, ReaderBackground::Sepia, ReaderTheme::Dark)
                    .surface,
                expected
            );
        }
    }
}

static READER_SETTINGS_SAVE_VERSION: AtomicU64 = AtomicU64::new(0);
static READER_SETTINGS_SAVE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static READER_PROGRESS_SAVE_VERSION: AtomicU64 = AtomicU64::new(0);
static READER_PROGRESS_SAVE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

const READER_FONT_DEFAULT: f32 = 11.0;
const READER_FONT_MIN: f32 = 8.0;
const READER_FONT_MAX: f32 = 20.0;

struct OpenBook {
    id: String,
    title: String,
    /// 各章原始 body HTML（已消毒，不含运行时注音）。
    chapters: Vec<String>,
    /// 按访问惰性生成、已切块的注音章节；关闭注音时仍从原始章节投影。
    /// 直接缓存 PortalList 所需的块，避免每次设置刷新或旋转恢复都重新 split_blocks。
    glossed_chapter_chunks: Vec<Option<Vec<String>>>,
    glosses: HashMap<String, String>,
    known_words: HashSet<String>,
    /// 各章标题（TOC 用）。
    chapter_titles: Vec<String>,
    /// 各章高亮（章下标 → 区间列表）。
    highlights: HashMap<usize, Vec<readwise_core::books::Highlight>>,
}

impl OpenBook {
    fn chapter_chunks(&mut self, idx: usize, gloss_enabled: bool) -> Option<Vec<String>> {
        let raw = self.chapters.get(idx)?;
        if !gloss_enabled || self.glosses.is_empty() {
            return Some(readwise_core::books::epub_meta::split_blocks(raw));
        }
        if self.glossed_chapter_chunks.get(idx)?.is_none() {
            use readwise_core::vocab::annotate::{annotate_html, flatten_lists};
            let flat = flatten_lists(raw);
            let glossed = annotate_html(&flat, &self.glosses, &self.known_words);
            self.glossed_chapter_chunks[idx] =
                Some(readwise_core::books::epub_meta::split_blocks(&glossed));
        }
        self.glossed_chapter_chunks[idx].clone()
    }

    fn mark_word_known(&mut self, lemma: &str) {
        self.known_words.insert(lemma.to_string());
        for chunks in self.glossed_chapter_chunks.iter_mut().flatten() {
            for chunk in chunks {
                if chunk.contains("<ruby") {
                    *chunk = readwise_core::vocab::annotate::remove_ruby_for_lemma(chunk, lemma);
                }
            }
        }
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
            glossed_chapter_chunks: vec![None, None],
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
        let second = book.chapter_chunks(1, true).unwrap().join("");
        assert!(second.contains("<ruby>murmur"));
        assert!(book.glossed_chapter_chunks[0].is_none());
        assert!(book.glossed_chapter_chunks[1].is_some());

        let raw_first = book.chapter_chunks(0, false).unwrap().join("");
        assert_eq!(raw_first, "<p>The solitude.</p>");
        assert!(book.glossed_chapter_chunks[0].is_none());
    }

    #[test]
    fn newly_known_words_are_skipped_by_uncached_chapters() {
        let mut book = book();
        book.mark_word_known("solitude");
        let first = book.chapter_chunks(0, true).unwrap().join("");
        assert!(!first.contains("<ruby>solitude"));
    }

    #[test]
    fn newly_known_words_are_removed_from_cached_chapter_chunks() {
        let mut book = book();
        assert!(book
            .chapter_chunks(0, true)
            .unwrap()
            .join("")
            .contains("<ruby>solitude"));

        book.mark_word_known("solitude");

        assert!(!book
            .chapter_chunks(0, true)
            .unwrap()
            .join("")
            .contains("<ruby>solitude"));
    }

    #[test]
    fn cached_glossed_chunks_are_reused_without_exposing_mutable_cache_state() {
        let mut book = book();
        let mut first = book.chapter_chunks(0, true).unwrap();
        first[0].clear();

        let second = book.chapter_chunks(0, true).unwrap();
        assert!(second[0].contains("<ruby>solitude"));
        assert!(book.glossed_chapter_chunks[0].is_some());
    }

    #[test]
    fn long_glossed_chapter_reuses_its_portal_block_projection() {
        let mut book = book();
        book.chapters[0] = (0..512)
            .map(|idx| format!("<p>Paragraph {idx}: solitude.</p>"))
            .collect();

        let first = book.chapter_chunks(0, true).unwrap();
        assert_eq!(first.len(), 512);
        book.chapters[0] = "<p>Replacement content.</p>".to_string();

        let second = book.chapter_chunks(0, true).unwrap();
        assert_eq!(second, first);
    }
}

/// 当前选区（块 + TextFlow UTF-8 字节区间，ChapterList 写入，App 读）。
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
    Show { anchor: Option<DVec2> },
    Hide,
}

const SELECTION_POPUP_MAX_WIDTH: f64 = 520.0;
const SELECTION_POPUP_SIDE_INSET: f64 = 12.0;
const SELECTION_POPUP_GAP: f64 = 10.0;
const SELECTION_POPUP_ESTIMATED_HEIGHT: f64 = 56.0;
const WORD_CARD_MAX_WIDTH: f64 = 420.0;
const WORD_CARD_SIDE_INSET: f64 = 12.0;
const WORD_CARD_GAP: f64 = 10.0;
const WORD_CARD_ESTIMATED_HEIGHT: f64 = 116.0;
const LIBRARY_ACTION_MENU_MAX_WIDTH: f64 = 238.0;
const LIBRARY_ACTION_MENU_ESTIMATED_HEIGHT: f64 = 238.0;

fn selection_popup_geometry(viewport: Rect, anchor: DVec2, popup_height: f64) -> (DVec2, f64) {
    let horizontal_inset = SELECTION_POPUP_SIDE_INSET.min(viewport.size.x.max(0.0) * 0.25);
    let vertical_inset = SELECTION_POPUP_SIDE_INSET.min(viewport.size.y.max(0.0) * 0.25);
    let popup_width =
        SELECTION_POPUP_MAX_WIDTH.min((viewport.size.x - horizontal_inset * 2.0).max(1.0));
    let min_x = viewport.pos.x + horizontal_inset;
    let max_x = (viewport.pos.x + viewport.size.x - horizontal_inset - popup_width).max(min_x);
    let x = (anchor.x - popup_width * 0.5).clamp(min_x, max_x);

    let popup_height = popup_height.max(1.0);
    let min_y = viewport.pos.y + vertical_inset;
    let max_y = (viewport.pos.y + viewport.size.y - vertical_inset - popup_height).max(min_y);
    let above = anchor.y - popup_height - SELECTION_POPUP_GAP;
    let below = anchor.y + SELECTION_POPUP_GAP;
    let y = if above >= min_y {
        above
    } else if below <= max_y {
        below
    } else {
        above.clamp(min_y, max_y)
    };
    (dvec2(x, y), popup_width)
}

fn word_card_geometry(viewport: Rect, anchor: DVec2, card_height: f64) -> (DVec2, f64) {
    let horizontal_inset = WORD_CARD_SIDE_INSET.min(viewport.size.x.max(0.0) * 0.25);
    let vertical_inset = WORD_CARD_SIDE_INSET.min(viewport.size.y.max(0.0) * 0.25);
    let card_width = WORD_CARD_MAX_WIDTH.min((viewport.size.x - horizontal_inset * 2.0).max(1.0));
    let min_x = viewport.pos.x + horizontal_inset;
    let max_x = (viewport.pos.x + viewport.size.x - horizontal_inset - card_width).max(min_x);
    let x = (anchor.x - card_width * 0.5).clamp(min_x, max_x);

    let card_height = card_height.max(1.0);
    let min_y = viewport.pos.y + vertical_inset;
    let max_y = (viewport.pos.y + viewport.size.y - vertical_inset - card_height).max(min_y);
    let above = anchor.y - card_height - WORD_CARD_GAP;
    let below = anchor.y + WORD_CARD_GAP;
    let y = if above >= min_y {
        above
    } else if below <= max_y {
        below
    } else {
        above.clamp(min_y, max_y)
    };
    (dvec2(x, y), card_width)
}

fn library_action_menu_geometry(viewport: Rect, anchor: Rect, menu_height: f64) -> (DVec2, f64) {
    let inset = 8.0_f64.min(viewport.size.x.max(0.0) * 0.2);
    let width = LIBRARY_ACTION_MENU_MAX_WIDTH.min((viewport.size.x - inset * 2.0).max(1.0));
    let min_x = viewport.pos.x + inset;
    let max_x = (viewport.pos.x + viewport.size.x - inset - width).max(min_x);
    let x = (anchor.pos.x + anchor.size.x - width).clamp(min_x, max_x);
    let min_y = viewport.pos.y + inset;
    let max_y = (viewport.pos.y + viewport.size.y - inset - menu_height).max(min_y);
    let below = anchor.pos.y + anchor.size.y + 6.0;
    let above = anchor.pos.y - menu_height - 6.0;
    let y = if below <= max_y {
        below
    } else if above >= min_y {
        above
    } else {
        below.clamp(min_y, max_y)
    };
    (dvec2(x, y), width)
}

fn library_card_release_opens_reader(menu_was_open: bool, release_is_over: bool) -> bool {
    !menu_was_open && release_is_over
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
    Deleted { chapter: usize, ids: Vec<String> },
    Error(String),
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
            Err(e) => Cx::post_action(HlAction::Error(e)),
        }
    });
}

fn spawn_delete_highlights(book_id: String, chapter: usize, ids: Vec<String>) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        match db.delete_highlights(&book_id, &ids) {
            Ok(_) => Cx::post_action(HlAction::Deleted { chapter, ids }),
            Err(error) => Cx::post_action(HlAction::Error(error)),
        }
    });
}

/// 当前章下标（App 写、TocList 读，做高亮）。
static CURRENT_CHAPTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// First visible paragraph, used only to restore an Android same-process App rebuild.
static CURRENT_READER_BLOCK: AtomicUsize = AtomicUsize::new(0);
/// Exact offset inside the first visible paragraph, stored as `f64::to_bits()`.
static CURRENT_READER_SCROLL_BITS: AtomicU64 = AtomicU64::new(0.0_f64.to_bits());

/// 打开书的结果（后台 → UI）。
#[derive(Debug)]
enum ReaderAction {
    Opened {
        resume: ReaderLocation,
        gloss_count: usize,
        parse_ms: u128,
        initial_gloss_ms: u128,
    },
    OpenError(String),
}

#[derive(Debug)]
enum ReaderPaginationAction {
    Ready {
        book_id: String,
        chapter_block_counts: Vec<usize>,
    },
}

/// 全书分块统计移到后台，避免为了总页数阻塞打开首章。每次只克隆一个章节，
/// 不额外复制整本 EPUB；切书后旧任务会通过 book_id 自行退出。
fn spawn_build_reader_pagination(book_id: String) {
    std::thread::spawn(move || {
        let chapter_count = {
            let guard = CURRENT_BOOK.read().unwrap();
            let Some(book) = guard.as_ref().filter(|book| book.id == book_id) else {
                return;
            };
            book.chapters.len()
        };
        let mut chapter_block_counts = Vec::with_capacity(chapter_count);
        for chapter in 0..chapter_count {
            let html = {
                let guard = CURRENT_BOOK.read().unwrap();
                let Some(book) = guard.as_ref().filter(|book| book.id == book_id) else {
                    return;
                };
                let Some(html) = book.chapters.get(chapter) else {
                    return;
                };
                html.clone()
            };
            chapter_block_counts.push(
                readwise_core::books::epub_meta::split_blocks(&html)
                    .len()
                    .max(1),
            );
        }
        Cx::post_action(ReaderPaginationAction::Ready {
            book_id,
            chapter_block_counts,
        });
    });
}

/// 持久化的阅读位置。旧版本只保存章节号；v2 额外保存首个可见段落及其滚动偏移。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ReaderLocation {
    chapter: usize,
    block: usize,
    scroll: f64,
}

impl ReaderLocation {
    fn encode(self) -> String {
        let scroll = if self.scroll.is_finite() {
            self.scroll
        } else {
            0.0
        };
        format!("v2:{}:{}:{scroll}", self.chapter, self.block)
    }

    fn decode(value: &str, chapter_count: usize) -> Self {
        let legacy = || {
            value
                .parse::<usize>()
                .ok()
                .filter(|chapter| *chapter < chapter_count)
                .map(|chapter| Self {
                    chapter,
                    ..Self::default()
                })
                .unwrap_or_default()
        };
        let Some(encoded) = value.strip_prefix("v2:") else {
            return legacy();
        };
        let mut fields = encoded.split(':');
        let Some(chapter) = fields.next().and_then(|field| field.parse::<usize>().ok()) else {
            return legacy();
        };
        let Some(block) = fields.next().and_then(|field| field.parse::<usize>().ok()) else {
            return legacy();
        };
        let Some(scroll) = fields.next().and_then(|field| field.parse::<f64>().ok()) else {
            return legacy();
        };
        if fields.next().is_some() || chapter >= chapter_count || !scroll.is_finite() {
            return legacy();
        }
        Self {
            chapter,
            block,
            scroll,
        }
    }
}

#[cfg(test)]
mod reader_location_tests {
    use super::*;

    #[test]
    fn reader_location_round_trips_chapter_block_and_scroll() {
        let location = ReaderLocation {
            chapter: 3,
            block: 27,
            scroll: -18.75,
        };
        assert_eq!(ReaderLocation::decode(&location.encode(), 8), location);
    }

    #[test]
    fn reader_location_accepts_legacy_chapters_and_rejects_invalid_values() {
        assert_eq!(
            ReaderLocation::decode("4", 8),
            ReaderLocation {
                chapter: 4,
                ..ReaderLocation::default()
            }
        );
        assert_eq!(ReaderLocation::decode("9", 8), ReaderLocation::default());
        assert_eq!(
            ReaderLocation::decode("v2:2:10:NaN", 8),
            ReaderLocation::default()
        );
        assert_eq!(
            ReaderLocation::decode("v2:20:10:-1", 8),
            ReaderLocation::default()
        );
    }
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
            .map(|status| ReaderLocation::decode(&status.location, total))
            .unwrap_or_default();
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
            glossed_chapter_chunks: vec![None; chapters.len()],
            chapters,
            chapter_titles,
            glosses,
            known_words,
            highlights,
        };
        let gloss_started = std::time::Instant::now();
        if *GLOSS_ENABLED.read().unwrap() {
            let _ = book.chapter_chunks(resume.chapter, true);
        }
        let initial_gloss_ms = gloss_started.elapsed().as_millis();
        *CURRENT_BOOK.write().unwrap() = Some(book);
        Cx::post_action(ReaderAction::Opened {
            resume,
            gloss_count,
            parse_ms,
            initial_gloss_ms,
        });
    });
}

/// 章节 → PortalList 块。永久高亮只保留数据库坐标，由 ChapterList 在正文
/// 排版完成后覆盖绘制，绝不向 HTML 注入 `<mark>` 或改变行内 run。
fn build_chapter_chunks(book: &mut OpenBook, idx: usize) -> Option<Vec<String>> {
    book.chapter_chunks(idx, *GLOSS_ENABLED.read().unwrap())
}

fn overlapping_highlight_ids(
    highlights: &[readwise_core::books::Highlight],
    range: TextRange,
) -> Vec<String> {
    let (selection_start, selection_end) = range;
    highlights
        .iter()
        .filter(|highlight| {
            let highlight_start = (highlight.start_block, highlight.start_char);
            let highlight_end = (highlight.end_block, highlight.end_char);
            highlight_start < selection_end && selection_start < highlight_end
        })
        .map(|highlight| highlight.id.clone())
        .collect()
}

#[cfg(test)]
mod highlight_overlap_tests {
    use super::*;

    fn highlight(id: &str, start: TextPoint, end: TextPoint) -> readwise_core::books::Highlight {
        readwise_core::books::Highlight {
            id: id.to_string(),
            start_block: start.0,
            start_char: start.1,
            end_block: end.0,
            end_char: end.1,
            color: "yellow".to_string(),
        }
    }

    #[test]
    fn selection_deletes_every_intersecting_highlight_but_not_touching_neighbors() {
        let highlights = vec![
            highlight("before", (2, 0), (2, 4)),
            highlight("first", (2, 4), (2, 12)),
            highlight("cross", (2, 10), (3, 8)),
            highlight("after", (3, 8), (3, 20)),
        ];
        assert_eq!(
            overlapping_highlight_ids(&highlights, ((2, 6), (3, 8))),
            vec!["first".to_string(), "cross".to_string()]
        );
        assert!(overlapping_highlight_ids(&highlights, ((4, 0), (4, 3))).is_empty());
    }

    #[test]
    fn highlight_projects_across_blocks_without_exceeding_visible_text() {
        let highlight = highlight("cross", (2, 4), (4, 18));
        assert_eq!(highlight_range_in_block(&highlight, 1, 20), None);
        assert_eq!(highlight_range_in_block(&highlight, 2, 20), Some((4, 20)));
        assert_eq!(highlight_range_in_block(&highlight, 3, 12), Some((0, 12)));
        assert_eq!(highlight_range_in_block(&highlight, 4, 10), Some((0, 10)));
        assert_eq!(highlight_range_in_block(&highlight, 5, 20), None);
    }
}

#[cfg(test)]
mod selection_popup_tests {
    use super::*;

    #[test]
    fn popup_prefers_above_selection_and_stays_inside_viewport() {
        let viewport = Rect {
            pos: dvec2(100.0, 80.0),
            size: dvec2(800.0, 600.0),
        };
        let (position, width) = selection_popup_geometry(viewport, dvec2(500.0, 400.0), 76.0);
        assert_eq!(width, 520.0);
        assert_eq!(position, dvec2(240.0, 314.0));
    }

    #[test]
    fn popup_flips_below_and_shrinks_for_a_narrow_viewport() {
        let viewport = Rect {
            pos: dvec2(20.0, 40.0),
            size: dvec2(320.0, 500.0),
        };
        let (position, width) = selection_popup_geometry(viewport, dvec2(28.0, 55.0), 76.0);
        assert_eq!(width, 296.0);
        assert_eq!(position, dvec2(32.0, 65.0));
    }

    #[test]
    fn popup_clamps_to_bottom_when_neither_side_has_enough_space() {
        let viewport = Rect {
            pos: dvec2(0.0, 0.0),
            size: dvec2(260.0, 100.0),
        };
        let (position, _) = selection_popup_geometry(viewport, dvec2(130.0, 50.0), 90.0);
        assert_eq!(position.y, 12.0);
    }

    #[test]
    fn word_card_stays_near_the_clicked_word() {
        let viewport = Rect {
            pos: dvec2(100.0, 80.0),
            size: dvec2(800.0, 600.0),
        };
        let (position, width) = word_card_geometry(viewport, dvec2(500.0, 400.0), 116.0);
        assert_eq!(width, 420.0);
        assert_eq!(position, dvec2(290.0, 274.0));
    }

    #[test]
    fn word_card_flips_below_and_shrinks_on_mobile() {
        let viewport = Rect {
            pos: dvec2(20.0, 40.0),
            size: dvec2(320.0, 500.0),
        };
        let (position, width) = word_card_geometry(viewport, dvec2(28.0, 55.0), 116.0);
        assert_eq!(width, 296.0);
        assert_eq!(position, dvec2(32.0, 65.0));
    }

    #[test]
    fn library_menu_aligns_to_more_button_and_flips_above_near_bottom() {
        let viewport = Rect {
            pos: dvec2(100.0, 80.0),
            size: dvec2(700.0, 560.0),
        };
        let anchor = Rect {
            pos: dvec2(720.0, 590.0),
            size: dvec2(36.0, 30.0),
        };
        let (position, width) = library_action_menu_geometry(viewport, anchor, 238.0);
        assert_eq!(width, 238.0);
        assert_eq!(position, dvec2(518.0, 346.0));
    }

    #[test]
    fn library_menu_shrinks_and_stays_inside_narrow_viewport() {
        let viewport = Rect {
            pos: dvec2(20.0, 40.0),
            size: dvec2(220.0, 500.0),
        };
        let anchor = Rect {
            pos: dvec2(190.0, 100.0),
            size: dvec2(34.0, 30.0),
        };
        let (position, width) = library_action_menu_geometry(viewport, anchor, 238.0);
        assert_eq!(width, 204.0);
        assert_eq!(position, dvec2(28.0, 136.0));
    }

    #[test]
    fn an_open_library_menu_blocks_card_release_from_opening_the_reader() {
        assert!(!library_card_release_opens_reader(true, true));
        assert!(!library_card_release_opens_reader(false, false));
        assert!(library_card_release_opens_reader(false, true));
    }
}

#[derive(Clone, Debug)]
struct ReaderProgressSnapshot {
    book_id: String,
    chapter_count: usize,
    location: ReaderLocation,
}

fn write_reader_progress(snapshot: &ReaderProgressSnapshot) -> Result<(), String> {
    let Some(db) = DB.get() else { return Ok(()) };
    db.update_progress(
        &snapshot.book_id,
        (snapshot.location.chapter + 1) as i64,
        snapshot.chapter_count as i64,
        &snapshot.location.encode(),
    )
}

/// 串行、去旧地保存滚动位置，避免密集滚动产生的后台写入乱序覆盖新位置。
fn spawn_save_progress(snapshot: ReaderProgressSnapshot) {
    let version = READER_PROGRESS_SAVE_VERSION.fetch_add(1, Ordering::Relaxed) + 1;
    std::thread::spawn(move || {
        let Ok(_guard) = READER_PROGRESS_SAVE_LOCK.lock() else {
            ::log::error!("阅读进度保存队列已损坏");
            return;
        };
        if version != READER_PROGRESS_SAVE_VERSION.load(Ordering::Relaxed) {
            return;
        }
        if let Err(error) = write_reader_progress(&snapshot) {
            ::log::error!("保存阅读进度失败: {error}");
        }
    });
}

/// 进入后台、离开阅读页或退出时同步落盘；版本号会淘汰仍在等待的旧异步写入。
fn save_progress_now(snapshot: ReaderProgressSnapshot) {
    READER_PROGRESS_SAVE_VERSION.fetch_add(1, Ordering::Relaxed);
    let Ok(_guard) = READER_PROGRESS_SAVE_LOCK.lock() else {
        ::log::error!("阅读进度保存队列已损坏");
        return;
    };
    if let Err(error) = write_reader_progress(&snapshot) {
        ::log::error!("保存阅读进度失败: {error}");
    }
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
    Renamed {
        items: usize,
        book_id: String,
        title: String,
    },
    WantToReadChanged {
        items: usize,
        book_id: String,
        enabled: bool,
    },
    MarkedCompleted {
        items: usize,
        book_id: String,
        title: String,
    },
    Deleted {
        items: usize,
        title: String,
    },
    TagError(String),
    UpdateError(String),
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
            want_to_read: b.want_to_read,
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

fn spawn_rename_book(book_id: String, title: String) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else {
            Cx::post_action(ShelfAction::UpdateError("数据库尚未就绪".to_string()));
            return;
        };
        let title = match db.rename_book(&book_id, &title) {
            Ok(title) => title,
            Err(error) => {
                Cx::post_action(ShelfAction::UpdateError(error));
                return;
            }
        };
        match refresh_shelf(db) {
            Ok(items) => Cx::post_action(ShelfAction::Renamed {
                items,
                book_id,
                title,
            }),
            Err(error) => Cx::post_action(ShelfAction::UpdateError(error)),
        }
    });
}

fn spawn_set_book_want_to_read(book_id: String, enabled: bool) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else {
            Cx::post_action(ShelfAction::UpdateError("数据库尚未就绪".to_string()));
            return;
        };
        if let Err(error) = db.set_book_want_to_read(&book_id, enabled) {
            Cx::post_action(ShelfAction::UpdateError(error));
            return;
        }
        match refresh_shelf(db) {
            Ok(items) => Cx::post_action(ShelfAction::WantToReadChanged {
                items,
                book_id,
                enabled,
            }),
            Err(error) => Cx::post_action(ShelfAction::UpdateError(error)),
        }
    });
}

fn spawn_mark_book_completed(book_id: String, title: String) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else {
            Cx::post_action(ShelfAction::UpdateError("数据库尚未就绪".to_string()));
            return;
        };
        if let Err(error) = db.mark_book_completed(&book_id) {
            Cx::post_action(ShelfAction::UpdateError(error));
            return;
        }
        match refresh_shelf(db) {
            Ok(items) => Cx::post_action(ShelfAction::MarkedCompleted {
                items,
                book_id,
                title,
            }),
            Err(error) => Cx::post_action(ShelfAction::UpdateError(error)),
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
        if let Some(db) = DB.get() {
            Cx::post_action(db_ready_action(db));
            return;
        }
        let data_dir =
            readwise_core::paths::app_data_dir_from_platform(platform_data_dir.as_deref());
        match readwise_core::Db::open_in(data_dir) {
            Ok(db) => {
                let _ = DB.set(db);
                if let Some(db) = DB.get() {
                    Cx::post_action(db_ready_action(db));
                } else {
                    Cx::post_action(DbAction::Failed("数据库句柄初始化后不可用".to_string()));
                }
            }
            Err(e) => Cx::post_action(DbAction::Failed(e)),
        }
    });
}

fn db_ready_action(db: &readwise_core::Db) -> DbAction {
    let entries = db.vocab_book_list().unwrap_or_default();
    let known = db.get_known_words().unwrap_or_default();
    let reader_settings = db.load_reader_settings().unwrap_or_default();
    *DB_READY_COUNTS.write().unwrap() = Some((entries.len(), known.len()));
    DbAction::Ready {
        vocab_count: entries.len(),
        known_count: known.len(),
        reader_settings,
    }
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
        library_view_mode: *LIBRARY_VIEW_MODE.read().unwrap(),
        theme: *READER_THEME.read().unwrap(),
        background: *READER_BACKGROUND.read().unwrap(),
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
        width: Fill height: Fit
        new_batch: true
        // Child buttons and top-level overlays must win hit testing. Enabling
        // overload here makes the card deliberately click through both.
        capture_overload: false
        cursor: MouseCursor.Hand
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
                max_lines: 2
                text_overflow: Ellipsis
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
        }
        more_btn := ButtonFlatter{
            width: 40 height: 36 text: "•••"
            draw_text.text_style.font_size: 13
        }
    }

    mod.widgets.BookShelfGridCard = RoundedView{
        width: Fill height: 226
        new_batch: true
        capture_overload: false
        cursor: MouseCursor.Hand
        flow: Down spacing: 5 padding: 10
        align: Align{ x: 0.5 }
        draw_bg.color: theme.color_bg_container
        draw_bg.border_radius: 8.0
        cover := View{
            width: 86 height: 116
            flow: Overlay
            cover_ph := SolidView{
                width: Fill height: Fill
                new_batch: true
                align: Center
                draw_bg.color: theme.color_highlight
                draw_bg.border_radius: 5.0
                cover_letter := Label{
                    text: ""
                    draw_text.text_style.font_size: 28
                }
            }
            cover_img := Image{
                width: Fill height: Fill
                fit: ImageFit.Biggest
            }
        }
        grid_title_row := View{
            width: Fill
            height: Fit
            flow: Right spacing: 4
            align: Align{y: 0.5}
            card_title := Label{
                width: Fill
                max_lines: 2
                text_overflow: Ellipsis
                text: ""
                draw_text.text_style.font_size: 12
            }
            more_btn := ButtonFlatter{
                width: 34 height: 30 text: "•••"
                draw_text.text_style.font_size: 11
            }
        }
        card_author := Label{
            width: Fill
            max_lines: 1
            text_overflow: Ellipsis
            text: ""
            draw_text.color: theme.color_label_inner_inactive
            draw_text.text_style.font_size: 9
        }
        card_status := Label{
            width: Fill
            max_lines: 1
            text_overflow: Ellipsis
            text: ""
            draw_text.color: theme.color_highlight
            draw_text.text_style.font_size: 9
        }
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
            GridRow := View{
                width: Fill height: Fit
                flow: Right spacing: 12
                grid_cell_0 := View{
                    width: Fill height: 226
                    grid_card := mod.widgets.BookShelfGridCard{}
                }
                grid_cell_1 := View{
                    width: Fill height: 226
                    grid_card := mod.widgets.BookShelfGridCard{}
                }
                grid_cell_2 := View{
                    width: Fill height: 226
                    grid_card := mod.widgets.BookShelfGridCard{}
                }
                grid_cell_3 := View{
                    width: Fill height: 226
                    grid_card := mod.widgets.BookShelfGridCard{}
                }
            }
            GridRowAlternate := View{
                width: Fill height: Fit
                flow: Right spacing: 12
                grid_cell_0 := View{
                    width: Fill height: 226
                    grid_card := mod.widgets.BookShelfGridCard{}
                }
                grid_cell_1 := View{
                    width: Fill height: 226
                    grid_card := mod.widgets.BookShelfGridCard{}
                }
                grid_cell_2 := View{
                    width: Fill height: 226
                    grid_card := mod.widgets.BookShelfGridCard{}
                }
                grid_cell_3 := View{
                    width: Fill height: 226
                    grid_card := mod.widgets.BookShelfGridCard{}
                }
            }
        }
    }

    mod.widgets.TocListBase = #(TocList::register_widget(vm))
    mod.widgets.TocList = set_type_default() do mod.widgets.TocListBase{
        width: Fill height: Fill
        active_bg: theme.color_bg_highlight_inline
        inactive_bg: theme.color_bg_app
        text_color: theme.color_text
        button_hover: theme.color_bg_highlight
        button_pressed: theme.color_bg_highlight_inline
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
                    align: Align{x: 0.0 y: 0.5}
                    label_walk: Walk{width: Fill height: Fit}
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
            // History restoration must land at the tail immediately. A smooth
            // tail makes a freshly-opened thread visibly traverse every message.
            smooth_tail: false
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

    mod.widgets.ReaderPageIndicatorBase = #(ReaderPageIndicator::register_widget(vm))
    mod.widgets.ReaderPageIndicator = set_type_default() do mod.widgets.ReaderPageIndicatorBase{
        width: Fit height: Fit
        cursor: MouseCursor.Hand
        indicator_bg := RoundedView{
            width: Fit height: 25
            new_batch: true
            padding: Inset{left: 9 right: 9 top: 4 bottom: 4}
            draw_bg.color: #0000
            draw_bg.border_radius: 8.0
            page_text := Label{
                width: Fit height: Fit
                text: "—"
                draw_text.color: theme.color_label_inner_inactive
                draw_text.text_style.font_size: 9
            }
        }
    }

    let ReaderCodeView = mod.widgets.CodeView{
        editor +: {
            width: Fill
            height: Fit
            word_wrap: false
            show_gutter: false
            draw_bg +: {color: #0000}
            draw_text +: {
                text_style +: {font_size: 10.5 line_spacing: 1.35}
            }
        }
    }

    let ReaderCodeViewLight = ReaderCodeView{
        editor +: {
            token_colors +: {
                whitespace: #7b746b
                delimiter: #2b2925
                delimiter_highlight: #111
                unknown: #2b2925
                branch_keyword: #7b3f8c
                constant: #9a3f35
                identifier: #2b2925
                loop_keyword: #a34f00
                number: #3c6e34
                other_keyword: #295f9e
                punctuator: #2b2925
                string: #9a3f35
                function: #6f5600
                typename: #00736d
                comment: #4f7250
            }
        }
    }

    let ReaderCodePara = RoundedView{
        width: Fill height: Fit
        new_batch: true
        margin: Inset{top: 4 bottom: 4 left: 2 right: 2}
        padding: Inset{top: 10 bottom: 10 left: 12 right: 12}
        draw_bg +: {
            border_radius: 7.0
            border_size: 1.0
        }
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
        html_default_inset: theme.space_1
        draw_reader_selection.color: #0000
        list := PortalList{
            width: Fill height: Fill
            selectable: true
            // Desktop text selection owns pointer drags. Wheel/scrollbar
            // scrolling remains available; compact mode enables touch drag.
            drag_scrolling: false
            padding: Inset{ left: 32 right: 32 top: 8 bottom: 8 }
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
            TablePara := View{
                width: Fill height: Fit
                flow: Down
                align: HCenter
                margin: Inset{top: 8 bottom: 10}
                para_html := Html{
                    width: Fill{max: 960} height: Fit
                    selectable: true
                    a.color: theme.color_text
                    a.hover_color: theme.color_text_hover
                    a.pressed_color: theme.color_text_focus
                    img := mod.widgets.ChapterImage{}
                    mark := mod.widgets.MarkSpan{}
                }
            }
            TableGlossPara := View{
                width: Fill height: Fit
                flow: Down
                align: HCenter
                margin: Inset{top: 8 bottom: 10}
                para_html := Html{
                    width: Fill{max: 960} height: Fit
                    selectable: true
                    padding: Inset{top: 22}
                    text_style_normal.line_spacing: 2.2
                    text_style_bold.line_spacing: 2.2
                    text_style_italic.line_spacing: 2.2
                    text_style_fixed.line_spacing: 2.2
                    a.color: theme.color_text
                    a.hover_color: theme.color_text_hover
                    a.pressed_color: theme.color_text_focus
                    ruby := mod.widgets.RubyWord{}
                    img := mod.widgets.ChapterImage{}
                    mark := mod.widgets.MarkSpan{}
                }
            }
            CodePaper := ReaderCodePara{
                draw_bg +: {color: #f7f4ea border_color: #d8d1c2}
                code_view := ReaderCodeViewLight{}
            }
            CodeSepia := ReaderCodePara{
                draw_bg +: {color: #f2e3c6 border_color: #ccb995}
                code_view := ReaderCodeViewLight{}
            }
            CodeSage := ReaderCodePara{
                draw_bg +: {color: #e7efe3 border_color: #becdb8}
                code_view := ReaderCodeViewLight{}
            }
            CodeNight := ReaderCodePara{
                draw_bg +: {color: #20252b border_color: #46505a}
                code_view := ReaderCodeView{}
            }
            CodeThemeLight := ReaderCodePara{
                draw_bg +: {color: #d9d9d9 border_color: #b9b9b9}
                code_view := ReaderCodeViewLight{}
            }
            CodeThemeDark := ReaderCodePara{
                draw_bg +: {color: #4d4d4d border_color: #686868}
                code_view := ReaderCodeView{}
            }
        }
    }

    startup() do #(App::script_component(vm)){
        swatch_inactive: theme.color_bg_container
        ui: Root{
            main_window := Window{
                window.title: "ReadWise"
                window.inner_size: vec2(1080, 720)
                caption_bar +: {
                    new_batch: true
                }
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
                    main_column := SolidView{
                        width: Fill height: Fill
                        new_batch: true
                        flow: Down
                        draw_bg.color: theme.color_bg_app
                        mobile_safe_top := SolidView{
                            visible: false
                            width: Fill height: 0
                            draw_bg.color: theme.color_bg_app
                        }
                        pages := PageFlip{
                            width: Fill height: Fill
                            active_page: @page_library

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
                                    library_view_list := RadioButtonTabFlat{
                                        text: "列表"
                                    }
                                    library_view_grid := RadioButtonTabFlat{
                                        text: "网格"
                                    }
                                    import_epub_btn := ButtonFlat{
                                        // 平台能力在初始脚本对象中确定；不要依赖 Startup
                                        // 再把静态 false 改回来，ScriptReapply 会覆盖它。
                                        visible: #(cfg!(any(target_os = "android", target_os = "ios", target_os = "macos")))
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
                                    // The first real projection decides whether
                                    // this overlay is needed. Avoid flashing a
                                    // stale empty state after mobile rotation.
                                    visible: false
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
                                // Keep the PortalList mounted; it reads the
                                // centralized shelf projection in draw_walk.
                                book_shelf := mod.widgets.BookShelf{}
                                library_action_layer := View{
                                    // Keep the transparent overlay owner mounted so it
                                    // has a valid Area before the first menu request.
                                    // Only its interactive children are toggled.
                                    width: Fill height: Fill
                                    flow: Overlay
                                    library_action_dismiss := View{
                                        visible: false
                                        width: Fill height: Fill
                                        // View only emits pointer actions when it has a
                                        // cursor or animator. It stays behind the menu,
                                        // so only genuine outside clicks reach it.
                                        cursor: MouseCursor.Default
                                        capture_overload: false
                                    }
                                    library_action_menu := RoundedShadowView{
                                        visible: false
                                        width: 238 height: Fit
                                        new_batch: true
                                        flow: Down spacing: 2 padding: 8
                                        draw_bg.color: theme.color_bg_container
                                        draw_bg.border_color: theme.color_bg_highlight_inline
                                        draw_bg.border_radius: 10.0
                                        draw_bg.shadow_color: #0004
                                        draw_bg.shadow_radius: 12.0
                                        draw_bg.shadow_offset: vec2(0, 4)
                                        library_share_book := ButtonFlatter{
                                            width: Fill height: 34 text: "分享"
                                        }
                                        library_want_to_read := ButtonFlatter{
                                            width: Fill height: 34 text: "加入欲读清单"
                                        }
                                        Hr{}
                                        library_add_collection := ButtonFlatter{
                                            width: Fill height: 34 text: "添加到藏书"
                                        }
                                        library_mark_finished := ButtonFlatter{
                                            width: Fill height: 34 text: "标记为已读完"
                                        }
                                        library_rename_book := ButtonFlatter{
                                            width: Fill height: 34 text: "重新命名"
                                        }
                                        library_delete_book := ButtonFlatter{
                                            width: Fill height: 34 text: "删除"
                                            draw_text.color: theme.color_error
                                        }
                                    }
                                }
                            }
                        }

                        page_reader := SolidView{
                            width: Fill height: Fill
                            flow: Down
                            draw_bg.color: #xf7f4ea
                            header := SolidView{
                                width: Fill height: Fit
                                padding: 12 spacing: 12
                                align: Align{ y: 0.5 }
                                draw_bg.color: #xf0ecdf
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
                            reader_body := SolidView{
                                width: Fill height: Fill
                                flow: Overlay
                                draw_bg.color: #xf7f4ea
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
                                    reader_surface := SolidView{
                                        width: Fill height: Fill
                                        new_batch: true
                                        flow: Down
                                        draw_bg.color: #xf7f4ea
                                        chapter_list := mod.widgets.ChapterList{}
                                        reader_footer := View{
                                            width: Fill height: 30
                                            flow: Overlay
                                            reader_page_center_layer := View{
                                                width: Fill height: Fill
                                                align: Center
                                                reader_page_indicator := mod.widgets.ReaderPageIndicator{}
                                            }
                                            reader_page_right_layer := View{
                                                width: Fill height: Fill
                                                align: Align{x: 1.0 y: 0.5}
                                                padding: Inset{right: 18}
                                                reader_pages_left := Label{
                                                    width: Fit height: Fit
                                                    text: ""
                                                    draw_text.color: theme.color_label_inner_inactive
                                                    draw_text.text_style.font_size: 8.5
                                                }
                                            }
                                        }
                                    }
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
                                    reader_chat_list := mod.widgets.ChatList{reader_style: true}
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
                                    flow: Overlay
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
                                            visible: false
                                            width: Fill
                                            text: ""
                                            draw_text.color: theme.color_label_inner_inactive
                                            draw_text.text_style.font_size: 8.5
                                        }
                                    }
                                }
                                word_card_layer := View{
                                    // Keep the transparent owner in the draw tree so its
                                    // Area can schedule the very first popup frame. Only
                                    // the card itself is toggled and receives pointer hits.
                                    width: Fill height: Fill
                                    flow: Overlay
                                    word_card := RoundedShadowView{
                                        visible: false
                                        width: 420 height: Fit
                                        new_batch: true
                                        padding: 14
                                        flow: Down spacing: 10
                                        draw_bg.color: theme.color_bg_container
                                        draw_bg.border_color: theme.color_bg_highlight_inline
                                        draw_bg.border_radius: 10.0
                                        draw_bg.shadow_color: #0004
                                        draw_bg.shadow_radius: 12.0
                                        draw_bg.shadow_offset: vec2(0, 4)
                                        card_head := View{
                                            width: Fill
                                            height: Fit
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
                            reader_background_row := View{
                                width: Fill height: Fit
                                flow: Flow.Right{wrap: true} spacing: 12
                                align: Align{y: 0.5}
                                reader_background_label := Label{text: "阅读背景"}
                                reader_background_paper := RadioButtonTabFlat{text: "纸白"}
                                reader_background_sepia := RadioButtonTabFlat{text: "米黄"}
                                reader_background_sage := RadioButtonTabFlat{text: "浅绿"}
                                reader_background_night := RadioButtonTabFlat{text: "夜间"}
                                reader_background_theme := RadioButtonTabFlat{text: "跟随主题"}
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
                    reader_image_modal := Modal{
                        content +: {
                            width: Fill height: Fill
                            image_viewer_card := RoundedView{
                                width: Fill height: Fill
                                new_batch: true
                                flow: Down spacing: 10
                                margin: Inset{left: 18 right: 18 top: 18 bottom: 18}
                                padding: Inset{left: 12 right: 12 top: 10 bottom: 12}
                                draw_bg.color: theme.color_bg_container
                                draw_bg.border_radius: 10.0
                                image_viewer_header := View{
                                    width: Fill height: Fit
                                    flow: Right spacing: 10
                                    align: Align{y: 0.5}
                                    image_viewer_info := Label{
                                        width: Fill height: Fit
                                        text: "原图"
                                        draw_text.color: theme.color_label_inner
                                        draw_text.text_style.font_size: 10
                                    }
                                    image_viewer_close := ButtonFlat{text: "关闭"}
                                }
                                image_viewer_image := Image{
                                    width: Fill height: Fill
                                    fit: ImageFit.Smallest
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
                    library_rename_modal := Modal{
                        content +: {
                            width: 400 height: Fit
                            library_rename_card := RoundedView{
                                width: Fill height: Fit
                                new_batch: true
                                flow: Down spacing: 12 padding: 22
                                draw_bg.color: theme.color_bg_container
                                draw_bg.border_radius: 10.0
                                library_rename_title := Label{
                                    width: Fill
                                    text: "重新命名"
                                    draw_text.text_style.font_size: 15
                                }
                                library_rename_input := TextInput{
                                    width: Fill height: 40
                                    empty_text: "输入新的书名"
                                }
                                library_rename_status := Label{
                                    width: Fill height: Fit
                                    text: ""
                                    draw_text.color: theme.color_highlight
                                    draw_text.text_style.font_size: 9
                                }
                                library_rename_actions := View{
                                    width: Fill height: Fit
                                    flow: Right spacing: 8
                                    align: Align{x: 1.0 y: 0.5}
                                    library_rename_cancel := ButtonFlat{text: "取消"}
                                    library_rename_save := Button{text: "保存"}
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Script, ScriptHook, Widget)]
struct ReaderPageIndicator {
    #[deref]
    view: View,
    #[rust]
    current: usize,
    #[rust]
    total: usize,
    #[rust]
    hovered: bool,
}

impl ReaderPageIndicator {
    fn refresh_text(&self, cx: &mut Cx) {
        let text = if self.total == 0 {
            "—".to_string()
        } else if self.hovered {
            format!("{} of {}", self.current, self.total)
        } else {
            self.current.to_string()
        };
        self.view.label(cx, ids!(page_text)).set_text(cx, &text);
    }

    fn set_pages(&mut self, cx: &mut Cx, current: usize, total: usize) {
        self.current = current.min(total).max(usize::from(total > 0));
        self.total = total;
        self.refresh_text(cx);
    }
}

impl Widget for ReaderPageIndicator {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        self.view.draw_walk(cx, scope, walk)
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        self.view.handle_event(cx, event, scope);
        match event.hits(cx, self.view.area()) {
            Hit::FingerHoverIn(_) if !self.hovered => {
                self.hovered = true;
                self.refresh_text(cx);
            }
            Hit::FingerHoverOut(_) if self.hovered => {
                self.hovered = false;
                self.refresh_text(cx);
            }
            _ => {}
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
    #[live]
    html_default_inset: f64,
    #[live]
    draw_reader_selection: DrawColor,
    /// 选区自跟踪（PortalList 未暴露区间 API，用公开的命中接口镜像）。
    /// 普通文字与原生选区同步；注音词/链接等交互内联文字由镜像投影绘制。
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

struct UnifiedReaderLayout {
    laidout: Rc<LaidoutText>,
    origin: DVec2,
    font_scale: f32,
    text_len: usize,
}

/// 把每行的选区扩展到相邻字形框之间的中线。相邻两行选中时，前一行的
/// bottom 与后一行的 top 因而完全相同，不会在行距中留下未着色的细缝。
fn visual_selection_row_bounds(
    previous_glyph_bottom: Option<f32>,
    glyph_top: f32,
    glyph_bottom: f32,
    next_glyph_top: Option<f32>,
    edge_padding: f32,
) -> (f32, f32) {
    let top = previous_glyph_bottom
        .map(|previous_bottom| (previous_bottom + glyph_top) * 0.5)
        .unwrap_or(glyph_top - edge_padding);
    let bottom = next_glyph_top
        .map(|next_top| (glyph_bottom + next_top) * 0.5)
        .unwrap_or(glyph_bottom + edge_padding);
    (top.min(glyph_top), bottom.max(glyph_bottom))
}

impl ChapterList {
    fn set_compact_layout(&mut self, cx: &mut Cx, compact_layout: bool) {
        if self.compact_layout == compact_layout {
            return;
        }
        self.compact_layout = compact_layout;
        let reader_padding = if compact_layout {
            Inset {
                left: 12.0,
                right: 12.0,
                top: 8.0,
                bottom: 8.0,
            }
        } else {
            Inset {
                left: 32.0,
                right: 32.0,
                top: 8.0,
                bottom: 8.0,
            }
        };
        let drag_scrolling = compact_layout;
        let mut list_widget = self.view.widget(cx, ids!(list));
        script_apply_eval!(cx, list_widget, {
            padding: #(reader_padding)
            drag_scrolling: #(drag_scrolling)
        });
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

    /// 把富文本的可见正文按当前阅读字体统一排版。实际 Html 仍按各个富文本
    /// run 绘制；统一布局只服务于精确命中，以及注音段落连续的选区底色。
    fn unified_markup_layout(
        &self,
        cx: &mut Cx,
        widget: &WidgetRef,
        html_source: &str,
    ) -> Option<UnifiedReaderLayout> {
        let plain = readwise_core::vocab::annotate::rendered_text(html_source);
        if plain.is_empty() {
            return None;
        }
        let html_ref = widget.widget(cx, ids!(para_html));
        let rect = html_ref.area().rect(cx);
        let mut html = html_ref.borrow_mut::<Html>()?;
        let text_flow = &mut html.text_flow;

        // Gloss templates replace Html's theme inset with 22px of top room
        // for ruby annotations and zero horizontal inset.
        let has_gloss = html_source.to_ascii_lowercase().contains("<ruby");
        let (padding_left, padding_right, padding_top) = if has_gloss {
            (0.0, 0.0, 22.0)
        } else {
            (
                self.html_default_inset,
                self.html_default_inset,
                self.html_default_inset,
            )
        };
        let trimmed = html_source.trim_start().to_ascii_lowercase();
        let (heading_scale, block_margin_top) = if trimmed.starts_with("<h1") {
            (2.0, text_flow.heading_margin.top)
        } else if trimmed.starts_with("<h2") {
            (1.5, text_flow.heading_margin.top)
        } else if trimmed.starts_with("<h3") {
            (1.17, text_flow.heading_margin.top)
        } else if trimmed.starts_with("<h4") {
            (1.0, text_flow.heading_margin.top)
        } else if trimmed.starts_with("<h5") {
            (0.83, text_flow.heading_margin.top)
        } else if trimmed.starts_with("<h6") {
            (0.67, text_flow.heading_margin.top)
        } else if trimmed.starts_with("<p") {
            (1.0, text_flow.paragraph_margin.top)
        } else {
            (1.0, 0.0)
        };

        let saved_style = text_flow.draw_text.text_style.clone();
        let saved_max_lines = text_flow.draw_text.max_lines;
        let font_scale = text_flow.draw_text.font_scale;
        let mut hit_style = if trimmed.starts_with("<h") {
            text_flow.text_style_bold.clone()
        } else {
            text_flow.text_style_normal.clone()
        };
        hit_style.font_size = text_flow.font_size * heading_scale;
        let top_drop = hit_style.top_drop;
        text_flow.draw_text.text_style = hit_style;
        text_flow.draw_text.max_lines = 0;
        let max_width = (rect.size.x - padding_left - padding_right).max(1.0) as f32;
        let laidout = text_flow.draw_text.layout(
            cx,
            0.0,
            0.0,
            Some(max_width),
            true,
            Align::default(),
            &plain,
        );
        text_flow.draw_text.text_style = saved_style;
        text_flow.draw_text.max_lines = saved_max_lines;

        if laidout.rows.is_empty() {
            return None;
        }
        // TextFlow 在排版之后才通过 temp_y_shift 应用 top_drop。统一布局的
        // 字符位置必须补上同一偏移，否则选区会整体停在实际文字的上方。
        let text_y_shift = laidout
            .rows
            .first()
            .and_then(|row| row.glyphs.first())
            .map(|glyph| glyph.font_size_in_lpxs * top_drop)
            .unwrap_or(0.0) as f64;
        let origin = rect.pos
            + dvec2(
                padding_left,
                padding_top
                    + text_flow.font_size as f64 * heading_scale as f64 * block_margin_top
                    + text_y_shift,
            );
        Some(UnifiedReaderLayout {
            laidout,
            origin,
            font_scale,
            text_len: plain.len(),
        })
    }

    /// 斜体、粗体、链接、注音和 `<mark>` 都会把同一段拆成多个 TextFlow run。
    /// Makepad 逐 run 的命中使用跨行整体包围框，run 重叠时会错误地总选中
    /// 前一个，因此这些段落统一按连续的可见正文布局命中。
    fn unified_markup_hit_index(
        &self,
        cx: &mut Cx,
        widget: &WidgetRef,
        html_source: &str,
        abs: DVec2,
    ) -> Option<usize> {
        let layout = self.unified_markup_layout(cx, widget, html_source)?;
        let local = LayoutPoint::new(
            ((abs.x - layout.origin.x) / layout.font_scale as f64) as f32,
            ((abs.y - layout.origin.y) / layout.font_scale as f64) as f32,
        );
        Some(
            layout
                .laidout
                .point_in_lpxs_to_cursor(local)
                .index
                .min(layout.text_len),
        )
    }

    /// 按连续的可见正文坐标绘制一个不参与布局的覆盖层。RubyWord 等自定义
    /// run 仍由原始 Html/TextFlow 排版；选区或永久高亮只在排版后增加颜色。
    fn draw_unified_range_overlay(
        &mut self,
        cx: &mut Cx2d,
        widget: &WidgetRef,
        html_source: &str,
        range: (usize, usize),
        color: Vec4f,
    ) {
        let Some(layout) = self.unified_markup_layout(cx, widget, html_source) else {
            return;
        };
        let start = range.0.min(layout.text_len);
        let end = range.1.min(layout.text_len);
        if start >= end {
            return;
        }
        self.draw_reader_selection.color = color;
        let selection = LayoutSelection {
            anchor: LayoutCursor {
                index: start,
                prefer_next_row: false,
            },
            cursor: LayoutCursor {
                index: end,
                prefer_next_row: false,
            },
        };
        let start_row = layout
            .laidout
            .cursor_to_position(selection.start())
            .row_index;
        let edge_padding_in_lpxs = 2.0 / layout.font_scale.max(f32::EPSILON);
        for (row_offset, selection_rect) in layout
            .laidout
            .selection_rects(selection)
            .into_iter()
            .enumerate()
        {
            let row_index = start_row + row_offset;
            let Some(row) = layout.laidout.rows.get(row_index) else {
                continue;
            };
            let glyph_top = row.origin_in_lpxs.y - row.ascender_in_lpxs;
            let glyph_bottom = row.origin_in_lpxs.y - row.descender_in_lpxs;
            let previous_glyph_bottom = row_index.checked_sub(1).and_then(|previous_index| {
                layout
                    .laidout
                    .rows
                    .get(previous_index)
                    .map(|previous| previous.origin_in_lpxs.y - previous.descender_in_lpxs)
            });
            let next_glyph_top = layout
                .laidout
                .rows
                .get(row_index + 1)
                .map(|next| next.origin_in_lpxs.y - next.ascender_in_lpxs);
            let (visual_top, visual_bottom) = visual_selection_row_bounds(
                previous_glyph_bottom,
                glyph_top,
                glyph_bottom,
                next_glyph_top,
                edge_padding_in_lpxs,
            );
            self.draw_reader_selection.draw_abs(
                cx,
                Rect {
                    pos: layout.origin
                        + dvec2(
                            (selection_rect.rect_in_lpxs.origin.x * layout.font_scale) as f64,
                            (visual_top * layout.font_scale) as f64,
                        ),
                    size: dvec2(
                        (selection_rect.rect_in_lpxs.size.width * layout.font_scale) as f64,
                        ((visual_bottom - visual_top) * layout.font_scale) as f64,
                    ),
                },
            );
        }
    }

    /// 命中可见块的 (块id, 正文字符下标)。点位在视口外或块间隙时吸附到
    /// 最近边界，与 PortalList 原生选择规则保持一致。
    fn hit_block_char(&self, cx: &mut Cx, abs: DVec2) -> Option<(usize, usize)> {
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
                let paragraphs = PARAGRAPHS.read().unwrap();
                let paragraph = paragraphs.get(*id)?;
                let plain_len = readwise_core::vocab::annotate::rendered_text(paragraph).len();
                let is_code = CODE_BLOCKS
                    .read()
                    .unwrap()
                    .get(*id)
                    .and_then(Option::as_ref)
                    .is_some();
                let char_idx = if needs_unified_reader_hit_test(paragraph) && !is_code {
                    self.unified_markup_hit_index(cx, widget, paragraph, abs)?
                } else {
                    widget
                        .selection_point_to_char_index(cx, abs)?
                        .saturating_sub(textflow_selection_prefix_len(paragraph))
                };
                return Some((*id, char_idx.min(plain_len)));
            }
        }
        if let Some((id, widget, _)) = visible
            .iter()
            .rev()
            .find(|(_, _, rect)| rect.pos.y + rect.size.y <= abs.y)
        {
            let len = PARAGRAPHS
                .read()
                .unwrap()
                .get(*id)
                .map(|paragraph| readwise_core::vocab::annotate::rendered_text(paragraph).len())
                .unwrap_or_else(|| widget.selection_text_len());
            return Some((*id, len));
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
        let paragraphs = PARAGRAPHS.read().unwrap();
        let list = self.view.portal_list(cx, ids!(list));
        let first = list.first_id();
        let vis = list.visible_items() + 1;
        for id in first..first + vis {
            let Some((_, widget)) = list.get_item(id) else {
                continue;
            };
            let paragraph = paragraphs.get(id).map(String::as_str).unwrap_or("");
            let prefix = textflow_selection_prefix_len(paragraph);
            let source_len = readwise_core::vocab::annotate::rendered_text(paragraph).len();
            match range {
                None => widget.selection_clear(),
                Some((start, end)) if id < start.0 || id > end.0 => widget.selection_clear(),
                Some((start, end)) if id == start.0 && id == end.0 => {
                    widget.selection_set(start.1 + prefix, end.1 + prefix)
                }
                Some((start, _)) if id == start.0 => {
                    widget.selection_set(start.1 + prefix, source_len + prefix)
                }
                Some((_, end)) if id == end.0 => widget.selection_set(prefix, end.1 + prefix),
                Some(_) => widget.selection_set(prefix, source_len + prefix),
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
            .map(|block| readwise_core::vocab::annotate::rendered_text(block).len())
            .unwrap_or(0);
        let text_offset = self
            .sel_anchor
            .map(|anchor| if id >= anchor.0 { len } else { 0 })
            .unwrap_or(0);
        self.set_cursor((id, text_offset));
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
            SelAction::Show {
                anchor: self.last_abs,
            }
        } else {
            SelAction::Hide
        });
    }

    fn track_selection(&mut self, cx: &mut Cx, event: &Event) {
        match event {
            Event::MouseDown(e) => {
                if e.button != MouseButton::PRIMARY {
                    return;
                }
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
                // Inline text widgets (ruby words and links) must keep receiving
                // clicks, but they are also valid text-selection starting points.
                // Their FingerUp `was_tap()` check distinguishes a click from a drag.
                if let Some(hit) = self.hit_block_char(cx, e.abs) {
                    self.sel_anchor = Some(hit);
                    self.selecting = true;
                }
                if std::env::var("SEL_DEBUG").is_ok() {
                    eprintln!("[SEL] down: anchor={:?}", self.sel_anchor);
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
            Event::MouseUp(event) if self.selecting => {
                self.last_abs = Some(event.abs);
                self.finish_selection();
            }
            Event::LongPress(event) if self.compact_layout => {
                let list = self.view.portal_list(cx, ids!(list));
                if !list.area().rect(cx).contains(event.abs) {
                    return;
                }
                let Some((block_id, text_offset)) = self.hit_block_char(cx, event.abs) else {
                    return;
                };
                let Some((start, end)) =
                    PARAGRAPHS
                        .read()
                        .unwrap()
                        .get(block_id)
                        .and_then(|paragraph| {
                            let text = readwise_core::vocab::annotate::rendered_text(paragraph);
                            word_range_at(&text, text_offset)
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
                    TouchState::Stop => {
                        self.last_abs = Some(touch.abs);
                        self.finish_selection();
                    }
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
        let code_blocks = CODE_BLOCKS.read().unwrap();
        let reader_font = *READER_FONT.read().unwrap();
        let reader_spacing = *READER_SPACING.read().unwrap();
        let reader_background = *READER_BACKGROUND.read().unwrap();
        let reader_theme = *READER_THEME.read().unwrap();
        let (_, reader_text_color) = reader_palette(reader_background, reader_theme);
        let reader_scene = reader_scene_palette(reader_background, reader_theme);
        let code_template = match reader_background {
            ReaderBackground::Paper => id!(CodePaper),
            ReaderBackground::Sepia => id!(CodeSepia),
            ReaderBackground::Sage => id!(CodeSage),
            ReaderBackground::Night => id!(CodeNight),
            ReaderBackground::FollowTheme if reader_theme == ReaderTheme::Light => {
                id!(CodeThemeLight)
            }
            ReaderBackground::FollowTheme => id!(CodeThemeDark),
        };
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
        self.draw_reader_selection.color = selection_color;
        let selection_range = *SEL_RANGE.read().unwrap();
        let chapter_idx = CURRENT_CHAPTER.load(Ordering::Relaxed);
        let chapter_highlights = CURRENT_BOOK
            .read()
            .unwrap()
            .as_ref()
            .and_then(|book| book.highlights.get(&chapter_idx))
            .cloned()
            .unwrap_or_default();
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
                    let code_text = code_blocks.get(item_id).and_then(Option::as_deref);
                    let has_gloss = para.contains("<ruby");
                    let is_table = is_table_block(para);
                    let template = if code_text.is_some() {
                        code_template
                    } else if is_table && has_gloss {
                        id!(TableGlossPara)
                    } else if is_table {
                        id!(TablePara)
                    } else if has_gloss {
                        id!(GlossPara)
                    } else {
                        id!(Para)
                    };
                    let line_spacing = if has_gloss {
                        reader_spacing.gloss_line_spacing()
                    } else {
                        reader_spacing.plain_line_spacing()
                    };
                    let item = list.item(cx, item_id, template);
                    if let Some(code_text) = code_text {
                        item.widget(cx, ids!(code_view)).set_text(cx, code_text);
                    } else {
                        let html = item.widget(cx, ids!(para_html));
                        if let Some(mut inner) = html.borrow_mut::<Html>() {
                            if inner.body.as_ref() != para {
                                // Html assigns anonymous custom tags (`mark`, `ruby`,
                                // links) positional IDs. Adding/removing a mark can
                                // therefore move a RubyWord onto an ID previously owned
                                // by MarkSpan. TextFlow::item_with_scope does not replace
                                // an existing item when its template changes, so retaining
                                // the map would draw the gloss as ordinary inline content.
                                // Rebuild only when the document actually changes; steady
                                // frames keep their widgets and area references.
                                inner.text_flow.clear_items();
                                inner.set_text(cx, para);
                            }
                            inner.text_flow.selectable = !self.compact_layout;
                            inner.text_flow.font_size = reader_font_size;
                            inner.text_flow.font_color = reader_text_color;
                            inner.text_flow.draw_text.color = reader_text_color;
                            // RubyWord is drawn as a separate continuation run. Its
                            // native selection rect can use a different row origin
                            // from neighboring text, so gloss paragraphs draw one
                            // unified selection overlay after the Html pass instead.
                            inner.text_flow.draw_selection.color = if has_gloss {
                                Vec4f::default()
                            } else {
                                selection_color
                            };
                            inner.text_flow.draw_block.table_header_bg_color = reader_scene.field;
                            inner.text_flow.draw_block.table_border_color = reader_scene.border;
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
                        };
                    }
                    if let Some((start, end)) = selection_range {
                        let prefix = textflow_selection_prefix_len(para);
                        let source_len = readwise_core::vocab::annotate::rendered_text(para).len();
                        if item_id < start.0 || item_id > end.0 {
                            item.selection_clear();
                        } else if item_id == start.0 && item_id == end.0 {
                            item.selection_set(start.1 + prefix, end.1 + prefix);
                        } else if item_id == start.0 {
                            item.selection_set(start.1 + prefix, source_len + prefix);
                        } else if item_id == end.0 {
                            item.selection_set(prefix, end.1 + prefix);
                        } else {
                            item.selection_set(prefix, source_len + prefix);
                        }
                    } else {
                        item.selection_clear();
                    }
                    item.draw_all_unscoped(cx);
                    let source_len = readwise_core::vocab::annotate::rendered_text(para).len();
                    if code_text.is_none() {
                        for highlight in &chapter_highlights {
                            let Some(range) =
                                highlight_range_in_block(highlight, item_id, source_len)
                            else {
                                continue;
                            };
                            let color = crate::mark_span::hl_color(
                                crate::mark_span::hl_slot_for_name(&highlight.color),
                            );
                            self.draw_unified_range_overlay(cx, &item, para, range, color);
                        }
                    }
                    if has_gloss {
                        if let Some(range) = selection_range
                            .and_then(|range| selection_range_in_block(range, item_id, source_len))
                        {
                            self.draw_unified_range_overlay(
                                cx,
                                &item,
                                para,
                                range,
                                selection_color,
                            );
                        }
                    }
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
    #[live]
    text_color: Vec4f,
    #[live]
    button_hover: Vec4f,
    #[live]
    button_pressed: Vec4f,
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
                    let mut button = item.widget(cx, ids!(toc_item_btn));
                    button.set_text(cx, &text);
                    let text_color = self.text_color;
                    let button_hover = self.button_hover;
                    let button_pressed = self.button_pressed;
                    script_apply_eval!(cx, button, {
                        draw_text +: {
                            color: #(text_color)
                            color_hover: #(text_color)
                            color_down: #(text_color)
                            color_focus: #(text_color)
                        }
                        draw_bg +: {
                            color: #0000
                            color_hover: #(button_hover)
                            color_down: #(button_pressed)
                            color_focus: #(button_hover)
                            border_color: #0000
                            border_color_hover: #0000
                            border_color_down: #0000
                            border_color_focus: #0000
                        }
                    });
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
        let palette = reader_scene_palette(
            *READER_BACKGROUND.read().unwrap(),
            *READER_THEME.read().unwrap(),
        );
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
                    let note_title = item.widget(cx, ids!(note_title));
                    let note_preview = item.widget(cx, ids!(note_preview));
                    let note_delete = item.widget(cx, ids!(note_delete_btn));
                    apply_reader_surface_palette(cx, item.clone(), palette.surface, palette.border);
                    apply_reader_label_palette(cx, note_title, palette.text);
                    apply_reader_label_palette(cx, note_preview, palette.muted);
                    apply_reader_button_palette(cx, note_delete, palette);
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
    #[live(false)]
    reader_style: bool,
}

impl Widget for ChatList {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let data = CHAT_DATA.read().unwrap();
        let language = *APP_LANGUAGE.read().unwrap();
        let reader_palette = self.reader_style.then(|| {
            reader_scene_palette(
                *READER_BACKGROUND.read().unwrap(),
                *READER_THEME.read().unwrap(),
            )
        });
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
                        if let Some(palette) = reader_palette {
                            apply_reader_surface_palette(
                                cx,
                                item.clone(),
                                palette.surface,
                                palette.border,
                            );
                            let message_label = item.widget(cx, ids!(message));
                            apply_reader_label_palette(cx, message_label, palette.text);
                        }
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
                    if let Some(palette) = reader_palette {
                        let background = match message.role {
                            ChatRole::User => palette.field,
                            ChatRole::Assistant => palette.surface,
                        };
                        apply_reader_surface_palette(cx, item.clone(), background, palette.border);
                        let message_label = item.widget(cx, ids!(message));
                        apply_reader_label_palette(cx, message_label, palette.text);
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
    /// 卡片 WidgetUid → 已加载的封面路径。纹理属于组件实例，不能只按
    /// book_index 缓存：窗口重排可能用一个新卡片实例展示同一本书。
    #[rust]
    loaded_covers: HashMap<WidgetUid, Option<String>>,
    #[rust]
    projection_revision: usize,
    #[rust]
    alternate_template: bool,
    #[rust]
    last_view_mode: LibraryViewMode,
    #[rust]
    grid_columns: usize,
}

impl BookShelf {
    fn bind_book_card(
        &mut self,
        cx: &mut Cx2d,
        card: &WidgetRef,
        book: &ShelfItem,
        language: AppLanguage,
    ) {
        let letter = book
            .title
            .chars()
            .next()
            .map(|c| c.to_string())
            .unwrap_or_default();
        card.label(cx, ids!(cover_letter)).set_text(cx, &letter);
        card.label(cx, ids!(card_title)).set_text(cx, &book.title);
        card.label(cx, ids!(card_author)).set_text(cx, &book.author);
        card.label(cx, ids!(card_status)).set_text(
            cx,
            &shelf_status(
                language,
                &book.status,
                book.progress_current,
                book.progress_total,
            ),
        );
        let card_uid = card.widget_uid();
        if shelf_cover_needs_load(&self.loaded_covers, card_uid, &book.cover_path) {
            let img = card.image(cx, ids!(cover_img));
            match &book.cover_path {
                Some(path) => {
                    let path = std::path::Path::new(path);
                    if let Err(error) = img.load_image_file_by_path(cx, path) {
                        println!("封面加载失败 {}: {:?}", path.display(), error);
                        img.set_texture(cx, None);
                    }
                }
                None => img.set_texture(cx, None),
            }
            if card_uid != WidgetUid(0) {
                self.loaded_covers.insert(card_uid, book.cover_path.clone());
            }
        }
    }

    fn grid_columns(&self) -> usize {
        self.grid_columns.max(2)
    }
}

impl Widget for BookShelf {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let shelf = SHELF_VISIBLE.read().unwrap();
        let language = *APP_LANGUAGE.read().unwrap();
        let view_mode = *LIBRARY_VIEW_MODE.read().unwrap();
        let measured_width = self.view.area().rect(cx).size.x;
        let available_width = if measured_width > 0.0 {
            measured_width
        } else {
            cx.display_context.screen_size.x
        };
        let grid_columns = shelf_grid_columns_for_width(available_width);

        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_portal_list().borrow_mut() {
                let item_count = if view_mode == LibraryViewMode::Grid {
                    shelf_grid_row_count(shelf.entries.len(), grid_columns)
                } else {
                    shelf.entries.len()
                };
                list.set_item_range(cx, 0, item_count);
                if self.projection_revision != shelf.revision
                    || self.last_view_mode != view_mode
                    || (view_mode == LibraryViewMode::Grid && self.grid_columns != grid_columns)
                {
                    list.set_first_id_and_scroll(0, 0.0);
                    self.projection_revision = shelf.revision;
                    self.last_view_mode = view_mode;
                    self.grid_columns = grid_columns;
                    self.alternate_template = !self.alternate_template;
                    self.loaded_covers.clear();
                }
                let template = if view_mode == LibraryViewMode::Grid {
                    if self.alternate_template {
                        id!(GridRowAlternate)
                    } else {
                        id!(GridRow)
                    }
                } else if self.alternate_template {
                    id!(CardAlternate)
                } else {
                    id!(Card)
                };
                while let Some(item_id) = list.next_visible_item(cx) {
                    // PortalList deliberately probes beyond range_end until a pass
                    // draws nothing. Never instantiate that sentinel: doing so gives
                    // it the template's height and turns stale filtered rows into
                    // visible empty cards.
                    if item_id >= item_count {
                        continue;
                    }
                    let item = list.item(cx, item_id, template);
                    if view_mode == LibraryViewMode::Grid {
                        for (column, cell_id) in [
                            id!(grid_cell_0),
                            id!(grid_cell_1),
                            id!(grid_cell_2),
                            id!(grid_cell_3),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            let cell = item.widget(cx, &[cell_id]);
                            cell.set_visible(cx, column < grid_columns);
                            if column >= grid_columns {
                                continue;
                            }
                            let card = cell.widget(cx, ids!(grid_card));
                            let book_index = shelf_grid_book_index(
                                item_id,
                                column,
                                grid_columns,
                                shelf.entries.len(),
                            );
                            card.set_visible(cx, book_index.is_some());
                            if let Some(book_index) = book_index {
                                self.bind_book_card(
                                    cx,
                                    &card,
                                    &shelf.entries[book_index],
                                    language,
                                );
                            }
                        }
                    } else if let Some(book) = shelf.entry_for_item_id(item_id) {
                        self.bind_book_card(cx, &item, book, language);
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
    #[rust]
    editing_book_title_id: Option<String>,
    /// 三点菜单由书库公共覆盖层持有，卡片只提供书籍 id 与按钮锚点。
    #[rust]
    library_action_book_id: Option<String>,
    #[rust]
    library_action_anchor: Option<Rect>,
    #[rust]
    pending_library_action_layout: bool,
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
    reading_progress_save_timer: Timer,
    #[rust]
    statistics_after_pending_session: bool,
    #[rust]
    chapter_idx: usize,
    #[rust]
    reader_chapter_block_counts: Vec<usize>,
    #[rust]
    reader_page_span: usize,
    #[rust]
    pending_reader_pagination_layout: bool,
    /// 选区工具条由 App 这一覆盖层所有者定位；锚点是选区手势结束的屏幕坐标。
    #[rust]
    selection_popup_anchor: Option<DVec2>,
    #[rust]
    pending_selection_popup_layout: bool,
    /// 生词释义卡片与选区工具条一样由阅读器覆盖层定位，避免参与纵向正文布局。
    #[rust]
    word_card_anchor: Option<DVec2>,
    #[rust]
    pending_word_card_layout: bool,
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
    /// A responsive transition can recreate hidden widget subtrees during the
    /// next Draw. Project state only after that draw has instantiated them.
    #[rust]
    pending_ui_projection_after_draw: bool,
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
    fn project_db_ready_status(&self, cx: &mut Cx, vocab_count: usize, known_count: usize) {
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
    }

    /// Mobile rotation can instantiate subtrees that were hidden in the other
    /// responsive mode. Re-project business state after a complete Draw so DSL
    /// placeholder values cannot win and WidgetRef setters cannot hit an empty
    /// reference.
    fn restore_projected_ui_after_draw(&mut self, cx: &mut Cx) {
        self.activate_page(cx, self.active_page);
        if let Some((vocab_count, known_count)) = *DB_READY_COUNTS.read().unwrap() {
            self.project_db_ready_status(cx, vocab_count, known_count);
        }
        self.refresh_shelf_projection(cx);
        let shelf_count = SHELF.read().unwrap().len();
        let visible_count = SHELF_VISIBLE.read().unwrap().entries.len();
        ::log::info!(
            "Responsive projection after draw: shelf={shelf_count}, visible={visible_count}, page={:?}",
            self.active_page
        );
        self.ui.redraw(cx);
    }

    fn activate_page(&mut self, cx: &mut Cx, page: AppPage) {
        if page != AppPage::Library && self.library_action_book_id.is_some() {
            self.close_library_action_menu(cx);
        }
        self.ui
            .page_flip(cx, ids!(pages))
            .set_active_page(cx, page.live_id());
        self.active_page = page;
        ACTIVE_PAGE.store(page.persisted(), Ordering::Relaxed);
        self.on_reader_page = page == AppPage::Reader;
        self.apply_reader_background(cx, *READER_BACKGROUND.read().unwrap());
    }

    fn apply_navigation_palette(&self, cx: &mut Cx, page: AppPage) {
        let theme = *READER_THEME.read().unwrap();
        let palette = navigation_palette_for_page(page, *READER_BACKGROUND.read().unwrap(), theme);

        for id in [id!(sidebar), id!(mobile_safe_top), id!(mobile_nav)] {
            apply_reader_surface_palette(
                cx,
                self.ui.widget(cx, &[id]),
                palette.surface,
                palette.border,
            );
        }
        apply_reader_surface_palette(
            cx,
            self.ui.widget(cx, ids!(main_window.caption_bar)),
            palette.surface,
            palette.surface,
        );
        apply_reader_label_palette(
            cx,
            self.ui
                .widget(cx, ids!(main_window.caption_bar.caption_label.label)),
            palette.text,
        );
        apply_reader_label_palette(cx, self.ui.widget(cx, ids!(app_title)), palette.text);
        apply_reader_label_palette(cx, self.ui.widget(cx, ids!(db_status)), palette.muted);
        for id in [
            id!(nav_library),
            id!(nav_reader),
            id!(nav_chat),
            id!(nav_skills),
            id!(nav_vocab),
            id!(nav_statistics),
            id!(nav_settings),
            id!(mobile_nav_library),
            id!(mobile_nav_reader),
            id!(mobile_nav_chat),
            id!(mobile_nav_skills),
            id!(mobile_nav_vocab),
            id!(mobile_nav_statistics),
            id!(mobile_nav_settings),
        ] {
            apply_reader_button_palette(cx, self.ui.widget(cx, &[id]), palette);
        }
        self.ui.widget(cx, ids!(sidebar)).redraw(cx);
        self.ui.widget(cx, ids!(mobile_nav)).redraw(cx);
    }

    fn sync_library_view_controls(&self, cx: &mut Cx, mode: LibraryViewMode) {
        self.ui
            .radio_button(cx, ids!(library_view_list))
            .set_active(cx, mode == LibraryViewMode::List, Animate::No);
        self.ui
            .radio_button(cx, ids!(library_view_grid))
            .set_active(cx, mode == LibraryViewMode::Grid, Animate::No);
    }

    fn set_library_view_mode(&mut self, cx: &mut Cx, mode: LibraryViewMode) {
        self.close_library_action_menu(cx);
        if *LIBRARY_VIEW_MODE.read().unwrap() == mode {
            self.sync_library_view_controls(cx, mode);
            return;
        }
        *LIBRARY_VIEW_MODE.write().unwrap() = mode;
        self.sync_library_view_controls(cx, mode);
        self.ui.widget(cx, ids!(book_shelf)).redraw(cx);
        self.persist_reader_settings(cx);
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
        if self.library_tag_filter.as_ref().is_some_and(|selected| {
            selected != WANT_TO_READ_FILTER
                && !tags.iter().any(|tag| tag.eq_ignore_ascii_case(selected))
        }) {
            self.library_tag_filter = None;
        }

        let all = SHELF.read().unwrap().clone();
        let visible = shelf_items_for_tag(&all, self.library_tag_filter.as_deref());
        let visible_count = visible.len();
        let total = all.len();
        SHELF_VISIBLE.write().unwrap().replace(visible);

        let mut labels = Vec::with_capacity(tags.len() + 2);
        labels.push(tr("all_tags").to_string());
        labels.push(tr("want_to_read").to_string());
        labels.extend(tags.iter().cloned());
        let selected_index = self
            .library_tag_filter
            .as_ref()
            .and_then(|selected| {
                if selected == WANT_TO_READ_FILTER {
                    Some(1)
                } else {
                    tags.iter()
                        .position(|tag| tag.eq_ignore_ascii_case(selected))
                        .map(|index| index + 2)
                }
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
        let is_want_to_read_filter =
            self.library_tag_filter.as_deref() == Some(WANT_TO_READ_FILTER);
        self.ui.label(cx, ids!(library_empty_title)).set_text(
            cx,
            if is_filtered_empty && is_want_to_read_filter {
                tr("want_to_read_empty")
            } else if is_filtered_empty {
                tr("no_tag_matches")
            } else {
                tr("library_empty")
            },
        );
        self.ui.label(cx, ids!(library_empty_hint)).set_text(
            cx,
            if is_filtered_empty && is_want_to_read_filter {
                tr("want_to_read_empty_hint")
            } else if is_filtered_empty {
                tr("no_tag_matches_hint")
            } else if cfg!(any(target_os = "android", target_os = "ios")) {
                localized(
                    "点击右上角“导入 EPUB”从系统文件选择器中添加书籍",
                    "Choose Import EPUB in the top-right corner to add books from the system picker",
                )
            } else if cfg!(target_os = "macos") {
                localized(
                    "点击右上角“导入 EPUB”，或把 EPUB 拖进窗口",
                    "Choose Import EPUB in the top-right corner, or drop an EPUB here",
                )
            } else {
                tr("library_empty_hint")
            },
        );
        let (show_empty, show_shelf) = library_surface_visibility(visible_count);
        self.ui
            .widget(cx, ids!(library_empty))
            .set_visible(cx, show_empty);
        self.ui
            .widget(cx, ids!(book_shelf))
            .set_visible(cx, show_shelf);
        self.ui.widget(cx, ids!(book_shelf)).redraw(cx);
        self.ui.widget(cx, ids!(page_library)).redraw(cx);
    }

    fn set_library_tag_filter(&mut self, cx: &mut Cx, index: usize) {
        self.close_library_action_menu(cx);
        self.library_tag_filter = match index {
            0 => None,
            1 => Some(WANT_TO_READ_FILTER.to_string()),
            index => Self::available_shelf_tags().get(index - 2).cloned(),
        };
        self.refresh_shelf_projection(cx);
    }

    fn library_action_book(&self) -> Option<ShelfItem> {
        let book_id = self.library_action_book_id.as_deref()?;
        SHELF
            .read()
            .unwrap()
            .iter()
            .find(|book| book.id == book_id)
            .cloned()
    }

    fn position_library_action_menu(&self, cx: &mut Cx, use_measured_height: bool) -> bool {
        let Some(anchor) = self.library_action_anchor else {
            return false;
        };
        let viewport = self
            .ui
            .widget(cx, ids!(library_body))
            .area()
            .clipped_rect(cx);
        if viewport.size.x <= 0.0 || viewport.size.y <= 0.0 {
            return false;
        }
        let menu = self.ui.view(cx, ids!(library_action_menu));
        let measured_height = menu.area().rect(cx).size.y;
        let menu_height = if use_measured_height && measured_height > 0.0 {
            measured_height
        } else {
            LIBRARY_ACTION_MENU_ESTIMATED_HEIGHT
        };
        let (position, width) = library_action_menu_geometry(viewport, anchor, menu_height);
        let Some(mut inner) = menu.borrow_mut() else {
            return false;
        };
        let old_position = inner.walk.abs_pos;
        let old_width = match inner.walk.width {
            Size::Fixed(value) => value,
            _ => -1.0,
        };
        let changed = old_position
            .map(|old| (old.x - position.x).abs() > 0.5 || (old.y - position.y).abs() > 0.5)
            .unwrap_or(true)
            || (old_width - width).abs() > 0.5;
        inner.walk.abs_pos = Some(position);
        inner.walk.width = Size::Fixed(width);
        changed
    }

    fn open_library_action_menu(&mut self, cx: &mut Cx, book: &ShelfItem, anchor: Rect) {
        self.library_action_book_id = Some(book.id.clone());
        self.library_action_anchor = Some(anchor);
        self.ui.widget(cx, ids!(library_want_to_read)).set_text(
            cx,
            tr(if book.want_to_read {
                "remove_want_to_read"
            } else {
                "add_want_to_read"
            }),
        );
        self.ui
            .widget(cx, ids!(library_action_dismiss))
            .set_visible(cx, true);
        self.ui
            .widget(cx, ids!(library_action_menu))
            .set_visible(cx, true);
        self.position_library_action_menu(cx, false);
        self.pending_library_action_layout = true;
        self.ui.widget(cx, ids!(library_action_layer)).redraw(cx);
    }

    fn close_library_action_menu(&mut self, cx: &mut Cx) {
        self.library_action_book_id = None;
        self.library_action_anchor = None;
        self.pending_library_action_layout = false;
        self.ui
            .widget(cx, ids!(library_action_dismiss))
            .set_visible(cx, false);
        self.ui
            .widget(cx, ids!(library_action_menu))
            .set_visible(cx, false);
        self.ui.widget(cx, ids!(library_action_layer)).redraw(cx);
    }

    fn open_book_for_reading(&mut self, cx: &mut Cx, book: ShelfItem) {
        self.close_library_action_menu(cx);
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

    fn open_library_rename_editor(&mut self, cx: &mut Cx, book: &ShelfItem) {
        self.close_library_action_menu(cx);
        self.editing_book_title_id = Some(book.id.clone());
        self.ui
            .text_input(cx, ids!(library_rename_input))
            .set_text(cx, &book.title);
        self.ui
            .label(cx, ids!(library_rename_status))
            .set_text(cx, "");
        self.ui.modal(cx, ids!(library_rename_modal)).open(cx);
    }

    fn save_library_rename(&mut self, cx: &mut Cx) {
        let Some(book_id) = self.editing_book_title_id.clone() else {
            return;
        };
        let title = self.ui.text_input(cx, ids!(library_rename_input)).text();
        if let Err(error) = readwise_core::books::normalize_book_title(&title) {
            self.ui.label(cx, ids!(library_rename_status)).set_text(
                cx,
                &if app_language() == AppLanguage::English {
                    "Enter a title between 1 and 300 characters.".to_string()
                } else {
                    error
                },
            );
            return;
        }
        self.ui
            .label(cx, ids!(library_rename_status))
            .set_text(cx, localized("正在保存书名…", "Saving title…"));
        spawn_rename_book(book_id, title);
    }

    fn open_library_tag_editor(&mut self, cx: &mut Cx, book: &ShelfItem) {
        self.close_library_action_menu(cx);
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
        self.close_library_action_menu(cx);
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

    fn handle_library_book_card_actions(
        &mut self,
        cx: &mut Cx,
        actions: &Actions,
        card: &WidgetRef,
        book: &ShelfItem,
        library_menu_was_open: bool,
    ) {
        let more_button = card.button(cx, ids!(more_btn));
        if more_button.clicked(actions) {
            if !library_menu_was_open {
                self.open_library_action_menu(cx, book, more_button.area().clipped_rect(cx));
            }
            return;
        }
        if let Some(action) = actions.find_widget_action(card.widget_uid()) {
            if let ViewAction::FingerUp(event) = action.cast() {
                if library_card_release_opens_reader(library_menu_was_open, event.is_over) {
                    self.open_book_for_reading(cx, book.clone());
                }
            }
        }
    }

    fn open_reader_image_viewer(&mut self, cx: &mut Cx, path: &str) {
        self.clear_reader_selection(cx);
        let image = self.ui.image(cx, ids!(image_viewer_image));
        if let Err(error) = image.load_image_file_by_path(cx, std::path::Path::new(path)) {
            ::log::error!("原图加载失败 {path}: {error:?}");
            self.ui.label(cx, ids!(status)).set_text(
                cx,
                localized("原图加载失败", "Failed to load the original image"),
            );
            return;
        }
        let info = image
            .size_in_pixels(cx)
            .map(|(width, height)| {
                if app_language() == AppLanguage::English {
                    format!("Original image · {width} × {height} px")
                } else {
                    format!("原图 · {width} × {height} 像素")
                }
            })
            .unwrap_or_else(|| localized("原图", "Original image").to_string());
        self.ui
            .label(cx, ids!(image_viewer_info))
            .set_text(cx, &info);
        self.apply_reader_background(cx, *READER_BACKGROUND.read().unwrap());
        self.ui.modal(cx, ids!(reader_image_modal)).open(cx);
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

    fn sync_reader_background_controls(&self, cx: &mut Cx, background: ReaderBackground) {
        for (id, option) in [
            (id!(reader_background_paper), ReaderBackground::Paper),
            (id!(reader_background_sepia), ReaderBackground::Sepia),
            (id!(reader_background_sage), ReaderBackground::Sage),
            (id!(reader_background_night), ReaderBackground::Night),
            (id!(reader_background_theme), ReaderBackground::FollowTheme),
        ] {
            self.ui
                .radio_button(cx, &[id])
                .set_active(cx, background == option, Animate::No);
        }
    }

    fn apply_reader_background(&self, cx: &mut Cx, background: ReaderBackground) {
        let palette = reader_scene_palette(background, *READER_THEME.read().unwrap());

        for id in [
            id!(main_column),
            id!(page_reader),
            id!(reader_body),
            id!(reader_surface),
        ] {
            apply_reader_surface_palette(
                cx,
                self.ui.widget(cx, &[id]),
                palette.surface,
                palette.surface,
            );
        }
        apply_reader_surface_palette(
            cx,
            self.ui.widget(cx, ids!(header)),
            palette.surface,
            palette.border,
        );
        for id in [id!(toc_panel), id!(note_panel), id!(reader_ai_panel)] {
            apply_reader_surface_palette(
                cx,
                self.ui.widget(cx, &[id]),
                palette.surface,
                palette.border,
            );
        }
        for id in [
            id!(note_composer),
            id!(word_card),
            id!(indicator_bg),
            id!(library_rename_card),
        ] {
            apply_reader_surface_palette(
                cx,
                self.ui.widget(cx, &[id]),
                palette.surface,
                palette.border,
            );
        }
        // 临时菜单不能与页面共用 surface；按钮再回到 surface，
        // 形成“正文 → field 浮层 → surface 操作”的稳定层级。
        for id in [id!(selection_popup), id!(library_action_menu)] {
            apply_reader_surface_palette(
                cx,
                self.ui.widget(cx, &[id]),
                palette.field,
                palette.border,
            );
        }
        apply_reader_surface_palette(
            cx,
            self.ui.widget(cx, ids!(image_viewer_card)),
            palette.surface,
            palette.border,
        );

        for id in [
            id!(title),
            id!(status),
            id!(note_panel_title),
            id!(note_quote),
            id!(note_empty_title),
            id!(reader_ai_title),
            id!(card_word),
            id!(image_viewer_info),
            id!(library_rename_title),
        ] {
            apply_reader_label_palette(cx, self.ui.widget(cx, &[id]), palette.text);
        }
        for id in [
            id!(chapter_pos),
            id!(page_text),
            id!(reader_pages_left),
            id!(note_hint),
            id!(note_empty_hint),
            id!(reader_chat_status),
            id!(sel_hint),
            id!(card_hint),
            id!(library_rename_status),
        ] {
            apply_reader_label_palette(cx, self.ui.widget(cx, &[id]), palette.muted);
        }
        for id in [id!(note_quote_title), id!(card_gloss)] {
            apply_reader_label_palette(cx, self.ui.widget(cx, &[id]), palette.accent);
        }

        for id in [
            id!(toc_btn),
            id!(notes_btn),
            id!(reader_ai_btn),
            id!(note_close_btn),
            id!(note_cancel_btn),
            id!(note_save_btn),
            id!(reader_ai_close),
            id!(reader_chat_send),
            id!(reader_chat_cancel),
            id!(reader_chat_clear),
            id!(close_card_btn),
            id!(add_vocab_btn),
            id!(know_btn),
            id!(image_viewer_close),
            id!(library_rename_cancel),
            id!(library_rename_save),
        ] {
            apply_reader_button_palette(cx, self.ui.widget(cx, &[id]), palette);
        }
        let popup_button_palette = ReaderScenePalette {
            field: palette.surface,
            ..palette
        };
        for id in [
            id!(copy_sel_btn),
            id!(note_from_sel_btn),
            id!(vocab_from_sel_btn),
            id!(translate_sel_btn),
            id!(ask_ai_sel_btn),
            id!(speak_sel_btn),
            id!(hl_btn),
            id!(sel_close_btn),
            id!(library_share_book),
            id!(library_want_to_read),
            id!(library_add_collection),
            id!(library_mark_finished),
            id!(library_rename_book),
            id!(library_delete_book),
        ] {
            apply_reader_button_palette(cx, self.ui.widget(cx, &[id]), popup_button_palette);
        }
        let destructive = if background == ReaderBackground::Night
            || (background == ReaderBackground::FollowTheme
                && *READER_THEME.read().unwrap() == ReaderTheme::Dark)
        {
            Vec4f::from_u32(0xff8a_84ff)
        } else {
            Vec4f::from_u32(0xb43c_36ff)
        };
        let mut delete_button = self.ui.widget(cx, ids!(library_delete_book));
        script_apply_eval!(cx, delete_button, {
            draw_text +: {
                color: #(destructive)
                color_hover: #(destructive)
                color_down: #(destructive)
                color_focus: #(destructive)
            }
        });
        for id in [
            id!(note_extra),
            id!(reader_chat_input),
            id!(library_rename_input),
        ] {
            apply_reader_input_palette(cx, self.ui.widget(cx, &[id]), palette);
        }

        if let Some(mut toc) = self.ui.widget(cx, ids!(toc_list)).borrow_mut::<TocList>() {
            toc.active_bg = palette.field;
            toc.inactive_bg = palette.surface;
            toc.text_color = palette.text;
            toc.button_hover = palette.hover;
            toc.button_pressed = palette.pressed;
        }
        self.apply_navigation_palette(cx, self.active_page);
        self.ui.widget(cx, ids!(main_column)).redraw(cx);
        self.ui.widget(cx, ids!(page_reader)).redraw(cx);
    }

    fn set_reader_background(&mut self, cx: &mut Cx, background: ReaderBackground) {
        if *READER_BACKGROUND.read().unwrap() == background {
            self.sync_reader_background_controls(cx, background);
            self.apply_reader_background(cx, background);
            return;
        }
        *READER_BACKGROUND.write().unwrap() = background;
        self.sync_reader_background_controls(cx, background);
        self.apply_reader_background(cx, background);
        self.persist_reader_settings(cx);
        self.apply_runtime_theme(cx, *READER_THEME.read().unwrap());
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
        set_text!(library_view_list, "list_view");
        set_text!(library_view_grid, "grid_view");
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
        set_text!(library_share_book, "share_book");
        let want_to_read_key = if self
            .library_action_book()
            .is_some_and(|book| book.want_to_read)
        {
            "remove_want_to_read"
        } else {
            "add_want_to_read"
        };
        self.ui
            .widget(cx, ids!(library_want_to_read))
            .set_text(cx, crate::i18n::text(language, want_to_read_key));
        set_text!(library_add_collection, "add_to_collection");
        set_text!(library_mark_finished, "mark_finished");
        set_text!(library_rename_book, "rename_book");
        set_text!(library_delete_book, "delete");
        set_text!(library_rename_title, "rename_book");
        set_text!(library_rename_cancel, "cancel");
        set_text!(library_rename_save, "save");
        set_empty_text!(library_rename_input, "rename_book_placeholder");
        set_text!(library_delete_modal_title, "delete_book_title");
        set_text!(library_delete_cancel, "cancel");
        set_text!(library_delete_confirm, "delete_book_confirm");
        set_text!(image_viewer_close, "close");

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
        let highlight_key = if self.selected_highlight_ids().is_empty() {
            "highlight"
        } else {
            "remove_highlight"
        };
        self.ui
            .widget(cx, ids!(hl_btn))
            .set_text(cx, crate::i18n::text(language, highlight_key));
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
        set_text!(reader_background_label, "reader_background");
        set_text!(reader_background_paper, "background_paper");
        set_text!(reader_background_sepia, "background_sepia");
        set_text!(reader_background_sage, "background_sage");
        set_text!(reader_background_night, "background_night");
        set_text!(reader_background_theme, "background_theme");
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
        self.apply_reader_background(cx, *READER_BACKGROUND.read().unwrap());
        self.persist_reader_settings(cx);
        self.apply_runtime_theme(cx, theme);
    }

    fn apply_runtime_theme(&mut self, cx: &mut Cx, _theme: ReaderTheme) {
        // script_mod 会依据 READER_BACKGROUND + READER_THEME 重建完整场景主题。
        // 不能只改页面底色，否则模板化卡片、输入框和按钮仍会保留灰色 token。
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

    fn sync_reader_panel_layout(&mut self, cx: &mut Cx) {
        let panel_open = self.toc_open || self.note_open || self.reader_ai_open;
        if let Some(mut chapter_list) = self
            .ui
            .widget(cx, ids!(chapter_list))
            .borrow_mut::<ChapterList>()
        {
            chapter_list.set_compact_layout(cx, self.compact_layout);
        }
        self.ui
            .widget(cx, ids!(reader_surface))
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
        self.invalidate_reader_pagination_layout(cx);
    }

    fn update_responsive_layout(
        &mut self,
        cx: &mut Cx,
        size: DVec2,
        safe_area_insets: SafeAreaInsets,
    ) -> bool {
        if size.x <= 0.0 || size.y <= 0.0 {
            return false;
        }
        let compact = uses_compact_layout(size);
        let layout_changed = !self.responsive_layout_initialized || compact != self.compact_layout;
        let safe_area_changed = safe_area_insets != self.safe_area_insets;
        if !layout_changed && !safe_area_changed {
            return false;
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
        true
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
        self.flush_reader_progress(cx);
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

    fn current_reader_progress_snapshot(&self, cx: &mut Cx) -> Option<ReaderProgressSnapshot> {
        let (book_id, chapter_count) = {
            let guard = CURRENT_BOOK.read().unwrap();
            let book = guard.as_ref()?;
            (book.id.clone(), book.chapters.len())
        };
        let list = self
            .ui
            .widget(cx, ids!(chapter_list))
            .portal_list(cx, ids!(list));
        let block_count = PARAGRAPHS.read().unwrap().len();
        let block = list.first_id().min(block_count.saturating_sub(1));
        let scroll = list.scroll_position();
        let scroll = if scroll.is_finite() { scroll } else { 0.0 };
        CURRENT_READER_BLOCK.store(block, Ordering::Relaxed);
        CURRENT_READER_SCROLL_BITS.store(scroll.to_bits(), Ordering::Relaxed);
        Some(ReaderProgressSnapshot {
            book_id,
            chapter_count,
            location: ReaderLocation {
                chapter: self.chapter_idx.min(chapter_count.saturating_sub(1)),
                block,
                scroll,
            },
        })
    }

    fn queue_reader_progress_save(&self, cx: &mut Cx) {
        if let Some(snapshot) = self.current_reader_progress_snapshot(cx) {
            spawn_save_progress(snapshot);
        }
    }

    fn debounce_reader_progress_save(&mut self, cx: &mut Cx) {
        cx.stop_timer(self.reading_progress_save_timer);
        self.reading_progress_save_timer = cx.start_timeout(0.75);
    }

    fn flush_reader_progress(&mut self, cx: &mut Cx) {
        cx.stop_timer(self.reading_progress_save_timer);
        self.reading_progress_save_timer = Timer::default();
        if let Some(snapshot) = self.current_reader_progress_snapshot(cx) {
            save_progress_now(snapshot);
        }
    }

    /// 切换到指定章节并恢复章内位置。普通章节跳转传入章首并立即保存；
    /// 从数据库/进程状态恢复时不先写入章首，避免覆盖尚未应用的旧位置。
    fn set_chapter_at(
        &mut self,
        cx: &mut Cx,
        idx: usize,
        block: usize,
        scroll: f64,
        persist: bool,
    ) {
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
        let chapter_blocks = chunks.len().max(1);
        let restored_block = if block == usize::MAX {
            reader_last_page_start(chunks.len(), self.reader_page_span)
        } else {
            block.min(chunks.len().saturating_sub(1))
        };
        let restored_scroll = if scroll.is_finite() { scroll } else { 0.0 };
        self.clear_reader_selection(cx);
        self.close_word_card(cx);
        self.close_note_composer(cx);
        self.chapter_idx = idx;
        CURRENT_CHAPTER.store(idx, std::sync::atomic::Ordering::Relaxed);
        CURRENT_READER_BLOCK.store(restored_block, Ordering::Relaxed);
        CURRENT_READER_SCROLL_BITS.store(restored_scroll.to_bits(), Ordering::Relaxed);
        replace_paragraphs(chunks);
        if self.reader_chapter_block_counts.len() != total {
            self.reader_chapter_block_counts = vec![0; total];
        }
        self.reader_chapter_block_counts[idx] = chapter_blocks;
        self.reader_page_span = 0;
        self.pending_reader_pagination_layout = true;
        self.ui
            .label(cx, ids!(title))
            .set_text(cx, &format!("《{}》", title));
        self.ui
            .label(cx, ids!(chapter_pos))
            .set_text(cx, &format!("{}/{}", idx + 1, total));
        self.ui
            .widget(cx, ids!(chapter_list))
            .portal_list(cx, ids!(list))
            .set_first_id_and_scroll(restored_block, restored_scroll);
        if persist {
            spawn_save_progress(ReaderProgressSnapshot {
                book_id: id,
                chapter_count: total,
                location: ReaderLocation {
                    chapter: idx,
                    block: restored_block,
                    scroll: restored_scroll,
                },
            });
        }
        self.update_reader_page_footer(cx);
        self.ui.redraw(cx);
    }

    fn set_chapter(&mut self, cx: &mut Cx, idx: usize) {
        self.set_chapter_at(cx, idx, 0, 0.0, true);
    }

    /// Re-project the process-lifetime reader state into a freshly-created
    /// widget tree. This is the fallback for Android vendors that recreate the
    /// Activity despite the manifest configuration-change declaration.
    fn restore_reader_after_rebuild(&mut self, cx: &mut Cx) {
        if CURRENT_BOOK.read().unwrap().is_none() {
            return;
        }
        let chapter = CURRENT_CHAPTER.load(Ordering::Relaxed);
        let first_block = CURRENT_READER_BLOCK.load(Ordering::Relaxed);
        let first_scroll = f64::from_bits(CURRENT_READER_SCROLL_BITS.load(Ordering::Relaxed));
        self.set_chapter_at(cx, chapter, first_block, first_scroll, false);
        self.update_progress_label(cx);
    }

    fn update_reader_page_footer(&mut self, cx: &mut Cx) {
        let first_block = self
            .ui
            .widget(cx, ids!(chapter_list))
            .portal_list(cx, ids!(list))
            .first_id();
        let summary = (self.reader_page_span > 0)
            .then(|| {
                reader_page_summary(
                    &self.reader_chapter_block_counts,
                    self.chapter_idx,
                    first_block,
                    self.reader_page_span,
                )
            })
            .flatten();
        if let Some(mut indicator) = self
            .ui
            .widget(cx, ids!(reader_page_indicator))
            .borrow_mut::<ReaderPageIndicator>()
        {
            match summary {
                Some(summary) => indicator.set_pages(cx, summary.current, summary.total),
                None => indicator.set_pages(cx, 0, 0),
            }
        }
        let pages_left = match summary {
            Some(summary) if app_language() == AppLanguage::English => {
                if summary.pages_left_in_chapter == 1 {
                    "1 page left in chapter".to_string()
                } else {
                    format!("{} pages left in chapter", summary.pages_left_in_chapter)
                }
            }
            Some(summary) => format!("本章还剩 {} 页", summary.pages_left_in_chapter),
            None if app_language() == AppLanguage::English => "Calculating pages…".to_string(),
            None => "正在计算页码…".to_string(),
        };
        self.ui
            .label(cx, ids!(reader_pages_left))
            .set_text(cx, &pages_left);
    }

    fn invalidate_reader_pagination_layout(&mut self, cx: &mut Cx) {
        self.pending_reader_pagination_layout = true;
        self.ui.redraw(cx);
    }

    /// 必须在一次完整 Draw 后读取 PortalList 的真实可见项目数；提前读取会得到旧布局，
    /// 导致字号或横竖屏变化后页码与翻页步长不一致。
    fn resolve_reader_pagination_layout_after_draw(&mut self, cx: &mut Cx) {
        if !self.pending_reader_pagination_layout || CURRENT_BOOK.read().unwrap().is_none() {
            return;
        }
        let visible = self
            .ui
            .widget(cx, ids!(chapter_list))
            .portal_list(cx, ids!(list))
            .visible_items();
        if visible == 0 {
            return;
        }
        self.reader_page_span = visible.saturating_sub(1).max(1);
        self.pending_reader_pagination_layout = false;
        self.update_reader_page_footer(cx);
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
        CURRENT_READER_BLOCK.store(list.first_id(), Ordering::Relaxed);
        CURRENT_READER_SCROLL_BITS.store(list.scroll_position().to_bits(), Ordering::Relaxed);
        let blocks = PARAGRAPHS.read().unwrap().len().max(1);
        let read_to = (list.first_id() + list.visible_items()).min(blocks);
        let pct = read_to * 100 / blocks;
        let text = format!("{}/{} · {}%", self.chapter_idx + 1, total_ch, pct);
        self.ui.label(cx, ids!(chapter_pos)).set_text(cx, &text);
        self.update_reader_page_footer(cx);
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
        let step = if self.reader_page_span > 0 {
            self.reader_page_span
        } else {
            vis.saturating_sub(1).max(1)
        };
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
                self.set_chapter_at(cx, idx, usize::MAX, 0.0, true);
            }
        } else {
            list.set_first_id_and_scroll(first.saturating_sub(step), 0.0);
            self.ui.redraw(cx);
        }
        self.update_progress_label(cx);
        self.debounce_reader_progress_save(cx);
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
                self.debounce_reader_progress_save(cx);
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
        self.invalidate_reader_pagination_layout(cx);
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
        let scene_changed = *READER_THEME.read().unwrap() != settings.theme
            || *READER_BACKGROUND.read().unwrap() != settings.background;
        *APP_LANGUAGE.write().unwrap() = settings.language;
        *LIBRARY_VIEW_MODE.write().unwrap() = settings.library_view_mode;
        *READER_THEME.write().unwrap() = settings.theme;
        *READER_BACKGROUND.write().unwrap() = settings.background;
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
        self.sync_library_view_controls(cx, settings.library_view_mode);
        self.sync_reader_background_controls(cx, settings.background);
        self.apply_reader_background(cx, settings.background);
        self.sync_app_language_controls(cx, settings.language);
        self.apply_ui_language(cx, settings.language);
        self.sync_reader_font_controls(cx, settings.font);
        self.sync_reader_spacing_controls(cx, settings.spacing);
        self.set_gloss_rows_visible(cx, settings.gloss_enabled);
        self.refresh_swatches(cx);

        self.refresh_current_chapter(cx);
        self.ui.widget(cx, ids!(book_shelf)).redraw(cx);
        if scene_changed {
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
            let chapter_blocks = chunks.len().max(1);
            replace_paragraphs(chunks);
            if let Some(count) = self.reader_chapter_block_counts.get_mut(self.chapter_idx) {
                *count = chapter_blocks;
            }
        }
        self.invalidate_reader_pagination_layout(cx);
        self.ui.redraw(cx);
    }

    fn selected_highlight_ids(&self) -> Vec<String> {
        let Some(range) = *SEL_RANGE.read().unwrap() else {
            return Vec::new();
        };
        CURRENT_BOOK
            .read()
            .unwrap()
            .as_ref()
            .and_then(|book| book.highlights.get(&self.chapter_idx))
            .map(|highlights| overlapping_highlight_ids(highlights, range))
            .unwrap_or_default()
    }

    fn update_highlight_action_label(&self, cx: &mut Cx) {
        let key = if self.selected_highlight_ids().is_empty() {
            "highlight"
        } else {
            "remove_highlight"
        };
        self.ui
            .widget(cx, ids!(hl_btn))
            .set_text(cx, crate::i18n::text(app_language(), key));
    }

    fn reset_selection_action_hover(&self, cx: &mut Cx) {
        self.ui.button(cx, ids!(copy_sel_btn)).reset_hover(cx);
        self.ui.button(cx, ids!(note_from_sel_btn)).reset_hover(cx);
        self.ui.button(cx, ids!(vocab_from_sel_btn)).reset_hover(cx);
        self.ui.button(cx, ids!(translate_sel_btn)).reset_hover(cx);
        self.ui.button(cx, ids!(ask_ai_sel_btn)).reset_hover(cx);
        self.ui.button(cx, ids!(speak_sel_btn)).reset_hover(cx);
        self.ui.button(cx, ids!(hl_btn)).reset_hover(cx);
        self.ui.button(cx, ids!(sel_close_btn)).reset_hover(cx);
    }

    fn set_selection_hint(&mut self, cx: &mut Cx, text: &str) {
        let hint = self.ui.widget(cx, ids!(sel_hint));
        hint.set_visible(cx, !text.is_empty());
        hint.set_text(cx, text);
        if self.selection_popup_anchor.is_some() {
            self.pending_selection_popup_layout = true;
        }
        hint.redraw(cx);
    }

    /// 将工具条锚定在选区手势结束处，并严格限制在正文视口内。
    /// 第一次使用估算高度立即显示；完成一帧布局后再用真实高度校正一次。
    fn position_selection_popup(&self, cx: &mut Cx, use_measured_height: bool) -> bool {
        let Some(anchor) = self.selection_popup_anchor else {
            return false;
        };
        let viewport = self
            .ui
            .widget(cx, ids!(chapter_list))
            .area()
            .clipped_rect(cx);
        if viewport.size.x <= 0.0 || viewport.size.y <= 0.0 {
            return false;
        }
        let popup = self.ui.view(cx, ids!(selection_popup));
        let measured_height = popup.area().rect(cx).size.y;
        let popup_height = if use_measured_height && measured_height > 0.0 {
            measured_height
        } else {
            SELECTION_POPUP_ESTIMATED_HEIGHT
        };
        let (position, width) = selection_popup_geometry(viewport, anchor, popup_height);
        let Some(mut inner) = popup.borrow_mut() else {
            return false;
        };
        let old_position = inner.walk.abs_pos;
        let old_width = match inner.walk.width {
            Size::Fixed(value) => value,
            _ => -1.0,
        };
        let changed = old_position
            .map(|old| (old.x - position.x).abs() > 0.5 || (old.y - position.y).abs() > 0.5)
            .unwrap_or(true)
            || (old_width - width).abs() > 0.5;
        inner.walk.abs_pos = Some(position);
        inner.walk.width = Size::Fixed(width);
        changed
    }

    /// 将生词卡片锚定到被点击的注音词附近，并限制在正文视口内。
    /// 首帧使用估算高度，绘制完成后再按卡片真实高度校正，避免闪到阅读器底部。
    fn position_word_card(&self, cx: &mut Cx, use_measured_height: bool) -> bool {
        let Some(anchor) = self.word_card_anchor else {
            return false;
        };
        let viewport = self
            .ui
            .widget(cx, ids!(chapter_list))
            .area()
            .clipped_rect(cx);
        if viewport.size.x <= 0.0 || viewport.size.y <= 0.0 {
            return false;
        }
        let card = self.ui.view(cx, ids!(word_card));
        let measured_height = card.area().rect(cx).size.y;
        let card_height = if use_measured_height && measured_height > 0.0 {
            measured_height
        } else {
            WORD_CARD_ESTIMATED_HEIGHT
        };
        let (position, width) = word_card_geometry(viewport, anchor, card_height);
        let Some(mut inner) = card.borrow_mut() else {
            return false;
        };
        let old_position = inner.walk.abs_pos;
        let old_width = match inner.walk.width {
            Size::Fixed(value) => value,
            _ => -1.0,
        };
        let changed = old_position
            .map(|old| (old.x - position.x).abs() > 0.5 || (old.y - position.y).abs() > 0.5)
            .unwrap_or(true)
            || (old_width - width).abs() > 0.5;
        inner.walk.abs_pos = Some(position);
        inner.walk.width = Size::Fixed(width);
        changed
    }

    fn close_word_card(&mut self, cx: &mut Cx) {
        self.card = None;
        self.word_card_anchor = None;
        self.pending_word_card_layout = false;
        self.ui.widget(cx, ids!(word_card)).set_visible(cx, false);
        self.ui.widget(cx, ids!(word_card_layer)).redraw(cx);
    }

    fn clear_reader_selection(&mut self, cx: &mut Cx) {
        *SEL_RANGE.write().unwrap() = None;
        SELECTED_TEXT.write().unwrap().clear();
        self.selection_popup_anchor = None;
        self.pending_selection_popup_layout = false;
        self.reset_selection_action_hover(cx);
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
        self.reset_selection_action_hover(cx);
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
    fn handle_startup(&mut self, cx: &mut Cx) {
        self.app_in_foreground = true;
        let has_current_book = CURRENT_BOOK.read().unwrap().is_some();
        let page =
            restored_page_after_rebuild(ACTIVE_PAGE.load(Ordering::Relaxed), has_current_book);
        self.activate_page(cx, page);
        self.refresh_shelf_projection(cx);
        if has_current_book {
            self.restore_reader_after_rebuild(cx);
            if page == AppPage::Reader {
                self.start_reading_timer(cx);
            }
        }
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
        if self.reading_progress_save_timer.is_timer(event).is_some() {
            self.reading_progress_save_timer = Timer::default();
            self.queue_reader_progress_save(cx);
        }
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

    fn handle_shutdown(&mut self, cx: &mut Cx) {
        self.flush_reader_progress(cx);
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
        // A menu action can close the overlay before this action batch reaches
        // the shelf-card loop. Preserve the state from the start of the batch
        // so that the same pointer release can never open a book underneath.
        let library_action_menu_was_open = self.library_action_book_id.is_some();
        let chapter_list = self
            .ui
            .widget(cx, ids!(chapter_list))
            .portal_list(cx, ids!(list));
        if self.on_reader_page
            && CURRENT_BOOK.read().unwrap().is_some()
            && chapter_list.scrolled(actions)
        {
            self.close_word_card(cx);
            self.update_progress_label(cx);
            self.debounce_reader_progress_save(cx);
        }
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
            #[cfg(target_os = "android")]
            if let Some(import_action) =
                action.downcast_ref::<crate::android_import::AndroidImportAction>()
            {
                match import_action {
                    crate::android_import::AndroidImportAction::Picked(paths) => {
                        self.ui.label(cx, ids!(library_hint)).set_text(
                            cx,
                            localized("正在导入所选 EPUB…", "Importing selected EPUB files…"),
                        );
                        spawn_import(paths.clone(), true);
                    }
                    crate::android_import::AndroidImportAction::Cancelled => {
                        self.ui
                            .label(cx, ids!(library_hint))
                            .set_text(cx, localized("已取消导入", "Import cancelled"));
                    }
                    crate::android_import::AndroidImportAction::Error(error) => {
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
                        self.project_db_ready_status(cx, *vocab_count, *known_count);
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
                        // Restore the thread at its tail before the chat page's
                        // first visible draw instead of animating from item zero.
                        self.redraw_chat(cx);
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
                        if self.word_card_anchor.is_some() {
                            self.pending_word_card_layout = true;
                        }
                        self.set_selection_hint(cx, hint);
                        if *added {
                            spawn_load_vocab();
                        }
                    }
                    CardAction::MarkedKnown => {
                        self.close_word_card(cx);
                        self.refresh_current_chapter(cx);
                    }
                }
                continue;
            }
            if let Some(sel) = action.downcast_ref::<SelAction>() {
                match sel {
                    SelAction::Show { anchor } => {
                        let viewport = self
                            .ui
                            .widget(cx, ids!(chapter_list))
                            .area()
                            .clipped_rect(cx);
                        self.selection_popup_anchor =
                            Some(anchor.unwrap_or_else(|| viewport.pos + viewport.size * 0.5));
                        self.reset_selection_action_hover(cx);
                        self.ui.widget(cx, ids!(sel_bar)).set_visible(cx, true);
                        self.position_selection_popup(cx, false);
                        self.pending_selection_popup_layout = true;
                        self.update_highlight_action_label(cx);
                        self.set_selection_hint(cx, "");
                    }
                    SelAction::Hide => {
                        self.selection_popup_anchor = None;
                        self.pending_selection_popup_layout = false;
                        self.reset_selection_action_hover(cx);
                        self.ui.widget(cx, ids!(sel_bar)).set_visible(cx, false);
                    }
                }
                self.ui.redraw(cx);
                continue;
            }
            if let Some(highlight_action) = action.downcast_ref::<HlAction>() {
                match highlight_action {
                    HlAction::Saved(hl, chapter) => {
                        if let Some(book) = CURRENT_BOOK.write().unwrap().as_mut() {
                            book.highlights
                                .entry(*chapter)
                                .or_default()
                                .push(hl.clone());
                        }
                        self.clear_reader_selection(cx);
                        self.refresh_current_chapter(cx);
                    }
                    HlAction::Deleted { chapter, ids } => {
                        let ids = ids.iter().map(String::as_str).collect::<HashSet<_>>();
                        if let Some(book) = CURRENT_BOOK.write().unwrap().as_mut() {
                            let remove_chapter = if let Some(highlights) =
                                book.highlights.get_mut(chapter)
                            {
                                highlights.retain(|highlight| !ids.contains(highlight.id.as_str()));
                                highlights.is_empty()
                            } else {
                                false
                            };
                            if remove_chapter {
                                book.highlights.remove(chapter);
                            }
                        }
                        self.clear_reader_selection(cx);
                        self.refresh_current_chapter(cx);
                    }
                    HlAction::Error(error) => {
                        let message = if app_language() == AppLanguage::English {
                            format!("Highlight operation failed: {error}")
                        } else {
                            format!("高亮操作失败：{error}")
                        };
                        self.set_selection_hint(cx, &message);
                    }
                }
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
                        resume,
                        gloss_count,
                        parse_ms,
                        initial_gloss_ms,
                    } => {
                        ::log::info!(
                            "Opened EPUB: parse={}ms, initial chapter gloss={}ms, lazy remaining chapters",
                            parse_ms,
                            initial_gloss_ms
                        );
                        self.reader_chapter_block_counts.clear();
                        self.reader_page_span = 0;
                        self.set_chapter_at(cx, resume.chapter, resume.block, resume.scroll, false);
                        self.update_progress_label(cx);
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
                            spawn_load_notes(book_id.clone());
                            spawn_build_reader_pagination(book_id);
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
            if let Some(pagination) = action.downcast_ref::<ReaderPaginationAction>() {
                match pagination {
                    ReaderPaginationAction::Ready {
                        book_id,
                        chapter_block_counts,
                    } => {
                        let is_current = CURRENT_BOOK
                            .read()
                            .unwrap()
                            .as_ref()
                            .is_some_and(|book| book.id == *book_id);
                        if is_current {
                            self.reader_chapter_block_counts = chapter_block_counts.clone();
                            if let Some(count) =
                                self.reader_chapter_block_counts.get_mut(self.chapter_idx)
                            {
                                *count = PARAGRAPHS.read().unwrap().len().max(1);
                            }
                            self.update_reader_page_footer(cx);
                        }
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
                        } else if cfg!(any(target_os = "android", target_os = "ios")) {
                            if app_language() == AppLanguage::English {
                                format!("{items} books · Choose Import EPUB to add files")
                            } else {
                                format!("共 {items} 本 · 点击“导入 EPUB”从文件中添加")
                            }
                        } else if cfg!(target_os = "macos")
                            && app_language() == AppLanguage::English
                        {
                            format!("{items} books · Choose Import EPUB or drop an EPUB here")
                        } else if cfg!(target_os = "macos") {
                            format!("共 {items} 本 · 点击“导入 EPUB”或把 EPUB 拖进窗口")
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
                    ShelfAction::Renamed {
                        items,
                        book_id,
                        title,
                    } => {
                        self.editing_book_title_id = None;
                        self.ui.modal(cx, ids!(library_rename_modal)).close(cx);
                        if let Some(item) = CURRENT_ITEM.write().unwrap().as_mut() {
                            if item.id == *book_id {
                                item.title = title.clone();
                            }
                        }
                        if let Some(book) = CURRENT_BOOK.write().unwrap().as_mut() {
                            if book.id == *book_id {
                                book.title = title.clone();
                            }
                        }
                        self.refresh_shelf_projection(cx);
                        self.ui.label(cx, ids!(library_hint)).set_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!("Renamed to “{title}” · {items} books")
                            } else {
                                format!("已重命名为《{title}》 · 共 {items} 本")
                            },
                        );
                    }
                    ShelfAction::WantToReadChanged {
                        items,
                        book_id,
                        enabled,
                    } => {
                        if let Some(item) = CURRENT_ITEM.write().unwrap().as_mut() {
                            if item.id == *book_id {
                                item.want_to_read = *enabled;
                            }
                        }
                        self.refresh_shelf_projection(cx);
                        self.ui.label(cx, ids!(library_hint)).set_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!(
                                    "{} Want to Read · {items} books",
                                    if *enabled { "Added to" } else { "Removed from" }
                                )
                            } else if *enabled {
                                format!("已加入欲读清单 · 共 {items} 本")
                            } else {
                                format!("已移出欲读清单 · 共 {items} 本")
                            },
                        );
                    }
                    ShelfAction::MarkedCompleted {
                        items,
                        book_id,
                        title,
                    } => {
                        if let Some(item) = CURRENT_ITEM.write().unwrap().as_mut() {
                            if item.id == *book_id {
                                item.status = "completed".to_string();
                                if item.progress_total > 0 {
                                    item.progress_current = item.progress_total;
                                }
                            }
                        }
                        self.refresh_shelf_projection(cx);
                        self.ui.label(cx, ids!(library_hint)).set_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!("Marked “{title}” as finished · {items} books")
                            } else {
                                format!("已将《{title}》标记为读完 · 共 {items} 本")
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
                    ShelfAction::UpdateError(e) => {
                        let message = if app_language() == AppLanguage::English {
                            format!("Failed to update book: {e}")
                        } else {
                            format!("更新书籍失败：{e}")
                        };
                        if self.editing_book_title_id.is_some() {
                            self.ui
                                .label(cx, ids!(library_rename_status))
                                .set_text(cx, &message);
                        } else {
                            self.ui.label(cx, ids!(library_hint)).set_text(cx, &message);
                        }
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
            if let ChapterImageAction::Open(path) = action.cast() {
                self.open_reader_image_viewer(cx, &path);
                continue;
            }
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
            if let RubyWordAction::Clicked {
                word,
                gloss,
                anchor,
            } = action.cast()
            {
                let word = word
                    .trim_end_matches(['.', ',', ';', ':', '!', '?'])
                    .to_string();
                let lemma = readwise_core::vocab::lemma::lemmatize(&word);
                self.ui.label(cx, ids!(card_word)).set_text(cx, &word);
                self.ui.label(cx, ids!(card_gloss)).set_text(cx, &gloss);
                self.ui.label(cx, ids!(card_hint)).set_text(cx, "");
                self.ui.widget(cx, ids!(word_card)).set_visible(cx, true);
                self.word_card_anchor = Some(anchor);
                self.position_word_card(cx, false);
                self.pending_word_card_layout = true;
                self.card = Some((word, lemma, gloss));
                self.ui.widget(cx, ids!(word_card_layer)).redraw(cx);
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
        if self
            .ui
            .button(cx, ids!(library_rename_cancel))
            .clicked(actions)
            || self
                .ui
                .modal(cx, ids!(library_rename_modal))
                .dismissed(actions)
        {
            self.editing_book_title_id = None;
            self.ui.modal(cx, ids!(library_rename_modal)).close(cx);
        }
        if self
            .ui
            .button(cx, ids!(library_rename_save))
            .clicked(actions)
            || self
                .ui
                .text_input(cx, ids!(library_rename_input))
                .returned(actions)
                .is_some()
        {
            self.save_library_rename(cx);
        }

        if self
            .ui
            .button(cx, ids!(library_share_book))
            .clicked(actions)
        {
            if let Some(book) = self.library_action_book() {
                let shared = if book.author.trim().is_empty() {
                    book.title
                } else {
                    format!("{} — {}", book.title, book.author)
                };
                cx.copy_to_clipboard(&shared);
                self.close_library_action_menu(cx);
                self.ui.label(cx, ids!(library_hint)).set_text(
                    cx,
                    localized(
                        "书籍信息已复制，可粘贴到其他应用分享",
                        "Book details copied; paste them into another app to share",
                    ),
                );
            }
        }
        if self
            .ui
            .button(cx, ids!(library_want_to_read))
            .clicked(actions)
        {
            if let Some(book) = self.library_action_book() {
                let enabled = !book.want_to_read;
                self.close_library_action_menu(cx);
                self.ui
                    .label(cx, ids!(library_hint))
                    .set_text(cx, localized("正在更新欲读清单…", "Updating Want to Read…"));
                spawn_set_book_want_to_read(book.id, enabled);
            }
        }
        if self
            .ui
            .button(cx, ids!(library_add_collection))
            .clicked(actions)
        {
            if let Some(book) = self.library_action_book() {
                self.open_library_tag_editor(cx, &book);
            }
        }
        if self
            .ui
            .button(cx, ids!(library_mark_finished))
            .clicked(actions)
        {
            if let Some(book) = self.library_action_book() {
                self.close_library_action_menu(cx);
                self.ui
                    .label(cx, ids!(library_hint))
                    .set_text(cx, localized("正在标记为已读完…", "Marking as finished…"));
                spawn_mark_book_completed(book.id, book.title);
            }
        }
        if self
            .ui
            .button(cx, ids!(library_rename_book))
            .clicked(actions)
        {
            if let Some(book) = self.library_action_book() {
                self.open_library_rename_editor(cx, &book);
            }
        }
        if self
            .ui
            .button(cx, ids!(library_delete_book))
            .clicked(actions)
        {
            if let Some(book) = self.library_action_book() {
                self.open_delete_book_confirmation(cx, &book);
            }
        }
        if self
            .ui
            .view(cx, ids!(library_action_dismiss))
            .finger_up(actions)
            .is_some()
        {
            self.close_library_action_menu(cx);
        }
        if self
            .ui
            .button(cx, ids!(image_viewer_close))
            .clicked(actions)
            || self
                .ui
                .modal(cx, ids!(reader_image_modal))
                .dismissed(actions)
        {
            self.ui.modal(cx, ids!(reader_image_modal)).close(cx);
        }

        if let Some(index) = self
            .ui
            .radio_button_set(cx, ids_array!(library_view_list, library_view_grid))
            .selected(cx, actions)
        {
            self.set_library_view_mode(
                cx,
                if index == 0 {
                    LibraryViewMode::List
                } else {
                    LibraryViewMode::Grid
                },
            );
        }

        let shelf_view_mode = *LIBRARY_VIEW_MODE.read().unwrap();
        let grid_columns = self
            .ui
            .widget(cx, ids!(book_shelf))
            .borrow::<BookShelf>()
            .map(|shelf| shelf.grid_columns())
            .unwrap_or_else(|| shelf_grid_columns_for_width(cx.display_context.screen_size.x));
        let shelf_list = self
            .ui
            .widget(cx, ids!(book_shelf))
            .portal_list(cx, ids!(list));
        for (item_id, item) in shelf_list.items_with_actions(actions) {
            if shelf_view_mode == LibraryViewMode::Grid {
                for (column, cell_id) in [
                    id!(grid_cell_0),
                    id!(grid_cell_1),
                    id!(grid_cell_2),
                    id!(grid_cell_3),
                ]
                .into_iter()
                .enumerate()
                .take(grid_columns)
                {
                    let book = {
                        let shelf = SHELF_VISIBLE.read().unwrap();
                        shelf_grid_book_index(item_id, column, grid_columns, shelf.entries.len())
                            .and_then(|book_index| shelf.entry_for_item_id(book_index).cloned())
                    };
                    let Some(book) = book else { continue };
                    let card = item.widget(cx, &[cell_id, id!(grid_card)]);
                    self.handle_library_book_card_actions(
                        cx,
                        actions,
                        &card,
                        &book,
                        library_action_menu_was_open,
                    );
                }
            } else {
                let book = SHELF_VISIBLE
                    .read()
                    .unwrap()
                    .entry_for_item_id(item_id)
                    .cloned();
                if let Some(book) = book {
                    self.handle_library_book_card_actions(
                        cx,
                        actions,
                        &item,
                        &book,
                        library_action_menu_was_open,
                    );
                }
            }
        }
        if self.ui.button(cx, ids!(import_epub_btn)).clicked(actions) {
            #[cfg(target_os = "macos")]
            match crate::desktop_import::open_epub_picker(
                localized("导入 EPUB", "Import EPUB"),
                localized("导入", "Import"),
            ) {
                Ok(Some(paths)) => {
                    self.ui.label(cx, ids!(library_hint)).set_text(
                        cx,
                        localized("正在导入所选 EPUB…", "Importing selected EPUB files…"),
                    );
                    spawn_import(paths, false);
                }
                Ok(None) => self
                    .ui
                    .label(cx, ids!(library_hint))
                    .set_text(cx, localized("已取消导入", "Import cancelled")),
                Err(error) => self.ui.label(cx, ids!(library_hint)).set_text(
                    cx,
                    &if app_language() == AppLanguage::English {
                        format!("Could not open the file picker: {error}")
                    } else {
                        format!("无法打开文件选择器：{error}")
                    },
                ),
            }
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
            #[cfg(target_os = "android")]
            match crate::android_import::open_epub_picker() {
                Ok(()) => self.ui.label(cx, ids!(library_hint)).set_text(
                    cx,
                    localized(
                        "请从系统文件选择器中选择 EPUB…",
                        "Select EPUB files in the system picker…",
                    ),
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
            // The library page or its PortalList may only be instantiated when
            // entering it after a mobile responsive transition.
            self.refresh_shelf_projection(cx);
            if cfg!(any(target_os = "android", target_os = "ios")) {
                self.pending_ui_projection_after_draw = true;
            }
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
            .radio_button_set(
                cx,
                ids_array!(
                    reader_background_paper,
                    reader_background_sepia,
                    reader_background_sage,
                    reader_background_night,
                    reader_background_theme
                ),
            )
            .selected(cx, actions)
        {
            self.set_reader_background(
                cx,
                match index {
                    1 => ReaderBackground::Sepia,
                    2 => ReaderBackground::Sage,
                    3 => ReaderBackground::Night,
                    4 => ReaderBackground::FollowTheme,
                    _ => ReaderBackground::Paper,
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
                let ids = self.selected_highlight_ids();
                if ids.is_empty() {
                    self.set_selection_hint(cx, localized("正在添加高亮…", "Adding highlight…"));
                    spawn_add_highlight(book_id, self.chapter_idx, range);
                } else {
                    self.set_selection_hint(cx, localized("正在删除高亮…", "Removing highlight…"));
                    spawn_delete_highlights(book_id, self.chapter_idx, ids);
                }
            }
        }
        if self.ui.button(cx, ids!(note_from_sel_btn)).clicked(actions) {
            self.open_note_composer(cx);
        }
        if self.ui.button(cx, ids!(copy_sel_btn)).clicked(actions) {
            let text = SELECTED_TEXT.read().unwrap().trim().to_string();
            if !text.is_empty() {
                cx.copy_to_clipboard(&text);
                self.set_selection_hint(cx, localized("已复制到剪贴板 ✓", "Copied ✓"));
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
                self.set_selection_hint(cx, localized("加入中…", "Adding…"));
                spawn_add_vocab(lemma, word, String::new(), book_id);
            } else {
                self.set_selection_hint(
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
                self.set_selection_hint(
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
                self.set_selection_hint(cx, localized("正在发音…", "Speaking…"));
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
            self.close_word_card(cx);
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
            replace_paragraphs(sample_chapter::build_paragraphs(PARA_COUNT));
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
        let background = *READER_BACKGROUND.read().unwrap();
        let theme = *READER_THEME.read().unwrap();
        let palette = reader_scene_palette(background, theme);
        let surface = palette.surface;
        let text = palette.text;
        let field = palette.field;
        let border = palette.border;
        let muted = palette.muted;
        let accent = palette.accent;
        let hover = palette.hover;
        let pressed = palette.pressed;
        let uses_dark_controls = background == ReaderBackground::Night
            || (background == ReaderBackground::FollowTheme && theme == ReaderTheme::Dark);
        let chart_vocab = if uses_dark_controls {
            Vec4f::from_u32(0x7fb2_a6ff)
        } else {
            Vec4f::from_u32(0x397f_70ff)
        };

        script_eval!(vm, {
            mod.readwise_theme_dark = mod.themes.dark{}
            mod.readwise_theme_light = mod.themes.light{}
        });
        if uses_dark_controls {
            script_eval!(vm, {
                mod.readwise_theme_scene = mod.readwise_theme_dark{}
            });
        } else {
            script_eval!(vm, {
                mod.readwise_theme_scene = mod.readwise_theme_light{}
            });
        }
        script_eval!(vm, {
            // 阅读背景是整个应用场景的单一颜色来源。这里覆盖组件模板实际
            // 消费的 token，保证所有页面、卡片和控件在 ScriptReapply 后
            // 仍与正文使用同一纸张配色，而不会退回官方主题的中性灰。
            mod.readwise_theme_scene = mod.readwise_theme_scene{
                color_bg_app: #(surface)
                color_fg_app: #(surface)
                color_app_caption_bar: #(surface)
                color_bg_container: #(field)
                color_bg_even: #(field)
                color_bg_odd: #(field)
                color_bg_highlight: #(hover)
                color_bg_unfocussed: #(hover)
                color_bg_highlight_inline: #(field)
                color_highlight: #(accent)

                color_text: #(text)
                color_text_val: #(muted)
                color_text_hl: #(text)
                color_text_hover: #(text)
                color_text_focus: #(text)
                color_text_down: #(text)
                color_text_disabled: #(muted)
                color_text_placeholder: #(muted)
                color_text_placeholder_hover: #(muted)
                color_text_meta: #(muted)
                color_text_cursor: #(accent)

                color_label_inner: #(text)
                color_label_inner_down: #(text)
                color_label_inner_drag: #(text)
                color_label_inner_hover: #(text)
                color_label_inner_focus: #(text)
                color_label_inner_active: #(text)
                color_label_inner_inactive: #(muted)
                color_label_inner_disabled: #(muted)
                color_label_outer: #(text)
                color_label_outer_off: #(muted)
                color_label_outer_down: #(text)
                color_label_outer_drag: #(text)
                color_label_outer_hover: #(text)
                color_label_outer_focus: #(text)
                color_label_outer_active: #(text)
                color_label_outer_active_focus: #(text)
                color_label_outer_disabled: #(muted)

                color_bevel: #(border)
                color_bevel_hover: #(border)
                color_bevel_focus: #(accent)
                color_bevel_active: #(border)
                color_bevel_empty: #(border)
                color_bevel_down: #(border)
                color_bevel_drag: #(border)
                color_bevel_disabled: #(border)
                color_bevel_inset_1: #(border)
                color_bevel_inset_1_hover: #(border)
                color_bevel_inset_1_focus: #(accent)
                color_bevel_inset_1_active: #(border)
                color_bevel_inset_1_empty: #(border)
                color_bevel_inset_1_down: #(border)
                color_bevel_inset_1_drag: #(border)
                color_bevel_inset_1_disabled: #(border)
                color_bevel_inset_2: #(border)
                color_bevel_inset_2_hover: #(border)
                color_bevel_inset_2_focus: #(accent)
                color_bevel_inset_2_active: #(border)
                color_bevel_inset_2_empty: #(border)
                color_bevel_inset_2_down: #(border)
                color_bevel_inset_2_drag: #(border)
                color_bevel_inset_2_disabled: #(border)
                color_bevel_outset_1: #(border)
                color_bevel_outset_1_hover: #(border)
                color_bevel_outset_1_focus: #(accent)
                color_bevel_outset_1_active: #(border)
                color_bevel_outset_1_down: #(border)
                color_bevel_outset_1_drag: #(border)
                color_bevel_outset_1_disabled: #(border)
                color_bevel_outset_2: #(border)
                color_bevel_outset_2_hover: #(border)
                color_bevel_outset_2_focus: #(accent)
                color_bevel_outset_2_active: #(border)
                color_bevel_outset_2_down: #(border)
                color_bevel_outset_2_drag: #(border)
                color_bevel_outset_2_disabled: #(border)

                color_inset: #(field)
                color_inset_hover: #(field)
                color_inset_down: #(pressed)
                color_inset_active: #(field)
                color_inset_focus: #(surface)
                color_inset_drag: #(field)
                color_inset_disabled: #(field)
                color_inset_empty: #(field)
                color_inset_1: #(field)
                color_inset_1_hover: #(field)
                color_inset_1_down: #(pressed)
                color_inset_1_active: #(field)
                color_inset_1_focus: #(surface)
                color_inset_1_drag: #(field)
                color_inset_1_disabled: #(field)
                color_inset_1_empty: #(field)
                color_inset_2: #(field)
                color_inset_2_hover: #(field)
                color_inset_2_down: #(pressed)
                color_inset_2_active: #(field)
                color_inset_2_focus: #(surface)
                color_inset_2_drag: #(field)
                color_inset_2_disabled: #(field)
                color_inset_2_empty: #(field)

                color_outset: #(field)
                color_outset_hover: #(hover)
                color_outset_down: #(pressed)
                color_outset_active: #(field)
                color_outset_focus: #(field)
                color_outset_drag: #(hover)
                color_outset_disabled: #(field)
                color_outset_inactive: #(field)
                color_outset_1: #(field)
                color_outset_1_hover: #(hover)
                color_outset_1_down: #(pressed)
                color_outset_1_active: #(field)
                color_outset_1_focus: #(field)
                color_outset_1_drag: #(hover)
                color_outset_1_disabled: #(field)
                color_outset_2: #(field)
                color_outset_2_hover: #(hover)
                color_outset_2_down: #(pressed)
                color_outset_2_active: #(field)
                color_outset_2_focus: #(field)
                color_outset_2_drag: #(hover)
                color_outset_2_disabled: #(field)

                color_icon: #(muted)
                color_icon_inactive: #(muted)
                color_icon_active: #(text)
                color_icon_disabled: #(muted)
                color_mark: #(accent)
                color_mark_empty: #(field)
                color_mark_off: #(muted)
                color_mark_hover: #(accent)
                color_mark_active: #(accent)
                color_mark_active_hover: #(accent)
                color_mark_focus: #(accent)
                color_mark_down: #(accent)
                color_mark_disabled: #(muted)
                color_selection: #(hover)
                color_selection_hover: #(hover)
                color_selection_down: #(pressed)
                color_selection_focus: #(pressed)

                color_chart_grid: #(border)
                color_chart_reading: #(accent)
                color_chart_vocab: #(chart_vocab)
            }
            mod.theme = mod.readwise_theme_scene
        });
        makepad_widgets::widgets_mod(vm);
        makepad_code_editor::script_mod(vm);
        crate::ruby_word::script_mod(vm);
        crate::chapter_image::script_mod(vm);
        crate::mark_span::script_mod(vm);
        self::script_mod(vm)
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        if self.library_action_book_id.is_some()
            && (matches!(event, Event::KeyDown(key) if key.key_code == KeyCode::Escape)
                || event.back_pressed())
        {
            self.close_library_action_menu(cx);
        }
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
                if cfg!(any(target_os = "android", target_os = "ios")) {
                    self.ui.label(cx, ids!(library_hint)).set_text(
                        cx,
                        localized(
                            "点击“导入 EPUB”，从系统文件选择器中选择书籍。",
                            "Choose Import EPUB to select books from the system picker.",
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
            Event::WindowGeomChange(event) => {
                if CURRENT_BOOK.read().unwrap().is_some() {
                    self.pending_reader_pagination_layout = true;
                }
                if self.selection_popup_anchor.is_some() {
                    self.pending_selection_popup_layout = true;
                }
                if self.word_card_anchor.is_some() {
                    self.pending_word_card_layout = true;
                }
                if self.library_action_anchor.is_some() {
                    self.pending_library_action_layout = true;
                }
                let responsive_changed = self.update_responsive_layout(
                    cx,
                    event.new_geom.inner_size,
                    event.new_geom.safe_area_insets,
                );
                let restore_mobile_geometry = cfg!(any(target_os = "android", target_os = "ios"))
                    && (responsive_changed
                        || screen_orientation_changed(
                            event.old_geom.inner_size,
                            event.new_geom.inner_size,
                        ));
                if restore_mobile_geometry {
                    self.pending_ui_projection_after_draw = true;
                    self.ui.redraw(cx);
                }
            }
            Event::VirtualKeyboard(event) => self.handle_virtual_keyboard(cx, event),
            Event::KeyDown(ke) => self.handle_reader_key(cx, ke),
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
        if matches!(event, Event::Draw(_)) {
            if self.pending_ui_projection_after_draw {
                self.pending_ui_projection_after_draw = false;
                self.restore_projected_ui_after_draw(cx);
            }
            self.resolve_reader_pagination_layout_after_draw(cx);
            if self.pending_selection_popup_layout {
                self.pending_selection_popup_layout = false;
                if self.position_selection_popup(cx, true) {
                    self.ui.redraw(cx);
                }
            }
            if self.pending_word_card_layout {
                self.pending_word_card_layout = false;
                if self.position_word_card(cx, true) {
                    self.ui.redraw(cx);
                }
            }
            if self.pending_library_action_layout {
                self.pending_library_action_layout = false;
                if self.position_library_action_menu(cx, true) {
                    self.ui.redraw(cx);
                }
            }
        }
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
