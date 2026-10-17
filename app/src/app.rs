use crate::chapter_image::ChapterImageAction;
use crate::reader_book_pages::BookPages;
use crate::reader_page_turn::{DrawReaderPageTurn, PageTurn};
use crate::reader_touch::{DoubleTap, ListTap, TapOutcome, ToolbarTap};
use crate::reader_word::word_range_at;
use crate::ruby_word::RubyWordAction;
use crate::sample_chapter;
use crate::settings_swatch::ColorKind;
use crate::shelf_list::*;
use makepad_ai::{
    AiBackend, AiEvent, AiRequest, BackendConfig, Message, OpenAiBackend, RequestId, StreamDelta,
};
use makepad_widgets::makepad_draw::text::geom::Point as LayoutPoint;
use makepad_widgets::makepad_draw::text::layouter::LaidoutText;
use makepad_widgets::makepad_platform::event::{TouchPoint, TouchState, WindowDragQueryResponse};
use makepad_widgets::*;
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock, RwLock};
use std::time::Instant;

type AiSettings = readwise_core::ai::AiSettings;
type AppLanguage = readwise_core::settings::AppLanguage;
type LibraryViewMode = readwise_core::settings::LibraryViewMode;
type LocalAiSettings = readwise_core::local_ai::LocalAiSettings;
type LocalAiSession = readwise_core::local_ai::LocalAiSession;
type ReaderBackground = readwise_core::settings::ReaderBackground;
type ReaderColumns = readwise_core::settings::ReaderColumns;
type ReaderTurnMode = readwise_core::settings::ReaderTurnMode;
type ReaderFont = readwise_core::settings::ReaderFont;
type ReaderSpacing = readwise_core::settings::ReaderSpacing;
type ReaderSettings = readwise_core::settings::ReaderSettings;
type ReaderTheme = readwise_core::settings::ReaderTheme;
type StatisticsSnapshot = readwise_core::statistics::models::StatisticsSnapshot;
type VocabEntry = readwise_core::vocab::models::VocabBookEntry;

app_main!(App);

/// Passive widget notifications still reach widgets, but require no global
/// control lookup. Never skip a mixed batch containing a click or data action.
fn app_action_batch_is_passive(actions: &Actions, on_reader_page: bool) -> bool {
    actions.iter().all(|action| {
        let Some(widget) = action.as_widget_action() else {
            return false;
        };
        matches!(
            widget.action.downcast_ref::<ShelfListAction>(),
            Some(ShelfListAction::Scroll)
        ) || (!on_reader_page
            && matches!(
                widget.action.downcast_ref::<PortalListAction>(),
                Some(PortalListAction::Scroll)
            ))
            || matches!(
                widget.action.downcast_ref::<ViewAction>(),
                Some(
                    ViewAction::FingerDown(_)
                        | ViewAction::FingerMove(_)
                        | ViewAction::FingerHoverIn(_)
                        | ViewAction::FingerHoverOut(_)
                )
            )
            || matches!(
                widget.action.downcast_ref::<ButtonAction>(),
                Some(ButtonAction::Pressed(_) | ButtonAction::Released(_))
            )
    })
}

#[cfg(test)]
mod passive_action_tests {
    use super::*;

    fn widget_action(value: impl WidgetActionTrait) -> Action {
        Box::new(WidgetAction {
            data: None,
            action: Box::new(value),
            widget_uid: WidgetUid(1),
            group: None,
        })
    }

    #[test]
    fn scroll_only_batches_skip_global_lookups_but_reader_progress_does_not() {
        assert!(app_action_batch_is_passive(
            &vec![widget_action(ShelfListAction::Scroll)],
            false
        ));
        let actions = vec![widget_action(PortalListAction::Scroll)];
        assert!(app_action_batch_is_passive(&actions, false));
        assert!(!app_action_batch_is_passive(&actions, true));
    }

    #[test]
    fn deferred_reader_taps_inspect_widget_payloads() {
        assert!(reader_tap_opens_inline(&widget_action(
            RubyWordAction::Clicked {
                word: "solitude".into(),
                gloss: "孤独".into(),
                anchor: dvec2(100.0, 200.0),
            }
        )));
        assert!(reader_tap_opens_inline(&widget_action(
            makepad_widgets::html::HtmlLinkAction::Clicked {
                url: "chapter.xhtml".into(),
                key_modifiers: Default::default(),
            }
        )));
        assert!(!reader_tap_opens_inline(&widget_action(
            ButtonAction::Clicked(Default::default())
        )));
        let toolbar: Action = Box::new(ReaderToolbarTap);
        assert!(!reader_tap_opens_inline(&toolbar));
    }

    #[test]
    fn clicks_and_custom_actions_in_scroll_batches_are_never_skipped() {
        let actions = vec![
            widget_action(ShelfListAction::Scroll),
            widget_action(ButtonAction::Clicked(Default::default())),
        ];
        assert!(!app_action_batch_is_passive(&actions, false));
        assert!(!app_action_batch_is_passive(&actions, true));
        let actions: ActionsBuf = vec![
            widget_action(ShelfListAction::Scroll),
            Box::new(ReaderToolbarTap),
        ];
        assert!(!app_action_batch_is_passive(&actions, false));
        assert!(!app_action_batch_is_passive(
            &vec![widget_action(ViewAction::FingerUp(FingerUpEvent {
                window_id: WindowId(0, 0),
                abs: dvec2(0.0, 0.0),
                abs_start: dvec2(0.0, 0.0),
                capture_time: 1.0,
                time: 1.1,
                digit_id: Default::default(),
                device: DigitDevice::Mouse {
                    button: MouseButton::PRIMARY
                },
                has_long_press_occurred: false,
                tap_count: 1,
                modifiers: Default::default(),
                rect: Default::default(),
                is_over: true,
                is_sweep: false,
            }))],
            false
        ));
    }
}

#[cfg(test)]
mod reader_icon_tests {
    #[test]
    fn toolbar_icons_have_renderable_svg_geometry() {
        let icons = [
            include_str!("../resources/icons/reader_back.svg"),
            include_str!("../resources/icons/reader_contents.svg"),
            include_str!("../resources/icons/reader_bookmarks.svg"),
            include_str!("../resources/icons/reader_notes.svg"),
            include_str!("../resources/icons/reader_search.svg"),
            include_str!("../resources/icons/reader_bookmark.svg"),
            include_str!("../resources/icons/reader_bookmark_filled.svg"),
            include_str!("../resources/icons/reader_settings.svg"),
            include_str!("../resources/icons/nav_reading.svg"),
            include_str!("../resources/icons/nav_shelf.svg"),
            include_str!("../resources/icons/nav_vocab.svg"),
            include_str!("../resources/icons/nav_profile.svg"),
            include_str!("../resources/icons/nav_time.svg"),
            include_str!("../resources/icons/nav_completed.svg"),
            include_str!("../resources/icons/nav_chat.svg"),
            include_str!("../resources/icons/nav_skills.svg"),
        ];
        for svg in icons {
            let document = makepad_widgets::makepad_draw::svg::parse_svg(svg);
            let (left, top, right, bottom) = document.compute_bounds().expect("icon geometry");
            assert!(right > left && bottom > top);
        }
    }
}

const PARA_COUNT: usize = 200;
const COMPACT_LAYOUT_BREAKPOINT: f64 = 720.0;
const DESKTOP_MIN_WINDOW_WIDTH: f64 = COMPACT_LAYOUT_BREAKPOINT;
const MOBILE_LANDSCAPE_COMPACT_HEIGHT: f64 = 600.0;
const READER_SWIPE_MIN_DISTANCE: f64 = 64.0;
const READER_SWIPE_MAX_DURATION: f64 = 0.9;
const READER_SWIPE_AXIS_RATIO: f64 = 1.35;
const READER_SWIPE_VERTICAL_CANCEL: f64 = 28.0;

fn current_clipped_rect(cx: &Cx, area: Area) -> Rect {
    if !area.is_valid(cx) {
        return Rect::default();
    }
    // A ScriptReapply can clear a draw list before its widgets receive fresh
    // Areas. Makepad's Area::is_valid checks the redraw ID, but not rect_id.
    if let Area::Rect(rect) = area {
        let Some(draw_list) = cx.draw_lists.checked_index(rect.draw_list_id) else {
            return Rect::default();
        };
        if rect.rect_id >= draw_list.rect_areas.len() {
            return Rect::default();
        }
    }
    area.clipped_rect(cx)
}

fn uses_compact_layout_for_platform(size: DVec2, is_mobile: bool) -> bool {
    is_mobile
        && size.x > 0.0
        && size.y > 0.0
        && (size.x < COMPACT_LAYOUT_BREAKPOINT || size.y < MOBILE_LANDSCAPE_COMPACT_HEIGHT)
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

fn reader_body_top_inset(compact: bool, in_caption: bool) -> f64 {
    if compact || in_caption {
        0.0
    } else {
        54.0
    }
}

fn mobile_reader_button_size(width: f64, insets: SafeAreaInsets) -> f64 {
    // Seven actions, five intra-group gaps, and 8px of padding at each edge.
    ((width - insets.left - insets.right - 26.0) / 7.0).clamp(24.0, 44.0)
}

fn reader_customise_rect(size: DVec2, insets: SafeAreaInsets) -> Rect {
    let available = dvec2(
        (size.x - insets.left - insets.right).max(1.0),
        (size.y - insets.top - insets.bottom).max(1.0),
    );
    let card = dvec2(
        (available.x - 24.0).clamp(1.0, 560.0),
        (available.y - 24.0).clamp(1.0, 620.0),
    );
    Rect {
        pos: dvec2(insets.left, insets.top) + (available - card) * 0.5,
        size: card,
    }
}

fn reader_font_display_label(language: AppLanguage, font: ReaderFont) -> &'static str {
    if font == ReaderFont::Original {
        crate::i18n::text(language, "reader_font_original")
    } else {
        font.label()
    }
}

fn library_surface_visibility(visible_count: usize) -> (bool, bool) {
    // Keep the data-owning PortalList mounted even when it is empty. If it is
    // hidden, a responsive rebuild can leave it uninstantiated and later
    // set_visible calls become no-ops until another user action finds it.
    (visible_count == 0, true)
}

fn shelf_grid_columns_for_width(width: f64) -> usize {
    // Match the number of columns to the space available for roughly 200px cards.
    // The row templates have six cells; narrow screens always keep two columns.
    ((width - 48.0 + 8.0) / 208.0).floor().clamp(2.0, 6.0) as usize
}

fn shelf_grid_cover_height(width: f64, columns: usize) -> f64 {
    let columns = columns.max(1) as f64;
    // PortalList padding, row gaps, scrollbar, and the card's own padding.
    let cell_width = (width - 48.0 - 8.0 * (columns - 1.0) - 12.0) / columns;
    ((cell_width - 20.0).max(1.0) * 116.0 / 86.0).round()
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

const LIST_COVER_WIDTH_TO_HEIGHT: f64 = 44.0 / 56.0;

fn shelf_list_cover_size(info_height: f64, shelf_width: f64) -> DVec2 {
    // Reserve the list's 48px padding, the card's 24px padding, two
    // 14px gaps, a 40px menu and at least 80px for text. Cover growth
    // must not narrow the text and recursively inflate the next measurement.
    let max_width = (shelf_width - 48.0 - 24.0 - 28.0 - 40.0 - 80.0).clamp(32.0, 128.0);
    let width = (info_height.max(0.0) * LIST_COVER_WIDTH_TO_HEIGHT).min(max_width);
    dvec2(width, width / LIST_COVER_WIDTH_TO_HEIGHT)
}

fn reader_last_page_start(block_count: usize, blocks_per_page: usize) -> usize {
    let last = block_count.saturating_sub(1);
    let span = blocks_per_page.max(1);
    (last / span) * span
}

fn reader_tap_opens_inline(action: &Action) -> bool {
    reader_tap_selectable_inline(action)
        || action.as_widget_action().is_some_and(|action| {
            matches!(
                action.cast(),
                crate::chapter_image::ChapterImageAction::Open(_)
            )
        })
}

fn reader_tap_selectable_inline(action: &Action) -> bool {
    let Some(action) = action.as_widget_action() else {
        return false;
    };
    matches!(action.cast(), RubyWordAction::Clicked { .. })
        || matches!(
            action.cast(),
            makepad_widgets::html::HtmlLinkAction::Clicked { .. }
        )
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
    ReadingHome,
    Profile,
    Notes,
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
            Self::ReadingHome => id!(page_reading_home),
            Self::Profile => id!(page_profile),
            Self::Notes => id!(page_notes),
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
            7 => Self::ReadingHome,
            8 => Self::Profile,
            9 => Self::Notes,
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
            Self::ReadingHome => 7,
            Self::Profile => 8,
            Self::Notes => 9,
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
    vertical: bool,
    allow_previous: bool,
    allow_next: bool,
}

impl ReaderSwipeGesture {
    fn begin(&mut self, uid: u64, abs: DVec2, time: f64) {
        self.origin = Some((uid, abs, time));
        self.cancelled = false;
        self.vertical = false;
        self.allow_previous = true;
        self.allow_next = true;
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
        let project = |pos: DVec2| {
            if self.vertical {
                dvec2(pos.y, pos.x)
            } else {
                pos
            }
        };
        let start = project(start);
        let end = project(touch.abs);
        let delta = end - start;
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
        reader_swipe_direction(start, end, event_time - start_time).filter(|direction| {
            match direction {
                ReaderSwipeDirection::Previous => self.allow_previous,
                ReaderSwipeDirection::Next => self.allow_next,
            }
        })
    }
}

static PARAGRAPHS: LazyLock<RwLock<Vec<String>>> = LazyLock::new(|| RwLock::new(Vec::new()));
static CODE_BLOCKS: LazyLock<RwLock<Vec<Option<String>>>> =
    LazyLock::new(|| RwLock::new(Vec::new()));

fn replace_paragraphs(chunks: Vec<String>) {
    if *PARAGRAPHS.read().unwrap() == chunks {
        return;
    }
    READER_CONTENT_VERSION.fetch_add(1, Ordering::Relaxed);
    let code_blocks = chunks
        .iter()
        .map(|chunk| readwise_core::books::epub_meta::code_block_text(chunk))
        .collect();
    *CODE_BLOCKS.write().unwrap() = code_blocks;
    *PARAGRAPHS.write().unwrap() = chunks;
}

fn silence_current_paragraph_glosses(lemma: &str) -> bool {
    let mut paragraphs = PARAGRAPHS.write().unwrap();
    silence_paragraph_glosses(&mut paragraphs, lemma)
}

fn silence_paragraph_glosses(paragraphs: &mut [String], lemma: &str) -> bool {
    let mut changed = false;
    for paragraph in paragraphs.iter_mut().filter(|text| text.contains("<ruby")) {
        let silent = readwise_core::vocab::annotate::silence_ruby_for_lemma(paragraph, lemma);
        if *paragraph != silent {
            *paragraph = silent;
            changed = true;
        }
    }
    // Only gloss text changed. The same ruby widgets, words, and line spacing
    // remain, so existing pagination metrics are still valid.
    changed
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

/// 连续正文使用一次稳定的纯文本布局做鼠标命中，避免 TextFlow 按行内 run
/// 记录的 SelectionTracker Area 在选区重绘/窗口缩放后出现重叠或旧引用。
/// 表格、列表、引用和图片有额外的几何结构，仍交给实际 widget 命中。
fn supports_unified_reader_hit_test(html: &str) -> bool {
    let mut remaining = html;
    while let Some(tag_start) = remaining.find('<') {
        remaining = &remaining[tag_start + 1..];
        let trimmed = remaining.trim_start();
        let candidate = trimmed.strip_prefix('/').unwrap_or(trimmed).trim_start();
        let name_end = candidate
            .find(|ch: char| !ch.is_ascii_alphanumeric())
            .unwrap_or(candidate.len());
        let name = candidate[..name_end].to_ascii_lowercase();
        if matches!(
            name.as_str(),
            "img"
                | "image"
                | "svg"
                | "table"
                | "thead"
                | "tbody"
                | "tr"
                | "th"
                | "td"
                | "ul"
                | "ol"
                | "li"
                | "blockquote"
                | "pre"
                | "hr"
                | "details"
                | "summary"
        ) {
            return false;
        }
        let Some(tag_end) = remaining.find('>') else {
            break;
        };
        remaining = &remaining[tag_end + 1..];
    }
    true
}

fn reader_plain_paragraph(html: &str) -> bool {
    let html = html.trim();
    let Some(open_end) = html.find('>') else {
        return false;
    };
    let open = &html[..open_end];
    if open != "<p" && !open.starts_with("<p ") {
        return false;
    }
    html[open_end + 1..]
        .strip_suffix("</p>")
        .is_some_and(|body| !body.contains('<'))
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

fn ordered_selection_range(anchor: TextPoint, cursor: TextPoint) -> TextRange {
    if cursor < anchor {
        (cursor, anchor)
    } else {
        (anchor, cursor)
    }
}

/// Choose between TextFlow's real rich-text hit and the continuous fallback.
///
/// The native result owns normal movement because it was produced by the exact
/// runs that are visible on screen. The fallback is only allowed to take over
/// when the native result moves backwards (or remains stuck despite a real
/// pointer move) while the pointer is moving forwards, and vice versa. This
/// preserves exact visual-line mapping without getting trapped at an italic,
/// link, mark, or RubyWord run boundary.
fn choose_reader_hit_index(
    native: usize,
    fallback: usize,
    previous: usize,
    forward: bool,
    pointer_moved: bool,
) -> usize {
    if forward {
        if native > previous || (native == previous && !pointer_moved) {
            native
        } else {
            fallback.max(previous)
        }
    } else if native < previous || (native == previous && !pointer_moved) {
        native
    } else {
        fallback.min(previous)
    }
}

fn reader_drag_moves_forward(previous: DVec2, current: DVec2) -> bool {
    // Raised inline runs can move the reported glyph box a few pixels without
    // crossing a visual row. Compare against the previous pointer sample—not
    // the fixed selection anchor—so reversing inside the same side of the
    // anchor can shrink the selection again.
    if (current.y - previous.y).abs() <= 8.0 {
        current.x >= previous.x
    } else {
        current.y > previous.y
    }
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
        choose_reader_hit_index, is_table_block, normalize_rich_selection_rects,
        ordered_selection_range, reader_block_visual_height, reader_drag_moves_forward,
        reader_fallback_source_index, reader_geometry_capture_policy,
        reader_geometry_width_is_stable, reader_markup_available_width, reader_pagination_rows,
        reader_painted_page_cuts, reader_plain_paragraph, selection_range_in_block,
        supports_unified_reader_hit_test, visual_selection_row_bounds,
    };
    use makepad_widgets::{dvec2, Rect};

    #[test]
    fn recognizes_only_top_level_table_blocks() {
        assert!(is_table_block("  <TABLE class='data'><tr></tr></TABLE>"));
        assert!(!is_table_block("<p>Table 1</p><table></table>"));
        assert!(!is_table_block("<tableau>not a table</tableau>"));
    }

    #[test]
    fn continuous_reader_blocks_use_stable_unified_hit_testing() {
        assert!(supports_unified_reader_hit_test(
            "<p>before <MARK>selected</MARK> after</p>"
        ));
        assert!(supports_unified_reader_hit_test(
            "<p>before <em>italic</em> after</p>"
        ));
        assert!(supports_unified_reader_hit_test(
            "<p>before <ruby t='word'>vocabulary</ruby> after</p>"
        ));
        assert!(supports_unified_reader_hit_test(
            "<p>before <a href='chapter.xhtml'>link</a> after</p>"
        ));
        assert!(supports_unified_reader_hit_test("<p>ordinary text</p>"));
        assert!(supports_unified_reader_hit_test("plain text"));
        assert!(supports_unified_reader_hit_test(
            "<strong>standalone text</strong>"
        ));
        assert!(!supports_unified_reader_hit_test(
            "<p>before <image>name</image> after</p>"
        ));
        assert!(!supports_unified_reader_hit_test(
            "<p><em>caption</em><img src='figure.png'/></p>"
        ));
        assert!(!supports_unified_reader_hit_test(
            "<blockquote><p>quoted</p></blockquote>"
        ));
        assert!(!supports_unified_reader_hit_test("<ul><li>item</li></ul>"));
        assert!(!supports_unified_reader_hit_test(
            "<table><tr><td>cell</td></tr></table>"
        ));
    }

    #[test]
    fn long_plain_paragraph_uses_painted_row_selection_geometry() {
        let text = "夏帆觉得母亲内里已全然换了个人。".repeat(30);
        assert!(reader_plain_paragraph(&format!("<p>{text}</p>")));
        assert!(reader_plain_paragraph(
            "<p class='body'>穿得稍微像样些嘛。</p>"
        ));
        assert!(!reader_plain_paragraph(
            "<p>带有<ruby>注音</ruby>的正文</p>"
        ));
        assert!(!reader_plain_paragraph("<p>带有<em>斜体</em>的正文</p>"));
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
    fn release_cursor_replaces_a_stale_last_move_across_blocks() {
        let anchor = (2, 4);
        let stale_move = (2, 18);
        let release = (4, 7);
        assert_eq!(
            ordered_selection_range(anchor, stale_move),
            (anchor, stale_move)
        );

        let final_range = ordered_selection_range(anchor, release);
        assert_eq!(final_range, (anchor, release));
        assert_eq!(selection_range_in_block(final_range, 2, 20), Some((4, 20)));
        assert_eq!(selection_range_in_block(final_range, 3, 12), Some((0, 12)));
        assert_eq!(selection_range_in_block(final_range, 4, 20), Some((0, 7)));
    }

    #[test]
    fn real_rich_text_hit_owns_visual_line_changes() {
        // The continuous fallback still thinks the pointer is on the preceding
        // visual line. A progressing native hit must win.
        assert_eq!(choose_reader_hit_index(42, 31, 30, true, true), 42);
    }

    #[test]
    fn continuous_fallback_only_bridges_a_stuck_inline_run() {
        assert_eq!(choose_reader_hit_index(30, 36, 30, true, true), 36);
        assert_eq!(choose_reader_hit_index(30, 36, 30, true, false), 30);
        assert_eq!(choose_reader_hit_index(30, 24, 30, false, true), 24);
    }

    #[test]
    fn drag_direction_follows_the_last_pointer_sample_so_selection_can_shrink() {
        assert!(reader_drag_moves_forward(
            dvec2(120.0, 40.0),
            dvec2(126.0, 43.0)
        ));
        assert!(!reader_drag_moves_forward(
            dvec2(126.0, 40.0),
            dvec2(120.0, 43.0)
        ));
        assert!(!reader_drag_moves_forward(
            dvec2(120.0, 60.0),
            dvec2(200.0, 40.0)
        ));
        assert_eq!(choose_reader_hit_index(34, 34, 36, false, true), 34);
    }

    #[test]
    fn rich_selection_merges_and_bottom_aligns_a_raised_ruby_fragment() {
        let rects = normalize_rich_selection_rects(&[
            Rect {
                pos: dvec2(10.0, 20.0),
                size: dvec2(30.0, 14.0),
            },
            Rect {
                pos: dvec2(40.0, 12.0),
                size: dvec2(20.0, 14.0),
            },
            Rect {
                pos: dvec2(60.0, 20.0),
                size: dvec2(35.0, 14.0),
            },
        ]);
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].pos, dvec2(10.0, 18.0));
        assert_eq!(rects[0].size, dvec2(85.0, 18.0));
    }

    #[test]
    fn rich_selection_joins_the_vertical_gap_between_actual_wrapped_rows() {
        let rects = normalize_rich_selection_rects(&[
            Rect {
                pos: dvec2(50.0, 20.0),
                size: dvec2(50.0, 14.0),
            },
            Rect {
                pos: dvec2(10.0, 40.0),
                size: dvec2(70.0, 14.0),
            },
        ]);
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0].pos.y + rects[0].size.y, rects[1].pos.y);
    }

    #[test]
    fn rich_selection_rows_share_one_boundary_even_when_tall_annotations_overlap() {
        let rects = normalize_rich_selection_rects(&[
            Rect {
                pos: dvec2(20.0, 20.0),
                size: dvec2(80.0, 20.0),
            },
            Rect {
                pos: dvec2(10.0, 34.0),
                size: dvec2(90.0, 20.0),
            },
        ]);
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0].pos.y + rects[0].size.y, rects[1].pos.y);
        assert!(rects[0].pos.y + rects[0].size.y <= 37.0);
    }

    #[test]
    fn page_cuts_use_raw_row_gaps_instead_of_selection_strip_boundaries() {
        let rects = [
            Rect {
                pos: dvec2(20.0, 20.0),
                size: dvec2(80.0, 20.0),
            },
            Rect {
                pos: dvec2(10.0, 34.0),
                size: dvec2(90.0, 20.0),
            },
            Rect {
                pos: dvec2(10.0, 68.0),
                size: dvec2(90.0, 20.0),
            },
        ];
        // The first two visual rows overlap, so a cut at the selection
        // strip's midpoint would remove part of a letter on either page.
        let cuts = reader_pagination_rows(&rects);
        assert_eq!(cuts.len(), 1);
        assert_eq!(cuts[0].0, 54.0);
        assert_eq!(cuts[0].1, dvec2(10.5, 78.0));
    }

    #[test]
    fn padded_selection_rows_still_produce_a_complete_line_page_cut() {
        use crate::reader_columns::{compose_page, BlockMetrics, LineBoundary, PageAnchor};

        let rects = [
            Rect {
                pos: dvec2(10.0, 10.0),
                size: dvec2(90.0, 20.0),
            },
            Rect {
                pos: dvec2(10.0, 26.0),
                size: dvec2(90.0, 20.0),
            },
        ];
        let cuts = reader_pagination_rows(&rects);
        assert_eq!(cuts.len(), 1);
        assert_eq!(cuts[0].0, 28.0);
        let metrics = [Some(BlockMetrics {
            height: 50.0,
            boundaries: vec![LineBoundary {
                y: cuts[0].0,
                source: 10,
            }],
            keep_together: false,
        })];
        let first = compose_page(&metrics, PageAnchor::default(), 30.0).unwrap();
        assert_eq!(first.next.y, 28.0);
        let second = compose_page(&metrics, first.next, 30.0).unwrap();
        assert_eq!(second.fragments[0].from, 28.0);
        assert_eq!(second.fragments[0].to, 50.0);
    }

    #[test]
    fn missing_text_map_keeps_monotone_line_sources() {
        assert_eq!(reader_fallback_source_index(0, 100, 80), 0);
        assert_eq!(reader_fallback_source_index(50, 100, 80), 40);
        assert_eq!(reader_fallback_source_index(100, 100, 80), 80);
    }

    #[test]
    fn painted_page_cuts_follow_shifted_glyph_rows() {
        let rows = [
            (
                Rect {
                    pos: dvec2(10.0, 20.0),
                    size: dvec2(40.0, 12.0),
                },
                0,
            ),
            (
                Rect {
                    pos: dvec2(50.0, 20.0),
                    size: dvec2(40.0, 12.0),
                },
                4,
            ),
            (
                Rect {
                    pos: dvec2(10.0, 46.0),
                    size: dvec2(80.0, 12.0),
                },
                10,
            ),
        ];
        assert_eq!(reader_painted_page_cuts(&rows), vec![(34.0, 10)]);
    }

    #[test]
    fn last_row_moves_whole_when_ink_reaches_the_page_edge() {
        use crate::reader_columns::{compose_page, BlockMetrics, LineBoundary, PageAnchor};

        // Measured from the two last painted lines of 夏帆 191/192. The
        // layout row box had suggested 156.3, which bisected the last ink.
        let rows = [
            (
                Rect {
                    pos: dvec2(4.3, 127.1767),
                    size: dvec2(478.6, 13.7067),
                },
                541,
            ),
            (
                Rect {
                    pos: dvec2(3.09, 151.1533),
                    size: dvec2(391.76, 13.4667),
                },
                649,
            ),
        ];
        let cut = reader_painted_page_cuts(&rows)[0].0;
        assert!(cut > 140.8834 && cut < 151.1533);
        let metrics = [Some(BlockMetrics {
            height: 181.7,
            boundaries: vec![LineBoundary {
                y: cut,
                source: 649,
            }],
            keep_together: false,
        })];
        let first = compose_page(&metrics, PageAnchor::default(), 166.65).unwrap();
        assert_eq!(first.next.y, cut);
    }

    #[test]
    fn page_cut_never_crosses_a_later_run_on_an_earlier_visual_row() {
        // TextFlow draws a wrapping run before the next inline run. The
        // latter can extend below an apparent gap between the first run's
        // rows, even though it is still part of the upper visual row.
        let rows = [
            (
                Rect {
                    pos: dvec2(10.0, 20.0),
                    size: dvec2(40.0, 12.0),
                },
                0,
            ),
            (
                Rect {
                    pos: dvec2(10.0, 46.0),
                    size: dvec2(80.0, 12.0),
                },
                10,
            ),
            (
                Rect {
                    pos: dvec2(50.0, 20.0),
                    size: dvec2(40.0, 30.0),
                },
                4,
            ),
            (
                Rect {
                    pos: dvec2(10.0, 72.0),
                    size: dvec2(80.0, 12.0),
                },
                20,
            ),
        ];
        assert_eq!(reader_painted_page_cuts(&rows), vec![(60.0, 20)]);
    }

    #[test]
    fn paragraph_height_includes_the_last_painted_row() {
        assert_eq!(reader_block_visual_height(124.7, 127.9), 128.9);
        assert_eq!(reader_block_visual_height(124.7, 120.0), 124.7);
    }

    #[test]
    fn resized_portal_item_must_stabilize_before_geometry_is_cached() {
        assert!(!reader_geometry_width_is_stable(720.0, 540.0));
        assert!(!reader_geometry_width_is_stable(0.0, 540.0));
        assert!(reader_geometry_width_is_stable(540.0, 540.25));
    }

    #[test]
    fn live_drag_paints_immediately_while_persistent_cache_waits() {
        let current = Some((12, 38));
        assert_eq!(
            reader_geometry_capture_policy(current, current, false),
            (true, false)
        );
        assert_eq!(
            reader_geometry_capture_policy(current, current, true),
            (true, true)
        );
    }

    #[test]
    fn adjacent_selected_rows_share_the_gap_boundary() {
        let first = visual_selection_row_bounds(None, 0.0, 10.0, Some(20.0), 2.0);
        let second = visual_selection_row_bounds(Some(10.0), 20.0, 30.0, None, 2.0);
        assert_eq!(first, (-2.0, 15.0));
        assert_eq!(second, (15.0, 32.0));
        assert_eq!(first.1, second.0);
    }

    #[test]
    fn reader_overlay_uses_fill_host_when_html_area_is_content_sized() {
        let host = Rect {
            pos: dvec2(40.0, 20.0),
            size: dvec2(720.0, 120.0),
        };
        let stale_content_area = Rect {
            pos: dvec2(40.0, 20.0),
            size: dvec2(318.0, 120.0),
        };
        assert_eq!(
            reader_markup_available_width(host, stale_content_area, 0.0, 0.0),
            720.0
        );
    }

    #[test]
    fn reader_overlay_keeps_html_width_when_it_extends_past_host() {
        let host = Rect {
            pos: dvec2(40.0, 20.0),
            size: dvec2(640.0, 120.0),
        };
        let html = Rect {
            pos: dvec2(48.0, 20.0),
            size: dvec2(660.0, 120.0),
        };
        assert_eq!(reader_markup_available_width(host, html, 8.0, 8.0), 644.0);
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

/// 当前书的笔记（后台线程刷新，NoteList 展示引用并允许编辑正文）。
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
    /// Replacing a thread can reuse the same PortalList row IDs.
    revision: u64,
    streaming_revision: u64,
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

static PROFILE_STATISTICS: LazyLock<RwLock<Option<StatisticsSnapshot>>> =
    LazyLock::new(|| RwLock::new(None));
static ALL_NOTES: LazyLock<RwLock<Vec<readwise_core::notes::models::Note>>> =
    LazyLock::new(|| RwLock::new(Vec::new()));

#[derive(Debug)]
enum AllNotesAction {
    Loaded(Vec<readwise_core::notes::models::Note>),
    Error(String),
}

fn spawn_load_all_notes() {
    std::thread::spawn(|| {
        let Some(db) = DB.get() else { return };
        match db.get_all_notes() {
            Ok(notes) => Cx::post_action(AllNotesAction::Loaded(notes)),
            Err(error) => Cx::post_action(AllNotesAction::Error(error)),
        }
    });
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
        assert!(!uses_compact_layout_for_platform(
            dvec2(600.0, 900.0),
            false
        ));
        assert!(!uses_compact_layout_for_platform(dvec2(0.0, 0.0), true));
    }

    #[test]
    fn mobile_toolbar_actions_fit_phone_widths_and_cutout_insets() {
        for (width, insets) in [
            (320.0, SafeAreaInsets::default()),
            (360.0, SafeAreaInsets::default()),
            (390.0, SafeAreaInsets::default()),
            (
                844.0,
                SafeAreaInsets {
                    left: 44.0,
                    right: 44.0,
                    ..Default::default()
                },
            ),
        ] {
            let button = mobile_reader_button_size(width, insets);
            assert!(button >= 40.0);
            assert!(button <= 44.0);
            assert!(7.0 * button + 26.0 + insets.left + insets.right <= width + 0.001);
        }
        // The mobile bar is an overlay, so showing it never reduces the body.
        assert_eq!(reader_body_top_inset(true, false), 0.0);
        assert_eq!(reader_body_top_inset(true, true), 0.0);
        assert_eq!(reader_body_top_inset(false, true), 0.0);
        assert_eq!(reader_body_top_inset(false, false), 54.0);
    }

    #[test]
    fn customise_dialog_fits_the_safe_area_in_both_phone_orientations() {
        for (size, insets) in [
            (dvec2(320.0, 568.0), SafeAreaInsets::default()),
            (
                dvec2(390.0, 844.0),
                SafeAreaInsets {
                    top: 47.0,
                    bottom: 34.0,
                    ..Default::default()
                },
            ),
            (
                dvec2(844.0, 390.0),
                SafeAreaInsets {
                    left: 44.0,
                    right: 44.0,
                    bottom: 21.0,
                    ..Default::default()
                },
            ),
            (dvec2(1080.0, 720.0), SafeAreaInsets::default()),
        ] {
            let rect = reader_customise_rect(size, insets);
            assert!(rect.pos.x >= insets.left + 12.0);
            assert!(rect.pos.y >= insets.top + 12.0);
            assert!(rect.pos.x + rect.size.x <= size.x - insets.right - 12.0);
            assert!(rect.pos.y + rect.size.y <= size.y - insets.bottom - 12.0);
            assert!(rect.size.x >= 296.0);
            assert!(rect.size.y >= 300.0);
        }
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
        for page in [AppPage::ReadingHome, AppPage::Profile, AppPage::Notes] {
            assert_eq!(AppPage::from_persisted(page.persisted()), page);
            assert_eq!(restored_page_after_rebuild(page.persisted(), false), page);
        }
        assert_eq!(AppPage::Settings.persisted(), 6);
        assert_eq!(
            restored_page_after_rebuild(usize::MAX, true),
            AppPage::Library
        );
    }

    #[test]
    fn long_press_and_double_click_share_word_boundaries() {
        assert_eq!(word_range_at("read wise", 2), Some((0, 4)));
        assert_eq!(word_range_at("can't stop", 3), Some((0, 5)));
        assert_eq!(word_range_at("阅读", 3), Some((0, 6)));
        assert_eq!(word_range_at("A—can’t", 7), Some((4, 11)));
        assert_eq!(word_range_at("read wise", 4), None);
        assert_eq!(word_range_at("阅读。", 6), Some((6, 9)));
        assert_eq!(word_range_at("é—word", 2), Some((2, 5)));
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

    #[test]
    fn vertical_swipes_cross_only_the_edge_where_the_gesture_started() {
        let stop = |x, y| TouchPoint {
            state: TouchState::Stop,
            abs: dvec2(x, y),
            time: 1.4,
            uid: 1,
            rotation_angle: 0.0,
            force: 1.0,
            radius: dvec2(1.0, 1.0),
            handled: Default::default(),
            sweep_lock: Default::default(),
        };
        for (previous, next, end, expected) in [
            (false, false, stop(200.0, 300.0), None),
            (
                false,
                true,
                stop(200.0, 300.0),
                Some(ReaderSwipeDirection::Next),
            ),
            (false, true, stop(200.0, 500.0), None),
            (
                true,
                false,
                stop(200.0, 500.0),
                Some(ReaderSwipeDirection::Previous),
            ),
            (true, true, stop(100.0, 400.0), None),
        ] {
            let mut gesture = ReaderSwipeGesture::default();
            gesture.begin(1, dvec2(200.0, 400.0), 1.0);
            gesture.vertical = true;
            gesture.allow_previous = previous;
            gesture.allow_next = next;
            assert_eq!(gesture.update(&end, 1.4), expected);
            assert_eq!(gesture.update(&end, 1.5), None);
        }
    }

    #[test]
    fn long_press_cancel_prevents_both_horizontal_and_vertical_page_turns() {
        for vertical in [false, true] {
            let mut gesture = ReaderSwipeGesture::default();
            gesture.begin(1, dvec2(200.0, 400.0), 1.0);
            gesture.vertical = vertical;
            gesture.cancel();
            let end = TouchPoint {
                state: TouchState::Stop,
                abs: dvec2(100.0, 300.0),
                time: 1.4,
                uid: 1,
                rotation_angle: 0.0,
                force: 1.0,
                radius: dvec2(1.0, 1.0),
                handled: Default::default(),
                sweep_lock: Default::default(),
            };
            assert_eq!(gesture.update(&end, 1.4), None);
        }
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
    last_read_at: Option<i64>,
    want_to_read: bool,
    tags: Vec<String>,
    cover_path: Option<String>,
    file_path: String,
}

/// Keep the open book in this process; otherwise use persisted reading time.
/// Older databases may have a reading status without a recorded timestamp.
fn recent_read_shelf_item<'a>(
    items: &'a [ShelfItem],
    current_id: Option<&str>,
) -> Option<&'a ShelfItem> {
    current_id
        .and_then(|id| items.iter().find(|book| book.id == id))
        .or_else(|| {
            items
                .iter()
                .filter(|book| book.last_read_at.is_some())
                .max_by_key(|book| book.last_read_at)
        })
        .or_else(|| items.iter().find(|book| book.status == "reading"))
}

const WANT_TO_READ_FILTER: &str = "\0readwise:want-to-read";
const READING_FILTER: &str = "\0readwise:reading";
const COMPLETED_FILTER: &str = "\0readwise:completed";

fn shelf_items_for_tag(items: &[ShelfItem], selected: Option<&str>) -> Vec<ShelfItem> {
    items
        .iter()
        .filter(|book| {
            selected.is_none_or(|selected| {
                if selected == WANT_TO_READ_FILTER {
                    return book.want_to_read;
                }
                if selected == READING_FILTER {
                    return book.status == "reading";
                }
                if selected == COMPLETED_FILTER {
                    return book.status == "completed";
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
            last_read_at: None,
            want_to_read: false,
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            cover_path: None,
            file_path: String::new(),
        }
    }

    #[test]
    fn recent_reading_uses_last_read_time_and_keeps_the_open_book() {
        let mut older = book("older", &[]);
        older.status = "reading".into();
        older.last_read_at = Some(10);
        let mut latest = book("latest", &[]);
        latest.status = "completed".into();
        latest.last_read_at = Some(20);
        let books = vec![older, latest];

        assert_eq!(recent_read_shelf_item(&books, None).unwrap().id, "latest");
        assert_eq!(
            recent_read_shelf_item(&books, Some("older")).unwrap().id,
            "older"
        );
        assert_eq!(
            recent_read_shelf_item(&books, Some("deleted")).unwrap().id,
            "latest"
        );
        assert!(recent_read_shelf_item(&[book("unread", &[])], None).is_none());
    }

    #[test]
    fn narrow_shelf_cover_measurements_reserve_text_space() {
        for width in [280.0, 320.0, 390.0, 844.0] {
            let cover = shelf_list_cover_size(1000.0, width);
            let text_width = width - 48.0 - 24.0 - 28.0 - 40.0 - cover.x;
            assert!(text_width >= 80.0);
            assert!((cover.x / cover.y - LIST_COVER_WIDTH_TO_HEIGHT).abs() < 0.001);
        }
        assert_eq!(shelf_list_cover_size(56.0, 390.0), dvec2(44.0, 56.0));
    }

    #[test]
    fn profile_book_filters_use_reading_status_and_keep_tags_separate() {
        let mut reading = book("a", &["completed"]);
        reading.status = "reading".into();
        let mut finished = book("b", &["reading"]);
        finished.status = "completed".into();
        let items = vec![reading, finished, book("c", &[])];
        assert_eq!(shelf_items_for_tag(&items, Some(READING_FILTER))[0].id, "a");
        assert_eq!(
            shelf_items_for_tag(&items, Some(COMPLETED_FILTER))[0].id,
            "b"
        );
        assert_eq!(shelf_items_for_tag(&items, Some("reading"))[0].id, "b");
        assert_eq!(shelf_items_for_tag(&items, None).len(), 3);
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
        assert_eq!(shelf_grid_columns_for_width(900.0), 4);
        assert_eq!(shelf_grid_columns_for_width(1200.0), 5);
        assert_eq!(shelf_grid_columns_for_width(1600.0), 6);
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
    fn chapter_page_count_is_not_a_page_turn_readiness_gate() {
        let mut pager = ReaderPagedState {
            spread_ready: true,
            needs_page_alignment: true,
            ..Default::default()
        };
        assert!(!pager.page_index.complete);
        assert!(pager.page_ready());
        pager.pending_source = Some((5, 20));
        assert!(!pager.page_ready());
        pager.pending_source = None;
        pager.requested_last_page = true;
        assert!(!pager.page_ready());
        pager.requested_last_page = false;
        pager.spread_ready = false;
        assert!(!pager.page_ready());
    }

    #[test]
    fn rapid_turns_coalesce_and_resume_once_the_page_and_animation_are_ready() {
        let mut pager = ReaderPagedState::default();
        for _ in 0..100 {
            pager.pending_turn = Some(true);
        }
        assert_eq!(pager.take_pending_turn(false), None);
        assert_eq!(pager.pending_turn, Some(true));
        pager.spread_ready = true;
        assert_eq!(pager.take_pending_turn(true), None);
        assert_eq!(pager.take_pending_turn(false), Some(true));
        assert_eq!(pager.take_pending_turn(false), None);
        assert!(!pager.page_index.complete);
    }

    #[test]
    fn reverse_a_quick_turn_before_the_chapter_index_is_ready() {
        use crate::reader_columns::PageAnchor;
        let mut pager = ReaderPagedState {
            spread_ready: true,
            anchor: PageAnchor { block: 50, y: 40.0 },
            pending_turn: Some(false),
            ..Default::default()
        };
        // An unvisited previous page must wait for a known boundary.
        assert_eq!(pager.take_pending_turn(false), None);
        pager.turn_history.push(PageAnchor { block: 49, y: 10.0 });
        assert_eq!(pager.take_pending_turn(false), Some(false));
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
static READER_BOOKMARKS: LazyLock<RwLock<Vec<readwise_core::books::Bookmark>>> =
    LazyLock::new(|| RwLock::new(Vec::new()));
static READER_SEARCH_RESULTS: LazyLock<RwLock<Vec<ReaderSearchHit>>> =
    LazyLock::new(|| RwLock::new(Vec::new()));
static READER_HIGHLIGHT_JUMPS: LazyLock<RwLock<Vec<ReaderSearchHit>>> =
    LazyLock::new(|| RwLock::new(Vec::new()));

#[derive(Debug, Clone)]
struct ReaderSearchHit {
    chapter: usize,
    block: usize,
    excerpt: String,
}
static OPEN_BOOK_REQUEST_ID: AtomicU64 = AtomicU64::new(0);
static READER_BOOK_PAGINATION_REVISION: AtomicU64 = AtomicU64::new(0);

fn reader_book_content_key() -> Option<(String, u64, usize)> {
    let book = CURRENT_BOOK.read().unwrap();
    let book = book.as_ref()?;
    let key = reader_source_hash(&format!(
        "{}:{}:{}:{}",
        book.id,
        OPEN_BOOK_REQUEST_ID.load(Ordering::Relaxed),
        READER_BOOK_PAGINATION_REVISION.load(Ordering::Relaxed),
        *GLOSS_ENABLED.read().unwrap()
    ));
    Some((book.id.clone(), key, book.chapters.len()))
}

/// Visible layout changes only when the displayed content or typography does.
/// A known-word save rebuilds book-wide page counts, but silencing an existing
/// ruby does not invalidate the active page's geometry or reading anchor.
fn reader_page_layout_signature(chapter: usize, content_version: u64, layout_key: u64) -> u64 {
    reader_source_hash(&format!(
        "{}:{chapter}:{content_version}:{layout_key}",
        OPEN_BOOK_REQUEST_ID.load(Ordering::Relaxed)
    ))
}

/// 当前打开的书对应的书架项。
static CURRENT_ITEM: LazyLock<RwLock<Option<ShelfItem>>> = LazyLock::new(|| RwLock::new(None));

fn requested_book_is_ready() -> bool {
    let requested_id = CURRENT_ITEM
        .read()
        .unwrap()
        .as_ref()
        .map(|item| item.id.clone());
    CURRENT_BOOK
        .read()
        .unwrap()
        .as_ref()
        .is_some_and(|book| requested_id.as_deref() == Some(book.id.as_str()))
}

/// 释义卡片操作结果（后台 → UI）。
#[derive(Debug)]
enum CardAction {
    VocabAdded(bool),
    MarkKnownSaved {
        lemma: String,
        known_count: Option<usize>,
    },
    MarkKnownFailed {
        lemma: String,
        book_id: Option<String>,
        error: String,
    },
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

fn spawn_mark_known(lemma: String, book_id: Option<String>) {
    std::thread::spawn(move || {
        let result = DB
            .get()
            .ok_or_else(|| "数据库尚未就绪".to_string())
            .and_then(|db| {
                db.add_known_word(&lemma, "marked")?;
                let known_count = match db.get_known_words() {
                    Ok(words) => Some(words.len()),
                    Err(error) => {
                        ::log::error!("刷新已认识单词数量失败: {}", error);
                        None
                    }
                };
                Ok(known_count)
            });
        match result {
            Ok(known_count) => Cx::post_action(CardAction::MarkKnownSaved { lemma, known_count }),
            Err(error) => {
                ::log::error!("标记已认识失败: {}", error);
                Cx::post_action(CardAction::MarkKnownFailed {
                    lemma,
                    book_id,
                    error,
                });
            }
        }
    });
}

/// 阅读字号（None = 用主题默认；A+/A- 调整后生效）。
static READER_FONT_SIZE: LazyLock<RwLock<Option<f32>>> = LazyLock::new(|| RwLock::new(None));
static READER_BOLD_TEXT: LazyLock<RwLock<bool>> = LazyLock::new(|| RwLock::new(false));
static READER_CHARACTER_SPACING: LazyLock<RwLock<f32>> = LazyLock::new(|| RwLock::new(0.0));
static READER_WORD_SPACING: LazyLock<RwLock<f32>> = LazyLock::new(|| RwLock::new(0.0));
static READER_MARGIN: LazyLock<RwLock<f32>> = LazyLock::new(|| RwLock::new(0.0));
static READER_JUSTIFY_TEXT: LazyLock<RwLock<bool>> = LazyLock::new(|| RwLock::new(false));
static READER_CUSTOMISE_OPEN: AtomicBool = AtomicBool::new(false);
static READER_APPEARANCE_OPEN: AtomicBool = AtomicBool::new(false);

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
static READER_COLUMNS: LazyLock<RwLock<ReaderColumns>> =
    LazyLock::new(|| RwLock::new(ReaderColumns::Auto));
static READER_TURN_MODE: LazyLock<RwLock<ReaderTurnMode>> =
    LazyLock::new(|| RwLock::new(ReaderTurnMode::Curl));
static READER_CONTENT_VERSION: AtomicU64 = AtomicU64::new(0);

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
    LazyLock::new(|| RwLock::new(ReaderFont::Original));

/// EPUB 正文行距；带注音段落始终比普通段落保留更多垂直空间。
static READER_SPACING: LazyLock<RwLock<ReaderSpacing>> =
    LazyLock::new(|| RwLock::new(ReaderSpacing::Default));

fn reader_palette(background: ReaderBackground, theme: ReaderTheme) -> (Vec4f, Vec4f) {
    let (surface, text) = match background {
        ReaderBackground::FollowTheme if theme == ReaderTheme::Light => (0xd9d9_d9ff, 0x3333_33ff),
        ReaderBackground::FollowTheme => (0x4d4d_4dff, 0xd9d9_d9ff),
        ReaderBackground::Original => (0xffffff_ff, 0x2020_20ff),
        ReaderBackground::Paper => (0xf7f4_eaff, 0x2b29_25ff),
        ReaderBackground::Sepia => (0xf2e3_c6ff, 0x3b30_25ff),
        ReaderBackground::Sage => (0xe7ef_e3ff, 0x2734_2bff),
        ReaderBackground::Bold => (0xfaf8_f2ff, 0x1717_17ff),
        ReaderBackground::Night => (0x2025_2bff, 0xd8d4_caff),
    };
    (Vec4f::from_u32(surface), Vec4f::from_u32(text))
}

pub(crate) fn current_reader_image_palette() -> (Vec4f, Vec4f) {
    reader_palette(
        *READER_BACKGROUND.read().unwrap(),
        *READER_THEME.read().unwrap(),
    )
}

pub(crate) fn current_reader_link_color(hovered: bool, pressed: bool) -> Vec4f {
    let background = *READER_BACKGROUND.read().unwrap();
    let theme = *READER_THEME.read().unwrap();
    let dark = background == ReaderBackground::Night
        || (background == ReaderBackground::FollowTheme && theme == ReaderTheme::Dark);
    let rgba = if dark {
        if pressed {
            0xff91_91ff
        } else if hovered {
            0xffc0_c0ff
        } else {
            0xffaa_aaff
        }
    } else if pressed {
        0x8e1c_27ff
    } else if hovered {
        0xc02a_35ff
    } else {
        0xa923_2eff
    };
    Vec4f::from_u32(rgba)
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
        ReaderBackground::Original => (
            0xf3f3_f3ff,
            0xd8d8_d8ff,
            0x6666_66ff,
            0x365f_8dff,
            0x2222_2214,
            0x2222_2228,
        ),
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
        ReaderBackground::Bold => (
            0xeae8_e2ff,
            0xbdb9_afff,
            0x5555_55ff,
            0x2b2b_2bff,
            0x1111_1118,
            0x1111_112e,
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

fn apply_reader_button_palette_with_focus(
    cx: &mut Cx,
    mut widget: WidgetRef,
    palette: ReaderScenePalette,
    focus_background: Vec4f,
) {
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
            color_focus: #(focus_background)
            border_color: #(border)
            border_color_hover: #(border)
            border_color_down: #(border)
            border_color_focus: #(border)
        }
    });
}

fn apply_reader_button_palette(cx: &mut Cx, widget: WidgetRef, palette: ReaderScenePalette) {
    apply_reader_button_palette_with_focus(cx, widget, palette, palette.hover);
}

// Flat navigation/entry rows should not inherit the input-field border and fill.
fn apply_flat_entry_background(cx: &mut Cx, mut widget: WidgetRef, palette: ReaderScenePalette) {
    let clear = vec4(0.0, 0.0, 0.0, 0.0);
    let hover = palette.hover;
    let pressed = palette.pressed;
    script_apply_eval!(cx, widget, {draw_bg +: {
        color: #(clear) color_hover: #(hover) color_down: #(pressed) color_focus: #(clear)
        border_color: #(clear) border_color_hover: #(clear) border_color_down: #(clear) border_color_focus: #(clear)
    }});
}

fn apply_reader_icon_palette(cx: &mut Cx, mut widget: WidgetRef, color: Vec4f) {
    script_apply_eval!(cx, widget, { draw_icon +: { color: #(color) } });
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

fn apply_library_control_palette(cx: &mut Cx, mut widget: WidgetRef, palette: ReaderScenePalette) {
    let surface = palette.surface;
    let text = palette.text;
    let muted = palette.muted;
    let border = palette.border;
    let accent = palette.accent;
    let hover = palette.hover;
    let pressed = palette.pressed;
    // All library header controls belong to the page, not the popup layer.
    // Apply every state after settings load/reapply; the initial template can
    // otherwise retain the startup paper palette instead of the saved one.
    script_apply_eval!(cx, widget, {
        draw_bg +: {
            color: #(surface)
            color_active: #(surface)
            color_focus: #(surface)
            color_disabled: #(surface)
            color_hover: #(hover)
            color_down: #(pressed)
            border_color: #(border)
            border_color_active: #(accent)
            border_color_2: #(border)
            border_color_hover: #(accent)
            border_color_2_hover: #(accent)
            border_color_focus: #(accent)
            border_color_2_focus: #(accent)
            border_color_down: #(accent)
            border_color_2_down: #(accent)
            border_color_disabled: #(border)
            border_color_2_disabled: #(border)
            arrow_color: #(text)
            arrow_color_hover: #(text)
            arrow_color_focus: #(text)
            arrow_color_down: #(text)
            arrow_color_disabled: #(muted)
        }
        draw_text +: {
            color: #(text)
            color_active: #(accent)
            color_hover: #(text)
            color_focus: #(text)
            color_down: #(text)
            color_disabled: #(muted)
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
            AppPage::ReadingHome,
            AppPage::Profile,
            AppPage::Notes,
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
    chapter_paths: Vec<String>,
    figure_targets: Vec<HashMap<String, String>>,
    /// 按访问惰性生成、已切块的注音章节；关闭注音时仍从原始章节投影。
    /// 直接缓存 PortalList 所需的块，避免每次设置刷新或旋转恢复都重新 split_blocks。
    glossed_chapter_chunks: Vec<Option<Vec<String>>>,
    glosses: HashMap<String, String>,
    known_words: HashSet<String>,
    /// EPUB navigation entries, including anchors within a spine document.
    toc: Vec<readwise_core::books::epub_meta::TocEntry>,
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
        if !self.known_words.insert(lemma.to_string()) {
            return;
        }
        READER_BOOK_PAGINATION_REVISION.fetch_add(1, Ordering::Relaxed);
        for chunks in self.glossed_chapter_chunks.iter_mut().flatten() {
            // Keep visited chapters' ruby slots and line spacing stable, just
            // like the visible projection. Pagination preparation must reuse
            // these chunks rather than measuring a differently shaped chapter.
            silence_paragraph_glosses(chunks, lemma);
        }
    }

    fn unmark_word_known(&mut self, lemma: &str) {
        if self.known_words.remove(lemma) {
            READER_BOOK_PAGINATION_REVISION.fetch_add(1, Ordering::Relaxed);
            // A failed save must rebuild cached annotations from the original
            // chapter HTML; the in-place removal above cannot be reversed.
            self.glossed_chapter_chunks.fill(None);
        }
    }

    fn take_highlights(
        &mut self,
        chapter: usize,
        ids: &[String],
    ) -> Vec<(usize, readwise_core::books::Highlight)> {
        let wanted = ids.iter().map(String::as_str).collect::<HashSet<_>>();
        let mut removed = Vec::new();
        let mut empty = false;
        if let Some(highlights) = self.highlights.get_mut(&chapter) {
            let mut index = 0;
            highlights.retain(|highlight| {
                let take = wanted.contains(highlight.id.as_str());
                if take {
                    removed.push((index, highlight.clone()));
                }
                index += 1;
                !take
            });
            empty = highlights.is_empty();
        }
        if empty {
            self.highlights.remove(&chapter);
        }
        removed
    }

    fn restore_highlights(
        &mut self,
        chapter: usize,
        removed: &[(usize, readwise_core::books::Highlight)],
    ) -> bool {
        if removed.is_empty() {
            return false;
        }
        let highlights = self.highlights.entry(chapter).or_default();
        let mut restored = false;
        for (index, highlight) in removed {
            if !highlights.iter().any(|item| item.id == highlight.id) {
                highlights.insert((*index).min(highlights.len()), highlight.clone());
                restored = true;
            }
        }
        restored
    }
}

#[derive(Debug, PartialEq)]
enum ReaderLinkTarget {
    External(String),
    Image(String),
    Chapter {
        index: usize,
        fragment: Option<String>,
    },
    Unsupported,
}

fn normalized_epub_path(path: &str) -> String {
    let mut parts = Vec::new();
    for part in path.replace('\\', "/").split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            value => parts.push(value.to_string()),
        }
    }
    parts.join("/")
}

fn chapter_link_target(
    book: &OpenBook,
    index: usize,
    fragment: Option<String>,
) -> ReaderLinkTarget {
    if let Some(image) = fragment
        .as_deref()
        .and_then(|id| book.figure_targets.get(index)?.get(id))
    {
        ReaderLinkTarget::Image(image.clone())
    } else {
        ReaderLinkTarget::Chapter { index, fragment }
    }
}

fn resolve_reader_link(book: &OpenBook, current: usize, href: &str) -> ReaderLinkTarget {
    let href = href.trim();
    if href.starts_with("https://") || href.starts_with("http://") || href.starts_with("mailto:") {
        return ReaderLinkTarget::External(href.to_string());
    }
    if href.is_empty() || href.starts_with('/') || href.contains(':') {
        return ReaderLinkTarget::Unsupported;
    }
    let (path, fragment) = href
        .split_once('#')
        .map_or((href, None), |(path, fragment)| {
            (
                path,
                (!fragment.is_empty()).then(|| percent_decode_path(fragment)),
            )
        });
    let path = path.split('?').next().unwrap_or(path);
    if path.is_empty() {
        return chapter_link_target(book, current, fragment);
    }
    let relative = percent_decode_path(path);
    let base = book
        .chapter_paths
        .get(current)
        .and_then(|path| path.rsplit_once('/'))
        .map_or("", |(parent, _)| parent);
    let target = normalized_epub_path(&format!("{base}/{relative}"));
    book.chapter_paths
        .iter()
        .position(|path| normalized_epub_path(path) == target)
        .map_or(ReaderLinkTarget::Unsupported, |index| {
            chapter_link_target(book, index, fragment)
        })
}

fn reader_anchor_block(chunks: &[String], fragment: &str) -> usize {
    let quoted = [
        format!("id=\"{fragment}\""),
        format!("id='{fragment}'"),
        format!("name=\"{fragment}\""),
        format!("name='{fragment}'"),
    ];
    chunks
        .iter()
        .position(|block| quoted.iter().any(|needle| block.contains(needle)))
        .unwrap_or(0)
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
            chapter_paths: vec!["Text/one.xhtml".into(), "Text/two.xhtml".into()],
            figure_targets: vec![HashMap::new(), HashMap::new()],
            glossed_chapter_chunks: vec![None, None],
            glosses: HashMap::from([
                ("solitude".to_string(), "孤独".to_string()),
                ("murmur".to_string(), "低语".to_string()),
            ]),
            known_words: HashSet::new(),
            toc: Vec::new(),
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
    fn newly_known_words_are_silenced_without_changing_cached_ruby_slots() {
        let mut book = book();
        let before = book.chapter_chunks(0, true).unwrap();

        book.mark_word_known("solitude");

        let after = book.chapter_chunks(0, true).unwrap();
        assert_eq!(before.len(), after.len());
        assert_eq!(
            readwise_core::vocab::annotate::rendered_text(&before.join("")),
            readwise_core::vocab::annotate::rendered_text(&after.join(""))
        );
        assert!(after.join("").contains("<ruby>solitude\u{e000}</ruby>"));
        assert!(!after.join("").contains("孤独"));
        assert_eq!(
            pagination_chapter_chunks(
                &book.chapters[0],
                book.glossed_chapter_chunks[0].clone(),
                true,
                &book.glosses,
                &book.known_words,
            ),
            after
        );
        // The annotation switch must still project the untouched source HTML.
        assert_eq!(
            pagination_chapter_chunks(
                &book.chapters[0],
                book.glossed_chapter_chunks[0].clone(),
                false,
                &book.glosses,
                &book.known_words,
            ),
            vec![book.chapters[0].clone()]
        );
    }

    #[test]
    fn known_word_recount_does_not_change_visible_layout_signature() {
        let mut book = book();
        let mut paragraphs = book.chapter_chunks(0, true).unwrap();
        let signature = reader_page_layout_signature(0, 7, 10);
        let revision = READER_BOOK_PAGINATION_REVISION.load(Ordering::Relaxed);

        book.mark_word_known("solitude");
        assert!(silence_paragraph_glosses(&mut paragraphs, "solitude"));
        assert_ne!(
            READER_BOOK_PAGINATION_REVISION.load(Ordering::Relaxed),
            revision
        );
        assert_eq!(reader_page_layout_signature(0, 7, 10), signature);
        assert_eq!(paragraphs, book.chapter_chunks(0, true).unwrap());

        // Actual text, chapter, or typography changes must still remeasure.
        assert_ne!(reader_page_layout_signature(0, 8, 10), signature);
        assert_ne!(reader_page_layout_signature(1, 7, 10), signature);
        assert_ne!(reader_page_layout_signature(0, 7, 11), signature);
    }

    #[test]
    fn failed_known_word_save_restores_cached_glosses() {
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
            .contains("孤独"));

        book.unmark_word_known("solitude");
        assert!(book
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

    #[test]
    fn reader_links_resolve_external_relative_and_fragment_targets() {
        let mut book = book();
        book.figure_targets[0].insert("fig3-1".into(), "/books/res/fig3-1.png".into());
        book.figure_targets[1].insert("fig3-2".into(), "/books/res/fig3-2.png".into());
        assert_eq!(
            resolve_reader_link(&book, 0, "https://example.org/"),
            ReaderLinkTarget::External("https://example.org/".into())
        );
        assert_eq!(
            resolve_reader_link(&book, 0, "two.xhtml#section"),
            ReaderLinkTarget::Chapter {
                index: 1,
                fragment: Some("section".into())
            }
        );
        assert_eq!(
            resolve_reader_link(&book, 1, "#part"),
            ReaderLinkTarget::Chapter {
                index: 1,
                fragment: Some("part".into())
            }
        );
        assert_eq!(
            resolve_reader_link(&book, 0, "#fig3-1"),
            ReaderLinkTarget::Image("/books/res/fig3-1.png".into())
        );
        assert_eq!(
            resolve_reader_link(&book, 0, "two.xhtml#fig3-2"),
            ReaderLinkTarget::Image("/books/res/fig3-2.png".into())
        );
        assert_eq!(
            resolve_reader_link(&book, 0, "javascript:bad()"),
            ReaderLinkTarget::Unsupported
        );
        assert_eq!(
            reader_anchor_block(
                &["<p>x</p>".into(), "<h2 id=\"part\">y</h2>".into()],
                "part"
            ),
            1
        );
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
/// A visible reader information card owns pointer and scroll input above the page.
static READER_INFO_POPUP_OPEN: AtomicBool = AtomicBool::new(false);

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

fn selection_popup_captures_pointer(
    popup_open: bool,
    selection_exists: bool,
    popup_rect: impl FnOnce() -> Rect,
    abs: DVec2,
) -> bool {
    popup_open && selection_exists && popup_rect().contains(abs)
}

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
    (dvec2(x, y.clamp(min_y, max_y)), popup_width)
}

fn reader_overlay_viewport(viewport: Rect, size: DVec2, safe: SafeAreaInsets) -> Rect {
    let left = viewport.pos.x.max(safe.left);
    let top = viewport.pos.y.max(safe.top);
    let right = (viewport.pos.x + viewport.size.x).min(size.x - safe.right);
    let bottom = (viewport.pos.y + viewport.size.y).min(size.y - safe.bottom);
    Rect {
        pos: dvec2(left, top),
        size: dvec2((right - left).max(0.0), (bottom - top).max(0.0)),
    }
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

fn library_card_release_opens_reader(
    menu_was_open: bool,
    release_is_over: bool,
    release_was_tap: bool,
) -> bool {
    !menu_was_open && release_is_over && release_was_tap
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

const NOTE_EXTRA_SEPARATOR: &str = "\n\n[readwise-note-extra]\n\n";

fn format_note_content(quote: &str, extra: &str) -> String {
    let quote = quote.trim();
    let extra = extra.trim();
    match (quote.is_empty(), extra.is_empty()) {
        (true, true) => String::new(),
        (false, true) => quote.to_string(),
        (true, false) => extra.to_string(),
        (false, false) => format!("{quote}{NOTE_EXTRA_SEPARATOR}{extra}"),
    }
}

fn split_note_content(content: &str) -> (&str, &str) {
    content
        .split_once(NOTE_EXTRA_SEPARATOR)
        .map(|(quote, extra)| (quote.trim(), extra.trim()))
        .unwrap_or((content.trim(), ""))
}

#[cfg(test)]
mod note_content_tests {
    use super::{format_note_content, split_note_content, NOTE_EXTRA_SEPARATOR};

    #[test]
    fn note_content_keeps_quote_separate_from_editable_body() {
        let content = format_note_content("  原文第一段\n原文第二段  ", "  我的理解  ");
        assert_eq!(
            content,
            format!("原文第一段\n原文第二段{NOTE_EXTRA_SEPARATOR}我的理解")
        );
        assert_eq!(
            split_note_content(&content),
            ("原文第一段\n原文第二段", "我的理解")
        );
    }

    #[test]
    fn legacy_note_content_is_shown_as_one_complete_quote() {
        assert_eq!(
            split_note_content("  旧笔记\n仍是完整引用  "),
            ("旧笔记\n仍是完整引用", "")
        );
    }
}

fn spawn_create_note(item: ShelfItem, quote: String, extra: String) {
    let english = app_language() == AppLanguage::English;
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        let content = format_note_content(&quote, &extra);
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

fn spawn_update_note(book_id: String, note_id: String, content: String) {
    let english = app_language() == AppLanguage::English;
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        if let Err(e) = db.update_note(&note_id, &content) {
            Cx::post_action(NoteAction::Error(e));
            return;
        }
        match db.get_notes_by_book(&book_id) {
            Ok(notes) => Cx::post_action(NoteAction::Refreshed {
                book_id,
                notes,
                message: Some(if english {
                    "Note updated ✓".to_string()
                } else {
                    "笔记已更新 ✓".to_string()
                }),
                finish_composer: false,
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
    Saved {
        book_id: String,
        highlight: readwise_core::books::Highlight,
        chapter: usize,
    },
    Deleted {
        book_id: String,
        chapter: usize,
        ids: Vec<String>,
    },
    DeleteFailed {
        book_id: String,
        chapter: usize,
        removed: Vec<(usize, readwise_core::books::Highlight)>,
        error: String,
    },
    Error(String),
}

fn spawn_add_highlight(book_id: String, chapter: usize, range: TextRange) {
    std::thread::spawn(move || {
        let Some(db) = DB.get() else { return };
        let slot = *crate::mark_span::HL_SLOT.read().unwrap();
        let names = crate::mark_span::HL_COLOR_NAMES;
        let color = names[slot.min(names.len() - 1)];
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
                Cx::post_action(HlAction::Saved {
                    book_id,
                    highlight: hl,
                    chapter,
                });
            }
            Err(e) => Cx::post_action(HlAction::Error(e)),
        }
    });
}

fn spawn_delete_highlights(
    book_id: String,
    chapter: usize,
    ids: Vec<String>,
    removed: Vec<(usize, readwise_core::books::Highlight)>,
) {
    std::thread::spawn(move || {
        let result = DB
            .get()
            .ok_or_else(|| "数据库尚未就绪".to_string())
            .and_then(|db| db.delete_highlights(&book_id, &ids));
        match result {
            Ok(_) => Cx::post_action(HlAction::Deleted {
                book_id,
                chapter,
                ids,
            }),
            Err(error) => Cx::post_action(HlAction::DeleteFailed {
                book_id,
                chapter,
                removed,
                error,
            }),
        }
    });
}

/// 当前章下标（App 写、TocList 读，做高亮）。
static CURRENT_CHAPTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// First visible paragraph, used only to restore an Android same-process App rebuild.
static CURRENT_READER_BLOCK: AtomicUsize = AtomicUsize::new(0);
/// Exact offset inside the first visible paragraph, stored as `f64::to_bits()`.
static CURRENT_READER_SCROLL_BITS: AtomicU64 = AtomicU64::new(0.0_f64.to_bits());
static CURRENT_READER_SOURCE: AtomicUsize = AtomicUsize::new(usize::MAX);

/// 打开书的结果（后台 → UI）。
#[derive(Debug)]
enum ReaderAction {
    Opened {
        request_id: u64,
        resume: ReaderLocation,
        bookmarks: Vec<readwise_core::books::Bookmark>,
        gloss_count: usize,
        parse_ms: u128,
        initial_gloss_ms: u128,
    },
    OpenError {
        request_id: u64,
        error: String,
    },
}

#[derive(Debug)]
enum BookmarkAction {
    Updated {
        book_id: String,
        bookmarks: Vec<readwise_core::books::Bookmark>,
    },
    Error {
        book_id: String,
        error: String,
    },
}

#[derive(Debug)]
enum ReaderSearchAction {
    Results {
        book_id: String,
        query: String,
        hits: Vec<ReaderSearchHit>,
    },
}

fn spawn_reader_search(book_id: String, query: String) {
    std::thread::spawn(move || {
        let chapters = CURRENT_BOOK
            .read()
            .unwrap()
            .as_ref()
            .filter(|book| book.id == book_id)
            .map(|book| book.chapters.clone());
        let mut hits = Vec::new();
        if let Some(chapters) = chapters {
            let needle = query.trim().to_lowercase();
            if !needle.is_empty() {
                for (chapter, html) in chapters.iter().enumerate() {
                    for (block, fragment) in readwise_core::books::epub_meta::split_blocks(html)
                        .iter()
                        .enumerate()
                    {
                        let plain = readwise_core::vocab::annotate::rendered_text(fragment);
                        if plain.to_lowercase().contains(&needle) {
                            let excerpt = plain.split_whitespace().collect::<Vec<_>>().join(" ");
                            hits.push(ReaderSearchHit {
                                chapter,
                                block,
                                excerpt: excerpt.chars().take(120).collect(),
                            });
                            if hits.len() >= 100 {
                                break;
                            }
                        }
                    }
                    if hits.len() >= 100 {
                        break;
                    }
                }
            }
        }
        Cx::post_action(ReaderSearchAction::Results {
            book_id,
            query,
            hits,
        });
    });
}

fn spawn_bookmark_change(
    book_id: String,
    remove_id: Option<String>,
    location: String,
    label: String,
) {
    std::thread::spawn(move || {
        let result = DB
            .get()
            .ok_or_else(|| "数据库尚未就绪".to_string())
            .and_then(|db| {
                if let Some(id) = remove_id {
                    db.delete_bookmark(&book_id, &id)?;
                } else {
                    db.add_bookmark(&book_id, &location, &label)?;
                }
                db.get_bookmarks(&book_id)
            });
        match result {
            Ok(bookmarks) => Cx::post_action(BookmarkAction::Updated { book_id, bookmarks }),
            Err(error) => Cx::post_action(BookmarkAction::Error { book_id, error }),
        }
    });
}

#[derive(Debug)]
enum ReaderPaginationAction {
    Ready {
        book_id: String,
        content: Arc<crate::reader_book_pages::BookContent>,
    },
}

fn pagination_chapter_chunks(
    html: &str,
    cached: Option<Vec<String>>,
    gloss_enabled: bool,
    glosses: &HashMap<String, String>,
    known_words: &HashSet<String>,
) -> Vec<String> {
    if let Some(cached) = cached.filter(|_| gloss_enabled) {
        return cached;
    }
    let html = if gloss_enabled && !glosses.is_empty() {
        use readwise_core::vocab::annotate::{annotate_html, flatten_lists};
        annotate_html(&flatten_lists(html), glosses, known_words)
    } else {
        html.to_string()
    };
    readwise_core::books::epub_meta::split_blocks(&html)
}

/// Prepare the same chapter HTML used by the reader off the rendering thread.
/// Glyph layout is then measured incrementally with the display renderer.
fn spawn_build_reader_pagination(book_id: String, content_key: u64) {
    std::thread::spawn(move || {
        let (chapter_count, glosses, known_words) = {
            let guard = CURRENT_BOOK.read().unwrap();
            let Some(book) = guard.as_ref().filter(|book| book.id == book_id) else {
                return;
            };
            (
                book.chapters.len(),
                book.glosses.clone(),
                book.known_words.clone(),
            )
        };
        let gloss_enabled = *GLOSS_ENABLED.read().unwrap();
        let mut chapters = Vec::with_capacity(chapter_count);
        for chapter in 0..chapter_count {
            if reader_book_content_key().is_none_or(|(_, key, _)| key != content_key) {
                return;
            }
            let (html, cached) = {
                let guard = CURRENT_BOOK.read().unwrap();
                let Some(book) = guard.as_ref().filter(|book| book.id == book_id) else {
                    return;
                };
                let Some(html) = book.chapters.get(chapter) else {
                    return;
                };
                (html.clone(), book.glossed_chapter_chunks[chapter].clone())
            };
            let mut paragraphs =
                pagination_chapter_chunks(&html, cached, gloss_enabled, &glosses, &known_words);
            if paragraphs.is_empty() {
                paragraphs.push(String::new());
            }
            let codes = paragraphs
                .iter()
                .map(|html| readwise_core::books::epub_meta::code_block_text(html))
                .collect();
            chapters.push(crate::reader_book_pages::ChapterContent { paragraphs, codes });
        }
        Cx::post_action(ReaderPaginationAction::Ready {
            book_id,
            content: Arc::new(crate::reader_book_pages::BookContent {
                key: content_key,
                chapters,
            }),
        });
    });
}

/// 持久化阅读位置：兼容旧章节号和 v2 像素偏移；v3 使用原文 UTF-8 偏移，
/// 字号、分栏和横竖屏重排后恢复到包含该字符的行。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ReaderLocation {
    chapter: usize,
    block: usize,
    scroll: f64,
    source: Option<usize>,
}

impl ReaderLocation {
    fn encode(self) -> String {
        let scroll = if self.scroll.is_finite() {
            self.scroll
        } else {
            0.0
        };
        if let Some(source) = self.source {
            format!("v3:{}:{}:{source}", self.chapter, self.block)
        } else {
            format!("v2:{}:{}:{scroll}", self.chapter, self.block)
        }
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
        let v3 = value.starts_with("v3:");
        let Some(encoded) = value
            .strip_prefix("v2:")
            .or_else(|| value.strip_prefix("v3:"))
        else {
            return legacy();
        };
        let mut fields = encoded.split(':');
        let Some(chapter) = fields.next().and_then(|field| field.parse::<usize>().ok()) else {
            return legacy();
        };
        let Some(block) = fields.next().and_then(|field| field.parse::<usize>().ok()) else {
            return legacy();
        };
        let Some(position) = fields.next() else {
            return legacy();
        };
        let source = if v3 {
            position.parse::<usize>().ok()
        } else {
            None
        };
        if v3 && source.is_none() {
            return legacy();
        }
        let Some(scroll) = (if v3 {
            Some(0.0)
        } else {
            position.parse::<f64>().ok()
        }) else {
            return legacy();
        };
        if fields.next().is_some() || chapter >= chapter_count || !scroll.is_finite() {
            return legacy();
        }
        Self {
            chapter,
            block,
            scroll,
            source,
        }
    }
}

fn bookmark_covers_location(bookmark: ReaderLocation, current: ReaderLocation) -> bool {
    bookmark.chapter == current.chapter
        && bookmark.block == current.block
        && bookmark
            .source
            .zip(current.source)
            .is_none_or(|(saved, now)| saved.abs_diff(now) < 256)
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
            source: None,
        };
        assert_eq!(ReaderLocation::decode(&location.encode(), 8), location);
    }

    #[test]
    fn reader_location_v3_preserves_source_anchor_independently_of_pixels() {
        let location = ReaderLocation {
            chapter: 3,
            block: 27,
            scroll: 0.0,
            source: Some(195),
        };
        assert_eq!(location.encode(), "v3:3:27:195");
        assert_eq!(ReaderLocation::decode(&location.encode(), 8), location);
        for value in ["v3:3:27:-1", "v3:3:27:NaN", "v3:3:27:1:2", "v3:8:27:1"] {
            assert_eq!(ReaderLocation::decode(value, 8), ReaderLocation::default());
        }
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
    let request_id = OPEN_BOOK_REQUEST_ID.fetch_add(1, Ordering::AcqRel) + 1;
    *CURRENT_ITEM.write().unwrap() = Some(item.clone());
    std::thread::spawn(move || {
        let open_started = std::time::Instant::now();
        let loaded = match readwise_core::books::epub_meta::load_chapters_with_toc(
            std::path::Path::new(&item.file_path),
        ) {
            Ok(list) => list,
            Err(e) => {
                Cx::post_action(ReaderAction::OpenError {
                    request_id,
                    error: e,
                });
                return;
            }
        };
        let mut chapters = Vec::with_capacity(loaded.chapters.len());
        let mut chapter_paths = Vec::with_capacity(loaded.chapters.len());
        let mut figure_targets = Vec::with_capacity(loaded.chapters.len());
        for info in loaded.chapters {
            chapter_paths.push(info.source_path);
            figure_targets.push(info.figure_targets);
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
        let bookmarks = DB
            .get()
            .and_then(|db| db.get_bookmarks(&item.id).ok())
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
            chapter_paths,
            figure_targets,
            toc: loaded.toc,
            glosses,
            known_words,
            highlights,
        };
        let gloss_started = std::time::Instant::now();
        if *GLOSS_ENABLED.read().unwrap() {
            let _ = book.chapter_chunks(resume.chapter, true);
        }
        let initial_gloss_ms = gloss_started.elapsed().as_millis();
        let requested_item = CURRENT_ITEM.read().unwrap();
        if OPEN_BOOK_REQUEST_ID.load(Ordering::Acquire) != request_id
            || requested_item
                .as_ref()
                .is_none_or(|item| item.id != book.id)
        {
            return;
        }
        *CURRENT_BOOK.write().unwrap() = Some(book);
        drop(requested_item);
        Cx::post_action(ReaderAction::Opened {
            request_id,
            resume,
            bookmarks,
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

    #[test]
    fn failed_delete_restores_only_removed_highlights_in_original_order() {
        let mut book = OpenBook {
            id: "book".into(),
            title: String::new(),
            chapters: Vec::new(),
            chapter_paths: Vec::new(),
            figure_targets: Vec::new(),
            glossed_chapter_chunks: Vec::new(),
            glosses: HashMap::new(),
            known_words: HashSet::new(),
            toc: Vec::new(),
            highlights: HashMap::from([(
                2,
                vec![
                    highlight("first", (0, 0), (0, 1)),
                    highlight("second", (0, 1), (0, 2)),
                    highlight("third", (0, 2), (0, 3)),
                ],
            )]),
        };
        let ids = vec!["first".to_string(), "third".to_string()];
        let removed = book.take_highlights(2, &ids);
        let remaining = book.highlights.get(&2).unwrap();
        assert_eq!(
            remaining
                .iter()
                .map(|hl| hl.id.as_str())
                .collect::<Vec<_>>(),
            vec!["second"]
        );
        assert!(book.restore_highlights(2, &removed));
        assert!(!book.restore_highlights(2, &removed));
        assert_eq!(
            book.highlights
                .get(&2)
                .unwrap()
                .iter()
                .map(|hl| hl.id.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second", "third"]
        );
    }
}

#[cfg(test)]
mod selection_popup_tests {
    use super::*;

    #[test]
    fn hidden_popup_never_reads_its_stale_draw_rect() {
        use std::cell::Cell;

        let stale_rect = Rect {
            pos: dvec2(100.0, 100.0),
            size: dvec2(520.0, 56.0),
        };
        let pointer = dvec2(200.0, 120.0);
        let area_read = Cell::new(false);
        assert!(!selection_popup_captures_pointer(
            false,
            true,
            || {
                area_read.set(true);
                stale_rect
            },
            pointer,
        ));
        assert!(!area_read.get());
        assert!(selection_popup_captures_pointer(
            true,
            true,
            || stale_rect,
            pointer,
        ));
        assert!(!selection_popup_captures_pointer(
            true,
            false,
            || stale_rect,
            pointer,
        ));
    }

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
    fn wrapped_selection_actions_stay_inside_mobile_safe_areas() {
        for (size, safe) in [
            (
                dvec2(320.0, 568.0),
                SafeAreaInsets {
                    top: 24.0,
                    bottom: 34.0,
                    ..SafeAreaInsets::default()
                },
            ),
            (
                dvec2(844.0, 390.0),
                SafeAreaInsets {
                    left: 44.0,
                    right: 44.0,
                    bottom: 21.0,
                    ..SafeAreaInsets::default()
                },
            ),
        ] {
            let viewport = reader_overlay_viewport(
                Rect {
                    pos: dvec2(0.0, 0.0),
                    size,
                },
                size,
                safe,
            );
            for anchor in [dvec2(0.0, 0.0), size * 0.5, size, size + dvec2(20.0, 20.0)] {
                for height in [88.0, 126.0] {
                    let (position, width) = selection_popup_geometry(viewport, anchor, height);
                    assert!(position.x >= safe.left);
                    assert!(position.y >= safe.top);
                    assert!(position.x + width <= size.x - safe.right);
                    assert!(position.y + height <= size.y - safe.bottom);
                }
            }
        }
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
        assert!(!library_card_release_opens_reader(true, true, true));
        assert!(!library_card_release_opens_reader(false, false, true));
        assert!(!library_card_release_opens_reader(false, true, false));
        assert!(library_card_release_opens_reader(false, true, true));
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
            last_read_at: b.last_read_at,
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

#[derive(Debug)]
struct LanTransferAction {
    session_id: String,
    notice: crate::lan_transfer::TransferNotice,
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

/// Non-modal aA content and the modal draft editor both own user input, while
/// draw/measure/reload events must still reach the chapter underneath them.
fn reader_colour_panel_blocks_event(
    appearance_open: bool,
    customise_open: bool,
    event: &Event,
) -> bool {
    (appearance_open || customise_open)
        && matches!(
            event,
            Event::MouseDown(_)
                | Event::MouseMove(_)
                | Event::MouseUp(_)
                | Event::Scroll(_)
                | Event::TouchUpdate(_)
                | Event::LongPress(_)
                | Event::KeyDown(_)
                | Event::KeyUp(_)
        )
}

/// A future highlighter choice has no effect on the geometry or existing marks.
fn reader_customise_change_is_highlight_only(
    current: &ReaderSettings,
    draft: &ReaderSettings,
) -> bool {
    let mut expected = current.clone();
    expected.highlight_color_slot = draft.highlight_color_slot;
    current.highlight_color_slot != draft.highlight_color_slot && expected == *draft
}

#[cfg(test)]
mod customise_highlight_tests {
    use super::*;

    #[test]
    fn appearance_scrolling_cannot_reach_the_chapter_but_reload_still_can() {
        use makepad_widgets::makepad_platform::event::{ScrollEvent, TouchUpdateEvent};
        let scroll = Event::Scroll(ScrollEvent {
            window_id: WindowId(0, 0),
            scroll: dvec2(0.0, 120.0),
            abs: dvec2(600.0, 200.0),
            modifiers: Default::default(),
            handled_x: Default::default(),
            handled_y: Default::default(),
            is_mouse: true,
            time: 1.0,
        });
        let touch = Event::TouchUpdate(TouchUpdateEvent {
            window_id: WindowId(0, 0),
            time: 1.0,
            modifiers: Default::default(),
            touches: vec![],
        });
        for (appearance, customise) in [(false, false), (true, false), (false, true), (true, true)]
        {
            for event in [&scroll, &touch] {
                assert_eq!(
                    reader_colour_panel_blocks_event(appearance, customise, event),
                    appearance || customise
                );
            }
            for event in [&Event::LiveEdit, &Event::ScriptReapply] {
                assert!(!reader_colour_panel_blocks_event(
                    appearance, customise, event
                ));
            }
        }
    }

    #[test]
    fn choosing_a_marker_needs_no_chapter_or_layout_rebuild() {
        let current = ReaderSettings::default();
        for slot in crate::mark_span::MARKER_SLOTS {
            let mut draft = current.clone();
            draft.highlight_color_slot = slot;
            assert_eq!(
                reader_customise_change_is_highlight_only(&current, &draft),
                slot != current.highlight_color_slot
            );
        }
    }

    #[test]
    fn mixed_layout_changes_still_use_the_full_settings_apply_path() {
        let current = ReaderSettings::default();
        let mut draft = current.clone();
        draft.highlight_color_slot = 7;
        draft.font_size = Some(18.0);
        assert!(!reader_customise_change_is_highlight_only(&current, &draft));
        draft.font_size = current.font_size;
        draft.gloss_enabled = !current.gloss_enabled;
        assert!(!reader_customise_change_is_highlight_only(&current, &draft));
    }
}

fn settings_swatch_rows(settings: &ReaderSettings) -> [([LiveId; 4], LiveId, ColorKind, usize); 6] {
    [
        (
            [id!(gc0), id!(gc1), id!(gc2), id!(gc3)],
            id!(gloss_color_name),
            ColorKind::Gloss,
            settings.gloss_color_slot,
        ),
        (
            [id!(gb0), id!(gb1), id!(gb2), id!(gb3)],
            id!(gloss_bg_name),
            ColorKind::Background,
            settings.gloss_background_slot,
        ),
        (
            [id!(wc0), id!(wc1), id!(wc2), id!(wc3)],
            id!(word_color_name),
            ColorKind::Word,
            settings.word_color_slot,
        ),
        (
            [id!(wb0), id!(wb1), id!(wb2), id!(wb3)],
            id!(word_bg_name),
            ColorKind::Background,
            settings.word_background_slot,
        ),
        (
            [id!(sc0), id!(sc1), id!(sc2), id!(sc3)],
            id!(selection_color_name),
            ColorKind::Selection,
            settings.selection_color_slot,
        ),
        (
            [id!(hc0), id!(hc1), id!(hc2), id!(hc3)],
            id!(highlight_color_name),
            ColorKind::Highlight,
            settings.highlight_color_slot,
        ),
    ]
}

fn current_reader_settings() -> ReaderSettings {
    use crate::ruby_word::*;

    ReaderSettings {
        language: *APP_LANGUAGE.read().unwrap(),
        library_view_mode: *LIBRARY_VIEW_MODE.read().unwrap(),
        theme: *READER_THEME.read().unwrap(),
        background: *READER_BACKGROUND.read().unwrap(),
        columns: *READER_COLUMNS.read().unwrap(),
        turn_mode: *READER_TURN_MODE.read().unwrap(),
        font: *READER_FONT.read().unwrap(),
        bold_text: *READER_BOLD_TEXT.read().unwrap(),
        spacing: *READER_SPACING.read().unwrap(),
        character_spacing: *READER_CHARACTER_SPACING.read().unwrap(),
        word_spacing: *READER_WORD_SPACING.read().unwrap(),
        margin: *READER_MARGIN.read().unwrap(),
        justify_text: *READER_JUSTIFY_TEXT.read().unwrap(),
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

    mod.widgets.PopupMenuPosition = #(PopupMenuPosition::script_api(vm))

    // Keep the slider's native drag handling while drawing the reading controls
    // as a quiet gray track, cyan progress, and an always-visible white thumb.
    let ReaderLayoutSlider = SliderMinimal{
        width: Fill height: 24
        margin: 0
        label_walk: Walk{width: 0 height: 0 margin: 0}
        text_input: TextInput{width: 0 height: 0}
        draw_bg +: {
            label_size: 0.
            pixel: fn() {
                let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                let radius = 8.
                let center_y = self.rect_size.y * 0.5
                let track_x = radius
                let track_width = max(0., self.rect_size.x - radius * 2.)
                let thumb_x = track_x + track_width * self.slide_pos

                sdf.box(track_x, center_y - 2.5, track_width, 5., 2.)
                sdf.fill(#b9c2c6)
                sdf.box(track_x, center_y - 2.5, max(0., thumb_x - track_x), 5., 2.)
                sdf.fill(#20b9c5)
                sdf.circle(thumb_x, center_y, radius)
                sdf.fill_keep(#fff)
                sdf.stroke(#cbd5d8, 1.)
                return sdf.result
            }
        }
    }

    mod.widgets.BookShelfCard = RoundedView{
        width: Fill height: Fit
        new_batch: true
        // Child buttons and top-level overlays must win hit testing. Enabling
        // overload here makes the card deliberately click through both.
        capture_overload: false
        cursor: MouseCursor.Hand
        flow: Right spacing: 14 padding: 12
        align: Align{ y: 0.5 }
        // Let the current reading surface show through, including after a
        // palette change or PortalList item reuse. Cards are not popup panels.
        draw_bg.color: theme.color_u_hidden
        draw_bg.border_radius: 8.0
        cover := View{
            // Use a real initial size; a zero-sized overlay with Fill children
            // can consume the row during cold-start measurement.
            width: 72 height: 92
            flow: Overlay
            cover_ph := SolidView{
                width: Fill height: Fill
                visible: false
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
            enable_long_press: true
            draw_text.text_style.font_size: 13
        }
    }

    mod.widgets.BookShelfGridCard = RoundedView{
        width: Fill height: Fit
        new_batch: true
        capture_overload: false
        cursor: MouseCursor.Hand
        flow: Down spacing: 5 padding: 10
        draw_bg.color: theme.color_u_hidden
        draw_bg.border_radius: 8.0
        cover := View{
            width: Fill height: 116
            flow: Overlay
            align: Center
            cover_ph := SolidView{
                width: Fill height: Fill
                visible: false
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
                fit: ImageFit.Smallest
            }
        }
        grid_status_row := View{
            width: Fill
            height: Fit
            flow: Right spacing: 4
            align: Align{y: 0.5}
            card_status := Label{
                width: Fill
                max_lines: 1
                text_overflow: Ellipsis
                text: ""
                draw_text.color: theme.color_highlight
                draw_text.text_style.font_size: 9
            }
            more_btn := ButtonFlatter{
                width: 34 height: 30 text: "•••"
                enable_long_press: true
                draw_text.text_style.font_size: 11
            }
        }
    }

    mod.widgets.BookShelfBase = #(BookShelf::register_widget(vm))
    mod.widgets.BookShelf = set_type_default() do mod.widgets.BookShelfBase{
        width: Fill height: Fill
        list := ShelfList{
            width: Fill height: Fill
            padding: Inset{ left: 24 right: 24 top: 8 bottom: 24 }
            spacing: 10
            scroll_bar: ScrollBar{}
            Card := mod.widgets.BookShelfCard{}
            CardAlternate := mod.widgets.BookShelfCard{}
            GridRow := View{
                width: Fill height: Fit
                flow: Right spacing: 8
                grid_cell_0 := View{
                    width: Fill height: Fit
                    grid_card := mod.widgets.BookShelfGridCard{}
                }
                grid_cell_1 := View{
                    width: Fill height: Fit
                    grid_card := mod.widgets.BookShelfGridCard{}
                }
                grid_cell_2 := View{
                    width: Fill height: Fit
                    grid_card := mod.widgets.BookShelfGridCard{}
                }
                grid_cell_3 := View{
                    width: Fill height: Fit
                    grid_card := mod.widgets.BookShelfGridCard{}
                }
                grid_cell_4 := View{
                    width: Fill height: Fit
                    grid_card := mod.widgets.BookShelfGridCard{}
                }
                grid_cell_5 := View{
                    width: Fill height: Fit
                    grid_card := mod.widgets.BookShelfGridCard{}
                }
            }
            GridRowAlternate := View{
                width: Fill height: Fit
                flow: Right spacing: 8
                grid_cell_0 := View{
                    width: Fill height: Fit
                    grid_card := mod.widgets.BookShelfGridCard{}
                }
                grid_cell_1 := View{
                    width: Fill height: Fit
                    grid_card := mod.widgets.BookShelfGridCard{}
                }
                grid_cell_2 := View{
                    width: Fill height: Fit
                    grid_card := mod.widgets.BookShelfGridCard{}
                }
                grid_cell_3 := View{
                    width: Fill height: Fit
                    grid_card := mod.widgets.BookShelfGridCard{}
                }
                grid_cell_4 := View{
                    width: Fill height: Fit
                    grid_card := mod.widgets.BookShelfGridCard{}
                }
                grid_cell_5 := View{
                    width: Fill height: Fit
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
        list := ShelfList{
            width: Fill height: Fill
            padding: Inset{ left: 8 right: 8 top: 8 bottom: 8 }
            spacing: 2
            // The reader owns its complete color scene. Keep the idle track
            // transparent and use a restrained, theme-independent handle while
            // it is hovered or dragged instead of inheriting the app's gray.
            scroll_bar: ScrollBar{
                draw_bg +: {
                    color: #0000
                    color_hover: #x66877c88
                    color_drag: #x4f776cbb
                    border_color: #0000
                    border_color_hover: #0000
                    border_color_drag: #0000
                }
            }
            TocItem := SolidView{
                width: Fill height: Fit
                flow: Overlay
                draw_bg.color: theme.color_bg_app
                toc_item_label := Label{
                    width: Fill height: Fit
                    padding: Inset{left: 12 right: 12 top: 8 bottom: 8}
                    text: ""
                }
                toc_item_btn := ButtonFlat{
                    width: Fill height: Fill
                    text: ""
                    enable_long_press: true
                    draw_bg +: {
                        color: #0000
                        color_hover: #x66877c20
                        color_down: #x4f776c30
                        color_focus: #x66877c20
                        border_color: #0000
                        border_color_hover: #0000
                        border_color_down: #0000
                        border_color_focus: #0000
                    }
                }
            }
        }
    }

    mod.widgets.ReaderJumpListBase = #(ReaderJumpList::register_widget(vm))
    mod.widgets.ReaderJumpList = set_type_default() do mod.widgets.ReaderJumpListBase{
        width: Fill height: Fill
        text_color: theme.color_text
        list := PortalList{
            width: Fill height: Fill
            padding: Inset{left: 8 right: 8 top: 8 bottom: 8}
            spacing: 5
            scroll_bar: ScrollBar{}
            JumpItem := RoundedView{
                width: Fill height: Fit
                new_batch: true
                flow: Overlay
                draw_bg.color: theme.color_bg_container
                draw_bg.border_radius: 7.0
                jump_label := Label{
                    width: Fill height: Fit
                    padding: 10
                    max_lines: 3
                    text_overflow: Ellipsis
                    text: ""
                    draw_text.text_style.font_size: 10
                }
                jump_btn := ButtonFlatter{
                    width: Fill height: Fill
                    text: ""
                    draw_bg.color: #0000
                    draw_bg.color_hover: #x66877c20
                    draw_bg.color_down: #x66877c30
                    draw_bg.border_color: #0000
                    draw_bg.border_color_hover: #0000
                    draw_bg.border_color_down: #0000
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
        note_bound_id := Label{visible: false text: ""}
        note_quote_title := Label{
            width: Fill
            text: "引用"
            draw_text.color: theme.color_highlight
            draw_text.text_style.font_size: 9
        }
        note_quote := TextInput{
            width: Fill
            height: Fit
            is_multiline: true
            is_read_only: true
            draw_text.text_style.font_size: 10
        }
        note_body_title := Label{
            width: Fill
            text: "笔记"
            draw_text.color: theme.color_highlight
            draw_text.text_style.font_size: 9
        }
        note_body_input := TextInput{
            width: Fill
            height: 54
            is_multiline: true
            empty_text: "补充笔记（可编辑）"
            scroll_bar: ScrollBar{
                draw_bg +: {
                    color: #0000
                    color_hover: #x66877c88
                    color_drag: #x4f776cbb
                    border_color: #0000
                    border_color_hover: #0000
                    border_color_drag: #0000
                }
            }
        }
        note_actions := View{
            width: Fill height: Fit
            flow: Right
            align: Align{x: 1.0 y: 0.5}
            note_save_btn := ButtonFlat{ text: "保存" }
            note_delete_btn := ButtonFlatter{ text: "删除" }
        }
    }

    let MobileTab = ButtonFlatter{
        width: Fill height: 58
        margin: 0 padding: 0
        flow: Down spacing: 4 align: Center
        icon_walk: Walk{width: 22 height: 22}
        label_walk: Walk{width: Fit height: Fit}
        draw_text.text_style.font_size: 10
    }
    let ProfileMetric = View{
        width: Fill height: 96
        flow: Overlay
        metric_button := ButtonFlat{
            width: Fill height: Fill
            margin: 0 padding: 0
            text: ""
            enable_long_press: true
            draw_bg.border_radius: 12.0
        }
        metric_content := View{
            width: Fill height: Fill
            flow: Down spacing: 8 padding: 12
            new_batch: true
            metric_heading := View{
                width: Fill height: 24
                flow: Right spacing: 8 align: Align{y: 0.5}
                metric_icon := Icon{
                    width: 18 height: 18
                    icon_walk: Walk{width: 18 height: 18}
                }
                metric_title := Label{
                    width: Fill height: Fit
                    text: ""
                    draw_text.text_style: theme.font_bold{font_size: 11}
                }
            }
            metric_description_row := View{
                width: Fill height: Fit padding: Inset{left: 26}
                metric_description := Label{
                    width: Fill height: Fit
                    text: ""
                    draw_text.text_style.font_size: 10
                }
            }
        }
    }
    let ProfileEntry = ButtonFlatter{
        width: Fill height: 54
        margin: 0 padding: Inset{left: 14 right: 14 top: 10 bottom: 10}
        spacing: 12 align: Align{x: 0.0 y: 0.5}
        enable_long_press: true
        icon_walk: Walk{width: 20 height: 20}
        draw_text.text_style.font_size: 12
    }
    mod.widgets.AllNotesListBase = #(AllNotesList::register_widget(vm))
    mod.widgets.AllNotesList = set_type_default() do mod.widgets.AllNotesListBase{
        width: Fill height: Fill
        list := PortalList{
            width: Fill height: Fill spacing: 12 padding: 16
            scroll_bar: ScrollBar{}
            Note := RoundedView{
                width: Fill height: Fit flow: Down spacing: 10 padding: 16
                new_batch: true draw_bg.border_radius: 12.0
                source := Label{width: Fill text: "" draw_text.text_style.font_size: 11}
                quote := Label{width: Fill height: Fit text: "" draw_text.text_style.font_size: 12}
                body := Label{width: Fill height: Fit text: "" draw_text.text_style.font_size: 12}
            }
        }
    }

    mod.widgets.NoteListBase = #(NoteList::register_widget(vm))
    mod.widgets.NoteList = set_type_default() do mod.widgets.NoteListBase{
        width: Fill height: Fill
        list := PortalList{
            width: Fill height: Fill
            spacing: 8
            // Text inputs own pointer drags for selecting and copying notes.
            // The list still scrolls with the wheel and its scrollbar.
            drag_scrolling: false
            scroll_bar: ScrollBar{
                draw_bg +: {
                    color: #0000
                    color_hover: #x66877c88
                    color_drag: #x4f776cbb
                    border_color: #0000
                    border_color_hover: #0000
                    border_color_drag: #0000
                }
            }
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

    let ChatUserTemplate = RoundedView{
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
    let ChatAssistantTemplate = RoundedView{
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
            User := ChatUserTemplate{}
            UserAlternate := ChatUserTemplate{}
            Assistant := ChatAssistantTemplate{}
            AssistantAlternate := ChatAssistantTemplate{}
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
            scroll_bar: ScrollBar{
                draw_bg +: {
                    // The default scrollbar track is an opaque light gray;
                    // let the reading surface show through instead.
                    color: #0000
                    color_hover: #x66877c88
                    color_drag: #x4f776cbb
                    border_color: #0000
                    border_color_hover: #0000
                    border_color_drag: #0000
                }
            }
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
            // Keep horizontal scrolling active without painting its handle.
            scroll_bars +: {scroll_bar_x +: {bar_size: 0.0}}
            draw_bg +: {color: #0000}
            draw_text +: {
                text_style +: {font_size: 10.5 line_spacing: 1.35}
            }
            // CodeView is read-only in the reader. Its upstream insertion caret
            // is always white, so suppress that editing affordance while retaining
            // the normal text pointer and selection interaction.
            draw_cursor +: {color: #0000}
            // Upstream mixes generic app-theme uniforms here; those can be too
            // close to an EPUB code surface. Preserve its connected-row geometry
            // but give it an explicit reader-grade contrast.
            draw_selection +: {
                color: uniform(#x6aa9ff88)
                pixel: fn() {
                    let sdf = Sdf2d.viewport(self.rect_pos + self.pos * self.rect_size)
                    sdf.box(
                        self.rect_pos.x,
                        self.rect_pos.y,
                        self.rect_size.x,
                        self.rect_size.y,
                        self.border_radius
                    )
                    if self.prev_w > 0.0 {
                        sdf.box(
                            self.prev_x,
                            self.rect_pos.y - self.rect_size.y,
                            self.prev_w,
                            self.rect_size.y,
                            self.border_radius
                        )
                        sdf.gloop(self.gloopiness)
                    }
                    if self.next_w > 0.0 {
                        sdf.box(
                            self.next_x,
                            self.rect_pos.y + self.rect_size.y,
                            self.next_w,
                            self.rect_size.y,
                            self.border_radius
                        )
                        sdf.gloop(self.gloopiness)
                    }
                    return sdf.fill(self.color)
                }
            }
        }
    }

    let ReaderCodeViewLight = ReaderCodeView{
        editor +: {
            draw_selection +: {color: uniform(#x3478c988)}
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

    // Style the CodeEditor's vertical scrollbar in the source-backed templates.
    // Applying this dynamically to portal-list items loses the script source
    // when the list recycles a row and can crash the renderer.
    let ReaderCodeViewPaper = ReaderCodeViewLight{editor +:{scroll_bars +:{
        scroll_bar_y +:{draw_bg +:{color:#d5cdbd color_hover:#716a60 color_drag:#805d35}}
    }}}
    let ReaderCodeViewSepia = ReaderCodeViewLight{editor +:{scroll_bars +:{
        scroll_bar_y +:{draw_bg +:{color:#ccb995 color_hover:#x75624e color_drag:#85562f}}
    }}}
    let ReaderCodeViewSage = ReaderCodeViewLight{editor +:{scroll_bars +:{
        scroll_bar_y +:{draw_bg +:{color:#becdb8 color_hover:#607065 color_drag:#3e6850}}
    }}}
    let ReaderCodeViewNight = ReaderCodeView{editor +:{scroll_bars +:{
        scroll_bar_y +:{draw_bg +:{color:#46505a color_hover:#9a9b99 color_drag:#b7c9a6}}
    }}}
    let ReaderCodeViewThemeLight = ReaderCodeViewLight{editor +:{scroll_bars +:{
        scroll_bar_y +:{draw_bg +:{color:#b9b9b9 color_hover:#686868 color_drag:#365f8d}}
    }}}
    let ReaderCodeViewThemeDark = ReaderCodeView{editor +:{scroll_bars +:{
        scroll_bar_y +:{draw_bg +:{color:#686868 color_hover:#aaaaaa color_drag:#b8c8dc}}
    }}}

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

    let ReaderPopoverArrow = View{
        width: 16 height: 10
        show_bg: true
        draw_bg +: {
            color: theme.color_bg_container
            border_color: theme.color_bg_container
            pixel: fn() {
                let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                // An open, softly curved outline joins the panel without
                // drawing a horizontal line across the pointer's base.
                sdf.move_to(0.8, self.rect_size.y + 0.8)
                sdf.line_to(5.4, 3.2)
                sdf.line_to(6.5, 1.9)
                sdf.line_to(7.15, 1.3)
                sdf.line_to(7.75, 1.05)
                sdf.line_to(8.25, 1.05)
                sdf.line_to(8.85, 1.3)
                sdf.line_to(9.5, 1.9)
                sdf.line_to(10.6, 3.2)
                sdf.line_to(self.rect_size.x - 0.8, self.rect_size.y + 0.8)
                sdf.stroke(self.border_color, 1.2)
                return sdf.result
            }
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
        font_preview_normal: theme.font_regular{}
        font_preview_italic: theme.font_italic{}
        font_preview_bold: theme.font_bold{}
        font_preview_bold_italic: theme.font_bold_italic{}
        font_active_normal: theme.font_regular{}
        font_active_italic: theme.font_italic{}
        font_active_bold: theme.font_bold{}
        font_active_bold_italic: theme.font_bold_italic{}
        html_default_inset: theme.space_1
        draw_reader_selection.color: #0000
        draw_page_turn: mod.draw.DrawReaderPageTurn{}
        list_host := View{
        width: Fill height: Fill
        list := PortalList{
            width: Fill height: Fill
            selectable: true
            // Desktop text selection owns pointer drags. Wheel/scrollbar
            // scrolling remains available; mobile devices enable touch drag.
            drag_scrolling: #(cfg!(any(target_os = "android", target_os = "ios")))
            padding: Inset{ left: 32 right: 32 top: 8 bottom: 8 }
            spacing: 6
            scroll_bar: ScrollBar{}
            Para := View{
                width: Fill height: Fit
                para_html := Html{
                    width: Fill height: Fit
                    selectable: true
                    a := mod.widgets.ReaderLink{}
                    img := mod.widgets.ChapterImage{}
                    mark := mod.widgets.MarkSpan{}
                    reader_run := mod.widgets.ReaderRun{}
                    reader_code := mod.widgets.ReaderInlineCode{}
                    readeralignon := mod.widgets.ReaderTextAlignOn{}
                    readeralignoff := mod.widgets.ReaderTextAlignOff{}
                }
            }
            GlossPara := View{
                width: Fill height: Fit
                para_html := Html{
                    width: Fill height: Fit
                    selectable: true
                    a := mod.widgets.ReaderLink{}
                    ruby := mod.widgets.RubyWord{}
                    reader_run := mod.widgets.ReaderRun{}
                    reader_code := mod.widgets.ReaderInlineCode{}
                    readeralignon := mod.widgets.ReaderTextAlignOn{}
                    readeralignoff := mod.widgets.ReaderTextAlignOff{}
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
                    a := mod.widgets.ReaderLink{}
                    img := mod.widgets.ChapterImage{}
                    mark := mod.widgets.MarkSpan{}
                    reader_code := mod.widgets.ReaderInlineCode{}
                    readeralignon := mod.widgets.ReaderTextAlignOn{}
                    readeralignoff := mod.widgets.ReaderTextAlignOff{}
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
                    a := mod.widgets.ReaderLink{}
                    ruby := mod.widgets.RubyWord{}
                    img := mod.widgets.ChapterImage{}
                    mark := mod.widgets.MarkSpan{}
                    reader_code := mod.widgets.ReaderInlineCode{}
                    readeralignon := mod.widgets.ReaderTextAlignOn{}
                    readeralignoff := mod.widgets.ReaderTextAlignOff{}
                }
            }
            CodePaper := ReaderCodePara{
                draw_bg +: {color: #f7f4ea border_color: #d8d1c2}
                code_view := ReaderCodeViewPaper{}
            }
            CodeSepia := ReaderCodePara{
                draw_bg +: {color: #f2e3c6 border_color: #ccb995}
                code_view := ReaderCodeViewSepia{}
            }
            CodeSage := ReaderCodePara{
                draw_bg +: {color: #e7efe3 border_color: #becdb8}
                code_view := ReaderCodeViewSage{}
            }
            CodeNight := ReaderCodePara{
                draw_bg +: {color: #20252b border_color: #46505a}
                code_view := ReaderCodeViewNight{}
            }
            CodeThemeLight := ReaderCodePara{
                draw_bg +: {color: #d9d9d9 border_color: #b9b9b9}
                code_view := ReaderCodeViewThemeLight{}
            }
            CodeThemeDark := ReaderCodePara{
                draw_bg +: {color: #4d4d4d border_color: #686868}
                code_view := ReaderCodeViewThemeDark{}
            }
        }
        }
    }

    startup() do #(App::script_component(vm)){
        ui: Root{
            main_window := Window{
                window.title: "ReadWise"
                window.inner_size: vec2(1080, 720)
                caption_bar +: {
                    new_batch: true
                    caption_label +: {
                        flow: Overlay
                        label +: {visible: false}
                        reader_titlebar := View{
                            visible: false
                            width: Fill height: Fill
                            flow: Right
                            align: Align{y: 0.5}
                            padding: Inset{left: 92 right: 18 top: 0 bottom: 4}
                            reader_titlebar_left := View{
                                width: 160 height: Fit
                                flow: Right spacing: 4
                                reader_titlebar_left_buttons := View{
                                    visible: false
                                    width: Fit height: Fit
                                    flow: Right spacing: 4
                                    titlebar_back_btn := ButtonFlatterIcon{
                                        width: 28 height: 28 icon_walk: Walk{width: 14 height: 14}
                                        draw_icon +: {svg: crate_resource("self:resources/icons/reader_back.svg")}
                                    }
                                    titlebar_toc_btn := ButtonFlatterIcon{
                                        width: 28 height: 28 icon_walk: Walk{width: 14 height: 14}
                                        draw_icon +: {svg: crate_resource("self:resources/icons/reader_contents.svg")}
                                    }
                                    titlebar_bookmarks_btn := ButtonFlatterIcon{
                                        width: 28 height: 28 icon_walk: Walk{width: 14 height: 14}
                                        draw_icon +: {svg: crate_resource("self:resources/icons/reader_bookmarks.svg")}
                                    }
                                    titlebar_notes_btn := ButtonFlatterIcon{
                                        width: 28 height: 28 icon_walk: Walk{width: 14 height: 14}
                                        draw_icon +: {svg: crate_resource("self:resources/icons/reader_notes.svg")}
                                    }
                                }
                            }
                            reader_titlebar_title_slot := View{
                                width: Fill height: Fit
                                align: Center
                                titlebar_reader_title := Label{
                                    text: "阅读"
                                    draw_text.text_style.font_size: 14
                                }
                            }
                            reader_titlebar_right := View{
                                width: 234 height: Fit
                                flow: Right spacing: 4
                                align: Align{x: 1.0 y: 0.5}
                                reader_titlebar_right_buttons := View{
                                    visible: false
                                    width: Fit height: Fit
                                    flow: Right spacing: 4
                                    titlebar_appearance_btn := ButtonFlatter{
                                        width: 28 height: 28 text: "aA"
                                        draw_text.text_style.font_size: 11
                                    }
                                    titlebar_search_btn := ButtonFlatterIcon{
                                        width: 28 height: 28 icon_walk: Walk{width: 14 height: 14}
                                        draw_icon +: {svg: crate_resource("self:resources/icons/reader_search.svg")}
                                    }
                                    titlebar_bookmark_btn := ButtonFlatterIcon{
                                        width: 28 height: 28 icon_walk: Walk{width: 14 height: 14}
                                        draw_icon +: {svg: crate_resource("self:resources/icons/reader_bookmark.svg")}
                                    }
                                    titlebar_bookmark_filled_btn := ButtonFlatterIcon{
                                        visible: false
                                        width: 28 height: 28 icon_walk: Walk{width: 14 height: 14}
                                        draw_icon +: {svg: crate_resource("self:resources/icons/reader_bookmark_filled.svg")}
                                    }
                                }
                            }
                        }
                    }
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
                        profile_back_bar := View{
                            visible: false width: Fill height: 48
                            flow: Right spacing: 8 padding: Inset{left: 12 right: 16 top: 4 bottom: 4}
                            align: Align{y: 0.5}
                            profile_back := ButtonFlatter{
                                text: "我" height: 40 margin: 0 spacing: 6
                                icon_walk: Walk{width: 16 height: 16}
                                draw_icon +: {svg: crate_resource("self:resources/icons/reader_back.svg")}
                            }
                            profile_detail_title := Label{width: Fill text: "" draw_text.text_style.font_size: 13}
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
                                    library_actions := View{
                                        width: Fit height: Fit flow: Right spacing: 10
                                        align: Align{y: 0.5}
                                    library_view_list := RadioButtonTabFlat{
                                        text: "列表"
                                        draw_bg.color: theme.color_bg_app
                                        draw_bg.color_active: theme.color_bg_app
                                    }
                                    library_view_grid := RadioButtonTabFlat{
                                        text: "网格"
                                        draw_bg.color: theme.color_bg_app
                                        draw_bg.color_active: theme.color_bg_app
                                    }
                                    import_epub_btn := ButtonFlat{
                                        visible: true
                                        text: "导入 EPUB"
                                        draw_bg.color: theme.color_bg_app
                                        draw_bg.color_focus: theme.color_bg_app
                                    }
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
                                        // DropDown defaults to TopLeft for Fit height.
                                        // Center the label within our fixed-height field.
                                        align: Align{x: 0.0 y: 0.5}
                                        padding: Inset{left: 12 right: 24 top: 0 bottom: 0}
                                        draw_bg.color: theme.color_bg_app
                                        draw_bg.color_focus: theme.color_bg_app
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
                            flow: Overlay
                            draw_bg.color: #xf7f4ea
                            reader_body := SolidView{
                                width: Fill height: Fill
                                flow: Overlay
                                draw_bg.color: #xf7f4ea
                                reader_columns := View{
                                    width: Fill height: Fill
                                    flow: Right
                                    reader_surface := SolidView{
                                        width: Fill height: Fill
                                        new_batch: true
                                        flow: Down
                                        draw_bg.color: #xf7f4ea
                                        chapter_list := mod.widgets.ChapterList{}
                                        reader_footer := View{
                                            width: Fill height: 48
                                            flow: Down
                                            reader_page_center_layer := View{
                                                width: Fill height: 24
                                                flow: Right
                                                reader_page_first := View{
                                                    width: Fill height: Fill align: Center
                                                    reader_page_indicator := mod.widgets.ReaderPageIndicator{}
                                                }
                                                reader_page_second := View{
                                                    visible: false
                                                    width: Fill height: Fill align: Center
                                                    reader_page_indicator_second := mod.widgets.ReaderPageIndicator{}
                                                }
                                            }
                                            reader_page_right_layer := View{
                                                width: Fill height: Fill
                                                flow: Right
                                                align: Align{y: 0.5}
                                                padding: Inset{left: 18 right: 18}
                                                status := Label{
                                                    width: Fill height: Fit
                                                    text: ""
                                                    draw_text.color: theme.color_label_inner_inactive
                                                    draw_text.text_style.font_size: 8.5
                                                }
                                                chapter_pos := Label{
                                                    width: Fit height: Fit
                                                    text: ""
                                                    draw_text.color: theme.color_label_inner_inactive
                                                    draw_text.text_style.font_size: 8.5
                                                }
                                                // The desktop status fills this row, but
                                                // is hidden on mobile. Keep a mobile spacer
                                                // so remaining pages stays at the right edge.
                                                reader_footer_spacer := View{
                                                    visible: false
                                                    width: Fill height: Fill
                                                }
                                                reader_pages_left := Label{
                                                    width: Fit height: Fit
                                                    text: ""
                                                    draw_text.color: theme.color_label_inner_inactive
                                                    draw_text.text_style.font_size: 8.5
                                                }
                                            }
                                        }
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
                                    reader_chat_list := mod.widgets.ChatList{
                                        reader_style: true
                                        list +: {
                                            padding: Inset{left: 4 right: 4 top: 12 bottom: 12}
                                        }
                                    }
                                    reader_ai_composer := View{
                                        width: Fill height: Fit
                                        flow: Down spacing: 8
                                        reader_chat_input := TextInput{
                                            width: Fill height: 56
                                            is_multiline: true
                                            submit_on_enter: true
                                            empty_text: "继续追问，按回车发送"
                                        }
                                        reader_ai_actions := View{
                                            width: Fill height: Fit
                                            flow: Flow.Right{wrap: true} spacing: 6
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
                                reader_panel_layer := View{
                                    width: Fill height: Fill
                                    flow: Overlay
                                    padding: Inset{left: 18 right: 18 top: 0 bottom: 8}
                                    toc_popup := View{
                                        visible: false
                                        width: 320 height: Fill
                                        flow: Down
                                        toc_arrow_row := View{
                                            width: Fill height: 10
                                            flow: Right
                                            padding: Inset{left: 112}
                                            toc_arrow := ReaderPopoverArrow{}
                                        }
                                        toc_panel := RoundedShadowView{
                                            width: Fill height: Fill{max: 620}
                                            new_batch: true
                                            flow: Down spacing: 8 padding: 12
                                            draw_bg.color: theme.color_bg_container
                                            draw_bg.border_radius: 12.0
                                            draw_bg.shadow_color: #0003
                                            draw_bg.shadow_radius: 14.0
                                            draw_bg.shadow_offset: vec2(0, 5)
                                            toc_header := View{
                                                width: Fill height: Fit
                                                flow: Right align: Align{y: 0.5}
                                                toc_title := Label{
                                                    width: Fill text: "目录"
                                                    draw_text.text_style.font_size: 14
                                                }
                                                toc_close_btn := ButtonFlatter{text: "关闭"}
                                            }
                                            toc_loading := Label{
                                                visible: false
                                                width: Fill height: Fit
                                                padding: 12
                                                text: "目录加载中…"
                                                draw_text.color: theme.color_label_inner_inactive
                                            }
                                            toc_list := mod.widgets.TocList{}
                                        }
                                    }
                                    reader_bookmarks_popup := View{
                                        visible: false
                                        width: 340 height: Fill
                                        flow: Down
                                        reader_bookmarks_arrow_row := View{
                                            width: Fill height: 10
                                            flow: Right
                                            padding: Inset{left: 144}
                                            reader_bookmarks_arrow := ReaderPopoverArrow{}
                                        }
                                        reader_bookmarks_panel := RoundedShadowView{
                                            width: Fill height: Fill{max: 620}
                                            new_batch: true
                                            flow: Down spacing: 8 padding: 12
                                            draw_bg.color: theme.color_bg_container
                                            draw_bg.border_radius: 12.0
                                            draw_bg.shadow_color: #0003
                                            draw_bg.shadow_radius: 14.0
                                            draw_bg.shadow_offset: vec2(0, 5)
                                            reader_bookmarks_header := View{
                                                width: Fill height: Fit
                                                flow: Right
                                                align: Align{y: 0.5}
                                                reader_bookmarks_title := Label{
                                                    width: Fill text: "Bookmarks"
                                                    draw_text.text_style.font_size: 14
                                                }
                                                reader_bookmarks_close := ButtonFlatter{text: "X"}
                                            }
                                            reader_bookmarks_hint := Label{
                                                width: Fill height: Fit
                                                text: "当前书籍还没有书签"
                                                draw_text.text_style.font_size: 9
                                            }
                                            reader_bookmark_list := mod.widgets.ReaderJumpList{bookmarks: true}
                                        }
                                    }
                                    note_popup := View{
                                        visible: false
                                        width: 380 height: Fill
                                        flow: Down
                                        note_arrow_row := View{
                                            width: Fill height: 10
                                            flow: Right
                                            padding: Inset{left: 176}
                                            note_arrow := ReaderPopoverArrow{}
                                        }
                                        note_panel := RoundedShadowView{
                                            width: Fill height: Fill{max: 620}
                                            new_batch: true
                                            flow: Down spacing: 10 padding: 12
                                            draw_bg.color: theme.color_bg_container
                                            draw_bg.border_radius: 12.0
                                            draw_bg.shadow_color: #0003
                                            draw_bg.shadow_radius: 14.0
                                            draw_bg.shadow_offset: vec2(0, 5)
                                            note_header := View{
                                                width: Fill height: Fit
                                                flow: Right spacing: 8
                                                align: Align{y: 0.5}
                                                note_panel_title := Label{
                                                    width: Fill
                                                    text: "高亮与笔记"
                                                    draw_text.text_style.font_size: 14
                                                }
                                                reader_ai_btn := ButtonFlatter{ text: "AI" }
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
                                                note_quote := TextInput{
                                                    width: Fill
                                                    height: Fit
                                                    is_multiline: true
                                                    is_read_only: true
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
                                            reader_highlights_title := Label{
                                                visible: false
                                                width: Fill height: Fit
                                                text: "Highlights"
                                                draw_text.text_style.font_size: 11
                                            }
                                            reader_highlight_list := mod.widgets.ReaderJumpList{
                                                visible: false
                                                highlights: true
                                                width: Fill height: 180
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
                                    }
                                    reader_search_popup_slot := View{
                                        visible: false
                                        width: Fill height: Fill
                                        flow: Overlay
                                        align: Align{x: 1.0 y: 0.0}
                                        padding: Inset{top: 8}
                                        reader_search_panel := RoundedShadowView{
                                            width: 320 height: Fill{max: 620}
                                            new_batch: true
                                            flow: Down spacing: 9 padding: 12
                                            draw_bg.color: theme.color_bg_container
                                            draw_bg.border_radius: 12.0
                                            draw_bg.shadow_color: #0003
                                            draw_bg.shadow_radius: 14.0
                                            draw_bg.shadow_offset: vec2(0, 5)
                                            reader_search_header := View{
                                                width: Fill height: Fit
                                                flow: Right
                                                align: Align{y: 0.5}
                                                reader_search_title := Label{
                                                    width: Fill text: "Search in Book"
                                                    draw_text.text_style.font_size: 14
                                                }
                                                reader_search_close := ButtonFlatter{text: "X"}
                                            }
                                            reader_search_input := TextInput{
                                                width: Fill height: 40
                                                empty_text: "搜索书中内容"
                                            }
                                            reader_search_submit := ButtonFlat{
                                                width: Fill height: 36 text: "搜索"
                                            }
                                            reader_search_hint := Label{
                                                width: Fill height: Fit
                                                text: "输入关键词后按回车搜索"
                                                draw_text.text_style.font_size: 9
                                            }
                                            reader_search_list := mod.widgets.ReaderJumpList{bookmarks: false}
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
                                        selection_actions := View{
                                            width: Fill height: Fit
                                            flow: Flow.Right{wrap: true} spacing: 6
                                            align: Align{y: 0.5}
                                            copy_sel_btn := ButtonFlat{height: 32 text: "复制"}
                                            note_from_sel_btn := ButtonFlat{height: 32 text: "笔记"}
                                            vocab_from_sel_btn := ButtonFlat{height: 32 text: "生词"}
                                            translate_sel_btn := ButtonFlat{height: 32 text: "翻译"}
                                            ask_ai_sel_btn := ButtonFlat{height: 32 text: "问 AI"}
                                            speak_sel_btn := ButtonFlat{height: 32 text: "发音"}
                                            hl_btn := ButtonFlat{height: 32 text: "高亮"}
                                            sel_close_btn := ButtonFlatter{height: 32 text: "关闭"}
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
                                reader_theme_layer := View{
                                    // Keep the overlay owner mounted so its first popup
                                    // frame has an Area that can be redrawn.
                                    width: Fill height: Fill
                                    flow: Overlay
                                    align: Align{x: 1.0 y: 0.0}
                                    padding: Inset{right: 18 top: 0}
                                    reader_theme_arrow_row := View{
                                        visible: false
                                        width: 316 height: 10
                                        flow: Right
                                        align: Align{x: 1.0}
                                        padding: Inset{right: 70}
                                        reader_theme_arrow := ReaderPopoverArrow{}
                                    }
                                    reader_theme_menu := RoundedShadowView{
                                        visible: false
                                        width: 316 height: 356
                                        margin: Inset{top: 10}
                                        new_batch: true
                                        flow: Down spacing: 12 padding: 16
                                        draw_bg.color: theme.color_bg_container
                                        draw_bg.border_radius: 14.0
                                        draw_bg.shadow_color: #0003
                                        draw_bg.shadow_radius: 16.0
                                        draw_bg.shadow_offset: vec2(0, 5)
                                        reader_theme_scroll := ScrollYView{
                                            width: Fill height: Fill
                                            flow: Down spacing: 12
                                            reader_theme_title_row := View{
                                                width: Fill height: Fit
                                                align: HCenter
                                                reader_theme_title := Label{
                                                    width: Fit height: Fit
                                                    text: "Themes & Settings"
                                                    draw_text.text_style.font_size: 15
                                                }
                                            }
                                            reader_theme_modes := View{
                                                width: Fill height: Fit
                                                flow: Flow.Right{wrap: true}
                                                spacing: 7
                                                reader_mode_original := ButtonFlat{width: 88 height: 42 text: "Original"}
                                                reader_mode_quiet := ButtonFlat{width: 88 height: 42 text: "Quiet"}
                                                reader_mode_paper := ButtonFlat{width: 88 height: 42 text: "Paper"}
                                                reader_mode_bold := ButtonFlat{width: 88 height: 42 text: "Bold"}
                                                reader_mode_calm := ButtonFlat{width: 88 height: 42 text: "Calm"}
                                                reader_mode_focus := ButtonFlat{width: 88 height: 42 text: "Focus"}
                                            }
                                            reader_quick_pen_row := View{
                                                width: Fill height: Fit
                                                flow: Flow.Right{wrap: true} spacing: 4 align: Align{y: 0.5}
                                                reader_quick_pen_title := View{
                                                    width: 56 height: 44 flow: Down align: Align{y: 0.5}
                                                    reader_quick_pen_label := mod.widgets.SettingsColorTitle{
                                                        width: Fill height: Fit margin: 0 padding: 0 text: "Pen"
                                                        draw_text.text_style.font_size: 11
                                                    }
                                                }
                                                reader_quick_pen_options := mod.widgets.SettingsColorPicker{
                                                    width: 224 spacing: 0
                                                    reader_quick_pen_vermilion := mod.widgets.SettingsColorSwatch{}
                                                    reader_quick_pen_cobalt := mod.widgets.SettingsColorSwatch{}
                                                    reader_quick_pen_moss := mod.widgets.SettingsColorSwatch{}
                                                    reader_quick_pen_graphite := mod.widgets.SettingsColorSwatch{}
                                                    reader_quick_pen_name := mod.widgets.SettingsColorName{width: 48 margin: 0 padding: 0}
                                                }
                                            }
                                            reader_quick_highlight_row := View{
                                                width: Fill height: Fit
                                                flow: Flow.Right{wrap: true} spacing: 4 align: Align{y: 0.5}
                                                reader_quick_highlight_title := View{
                                                    width: 56 height: 44 flow: Down align: Align{y: 0.5}
                                                    reader_quick_highlight_label := mod.widgets.SettingsColorTitle{
                                                        width: Fill height: Fit margin: 0 padding: 0 text: "Highlighter"
                                                        draw_text.text_style.font_size: 11
                                                    }
                                                }
                                                reader_quick_highlight_options := mod.widgets.SettingsColorPicker{
                                                    width: 224 spacing: 0
                                                    reader_quick_highlight_yellow := mod.widgets.SettingsColorSwatch{}
                                                    reader_quick_highlight_green := mod.widgets.SettingsColorSwatch{}
                                                    reader_quick_highlight_blue := mod.widgets.SettingsColorSwatch{}
                                                    reader_quick_highlight_pink := mod.widgets.SettingsColorSwatch{}
                                                    reader_quick_highlight_name := mod.widgets.SettingsColorName{width: 48 margin: 0 padding: 0}
                                                }
                                            }
                                        }
                                        reader_customise_btn := ButtonFlat{
                                            width: Fill height: 42
                                            text: "Customise"
                                            icon_walk: Walk{width: 15 height: 15}
                                            draw_icon +: {svg: crate_resource("self:resources/icons/reader_settings.svg")}
                                        }
                                    }
                                }
                            }
                            header := SolidView{
                                width: Fill height: 54
                                padding: Inset{left: 18 right: 18 top: 6 bottom: 6}
                                flow: Down
                                align: Align{ y: 0.5 }
                                draw_bg.color: #xf0ecdf
                                mobile_reader_title := Label{
                                    visible: false
                                    width: Fill height: 32
                                    padding: 0
                                    flow: Right{wrap: true}
                                    align: Align{y: 0.5}
                                    max_lines: 1
                                    text_overflow: Ellipsis
                                    text: "阅读"
                                    draw_text +: {
                                        text_style.font_size: 14
                                    }
                                }
                                reader_toolbar_row := View{
                                    width: Fill height: Fill
                                    flow: Right align: Align{y: 0.5}
                                    reader_toolbar_left := View{
                                        width: 180 height: Fit
                                        flow: Right spacing: 4
                                        reader_toolbar_left_buttons := View{
                                            visible: false
                                            width: Fit height: Fit
                                            flow: Right spacing: 4
                                            reader_back_btn := ButtonFlatterIcon{
                                                width: 36 height: 36 icon_walk: Walk{width: 16 height: 16}
                                                draw_icon +: {svg: crate_resource("self:resources/icons/reader_back.svg")}
                                            }
                                            toc_btn := ButtonFlatterIcon{
                                                width: 36 height: 36 icon_walk: Walk{width: 16 height: 16}
                                                draw_icon +: {svg: crate_resource("self:resources/icons/reader_contents.svg")}
                                            }
                                            reader_bookmarks_btn := ButtonFlatterIcon{
                                                width: 36 height: 36 icon_walk: Walk{width: 16 height: 16}
                                                draw_icon +: {svg: crate_resource("self:resources/icons/reader_bookmarks.svg")}
                                            }
                                            notes_btn := ButtonFlatterIcon{
                                                width: 36 height: 36 icon_walk: Walk{width: 16 height: 16}
                                                draw_icon +: {svg: crate_resource("self:resources/icons/reader_notes.svg")}
                                            }
                                        }
                                    }
                                    reader_title_slot := View{
                                        width: Fill height: Fit
                                        align: Center
                                        title := Label{
                                            text: "阅读"
                                            draw_text.text_style.font_size: 14
                                        }
                                    }
                                    reader_toolbar_right := View{
                                        width: 180 height: Fit
                                        flow: Right spacing: 4
                                        align: Align{x: 1.0 y: 0.5}
                                        reader_toolbar_right_buttons := View{
                                            visible: false
                                            width: Fit height: Fit
                                            flow: Right spacing: 4
                                            reader_appearance_btn := ButtonFlatter{
                                                width: 36 height: 36 text: "aA"
                                                draw_text.text_style.font_size: 13
                                            }
                                            reader_search_btn := ButtonFlatterIcon{
                                                width: 36 height: 36 icon_walk: Walk{width: 16 height: 16}
                                                draw_icon +: {svg: crate_resource("self:resources/icons/reader_search.svg")}
                                            }
                                            reader_bookmark_btn := ButtonFlatterIcon{
                                                width: 36 height: 36 icon_walk: Walk{width: 16 height: 16}
                                                draw_icon +: {svg: crate_resource("self:resources/icons/reader_bookmark.svg")}
                                            }
                                            reader_bookmark_filled_btn := ButtonFlatterIcon{
                                                visible: false
                                                width: 36 height: 36 icon_walk: Walk{width: 16 height: 16}
                                                draw_icon +: {svg: crate_resource("self:resources/icons/reader_bookmark_filled.svg")}
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
                                    is_multiline: false
                                    flow: Right
                                    empty_text: "搜索单词、原形或释义"
                                }
                                vocab_filters := View{
                                    width: Fit height: Fit
                                    flow: Right spacing: 8
                                    vocab_filter_all := RadioButtonTabFlat{text: "全部"}
                                    vocab_filter_learning := RadioButtonTabFlat{text: "学习中"}
                                    vocab_filter_mastered := RadioButtonTabFlat{text: "已掌握"}
                                }
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

                        page_reading_home := ScrollYView{
                            width: Fill height: Fill flow: Down padding: 20 spacing: 20
                            reading_home_title := Label{text: "阅读" draw_text.text_style.font_size: 24}
                            reading_home_hint := Label{width: Fill text: "回到书中的世界" draw_text.text_style.font_size: 12}
                            reading_continue_card := RoundedView{
                                width: Fill height: Fit new_batch: true flow: Down spacing: 16 padding: 20
                                draw_bg.color: theme.color_bg_container draw_bg.border_radius: 16.0
                                reading_continue_title := Label{width: Fill height: Fit text: "从一本好书开始" draw_text.text_style.font_size: 20}
                                reading_continue_hint := Label{width: Fill height: Fit text: "在书架选择一本书，开始阅读。" draw_text.text_style.font_size: 12}
                                reading_continue := Button{width: Fill height: 48 text: "继续阅读" enable_long_press: true}
                            }
                            reading_browse := ProfileEntry{
                                text: "去书架选书"
                                draw_icon +: {svg: crate_resource("self:resources/icons/nav_shelf.svg")}
                            }
                        }
                        page_profile := ScrollYView{
                            width: Fill height: Fill flow: Down padding: 20 spacing: 16
                            profile_title := Label{text: "我" draw_text.text_style.font_size: 24}
                            profile_hint := Label{width: Fill text: "每一页，都在积累" draw_text.text_style.font_size: 12}
                            profile_metrics_top := View{
                                width: Fill height: Fit flow: Right spacing: 12
                                profile_notes := ProfileMetric{metric_content +: {metric_heading +: {
                                    metric_title.text: "笔记"
                                    metric_icon +: {draw_icon +: {svg: crate_resource("self:resources/icons/reader_notes.svg")}}
                                }}}
                                profile_reading := ProfileMetric{metric_content +: {metric_heading +: {
                                    metric_title.text: "在读"
                                    metric_icon +: {draw_icon +: {svg: crate_resource("self:resources/icons/nav_reading.svg")}}
                                }}}
                            }
                            profile_metrics_bottom := View{
                                width: Fill height: Fit flow: Right spacing: 12
                                profile_completed := ProfileMetric{metric_content +: {metric_heading +: {
                                    metric_title.text: "读完"
                                    metric_icon +: {draw_icon +: {svg: crate_resource("self:resources/icons/nav_completed.svg")}}
                                }}}
                                profile_duration := ProfileMetric{metric_content +: {metric_heading +: {
                                    metric_title.text: "阅读时长"
                                    metric_icon +: {draw_icon +: {svg: crate_resource("self:resources/icons/nav_time.svg")}}
                                }}}
                            }
                            profile_status := Label{width: Fill height: Fit text: "" draw_text.text_style.font_size: 10}
                            profile_tools := RoundedView{
                                width: Fill height: Fit flow: Down padding: 4 new_batch: true
                                draw_bg.color: theme.color_bg_container draw_bg.border_radius: 12.0
                                profile_chat := ProfileEntry{text: "AI 对话" draw_icon +: {svg: crate_resource("self:resources/icons/nav_chat.svg")}}
                                profile_skills := ProfileEntry{text: "AI 技能" draw_icon +: {svg: crate_resource("self:resources/icons/nav_skills.svg")}}
                                profile_settings := ProfileEntry{text: "设置" draw_icon +: {svg: crate_resource("self:resources/icons/reader_settings.svg")}}
                            }
                        }
                        page_notes := View{
                            width: Fill height: Fill flow: Down
                            all_notes_status := Label{width: Fill height: Fit padding: 16 text: "正在读取笔记…" draw_text.text_style.font_size: 12}
                            all_notes_list := mod.widgets.AllNotesList{}
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
                            skills_body := ScrollYView{
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
                            reader_turn_mode_row := View{
                                width: Fill height: Fit
                                flow: Flow.Right{wrap: true} spacing: 12
                                align: Align{y: 0.5}
                                reader_turn_mode_label := Label{text: "翻页方式"}
                                reader_turn_curl := RadioButtonTabFlat{text: "仿真翻页"}
                                reader_turn_slide := RadioButtonTabFlat{text: "左右滑动"}
                                reader_turn_scroll := RadioButtonTabFlat{text: "上下滚动"}
                            }
                            reader_scroll_hint := Label{
                                visible: false
                                width: Fill height: Fit
                                text: "上下滚动使用单列连续正文，不显示页码。"
                            }
                            reader_columns_row := View{
                                width: Fill height: Fit
                                flow: Flow.Right{wrap: true} spacing: 12
                                align: Align{y: 0.5}
                                reader_columns_label := Label{text: "正文分栏"}
                                reader_columns_one := RadioButtonTabFlat{text: "单列"}
                                reader_columns_two := RadioButtonTabFlat{text: "双列"}
                                reader_columns_auto := RadioButtonTabFlat{text: "自动"}
                            }
                            font_family_row := View{
                                width: Fill height: Fit
                                flow: Right spacing: 12
                                align: Align{y: 0.5}
                                font_family_label := Label{text: "正文字体"}
                                reader_font_choice := DropDown{
                                    width: 190 height: 32
                                    align: Align{x: 0.0 y: 0.5}
                                    padding: Inset{left: 12 right: 24 top: 0 bottom: 0}
                                    labels: ["Original"]
                                }
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
                                flow: Flow.Right{wrap: true} spacing: 12 align: Align{y: 0.5}
                                gloss_color_title := mod.widgets.SettingsColorTitle{text: "注音字色"}
                                gloss_color_picker := mod.widgets.SettingsColorPicker{
                                    gc0 := mod.widgets.SettingsColorSwatch{}
                                    gc1 := mod.widgets.SettingsColorSwatch{}
                                    gc2 := mod.widgets.SettingsColorSwatch{}
                                    gc3 := mod.widgets.SettingsColorSwatch{}
                                    gloss_color_name := mod.widgets.SettingsColorName{text: "青"}
                                }
                            }
                            gloss_bg_row := View{
                                width: Fill height: Fit
                                flow: Flow.Right{wrap: true} spacing: 12 align: Align{y: 0.5}
                                gloss_bg_title := mod.widgets.SettingsColorTitle{text: "注音背景"}
                                gloss_bg_picker := mod.widgets.SettingsColorPicker{
                                    gb0 := mod.widgets.SettingsColorSwatch{}
                                    gb1 := mod.widgets.SettingsColorSwatch{}
                                    gb2 := mod.widgets.SettingsColorSwatch{}
                                    gb3 := mod.widgets.SettingsColorSwatch{}
                                    gloss_bg_name := mod.widgets.SettingsColorName{text: "无"}
                                }
                            }
                            word_color_row := View{
                                width: Fill height: Fit
                                flow: Flow.Right{wrap: true} spacing: 12 align: Align{y: 0.5}
                                word_color_title := mod.widgets.SettingsColorTitle{text: "词字色"}
                                word_color_picker := mod.widgets.SettingsColorPicker{
                                    wc0 := mod.widgets.SettingsColorSwatch{}
                                    wc1 := mod.widgets.SettingsColorSwatch{}
                                    wc2 := mod.widgets.SettingsColorSwatch{}
                                    wc3 := mod.widgets.SettingsColorSwatch{}
                                    word_color_name := mod.widgets.SettingsColorName{text: "默认"}
                                }
                            }
                            word_bg_row := View{
                                width: Fill height: Fit
                                flow: Flow.Right{wrap: true} spacing: 12 align: Align{y: 0.5}
                                word_bg_title := mod.widgets.SettingsColorTitle{text: "词背景"}
                                word_bg_picker := mod.widgets.SettingsColorPicker{
                                    wb0 := mod.widgets.SettingsColorSwatch{}
                                    wb1 := mod.widgets.SettingsColorSwatch{}
                                    wb2 := mod.widgets.SettingsColorSwatch{}
                                    wb3 := mod.widgets.SettingsColorSwatch{}
                                    word_bg_name := mod.widgets.SettingsColorName{text: "无"}
                                }
                            }
                            settings_text_heading := Label{
                                text: "文本"
                                draw_text.text_style.font_size: 13
                                draw_text.color: theme.color_label_inner_inactive
                            }
                            sel_color_row := View{
                                width: Fill height: Fit
                                flow: Flow.Right{wrap: true} spacing: 12 align: Align{y: 0.5}
                                selection_color_title := mod.widgets.SettingsColorTitle{text: "选中颜色"}
                                selection_color_picker := mod.widgets.SettingsColorPicker{
                                    sc0 := mod.widgets.SettingsColorSwatch{}
                                    sc1 := mod.widgets.SettingsColorSwatch{}
                                    sc2 := mod.widgets.SettingsColorSwatch{}
                                    sc3 := mod.widgets.SettingsColorSwatch{}
                                    selection_color_name := mod.widgets.SettingsColorName{text: "青"}
                                }
                            }
                            hl_color_row := View{
                                width: Fill height: Fit
                                flow: Flow.Right{wrap: true} spacing: 12 align: Align{y: 0.5}
                                highlight_color_title := mod.widgets.SettingsColorTitle{text: "高亮颜色"}
                                highlight_color_picker := mod.widgets.SettingsColorPicker{
                                    hc0 := mod.widgets.SettingsColorSwatch{}
                                    hc1 := mod.widgets.SettingsColorSwatch{}
                                    hc2 := mod.widgets.SettingsColorSwatch{}
                                    hc3 := mod.widgets.SettingsColorSwatch{}
                                    highlight_color_name := mod.widgets.SettingsColorName{text: "黄"}
                                }
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
                            mobile_nav_tabs := View{
                                width: Fill height: 58 flow: Right spacing: 0
                                mobile_nav_reader := MobileTab{text: "阅读" draw_icon +: {svg: crate_resource("self:resources/icons/nav_reading.svg")}}
                                mobile_nav_library := MobileTab{text: "书架" draw_icon +: {svg: crate_resource("self:resources/icons/nav_shelf.svg")}}
                                mobile_nav_vocab := MobileTab{text: "词汇" draw_icon +: {svg: crate_resource("self:resources/icons/nav_vocab.svg")}}
                                mobile_nav_profile := MobileTab{text: "我" draw_icon +: {svg: crate_resource("self:resources/icons/nav_profile.svg")}}
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
                                flow: Overlay
                                margin: Inset{left: 18 right: 18 top: 18 bottom: 18}
                                padding: 12
                                draw_bg.color: theme.color_bg_container
                                draw_bg.border_radius: 10.0
                                image_viewer_center := View{
                                    width: Fill height: Fill
                                    align: Center
                                    image_viewer_image := Image{
                                        width: 1 height: 1
                                        fit: ImageFit.Stretch
                                    }
                                }
                                image_viewer_close_row := View{
                                    width: Fill height: Fit
                                    align: Align{x: 1.0}
                                    image_viewer_close := ButtonFlat{text: "关闭"}
                                }
                            }
                        }
                    }
                    library_import_modal := Modal{
                        align: TopLeft
                        content +: {
                            width: 480 height: 460
                            library_import_card := RoundedView{
                                width: Fill height: Fill
                                new_batch: true
                                flow: Down spacing: 12 padding: 16
                                draw_bg.color: theme.color_bg_container
                                draw_bg.border_radius: 14.0
                                library_import_title := Label{width: Fill text: "导入 EPUB" draw_text.text_style.font_size: 16}
                                library_import_scroll := ScrollYView{
                                    width: Fill height: Fill flow: Down spacing: 12
                                    library_import_choices := View{
                                        width: Fill height: Fit flow: Down spacing: 12
                                        library_import_local := Button{
                                            visible: #(cfg!(any(target_os = "android", target_os = "ios", target_os = "macos")))
                                            width: Fill height: 44 text: "本机文件"
                                        }
                                        library_import_lan := ButtonFlat{
                                            visible: #(cfg!(not(target_os = "ios")))
                                            width: Fill height: 44 text: "局域网传书"
                                        }
                                    }
                                    library_import_receiver := View{
                                        visible: false
                                        width: Fill height: Fit flow: Down spacing: 12
                                        library_import_instructions := TextBox{
                                            width: Fill height: Fit padding: 0
                                            flow: Flow.Right{wrap: true}
                                            text: "两台设备连接同一 Wi-Fi，在电脑浏览器打开下方网址，选择 EPUB 上传。关闭界面或切到后台仍可接收。"
                                            draw_text.text_style.font_size: 10
                                        }
                                        library_import_address := View{
                                            width: Fill height: Fit flow: Down spacing: 12
                                            library_import_urls := TextInput{
                                                width: Fill height: 92 empty_text: ""
                                                is_read_only: true is_multiline: true
                                                flow: Flow.Right{wrap: true}
                                                draw_text.text_style.font_size: 10
                                            }
                                            library_import_copy := ButtonFlat{width: Fill height: 40 text: "复制网址"}
                                        }
                                        library_import_background := ButtonFlat{
                                            visible: #(cfg!(target_os = "android"))
                                            width: Fill height: 40 text: "允许后台接收"
                                        }
                                        library_import_restart := ButtonFlat{visible: false width: Fill height: 40 text: "重新开启"}
                                    }
                                    library_import_status := TextBox{
                                        width: Fill height: Fit padding: 0
                                        flow: Flow.Right{wrap: true}
                                        text: "从本机文件导入，或通过局域网从电脑传书。"
                                        draw_text.text_style.font_size: 10
                                    }
                                }
                                library_import_stop := ButtonFlat{visible: false width: Fill height: 40 text: "停止接收"}
                                library_import_close := ButtonFlat{width: Fill height: 40 text: "关闭界面"}
                            }
                        }
                    }
                    reader_customise_modal := Modal{
                        align: TopLeft
                        content +: {
                            width: 560 height: 620
                            reader_customise_card := RoundedView{
                                width: Fill height: Fill
                                new_batch: true
                                flow: Down spacing: 16 padding: 20
                                draw_bg.color: theme.color_bg_container
                                draw_bg.border_radius: 16.0
                                reader_customise_header := View{
                                    width: Fill height: Fit
                                    flow: Right
                                    align: Align{y: 0.5}
                                    reader_customise_title := Label{
                                        width: Fill
                                        text: "Customise"
                                        draw_text.text_style.font_size: 17
                                    }
                                    reader_customise_close := ButtonFlatter{width: 40 height: 40 margin: 0 text: "X"}
                                }
                                reader_customise_scroll := ScrollYView{
                                    width: Fill height: Fill
                                    flow: Down spacing: 14
                                    scroll_bars +: {scroll_bar_y +: {draw_bg +: {
                                        color: #0000 color_hover: #0000 color_drag: #0000
                                        border_color: #0000 border_color_hover: #0000 border_color_drag: #0000
                                        border_size: 0.0
                                    }}}
                                    reader_preview := RoundedView{
                                        width: Fill height: 180
                                        new_batch: true
                                        flow: Down spacing: 6 padding: 16
                                        draw_bg.color: #f7f4ea
                                        draw_bg.border_radius: 10.0
                                        reader_preview_heading := Label{
                                            width: Fill height: Fit
                                            text: "Ruby 注音验证"
                                            draw_text.text_style.font_size: 15
                                        }
                                        reader_preview_scroll := ScrollYView{
                                            width: Fill height: Fill
                                            scroll_bars +: {scroll_bar_y +: {draw_bg +: {
                                                color: #0000 color_hover: #0000 color_drag: #0000
                                                border_color: #0000 border_color_hover: #0000 border_color_drag: #0000
                                                border_size: 0.0
                                            }}}
                                            reader_preview_columns := View{
                                                width: Fill height: Fit
                                                flow: Right spacing: 12
                                                reader_preview_text := Html{
                                                    width: Fill height: Fit
                                                    // First-row ruby paints above Html's measured text box.
                                                    clip_y: false
                                                    selectable: false
                                                    ruby := mod.widgets.RubyWord{preview: true}
                                                }
                                                reader_preview_second_column := View{
                                                    visible: false
                                                    width: Fill height: Fit
                                                    clip_y: false
                                                    reader_preview_text_second := Html{
                                                        width: Fill height: Fit
                                                        clip_y: false
                                                        selectable: false
                                                        ruby := mod.widgets.RubyWord{preview: true}
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    reader_customise_text_title := Label{
                                        text: "Text"
                                        draw_text.text_style.font_size: 13
                                    }
                                    reader_customise_font_row := View{
                                        width: Fill height: Fit
                                        flow: Right spacing: 8
                                        align: Align{y: 0.5}
                                        reader_customise_font_label := Label{width: Fill text: "Font"}
                                        reader_customise_font_choice := DropDown{
                                            width: 180 height: 32
                                            align: Align{x: 0.0 y: 0.5}
                                            padding: Inset{left: 12 right: 24 top: 0 bottom: 0}
                                            popup_menu_position: mod.widgets.PopupMenuPosition.BelowInput
                                            popup_menu: mod.widgets.PopupMenuFlat{
                                                width: 180
                                                menu_item: mod.widgets.PopupMenuItem{
                                                    height: 14
                                                    padding: Inset{left: 12 right: 8 top: 0 bottom: 0}
                                                }
                                            }
                                            labels: ["Original"]
                                        }
                                    }
                                    reader_customise_size_row := View{
                                        width: Fill height: Fit
                                        flow: Right spacing: 8
                                        align: Align{y: 0.5}
                                        reader_customise_size_label := Label{width: 164 text: "Text Size"}
                                        reader_customise_size_controls := View{
                                            width: Fill height: Fit
                                            flow: Right spacing: 8
                                            align: Align{y: 0.5}
                                            reader_customise_size_slider := ReaderLayoutSlider{min: 8. max: 20. step: 0. precision: 0}
                                            reader_customise_size_value := Label{width: 58 text: "12"}
                                        }
                                    }
                                    reader_customise_bold := CheckBox{text: "Bold Text"}
                                    reader_customise_highlight_row := View{
                                        width: Fill height: Fit flow: Right spacing: 8
                                        align: Align{y: 0.5}
                                        reader_customise_highlight_label := Label{width: 164 text: "Highlight Color"}
                                        reader_customise_highlight_options := mod.widgets.SettingsColorPicker{
                                            height: Fit
                                            reader_customise_highlight_yellow := mod.widgets.SettingsColorSwatch{}
                                            reader_customise_highlight_green := mod.widgets.SettingsColorSwatch{}
                                            reader_customise_highlight_blue := mod.widgets.SettingsColorSwatch{}
                                            reader_customise_highlight_pink := mod.widgets.SettingsColorSwatch{}
                                            reader_customise_highlight_name := mod.widgets.SettingsColorName{text: "Yellow"}
                                        }
                                    }
                                    reader_customise_gloss_title := Label{
                                        text: "Word Annotations"
                                        draw_text.text_style.font_size: 13
                                    }
                                    reader_customise_gloss_enable := CheckBox{text: "Show word annotations"}
                                    reader_customise_gloss_size_row := View{
                                        width: Fill height: Fit flow: Right spacing: 8
                                        align: Align{y: 0.5}
                                        reader_customise_gloss_size_label := Label{width: 164 text: "Annotation Size"}
                                        reader_customise_gloss_size_controls := View{
                                            width: Fill height: Fit
                                            flow: Right spacing: 8
                                            align: Align{y: 0.5}
                                            reader_customise_gloss_size_slider := ReaderLayoutSlider{min: 6. max: 12. step: 0. precision: 1}
                                            reader_customise_gloss_size_value := Label{width: 58 text: "7.5"}
                                        }
                                    }
                                    reader_customise_gloss_color_row := View{
                                        width: Fill height: Fit flow: Right spacing: 8 align: Align{y: 0.5}
                                        reader_customise_gloss_color_label := mod.widgets.SettingsColorTitle{width: 164 text: "Annotation Color"}
                                        reader_customise_gloss_color_options := View{
                                            width: Fit height: Fit flow: Right spacing: 4 align: Align{y: 0.5}
                                            reader_customise_gloss_color_green := mod.widgets.SettingsColorSwatch{}
                                            reader_customise_gloss_color_blue := mod.widgets.SettingsColorSwatch{}
                                            reader_customise_gloss_color_orange := mod.widgets.SettingsColorSwatch{}
                                            reader_customise_gloss_color_gray := mod.widgets.SettingsColorSwatch{}
                                            reader_customise_gloss_color_name := mod.widgets.SettingsColorName{}
                                        }
                                    }
                                    reader_customise_gloss_bg_row := View{
                                        width: Fill height: Fit flow: Right spacing: 8 align: Align{y: 0.5}
                                        reader_customise_gloss_bg_label := mod.widgets.SettingsColorTitle{width: 164 text: "Annotation Background"}
                                        reader_customise_gloss_bg_options := View{
                                            width: Fit height: Fit flow: Right spacing: 4 align: Align{y: 0.5}
                                            reader_customise_gloss_bg_none := mod.widgets.SettingsColorSwatch{}
                                            reader_customise_gloss_bg_gray := mod.widgets.SettingsColorSwatch{}
                                            reader_customise_gloss_bg_green := mod.widgets.SettingsColorSwatch{}
                                            reader_customise_gloss_bg_blue := mod.widgets.SettingsColorSwatch{}
                                            reader_customise_gloss_bg_name := mod.widgets.SettingsColorName{}
                                        }
                                    }
                                    reader_customise_word_color_row := View{
                                        width: Fill height: Fit flow: Right spacing: 8 align: Align{y: 0.5}
                                        reader_customise_word_color_label := mod.widgets.SettingsColorTitle{width: 164 text: "Word Color"}
                                        reader_customise_word_color_options := View{
                                            width: Fit height: Fit flow: Right spacing: 4 align: Align{y: 0.5}
                                            reader_customise_word_color_default := mod.widgets.SettingsColorSwatch{}
                                            reader_customise_word_color_green := mod.widgets.SettingsColorSwatch{}
                                            reader_customise_word_color_blue := mod.widgets.SettingsColorSwatch{}
                                            reader_customise_word_color_orange := mod.widgets.SettingsColorSwatch{}
                                            reader_customise_word_color_name := mod.widgets.SettingsColorName{}
                                        }
                                    }
                                    reader_customise_word_bg_row := View{
                                        width: Fill height: Fit flow: Right spacing: 8 align: Align{y: 0.5}
                                        reader_customise_word_bg_label := mod.widgets.SettingsColorTitle{width: 164 text: "Word Background"}
                                        reader_customise_word_bg_options := View{
                                            width: Fit height: Fit flow: Right spacing: 4 align: Align{y: 0.5}
                                            reader_customise_word_bg_none := mod.widgets.SettingsColorSwatch{}
                                            reader_customise_word_bg_gray := mod.widgets.SettingsColorSwatch{}
                                            reader_customise_word_bg_green := mod.widgets.SettingsColorSwatch{}
                                            reader_customise_word_bg_blue := mod.widgets.SettingsColorSwatch{}
                                            reader_customise_word_bg_name := mod.widgets.SettingsColorName{}
                                        }
                                    }
                                    reader_customise_layout_title := Label{
                                        text: "Accessibility & Layout"
                                        draw_text.text_style.font_size: 13
                                    }
                                    reader_customise_line_row := View{
                                        width: Fill height: Fit
                                        flow: Right spacing: 8
                                        align: Align{y: 0.5}
                                        reader_customise_line_label := Label{width: 164 text: "LINE SPACING"}
                                        reader_customise_line_controls := View{
                                            width: Fill height: Fit
                                            flow: Right spacing: 8
                                            align: Align{y: 0.5}
                                            reader_customise_line_slider := ReaderLayoutSlider{min: 0.8 max: 2.5}
                                            reader_customise_line_value := Label{width: 58 text: "1.55×"}
                                        }
                                    }
                                    reader_customise_character_row := View{
                                        width: Fill height: Fit
                                        flow: Right spacing: 8
                                        align: Align{y: 0.5}
                                        reader_customise_character_label := Label{width: 164 text: "CHARACTER SPACING"}
                                        reader_customise_character_controls := View{
                                            width: Fill height: Fit
                                            flow: Right spacing: 8
                                            align: Align{y: 0.5}
                                            reader_customise_character_slider := ReaderLayoutSlider{min: -10. max: 10.}
                                            reader_customise_character_value := Label{width: 58 text: "0%"}
                                        }
                                    }
                                    reader_customise_word_row := View{
                                        width: Fill height: Fit
                                        flow: Right spacing: 8
                                        align: Align{y: 0.5}
                                        reader_customise_word_label := Label{width: 164 text: "WORD SPACING"}
                                        reader_customise_word_controls := View{
                                            width: Fill height: Fit
                                            flow: Right spacing: 8
                                            align: Align{y: 0.5}
                                            reader_customise_word_slider := ReaderLayoutSlider{min: -20. max: 20.}
                                            reader_customise_word_value := Label{width: 58 text: "0%"}
                                        }
                                    }
                                    reader_customise_margin_row := View{
                                        width: Fill height: Fit
                                        flow: Right spacing: 8
                                        align: Align{y: 0.5}
                                        reader_customise_margin_label := Label{width: 164 text: "MARGINS"}
                                        reader_customise_margin_controls := View{
                                            width: Fill height: Fit
                                            flow: Right spacing: 8
                                            align: Align{y: 0.5}
                                            reader_customise_margin_slider := ReaderLayoutSlider{min: -10. max: 10.}
                                            reader_customise_margin_value := Label{width: 58 text: "0%"}
                                        }
                                    }
                                    reader_customise_columns_row := View{
                                        width: Fill height: Fit
                                        flow: Right spacing: 8
                                        align: Align{y: 0.5}
                                        reader_customise_columns_title := Label{
                                            width: Fill
                                            text: "Columns"
                                            draw_text.text_style.font_size: 13
                                        }
                                        reader_customise_columns_choice := DropDown{
                                            width: 180 height: 32
                                            align: Align{x: 0.0 y: 0.5}
                                            padding: Inset{left: 12 right: 24 top: 0 bottom: 0}
                                            popup_menu_position: mod.widgets.PopupMenuPosition.BelowInput
                                            popup_menu: mod.widgets.PopupMenuFlat{
                                                width: 180
                                                menu_item: mod.widgets.PopupMenuItem{
                                                    height: 22
                                                    padding: Inset{left: 12 right: 8 top: 0 bottom: 0}
                                                }
                                            }
                                            labels: ["1", "2", "Auto Set"]
                                        }
                                    }
                                    reader_customise_justify := CheckBox{text: "Justify Text"}
                                    reader_customise_reset := ButtonFlatter{
                                        width: Fill height: 42
                                        text: "Reset Theme"
                                    }
                                }
                                reader_customise_actions := View{
                                    width: Fill height: Fit
                                    flow: Right spacing: 10
                                    align: Align{x: 1.0 y: 0.5}
                                    reader_customise_cancel := ButtonFlatter{
                                        width: 100 height: 44 margin: 0 text: "Cancel"
                                    }
                                    reader_customise_save := ButtonFlat{
                                        width: 148 height: 44 margin: 0 text: "Save & Apply"
                                    }
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
        let text = if self.current == 0 {
            "…".to_string()
        } else if self.hovered && self.total > 0 {
            format!("{} of {}", self.current, self.total)
        } else {
            self.current.to_string()
        };
        self.view.label(cx, ids!(page_text)).set_text(cx, &text);
    }

    fn set_pages(&mut self, cx: &mut Cx, current: usize, total: usize) {
        self.current = if total > 0 {
            current.min(total)
        } else {
            current
        };
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
    #[source]
    source: ScriptObjectRef,
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
    font_preview_normal: TextStyle,
    #[live]
    font_preview_italic: TextStyle,
    #[live]
    font_preview_bold: TextStyle,
    #[live]
    font_preview_bold_italic: TextStyle,
    #[live]
    font_active_normal: TextStyle,
    #[live]
    font_active_italic: TextStyle,
    #[live]
    font_active_bold: TextStyle,
    #[live]
    font_active_bold_italic: TextStyle,
    #[live]
    html_default_inset: f64,
    #[live]
    draw_reader_selection: DrawColor,
    #[live]
    draw_page_turn: DrawReaderPageTurn,
    #[rust]
    page_turn: PageTurn,
    /// 选区自跟踪（PortalList 未暴露区间 API，用公开的命中接口镜像）。
    /// 普通文字与原生选区同步；注音词/链接等交互内联文字由镜像投影绘制。
    #[rust]
    sel_anchor: Option<(usize, usize)>,
    #[rust]
    sel_cursor: Option<(usize, usize)>,
    #[rust]
    selecting: bool,
    #[rust]
    last_abs: Option<DVec2>,
    #[rust]
    touch_selection: Option<crate::reader_touch::TouchSelection>,
    #[rust]
    toolbar_tap: ToolbarTap,
    #[rust]
    word_tap: DoubleTap,
    #[rust]
    pending_reader_tap: ActionsBuf,
    #[rust]
    reader_tap_timer: Timer,
    #[rust]
    pending_tap_context: Option<(u64, usize)>,
    #[rust]
    compact_layout: bool,
    /// TextFlow 本帧真实选区矩形，按正文块/范围/排版签名缓存为 item 相对坐标。
    /// 当前选区与保存后的永久高亮共用它，禁止再用另一套纯文本重排几何。
    #[rust]
    range_geometry_cache: HashMap<ReaderRangeGeometryKey, Vec<Rect>>,
    /// 上一次完成绘制后各段落的真实宽度。PortalList 当前重绘开始后，旧 Area
    /// 可能暂时失效，因此不能直接把 Area 当作可靠的上一帧宽度来源。
    #[rust]
    item_layout_widths: HashMap<(usize, usize), f64>,
    #[rust]
    // A paragraph can be drawn in both columns by two different portal slots.
    // Their glyph origins differ even though the source block is identical.
    hit_runs: HashMap<WidgetUid, (usize, u64, Vec<crate::reader_hit::HitRun>)>,
    #[rust]
    text_maps: HashMap<usize, (u64, crate::reader_text::TextMap)>,
    #[rust]
    pager: ReaderPagedState,
    #[rust]
    book_pages: BookPages,
    #[rust]
    turn_mode: ReaderTurnMode,
}

#[derive(Default)]
struct ReaderPagedState {
    active: bool,
    signature: u64,
    content_version: u64,
    layout: crate::reader_columns::ColumnLayout,
    metrics: Vec<Option<crate::reader_columns::BlockMetrics>>,
    anchor: crate::reader_columns::PageAnchor,
    next: crate::reader_columns::PageAnchor,
    drawn: Vec<(usize, WidgetRef, Rect)>,
    page_index: crate::reader_columns::PageIndex,
    pending_source: Option<(usize, usize)>,
    requested_source: Option<(usize, usize)>,
    scroll_gesture: crate::reader_scroll::ScrollGesture,
    measure_frame: NextFrame,
    requested_last_page: bool,
    extend_selection_after_draw: bool,
    spread_ready: bool,
    visible_pages: usize,
    needs_page_alignment: bool,
    // Coalesce a burst into one further turn, rather than dropping the input
    // or replaying a long wheel-event backlog after the reader stops scrolling.
    pending_turn: Option<bool>,
    turn_history: Vec<crate::reader_columns::PageAnchor>,
}

impl ReaderPagedState {
    fn page_ready(&self) -> bool {
        self.spread_ready && !self.requested_last_page && self.pending_source.is_none()
    }

    fn take_pending_turn(&mut self, animating: bool) -> Option<bool> {
        if animating || !self.page_ready() {
            return None;
        }
        if self.pending_turn == Some(false)
            && self.turn_history.is_empty()
            && self.page_index.containing(self.anchor).is_none()
        {
            return None;
        }
        self.pending_turn.take()
    }
}

#[derive(Debug)]
struct ReaderPageTurn(bool);

#[derive(Debug)]
struct ReaderToolbarTap;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct ReaderRangeGeometryKey {
    chapter: usize,
    block: usize,
    start: usize,
    end: usize,
    viewport_width_half_px: i64,
    font_size_bits: u32,
    line_spacing_bits: u32,
    font_kind: u8,
    source_hash: u64,
}

fn reader_source_hash(source: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    hasher.finish()
}

fn reader_range_geometry_key(
    chapter: usize,
    block: usize,
    range: (usize, usize),
    viewport_width: f64,
    font_size: f32,
    line_spacing: f32,
    font: ReaderFont,
    source: &str,
) -> ReaderRangeGeometryKey {
    ReaderRangeGeometryKey {
        chapter,
        block,
        start: range.0,
        end: range.1,
        viewport_width_half_px: (viewport_width * 2.0).round() as i64,
        font_size_bits: font_size.to_bits(),
        line_spacing_bits: line_spacing.to_bits(),
        font_kind: font.index() as u8,
        source_hash: reader_source_hash(source),
    }
}

fn reader_geometry_width_is_stable(before: f64, after: f64) -> bool {
    before > 0.0 && after > 0.0 && (before - after).abs() <= 0.5
}

fn reader_geometry_capture_policy(
    probe: Option<(usize, usize)>,
    current: Option<(usize, usize)>,
    width_stable: bool,
) -> (bool, bool) {
    // Live selection is always drawn immediately. Only reuse across frames is
    // gated on layout stability.
    (current.is_some() && probe == current, width_stable)
}

/// Read the rectangle instances that TextFlow just submitted through its
/// transparent DrawColor batch. This uses public draw-batch metadata and never
/// reaches into TextFlow's private SelectionTracker.
fn captured_draw_color_rects(cx: &Cx2d, draw: &DrawColor) -> Vec<Rect> {
    let Some(many) = draw.draw_super.many_instances.as_ref() else {
        return Vec::new();
    };
    let Some(shader_id) = draw.draw_super.draw_vars.draw_shader_id else {
        return Vec::new();
    };
    let mapping = &cx.draw_shaders[shader_id.index].mapping;
    let (Some(rect_pos), Some(rect_size)) = (mapping.rect_pos, mapping.rect_size) else {
        return Vec::new();
    };
    let stride = mapping.instances.total_slots;
    if stride == 0 {
        return Vec::new();
    }
    let start = many.instance_area.instance_offset;
    if start >= many.instances.len() {
        return Vec::new();
    }
    let mut rects = Vec::new();
    for base in (start..many.instances.len()).step_by(stride) {
        if base + rect_size + 1 >= many.instances.len()
            || base + rect_pos + 1 >= many.instances.len()
        {
            break;
        }
        let rect = Rect {
            pos: dvec2(
                many.instances[base + rect_pos] as f64,
                many.instances[base + rect_pos + 1] as f64,
            ),
            size: dvec2(
                many.instances[base + rect_size] as f64,
                many.instances[base + rect_size + 1] as f64,
            ),
        };
        if rect.size.x > 0.0 && rect.size.y > 0.0 {
            rects.push(rect);
        }
    }
    rects
}

#[derive(Clone, Copy, Debug)]
struct ReaderSelectionLine {
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
    max_height: f64,
}

/// Group TextFlow's per-run rectangles without changing their vertical bounds.
/// Raised ruby and other rich runs can extend above the ordinary text on a row.
fn rich_selection_lines(rects: &[Rect]) -> Vec<ReaderSelectionLine> {
    let mut lines: Vec<ReaderSelectionLine> = Vec::new();
    for rect in rects
        .iter()
        .copied()
        .filter(|rect| rect.size.x > 0.0 && rect.size.y > 0.0)
    {
        let right = rect.pos.x + rect.size.x;
        let bottom = rect.pos.y + rect.size.y;
        if let Some(line) = lines.last_mut() {
            let center = rect.pos.y + rect.size.y * 0.5;
            let line_center = (line.top + line.bottom) * 0.5;
            let vertical_tolerance = line.max_height.max(rect.size.y) * 1.05;
            let continues_horizontally = rect.pos.x + 3.0 >= line.right;
            if continues_horizontally && (center - line_center).abs() <= vertical_tolerance {
                line.left = line.left.min(rect.pos.x);
                line.right = line.right.max(right);
                line.top = line.top.min(rect.pos.y);
                line.bottom = line.bottom.max(bottom);
                line.max_height = line.max_height.max(rect.size.y);
                continue;
            }
        }
        lines.push(ReaderSelectionLine {
            left: rect.pos.x,
            right,
            top: rect.pos.y,
            bottom,
            max_height: rect.size.y,
        });
    }

    lines
}

/// Merge TextFlow's per-run selection rectangles into visual lines.
///
/// The painted selection is bottom-aligned and fills spaces between runs and
/// rows. These adjusted rectangles are unsuitable as page clipping boundaries.
fn normalize_rich_selection_rects(rects: &[Rect]) -> Vec<Rect> {
    let mut lines = rich_selection_lines(rects);
    // A ruby/italic fragment may be vertically raised, while neighboring prose
    // gives the real baseline. Bottom-align every fragment merged into that line.
    for line in &mut lines {
        line.top = line.bottom - line.max_height;
    }

    // Every adjacent pair must use the exact same boundary. A tall annotation
    // can make the raw glyph bands overlap; expanding each row back to its own
    // raw bounds would then paint that overlap twice as a darker stripe.
    let boundaries: Vec<f64> = lines
        .windows(2)
        .map(|pair| {
            let upper_center = (pair[0].top + pair[0].bottom) * 0.5;
            let lower_center = (pair[1].top + pair[1].bottom) * 0.5;
            (upper_center + lower_center) * 0.5
        })
        .collect();

    let mut out = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        let top = index
            .checked_sub(1)
            .and_then(|previous| boundaries.get(previous).copied())
            .unwrap_or(line.top - 2.0);
        let bottom = boundaries.get(index).copied().unwrap_or(line.bottom + 2.0);
        out.push(Rect {
            pos: dvec2(line.left, top),
            size: dvec2((line.right - line.left).max(1.0), (bottom - top).max(1.0)),
        });
    }
    out
}

/// Return cuts between the actual TextFlow row boxes. TextFlow adds 2px above
/// and below every selection rectangle; retaining that highlight padding can
/// make adjacent rows appear to overlap and leave pagination with no cuts.
fn reader_pagination_rows(rects: &[Rect]) -> Vec<(f64, DVec2)> {
    const SELECTION_PADDING: f64 = 2.0;
    let lines = reader_visual_row_bands(
        rects.iter().copied().map(|rect| (rect, 0)),
        SELECTION_PADDING,
    );
    lines
        .windows(2)
        .filter_map(|pair| {
            let upper = pair[0].0;
            let lower = pair[1].0;
            let gap = lower.top - upper.bottom;
            (gap >= 0.0).then(|| {
                (
                    // Leave most of the gap with the following row so its
                    // raised ruby label stays on the same page as its word.
                    upper.bottom + gap.min(4.0) * 0.5,
                    dvec2(
                        lower.left + (lower.right - lower.left).min(1.0) * 0.5,
                        lower.top + (lower.bottom - lower.top) * 0.5,
                    ),
                )
            })
        })
        .collect()
}

/// TextFlow emits runs in document order. A run can paint several rows before
/// the next run returns to an earlier visual row, so pagination must union
/// their vertical ink bands before choosing any cut.
fn reader_visual_row_bands(
    rows: impl Iterator<Item = (Rect, usize)>,
    padding: f64,
) -> Vec<(ReaderSelectionLine, usize)> {
    let mut rows: Vec<_> = rows
        .filter_map(|(rect, source)| {
            let top = rect.pos.y + padding;
            let bottom = rect.pos.y + rect.size.y - padding;
            (rect.size.x > 0.0 && bottom > top).then_some((rect, source, top, bottom))
        })
        .collect();
    rows.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.0.pos.x.total_cmp(&b.0.pos.x)));
    let mut lines: Vec<(ReaderSelectionLine, usize)> = Vec::new();
    for (rect, source, top, bottom) in rows {
        let right = rect.pos.x + rect.size.x;
        if let Some((line, first_source)) = lines.last_mut() {
            if top < line.bottom - 0.5 {
                line.left = line.left.min(rect.pos.x);
                line.right = line.right.max(right);
                line.top = line.top.min(top);
                line.bottom = line.bottom.max(bottom);
                line.max_height = line.max_height.max(bottom - top);
                *first_source = (*first_source).min(source);
                continue;
            }
        }
        lines.push((
            ReaderSelectionLine {
                left: rect.pos.x,
                right,
                top,
                bottom,
                max_height: bottom - top,
            },
            source,
        ));
    }
    lines
}

fn reader_painted_page_cuts(rows: &[(Rect, usize)]) -> Vec<(f64, usize)> {
    let lines = reader_visual_row_bands(rows.iter().copied(), 0.0);
    lines
        .windows(2)
        .filter_map(|pair| {
            let (upper, _) = pair[0];
            let (lower, source) = pair[1];
            let gap = lower.top - upper.bottom;
            // Ruby is painted above the word's row box; keep that space on the
            // next page rather than cutting halfway through the line gap.
            (gap >= 0.0).then_some((upper.bottom + gap.min(4.0) * 0.5, source))
        })
        .collect()
}

fn reader_block_visual_height(layout_height: f64, painted_bottom: f64) -> f64 {
    if painted_bottom > layout_height {
        painted_bottom + 1.0
    } else {
        layout_height
    }
}

fn reader_fallback_source_index(native: usize, native_len: usize, source_len: usize) -> usize {
    if native_len == 0 {
        return 0;
    }
    ((native.min(native_len) as f64 / native_len as f64) * source_len as f64) as usize
}

fn rects_relative_to(rects: &[Rect], origin: DVec2) -> Vec<Rect> {
    rects
        .iter()
        .map(|rect| Rect {
            pos: rect.pos - origin,
            size: rect.size,
        })
        .collect()
}

struct UnifiedReaderLayout {
    laidout: Rc<LaidoutText>,
    origin: DVec2,
    font_scale: f32,
    text_len: usize,
}

/// 把每行的选区扩展到相邻字形框之间的中线。相邻两行选中时，前一行的
/// bottom 与后一行的 top 因而完全相同，不会在行距中留下未着色的细缝。
#[cfg(test)]
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

/// Html/TextFlow 的 Area 在含自定义内联 widget（例如 RubyWord）时可能只反映
/// 当前已使用的内容宽度。阅读段落自身是 Fill，统一命中/着色必须以宿主段落的
/// 右边界为准，否则纯文本覆盖层会比正文更早换行，只覆盖视觉行的左半段。
fn reader_markup_available_width(
    host_rect: Rect,
    html_rect: Rect,
    padding_left: f64,
    padding_right: f64,
) -> f32 {
    let host_right = host_rect.pos.x + host_rect.size.x;
    let html_right = html_rect.pos.x + html_rect.size.x;
    let content_right = host_right.max(html_right);
    (content_right - html_rect.pos.x - padding_left - padding_right).max(1.0) as f32
}

impl ChapterList {
    fn redraw_visible_code_views(&mut self, cx: &mut Cx) {
        // Code blocks keep their own draw lists. Repaint both levels after a page
        // switch so glyphs cached while another list was scrolling are refreshed.
        let codes = CODE_BLOCKS.read().unwrap();
        for (block, item, _) in &self.pager.drawn {
            if codes.get(*block).is_some_and(Option::is_some) {
                item.redraw(cx);
                item.widget(cx, ids!(code_view)).redraw(cx);
            }
        }
    }
    fn cancel_pending_reader_tap(&mut self, cx: &mut Cx) {
        cx.stop_timer(self.reader_tap_timer);
        self.reader_tap_timer = Timer::empty();
        self.pending_reader_tap.clear();
        self.pending_tap_context = None;
    }

    fn cancel_reader_tap(&mut self, cx: &mut Cx) {
        self.word_tap.cancel();
        self.toolbar_tap.cancel();
        self.cancel_pending_reader_tap(cx);
    }

    fn defer_reader_tap(&mut self, cx: &mut Cx, actions: ActionsBuf) {
        if actions.is_empty() {
            return;
        }
        cx.extend_actions(std::mem::take(&mut self.pending_reader_tap));
        cx.stop_timer(self.reader_tap_timer);
        self.pending_reader_tap = actions;
        self.pending_tap_context = Some((
            READER_CONTENT_VERSION.load(Ordering::Relaxed),
            CURRENT_CHAPTER.load(Ordering::Relaxed),
        ));
        self.reader_tap_timer =
            cx.start_timeout(makepad_widgets::makepad_platform::event::finger::TAP_COUNT_TIME);
    }

    fn word_at_pointer(&self, cx: &mut Cx, abs: DVec2) -> Option<TextRange> {
        // Unlike drag selection, double-click must not snap blank margins to
        // an unrelated paragraph. Hit the painted glyph rather than its caret.
        for (id, widget, clip) in self.active_items(cx) {
            if !clip.contains(abs) {
                continue;
            }
            let paragraphs = PARAGRAPHS.read().unwrap();
            let paragraph = paragraphs.get(id)?;
            let offset = if let Some((_, _, runs)) = self
                .hit_runs
                .get(&widget.widget_uid())
                .filter(|(block, hash, _)| *block == id && *hash == reader_source_hash(paragraph))
            {
                let native =
                    crate::reader_hit::hit_word_index(runs, abs - widget.area().rect(cx).pos)?;
                self.text_map(id, paragraph)?.source_index(native)
            } else {
                self.hit_block_char(cx, abs)
                    .filter(|(block, _)| *block == id)?
                    .1
            };
            let text = readwise_core::vocab::annotate::rendered_text(paragraph);
            let (start, end) = word_range_at(&text, offset)?;
            return Some(((id, start), (id, end)));
        }
        None
    }

    fn select_pointer_word(&mut self, cx: &mut Cx, abs: DVec2) {
        self.reset_selection_gesture();
        *SEL_RANGE.write().unwrap() = self.word_at_pointer(cx, abs);
        self.last_abs = Some(abs);
        SELECTED_TEXT.write().unwrap().clear();
        self.apply_visible_selection(cx);
        self.finish_selection();
    }

    fn track_toolbar_tap(&mut self, cx: &mut Cx, event: &Event) -> bool {
        if !self.compact_layout {
            self.toolbar_tap.cancel();
            return false;
        }
        if let Event::TouchUpdate(event) = event {
            if event.touches.len() != 1 {
                self.toolbar_tap.cancel();
                return false;
            }
            let touch = &event.touches[0];
            match touch.state {
                TouchState::Start => {
                    if self.reader_rect(cx).contains(touch.abs)
                        && !SEL_RANGE.read().unwrap().is_some_and(|(a, b)| a != b)
                    {
                        self.toolbar_tap.begin(touch.uid, touch.abs, event.time);
                    } else {
                        self.toolbar_tap.cancel();
                    }
                }
                TouchState::Move => self.toolbar_tap.move_to(touch.uid, touch.abs),
                TouchState::Stop => {
                    let tapped = self.toolbar_tap.end(touch.uid, touch.abs, event.time);
                    return tapped && self.reader_rect(cx).contains(touch.abs);
                }
                TouchState::Stable => {}
            }
        }
        // External pointers on phones use the same tap/drag policy.
        match event {
            Event::MouseDown(event) if event.button == MouseButton::PRIMARY => {
                if self.reader_rect(cx).contains(event.abs)
                    && !SEL_RANGE.read().unwrap().is_some_and(|(a, b)| a != b)
                {
                    self.toolbar_tap.begin(0, event.abs, event.time);
                } else {
                    self.toolbar_tap.cancel();
                }
            }
            Event::MouseMove(event) => self.toolbar_tap.move_to(0, event.abs),
            Event::MouseUp(event) if event.button == MouseButton::PRIMARY => {
                let tapped = self.toolbar_tap.end(0, event.abs, event.time);
                return tapped && self.reader_rect(cx).contains(event.abs);
            }
            Event::LongPress(_) | Event::SelectionHandleDrag(_) => self.toolbar_tap.cancel(),
            _ => {}
        }
        false
    }

    fn source_anchor(&self, block: usize, scroll: f64) -> Option<(usize, usize)> {
        self.pager
            .requested_source
            .or(self.pager.pending_source)
            .filter(|(id, _)| *id == block)
            .or_else(|| {
                (self.pager.content_version == READER_CONTENT_VERSION.load(Ordering::Relaxed))
                    .then(|| self.pager.metrics.get(block).and_then(Option::as_ref))
                    .flatten()
                    .filter(|metric| scroll >= -0.5 || !metric.boundaries.is_empty())
                    .map(|metric| (block, metric.source_at((-scroll).max(0.0))))
            })
    }

    fn draw_scrolling(
        &mut self,
        cx: &mut Cx2d,
        scope: &mut Scope,
        walk: Walk,
        viewport: Rect,
    ) -> DrawStep {
        self.page_turn.cancel();
        let list_ref = self.view.portal_list(cx, ids!(list));
        let content_version = READER_CONTENT_VERSION.load(Ordering::Relaxed);
        let signature = reader_source_hash(&format!(
            "scroll:{content_version}:{}:{}:{:?}:{:?}:{:?}:{}:{}:{}:{}:{}:{}:{}",
            viewport.size.x,
            self.compact_layout,
            *READER_FONT.read().unwrap(),
            *READER_FONT_SIZE.read().unwrap(),
            *READER_SPACING.read().unwrap(),
            *crate::ruby_word::GLOSS_FONT_SIZE.read().unwrap(),
            *GLOSS_ENABLED.read().unwrap(),
            *READER_BOLD_TEXT.read().unwrap(),
            *READER_MARGIN.read().unwrap(),
            *READER_CHARACTER_SPACING.read().unwrap(),
            *READER_WORD_SPACING.read().unwrap(),
            *READER_JUSTIFY_TEXT.read().unwrap()
        ));
        let paragraphs = PARAGRAPHS.read().unwrap();
        let code_blocks = CODE_BLOCKS.read().unwrap();
        if self.pager.signature != signature || self.pager.metrics.len() != paragraphs.len() {
            // Retain the source character at the top, not an old pixel offset
            // measured at the previous column width/font size.
            self.pager.pending_source =
                self.source_anchor(list_ref.first_id(), list_ref.scroll_position());
            self.pager.metrics = vec![None; paragraphs.len()];
            self.pager.signature = signature;
            self.pager.content_version = content_version;
            self.range_geometry_cache.clear();
            self.item_layout_widths.clear();
            self.hit_runs.clear();
            self.text_maps.clear();
        }
        if let Some(source) = self.pager.requested_source.take() {
            self.pager.pending_source = Some(source);
        }
        self.pager.pending_source = self
            .pager
            .pending_source
            .filter(|(block, _)| *block < paragraphs.len());
        self.pager.active = false;
        self.pager.drawn.clear();
        self.pager.page_index = Default::default();
        self.pager.pending_turn = None;
        self.pager.turn_history.clear();
        self.pager.measure_frame = NextFrame::default();
        self.view.view(cx, ids!(list_host)).set_visible(cx, true);
        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_portal_list().borrow_mut() {
                self.hit_runs.clear();
                let mut drawn_blocks = HashSet::new();
                list.set_item_range(cx, 0, paragraphs.len());
                while let Some(item_id) = list.next_visible_item(cx) {
                    if let Some(para) = paragraphs.get(item_id) {
                        drawn_blocks.insert(item_id);
                        let code = code_blocks.get(item_id).and_then(Option::as_deref);
                        let metrics = self.draw_reader_paragraph(
                            cx, &mut list, item_id, item_id, para, code, false,
                        );
                        self.pager.metrics[item_id] = Some(metrics);
                    }
                }
                self.text_maps.retain(|id, _| drawn_blocks.contains(id));
            }
        }
        // Measure only the top/resume block, at its actual rendered width.
        // A separate clipped slot can probe the whole paragraph without
        // replacing the visible selection/highlight probe or its widget.
        let block = self
            .pager
            .pending_source
            .map_or(list_ref.first_id(), |(block, _)| block);
        if self.pager.metrics.get(block).is_some_and(Option::is_none) {
            let width = list_ref
                .get_item(list_ref.first_id())
                .map(|(_, item)| item.area().rect(cx).size.x)
                .unwrap_or(0.0);
            if width > 0.0 {
                if let Some(mut pool) = list_ref.borrow_mut() {
                    cx.begin_turtle(
                        Walk {
                            abs_pos: Some(viewport.pos),
                            width: Size::Fixed(width),
                            height: Size::Fixed(0.0),
                            ..Walk::default()
                        },
                        Layout::flow_down(),
                    );
                    self.pager.metrics[block] = Some(self.draw_reader_paragraph(
                        cx,
                        &mut pool,
                        block,
                        paragraphs.len(),
                        &paragraphs[block],
                        code_blocks.get(block).and_then(Option::as_deref),
                        true,
                    ));
                    cx.end_turtle();
                }
            }
        }
        if self.pager.requested_last_page {
            self.pager.requested_last_page = false;
            self.pager.pending_source = None;
            list_ref.scroll_to_end(cx);
            self.pager.measure_frame = cx.new_next_frame();
        } else if let Some((block, source)) = self.pager.pending_source {
            if let Some(metric) = self.pager.metrics.get(block).and_then(Option::as_ref) {
                list_ref.set_first_id_and_scroll(block, -metric.y_at(source));
                self.pager.pending_source = None;
                self.pager.measure_frame = cx.new_next_frame();
            }
        }
        DrawStep::done()
    }

    fn draw_pages(
        &mut self,
        cx: &mut Cx2d,
        scope: &mut Scope,
        walk: Walk,
        viewport: Rect,
    ) -> DrawStep {
        use crate::reader_columns::{compose_page, ColumnLayout, PageAnchor};
        let font = READER_FONT_SIZE
            .read()
            .unwrap()
            .unwrap_or(self.font_default_size);
        let layout = ColumnLayout::resolve_with_margin(
            *READER_COLUMNS.read().unwrap(),
            viewport.size.x,
            viewport.size.y,
            font as f64,
            self.pager.layout.count,
            ReaderSettings::margin_pixels(*READER_MARGIN.read().unwrap()) as f64,
        );
        let content_version = READER_CONTENT_VERSION.load(Ordering::Relaxed);
        let layout_signature = reader_source_hash(&format!(
            "{:?}:{:?}:{}:{:?}:{:?}:{}:{}:{}:{}:{}:{}:{}:{}",
            layout.width,
            layout.height,
            layout.count,
            *READER_FONT.read().unwrap(),
            *READER_SPACING.read().unwrap(),
            font,
            *crate::ruby_word::GLOSS_FONT_SIZE.read().unwrap(),
            *GLOSS_ENABLED.read().unwrap(),
            *READER_BOLD_TEXT.read().unwrap(),
            *READER_CHARACTER_SPACING.read().unwrap(),
            *READER_WORD_SPACING.read().unwrap(),
            *READER_JUSTIFY_TEXT.read().unwrap(),
            cx.current_dpi_factor()
        ));
        let chapter = CURRENT_CHAPTER.load(Ordering::Relaxed);
        if let Some((_, key, chapters)) = reader_book_content_key() {
            self.book_pages.configure(key, layout_signature, chapters);
        }
        let signature = reader_page_layout_signature(chapter, content_version, layout_signature);
        let list_ref = self.view.portal_list(cx, ids!(list));
        let mut anchor = PageAnchor {
            block: list_ref.first_id(),
            y: (-list_ref.scroll_position()).max(0.0),
        };
        let paragraphs = PARAGRAPHS.read().unwrap();
        let codes = CODE_BLOCKS.read().unwrap();
        if self.pager.signature != signature || self.pager.metrics.len() != paragraphs.len() {
            self.pager.needs_page_alignment = true;
            if self.pager.content_version == content_version && self.pager.active {
                self.pager.pending_source = self.pager.pending_source.or_else(|| {
                    self.pager
                        .metrics
                        .get(anchor.block)
                        .and_then(Option::as_ref)
                        .map(|metric| (anchor.block, metric.source_at(anchor.y)))
                });
            } else {
                self.pager.pending_source = None;
            }
            (self.pager.metrics, self.pager.page_index) = self
                .book_pages
                .restore(chapter, paragraphs.len())
                .unwrap_or_else(|| (vec![None; paragraphs.len()], Default::default()));
            self.pager.turn_history.clear();
            self.pager.signature = signature;
            self.pager.content_version = content_version;
            if let Some(requested) = self.pager.requested_source.take() {
                self.pager.pending_source = Some(requested);
            }
            self.range_geometry_cache.clear();
            self.item_layout_widths.clear();
            self.hit_runs.clear();
            self.text_maps.clear();
        }
        self.pager.active = true;
        if let Some(requested) = self.pager.requested_source.take() {
            self.pager.pending_source = Some(requested);
            self.pager.needs_page_alignment = true;
            self.pager.turn_history.clear();
        }
        self.pager.pending_source = self
            .pager
            .pending_source
            .filter(|(block, _)| *block < paragraphs.len());
        self.pager.layout = layout;
        self.pager.drawn.clear();
        self.view.view(cx, ids!(list_host)).set_visible(cx, false);
        let _ = self.view.draw_walk(cx, scope, walk);
        let Some(mut pool) = list_ref.borrow_mut() else {
            return DrawStep::done();
        };
        let started = Instant::now();
        let mut measured = 0;
        let mut pages = Vec::new();
        loop {
            pages.clear();
            let mut cursor = anchor;
            let mut missing = None;
            if let Some((block, source)) = self.pager.pending_source {
                if let Some(metric) = self.pager.metrics.get(block).and_then(Option::as_ref) {
                    anchor = PageAnchor {
                        block,
                        y: metric.y_at(source),
                    };
                    cursor = anchor;
                    pool.set_first_id_and_scroll(anchor.block, -anchor.y);
                    self.pager.pending_source = None;
                } else {
                    missing = Some(block);
                }
            }
            if missing.is_none() {
                for _ in 0..layout.count {
                    match compose_page(&self.pager.metrics, cursor, layout.height) {
                        Ok(page) => {
                            cursor = page.next;
                            pages.push(page);
                        }
                        Err(block) => {
                            missing = Some(block);
                            break;
                        }
                    }
                }
            }
            let Some(block) = missing else {
                self.pager.next = cursor;
                break;
            };
            if measured >= 8 || (measured > 0 && started.elapsed().as_millis() >= 10) {
                pages.clear();
                self.view.area().redraw(cx);
                break;
            }
            let Some(para) = paragraphs.get(block) else {
                break;
            };
            // A zero-height clipped measuring viewport uses the exact display
            // width while keeping all measurement pixels off screen.
            cx.begin_turtle(
                Walk {
                    abs_pos: Some(viewport.pos),
                    width: Size::Fixed(layout.width),
                    height: Size::Fixed(0.0),
                    ..Walk::default()
                },
                Layout::flow_down(),
            );
            let metric = self.draw_reader_paragraph(
                cx,
                &mut pool,
                block,
                measured,
                para,
                codes.get(block).and_then(Option::as_deref),
                true,
            );
            cx.end_turtle();
            self.pager.metrics[block] = Some(metric);
            measured += 1;
        }
        // Fill in the chapter index progressively. UI text layout stays on the
        // rendering thread, with bounded work per frame instead of blocking an
        // entire chapter before showing its first page.
        while measured < 8 && started.elapsed().as_millis() < 10 {
            // Warm the next spread first so a quick next turn does not wait
            // behind measurement of unrelated blocks elsewhere in the chapter.
            let mut cursor = self.pager.next;
            let mut upcoming = None;
            for _ in 0..layout.count {
                match compose_page(&self.pager.metrics, cursor, layout.height) {
                    Ok(page) => cursor = page.next,
                    Err(block) => {
                        upcoming = Some(block);
                        break;
                    }
                }
            }
            let preceding = self.book_pages.next_work(chapter, true);
            let block = upcoming.or_else(|| {
                preceding
                    .is_none()
                    .then(|| self.pager.metrics.iter().position(Option::is_none))
                    .flatten()
            });
            if let Some(block) = block {
                cx.begin_turtle(
                    Walk {
                        abs_pos: Some(viewport.pos),
                        width: Size::Fixed(layout.width),
                        height: Size::Fixed(0.0),
                        ..Walk::default()
                    },
                    Layout::flow_down(),
                );
                let metric = self.draw_reader_paragraph(
                    cx,
                    &mut pool,
                    block,
                    measured,
                    &paragraphs[block],
                    codes.get(block).and_then(Option::as_deref),
                    true,
                );
                cx.end_turtle();
                self.pager.metrics[block] = Some(metric);
                measured += 1;
            } else if let Some((other_chapter, block, content)) =
                preceding.or_else(|| self.book_pages.next_work(chapter, false))
            {
                // Measure other chapters with the display renderer and isolate
                // their source/selection caches from the visible chapter.
                let geometry = std::mem::take(&mut self.range_geometry_cache);
                let widths = std::mem::take(&mut self.item_layout_widths);
                let runs = std::mem::take(&mut self.hit_runs);
                let maps = std::mem::take(&mut self.text_maps);
                cx.begin_turtle(
                    Walk {
                        abs_pos: Some(viewport.pos),
                        width: Size::Fixed(layout.width),
                        height: Size::Fixed(0.0),
                        ..Walk::default()
                    },
                    Layout::flow_down(),
                );
                let para = &content.chapters[other_chapter];
                let metric = self.draw_reader_paragraph(
                    cx,
                    &mut pool,
                    block,
                    measured,
                    &para.paragraphs[block],
                    para.codes[block].as_deref(),
                    true,
                );
                cx.end_turtle();
                self.range_geometry_cache = geometry;
                self.item_layout_widths = widths;
                self.hit_runs = runs;
                self.text_maps = maps;
                self.book_pages
                    .record_block(other_chapter, block, metric, layout.height);
                measured += 1;
            } else {
                break;
            }
        }
        self.pager.anchor = anchor;
        self.pager.spread_ready = !pages.is_empty();
        self.pager.visible_pages = pages
            .iter()
            .filter(|page| !page.fragments.is_empty())
            .count();
        self.hit_runs.clear();
        for (column, page) in pages.iter().enumerate() {
            for (slot, fragment) in page.fragments.iter().enumerate() {
                let pos = viewport.pos
                    + dvec2(
                        layout.left + column as f64 * (layout.width + layout.gap),
                        layout.top + fragment.top,
                    );
                let clip = Rect {
                    pos,
                    size: dvec2(layout.width, fragment.to - fragment.from),
                };
                cx.begin_turtle(
                    Walk {
                        abs_pos: Some(pos),
                        width: Size::Fixed(clip.size.x),
                        height: Size::Fixed(clip.size.y),
                        ..Walk::default()
                    },
                    Layout::flow_down(),
                );
                cx.begin_turtle(
                    Walk {
                        abs_pos: Some(pos - dvec2(0.0, fragment.from)),
                        width: Size::Fixed(layout.width),
                        height: Size::fit(),
                        ..Walk::default()
                    },
                    Layout::flow_down(),
                );
                // Reuse a bounded set of visible slots, not one retained widget
                // for every paragraph ever visited in a long book.
                // Keep IDs dense: native PortalList event dispatch visits
                // min_id..=max_id when switching back to continuous scrolling.
                // Slots 0..8 are reserved for offscreen measurement above.
                let cache_id = 8 + slot * 2 + column;
                self.draw_reader_paragraph(
                    cx,
                    &mut pool,
                    fragment.block,
                    cache_id,
                    &paragraphs[fragment.block],
                    codes.get(fragment.block).and_then(Option::as_deref),
                    false,
                );
                if let Some((_, widget)) = pool.get_item(cache_id) {
                    self.pager.drawn.push((fragment.block, widget, clip));
                }
                cx.end_turtle();
                cx.end_turtle();
            }
        }
        self.pager
            .page_index
            .advance(&self.pager.metrics, layout.height);
        self.book_pages
            .record_current(chapter, &self.pager.metrics, &self.pager.page_index);
        if self.pager.page_index.complete && self.pager.requested_last_page {
            let last = self.pager.page_index.starts.len().saturating_sub(1);
            let target = self
                .pager
                .page_index
                .starts
                .get(last / layout.count * layout.count)
                .copied()
                .unwrap_or_default();
            pool.set_first_id_and_scroll(target.block, -target.y);
            self.pager.requested_last_page = false;
            self.pager.needs_page_alignment = false;
            self.pager.spread_ready = false;
            self.pager.measure_frame = cx.new_next_frame();
        } else if self.pager.needs_page_alignment && self.pager.pending_source.is_none() {
            // A restored source offset must remain visible, but subsequent
            // turns must land on the same canonical page starts as the footer.
            if let Some(target) = self.pager.page_index.containing(anchor) {
                if target != anchor {
                    pool.set_first_id_and_scroll(target.block, -target.y);
                    self.pager.spread_ready = false;
                    self.pager.measure_frame = cx.new_next_frame();
                }
                self.pager.needs_page_alignment = false;
            }
        }
        if !self.pager.page_index.complete || self.book_pages.next_work(chapter, false).is_some() {
            self.pager.measure_frame = cx.new_next_frame();
        }
        self.text_maps
            .retain(|id, _| self.pager.drawn.iter().any(|(block, _, _)| block == id));
        drop(pool);
        drop(codes);
        drop(paragraphs);
        if self.pager.extend_selection_after_draw && !pages.is_empty() {
            self.pager.extend_selection_after_draw = false;
            if self.selecting {
                self.extend_after_scroll(cx);
            }
        }
        DrawStep::done()
    }

    fn active_items(&self, cx: &mut Cx) -> Vec<(usize, WidgetRef, Rect)> {
        if self.pager.active {
            return self.pager.drawn.clone();
        }
        let list = self.view.portal_list(cx, ids!(list));
        (list.first_id()..list.first_id() + list.visible_items() + 1)
            .filter_map(|id| {
                list.get_item(id).map(|(_, item)| {
                    let clip = item.area().clipped_rect(cx);
                    (id, item, clip)
                })
            })
            .collect()
    }

    fn reader_rect(&self, cx: &mut Cx) -> Rect {
        if self.pager.active {
            self.view.area().rect(cx)
        } else {
            self.view.portal_list(cx, ids!(list)).area().rect(cx)
        }
    }

    /// Returns false only at an actual chapter boundary. Missing measurements
    /// keep the current page in place until the next draw completes them.
    fn turn_page(&mut self, cx: &mut Cx, forward: bool) -> bool {
        use crate::reader_columns::PageAnchor;
        if self.pager.page_ready() {
            // The current destination has already been painted. A quick
            // opposite-direction click should turn from it immediately,
            // even while its visual transition is still finishing.
            self.page_turn.finish_if_ready();
        }
        if self.page_turn.busy() || !self.pager.page_ready() {
            self.pager.pending_turn = Some(forward);
            self.pager.measure_frame = cx.new_next_frame();
            return true;
        }
        self.pager.pending_turn = None;
        let target = if forward {
            if self.pager.next.block >= self.pager.metrics.len() {
                return false;
            }
            self.pager.turn_history.push(self.pager.anchor);
            self.pager.next
        } else if let Some(previous) = self.pager.turn_history.pop() {
            previous
        } else {
            if self.pager.anchor == PageAnchor::default() {
                return false;
            }
            if self
                .pager
                .page_index
                .containing(self.pager.anchor)
                .is_none()
            {
                self.pager.pending_turn = Some(false);
                self.pager.measure_frame = cx.new_next_frame();
                return true;
            }
            let current = self.pager.page_index.starts.partition_point(|a| {
                a.block < self.pager.anchor.block
                    || (a.block == self.pager.anchor.block && a.y < self.pager.anchor.y - 0.5)
            });
            self.pager
                .page_index
                .starts
                .get(current.saturating_sub(self.pager.layout.count))
                .copied()
                .unwrap_or_default()
        };
        self.view
            .portal_list(cx, ids!(list))
            .set_first_id_and_scroll(target.block, -target.y);
        self.pager.anchor = target;
        // This is an intentional reading move, not a restored offset. Do not
        // snap it backwards later when the rest of the index becomes available.
        self.pager.needs_page_alignment = false;
        self.pager.spread_ready = false;
        self.pager.extend_selection_after_draw = self.selecting;
        if !self.selecting {
            self.page_turn.request(cx, forward);
        }
        self.view.redraw(cx);
        true
    }

    fn text_map(&self, block: usize, source: &str) -> Option<&crate::reader_text::TextMap> {
        self.text_maps
            .get(&block)
            .filter(|(hash, _)| *hash == reader_source_hash(source))
            .map(|(_, map)| map)
    }

    fn reset_selection_gesture(&mut self) {
        self.sel_anchor = None;
        self.sel_cursor = None;
        self.selecting = false;
        self.last_abs = None;
        self.touch_selection = None;
    }

    fn set_compact_layout(&mut self, cx: &mut Cx, compact_layout: bool) {
        if self.compact_layout == compact_layout {
            return;
        }
        self.compact_layout = compact_layout;
        self.cancel_reader_tap(cx);
        self.apply_reader_margin(cx);
        let drag_scrolling = compact_layout || cfg!(any(target_os = "android", target_os = "ios"));
        let mut list_widget = self.view.widget(cx, ids!(list));
        script_apply_eval!(cx, list_widget, {
            drag_scrolling: #(drag_scrolling)
        });
        let list = self.view.portal_list(cx, ids!(list));
        if let Some(mut inner) = list.borrow_mut() {
            inner.selectable = !compact_layout;
            inner.clear_selection(cx);
        }
        self.reset_selection_gesture();
        *SEL_RANGE.write().unwrap() = None;
        SELECTED_TEXT.write().unwrap().clear();
        Cx::post_action(SelAction::Hide);
        self.view.redraw(cx);
    }

    fn apply_reader_margin(&mut self, cx: &mut Cx) {
        let margin = ReaderSettings::margin_pixels(*READER_MARGIN.read().unwrap()) as f64;
        let edge = if self.compact_layout {
            margin.min(24.0)
        } else {
            margin
        };
        let top = (margin - 20.0).clamp(8.0, 60.0);
        let reader_padding = Inset {
            left: edge,
            right: edge,
            top,
            bottom: top,
        };
        let mut list_widget = self.view.widget(cx, ids!(list));
        script_apply_eval!(cx, list_widget, {padding: #(reader_padding)});
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
        let host_rect = widget.area().rect(cx);
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
        let max_width = reader_markup_available_width(host_rect, rect, padding_left, padding_right);
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

    /// 正文统一按连续的可见文本布局命中，不读取 TextFlow 每次重绘生成的
    /// SelectionTracker Area。这样重复选择、取消和窗口缩放都使用同一套坐标。
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

    /// 命中可见块的 (块id, 正文字符下标)。点位在视口外或块间隙时吸附到
    /// 最近边界，与 PortalList 原生选择规则保持一致。
    fn hit_block_char(&self, cx: &mut Cx, abs: DVec2) -> Option<(usize, usize)> {
        let mut abs = abs;
        let mut items = self.active_items(cx);
        if self.pager.active {
            let closest = items
                .iter()
                .map(|(_, _, clip)| {
                    (clip.pos.x - abs.x)
                        .max(abs.x - clip.pos.x - clip.size.x)
                        .max(0.0)
                })
                .fold(f64::INFINITY, f64::min);
            items.retain(|(_, _, clip)| {
                (clip.pos.x - abs.x)
                    .max(abs.x - clip.pos.x - clip.size.x)
                    .max(0.0)
                    <= closest + 0.5
            });
            let (_, _, nearest) = items.iter().min_by(|a, b| {
                let distance =
                    |r: &Rect| (r.pos.y - abs.y).max(abs.y - r.pos.y - r.size.y).max(0.0);
                distance(&a.2).total_cmp(&distance(&b.2))
            })?;
            abs.y = abs
                .y
                .clamp(nearest.pos.y + 0.1, nearest.pos.y + nearest.size.y - 0.1);
        }
        let mut visible: Vec<_> = items
            .into_iter()
            .map(|(id, widget, _)| {
                let rect = widget.area().rect(cx);
                (id, widget, rect)
            })
            .collect();
        visible.sort_by_key(|(id, _, _)| *id);
        let (first_id, _, _) = visible.first()?;
        for (id, widget, rect) in &visible {
            // Horizontal margin is still on this visual row. Sending it to
            // the block/viewport start selects unrelated preceding paragraphs.
            if crate::reader_hit::contains_row(*rect, abs.y) {
                let paragraphs = PARAGRAPHS.read().unwrap();
                let paragraph = paragraphs.get(*id)?;
                let plain_len = readwise_core::vocab::annotate::rendered_text(paragraph).len();
                if let Some((run_block, source_hash, runs)) =
                    self.hit_runs.get(&widget.widget_uid())
                {
                    if *run_block == *id && *source_hash == reader_source_hash(paragraph) {
                        if let Some(index) = crate::reader_hit::hit_index(runs, abs - rect.pos) {
                            return Some((*id, self.text_map(*id, paragraph)?.source_index(index)));
                        }
                    }
                }
                let is_code = CODE_BLOCKS
                    .read()
                    .unwrap()
                    .get(*id)
                    .and_then(Option::as_ref)
                    .is_some();
                let char_idx = if supports_unified_reader_hit_test(paragraph) && !is_code {
                    let fallback = self
                        .unified_markup_hit_index(cx, widget, paragraph, abs)
                        .map(|index| index.min(plain_len));
                    let native = self.text_map(*id, paragraph).and_then(|map| {
                        widget
                            .selection_point_to_char_index(cx, abs)
                            .map(|index| map.source_index(index))
                    });
                    if let (Some(native), Some(fallback), Some(previous), Some(previous_abs)) = (
                        native,
                        fallback,
                        self.sel_cursor.filter(|point| point.0 == *id),
                        self.last_abs,
                    ) {
                        let pointer_moved = self
                            .last_abs
                            .map(|last| (abs - last).length() > 1.0)
                            .unwrap_or(false);
                        choose_reader_hit_index(
                            native,
                            fallback,
                            previous.1,
                            reader_drag_moves_forward(previous_abs, abs),
                            pointer_moved,
                        )
                    } else {
                        // A paginated right-column Html item can paint correctly
                        // while its native selection Area is absent. In that case
                        // use the same source-text layout as the drag fallback.
                        native.or(fallback)?
                    }
                } else {
                    let native = widget.selection_point_to_char_index(cx, abs)?;
                    if is_code {
                        native
                    } else {
                        self.text_map(*id, paragraph)?.source_index(native)
                    }
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

    fn set_cursor(&mut self, cursor: (usize, usize)) {
        let Some(anchor) = self.sel_anchor else {
            return;
        };
        self.sel_cursor = Some(cursor);
        *SEL_RANGE.write().unwrap() = Some(ordered_selection_range(anchor, cursor));
    }

    /// Pointer platforms may coalesce the final move before a release. Always hit-test
    /// the release coordinate itself so the persisted range and painted range end at the
    /// place where the reader actually let go, rather than at an older MouseMove sample.
    fn update_selection_cursor_at(&mut self, cx: &mut Cx, abs: DVec2) {
        if let Some(cursor) = self.hit_block_char(cx, abs) {
            self.set_cursor(cursor);
            self.apply_visible_selection(cx);
        }
        self.last_abs = Some(abs);
    }

    /// PortalList 会复用/回收离屏 item，因此每次滚动与重绘都要把全局区间
    /// 重新投影到当前可见 widget，而不能依赖上一次的原生选择状态。
    fn apply_visible_selection(&mut self, cx: &mut Cx) {
        let range = *SEL_RANGE.read().unwrap();
        let paragraphs = PARAGRAPHS.read().unwrap();
        let list = self.view.portal_list(cx, ids!(list));
        for (id, widget, _) in self.active_items(cx) {
            let paragraph = paragraphs.get(id).map(String::as_str).unwrap_or("");
            let source_len = readwise_core::vocab::annotate::rendered_text(paragraph).len();
            let is_code = CODE_BLOCKS
                .read()
                .unwrap()
                .get(id)
                .is_some_and(Option::is_some);
            let native_range = range
                .and_then(|range| selection_range_in_block(range, id, source_len))
                .and_then(|range| {
                    if is_code {
                        Some(range)
                    } else {
                        self.text_map(id, paragraph)?.native_range(range)
                    }
                });
            if let Some((start, end)) = native_range {
                widget.selection_set(start, end);
            } else {
                widget.selection_clear();
            }
            widget.redraw(cx);
        }
        list.redraw(cx);
        self.view.redraw(cx);
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
        let popup_anchor = self.last_abs;
        let range = *SEL_RANGE.read().unwrap();
        let non_empty = range.map(|(a, b)| a != b).unwrap_or(false);
        if let Some(range) = range.filter(|(a, b)| a != b) {
            *SELECTED_TEXT.write().unwrap() =
                readwise_core::vocab::annotate::rendered_text_for_range(
                    &PARAGRAPHS.read().unwrap(),
                    range,
                );
        }
        self.reset_selection_gesture();
        Cx::post_action(if non_empty {
            SelAction::Show {
                anchor: popup_anchor,
            }
        } else {
            SelAction::Hide
        });
    }

    fn handle_child_event(
        &mut self,
        cx: &mut Cx,
        event: &Event,
        scope: &mut Scope,
        selecting_touch: bool,
    ) {
        if self.pager.active {
            for (_, item, _) in self.active_items(cx) {
                let html = item.widget(cx, ids!(para_html));
                let previous = html
                    .borrow_mut::<Html>()
                    .map(|mut h| std::mem::replace(&mut h.text_flow.selectable, false));
                if selecting_touch {
                    let _ = cx.capture_actions(|cx| item.handle_event(cx, event, scope));
                } else {
                    item.handle_event(cx, event, scope);
                }
                if let (Some(previous), Some(mut h)) = (previous, html.borrow_mut::<Html>()) {
                    h.text_flow.selectable = previous;
                };
            }
            return;
        }
        if !matches!(event, Event::TouchUpdate(_) | Event::LongPress(_)) {
            self.view.handle_event(cx, event, scope);
            return;
        }
        // Keep selection metadata enabled while drawing, but let ChapterList
        // own touch gestures. TextFlow's standalone touch handling selects on
        // finger-down and shows a native copy toolbar, conflicting with scroll
        // and our highlight menu. Inline widgets must still receive ordinary taps.
        let list = self.view.portal_list(cx, ids!(list));
        let list_selectable = list
            .borrow_mut()
            .map(|mut inner| std::mem::replace(&mut inner.selectable, false));
        let mut flows = Vec::new();
        for id in list.first_id()..list.first_id() + list.visible_items() + 1 {
            if let Some((_, item)) = list.get_item(id) {
                let html = item.widget(cx, ids!(para_html));
                let previous = html
                    .borrow_mut::<Html>()
                    .map(|mut inner| std::mem::replace(&mut inner.text_flow.selectable, false));
                if let Some(previous) = previous {
                    flows.push((html, previous));
                }
            }
        }
        if selecting_touch {
            // Deliver long-press/release for capture cleanup (including RubyWord
            // and PortalList), without opening word cards or activating links.
            let _ = cx.capture_actions(|cx| self.view.handle_event(cx, event, scope));
        } else {
            self.view.handle_event(cx, event, scope);
        }
        for (html, previous) in flows {
            if let Some(mut inner) = html.borrow_mut::<Html>() {
                inner.text_flow.selectable = previous;
            }
        }
        if let (Some(previous), Some(mut inner)) = (list_selectable, list.borrow_mut()) {
            inner.selectable = previous;
        };
    }

    fn track_selection(&mut self, cx: &mut Cx, event: &Event) {
        match event {
            Event::MouseDown(e) => {
                if e.button != MouseButton::PRIMARY {
                    return;
                }
                if !self.reader_rect(cx).contains(e.abs) {
                    return;
                }
                let had_selection = SEL_RANGE.write().unwrap().take().is_some();
                SELECTED_TEXT.write().unwrap().clear();
                self.reset_selection_gesture();
                self.last_abs = Some(e.abs);
                Cx::post_action(SelAction::Hide);
                // 清除上一轮原生选区的 item 状态后再建立新锚点，避免快速
                // “选择→取消→再选择”时旧选区 Area 留到下一次完整 Draw。
                if had_selection {
                    self.apply_visible_selection(cx);
                }
                // Inline text widgets (ruby words and links) must keep receiving
                // clicks, but they are also valid text-selection starting points.
                // Their FingerUp `was_tap()` check distinguishes a click from a drag.
                if let Some(hit) = self.hit_block_char(cx, e.abs) {
                    self.sel_anchor = Some(hit);
                    self.sel_cursor = Some(hit);
                    self.selecting = true;
                }
            }
            Event::MouseMove(e) => {
                if self.selecting {
                    self.update_selection_cursor_at(cx, e.abs);
                }
            }
            Event::Scroll(_) if self.selecting => self.extend_after_scroll(cx),
            Event::MouseUp(event) if self.selecting => {
                self.update_selection_cursor_at(cx, event.abs);
                self.finish_selection();
            }
            Event::LongPress(event) => {
                if !self.reader_rect(cx).contains(event.abs) {
                    return;
                }
                let Some(((block_id, start), (_, end))) = self.word_at_pointer(cx, event.abs)
                else {
                    return;
                };
                self.sel_anchor = Some((block_id, start));
                self.sel_cursor = Some((block_id, end));
                self.selecting = true;
                self.touch_selection = Some(crate::reader_touch::TouchSelection::new(
                    event.uid,
                    event.abs,
                    ((block_id, start), (block_id, end)),
                ));
                self.last_abs = Some(event.abs);
                *SEL_RANGE.write().unwrap() = Some(((block_id, start), (block_id, end)));
                SELECTED_TEXT.write().unwrap().clear();
                Cx::post_action(SelAction::Hide);
                self.apply_visible_selection(cx);
            }
            Event::TouchUpdate(event) if self.touch_selection.is_some() => {
                let uid = self.touch_selection.as_ref().unwrap().uid;
                let Some(touch) = event.touches.iter().find(|touch| touch.uid == uid) else {
                    return;
                };
                match touch.state {
                    TouchState::Move | TouchState::Stop => {
                        if let Some(hit) = self.hit_block_char(cx, touch.abs) {
                            let range = self
                                .touch_selection
                                .as_mut()
                                .unwrap()
                                .range_at(touch.abs, hit);
                            self.sel_anchor = Some(range.0);
                            self.sel_cursor = Some(range.1);
                            *SEL_RANGE.write().unwrap() = Some(range);
                            self.last_abs = Some(touch.abs);
                            self.apply_visible_selection(cx);
                        }
                        if touch.state == TouchState::Stop {
                            self.finish_selection();
                        }
                    }
                    TouchState::Start | TouchState::Stable => {}
                }
            }
            _ => {}
        }
    }
}

impl ChapterList {
    fn draw_reader_paragraph(
        &mut self,
        cx: &mut Cx2d,
        list: &mut PortalList,
        item_id: usize,
        cache_id: usize,
        para: &str,
        code_text: Option<&str>,
        measure: bool,
    ) -> crate::reader_columns::BlockMetrics {
        let reader_font = *READER_FONT.read().unwrap();
        let bold_text = *READER_BOLD_TEXT.read().unwrap();
        let reader_spacing = *READER_SPACING.read().unwrap();
        let reader_background = *READER_BACKGROUND.read().unwrap();
        let reader_theme = *READER_THEME.read().unwrap();
        let (_, reader_text_color) = reader_palette(reader_background, reader_theme);
        let reader_scene = reader_scene_palette(reader_background, reader_theme);
        let code_template = match reader_background {
            ReaderBackground::Original => id!(CodeThemeLight),
            ReaderBackground::Paper => id!(CodePaper),
            ReaderBackground::Sepia => id!(CodeSepia),
            ReaderBackground::Sage => id!(CodeSage),
            ReaderBackground::Bold => id!(CodePaper),
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
        let selection_color =
            crate::settings_swatch::selection_color(*SELECTION_SLOT.read().unwrap());
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
        let normal_family = if reader_font != ReaderFont::Original {
            if bold_text {
                self.font_active_bold.font_family.clone()
            } else {
                self.font_active_normal.font_family.clone()
            }
        } else if bold_text {
            self.font_standard_bold.font_family.clone()
        } else {
            self.font_standard_normal.font_family.clone()
        };
        let italic_family = if reader_font != ReaderFont::Original {
            if bold_text {
                self.font_active_bold_italic.font_family.clone()
            } else {
                self.font_active_italic.font_family.clone()
            }
        } else if bold_text {
            self.font_standard_bold_italic.font_family.clone()
        } else {
            self.font_standard_italic.font_family.clone()
        };
        let bold_family = if reader_font != ReaderFont::Original {
            self.font_active_bold.font_family.clone()
        } else {
            self.font_standard_bold.font_family.clone()
        };
        let bold_italic_family = if reader_font != ReaderFont::Original {
            self.font_active_bold_italic.font_family.clone()
        } else {
            self.font_standard_bold_italic.font_family.clone()
        };
        // A live drag creates a new end offset on nearly every pointer event.
        // The cache is self-healing, so cap it instead of retaining thousands
        // of obsolete transient ranges for the current chapter.
        if self.range_geometry_cache.len() > 1024 {
            self.range_geometry_cache.clear();
        }
        let mut needs_geometry_redraw = false;

        // Adapt only the display HTML: list/block IDs and stored
        // selection/highlight offsets still use the original para.
        let with_inline_code = code_text.is_none().then(|| {
            crate::reader_inline_code::wrap_html(&crate::reader_lists::normalize_html(para))
        });
        let tracked = with_inline_code
            .as_deref()
            .and_then(crate::reader_hit::tracked_html);
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
        let line_spacing = reader_spacing.plain_line_spacing();
        let item = list.item(cx, cache_id, template);
        let previous_layout_width = self
            .item_layout_widths
            .get(&(chapter_idx, item_id))
            .copied();
        let source_text = readwise_core::vocab::annotate::rendered_text(para);
        let source_len = source_text.len();
        let current_range =
            selection_range.and_then(|range| selection_range_in_block(range, item_id, source_len));
        let highlight_ranges: Vec<_> = if code_text.is_none() {
            chapter_highlights
                .iter()
                .filter_map(|highlight| {
                    let range = highlight_range_in_block(highlight, item_id, source_len)?;
                    Some((
                        range,
                        crate::mark_span::hl_color(crate::mark_span::hl_slot_for_name(
                            &highlight.color,
                        )),
                    ))
                })
                .collect()
        } else {
            Vec::new()
        };
        let make_pre_draw_key = |range| {
            reader_range_geometry_key(
                chapter_idx,
                item_id,
                range,
                previous_layout_width.unwrap_or(0.0),
                reader_font_size,
                line_spacing,
                reader_font,
                para,
            )
        };
        let probe_range = if measure {
            Some((0, source_len))
        } else if code_text.is_none() {
            current_range.or_else(|| {
                highlight_ranges
                    .iter()
                    .find(|(range, _)| {
                        !self
                            .range_geometry_cache
                            .contains_key(&make_pre_draw_key(*range))
                    })
                    .map(|(range, _)| *range)
            })
        } else {
            current_range
        };
        let mut code_cache_changed = false;
        if let Some(code_text) = code_text {
            let code_view = item.widget(cx, ids!(code_view));
            if code_view.text() != code_text {
                code_view.set_text(cx, code_text);
                // Paged reading reuses PortalList slots. A code container's
                // DrawList can still hold the previous block at this slot.
                // Invalidate it for the next frame, then draw without that
                // cache now so the first frame's measured height is correct.
                cx.redraw_area_in_draw(item.area());
                if let Some(mut view) = item.borrow_mut::<View>() {
                    view.set_optimize(cx, ViewOptimize::None);
                    code_cache_changed = true;
                }
            }
        } else {
            let html = item.widget(cx, ids!(para_html));
            let display_html = tracked
                .as_deref()
                .or(with_inline_code.as_deref())
                .unwrap_or(para);
            if let Some(mut inner) = html.borrow_mut::<Html>() {
                if inner.body.as_ref() != display_html {
                    // Html assigns anonymous custom tags (`mark`, `ruby`,
                    // links) positional IDs. Adding/removing a mark can
                    // therefore move a RubyWord onto an ID previously owned
                    // by MarkSpan. TextFlow::item_with_scope does not replace
                    // an existing item when its template changes, so retaining
                    // the map would draw the gloss as ordinary inline content.
                    // Rebuild only when the document actually changes; steady
                    // frames keep their widgets and area references.
                    inner.text_flow.clear_items();
                    inner.set_text(cx, display_html);
                }
                // Required on every platform: it builds the native
                // text buffer, hit tracker and selection geometry.
                // Touch input ownership is handled separately below.
                inner.text_flow.selectable = true;
                inner.text_flow.font_size = reader_font_size;
                inner.text_flow.font_color = reader_text_color;
                inner.text_flow.draw_text.color = reader_text_color;
                // TextFlow still computes the authoritative rich-text
                // rectangles, but submits them transparently. ChapterList
                // captures and normalizes that same batch for both the live
                // selection and permanent highlights.
                inner.text_flow.draw_selection.color = Vec4f::default();
                inner.text_flow.draw_block.table_header_bg_color = reader_scene.field;
                inner.text_flow.draw_block.table_border_color = reader_scene.border;
                inner.text_flow.text_style_normal.font_family = normal_family.clone();
                inner.text_flow.text_style_italic.font_family = italic_family.clone();
                inner.text_flow.text_style_bold.font_family = bold_family.clone();
                inner.text_flow.text_style_bold_italic.font_family = bold_italic_family.clone();
                inner.text_flow.text_style_normal.line_spacing = line_spacing;
                inner.text_flow.text_style_italic.line_spacing = line_spacing;
                inner.text_flow.text_style_bold.line_spacing = line_spacing;
                inner.text_flow.text_style_bold_italic.line_spacing = line_spacing;
                inner.text_flow.text_style_fixed.line_spacing = line_spacing;
                let letter_spacing = *READER_CHARACTER_SPACING.read().unwrap();
                let word_spacing = *READER_WORD_SPACING.read().unwrap();
                let justify = *READER_JUSTIFY_TEXT.read().unwrap();
                let flow = &mut inner.text_flow;
                for style in [
                    &mut flow.text_style_normal,
                    &mut flow.text_style_italic,
                    &mut flow.text_style_bold,
                    &mut flow.text_style_bold_italic,
                ] {
                    style.letter_spacing = letter_spacing;
                    style.word_spacing = word_spacing;
                    style.justify = justify;
                }
            };
        }
        let native_probe = if measure {
            Some((0, usize::MAX))
        } else {
            probe_range.and_then(|range| {
                if code_text.is_some() {
                    Some(range)
                } else {
                    self.text_map(item_id, para)?.native_range(range)
                }
            })
        };
        if let Some((start, end)) = native_probe {
            item.selection_set(start, end);
        } else {
            item.selection_clear();
        }
        let capture_geometry = code_text.is_none() && native_probe.is_some();
        if capture_geometry {
            let html = item.widget(cx, ids!(para_html));
            if let Some(mut inner) = html.borrow_mut::<Html>() {
                // Seed the batch in the current draw-call group so every
                // selection rectangle emitted by TextFlow lands in one
                // inspectable instance buffer.
                inner.text_flow.draw_selection.new_draw_call(cx);
                inner.text_flow.draw_selection.begin_many_instances(cx);
            };
        }
        crate::reader_hit::begin_capture(measure);
        crate::reader_inline_code::begin_capture(native_probe);
        let page_height = if self.pager.active {
            self.pager.layout.height
        } else {
            0.0
        };
        crate::chapter_image::with_page_height(page_height, || item.draw_all_unscoped(cx));
        if code_cache_changed {
            if let Some(mut view) = item.borrow_mut::<View>() {
                view.set_optimize(cx, ViewOptimize::DrawList);
            }
        }
        let item_rect = item.area().rect(cx);
        let item_margin = item.walk(cx).margin;
        let (runs, overlays) = crate::reader_hit::end_capture(cx, item_rect.pos);
        let mut painted_bottom = 0.0_f64;
        let painted_cuts = if measure && !runs.is_empty() {
            let mut painted_rows = crate::reader_hit::painted_rows(&runs);
            painted_rows.extend(overlays);
            painted_bottom = painted_rows
                .iter()
                .map(|(rect, _)| rect.pos.y + rect.size.y)
                .fold(0.0_f64, f64::max);
            reader_painted_page_cuts(&painted_rows)
        } else {
            Vec::new()
        };
        let mut mapping_stable = true;
        if code_text.is_none() {
            let html = item.widget(cx, ids!(para_html));
            let native = html
                .borrow::<Html>()
                .and_then(|inner| crate::reader_text::native_text(&inner.text_flow));
            mapping_stable = native.as_ref().is_some_and(|native| {
                self.text_map(item_id, para)
                    .is_some_and(|map| map.matches_native(native))
            });
            if !mapping_stable {
                self.text_maps.remove(&item_id);
                if let Some(map) = native.and_then(|native| {
                    crate::reader_text::TextMap::from_html(&source_text, para, native)
                }) {
                    self.text_maps
                        .insert(item_id, (reader_source_hash(para), map));
                    // Fresh/recycled HTML needs one draw to discover
                    // all actual block breaks before probing a range.
                    needs_geometry_redraw |= probe_range.is_some();
                }
                self.range_geometry_cache
                    .retain(|key, _| key.chapter != chapter_idx || key.block != item_id);
            }
        }
        let painted_selection = if tracked.is_some() {
            native_probe.and_then(|(start, end)| {
                let rects = crate::reader_hit::selection_rects(&runs, start, end);
                (!rects.is_empty()).then_some(rects)
            })
        } else {
            None
        };
        let used_painted_selection = painted_selection.is_some();
        if tracked.is_some() {
            self.hit_runs
                .insert(item.widget_uid(), (item_id, reader_source_hash(para), runs));
        } else {
            self.hit_runs.remove(&item.widget_uid());
        }
        let width_stable = previous_layout_width
            .map(|previous| reader_geometry_width_is_stable(previous, item_rect.size.x))
            .unwrap_or(false);
        self.item_layout_widths
            .insert((chapter_idx, item_id), item_rect.size.x);
        let make_geometry_key = |range| {
            reader_range_geometry_key(
                chapter_idx,
                item_id,
                range,
                item_rect.size.x,
                reader_font_size,
                line_spacing,
                reader_font,
                para,
            )
        };
        let current_key = current_range.map(make_geometry_key);
        let highlight_entries: Vec<_> = highlight_ranges
            .iter()
            .map(|(range, color)| (*range, make_geometry_key(*range), *color))
            .collect();
        let mut captured_current_geometry = None;
        let mut page_lines = Vec::new();
        if capture_geometry {
            let html = item.widget(cx, ids!(para_html));
            if let Some(mut inner) = html.borrow_mut::<Html>() {
                let mut raw = painted_selection
                    .map(|rects| {
                        rects
                            .into_iter()
                            .map(|mut rect| {
                                rect.pos += item_rect.pos;
                                rect
                            })
                            .collect()
                    })
                    .unwrap_or_else(|| {
                        captured_draw_color_rects(cx, &inner.text_flow.draw_selection)
                    });
                if !used_painted_selection && !reader_plain_paragraph(para) {
                    crate::reader_inline_code::correct_rects(cx, &mut raw);
                }
                inner.text_flow.draw_selection.end_many_instances(cx);
                if measure {
                    painted_bottom = painted_bottom.max(
                        raw.iter()
                            .map(|rect| rect.pos.y + rect.size.y - item_rect.pos.y - 2.0)
                            .fold(0.0_f64, f64::max),
                    );
                }
                let normalized = normalize_rich_selection_rects(&raw);
                if measure {
                    page_lines = reader_pagination_rows(&raw);
                }
                let (draw_current_now, cache_for_reuse) = reader_geometry_capture_policy(
                    probe_range,
                    current_range,
                    width_stable && mapping_stable,
                );
                if draw_current_now && mapping_stable {
                    // Live selection is transient UI, not persistent cache.
                    // Paint this frame immediately even while a resized item
                    // is waiting for its next stable-width frame.
                    captured_current_geometry = Some(normalized.clone());
                }
                if cache_for_reuse {
                    if let Some(key) = probe_range.map(make_geometry_key) {
                        self.range_geometry_cache
                            .insert(key, rects_relative_to(&normalized, item_rect.pos));
                    }
                } else if mapping_stable || self.text_map(item_id, para).is_some() {
                    // The PortalList item was still transitioning from the
                    // old window width. Never poison the new-width cache with
                    // this frame; request one stable redraw instead.
                    needs_geometry_redraw = true;
                }
            };
        }

        if !measure {
            // Permanent highlights and the current selection are drawn from
            // exactly the same captured rich-text geometry. Translate cached
            // item-relative rectangles so PortalList scrolling cannot stale it.
            for (_, key, color) in &highlight_entries {
                if let Some(rects) = self.range_geometry_cache.get(key).cloned() {
                    self.draw_reader_selection.color = *color;
                    for rect in rects {
                        self.draw_reader_selection.draw_abs(
                            cx,
                            Rect {
                                pos: rect.pos + item_rect.pos,
                                size: rect.size,
                            },
                        );
                    }
                } else if self.text_map(item_id, para).is_some() {
                    needs_geometry_redraw = true;
                }
            }
            if let Some(key) = current_key {
                if let Some(rects) = captured_current_geometry {
                    self.draw_reader_selection.color = selection_color;
                    for rect in rects {
                        self.draw_reader_selection.draw_abs(cx, rect);
                    }
                } else if let Some(rects) = self.range_geometry_cache.get(&key).cloned() {
                    self.draw_reader_selection.color = selection_color;
                    for rect in rects {
                        self.draw_reader_selection.draw_abs(
                            cx,
                            Rect {
                                pos: rect.pos + item_rect.pos,
                                size: rect.size,
                            },
                        );
                    }
                }
            }
        }

        let mut boundaries = Vec::new();
        if measure && code_text.is_some() {
            let code = item.widget(cx, ids!(code_view));
            if let Some(view) = code.borrow::<makepad_code_editor::code_view::CodeView>() {
                if let Some(session) = &view.session {
                    let native = session.layout();
                    let rect = view.editor.area().rect(cx);
                    let scale = rect.size.y / native.height().max(1.0);
                    let mut source = 0;
                    for (index, line) in native
                        .lines(0, native.as_text().as_lines().len())
                        .enumerate()
                    {
                        if index > 0 {
                            boundaries.push(crate::reader_columns::LineBoundary {
                                y: rect.pos.y - item_rect.pos.y
                                    + item_margin.top
                                    + line.y() * scale,
                                source,
                            });
                        }
                        source += line.text().len() + 1;
                    }
                }
            };
        }
        if !is_table && code_text.is_none() {
            let native_len = item.selection_text_len();
            if !painted_cuts.is_empty() {
                for (cut_y, native) in painted_cuts {
                    let source = self.text_map(item_id, para).map_or_else(
                        || reader_fallback_source_index(native, native_len, source_len),
                        |map| map.source_index(native),
                    );
                    boundaries.push(crate::reader_columns::LineBoundary {
                        y: (cut_y + item_margin.top).max(0.0),
                        source,
                    });
                }
            } else {
                for (cut_y, point) in page_lines {
                    if let Some(native) = item.selection_point_to_char_index(cx, point) {
                        let source = self.text_map(item_id, para).map_or_else(
                            || reader_fallback_source_index(native, native_len, source_len),
                            |map| map.source_index(native),
                        );
                        boundaries.push(crate::reader_columns::LineBoundary {
                            y: (cut_y - item_rect.pos.y + item_margin.top).max(0.0),
                            source,
                        });
                    }
                }
            }
        }
        if needs_geometry_redraw && !measure {
            self.view.area().redraw(cx);
        }
        boundaries.sort_by(|a, b| a.y.total_cmp(&b.y));
        boundaries.dedup_by(|a, b| (a.y - b.y).abs() < 0.5);
        crate::reader_columns::BlockMetrics {
            height: reader_block_visual_height(item_rect.size.y, painted_bottom)
                + item_margin.top
                + item_margin.bottom,
            boundaries,
            keep_together: is_table || code_text.is_some() || para.contains("<img"),
        }
    }
}

impl Widget for ChapterList {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let viewport = cx.peek_walk_turtle(walk);
        let mode = *READER_TURN_MODE.read().unwrap();
        if self.turn_mode != mode {
            self.page_turn.cancel();
            self.pager.pending_turn = None;
            if self.turn_mode.is_paged() != mode.is_paged() {
                let list = self.view.portal_list(cx, ids!(list));
                self.pager.requested_source =
                    self.source_anchor(list.first_id(), list.scroll_position());
            }
            self.turn_mode = mode;
        }
        if !mode.is_paged() {
            return self.draw_scrolling(cx, scope, walk, viewport);
        }
        if viewport.size.x > 0.0 && viewport.size.y > 0.0 && viewport.size.y.is_finite() {
            self.draw_page_turn.slide = if mode == ReaderTurnMode::Slide {
                1.0
            } else {
                0.0
            };
            let paper = current_reader_image_palette().0;
            let appearance = reader_source_hash(&format!(
                "{viewport:?}:{}:{paper:?}:{:?}:{:?}:{:?}:{:?}:{}:{}",
                cx.current_dpi_factor(),
                *READER_COLUMNS.read().unwrap(),
                *READER_FONT.read().unwrap(),
                *READER_FONT_SIZE.read().unwrap(),
                *READER_SPACING.read().unwrap(),
                *crate::ruby_word::GLOSS_FONT_SIZE.read().unwrap(),
                *GLOSS_ENABLED.read().unwrap()
            ));
            if self.page_turn.can_draw_cached(appearance) {
                // Native paragraph areas stay valid in their retained pass.
                // Only the empty host and the snapshot quad need drawing.
                let _ = self.view.draw_walk(cx, scope, walk);
                self.page_turn
                    .draw_cached(cx, &mut self.draw_page_turn, viewport, paper, true);
                if let Some(forward) = self.pager.take_pending_turn(self.page_turn.busy()) {
                    Cx::post_action(ReaderPageTurn(forward));
                }
                return DrawStep::done();
            }
            let snapshot = self.page_turn.begin(cx, viewport, appearance, paper);
            let result = self.draw_pages(cx, scope, walk, viewport);
            if snapshot {
                let ready = self.pager.page_ready();
                self.page_turn
                    .end(cx, &mut self.draw_page_turn, viewport, paper, ready);
            }
            if let Some(forward) = self.pager.take_pending_turn(self.page_turn.busy()) {
                Cx::post_action(ReaderPageTurn(forward));
            }
            return result;
        }
        self.page_turn.cancel();
        // Hidden/zero-size reader surfaces must not change the active layout
        // or overwrite its reading anchor with a fallback rendering pass.
        DrawStep::done()
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        if self.reader_tap_timer.is_event(event).is_some() {
            self.reader_tap_timer = Timer::empty();
            let context = (
                READER_CONTENT_VERSION.load(Ordering::Relaxed),
                CURRENT_CHAPTER.load(Ordering::Relaxed),
            );
            if self.pending_tap_context.take() == Some(context)
                && ACTIVE_PAGE.load(Ordering::Relaxed) == AppPage::Reader.persisted()
            {
                cx.extend_actions(std::mem::take(&mut self.pending_reader_tap));
            } else {
                self.pending_reader_tap.clear();
            }
            return;
        }
        if matches!(
            event,
            Event::Scroll(_)
                | Event::LiveEdit
                | Event::ScriptReapply
                | Event::WindowLostFocus(_)
                | Event::MouseLeave(_)
        ) {
            self.cancel_reader_tap(cx);
        }
        if reader_colour_panel_blocks_event(
            READER_APPEARANCE_OPEN.load(Ordering::Acquire),
            READER_CUSTOMISE_OPEN.load(Ordering::Acquire),
            event,
        ) {
            self.cancel_reader_tap(cx);
            return;
        }
        self.page_turn.handle_event(cx, event, self.view.area());
        if matches!(event, Event::WindowGeomChange(_)) {
            self.cancel_reader_tap(cx);
            self.page_turn.cancel();
            self.pager.pending_turn = None;
        }
        if self.pager.measure_frame.is_event(event).is_some() {
            self.view.redraw(cx);
        }
        // While pixels are curled they no longer coincide with text hit areas.
        // A completed turn restores the unmodified native selection path.
        if self.page_turn.busy()
            && matches!(
                event,
                Event::MouseDown(_)
                    | Event::MouseMove(_)
                    | Event::MouseUp(_)
                    | Event::TouchUpdate(_)
                    | Event::LongPress(_)
            )
        {
            self.cancel_reader_tap(cx);
            return;
        }
        let reader_info_open = READER_INFO_POPUP_OPEN.load(Ordering::Relaxed);
        if ((reader_info_open || SELECTION_POPUP_POINTER.load(Ordering::Relaxed))
            && matches!(
                event,
                Event::MouseDown(_)
                    | Event::MouseMove(_)
                    | Event::MouseUp(_)
                    | Event::TouchUpdate(_)
                    | Event::LongPress(_)
            ))
            || (reader_info_open && matches!(event, Event::Scroll(_)))
        {
            self.cancel_reader_tap(cx);
            return;
        }
        let mut tap_outcome = self.word_tap.track(event, self.reader_rect(cx));
        if self.word_tap.second_press() {
            // Hold the second press locally; never open the first tap's card.
            self.cancel_pending_reader_tap(cx);
        }
        if tap_outcome == TapOutcome::Double {
            self.toolbar_tap.cancel();
            self.cancel_pending_reader_tap(cx);
            self.handle_child_event(cx, event, scope, true);
            let abs = match event {
                Event::MouseUp(e) => e.abs,
                Event::TouchUpdate(e) => e.touches[0].abs,
                _ => unreachable!(),
            };
            self.select_pointer_word(cx, abs);
            return;
        }
        if !self.pager.active && !self.selecting {
            if let Event::Scroll(e) = event {
                let turn = self.pager.scroll_gesture.on_scroll(e);
                if self.reader_rect(cx).contains(e.abs)
                    && e.scroll.y.abs() > e.scroll.x.abs()
                    && !e.handled_y.get()
                {
                    let list = self.view.portal_list(cx, ids!(list));
                    let down = e.scroll.y > 0.0;
                    let boundary = if down {
                        list.is_at_end()
                    } else {
                        list.first_id() == 0 && list.scroll_position() >= -0.5
                    };
                    if boundary && turn.is_some() {
                        e.handled_y.set(true);
                        Cx::post_action(ReaderPageTurn(down));
                        return;
                    }
                }
            }
        }
        if self.pager.active {
            if let Event::Scroll(e) = event {
                let turn = self.pager.scroll_gesture.on_scroll(e);
                // Code and tables retain horizontal scrolling inside a page.
                if !self.page_turn.busy() && e.scroll.x.abs() > e.scroll.y.abs() {
                    for (_, item, clip) in self.active_items(cx) {
                        if clip.contains(e.abs) {
                            item.handle_event(cx, event, scope);
                        }
                    }
                    if e.handled_x.get() {
                        self.pager.scroll_gesture.consume();
                        return;
                    }
                }
                if self.reader_rect(cx).contains(e.abs) && !e.handled_y.get() {
                    e.handled_y.set(true);
                    if let Some(forward) = turn {
                        Cx::post_action(ReaderPageTurn(forward));
                    }
                    return;
                }
                self.pager.scroll_gesture.consume();
            }
        }
        if matches!(event, Event::LongPress(_)) {
            self.cancel_reader_tap(cx);
            self.track_selection(cx, event);
            self.handle_child_event(cx, event, scope, self.touch_selection.is_some());
            return;
        }
        if self.touch_selection.is_some() && matches!(event, Event::TouchUpdate(_)) {
            self.cancel_reader_tap(cx);
            let released = matches!(event, Event::TouchUpdate(event) if event.touches.iter().any(|touch| {
                self.touch_selection.as_ref().is_some_and(|selection| selection.uid == touch.uid)
                    && touch.state == TouchState::Stop
            }));
            self.track_selection(cx, event);
            // Let PortalList observe the final release so its pre-long-press drag
            // state cannot leak into the next gesture. Selection moves stay local.
            if released {
                self.handle_child_event(cx, event, scope, true);
            }
            return;
        }
        let mut toolbar_tap = self.track_toolbar_tap(cx, event);
        if let Event::TouchUpdate(e) = event {
            let rect = self.reader_rect(cx);
            if e.touches
                .iter()
                .any(|touch| touch.state == TouchState::Start && rect.contains(touch.abs))
            {
                self.reset_selection_gesture();
                *SEL_RANGE.write().unwrap() = None;
                SELECTED_TEXT.write().unwrap().clear();
                self.apply_visible_selection(cx);
                Cx::post_action(SelAction::Hide);
                cx.hide_clipboard_actions();
            }
        }
        let actions = cx.capture_actions(|cx| self.handle_child_event(cx, event, scope, false));
        let list = self.view.portal_list(cx, ids!(list));
        if !self.pager.active && (list.was_scrolling() || list.scrolled(&actions)) {
            // A finger that stops inertia is not the first half of a double tap.
            self.cancel_reader_tap(cx);
            tap_outcome = TapOutcome::None;
            toolbar_tap = false;
        }
        let inline_activated = actions.iter().any(reader_tap_opens_inline);
        let mut delayed = Vec::new();
        let mut immediate = Vec::new();
        for action in actions {
            if tap_outcome == TapOutcome::Single && reader_tap_selectable_inline(&action) {
                delayed.push(action);
            } else {
                immediate.push(action);
            }
        }
        cx.extend_actions(immediate);
        if toolbar_tap && !inline_activated {
            delayed.push(Box::new(ReaderToolbarTap));
        }
        self.defer_reader_tap(cx, delayed);
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
    #[rust]
    activation_tap: ListTap,
}

impl Widget for TocList {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let requested_id = CURRENT_ITEM
            .read()
            .unwrap()
            .as_ref()
            .map(|item| item.id.clone());
        let guard = CURRENT_BOOK.read().unwrap();
        let ready_book = guard
            .as_ref()
            .filter(|book| requested_id.as_deref() == Some(book.id.as_str()));
        let toc: &[readwise_core::books::epub_meta::TocEntry] =
            ready_book.map(|b| b.toc.as_slice()).unwrap_or(&[]);
        let current = CURRENT_CHAPTER.load(std::sync::atomic::Ordering::Relaxed);
        let current_block = CURRENT_READER_BLOCK.load(Ordering::Relaxed);
        let current_chunks = ready_book
            .and_then(|book| book.chapters.get(current))
            .map(|chapter| readwise_core::books::epub_meta::split_blocks(chapter));
        let active = current_chunks.as_deref().and_then(|chunks| {
            toc.iter()
                .enumerate()
                .filter(|(_, entry)| entry.chapter == current)
                .filter_map(|(index, entry)| {
                    let block = entry
                        .fragment
                        .as_deref()
                        .map_or(0, |fragment| reader_anchor_block(chunks, fragment));
                    (block <= current_block).then_some((index, block, entry.level))
                })
                .max_by_key(|(_, block, level)| (*block, *level))
                .map(|(index, _, _)| index)
        });

        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_shelf_list().borrow_mut() {
                list.set_item_range(cx, 0, toc.len());
                while let Some(item_id) = list.next_visible_item(cx) {
                    let Some(entry) = toc.get(item_id) else {
                        continue;
                    };
                    let item = list.item(cx, item_id, id!(TocItem));
                    let label = item.label(cx, ids!(toc_item_label));
                    if label.text() != entry.title {
                        label.set_text(cx, &entry.title);
                    }
                    if let Some(mut inner) = label.borrow_mut() {
                        inner.draw_text.color = self.text_color;
                        inner.walk.margin.left = entry.level as f64 * 18.0;
                    }
                    if let Some(mut inner) = item.borrow_mut::<View>() {
                        let color = if Some(item_id) == active {
                            self.active_bg
                        } else {
                            self.inactive_bg
                        };
                        inner.draw_bg.draw_vars.set_dyn_instance(
                            cx,
                            id!(color),
                            &[color.x, color.y, color.z, color.w],
                        );
                    }
                    item.draw_all_unscoped(cx);
                }
            }
        }
        DrawStep::done()
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        let list = self.view.shelf_list(cx, ids!(list));
        let rect = current_clipped_rect(cx, list.area());
        let mut allow_click = self.activation_tap.allows_click(event, rect);
        let mut actions = cx.capture_actions(|cx| self.view.handle_event(cx, event, scope));
        // A touch that stops an existing flick is not a chapter selection.
        if list.scrolled(&actions) || list.was_scrolling() {
            self.activation_tap.cancel();
            allow_click = false;
        }
        if !allow_click {
            actions.retain(|action| {
                !action
                    .as_widget_action()
                    .is_some_and(|action| matches!(action.cast(), ButtonAction::Clicked(_)))
            });
        }
        cx.extend_actions(actions);
    }
}

#[derive(Script, ScriptHook, Widget)]
struct ReaderJumpList {
    #[deref]
    view: View,
    #[live]
    bookmarks: bool,
    #[live]
    highlights: bool,
    #[live]
    text_color: Vec4f,
}

impl Widget for ReaderJumpList {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let labels: Vec<String> = if self.bookmarks {
            READER_BOOKMARKS
                .read()
                .unwrap()
                .iter()
                .map(|bookmark| bookmark.label.clone())
                .collect()
        } else if self.highlights {
            READER_HIGHLIGHT_JUMPS
                .read()
                .unwrap()
                .iter()
                .map(|hit| format!("{} · {}", hit.chapter + 1, hit.excerpt))
                .collect()
        } else {
            READER_SEARCH_RESULTS
                .read()
                .unwrap()
                .iter()
                .map(|hit| format!("{} · {}", hit.chapter + 1, hit.excerpt))
                .collect()
        };
        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_portal_list().borrow_mut() {
                list.set_item_range(cx, 0, labels.len());
                while let Some(item_id) = list.next_visible_item(cx) {
                    let Some(text) = labels.get(item_id) else {
                        continue;
                    };
                    let item = list.item(cx, item_id, id!(JumpItem));
                    let label = item.label(cx, ids!(jump_label));
                    if label.text() != *text {
                        label.set_text(cx, text);
                    }
                    if let Some(mut inner) = label.borrow_mut() {
                        inner.draw_text.color = self.text_color;
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
        let note_input_palette = ReaderScenePalette {
            field: palette.surface,
            ..palette
        };
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
                    let (quote, body) = split_note_content(&note.content);
                    item.label(cx, ids!(note_quote_title))
                        .set_text(cx, crate::i18n::text(language, "quote"));
                    item.label(cx, ids!(note_body_title))
                        .set_text(cx, crate::i18n::text(language, "note"));
                    let binding = format!("{}:{}", projection.revision, note.id);
                    let bound_id = item.label(cx, ids!(note_bound_id));
                    let body_input = item.text_input(cx, ids!(note_body_input));
                    let placeholder = crate::i18n::text(language, "note_placeholder");
                    if body_input.empty_text() != placeholder {
                        body_input.set_empty_text(cx, placeholder.to_string());
                    }
                    if bound_id.text() != binding {
                        item.text_input(cx, ids!(note_quote)).set_text(cx, quote);
                        body_input.set_text(cx, body);
                        bound_id.set_text(cx, &binding);
                    }
                    item.widget(cx, ids!(note_delete_btn))
                        .set_text(cx, crate::i18n::text(language, "delete"));
                    item.widget(cx, ids!(note_save_btn))
                        .set_text(cx, crate::i18n::text(language, "save"));
                    let note_quote_title = item.widget(cx, ids!(note_quote_title));
                    let note_quote = item.widget(cx, ids!(note_quote));
                    let note_body_title = item.widget(cx, ids!(note_body_title));
                    let note_body_input = item.widget(cx, ids!(note_body_input));
                    let note_save = item.widget(cx, ids!(note_save_btn));
                    let note_delete = item.widget(cx, ids!(note_delete_btn));
                    apply_reader_surface_palette(cx, item.clone(), palette.surface, palette.border);
                    apply_reader_label_palette(cx, note_quote_title, palette.accent);
                    apply_reader_input_palette(cx, note_quote, note_input_palette);
                    apply_reader_label_palette(cx, note_body_title, palette.accent);
                    apply_reader_input_palette(cx, note_body_input, note_input_palette);
                    apply_reader_button_palette(cx, note_save, palette);
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
struct AllNotesList {
    #[deref]
    view: View,
}
impl Widget for AllNotesList {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let notes = ALL_NOTES.read().unwrap();
        let palette = reader_scene_palette(
            *READER_BACKGROUND.read().unwrap(),
            *READER_THEME.read().unwrap(),
        );
        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_portal_list().borrow_mut() {
                list.set_item_range(cx, 0, notes.len());
                while let Some(index) = list.next_visible_item(cx) {
                    let Some(note) = notes.get(index) else {
                        continue;
                    };
                    let item = list.item(cx, index, id!(Note));
                    let (quote, body) = split_note_content(&note.content);
                    item.label(cx, ids!(source))
                        .set_text(cx, note.book_title.as_deref().unwrap_or(tr("note")));
                    item.label(cx, ids!(quote)).set_text(cx, quote);
                    item.label(cx, ids!(body)).set_text(cx, body);
                    item.widget(cx, ids!(body))
                        .set_visible(cx, !body.is_empty());
                    apply_reader_surface_palette(cx, item.clone(), palette.field, palette.border);
                    for (id, color) in [
                        (id!(source), palette.accent),
                        (id!(quote), palette.text),
                        (id!(body), palette.muted),
                    ] {
                        let label = item.widget(cx, &[id]);
                        apply_reader_label_palette(cx, label, color);
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
    #[live(false)]
    reader_style: bool,
    #[rust]
    last_revision: u64,
    #[rust]
    last_streaming_revision: u64,
    #[rust]
    was_streaming: bool,
    #[rust]
    last_style: Option<(AppLanguage, ReaderBackground, ReaderTheme)>,
    #[rust]
    last_width: Option<f64>,
    #[rust]
    alternate_template: bool,
}

impl Widget for ChatList {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let width = cx.peek_walk_turtle(walk).size.x;
        let width_changed = self.last_width.is_none_or(|old| (old - width).abs() > 0.5);
        // Reading AI is also a narrow desktop sidebar. Reserve a small,
        // proportional gutter instead of the main chat's fixed 90px margin.
        let reader_message_inset = ((width - 8.0).max(0.0) * 0.06).min(24.0);
        let data = CHAT_DATA.read().unwrap();
        let language = *APP_LANGUAGE.read().unwrap();
        let background = *READER_BACKGROUND.read().unwrap();
        let theme = *READER_THEME.read().unwrap();
        let style = (language, background, theme);
        let style_changed = self.last_style != Some(style);
        let thread_changed = self.last_revision != data.revision;
        let streaming_changed = self.was_streaming != data.is_streaming
            || self.last_streaming_revision != data.streaming_revision;
        let palette = reader_scene_palette(background, theme);
        if thread_changed || style_changed || width_changed {
            // A script reload may reset a cached row's measured height to a
            // single line. Recreate only the visible rows when the whole
            // conversation, layout style or available width changes.
            self.alternate_template = !self.alternate_template;
        }
        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_portal_list().borrow_mut() {
                let message_count = data.messages.len();
                list.set_item_range(cx, 0, message_count + usize::from(data.is_streaming));
                while let Some(item_id) = list.next_visible_item(cx) {
                    if data.is_streaming && item_id == message_count {
                        let template = if self.alternate_template {
                            id!(AssistantAlternate)
                        } else {
                            id!(Assistant)
                        };
                        let (item, existed) = list.item_with_existed(cx, item_id, template);
                        if !existed
                            || thread_changed
                            || style_changed
                            || width_changed
                            || streaming_changed
                        {
                            if self.reader_style {
                                if let Some(mut row) = item.borrow_mut::<View>() {
                                    row.walk.margin = Inset {
                                        right: reader_message_inset,
                                        ..Inset::default()
                                    };
                                }
                            }
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
                        ChatRole::User if self.alternate_template => id!(UserAlternate),
                        ChatRole::User => id!(User),
                        ChatRole::Assistant if self.alternate_template => id!(AssistantAlternate),
                        ChatRole::Assistant => id!(Assistant),
                    };
                    let (item, existed) = list.item_with_existed(cx, item_id, template);
                    let finished_stream_row =
                        self.was_streaming && !data.is_streaming && item_id + 1 == message_count;
                    if !existed
                        || thread_changed
                        || style_changed
                        || width_changed
                        || finished_stream_row
                    {
                        if self.reader_style {
                            if let Some(mut row) = item.borrow_mut::<View>() {
                                row.walk.margin = match message.role {
                                    ChatRole::User => Inset {
                                        left: reader_message_inset,
                                        ..Inset::default()
                                    },
                                    ChatRole::Assistant => Inset {
                                        right: reader_message_inset,
                                        ..Inset::default()
                                    },
                                };
                            }
                        }
                        item.label(cx, ids!(message)).set_text(cx, &message.text);
                        let background = match message.role {
                            ChatRole::User if self.reader_style => palette.field,
                            ChatRole::Assistant => palette.surface,
                            ChatRole::User => palette.surface,
                        };
                        apply_reader_surface_palette(cx, item.clone(), background, palette.border);
                        let message_label = item.widget(cx, ids!(message));
                        apply_reader_label_palette(cx, message_label, palette.text);
                    }
                    item.draw_all_unscoped(cx);
                }
            }
        }
        self.last_revision = data.revision;
        self.last_streaming_revision = data.streaming_revision;
        self.was_streaming = data.is_streaming;
        self.last_style = Some(style);
        self.last_width = Some(width);
        if style_changed {
            cx.cx.redraw_all();
        }
        DrawStep::done()
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        if matches!(event, Event::LiveEdit | Event::ScriptReapply) {
            self.last_style = None;
        }
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
                    let skill_name = item.widget(cx, ids!(skill_name));
                    let skill_kind = item.widget(cx, ids!(skill_kind));
                    let skill_preview = item.widget(cx, ids!(skill_preview));
                    let skill_active = item.widget(cx, ids!(skill_active));
                    let skill_use = item.widget(cx, ids!(skill_use));
                    let skill_edit = item.widget(cx, ids!(skill_edit));
                    let skill_delete = item.widget(cx, ids!(skill_delete));
                    let palette = reader_scene_palette(
                        *READER_BACKGROUND.read().unwrap(),
                        *READER_THEME.read().unwrap(),
                    );
                    apply_reader_surface_palette(cx, item.clone(), palette.surface, palette.border);
                    apply_reader_label_palette(cx, skill_name, palette.text);
                    apply_reader_label_palette(cx, skill_kind, palette.accent);
                    apply_reader_label_palette(cx, skill_preview, palette.muted);
                    apply_reader_button_palette(cx, skill_active, palette);
                    apply_reader_button_palette(cx, skill_use, palette);
                    apply_reader_button_palette(cx, skill_edit, palette);
                    apply_reader_button_palette(cx, skill_delete, palette);
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
    #[rust]
    last_style: Option<(AppLanguage, ReaderBackground, ReaderTheme)>,
}

impl Widget for VocabList {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let projection = VOCAB_VISIBLE.read().unwrap();
        let language = *APP_LANGUAGE.read().unwrap();
        let background = *READER_BACKGROUND.read().unwrap();
        let theme = *READER_THEME.read().unwrap();
        let style = (language, background, theme);
        let style_changed = self.last_style != Some(style);
        let palette = reader_scene_palette(background, theme);
        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_portal_list().borrow_mut() {
                list.set_item_range(cx, 0, projection.entries.len());
                let projection_changed = self.projection_revision != projection.revision;
                if projection_changed {
                    list.set_first_id_and_scroll(0, 0.0);
                    self.projection_revision = projection.revision;
                }
                if projection_changed || style_changed {
                    // Script reload can reset a cached card's measured height.
                    // Recreate visible cards on the next draw.
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
                    let (item, existed) = list.item_with_existed(cx, item_id, template);
                    if !existed || style_changed {
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
                        apply_reader_surface_palette(
                            cx,
                            item.clone(),
                            palette.surface,
                            palette.border,
                        );
                        let vocab_word = item.widget(cx, ids!(vocab_word));
                        let vocab_status = item.widget(cx, ids!(vocab_status));
                        let vocab_gloss = item.widget(cx, ids!(vocab_gloss));
                        let vocab_context = item.widget(cx, ids!(vocab_context));
                        let vocab_meta = item.widget(cx, ids!(vocab_meta));
                        apply_reader_label_palette(cx, vocab_word, palette.text);
                        apply_reader_label_palette(cx, vocab_status, palette.accent);
                        apply_reader_label_palette(cx, vocab_gloss, palette.text);
                        apply_reader_label_palette(cx, vocab_context, palette.muted);
                        apply_reader_label_palette(cx, vocab_meta, palette.muted);
                        for id in [
                            id!(vocab_speak),
                            id!(vocab_review),
                            id!(vocab_toggle),
                            id!(vocab_remove),
                        ] {
                            let button = item.widget(cx, &[id]);
                            apply_reader_button_palette(cx, button, palette);
                        }
                    }
                    item.draw_all_unscoped(cx);
                }
            }
        }
        self.last_style = Some(style);
        if style_changed {
            cx.cx.redraw_all();
        }
        DrawStep::done()
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        if matches!(event, Event::LiveEdit | Event::ScriptReapply) {
            self.last_style = None;
        }
        self.view.handle_event(cx, event, scope);
    }
}

#[derive(Script, ScriptHook, Widget)]
struct BookShelf {
    #[deref]
    view: View,
    #[rust]
    activation_tap: ListTap,
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
    #[rust]
    grid_cover_height: f64,
    #[rust]
    list_cover_heights: HashMap<String, f64>,
    #[rust]
    list_cover_default_height: Option<f64>,
    #[rust]
    list_cover_width: f64,
    #[rust]
    last_language: AppLanguage,
}

impl BookShelf {
    fn bind_book_card(
        &mut self,
        cx: &mut Cx2d,
        card: &WidgetRef,
        book: &ShelfItem,
        language: AppLanguage,
        show_details: bool,
    ) {
        let letter = book
            .title
            .chars()
            .next()
            .map(|c| c.to_string())
            .unwrap_or_default();
        let cover_letter = card.label(cx, ids!(cover_letter));
        if cover_letter.text() != letter {
            cover_letter.set_text(cx, &letter);
        }
        if show_details {
            let title = card.label(cx, ids!(card_title));
            if title.text() != book.title {
                title.set_text(cx, &book.title);
            }
            let author = card.label(cx, ids!(card_author));
            if author.text() != book.author {
                author.set_text(cx, &book.author);
            }
        }
        let status = shelf_status(
            language,
            &book.status,
            book.progress_current,
            book.progress_total,
        );
        let card_status = card.label(cx, ids!(card_status));
        if card_status.text() != status {
            card_status.set_text(cx, &status);
        }
        let card_uid = card.widget_uid();
        if shelf_cover_needs_load(&self.loaded_covers, card_uid, &book.cover_path) {
            let img = card.image(cx, ids!(cover_img));
            let cover_placeholder = card.widget(cx, ids!(cover_ph));
            match &book.cover_path {
                Some(path) => {
                    // A cover that is still loading should not flash the
                    // fallback letter or its colored background.
                    cover_placeholder.set_visible(cx, false);
                    // PortalList can reuse this card for a different book.
                    img.set_texture(cx, None);
                    let path = std::path::Path::new(path);
                    if let Err(error) = img.load_image_file_by_path_async(cx, path) {
                        println!("封面加载失败 {}: {:?}", path.display(), error);
                        img.set_texture(cx, None);
                        cover_placeholder.set_visible(cx, true);
                    }
                }
                None => {
                    img.set_texture(cx, None);
                    cover_placeholder.set_visible(cx, true);
                }
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
        let grid_cover_height = shelf_grid_cover_height(available_width, grid_columns);
        let grid_geometry_changed = self.grid_columns != grid_columns
            || (self.grid_cover_height - grid_cover_height).abs() > 0.5;
        let language_changed = self.last_language != language;
        self.last_language = language;
        if language_changed {
            // Existing cards have their own cached draw lists. Rebinding their
            // labels during this draw needs a subsequent pass to paint them.
            cx.cx.redraw_all();
        }
        if view_mode == LibraryViewMode::List
            && measured_width > 0.0
            && (self.list_cover_width - measured_width).abs() > 0.5
        {
            self.list_cover_heights.clear();
            self.list_cover_default_height = None;
            self.list_cover_width = measured_width;
        }
        while let Some(step) = self.view.draw_walk(cx, scope, walk).step() {
            if let Some(mut list) = step.as_shelf_list().borrow_mut() {
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
                }
                self.grid_cover_height = grid_cover_height;
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
                    let (item, existed) = list.item_with_existed(cx, item_id, template);
                    if view_mode == LibraryViewMode::Grid {
                        // Stable rows already have their cell visibility, cover
                        // geometry and labels. Avoid walking six nested card
                        // trees for every scroll frame.
                        if !existed || language_changed || grid_geometry_changed {
                            for (column, cell_id) in [
                                id!(grid_cell_0),
                                id!(grid_cell_1),
                                id!(grid_cell_2),
                                id!(grid_cell_3),
                                id!(grid_cell_4),
                                id!(grid_cell_5),
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
                                    if let Some(mut cover) = card.view(cx, ids!(cover)).borrow_mut()
                                    {
                                        let cover_height_matches = matches!(
                                            cover.walk.height,
                                            Size::Fixed(height)
                                                if (height - grid_cover_height).abs() <= 0.5
                                        );
                                        if !cover_height_matches {
                                            cover.walk.height = Size::Fixed(grid_cover_height);
                                        }
                                    }
                                    if !existed || language_changed {
                                        self.bind_book_card(
                                            cx,
                                            &card,
                                            &shelf.entries[book_index],
                                            language,
                                            false,
                                        );
                                    }
                                }
                            }
                        }
                    } else if let Some(book) = shelf.entry_for_item_id(item_id) {
                        if !existed {
                            if let Some(height) = self
                                .list_cover_heights
                                .get(&book.id)
                                .copied()
                                .or(self.list_cover_default_height)
                            {
                                if let Some(mut cover) = item.view(cx, ids!(cover)).borrow_mut() {
                                    let size = shelf_list_cover_size(height, available_width);
                                    cover.walk.width = Size::Fixed(size.x);
                                    cover.walk.height = Size::Fixed(size.y);
                                }
                            }
                        }
                        if !existed || language_changed {
                            self.bind_book_card(cx, &item, book, language, true);
                        }
                    }
                    item.draw_all_unscoped(cx);
                    if view_mode == LibraryViewMode::List {
                        // The info area is current only after drawing. Cached
                        // heights let reused cards render their cover in this
                        // pass; only an actual height change needs another.
                        let info_height = item.view(cx, ids!(info)).area().rect(cx).size.y;
                        if info_height.is_finite() && info_height > 0.0 {
                            if let Some(book) = shelf.entry_for_item_id(item_id) {
                                if self
                                    .list_cover_heights
                                    .get(&book.id)
                                    .is_none_or(|height| (height - info_height).abs() > 0.5)
                                {
                                    self.list_cover_heights.insert(book.id.clone(), info_height);
                                }
                            }
                            self.list_cover_default_height = Some(
                                self.list_cover_default_height
                                    .map_or(info_height, |height| height.min(info_height)),
                            );
                            let cover_size = shelf_list_cover_size(info_height, available_width);
                            let cover_width = cover_size.x;
                            let cover_ref = item.view(cx, ids!(cover));
                            let mut cover_size_changed = false;
                            if let Some(mut cover) = cover_ref.borrow_mut() {
                                let old_width = match cover.walk.width {
                                    Size::Fixed(width) => width,
                                    _ => -1.0,
                                };
                                let old_height = match cover.walk.height {
                                    Size::Fixed(height) => height,
                                    _ => -1.0,
                                };
                                if (old_width - cover_width).abs() > 0.5
                                    || (old_height - cover_size.y).abs() > 0.5
                                {
                                    cover.walk.width = Size::Fixed(cover_width);
                                    cover.walk.height = Size::Fixed(cover_size.y);
                                    cover_size_changed = true;
                                }
                            };
                            if cover_size_changed {
                                // redraw() is ignored during a draw event. Queue this
                                // cached PortalList item for the next frame instead.
                                cx.cx.redraw_area_in_draw(item.area());
                            }
                        }
                    }
                }
            }
        }

        DrawStep::done()
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        let list = self.view.shelf_list(cx, ids!(list));
        let rect = current_clipped_rect(cx, list.area());
        let mut allow_activation = self.activation_tap.allows_click(event, rect);
        // The list still receives every release so it can clear finger capture.
        let mut actions = cx.capture_actions(|cx| self.view.handle_event(cx, event, scope));
        if list.scrolled(&actions) || list.was_scrolling() {
            self.activation_tap.cancel();
            allow_activation = false;
        }
        if !allow_activation {
            actions.retain(|action| {
                !action.as_widget_action().is_some_and(|action| {
                    matches!(action.cast(), ViewAction::FingerUp(_))
                        || matches!(action.cast(), ButtonAction::Clicked(_))
                })
            });
        }
        cx.extend_actions(actions);
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
    import_picker_frame: Option<NextFrame>,
    #[rust]
    reader_code_refresh_frame: Option<NextFrame>,
    #[rust]
    selection_popup_layout_frame: Option<NextFrame>,
    #[rust]
    import_dialog_open: bool,
    #[rust]
    lan_transfer_mode: bool,
    #[rust]
    lan_transfer: Option<crate::lan_transfer::LanTransfer>,
    #[rust]
    lan_transfer_status: String,
    #[rust]
    lan_import_count: usize,
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
    reader_pagination_content_key: u64,
    #[rust]
    reader_page_span: usize,
    #[rust]
    pending_reader_pagination_layout: bool,
    #[rust]
    reader_footer_signature: u64,
    /// 选区工具条由 App 这一覆盖层所有者定位；锚点是选区手势结束的屏幕坐标。
    #[rust]
    selection_popup_anchor: Option<DVec2>,
    #[rust]
    pending_selection_popup_layout: bool,
    /// 生词释义卡片由阅读器覆盖层定位，避免参与纵向正文布局。
    #[rust]
    word_card_anchor: Option<DVec2>,
    #[rust]
    pending_word_card_layout: bool,
    #[rust]
    on_reader_page: bool,
    #[rust]
    reader_toolbar_visible: bool,
    #[rust]
    reader_arrow_caption_layout: Option<bool>,
    #[rust]
    reader_theme_open: bool,
    #[rust]
    reader_customise_draft: Option<ReaderSettings>,
    #[rust]
    reader_customise_open_dropdown: Option<LiveId>,
    #[rust]
    active_font_applied: Option<ReaderFont>,
    #[rust]
    preview_font_applied: Option<ReaderFont>,
    #[rust]
    pending_customise_preview_after_draw: bool,
    #[rust]
    active_page: AppPage,
    #[rust]
    shelf_ready: bool,
    #[rust]
    pending_last_read_open: bool,
    #[rust]
    pending_theme_reapply: bool,
    #[rust]
    toc_open: bool,
    #[rust]
    bookmarks_open: bool,
    #[rust]
    search_open: bool,
    #[rust]
    reader_search_query: String,
    /// The TOC may be opened before its hidden panel has painted its first frame.
    #[rust]
    pending_toc_first_draw: bool,
    #[rust]
    note_open: bool,
    #[rust]
    reader_ai_open: bool,
    #[rust]
    compact_layout: bool,
    #[rust]
    responsive_layout_initialized: bool,
    #[rust]
    responsive_window_size: DVec2,
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
    /// Load a macOS font into the separate preview or saved-reading style.
    /// The preview slots never change the active chapter's font before Save.
    fn load_reader_font(&mut self, cx: &mut Cx, font: ReaderFont, preview: bool) {
        if (if preview {
            self.preview_font_applied
        } else {
            self.active_font_applied
        }) == Some(font)
        {
            return;
        }
        if font == ReaderFont::Original {
            if preview {
                self.preview_font_applied = Some(font);
            } else {
                self.active_font_applied = Some(font);
            }
            return;
        }
        let Some(files) = crate::reader_fonts::prepare_reader_font(font) else {
            ::log::warn!("Reader font {} is unavailable", font.label());
            if let Some(mut reader) = self
                .ui
                .widget(cx, ids!(chapter_list))
                .borrow_mut::<ChapterList>()
            {
                let normal = reader.font_standard_normal.font_family.clone();
                let italic = reader.font_standard_italic.font_family.clone();
                let bold = reader.font_standard_bold.font_family.clone();
                let bold_italic = reader.font_standard_bold_italic.font_family.clone();
                if preview {
                    reader.font_preview_normal.font_family = normal;
                    reader.font_preview_italic.font_family = italic;
                    reader.font_preview_bold.font_family = bold;
                    reader.font_preview_bold_italic.font_family = bold_italic;
                } else {
                    reader.font_active_normal.font_family = normal;
                    reader.font_active_italic.font_family = italic;
                    reader.font_active_bold.font_family = bold;
                    reader.font_active_bold_italic.font_family = bold_italic;
                }
                if !preview {
                    reader.view.redraw(cx);
                }
            }
            if preview {
                self.preview_font_applied = Some(font);
            } else {
                self.active_font_applied = Some(font);
            }
            return;
        };
        let regular_path = files.regular.path;
        let italic_path = files.italic.path;
        let bold_path = files.bold.path;
        let bold_italic_path = files.bold_italic.path;
        let regular_weight = files.regular.weight;
        let italic_weight = files.italic.weight;
        let bold_weight = files.bold.weight;
        let bold_italic_weight = files.bold_italic.weight;
        let mut reader = self.ui.widget(cx, ids!(chapter_list));
        if reader.borrow::<ChapterList>().is_none() {
            return;
        }
        if preview {
            script_apply_eval!(cx, reader, {
                font_preview_normal +: {font_family: mod.prelude.widgets.FontFamily{
                    latin := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.file_resource(#(regular_path)) weight: #(regular_weight)}
                    chinese := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.crate_resource("makepad_widgets:resources/LXGWWenKaiRegular.ttf")}
                    emoji := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.crate_resource("makepad_widgets:resources/NotoColorEmoji.ttf")}
                }}
                font_preview_italic +: {font_family: mod.prelude.widgets.FontFamily{
                    latin := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.file_resource(#(italic_path)) weight: #(italic_weight)}
                    chinese := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.crate_resource("makepad_widgets:resources/LXGWWenKaiRegular.ttf")}
                    emoji := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.crate_resource("makepad_widgets:resources/NotoColorEmoji.ttf")}
                }}
                font_preview_bold +: {font_family: mod.prelude.widgets.FontFamily{
                    latin := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.file_resource(#(bold_path)) weight: #(bold_weight)}
                    chinese := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.crate_resource("makepad_widgets:resources/LXGWWenKaiRegular.ttf")}
                    emoji := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.crate_resource("makepad_widgets:resources/NotoColorEmoji.ttf")}
                }}
                font_preview_bold_italic +: {font_family: mod.prelude.widgets.FontFamily{
                    latin := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.file_resource(#(bold_italic_path)) weight: #(bold_italic_weight)}
                    chinese := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.crate_resource("makepad_widgets:resources/LXGWWenKaiRegular.ttf")}
                    emoji := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.crate_resource("makepad_widgets:resources/NotoColorEmoji.ttf")}
                }}
            });
        } else {
            script_apply_eval!(cx, reader, {
                font_active_normal +: {font_family: mod.prelude.widgets.FontFamily{
                    latin := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.file_resource(#(regular_path)) weight: #(regular_weight)}
                    chinese := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.crate_resource("makepad_widgets:resources/LXGWWenKaiRegular.ttf")}
                    emoji := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.crate_resource("makepad_widgets:resources/NotoColorEmoji.ttf")}
                }}
                font_active_italic +: {font_family: mod.prelude.widgets.FontFamily{
                    latin := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.file_resource(#(italic_path)) weight: #(italic_weight)}
                    chinese := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.crate_resource("makepad_widgets:resources/LXGWWenKaiRegular.ttf")}
                    emoji := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.crate_resource("makepad_widgets:resources/NotoColorEmoji.ttf")}
                }}
                font_active_bold +: {font_family: mod.prelude.widgets.FontFamily{
                    latin := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.file_resource(#(bold_path)) weight: #(bold_weight)}
                    chinese := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.crate_resource("makepad_widgets:resources/LXGWWenKaiRegular.ttf")}
                    emoji := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.crate_resource("makepad_widgets:resources/NotoColorEmoji.ttf")}
                }}
                font_active_bold_italic +: {font_family: mod.prelude.widgets.FontFamily{
                    latin := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.file_resource(#(bold_italic_path)) weight: #(bold_italic_weight)}
                    chinese := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.crate_resource("makepad_widgets:resources/LXGWWenKaiRegular.ttf")}
                    emoji := mod.prelude.widgets.FontMember{res: mod.prelude.widgets.crate_resource("makepad_widgets:resources/NotoColorEmoji.ttf")}
                }}
            });
        }
        if !preview {
            reader.redraw(cx);
        }
        if preview {
            self.preview_font_applied = Some(font);
        } else {
            self.active_font_applied = Some(font);
        }
    }

    fn open_reader_customise(&mut self, cx: &mut Cx) {
        self.reader_customise_open_dropdown = None;
        self.reader_customise_draft = Some(current_reader_settings());
        self.pending_customise_preview_after_draw = true;
        READER_CUSTOMISE_OPEN.store(true, Ordering::Release);
        self.reader_swipe.cancel();
        if let Some(mut reader) = self
            .ui
            .widget(cx, ids!(chapter_list))
            .borrow_mut::<ChapterList>()
        {
            reader.page_turn.cancel();
            reader.pager.pending_turn = None;
        }
        self.configure_reader_customise_layout(cx);
        self.ui
            .widget(cx, ids!(reader_customise_scroll))
            .set_scroll_pos(cx, DVec2::default());
        self.ui.modal(cx, ids!(reader_customise_modal)).open(cx);
        self.load_reader_font(cx, self.reader_customise_draft.as_ref().unwrap().font, true);
        self.sync_reader_customise_controls(cx);
    }

    fn configure_library_import_layout(&self, cx: &mut Cx) {
        let size = self.responsive_window_size;
        if size.x <= 0.0 || size.y <= 0.0 {
            return;
        }
        let insets = self.safe_area_insets;
        let available = dvec2(
            (size.x - insets.left - insets.right).max(1.0),
            (size.y - insets.top - insets.bottom).max(1.0),
        );
        let card = dvec2(
            (available.x - 24.0).clamp(1.0, 480.0),
            (available.y - 24.0).clamp(1.0, if self.lan_transfer_mode { 460.0 } else { 280.0 }),
        );
        let content = self.ui.widget(cx, ids!(library_import_modal.content));
        if let Some(mut view) = content.borrow_mut::<View>() {
            view.walk.abs_pos = Some(dvec2(insets.left, insets.top) + (available - card) * 0.5);
            view.walk.width = Size::Fixed(card.x);
            view.walk.height = Size::Fixed(card.y);
        };
    }

    fn sync_library_import_dialog(&self, cx: &mut Cx) {
        for (id, zh, en) in [
            (id!(library_import_title), "导入 EPUB", "Import EPUB"),
            (id!(library_import_local), "本机文件", "Files on this device"),
            (id!(library_import_lan), "局域网传书", "Wi-Fi transfer"),
            (id!(library_import_copy), "复制网址", "Copy address"),
            (id!(library_import_background), "允许后台接收", "Allow background receiving"),
            (id!(library_import_restart), "重新开启", "Start again"),
            (id!(library_import_stop), "停止接收", "Stop receiving"),
            (id!(library_import_close), "关闭界面", "Close screen"),
            (id!(library_import_instructions),
                "两台设备连接同一 Wi-Fi，在电脑浏览器打开下方网址，选择 EPUB 上传（每本最大 200 MB）。",
                "Connect both devices to the same Wi-Fi. Open the address below in a computer browser and upload EPUB files (up to 200 MB each)."),
        ] {
            self.ui.widget(cx, &[id]).set_text(cx, localized(zh, en));
        }
        self.ui
            .widget(cx, ids!(library_import_choices))
            .set_visible(cx, !self.lan_transfer_mode);
        self.ui
            .widget(cx, ids!(library_import_receiver))
            .set_visible(cx, self.lan_transfer_mode);
        self.ui
            .widget(cx, ids!(library_import_address))
            .set_visible(cx, self.lan_transfer.is_some());
        self.ui
            .widget(cx, ids!(library_import_stop))
            .set_visible(cx, self.lan_transfer.is_some());
        self.ui
            .widget(cx, ids!(library_import_restart))
            .set_visible(cx, self.lan_transfer_mode && self.lan_transfer.is_none());
        let urls = self
            .lan_transfer
            .as_ref()
            .map(|s| s.urls.join("\n"))
            .unwrap_or_default();
        self.ui
            .text_input(cx, ids!(library_import_urls))
            .set_text(cx, &urls);
        self.ui
            .widget(cx, ids!(library_import_status))
            .set_text(cx, &self.lan_transfer_status);
        self.configure_library_import_layout(cx);
        if self.import_dialog_open && !self.ui.modal(cx, ids!(library_import_modal)).is_open() {
            self.ui.modal(cx, ids!(library_import_modal)).open(cx);
        }
        self.ui.redraw(cx);
    }

    fn open_library_import_dialog(&mut self, cx: &mut Cx) {
        self.close_library_action_menu(cx);
        self.ui
            .widget(cx, ids!(book_shelf))
            .shelf_list(cx, ids!(list))
            .stop_scrolling();
        self.import_dialog_open = true;
        self.ui
            .widget(cx, ids!(library_import_scroll))
            .set_scroll_pos(cx, DVec2::default());
        if self.lan_transfer.is_some() {
            self.lan_transfer_mode = true;
        } else {
            self.lan_transfer_mode = false;
            self.lan_transfer_status = localized(
                "从本机文件导入，或通过局域网从电脑传书。",
                "Import files from this device, or transfer books over Wi-Fi.",
            )
            .into();
        }
        self.sync_library_import_dialog(cx);
    }

    fn close_library_import_dialog(&mut self, cx: &mut Cx) {
        self.import_dialog_open = false;
        self.ui.modal(cx, ids!(library_import_modal)).close(cx);
    }

    fn start_lan_transfer(&mut self, cx: &mut Cx) {
        if self.lan_transfer.is_some() {
            return;
        }
        self.ui
            .widget(cx, ids!(library_import_scroll))
            .set_scroll_pos(cx, DVec2::default());
        let result = DB
            .get()
            .ok_or_else(|| {
                localized(
                    "书库尚未就绪，请稍后重试。",
                    "The library is not ready. Try again shortly.",
                )
                .to_string()
            })
            .and_then(|db| {
                crate::lan_transfer::LanTransfer::start(
                    db.data_dir(),
                    app_language() == AppLanguage::English,
                    |path| {
                        let db = DB.get().ok_or("Library is not ready")?;
                        let book = db.import_book(path)?;
                        match refresh_shelf(db) {
                            Ok(items) => Cx::post_action(ShelfAction::Loaded {
                                items,
                                imported: 1,
                                import_error: None,
                            }),
                            Err(error) => Cx::post_action(ShelfAction::Error(error)),
                        }
                        Ok(book.title)
                    },
                    |id, notice| {
                        Cx::post_action(LanTransferAction {
                            session_id: id.into(),
                            notice,
                        })
                    },
                )
            });
        match result {
            Ok(receiver) => {
                self.lan_transfer = Some(receiver);
                self.lan_transfer_mode = true;
                self.lan_import_count = 0;
                self.lan_transfer_status = localized("等待传书 · 关闭界面或切到后台仍可接收，网址 30 分钟后失效。", "Waiting for books. Receiving continues when this screen is closed or the app is in the background. This address expires in 30 minutes.").into();
            }
            Err(error) => self.lan_transfer_status = error,
        }
        self.sync_library_import_dialog(cx);
    }

    fn stop_lan_transfer(&mut self, cx: &mut Cx) {
        if self.lan_transfer.take().is_some() {
            self.ui
                .widget(cx, ids!(library_import_scroll))
                .set_scroll_pos(cx, DVec2::default());
            self.lan_transfer_status = localized(
                "传书已停止，请重新开启。",
                "Transfer stopped. Start again to receive books.",
            )
            .into();
            self.sync_library_import_dialog(cx);
        }
    }

    fn configure_reader_customise_layout(&self, cx: &mut Cx) {
        if self.responsive_window_size.x <= 0.0 || self.responsive_window_size.y <= 0.0 {
            return;
        }
        let rect = reader_customise_rect(self.responsive_window_size, self.safe_area_insets);
        let stacked = rect.size.x < 500.0;
        let mut content = self.ui.widget(cx, ids!(reader_customise_modal.content));
        if let Some(mut view) = content.borrow_mut::<View>() {
            view.walk.abs_pos = Some(rect.pos);
        }
        script_apply_eval!(cx, content, {width: #(rect.size.x) height: #(rect.size.y)});
        let mut card = self.ui.widget(cx, ids!(reader_customise_card));
        let padding = if stacked { 12.0 } else { 20.0 };
        let spacing = if stacked { 12.0 } else { 16.0 };
        script_apply_eval!(cx, card, {padding: #(padding) spacing: #(spacing)});
        let mut preview = self.ui.widget(cx, ids!(reader_preview));
        let preview_height = if stacked { 140.0 } else { 180.0 };
        script_apply_eval!(cx, preview, {height: #(preview_height)});
        for id in [
            id!(reader_customise_font_row),
            id!(reader_customise_size_row),
            id!(reader_customise_gloss_size_row),
            id!(reader_customise_highlight_row),
            id!(reader_customise_gloss_color_row),
            id!(reader_customise_gloss_bg_row),
            id!(reader_customise_word_color_row),
            id!(reader_customise_word_bg_row),
            id!(reader_customise_line_row),
            id!(reader_customise_character_row),
            id!(reader_customise_word_row),
            id!(reader_customise_margin_row),
            id!(reader_customise_columns_row),
        ] {
            let row = self.ui.widget(cx, &[id]);
            if let Some(mut view) = row.borrow_mut::<View>() {
                let dropdown_row =
                    id == id!(reader_customise_font_row) || id == id!(reader_customise_columns_row);
                view.layout.flow = if stacked && !dropdown_row {
                    Flow::Down
                } else {
                    Flow::right()
                };
                view.layout.spacing = if stacked && !dropdown_row { 6.0 } else { 8.0 };
            };
        }
        for id in [
            id!(reader_customise_highlight_options),
            id!(reader_customise_gloss_color_options),
            id!(reader_customise_gloss_bg_options),
            id!(reader_customise_word_color_options),
            id!(reader_customise_word_bg_options),
        ] {
            let options = self.ui.widget(cx, &[id]);
            if let Some(mut view) = options.borrow_mut::<View>() {
                view.walk.width = if stacked { Size::fill() } else { Size::fit() };
                view.layout.flow = if stacked {
                    Flow::right_wrap()
                } else {
                    Flow::right()
                };
            };
        }
        for id in [
            id!(reader_customise_size_label),
            id!(reader_customise_gloss_size_label),
            id!(reader_customise_highlight_label),
            id!(reader_customise_gloss_color_label),
            id!(reader_customise_gloss_bg_label),
            id!(reader_customise_word_color_label),
            id!(reader_customise_word_bg_label),
            id!(reader_customise_line_label),
            id!(reader_customise_character_label),
            id!(reader_customise_word_label),
            id!(reader_customise_margin_label),
        ] {
            let mut label = self.ui.widget(cx, &[id]);
            let width = if stacked {
                Size::fill()
            } else {
                Size::Fixed(164.0)
            };
            script_apply_eval!(cx, label, {width: #(width)});
        }
        for id in [
            id!(reader_customise_font_choice),
            id!(reader_customise_columns_choice),
        ] {
            let mut choice = self.ui.widget(cx, &[id]);
            let width = Size::Fixed(180.0);
            script_apply_eval!(cx, choice, {width: #(width)});
        }
        let mut save = self.ui.widget(cx, ids!(reader_customise_save));
        let width = if stacked {
            Size::fill()
        } else {
            Size::Fixed(148.0)
        };
        script_apply_eval!(cx, save, {width: #(width)});
        // Modal owns a separate overlay draw list; invalidate it after rotation.
        let mut modal = self.ui.widget(cx, ids!(reader_customise_modal));
        script_apply_eval!(cx, modal, {});
    }

    fn close_reader_customise_dropdowns(&mut self, cx: &mut Cx) {
        self.reader_customise_open_dropdown = None;
        for id in [
            id!(reader_customise_font_choice),
            id!(reader_customise_columns_choice),
        ] {
            if let Some(mut dropdown) = self.ui.widget(cx, &[id]).borrow_mut::<DropDown>() {
                dropdown.set_closed(cx);
            }
        }
    }

    fn close_reader_customise(&mut self, cx: &mut Cx) {
        self.close_reader_customise_dropdowns(cx);
        READER_CUSTOMISE_OPEN.store(false, Ordering::Release);
        self.reader_customise_draft = None;
        self.pending_customise_preview_after_draw = false;
        self.ui.modal(cx, ids!(reader_customise_modal)).close(cx);
    }

    fn save_reader_customise(&mut self, cx: &mut Cx) {
        self.close_reader_customise_dropdowns(cx);
        let draft = self.reader_customise_draft.take();
        READER_CUSTOMISE_OPEN.store(false, Ordering::Release);
        self.pending_customise_preview_after_draw = false;
        self.ui.modal(cx, ids!(reader_customise_modal)).close(cx);
        if let Some(settings) = draft {
            let current = current_reader_settings();
            if settings != current {
                if reader_customise_change_is_highlight_only(&current, &settings) {
                    *crate::mark_span::HL_SLOT.write().unwrap() = settings.highlight_color_slot;
                    self.refresh_swatches(cx);
                    self.ui.redraw(cx);
                } else {
                    self.apply_reader_settings(cx, settings);
                }
                self.persist_reader_settings(cx);
            }
        }
    }

    fn dismiss_reader_customise_dropdowns_outside(&mut self, cx: &mut Cx, abs: DVec2) -> bool {
        if self.reader_customise_draft.is_none() {
            return false;
        }
        // Makepad's DropDown dismisses on MouseDown outside its popup, but
        // Android sends TouchUpdate. Consume the outside press so controls
        // behind the overlay cannot also be activated.
        let dropdowns = [
            (id!(reader_customise_font_choice), 202.0),
            (id!(reader_customise_columns_choice), 72.0),
        ];
        let pressed_choice = dropdowns.iter().find_map(|(id, _)| {
            let widget = self.ui.widget(cx, &[*id]);
            let control = widget.area().rect(cx);
            control.contains(abs).then_some(*id)
        });
        if let Some(active_id) = self.reader_customise_open_dropdown {
            let menu_height = dropdowns
                .iter()
                .find_map(|(id, height)| (*id == active_id).then_some(*height))
                .unwrap_or(0.0);
            let active = self.ui.widget(cx, &[active_id]);
            let control = active.area().rect(cx);
            let menu = Rect {
                pos: control.pos + dvec2(0.0, control.size.y),
                size: dvec2(control.size.x, menu_height),
            };
            if control.contains(abs) {
                self.close_reader_customise_dropdowns(cx);
                return true;
            }
            if menu.contains(abs) {
                return false;
            }
            self.close_reader_customise_dropdowns(cx);
            if let Some(id) = pressed_choice {
                self.reader_customise_open_dropdown = Some(id);
                return false;
            }
            return true;
        }
        self.reader_customise_open_dropdown = pressed_choice;
        false
    }

    fn project_db_counts(&self, cx: &mut Cx, vocab_count: usize, known_count: usize) {
        let status = if app_language() == AppLanguage::English {
            format!(
                "Database ready\nVocabulary {}\nKnown {}",
                vocab_count, known_count
            )
        } else {
            format!("数据库就绪\n生词本 {}\n已认识 {}", vocab_count, known_count)
        };
        self.ui.label(cx, ids!(db_status)).set_text(cx, &status);
    }

    fn project_known_word_count(&self, cx: &mut Cx, known_count: usize) {
        let counts = {
            let mut cached = DB_READY_COUNTS.write().unwrap();
            cached.as_mut().map(|(vocab_count, cached_known_count)| {
                // Concurrent saves can finish in a different order than their
                // UI actions. Marking known words only increases this count.
                *cached_known_count = (*cached_known_count).max(known_count);
                (*vocab_count, *cached_known_count)
            })
        };
        if let Some((vocab_count, known_count)) = counts {
            self.project_db_counts(cx, vocab_count, known_count);
        }
    }

    fn project_db_ready_status(&self, cx: &mut Cx, vocab_count: usize, known_count: usize) {
        self.project_db_counts(cx, vocab_count, known_count);
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
        if self.on_reader_page {
            self.sync_reader_panel_layout(cx);
            // Reload can reset label text even when cached page geometry is unchanged.
            self.update_progress_label(cx);
            self.update_reader_page_footer(cx);
            // Reload resets draw Areas; repaint cached batches after projecting labels.
            cx.redraw_all();
        }
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
        let returning_to_reader = !self.on_reader_page && page == AppPage::Reader;
        if page != AppPage::Library && self.import_dialog_open {
            self.close_library_import_dialog(cx);
        }
        if page != AppPage::Library {
            self.ui
                .widget(cx, ids!(book_shelf))
                .shelf_list(cx, ids!(list))
                .stop_scrolling();
        }
        let retain_toolbar = self.on_reader_page && page == AppPage::Reader;
        if self.on_reader_page && page != AppPage::Reader {
            // Reader overlays retain their own local visibility even after the
            // whole reader page is hidden. Clear them before another page gets
            // pointer events so no stale Area from the old draw list survives.
            self.clear_reader_selection(cx);
            self.close_word_card(cx);
            if let Some(mut reader) = self
                .ui
                .widget(cx, ids!(chapter_list))
                .borrow_mut::<ChapterList>()
            {
                reader.page_turn.cancel();
            }
        }
        if page != AppPage::Library && self.library_action_book_id.is_some() {
            self.close_library_action_menu(cx);
        }
        self.ui
            .page_flip(cx, ids!(pages))
            .set_active_page(cx, page.live_id());
        self.active_page = page;
        ACTIVE_PAGE.store(page.persisted(), Ordering::Relaxed);
        self.on_reader_page = page == AppPage::Reader;
        if returning_to_reader {
            self.reader_code_refresh_frame = Some(cx.new_next_frame());
        } else if !self.on_reader_page {
            self.reader_code_refresh_frame = None;
        }
        if !self.on_reader_page {
            READER_INFO_POPUP_OPEN.store(false, Ordering::Relaxed);
            if self.reader_customise_draft.is_some() {
                self.close_reader_customise(cx);
            }
            self.reader_theme_open = false;
            self.ui
                .widget(cx, ids!(reader_theme_menu))
                .set_visible(cx, false);
            self.ui
                .widget(cx, ids!(reader_theme_arrow_row))
                .set_visible(cx, false);
        }
        self.ui
            .widget(cx, ids!(sidebar))
            .set_visible(cx, !self.compact_layout && !self.on_reader_page);
        self.sync_mobile_chrome(cx);
        self.set_reader_toolbar_visible(cx, retain_toolbar && self.reader_toolbar_visible);
        self.sync_reader_toolbar_placement(cx);
        self.apply_reader_background(cx, *READER_BACKGROUND.read().unwrap());
        self.sync_reading_home(cx);
        if page == AppPage::Skills {
            spawn_load_skills();
        }
        if page == AppPage::Notes {
            self.sync_all_notes_status(cx);
        }
        if let Some(snapshot) = PROFILE_STATISTICS.read().unwrap().as_ref() {
            self.apply_statistics(cx, snapshot);
        }
    }

    fn sync_all_notes_status(&self, cx: &mut Cx) {
        let count = ALL_NOTES.read().unwrap().len();
        self.ui.label(cx, ids!(all_notes_status)).set_text(
            cx,
            &if count == 0 {
                tr("no_notes_hint").to_string()
            } else {
                format!("{} · {}", localized("累计笔记", "Total notes"), count)
            },
        );
    }

    fn reading_home_book() -> Option<ShelfItem> {
        let current_id = CURRENT_BOOK
            .read()
            .unwrap()
            .as_ref()
            .map(|book| book.id.clone());
        let shelf = SHELF.read().unwrap();
        recent_read_shelf_item(&shelf, current_id.as_deref()).cloned()
    }

    fn sync_reading_home(&self, cx: &mut Cx) {
        let book = Self::reading_home_book();
        self.ui.label(cx, ids!(reading_continue_title)).set_text(
            cx,
            book.as_ref()
                .map(|b| b.title.as_str())
                .unwrap_or(localized("从一本好书开始", "Start with a good book")),
        );
        self.ui.label(cx, ids!(reading_continue_hint)).set_text(
            cx,
            &book
                .as_ref()
                .map(|b| {
                    format!(
                        "{} · {}",
                        b.author,
                        if b.progress_total > 0 {
                            format!("{} / {}", b.progress_current, b.progress_total)
                        } else {
                            localized("开始阅读", "Start reading").to_string()
                        }
                    )
                })
                .unwrap_or(
                    localized(
                        "在书架选择一本书，开始阅读。",
                        "Choose a book from your bookshelf to start reading.",
                    )
                    .to_string(),
                ),
        );
        self.ui.button(cx, ids!(reading_continue)).set_text(
            cx,
            if book.is_some() {
                localized("继续阅读", "Continue reading")
            } else {
                tr("browse_books")
            },
        );
    }

    fn sync_reader_toolbar_placement(&mut self, cx: &mut Cx) {
        let in_caption = self.on_reader_page
            && !self.compact_layout
            && self.ui.widget(cx, ids!(main_window.caption_bar)).visible();
        let titlebar = self.ui.widget(cx, ids!(reader_titlebar));
        if titlebar.visible() != in_caption {
            titlebar.set_visible(cx, in_caption);
            self.ui.widget(cx, ids!(main_window.caption_bar)).redraw(cx);
        }
        let header = self.ui.widget(cx, ids!(header));
        let header_visible = !in_caption
            && (!self.compact_layout || (self.on_reader_page && self.reader_toolbar_visible));
        if header.visible() != header_visible {
            header.set_visible(cx, header_visible);
            self.ui.widget(cx, ids!(page_reader)).redraw(cx);
        }
        let body_top = reader_body_top_inset(self.compact_layout, in_caption);
        let body = self.ui.widget(cx, ids!(reader_body));
        if let Some(mut inner) = body.borrow_mut::<View>() {
            if inner.walk.margin.top != body_top {
                inner.walk.margin.top = body_top;
                inner.redraw(cx);
            }
        }
        if self.reader_arrow_caption_layout != Some(in_caption) {
            self.reader_arrow_caption_layout = Some(in_caption);
            self.sync_reader_popup_arrow_alignment(cx, in_caption);
        }
    }

    fn sync_reader_popup_arrow_alignment(&self, cx: &mut Cx, in_caption: bool) {
        let button_size =
            mobile_reader_button_size(self.responsive_window_size.x, self.safe_area_insets);
        let left_offsets = if self.compact_layout {
            [1.0, 2.0, 3.0].map(|index| {
                self.safe_area_insets.left + index * (button_size + 2.0) + button_size / 2.0 - 18.0
            })
        } else if in_caption {
            [112.0, 144.0, 176.0]
        } else {
            [50.0, 90.0, 130.0]
        };
        for (id, left) in [
            id!(toc_arrow_row),
            id!(reader_bookmarks_arrow_row),
            id!(note_arrow_row),
        ]
        .into_iter()
        .zip(left_offsets)
        {
            let padding = Inset {
                left,
                right: 0.0,
                top: 0.0,
                bottom: 0.0,
            };
            let mut row = self.ui.widget(cx, &[id]);
            script_apply_eval!(cx, row, {padding: #(padding)});
        }
        let padding = Inset {
            left: 0.0,
            right: if self.compact_layout {
                self.safe_area_insets.right + 2.5 * button_size - 14.0
            } else if in_caption {
                70.0
            } else {
                90.0
            },
            top: 0.0,
            bottom: 0.0,
        };
        let mut row = self.ui.widget(cx, ids!(reader_theme_arrow_row));
        script_apply_eval!(cx, row, {padding: #(padding)});
        self.ui.widget(cx, ids!(reader_panel_layer)).redraw(cx);
        self.ui.widget(cx, ids!(reader_theme_layer)).redraw(cx);
    }

    fn set_reader_toolbar_visible(&mut self, cx: &mut Cx, visible: bool) {
        self.reader_toolbar_visible = visible;
        for id in [
            id!(reader_toolbar_left_buttons),
            id!(reader_toolbar_right_buttons),
            id!(reader_titlebar_left_buttons),
            id!(reader_titlebar_right_buttons),
        ] {
            self.ui.widget(cx, &[id]).set_visible(cx, visible);
        }
        self.ui.widget(cx, ids!(header)).redraw(cx);
        self.ui.widget(cx, ids!(main_window.caption_bar)).redraw(cx);
        self.sync_reader_toolbar_placement(cx);
        self.sync_reader_popup_top_inset(cx);
    }

    fn sync_reader_popup_top_inset(&self, cx: &mut Cx) {
        let top = if self.compact_layout && self.reader_toolbar_visible {
            88.0
        } else {
            0.0
        };
        for id in [id!(reader_panel_layer), id!(reader_theme_layer)] {
            let widget = self.ui.widget(cx, &[id]);
            if let Some(mut view) = widget.borrow_mut::<View>() {
                view.layout.padding.top = top;
                if id == id!(reader_theme_layer) {
                    view.layout.padding.left = if self.compact_layout {
                        self.safe_area_insets.left + 12.0
                    } else {
                        0.0
                    };
                    view.layout.padding.right = if self.compact_layout {
                        self.safe_area_insets.right + 12.0
                    } else {
                        18.0
                    };
                }
                view.redraw(cx);
            };
        }
        // Short landscape windows scroll the theme/colour content, not the fixed
        // Customise action. Account for the actual overlay origin and safe area.
        let layer_top = self
            .ui
            .widget(cx, ids!(reader_theme_layer))
            .area()
            .rect(cx)
            .pos
            .y;
        let popup_top = layer_top.max(self.safe_area_insets.top) + top + 10.0;
        let available =
            (self.responsive_window_size.y - self.safe_area_insets.bottom - popup_top - 12.0)
                .max(1.0);
        let width = if self.compact_layout {
            (self.responsive_window_size.x
                - self.safe_area_insets.left
                - self.safe_area_insets.right
                - 24.0)
                .clamp(1.0, 316.0)
        } else {
            316.0
        };
        let stacked = width < 316.0;
        for id in [
            id!(reader_quick_pen_title),
            id!(reader_quick_highlight_title),
        ] {
            let mut label = self.ui.widget(cx, &[id]);
            let height = if stacked {
                Size::fit()
            } else {
                Size::Fixed(44.0)
            };
            script_apply_eval!(cx, label, {height: #(height)});
        }
        let natural_height: f64 = if stacked { 400.0 } else { 356.0 };
        let height = natural_height.min(available);
        let mut menu = self.ui.widget(cx, ids!(reader_theme_menu));
        script_apply_eval!(cx, menu, {width: #(width) height: #(height)});
        let mode_width = ((width - 32.0 - 14.0) / 3.0).clamp(44.0, 88.0);
        for id in [
            id!(reader_mode_original),
            id!(reader_mode_quiet),
            id!(reader_mode_paper),
            id!(reader_mode_bold),
            id!(reader_mode_calm),
            id!(reader_mode_focus),
        ] {
            let mut button = self.ui.widget(cx, &[id]);
            script_apply_eval!(cx, button, {width: #(mode_width)});
        }
    }

    fn configure_reader_toolbar_layout(&mut self, cx: &mut Cx, size: DVec2) {
        let compact = self.compact_layout;
        let button_size = if compact {
            mobile_reader_button_size(size.x, self.safe_area_insets)
        } else {
            36.0
        };
        let header_height = if compact { 88.0 } else { 54.0 };
        let padding = Inset {
            left: if compact {
                self.safe_area_insets.left + 8.0
            } else {
                18.0
            },
            right: if compact {
                self.safe_area_insets.right + 8.0
            } else {
                18.0
            },
            top: 6.0,
            bottom: 6.0,
        };
        let mut header = self.ui.widget(cx, ids!(header));
        script_apply_eval!(cx, header, {height: #(header_height) padding: #(padding)});
        self.ui
            .widget(cx, ids!(mobile_reader_title))
            .set_visible(cx, compact);
        let mut title = self.ui.widget(cx, ids!(mobile_reader_title));
        let title_width = (size.x - padding.left - padding.right).max(1.0);
        script_apply_eval!(cx, title, {width: #(title_width)});
        self.ui
            .widget(cx, ids!(reader_title_slot))
            .set_visible(cx, !compact);
        let mut left = self.ui.widget(cx, ids!(reader_toolbar_left));
        let mut right = self.ui.widget(cx, ids!(reader_toolbar_right));
        let left_width = if compact {
            Size::fit()
        } else {
            Size::Fixed(180.0)
        };
        let right_width = if compact {
            Size::fill()
        } else {
            Size::Fixed(180.0)
        };
        script_apply_eval!(cx, left, {width: #(left_width)});
        script_apply_eval!(cx, right, {width: #(right_width)});
        for id in [
            id!(reader_toolbar_left_buttons),
            id!(reader_toolbar_right_buttons),
        ] {
            let spacing = if compact { 2.0 } else { 4.0 };
            let mut group = self.ui.widget(cx, &[id]);
            script_apply_eval!(cx, group, {spacing: #(spacing)});
        }
        for id in [
            id!(reader_back_btn),
            id!(toc_btn),
            id!(reader_bookmarks_btn),
            id!(notes_btn),
            id!(reader_appearance_btn),
            id!(reader_search_btn),
            id!(reader_bookmark_btn),
            id!(reader_bookmark_filled_btn),
        ] {
            let mut button = self.ui.widget(cx, &[id]);
            script_apply_eval!(cx, button, {
                width: #(button_size) height: #(button_size) margin: 0
            });
        }
        let mut menu = self.ui.widget(cx, ids!(reader_theme_menu));
        let menu_width = if compact {
            (size.x - self.safe_area_insets.left - self.safe_area_insets.right - 24.0)
                .clamp(1.0, 316.0)
        } else {
            316.0
        };
        script_apply_eval!(cx, menu, {width: #(menu_width)});
        let mut arrow = self.ui.widget(cx, ids!(reader_theme_arrow_row));
        script_apply_eval!(cx, arrow, {width: #(menu_width)});
        self.reader_arrow_caption_layout = None;
        self.sync_reader_toolbar_placement(cx);
        self.sync_reader_popup_top_inset(cx);
    }

    fn set_reader_title_text(&self, cx: &mut Cx, text: &str) {
        self.ui.label(cx, ids!(title)).set_text(cx, text);
        self.ui
            .label(cx, ids!(mobile_reader_title))
            .set_text(cx, text);
        self.ui
            .label(cx, ids!(titlebar_reader_title))
            .set_text(cx, text);
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
            id!(mobile_nav_vocab),
            id!(mobile_nav_profile),
        ] {
            // Navigation buttons keep keyboard focus after a mouse click. A
            // highlighted focus background makes the previously clicked button
            // animate back to normal when the next button takes focus.
            apply_reader_button_palette_with_focus(
                cx,
                self.ui.widget(cx, &[id]),
                palette,
                palette.field,
            );
        }
        for (id, selected) in [
            (
                id!(mobile_nav_reader),
                matches!(page, AppPage::Reader | AppPage::ReadingHome),
            ),
            (id!(mobile_nav_library), page == AppPage::Library),
            (id!(mobile_nav_vocab), page == AppPage::Vocab),
            (
                id!(mobile_nav_profile),
                matches!(
                    page,
                    AppPage::Profile
                        | AppPage::Chat
                        | AppPage::Skills
                        | AppPage::Statistics
                        | AppPage::Settings
                        | AppPage::Notes
                ),
            ),
        ] {
            let color = if selected {
                palette.accent
            } else {
                palette.muted
            };
            let mut tab = self.ui.widget(cx, &[id]);
            script_apply_eval!(cx, tab, {draw_text +: {color: #(color) color_hover: #(color) color_focus: #(color) color_down: #(color)}});
            apply_reader_icon_palette(cx, tab.clone(), color);
            apply_flat_entry_background(cx, tab, palette);
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
                && selected != READING_FILTER
                && selected != COMPLETED_FILTER
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
        labels.push(tr("reading_books").to_string());
        labels.push(tr("completed_books").to_string());
        labels.extend(tags.iter().cloned());
        let selected_index = self
            .library_tag_filter
            .as_ref()
            .and_then(|selected| {
                if selected == WANT_TO_READ_FILTER {
                    Some(1)
                } else if selected == READING_FILTER {
                    Some(2)
                } else if selected == COMPLETED_FILTER {
                    Some(3)
                } else {
                    tags.iter()
                        .position(|tag| tag.eq_ignore_ascii_case(selected))
                        .map(|index| index + 4)
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
        self.sync_reading_home(cx);
    }

    fn set_library_tag_filter(&mut self, cx: &mut Cx, index: usize) {
        self.close_library_action_menu(cx);
        self.library_tag_filter = match index {
            0 => None,
            1 => Some(WANT_TO_READ_FILTER.to_string()),
            2 => Some(READING_FILTER.to_string()),
            3 => Some(COMPLETED_FILTER.to_string()),
            index => Self::available_shelf_tags().get(index - 4).cloned(),
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
        self.toc_open = false;
        self.note_open = false;
        self.reader_ai_open = false;
        self.bookmarks_open = false;
        self.search_open = false;
        self.reader_search_query.clear();
        self.pending_toc_first_draw = false;
        READER_BOOKMARKS.write().unwrap().clear();
        READER_SEARCH_RESULTS.write().unwrap().clear();
        READER_HIGHLIGHT_JUMPS.write().unwrap().clear();
        self.ui
            .text_input(cx, ids!(reader_search_input))
            .set_text(cx, "");
        self.ui
            .widget(cx, ids!(toc_list))
            .shelf_list(cx, ids!(list))
            .set_first_id_and_scroll(0, 0.0);
        self.replace_notes(cx, Vec::new());
        self.set_reader_title_text(
            cx,
            &if app_language() == AppLanguage::English {
                format!("Loading “{}”…", book.title)
            } else {
                format!("《{}》加载中…", book.title)
            },
        );
        self.activate_page(cx, AppPage::Reader);
        self.sync_reader_panel_layout(cx);
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
                if library_card_release_opens_reader(
                    library_menu_was_open,
                    event.is_over,
                    event.was_tap(),
                ) {
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
        self.layout_reader_image_viewer(cx);
        self.apply_reader_background(cx, *READER_BACKGROUND.read().unwrap());
        self.ui.modal(cx, ids!(reader_image_modal)).open(cx);
    }

    fn layout_reader_image_viewer(&mut self, cx: &mut Cx) {
        let image = self.ui.image(cx, ids!(image_viewer_image));
        let Some((image_width, image_height)) = image.size_in_pixels(cx) else {
            return;
        };
        if image_width == 0 || image_height == 0 {
            return;
        }
        let size = self.responsive_window_size;
        let safe = self.safe_area_insets;
        // Card margins and padding each consume space on both sides.
        let available_width = (size.x - safe.left - safe.right - 60.0).max(1.0);
        let available_height = (size.y - safe.top - safe.bottom - 60.0).max(1.0);
        let scale =
            (available_width / image_width as f64).min(available_height / image_height as f64);
        if let Some(mut inner) = image.borrow_mut() {
            inner.walk.width = Size::Fixed(image_width as f64 * scale);
            inner.walk.height = Size::Fixed(image_height as f64 * scale);
        }
        image.redraw(cx);
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

    fn sync_reader_columns_controls(&self, cx: &mut Cx, columns: ReaderColumns) {
        for (id, mode) in [
            (id!(reader_columns_one), ReaderColumns::One),
            (id!(reader_columns_two), ReaderColumns::Two),
            (id!(reader_columns_auto), ReaderColumns::Auto),
        ] {
            self.ui
                .radio_button(cx, &[id])
                .set_active(cx, mode == columns, Animate::No);
        }
    }

    fn set_reader_columns(&mut self, cx: &mut Cx, columns: ReaderColumns) {
        if *READER_COLUMNS.read().unwrap() == columns {
            self.sync_reader_columns_controls(cx, columns);
            self.sync_reader_customise_controls(cx);
            return;
        }
        *READER_COLUMNS.write().unwrap() = columns;
        self.sync_reader_columns_controls(cx, columns);
        self.sync_reader_customise_controls(cx);
        self.clear_reader_selection(cx);
        self.invalidate_reader_pagination_layout(cx);
        self.persist_reader_settings(cx);
    }

    fn sync_reader_turn_controls(&self, cx: &mut Cx, mode: ReaderTurnMode) {
        for (id, option) in [
            (id!(reader_turn_curl), ReaderTurnMode::Curl),
            (id!(reader_turn_slide), ReaderTurnMode::Slide),
            (id!(reader_turn_scroll), ReaderTurnMode::Scroll),
        ] {
            self.ui
                .radio_button(cx, &[id])
                .set_active(cx, mode == option, Animate::No);
        }
        self.ui
            .view(cx, ids!(reader_columns_row))
            .set_visible(cx, mode.is_paged());
        self.ui
            .widget(cx, ids!(reader_scroll_hint))
            .set_visible(cx, !mode.is_paged());
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

    fn sync_reader_mode_controls(&self, cx: &mut Cx, background: ReaderBackground) {
        let accent = reader_scene_palette(background, *READER_THEME.read().unwrap()).accent;
        for (id, mode, surface, text) in [
            (
                id!(reader_mode_original),
                ReaderBackground::Original,
                0xffff_ffff,
                0x2020_20ff,
            ),
            (
                id!(reader_mode_quiet),
                ReaderBackground::Sepia,
                0xf2e3_c6ff,
                0x3b30_25ff,
            ),
            (
                id!(reader_mode_paper),
                ReaderBackground::Paper,
                0xf7f4_eaff,
                0x2b29_25ff,
            ),
            (
                id!(reader_mode_bold),
                ReaderBackground::Bold,
                0xfaf8_f2ff,
                0x1717_17ff,
            ),
            (
                id!(reader_mode_calm),
                ReaderBackground::Sage,
                0xe7ef_e3ff,
                0x2734_2bff,
            ),
            (
                id!(reader_mode_focus),
                ReaderBackground::Night,
                0x2025_2bff,
                0xd8d4_caff,
            ),
        ] {
            let mut button = self.ui.widget(cx, &[id]);
            let surface = Vec4f::from_u32(surface);
            let text = Vec4f::from_u32(text);
            let border = if mode == background { accent } else { surface };
            script_apply_eval!(cx, button, {
                draw_bg +: {
                    color: #(surface)
                    color_hover: #(surface)
                    color_down: #(surface)
                    color_focus: #(surface)
                    border_color: #(border)
                    border_color_focus: #(border)
                    border_color_hover: #(border)
                    border_color_down: #(border)
                    border_size: 2.0
                }
                draw_text +: {
                    color: #(text)
                    color_focus: #(text)
                    color_hover: #(text)
                    color_down: #(text)
                }
            });
        }
    }

    fn paint_color_swatch(
        &self,
        cx: &mut Cx,
        id: LiveId,
        color: Vec4f,
        selected: bool,
        palette: ReaderScenePalette,
    ) {
        let ring_width = if selected { 1.4 } else { 0.0 };
        let ring_color = palette.muted;
        let mut button = self.ui.widget(cx, &[id]);
        script_apply_eval!(cx, button, {
            draw_bg +: {
                color: #(color)
                border_color: #(ring_color)
                border_size: #(ring_width)
            }
        });
    }

    fn sync_reader_quick_colors(&self, cx: &mut Cx) {
        let settings = current_reader_settings();
        let palette = reader_scene_palette(settings.background, settings.theme);
        for (cells, name, label, kind, selected, slots) in [
            (
                [
                    id!(reader_quick_pen_vermilion),
                    id!(reader_quick_pen_cobalt),
                    id!(reader_quick_pen_moss),
                    id!(reader_quick_pen_graphite),
                ],
                id!(reader_quick_pen_name),
                id!(reader_quick_pen_label),
                ColorKind::Selection,
                settings.selection_color_slot,
                crate::settings_swatch::PEN_SLOTS,
            ),
            (
                [
                    id!(reader_quick_highlight_yellow),
                    id!(reader_quick_highlight_green),
                    id!(reader_quick_highlight_blue),
                    id!(reader_quick_highlight_pink),
                ],
                id!(reader_quick_highlight_name),
                id!(reader_quick_highlight_label),
                ColorKind::Highlight,
                settings.highlight_color_slot,
                crate::mark_span::MARKER_SLOTS,
            ),
        ] {
            for (cell, slot) in cells.into_iter().zip(slots) {
                self.paint_color_swatch(
                    cx,
                    cell,
                    kind.color(slot, palette.text),
                    selected == slot,
                    palette,
                );
            }
            self.ui
                .widget(cx, &[name])
                .set_text(cx, kind.name(selected, app_language()));
            for id in [name, label] {
                apply_reader_label_palette(cx, self.ui.widget(cx, &[id]), palette.muted);
            }
        }
    }

    fn sync_reader_customise_annotation_colors(&self, cx: &mut Cx, settings: &ReaderSettings) {
        let palette = reader_scene_palette(settings.background, settings.theme);
        for (cells, name, label, kind, selected) in [
            (
                [
                    id!(reader_customise_gloss_color_green),
                    id!(reader_customise_gloss_color_blue),
                    id!(reader_customise_gloss_color_orange),
                    id!(reader_customise_gloss_color_gray),
                ],
                id!(reader_customise_gloss_color_name),
                id!(reader_customise_gloss_color_label),
                ColorKind::Gloss,
                settings.gloss_color_slot,
            ),
            (
                [
                    id!(reader_customise_gloss_bg_none),
                    id!(reader_customise_gloss_bg_gray),
                    id!(reader_customise_gloss_bg_green),
                    id!(reader_customise_gloss_bg_blue),
                ],
                id!(reader_customise_gloss_bg_name),
                id!(reader_customise_gloss_bg_label),
                ColorKind::Background,
                settings.gloss_background_slot,
            ),
            (
                [
                    id!(reader_customise_word_color_default),
                    id!(reader_customise_word_color_green),
                    id!(reader_customise_word_color_blue),
                    id!(reader_customise_word_color_orange),
                ],
                id!(reader_customise_word_color_name),
                id!(reader_customise_word_color_label),
                ColorKind::Word,
                settings.word_color_slot,
            ),
            (
                [
                    id!(reader_customise_word_bg_none),
                    id!(reader_customise_word_bg_gray),
                    id!(reader_customise_word_bg_green),
                    id!(reader_customise_word_bg_blue),
                ],
                id!(reader_customise_word_bg_name),
                id!(reader_customise_word_bg_label),
                ColorKind::Background,
                settings.word_background_slot,
            ),
        ] {
            for (slot, cell) in cells.into_iter().enumerate() {
                self.paint_color_swatch(
                    cx,
                    cell,
                    kind.color(slot, palette.text),
                    selected == slot,
                    palette,
                );
            }
            self.ui
                .widget(cx, &[name])
                .set_text(cx, kind.name(selected, app_language()));
            for id in [name, label] {
                apply_reader_label_palette(cx, self.ui.widget(cx, &[id]), palette.muted);
            }
        }
    }

    fn sync_reader_customise_font_labels(
        &self,
        cx: &mut Cx,
        language: AppLanguage,
        font: ReaderFont,
    ) {
        let choice = self.ui.drop_down(cx, ids!(reader_customise_font_choice));
        choice.set_labels(
            cx,
            ReaderFont::ALL
                .iter()
                .map(|font| reader_font_display_label(language, *font).to_string())
                .collect(),
        );
        choice.set_selected_item(cx, font.index());
    }

    fn sync_reader_customise_highlighter(&self, cx: &mut Cx, settings: &ReaderSettings) {
        let palette = reader_scene_palette(settings.background, settings.theme);
        let cells = [
            id!(reader_customise_highlight_yellow),
            id!(reader_customise_highlight_green),
            id!(reader_customise_highlight_blue),
            id!(reader_customise_highlight_pink),
        ];
        for (id, slot) in cells.into_iter().zip(crate::mark_span::MARKER_SLOTS) {
            self.paint_color_swatch(
                cx,
                id,
                ColorKind::Highlight.color(slot, palette.text),
                settings.highlight_color_slot == slot,
                palette,
            );
        }
        self.ui
            .widget(cx, ids!(reader_customise_highlight_name))
            .set_text(
                cx,
                ColorKind::Highlight.name(settings.highlight_color_slot, app_language()),
            );
        for id in [
            id!(reader_customise_highlight_label),
            id!(reader_customise_highlight_name),
        ] {
            apply_reader_label_palette(cx, self.ui.widget(cx, &[id]), palette.muted);
        }
    }

    fn sync_reader_customise_controls(&self, cx: &mut Cx) {
        let settings = self
            .reader_customise_draft
            .clone()
            .unwrap_or_else(current_reader_settings);
        let background = settings.background;
        let palette = reader_scene_palette(background, settings.theme);
        self.sync_reader_customise_highlighter(cx, &settings);
        self.sync_reader_customise_annotation_colors(cx, &settings);
        let font = settings.font;
        let columns = settings.columns;
        let spacing = settings.spacing;
        let size = settings.font_size.unwrap_or(READER_FONT_DEFAULT);
        let character = settings.character_spacing;
        let word = settings.word_spacing;
        let margin = settings.margin;
        self.ui
            .label(cx, ids!(reader_customise_size_value))
            .set_text(cx, &format!("{size:.0}"));
        self.ui
            .slider(cx, ids!(reader_customise_size_slider))
            .set_value(cx, size as f64);
        self.ui
            .label(cx, ids!(reader_customise_line_value))
            .set_text(cx, &format!("{:.2}×", spacing.plain_line_spacing()));
        self.ui
            .label(cx, ids!(reader_customise_character_value))
            .set_text(cx, &format!("{:.0}%", character * 100.0));
        self.ui
            .label(cx, ids!(reader_customise_word_value))
            .set_text(cx, &format!("{:.0}%", word * 100.0));
        self.ui
            .label(cx, ids!(reader_customise_margin_value))
            .set_text(cx, &format!("{margin:.0}%"));
        self.ui
            .slider(cx, ids!(reader_customise_line_slider))
            .set_value(cx, spacing.plain_line_spacing() as f64);
        self.ui
            .slider(cx, ids!(reader_customise_character_slider))
            .set_value(cx, (character * 100.0).round() as f64);
        self.ui
            .slider(cx, ids!(reader_customise_word_slider))
            .set_value(cx, (word * 100.0).round() as f64);
        self.ui
            .slider(cx, ids!(reader_customise_margin_slider))
            .set_value(cx, margin as f64);
        self.ui
            .check_box(cx, ids!(reader_customise_bold))
            .set_active(cx, settings.bold_text, Animate::No);
        self.ui
            .check_box(cx, ids!(reader_customise_justify))
            .set_active(cx, settings.justify_text, Animate::No);
        self.ui
            .check_box(cx, ids!(reader_customise_gloss_enable))
            .set_active(cx, settings.gloss_enabled, Animate::No);
        self.ui
            .label(cx, ids!(reader_customise_gloss_size_value))
            .set_text(cx, &format!("{:.1}", settings.gloss_font_size));
        self.ui
            .slider(cx, ids!(reader_customise_gloss_size_slider))
            .set_value(cx, settings.gloss_font_size as f64);
        for id in [
            id!(reader_customise_gloss_size_row),
            id!(reader_customise_gloss_color_row),
            id!(reader_customise_gloss_bg_row),
            id!(reader_customise_word_color_row),
            id!(reader_customise_word_bg_row),
        ] {
            self.ui
                .widget(cx, &[id])
                .set_visible(cx, settings.gloss_enabled);
        }
        self.sync_reader_customise_font_labels(cx, app_language(), font);
        self.ui
            .drop_down(cx, ids!(reader_customise_columns_choice))
            .set_selected_item(
                cx,
                match columns {
                    ReaderColumns::One => 0,
                    ReaderColumns::Two => 1,
                    ReaderColumns::Auto => 2,
                },
            );
        apply_reader_surface_palette(
            cx,
            self.ui.widget(cx, ids!(reader_preview)),
            palette.surface,
            palette.border,
        );
        apply_reader_label_palette(
            cx,
            self.ui.widget(cx, ids!(reader_preview_heading)),
            palette.text,
        );
        let two_columns = reader_customise_rect(self.responsive_window_size, self.safe_area_insets)
            .size
            .x
            >= 500.0
            && match columns {
                ReaderColumns::One => false,
                ReaderColumns::Two => true,
                ReaderColumns::Auto => self
                    .ui
                    .widget(cx, ids!(chapter_list))
                    .borrow::<ChapterList>()
                    .is_some_and(|reader| reader.pager.layout.count > 1),
            };
        self.ui
            .widget(cx, ids!(reader_preview_second_column))
            .set_visible(cx, two_columns);
        crate::ruby_word::set_preview_appearance(&settings);
        let (first_body, second_body) =
            sample_chapter::build_glossed_preview(settings.gloss_enabled, two_columns);
        if let Some(mut columns_view) = self
            .ui
            .widget(cx, ids!(reader_preview_columns))
            .borrow_mut::<View>()
        {
            columns_view.layout.padding.top = 0.0;
            columns_view.redraw(cx);
        }
        if let Some(mut preview) = self
            .ui
            .widget(cx, ids!(reader_preview))
            .borrow_mut::<View>()
        {
            let inset = (ReaderSettings::margin_pixels(margin) * 0.3).clamp(8.0, 24.0) as f64;
            preview.layout.padding = Inset {
                left: inset,
                right: inset,
                top: 12.0,
                bottom: 12.0,
            };
            preview.redraw(cx);
        }
        let preview_families = self
            .ui
            .widget(cx, ids!(chapter_list))
            .borrow::<ChapterList>()
            .map(|reader| match font == ReaderFont::Original {
                false => [
                    reader.font_preview_normal.font_family.clone(),
                    reader.font_preview_bold.font_family.clone(),
                    reader.font_preview_italic.font_family.clone(),
                    reader.font_preview_bold_italic.font_family.clone(),
                ],
                true => [
                    reader.font_standard_normal.font_family.clone(),
                    reader.font_standard_bold.font_family.clone(),
                    reader.font_standard_italic.font_family.clone(),
                    reader.font_standard_bold_italic.font_family.clone(),
                ],
            });
        for (id, body) in [
            (id!(reader_preview_text), first_body.as_str()),
            (id!(reader_preview_text_second), second_body.as_str()),
        ] {
            if let Some(mut html) = self.ui.widget(cx, &[id]).borrow_mut::<Html>() {
                if html.body.as_ref() != body {
                    html.text_flow.clear_items();
                    html.set_text(cx, body);
                }
                let flow = &mut html.text_flow;
                flow.font_size = size;
                flow.font_color = palette.text;
                flow.draw_text.color = palette.text;
                let line_spacing = spacing.plain_line_spacing();
                for (index, style) in [
                    &mut flow.text_style_normal,
                    &mut flow.text_style_bold,
                    &mut flow.text_style_italic,
                    &mut flow.text_style_bold_italic,
                ]
                .into_iter()
                .enumerate()
                {
                    style.line_spacing = line_spacing;
                    style.letter_spacing = character;
                    style.word_spacing = word;
                    style.justify = settings.justify_text;
                    if let Some(families) = &preview_families {
                        let index = if settings.bold_text && index == 0 {
                            1
                        } else if settings.bold_text && index == 2 {
                            3
                        } else {
                            index
                        };
                        style.font_family = families[index].clone();
                    }
                }
                html.redraw(cx);
            }
        }
    }

    fn apply_reader_background(&self, cx: &mut Cx, background: ReaderBackground) {
        let palette = reader_scene_palette(background, *READER_THEME.read().unwrap());
        for id in [
            id!(library_tag_filter),
            id!(library_view_list),
            id!(library_view_grid),
            id!(import_epub_btn),
        ] {
            apply_library_control_palette(cx, self.ui.widget(cx, &[id]), palette);
        }

        for id in [
            id!(main_column),
            id!(page_reader),
            id!(reader_body),
            id!(reader_surface),
            id!(page_chat),
            id!(page_skills),
            id!(page_vocab),
            id!(page_statistics),
            id!(page_profile),
            id!(page_notes),
            id!(page_reading_home),
            id!(page_settings),
        ] {
            apply_reader_surface_palette(
                cx,
                self.ui.widget(cx, &[id]),
                palette.surface,
                palette.surface,
            );
        }
        // These pages are built from reusable Makepad widgets whose initial
        // draw colors are resolved before the saved reading background is
        // loaded. Re-apply their surfaces here so no stale white/gray panels
        // remain after navigation or a background change.
        for id in [
            id!(vocab_review_panel),
            id!(profile_tools),
            id!(reading_continue_card),
            id!(stat_today_card),
            id!(stat_total_card),
            id!(stat_days_card),
            id!(stat_books_card),
            id!(reading_chart_card),
            id!(vocab_chart_card),
            id!(reading_chart),
            id!(vocab_chart),
            id!(skills_left),
            id!(skill_editor),
        ] {
            apply_reader_surface_palette(
                cx,
                self.ui.widget(cx, &[id]),
                palette.surface,
                palette.border,
            );
        }
        let page_button_palette = ReaderScenePalette {
            field: palette.surface,
            ..palette
        };
        for id in [
            id!(vocab_start_review),
            id!(vocab_review_close),
            id!(vocab_review_speak),
            id!(vocab_review_reveal),
            id!(vocab_review_again),
            id!(vocab_review_known),
            id!(statistics_refresh),
            id!(skill_new),
            id!(skill_active_edit),
            id!(skill_cancel),
            id!(skill_save),
            id!(chat_send),
            id!(chat_cancel),
            id!(chat_clear),
        ] {
            apply_reader_button_palette(cx, self.ui.widget(cx, &[id]), page_button_palette);
        }
        for id in [
            id!(vocab_filter_all),
            id!(vocab_filter_learning),
            id!(vocab_filter_mastered),
        ] {
            apply_library_control_palette(cx, self.ui.widget(cx, &[id]), palette);
        }
        // Settings contains many RadioButtonTabFlat/CheckBox controls whose
        // active state otherwise falls back to the default white widget skin.
        // Keep every setting choice in the same reading-surface scene.
        for id in [
            id!(language_chinese),
            id!(language_english),
            id!(theme_dark),
            id!(theme_light),
            id!(reader_background_paper),
            id!(reader_background_sepia),
            id!(reader_background_sage),
            id!(reader_background_night),
            id!(reader_columns_one),
            id!(reader_columns_two),
            id!(reader_columns_auto),
            id!(reader_turn_curl),
            id!(reader_turn_slide),
            id!(reader_turn_scroll),
            id!(reader_background_theme),
            id!(reader_font_choice),
            id!(reader_spacing_default),
            id!(reader_spacing_comfortable),
            id!(reader_spacing_relaxed),
            id!(gloss_enable),
            id!(local_ai_auto_start),
        ] {
            apply_library_control_palette(cx, self.ui.widget(cx, &[id]), palette);
        }
        for id in [
            id!(profile_title),
            id!(reading_home_title),
            id!(reading_continue_title),
            id!(profile_detail_title),
            id!(all_notes_status),
        ] {
            apply_reader_label_palette(cx, self.ui.widget(cx, &[id]), palette.text);
        }
        for id in [
            id!(profile_hint),
            id!(reading_home_hint),
            id!(reading_continue_hint),
            id!(profile_status),
        ] {
            apply_reader_label_palette(cx, self.ui.widget(cx, &[id]), palette.muted);
        }
        for id in [
            id!(profile_notes),
            id!(profile_reading),
            id!(profile_completed),
            id!(profile_duration),
        ] {
            let metric = self.ui.widget(cx, &[id]);
            apply_reader_button_palette(cx, metric.widget(cx, ids!(metric_button)), palette);
            apply_reader_icon_palette(cx, metric.widget(cx, ids!(metric_icon)), palette.accent);
            apply_reader_label_palette(cx, metric.widget(cx, ids!(metric_title)), palette.text);
            apply_reader_label_palette(
                cx,
                metric.widget(cx, ids!(metric_description)),
                palette.muted,
            );
        }
        for id in [
            id!(profile_chat),
            id!(profile_skills),
            id!(profile_settings),
            id!(reading_continue),
            id!(reading_browse),
            id!(profile_back),
        ] {
            let widget = self.ui.widget(cx, &[id]);
            apply_reader_button_palette(cx, widget.clone(), palette);
            apply_reader_icon_palette(cx, widget, palette.accent);
        }

        for id in [
            id!(profile_chat),
            id!(profile_skills),
            id!(profile_settings),
            id!(reading_browse),
            id!(profile_back),
        ] {
            let widget = self.ui.widget(cx, &[id]);
            apply_flat_entry_background(cx, widget, palette);
        }

        for id in [
            id!(font_dec),
            id!(font_inc),
            id!(gloss_dec),
            id!(gloss_inc),
            id!(ai_save),
            id!(reload_button),
            id!(local_ai_start),
            id!(local_ai_save),
            id!(local_ai_stop),
        ] {
            apply_reader_button_palette(cx, self.ui.widget(cx, &[id]), page_button_palette);
        }
        for id in [
            id!(vocab_search),
            id!(chat_input),
            id!(skill_name_input),
            id!(skill_content_input),
            id!(ai_endpoint),
            id!(ai_model),
            id!(ai_api_key),
            id!(local_ai_binary),
            id!(local_ai_model_path),
            id!(local_ai_port),
            id!(local_ai_context),
            id!(local_ai_threads),
            id!(local_ai_gpu_layers),
        ] {
            apply_reader_input_palette(cx, self.ui.widget(cx, &[id]), page_button_palette);
        }
        apply_reader_surface_palette(
            cx,
            self.ui.widget(cx, ids!(header)),
            palette.surface,
            palette.border,
        );
        apply_reader_surface_palette(
            cx,
            self.ui.widget(cx, ids!(reader_ai_panel)),
            palette.surface,
            palette.border,
        );
        for id in [
            id!(note_composer),
            id!(word_card),
            id!(indicator_bg),
            id!(library_rename_card),
            id!(reader_customise_card),
        ] {
            apply_reader_surface_palette(
                cx,
                self.ui.widget(cx, &[id]),
                palette.surface,
                palette.border,
            );
        }
        // Reader popovers use the same fill as the current reading page;
        // their border and shadow still separate them from the text below.
        for id in [
            id!(toc_panel),
            id!(reader_bookmarks_panel),
            id!(note_panel),
            id!(reader_search_panel),
            id!(reader_theme_menu),
            id!(toc_arrow),
            id!(reader_bookmarks_arrow),
            id!(note_arrow),
            id!(reader_theme_arrow),
        ] {
            apply_reader_surface_palette(
                cx,
                self.ui.widget(cx, &[id]),
                palette.surface,
                palette.border,
            );
        }
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
            id!(mobile_reader_title),
            id!(titlebar_reader_title),
            id!(status),
            id!(toc_title),
            id!(note_panel_title),
            id!(reader_highlights_title),
            id!(note_empty_title),
            id!(reader_ai_title),
            id!(reader_bookmarks_title),
            id!(reader_search_title),
            id!(card_word),
            id!(library_rename_title),
            id!(reader_turn_mode_label),
            id!(reader_theme_title),
            id!(reader_customise_title),
            id!(reader_customise_text_title),
            id!(reader_customise_gloss_title),
            id!(reader_customise_layout_title),
            id!(reader_customise_columns_title),
            id!(reader_customise_size_value),
            id!(reader_customise_gloss_size_value),
            id!(reader_customise_line_value),
            id!(reader_customise_character_value),
            id!(reader_customise_word_value),
            id!(reader_customise_margin_value),
        ] {
            apply_reader_label_palette(cx, self.ui.widget(cx, &[id]), palette.text);
        }
        for id in [
            id!(chapter_pos),
            id!(page_text),
            id!(reader_pages_left),
            id!(toc_loading),
            id!(note_hint),
            id!(note_empty_hint),
            id!(reader_chat_status),
            id!(reader_bookmarks_hint),
            id!(reader_search_hint),
            id!(sel_hint),
            id!(card_hint),
            id!(library_rename_status),
            id!(reader_scroll_hint),
            id!(gloss_color_title),
            id!(gloss_color_name),
            id!(gloss_bg_title),
            id!(gloss_bg_name),
            id!(word_color_title),
            id!(word_color_name),
            id!(word_bg_title),
            id!(word_bg_name),
            id!(selection_color_title),
            id!(selection_color_name),
            id!(highlight_color_title),
            id!(highlight_color_name),
            id!(reader_customise_font_label),
            id!(reader_customise_size_label),
            id!(reader_customise_gloss_size_label),
            id!(reader_customise_gloss_color_label),
            id!(reader_customise_gloss_bg_label),
            id!(reader_customise_word_color_label),
            id!(reader_customise_word_bg_label),
            id!(reader_customise_line_label),
            id!(reader_customise_character_label),
            id!(reader_customise_word_label),
            id!(reader_customise_margin_label),
        ] {
            apply_reader_label_palette(cx, self.ui.widget(cx, &[id]), palette.muted);
        }
        for id in [id!(note_quote_title), id!(card_gloss)] {
            apply_reader_label_palette(cx, self.ui.widget(cx, &[id]), palette.accent);
        }

        for id in [
            id!(toc_btn),
            id!(toc_close_btn),
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
            id!(reader_back_btn),
            id!(reader_bookmarks_btn),
            id!(reader_bookmarks_close),
            id!(reader_appearance_btn),
            id!(reader_search_btn),
            id!(reader_search_close),
            id!(reader_search_submit),
            id!(reader_bookmark_btn),
            id!(reader_bookmark_filled_btn),
            id!(reader_customise_btn),
            id!(reader_customise_close),
            id!(reader_customise_cancel),
            id!(reader_customise_save),
            id!(reader_customise_font_choice),
            id!(reader_customise_columns_choice),
            id!(reader_customise_reset),
        ] {
            apply_reader_button_palette(cx, self.ui.widget(cx, &[id]), palette);
        }
        for id in [
            id!(reader_back_btn),
            id!(toc_btn),
            id!(reader_bookmarks_btn),
            id!(notes_btn),
            id!(reader_appearance_btn),
            id!(reader_search_btn),
            id!(reader_bookmark_btn),
            id!(reader_bookmark_filled_btn),
            id!(titlebar_back_btn),
            id!(titlebar_toc_btn),
            id!(titlebar_bookmarks_btn),
            id!(titlebar_notes_btn),
            id!(titlebar_appearance_btn),
            id!(titlebar_search_btn),
            id!(titlebar_bookmark_btn),
            id!(titlebar_bookmark_filled_btn),
        ] {
            apply_reader_button_palette_with_focus(
                cx,
                self.ui.widget(cx, &[id]),
                palette,
                palette.field,
            );
        }
        for id in [
            id!(reader_back_btn),
            id!(toc_btn),
            id!(reader_bookmarks_btn),
            id!(notes_btn),
            id!(reader_search_btn),
            id!(reader_bookmark_btn),
            id!(reader_bookmark_filled_btn),
            id!(titlebar_back_btn),
            id!(titlebar_toc_btn),
            id!(titlebar_bookmarks_btn),
            id!(titlebar_notes_btn),
            id!(titlebar_search_btn),
            id!(titlebar_bookmark_btn),
            id!(titlebar_bookmark_filled_btn),
            id!(reader_customise_btn),
        ] {
            apply_reader_icon_palette(cx, self.ui.widget(cx, &[id]), palette.text);
        }
        self.sync_reader_mode_controls(cx, background);
        self.sync_reader_quick_colors(cx);
        self.sync_reader_customise_controls(cx);
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
            id!(reader_chat_input),
            id!(reader_search_input),
            id!(library_rename_input),
        ] {
            apply_reader_input_palette(cx, self.ui.widget(cx, &[id]), palette);
        }
        let note_input_palette = ReaderScenePalette {
            field: palette.surface,
            ..palette
        };
        for id in [id!(note_quote), id!(note_extra)] {
            apply_reader_input_palette(cx, self.ui.widget(cx, &[id]), note_input_palette);
        }

        if let Some(mut toc) = self.ui.widget(cx, ids!(toc_list)).borrow_mut::<TocList>() {
            toc.active_bg = palette.field;
            toc.inactive_bg = palette.surface;
            toc.text_color = palette.text;
        }
        for id in [
            id!(reader_bookmark_list),
            id!(reader_search_list),
            id!(reader_highlight_list),
        ] {
            if let Some(mut list) = self.ui.widget(cx, &[id]).borrow_mut::<ReaderJumpList>() {
                list.text_color = palette.text;
                list.redraw(cx);
            }
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

    fn apply_reader_mode(&mut self, cx: &mut Cx, background: ReaderBackground) {
        let bold_text = background == ReaderBackground::Bold;
        let bold_changed = *READER_BOLD_TEXT.read().unwrap() != bold_text;
        *READER_BOLD_TEXT.write().unwrap() = bold_text;
        self.set_reader_background(cx, background);
        if bold_changed {
            self.invalidate_reader_pagination_layout(cx);
            self.persist_reader_settings(cx);
        }
        self.reader_theme_open = false;
        self.ui
            .widget(cx, ids!(reader_theme_menu))
            .set_visible(cx, false);
        self.ui
            .widget(cx, ids!(reader_theme_arrow_row))
            .set_visible(cx, false);
        self.ui.widget(cx, ids!(reader_theme_layer)).redraw(cx);
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
        set_text!(mobile_nav_library, "bookshelf");
        set_text!(mobile_nav_reader, "reader");
        set_text!(mobile_nav_vocab, "words");
        set_text!(mobile_nav_profile, "me");
        set_text!(profile_title, "me");
        for (id, key) in [
            (id!(profile_notes), "notes"),
            (id!(profile_reading), "reading_books"),
            (id!(profile_completed), "completed_books"),
            (id!(profile_duration), "reading_time"),
        ] {
            self.ui
                .widget(cx, &[id])
                .label(cx, ids!(metric_title))
                .set_text(cx, crate::i18n::text(language, key));
        }
        set_text!(profile_back, "me");
        set_text!(profile_hint, "profile_hint");
        self.sync_mobile_chrome(cx);
        set_text!(profile_chat, "chat");
        set_text!(profile_skills, "skills");
        set_text!(profile_settings, "settings");
        set_text!(reading_home_title, "reader");
        set_text!(reading_home_hint, "reading_home_hint");
        set_text!(reading_browse, "browse_books");
        self.sync_reading_home(cx);
        self.sync_all_notes_status(cx);
        if let Some(snapshot) = PROFILE_STATISTICS.read().unwrap().as_ref() {
            self.apply_statistics(cx, snapshot);
        }

        self.ui.widget(cx, ids!(library_title)).set_text(
            cx,
            crate::i18n::text(
                language,
                if self.compact_layout {
                    "bookshelf"
                } else {
                    "library"
                },
            ),
        );
        set_text!(library_view_list, "list_view");
        set_text!(library_view_grid, "grid_view");
        set_text!(import_epub_btn, "import_epub");
        set_text!(library_hint, "library_hint");
        if self.compact_layout {
            self.ui.label(cx, ids!(library_hint)).set_text(
                cx,
                localized(
                    "点击“导入 EPUB”添加书籍。",
                    "Choose Import EPUB to add books.",
                ),
            );
        }
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

        set_text!(toc_title, "contents");
        set_text!(toc_close_btn, "close");
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
        set_text!(reader_theme_title, "reader_themes_settings");
        set_text!(reader_mode_original, "reader_mode_original");
        set_text!(reader_mode_quiet, "reader_mode_quiet");
        set_text!(reader_mode_paper, "reader_mode_paper");
        set_text!(reader_mode_bold, "reader_mode_bold");
        set_text!(reader_mode_calm, "reader_mode_calm");
        set_text!(reader_mode_focus, "reader_mode_focus");
        set_text!(reader_customise_btn, "reader_customise");
        set_text!(reader_quick_pen_label, "reader_pen");
        set_text!(reader_quick_highlight_label, "reader_highlighter");
        set_text!(reader_customise_title, "reader_customise");
        set_text!(reader_preview_heading, "reader_preview");
        set_text!(reader_customise_text_title, "text_section");
        set_text!(reader_customise_font_label, "reader_font");
        set_text!(reader_customise_size_label, "reader_text_size");
        set_text!(reader_customise_bold, "reader_bold_text");
        set_text!(reader_customise_highlight_label, "highlight_color");
        set_text!(reader_customise_gloss_title, "reader_annotations");
        set_text!(reader_customise_gloss_enable, "reader_show_annotations");
        set_text!(reader_customise_gloss_size_label, "reader_annotation_size");
        set_text!(
            reader_customise_gloss_color_label,
            "reader_annotation_color"
        );
        set_text!(
            reader_customise_gloss_bg_label,
            "reader_annotation_background"
        );
        set_text!(reader_customise_word_color_label, "word_color");
        set_text!(reader_customise_word_bg_label, "word_background");
        set_text!(reader_customise_layout_title, "reader_accessibility_layout");
        set_text!(reader_customise_line_label, "reader_line_spacing");
        set_text!(reader_customise_character_label, "reader_character_spacing");
        set_text!(reader_customise_word_label, "reader_word_spacing");
        set_text!(reader_customise_margin_label, "reader_margins");
        set_text!(reader_customise_columns_title, "reader_columns");
        set_text!(reader_customise_justify, "reader_justify_text");
        set_text!(reader_customise_reset, "reader_reset_theme");
        set_text!(reader_customise_cancel, "cancel");
        set_text!(reader_customise_save, "reader_save_apply");
        let columns = self.ui.drop_down(cx, ids!(reader_customise_columns_choice));
        columns.set_labels(
            cx,
            vec![
                "1".to_string(),
                "2".to_string(),
                crate::i18n::text(language, "reader_auto_set").to_string(),
            ],
        );
        columns.set_selected_item(
            cx,
            match self
                .reader_customise_draft
                .as_ref()
                .map(|draft| draft.columns)
                .unwrap_or_else(|| *READER_COLUMNS.read().unwrap())
            {
                ReaderColumns::One => 0,
                ReaderColumns::Two => 1,
                ReaderColumns::Auto => 2,
            },
        );
        self.sync_reader_customise_font_labels(
            cx,
            language,
            self.reader_customise_draft
                .as_ref()
                .map(|draft| draft.font)
                .unwrap_or_else(|| *READER_FONT.read().unwrap()),
        );

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
        self.ui
            .label(cx, ids!(reader_columns_label))
            .set_text(cx, localized("正文分栏", "Columns"));
        for (id, zh, en) in [
            (id!(reader_columns_one), "单列", "1 column"),
            (id!(reader_columns_two), "双列", "2 columns"),
            (id!(reader_columns_auto), "自动", "Auto Set"),
        ] {
            self.ui.widget(cx, &[id]).set_text(cx, localized(zh, en));
        }
        set_text!(font_family_label, "body_font");
        self.ui
            .label(cx, ids!(reader_turn_mode_label))
            .set_text(cx, localized("翻页方式", "Page turns"));
        self.ui.label(cx, ids!(reader_scroll_hint)).set_text(
            cx,
            localized(
                "上下滚动使用单列连续正文，不显示页码。",
                "Vertical scrolling uses one continuous column without page numbers.",
            ),
        );
        for (id, zh, en) in [
            (id!(reader_turn_curl), "仿真翻页", "Page curl"),
            (id!(reader_turn_slide), "左右滑动", "Horizontal slide"),
            (id!(reader_turn_scroll), "上下滚动", "Vertical scroll"),
        ] {
            self.ui.widget(cx, &[id]).set_text(cx, localized(zh, en));
        }
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

        self.refresh_swatch_names(cx, language);
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
        if self.import_dialog_open {
            self.sync_library_import_dialog(cx);
        }
        self.refresh_shelf_projection(cx);
        self.refresh_vocab_projection(cx);
        self.refresh_pending_skill_indicator(cx);
        if matches!(self.active_page, AppPage::Statistics | AppPage::Profile) {
            spawn_load_statistics();
        }
        self.persist_reader_settings(cx);
        self.ui.redraw(cx);
    }

    fn sync_reader_font_controls(&self, cx: &mut Cx, font: ReaderFont) {
        let choice = self.ui.drop_down(cx, ids!(reader_font_choice));
        choice.set_labels(
            cx,
            ReaderFont::ALL
                .iter()
                .map(|font| font.label().to_string())
                .collect(),
        );
        choice.set_selected_item(cx, font.index());
    }

    fn set_reader_font(&mut self, cx: &mut Cx, font: ReaderFont) {
        if *READER_FONT.read().unwrap() == font {
            self.sync_reader_font_controls(cx, font);
            return;
        }
        *READER_FONT.write().unwrap() = font;
        self.load_reader_font(cx, font, false);
        self.sync_reader_font_controls(cx, font);
        self.refresh_current_chapter(cx);
        self.sync_reader_customise_controls(cx);
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
        self.sync_reader_customise_controls(cx);
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

    /// 安全区变化触发 LiveEdit，主题切换触发 ScriptReapply；两者都可能
    /// 覆盖运行时布局。恢复安全区、当前页面和控件投影，业务状态仍由 Rust 持有。
    fn restore_after_script_reload(&mut self, cx: &mut Cx) {
        self.pending_ui_projection_after_draw = true;
        self.active_font_applied = None;
        self.preview_font_applied = None;
        self.reader_arrow_caption_layout = None;
        self.activate_page(cx, self.active_page);
        self.pending_customise_preview_after_draw = self.reader_customise_draft.is_some();
        self.responsive_layout_initialized = false;
        self.update_responsive_layout(
            cx,
            cx.display_context.screen_size,
            cx.display_context.safe_area_insets,
        );
        self.apply_reader_settings(cx, current_reader_settings());
        self.ui
            .widget(cx, ids!(reader_theme_arrow_row))
            .set_visible(cx, self.on_reader_page && self.reader_theme_open);
        self.ui
            .widget(cx, ids!(reader_theme_menu))
            .set_visible(cx, self.on_reader_page && self.reader_theme_open);
        self.sync_reader_panel_layout(cx);
        if self.on_reader_page {
            let title = CURRENT_BOOK
                .read()
                .unwrap()
                .as_ref()
                .map(|book| book.title.clone());
            if let Some(title) = title {
                self.set_reader_title_text(cx, &format!("《{title}》"));
                self.refresh_bookmark_button(cx);
            }
        }

        self.refresh_shelf_projection(cx);
        if self.import_dialog_open {
            self.sync_library_import_dialog(cx);
        }

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
        if matches!(self.active_page, AppPage::Statistics | AppPage::Profile) {
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
        if self.reader_theme_open || READER_CUSTOMISE_OPEN.load(Ordering::Acquire) {
            self.reader_swipe.cancel();
            return None;
        }
        let paged = READER_TURN_MODE.read().unwrap().is_paged();
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
                    let can_begin = !self.virtual_keyboard_open
                        && self.on_reader_page
                        && !self.toc_open
                        && !self.note_open
                        && !self.bookmarks_open
                        && !self.reader_ai_open
                        && SEL_RANGE.read().unwrap().is_none()
                        && SELECTED_TEXT.read().unwrap().is_empty()
                        && chapter_rect.contains(touch.abs);
                    if can_begin {
                        self.reader_swipe.begin(touch.uid, touch.abs, event.time);
                        if !paged {
                            // Only a new swipe that STARTS at a chapter edge
                            // crosses it. Native drag/inertia within the chapter
                            // must not skip its final or first screen.
                            let list = self
                                .ui
                                .widget(cx, ids!(chapter_list))
                                .portal_list(cx, ids!(list));
                            self.reader_swipe.vertical = true;
                            self.reader_swipe.allow_next = list.is_at_end();
                            self.reader_swipe.allow_previous =
                                list.first_id() == 0 && list.scroll_position() >= -0.5;
                        }
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
        self.ui.label(cx, ids!(library_title)).set_text(
            cx,
            if self.compact_layout {
                tr("bookshelf")
            } else {
                tr("library")
            },
        );
        let detail_key = match self.active_page {
            AppPage::Chat => Some("chat"),
            AppPage::Skills => Some("skills"),
            AppPage::Statistics => Some("statistics"),
            AppPage::Settings => Some("settings"),
            AppPage::Notes => Some("notes"),
            _ => None,
        };
        self.ui
            .widget(cx, ids!(profile_back_bar))
            .set_visible(cx, self.compact_layout && detail_key.is_some());
        if let Some(key) = detail_key {
            self.ui
                .label(cx, ids!(profile_detail_title))
                .set_text(cx, tr(key));
        }
        self.ui.widget(cx, ids!(mobile_nav)).set_visible(
            cx,
            !self.on_reader_page
                && should_show_mobile_nav(self.compact_layout, self.virtual_keyboard_open),
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
        let panel_open = self.reader_ai_open;
        READER_INFO_POPUP_OPEN.store(
            self.on_reader_page
                && (self.toc_open || self.bookmarks_open || self.note_open || self.search_open),
            Ordering::Relaxed,
        );
        let surface_visible = !self.compact_layout || !panel_open;
        let reader_layout_changed = self.ui.widget(cx, ids!(reader_surface)).visible()
            != surface_visible
            || self.ui.widget(cx, ids!(reader_ai_panel)).visible() != self.reader_ai_open;
        let toc_ready = requested_book_is_ready();
        if let Some(mut chapter_list) = self
            .ui
            .widget(cx, ids!(chapter_list))
            .borrow_mut::<ChapterList>()
        {
            chapter_list.set_compact_layout(cx, self.compact_layout);
        }
        self.ui
            .widget(cx, ids!(reader_surface))
            .set_visible(cx, surface_visible);
        self.ui
            .widget(cx, ids!(toc_popup))
            .set_visible(cx, self.toc_open);
        self.ui
            .widget(cx, ids!(toc_list))
            .set_visible(cx, toc_ready);
        self.ui
            .widget(cx, ids!(toc_loading))
            .set_visible(cx, !toc_ready);
        self.ui
            .label(cx, ids!(toc_loading))
            .set_text(cx, localized("目录加载中…", "Loading contents…"));
        self.ui
            .widget(cx, ids!(note_popup))
            .set_visible(cx, self.note_open);
        self.ui
            .widget(cx, ids!(reader_bookmarks_popup))
            .set_visible(cx, self.bookmarks_open);
        self.ui
            .widget(cx, ids!(reader_bookmarks_hint))
            .set_visible(cx, READER_BOOKMARKS.read().unwrap().is_empty());
        self.ui
            .widget(cx, ids!(reader_search_popup_slot))
            .set_visible(cx, self.search_open);
        self.ui
            .widget(cx, ids!(reader_highlights_title))
            .set_visible(
                cx,
                self.note_open && !READER_HIGHLIGHT_JUMPS.read().unwrap().is_empty(),
            );
        self.ui.widget(cx, ids!(reader_highlight_list)).set_visible(
            cx,
            self.note_open && !READER_HIGHLIGHT_JUMPS.read().unwrap().is_empty(),
        );
        self.ui
            .widget(cx, ids!(reader_ai_panel))
            .set_visible(cx, self.reader_ai_open);

        let widths = if self.compact_layout {
            [Size::fill(); 5]
        } else {
            [
                Size::Fixed(320.0),
                Size::Fixed(340.0),
                Size::Fixed(380.0),
                Size::Fixed(340.0),
                Size::Fixed(360.0),
            ]
        };
        for (widget, width) in [
            self.ui.widget(cx, ids!(toc_popup)),
            self.ui.widget(cx, ids!(reader_bookmarks_popup)),
            self.ui.widget(cx, ids!(note_popup)),
            self.ui.widget(cx, ids!(reader_search_panel)),
            self.ui.widget(cx, ids!(reader_ai_panel)),
        ]
        .into_iter()
        .zip(widths)
        {
            if let Some(mut inner) = widget.borrow_mut::<View>() {
                inner.walk.width = width;
            }
        }
        if let Some(mut panel) = self
            .ui
            .widget(cx, ids!(reader_ai_panel))
            .borrow_mut::<View>()
        {
            let safe = if self.compact_layout {
                self.safe_area_insets
            } else {
                SafeAreaInsets::default()
            };
            // The parent already reserves the top safe area. The full-width
            // AI panel must reserve the side and bottom areas itself.
            panel.layout.padding = Inset {
                left: 12.0 + safe.left,
                right: 12.0 + safe.right,
                top: 12.0,
                bottom: 12.0 + safe.bottom,
            };
        }
        self.ui.widget(cx, ids!(reader_panel_layer)).redraw(cx);
        // Floating contents, bookmarks, notes and search leave the reader's
        // available width alone, so opening them must not repaginate the book.
        if reader_layout_changed {
            self.invalidate_reader_pagination_layout(cx);
        }
    }

    fn close_reader_info_popup(&mut self, cx: &mut Cx) {
        if !self.toc_open && !self.bookmarks_open && !self.note_open && !self.search_open {
            return;
        }
        self.toc_open = false;
        self.bookmarks_open = false;
        self.note_open = false;
        self.search_open = false;
        self.pending_toc_first_draw = false;
        self.sync_reader_panel_layout(cx);
    }

    fn finish_toc_first_draw(&mut self, cx: &mut Cx) {
        if !std::mem::take(&mut self.pending_toc_first_draw)
            || !self.on_reader_page
            || !self.toc_open
        {
            return;
        }
        let panel = self.ui.widget(cx, ids!(toc_panel));
        let list = self.ui.widget(cx, ids!(toc_list));
        if panel.borrow::<View>().is_none_or(|view| !view.visible)
            || panel.area().is_empty()
            || (requested_book_is_ready() && list.area().is_empty())
        {
            self.sync_reader_panel_layout(cx);
            self.ui.widget(cx, ids!(reader_panel_layer)).redraw(cx);
        }
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
        let size_changed = self.responsive_window_size != size;
        let safe_area_changed = safe_area_insets != self.safe_area_insets;
        if !layout_changed && !safe_area_changed && !size_changed {
            return false;
        }

        self.responsive_layout_initialized = true;
        self.responsive_window_size = size;
        self.compact_layout = compact;
        self.safe_area_insets = safe_area_insets;
        self.layout_reader_image_viewer(cx);
        self.ui
            .widget(cx, ids!(sidebar))
            .set_visible(cx, !compact && !self.on_reader_page);
        self.sync_mobile_chrome(cx);
        self.ui
            .widget(cx, ids!(mobile_safe_top))
            .set_visible(cx, compact && safe_area_insets.top > 0.0);
        self.ui.widget(cx, ids!(title)).set_visible(cx, true);
        self.ui.widget(cx, ids!(status)).set_visible(cx, !compact);
        self.ui
            .widget(cx, ids!(reader_footer_spacer))
            .set_visible(cx, compact);
        self.configure_reader_toolbar_layout(cx, size);
        self.configure_reader_customise_layout(cx);
        self.configure_library_import_layout(cx);
        if self.reader_customise_draft.is_some() {
            self.close_reader_customise_dropdowns(cx);
            self.sync_reader_customise_controls(cx);
        }
        self.set_reader_toolbar_visible(cx, self.on_reader_page && self.reader_toolbar_visible);
        self.sync_reader_toolbar_placement(cx);

        for id in [
            id!(chat_title),
            id!(skills_title),
            id!(statistics_title),
            id!(settings_title),
        ] {
            self.ui.widget(cx, &[id]).set_visible(cx, !compact);
        }
        let mut skills_body = self.ui.widget(cx, ids!(skills_body));
        let skills_flow = if compact { Flow::Down } else { Flow::right() };
        script_apply_eval!(cx, skills_body, {flow: #(skills_flow)});
        let mut skills_list = self.ui.widget(cx, ids!(skills_left));
        let list_width = if compact {
            Size::fill()
        } else {
            Size::Fixed(330.0)
        };
        let list_height = if compact {
            Size::Fixed(240.0)
        } else {
            Size::fill()
        };
        script_apply_eval!(cx, skills_list, {width: #(list_width) height: #(list_height)});
        let mut editor = self.ui.widget(cx, ids!(skill_editor));
        let editor_height = if compact {
            Size::Fixed(560.0)
        } else {
            Size::fill()
        };
        script_apply_eval!(cx, editor, {height: #(editor_height)});
        let mut shelf_header = self.ui.widget(cx, ids!(lib_title_row));
        let shelf_header_flow = if compact { Flow::Down } else { Flow::right() };
        script_apply_eval!(cx, shelf_header, {flow: #(shelf_header_flow)});
        let mut filters = self.ui.widget(cx, ids!(library_filter_row));
        let filter_flow = if compact { Flow::Down } else { Flow::right() };
        script_apply_eval!(cx, filters, {flow: #(filter_flow)});
        for id in [id!(vocab_header), id!(vocab_toolbar)] {
            let mut row = self.ui.widget(cx, &[id]);
            script_apply_eval!(cx, row, {flow: #(filter_flow)});
        }
        let mut vocab_filters = self.ui.widget(cx, ids!(vocab_filters));
        let vocab_filters_width = if compact { Size::fill() } else { Size::fit() };
        script_apply_eval!(cx, vocab_filters, {width: #(vocab_filters_width)});
        let mut dropdown = self.ui.widget(cx, ids!(library_tag_filter));
        let filter_width = if compact {
            Size::fill()
        } else {
            Size::Fixed(180.0)
        };
        script_apply_eval!(cx, dropdown, {width: #(filter_width)});
        let mut summary = self.ui.widget(cx, ids!(statistics_summary));
        let summary_flow = if compact { Flow::Down } else { Flow::right() };
        script_apply_eval!(cx, summary, {flow: #(summary_flow)});
        let safe_top = safe_area_insets.top;
        let mobile_nav_padding = Inset {
            left: safe_area_insets.left + 4.0,
            right: safe_area_insets.right + 4.0,
            top: 5.0,
            bottom: safe_area_insets.bottom + 5.0,
        };
        let mut mobile_safe_top = self.ui.widget(cx, ids!(mobile_safe_top));
        let mut mobile_nav = self.ui.widget(cx, ids!(mobile_nav));
        script_apply_eval!(cx, mobile_safe_top, {height: #(safe_top)});
        script_apply_eval!(cx, mobile_nav, {padding: #(mobile_nav_padding)});
        let content_padding = Inset {
            left: safe_area_insets.left + 20.0,
            right: safe_area_insets.right + 20.0,
            top: 20.0,
            bottom: 20.0,
        };
        for id in [id!(page_profile), id!(page_reading_home)] {
            let mut page = self.ui.widget(cx, &[id]);
            script_apply_eval!(cx, page, {padding: #(content_padding)});
        }
        let detail_padding = Inset {
            left: safe_area_insets.left + 12.0,
            right: safe_area_insets.right + 16.0,
            top: 4.0,
            bottom: 4.0,
        };
        let mut detail = self.ui.widget(cx, ids!(profile_back_bar));
        script_apply_eval!(cx, detail, {padding: #(detail_padding)});
        let side_padding = Inset {
            left: safe_area_insets.left,
            right: safe_area_insets.right,
            top: 0.0,
            bottom: 0.0,
        };
        for id in [
            id!(page_notes),
            id!(page_library),
            id!(page_chat),
            id!(page_skills),
        ] {
            let mut page = self.ui.widget(cx, &[id]);
            script_apply_eval!(cx, page, {padding: #(side_padding)});
        }
        let scroll_padding = Inset {
            left: safe_area_insets.left + 24.0,
            right: safe_area_insets.right + 24.0,
            top: 24.0,
            bottom: 24.0,
        };
        for id in [id!(page_settings), id!(page_statistics), id!(page_vocab)] {
            let mut page = self.ui.widget(cx, &[id]);
            script_apply_eval!(cx, page, {padding: #(scroll_padding)});
        }
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

    fn start_reading_timer(&mut self) {
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
            spawn_finish_reading_session(id, load_statistics || self.compact_layout);
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
        for (id, text) in [
            (
                id!(profile_notes),
                if english {
                    format!("{} total", snapshot.total_notes)
                } else {
                    format!("累计 {} 条", snapshot.total_notes)
                },
            ),
            (
                id!(profile_reading),
                if english {
                    format!(
                        "{} {}",
                        snapshot.reading_books,
                        if snapshot.reading_books == 1 {
                            "book"
                        } else {
                            "books"
                        }
                    )
                } else {
                    format!("累计 {} 本", snapshot.reading_books)
                },
            ),
            (
                id!(profile_completed),
                if english {
                    format!(
                        "{} {}",
                        snapshot.completed_books,
                        if snapshot.completed_books == 1 {
                            "book"
                        } else {
                            "books"
                        }
                    )
                } else {
                    format!("累计 {} 本", snapshot.completed_books)
                },
            ),
            (
                id!(profile_duration),
                Self::format_reading_duration(snapshot.total_reading_seconds),
            ),
        ] {
            self.ui
                .widget(cx, &[id])
                .label(cx, ids!(metric_description))
                .set_text(cx, &text);
        }
        self.ui.label(cx, ids!(profile_status)).set_text(cx, "");
        self.ui.widget(cx, ids!(page_profile)).redraw(cx);
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
                "Statistics updated"
            } else {
                "统计已更新"
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
        let search = self.ui.text_input(cx, ids!(vocab_search));
        if search.text() != self.vocab_search {
            search.set_text(cx, &self.vocab_search);
        }
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
            self.bookmarks_open = false;
            self.search_open = false;
        }
        self.sync_reader_panel_layout(cx);
        self.ui.redraw(cx);
    }

    fn refresh_reader_highlights(&self, cx: &mut Cx) {
        let hits = {
            let guard = CURRENT_BOOK.read().unwrap();
            let Some(book) = guard.as_ref() else { return };
            let mut chapters: Vec<_> = book.highlights.iter().collect();
            chapters.sort_by_key(|(index, _)| **index);
            let mut hits = Vec::new();
            for (index, highlights) in chapters {
                let Some(html) = book.chapters.get(*index) else {
                    continue;
                };
                let blocks = readwise_core::books::epub_meta::split_blocks(html);
                let mut highlights = highlights.clone();
                highlights.sort_by_key(|highlight| highlight.start_block);
                for highlight in highlights {
                    let excerpt = blocks
                        .get(highlight.start_block)
                        .map(|block| readwise_core::vocab::annotate::rendered_text(block))
                        .unwrap_or_default();
                    let excerpt: String = excerpt
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .chars()
                        .take(100)
                        .collect();
                    hits.push(ReaderSearchHit {
                        chapter: *index,
                        block: highlight.start_block,
                        excerpt: if excerpt.is_empty() {
                            localized("高亮", "Highlight").to_string()
                        } else {
                            excerpt
                        },
                    });
                }
            }
            hits
        };
        let has_highlights = !hits.is_empty();
        *READER_HIGHLIGHT_JUMPS.write().unwrap() = hits;
        self.ui
            .widget(cx, ids!(reader_highlights_title))
            .set_visible(cx, has_highlights);
        self.ui
            .widget(cx, ids!(reader_highlight_list))
            .set_visible(cx, has_highlights);
        self.ui.widget(cx, ids!(reader_highlight_list)).redraw(cx);
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
        let mut data = CHAT_DATA.write().unwrap();
        let revision = data.revision.wrapping_add(1);
        *data = ChatData {
            revision,
            ..Default::default()
        };
        drop(data);
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
        let source = self
            .ui
            .widget(cx, ids!(chapter_list))
            .borrow::<ChapterList>()
            .and_then(|reader| {
                reader
                    .source_anchor(block, scroll)
                    .map(|(_, source)| source)
            });
        CURRENT_READER_BLOCK.store(block, Ordering::Relaxed);
        CURRENT_READER_SCROLL_BITS.store(scroll.to_bits(), Ordering::Relaxed);
        CURRENT_READER_SOURCE.store(source.unwrap_or(usize::MAX), Ordering::Relaxed);
        Some(ReaderProgressSnapshot {
            book_id,
            chapter_count,
            location: ReaderLocation {
                chapter: self.chapter_idx.min(chapter_count.saturating_sub(1)),
                block,
                scroll,
                source,
            },
        })
    }

    fn refresh_bookmark_button(&self, cx: &mut Cx) {
        let Some(snapshot) = self.current_reader_progress_snapshot(cx) else {
            return;
        };
        let marked = READER_BOOKMARKS.read().unwrap().iter().any(|bookmark| {
            bookmark_covers_location(
                ReaderLocation::decode(&bookmark.location, snapshot.chapter_count),
                snapshot.location,
            )
        });
        self.ui
            .widget(cx, ids!(reader_bookmark_btn))
            .set_visible(cx, !marked);
        self.ui
            .widget(cx, ids!(reader_bookmark_filled_btn))
            .set_visible(cx, marked);
        self.ui
            .widget(cx, ids!(titlebar_bookmark_btn))
            .set_visible(cx, !marked);
        self.ui
            .widget(cx, ids!(titlebar_bookmark_filled_btn))
            .set_visible(cx, marked);
    }

    fn toggle_reader_bookmark(&mut self, cx: &mut Cx) {
        let Some(snapshot) = self.current_reader_progress_snapshot(cx) else {
            return;
        };
        let remove_id = READER_BOOKMARKS
            .read()
            .unwrap()
            .iter()
            .find_map(|bookmark| {
                bookmark_covers_location(
                    ReaderLocation::decode(&bookmark.location, snapshot.chapter_count),
                    snapshot.location,
                )
                .then(|| bookmark.id.clone())
            });
        let excerpt = PARAGRAPHS
            .read()
            .unwrap()
            .get(snapshot.location.block)
            .map(|block| readwise_core::vocab::annotate::rendered_text(block))
            .unwrap_or_default();
        let excerpt: String = excerpt
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(56)
            .collect();
        let label = if excerpt.is_empty() {
            format!("第 {} 章", snapshot.location.chapter + 1)
        } else {
            format!("第 {} 章 · {excerpt}", snapshot.location.chapter + 1)
        };
        spawn_bookmark_change(
            snapshot.book_id,
            remove_id,
            snapshot.location.encode(),
            label,
        );
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
        CURRENT_READER_SOURCE.store(usize::MAX, Ordering::Relaxed);
        if let Some(mut reader) = self
            .ui
            .widget(cx, ids!(chapter_list))
            .borrow_mut::<ChapterList>()
        {
            reader.pager.requested_last_page = block == usize::MAX;
            reader.page_turn.finish();
            reader.pager.spread_ready = false;
            reader.pager.pending_source = None;
            reader.pager.requested_source = None;
            reader.pager.pending_turn = None;
            reader.pager.turn_history.clear();
        }
        replace_paragraphs(chunks);
        self.reader_page_span = 0;
        self.pending_reader_pagination_layout = true;
        self.set_reader_title_text(cx, &format!("《{}》", title));
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
                    source: None,
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
        let source = CURRENT_READER_SOURCE.load(Ordering::Relaxed);
        self.set_chapter_at(cx, chapter, first_block, first_scroll, false);
        if source != usize::MAX {
            if let Some(mut reader) = self
                .ui
                .widget(cx, ids!(chapter_list))
                .borrow_mut::<ChapterList>()
            {
                reader.pager.requested_source = Some((first_block, source));
            }
        }
        self.update_progress_label(cx);
    }
    fn update_reader_page_footer(&mut self, cx: &mut Cx) {
        let paged_mode = READER_TURN_MODE.read().unwrap().is_paged();
        self.ui
            .view(cx, ids!(reader_footer))
            .set_visible(cx, paged_mode);
        if !paged_mode {
            return;
        }
        let widget = self.ui.widget(cx, ids!(chapter_list));
        let (visible_pages, summary) = widget
            .borrow::<ChapterList>()
            .map(|reader| {
                let pager = &reader.pager;
                (
                    pager.visible_pages,
                    (pager.active && pager.spread_ready)
                        .then(|| {
                            reader.book_pages.summary(
                                self.chapter_idx,
                                &pager.page_index,
                                pager.anchor,
                                pager.layout.count.max(1),
                            )
                        })
                        .flatten(),
                )
            })
            .unwrap_or((0, None));
        self.ui
            .view(cx, ids!(reader_page_second))
            .set_visible(cx, visible_pages == 2);
        for (offset, id) in [
            id!(reader_page_indicator),
            id!(reader_page_indicator_second),
        ]
        .into_iter()
        .enumerate()
        {
            if let Some(mut indicator) = self
                .ui
                .widget(cx, &[id])
                .borrow_mut::<ReaderPageIndicator>()
            {
                match summary {
                    Some(summary) if offset == 0 || offset < visible_pages => indicator.set_pages(
                        cx,
                        summary.current + offset,
                        summary.total.unwrap_or(0),
                    ),
                    _ => indicator.set_pages(cx, 0, 0),
                }
            }
        }
        let pages_left = match summary.and_then(|summary| summary.pages_left_in_chapter) {
            Some(left) if app_language() == AppLanguage::English => {
                format!(
                    "{} {} left in chapter",
                    left,
                    if left == 1 { "page" } else { "pages" }
                )
            }
            Some(left) => format!("本章还剩 {left} 页"),
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
        if let Some((book_id, key, _)) = reader_book_content_key() {
            if self.reader_pagination_content_key != key {
                self.reader_pagination_content_key = key;
                spawn_build_reader_pagination(book_id, key);
            }
        }
        let paged_signature = self
            .ui
            .widget(cx, ids!(chapter_list))
            .borrow::<ChapterList>()
            .filter(|reader| reader.pager.active)
            .map(|reader| {
                let pager = &reader.pager;
                reader_source_hash(&format!(
                    "{}:{:?}:{:?}:{}:{}:{}:{}:{}",
                    pager.signature,
                    pager.layout,
                    pager.anchor,
                    pager.page_index.starts.len(),
                    pager.page_index.complete,
                    reader.book_pages.revision,
                    pager.visible_pages,
                    pager.spread_ready
                ))
            });
        if let Some(signature) = paged_signature {
            if self.reader_footer_signature != signature {
                self.reader_footer_signature = signature;
                self.update_progress_label(cx);
                self.update_reader_page_footer(cx);
                let _ = self.current_reader_progress_snapshot(cx);
            }
            return;
        }
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
        let read_to = self
            .ui
            .widget(cx, ids!(chapter_list))
            .borrow::<ChapterList>()
            .filter(|reader| reader.pager.active)
            .map(|reader| reader.pager.next.block + usize::from(reader.pager.next.y > 0.0))
            .unwrap_or(list.first_id() + list.visible_items())
            .min(blocks);
        let pct = read_to * 100 / blocks;
        let text = format!("{}/{} · {}%", self.chapter_idx + 1, total_ch, pct);
        self.ui.label(cx, ids!(chapter_pos)).set_text(cx, &text);
        self.update_reader_page_footer(cx);
        self.refresh_bookmark_button(cx);
    }

    /// 视口翻屏；到底/到顶时跨章。
    fn page_scroll(&mut self, cx: &mut Cx, down: bool) {
        if self.reader_theme_open || READER_CUSTOMISE_OPEN.load(Ordering::Acquire) {
            return;
        }
        if self.compact_layout {
            self.set_reader_toolbar_visible(cx, false);
        }
        let chapter_widget = self.ui.widget(cx, ids!(chapter_list));
        let selecting = chapter_widget
            .borrow::<ChapterList>()
            .is_some_and(|reader| reader.selecting);
        if !selecting {
            self.clear_reader_selection(cx);
            self.close_word_card(cx);
        }
        let paged = chapter_widget
            .borrow_mut::<ChapterList>()
            .and_then(|mut reader| reader.pager.active.then(|| reader.turn_page(cx, down)));
        if let Some(turned) = paged {
            if !turned && CURRENT_BOOK.read().unwrap().is_some() {
                let previous_chapter = self.chapter_idx;
                if down {
                    self.set_chapter(cx, self.chapter_idx + 1);
                } else if self.chapter_idx > 0 {
                    self.set_chapter_at(cx, self.chapter_idx - 1, usize::MAX, 0.0, true);
                }
                if self.chapter_idx != previous_chapter && !selecting {
                    if let Some(mut reader) = chapter_widget.borrow_mut::<ChapterList>() {
                        reader.page_turn.request(cx, down);
                    }
                }
            }
            self.update_progress_label(cx);
            self.debounce_reader_progress_save(cx);
            return;
        }
        let list = self
            .ui
            .widget(cx, ids!(chapter_list))
            .portal_list(cx, ids!(list));
        let first = list.first_id();
        let scroll = list.scroll_position();
        let distance = (list.area().rect(cx).size.y * 0.85).max(1.0);
        if down {
            if list.is_at_end() {
                let idx = self.chapter_idx + 1;
                self.set_chapter(cx, idx);
            } else {
                list.set_first_id_and_scroll(first, scroll - distance);
                self.ui.redraw(cx);
            }
        } else if first == 0 && scroll >= -0.5 {
            if self.chapter_idx > 0 {
                let idx = self.chapter_idx - 1;
                self.set_chapter_at(cx, idx, usize::MAX, 0.0, true);
            }
        } else {
            list.set_first_id_and_scroll(first, scroll + distance);
            self.ui.redraw(cx);
        }
        self.update_progress_label(cx);
        self.debounce_reader_progress_save(cx);
    }

    /// 阅读页快捷键：空格/PgDn/→ 下一屏，PgUp/← 上一屏，Shift+←/→ 跨章，Home 回章首。
    fn handle_reader_key(&mut self, cx: &mut Cx, ke: &KeyEvent) {
        if !self.on_reader_page
            || self.reader_theme_open
            || READER_CUSTOMISE_OPEN.load(Ordering::Acquire)
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
                if let Some(mut reader) = self
                    .ui
                    .widget(cx, ids!(chapter_list))
                    .borrow_mut::<ChapterList>()
                {
                    reader.page_turn.cancel();
                    reader.pager.pending_turn = None;
                    reader.pager.turn_history.clear();
                }
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
        self.sync_reader_customise_controls(cx);
        self.persist_reader_settings(cx);
    }

    fn refresh_swatch_names(&self, cx: &mut Cx, language: AppLanguage) {
        for (_, name, kind, slot) in settings_swatch_rows(&current_reader_settings()) {
            self.ui
                .widget(cx, &[name])
                .set_text(cx, kind.name(slot, language));
        }
        self.sync_reader_quick_colors(cx);
    }

    /// Only project the saved colours and selected slots; never reflow a chapter.
    fn refresh_swatches(&self, cx: &mut Cx) {
        let palette = reader_scene_palette(
            *READER_BACKGROUND.read().unwrap(),
            *READER_THEME.read().unwrap(),
        );
        for (cells, name, kind, slot) in settings_swatch_rows(&current_reader_settings()) {
            for (index, cell) in cells.into_iter().enumerate() {
                let option_slot = kind.option_slot(index, slot);
                self.paint_color_swatch(
                    cx,
                    cell,
                    kind.color(option_slot, palette.text),
                    option_slot == slot,
                    palette,
                );
            }
            self.ui
                .widget(cx, &[name])
                .set_text(cx, kind.name(slot, app_language()));
        }
        self.sync_reader_quick_colors(cx);
        if SEL_RANGE.read().unwrap().is_some() {
            if let Some(mut reader) = self
                .ui
                .widget(cx, ids!(chapter_list))
                .borrow_mut::<ChapterList>()
            {
                // Repaint an already-visible selection in its existing geometry.
                // The aA input shield prevents incidental text events from doing
                // this for us; do not invalidate page metrics or reload content.
                reader.page_turn.cancel();
                reader.apply_visible_selection(cx);
            }
        }
    }

    fn apply_reader_settings(&mut self, cx: &mut Cx, settings: ReaderSettings) {
        use crate::ruby_word::*;

        let settings = settings.normalized();
        self.load_reader_font(cx, settings.font, false);
        let scene_changed = *READER_THEME.read().unwrap() != settings.theme
            || *READER_BACKGROUND.read().unwrap() != settings.background;
        *APP_LANGUAGE.write().unwrap() = settings.language;
        *LIBRARY_VIEW_MODE.write().unwrap() = settings.library_view_mode;
        *READER_THEME.write().unwrap() = settings.theme;
        *READER_BACKGROUND.write().unwrap() = settings.background;
        *READER_COLUMNS.write().unwrap() = settings.columns;
        *READER_TURN_MODE.write().unwrap() = settings.turn_mode;
        *READER_FONT.write().unwrap() = settings.font;
        *READER_BOLD_TEXT.write().unwrap() = settings.bold_text;
        *READER_SPACING.write().unwrap() = settings.spacing;
        *READER_CHARACTER_SPACING.write().unwrap() = settings.character_spacing;
        *READER_WORD_SPACING.write().unwrap() = settings.word_spacing;
        *READER_MARGIN.write().unwrap() = settings.margin;
        *READER_JUSTIFY_TEXT.write().unwrap() = settings.justify_text;
        if let Some(mut reader) = self
            .ui
            .widget(cx, ids!(chapter_list))
            .borrow_mut::<ChapterList>()
        {
            reader.apply_reader_margin(cx);
        }
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
        self.sync_reader_columns_controls(cx, settings.columns);
        self.sync_reader_turn_controls(cx, settings.turn_mode);
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
        let location = self
            .current_reader_progress_snapshot(cx)
            .map(|snapshot| snapshot.location);
        let source_text = location.and_then(|location| {
            PARAGRAPHS
                .read()
                .unwrap()
                .get(location.block)
                .map(|text| readwise_core::vocab::annotate::rendered_text(text))
        });
        let chunks = {
            let mut guard = CURRENT_BOOK.write().unwrap();
            guard
                .as_mut()
                .and_then(|b| build_chapter_chunks(b, self.chapter_idx))
        };
        if let Some(chunks) = chunks {
            let resume = location.filter(|location| {
                chunks.get(location.block).is_some_and(|text| {
                    source_text.as_deref()
                        == Some(readwise_core::vocab::annotate::rendered_text(text).as_str())
                })
            });
            replace_paragraphs(chunks);
            if let Some(mut reader) = self
                .ui
                .widget(cx, ids!(chapter_list))
                .borrow_mut::<ChapterList>()
            {
                reader.page_turn.cancel();
                reader.pager.requested_source = resume
                    .and_then(|location| location.source.map(|source| (location.block, source)));
            }
        }
        self.invalidate_reader_pagination_layout(cx);
        self.ui.redraw(cx);
    }

    fn hide_current_word_gloss(&mut self, cx: &mut Cx, lemma: &str) {
        if !silence_current_paragraph_glosses(lemma) {
            return;
        }
        self.redraw_reader_annotations(cx);
    }

    fn redraw_reader_annotations(&self, cx: &mut Cx) {
        let chapter = self.ui.widget(cx, ids!(chapter_list));
        if let Some(mut reader) = chapter.borrow_mut::<ChapterList>() {
            reader.page_turn.cancel();
        }
        chapter.redraw(cx);
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
    fn position_selection_popup(&mut self, cx: &mut Cx, use_measured_height: bool) -> bool {
        let Some(anchor) = self.selection_popup_anchor else {
            return false;
        };
        let mut viewport = self
            .ui
            .widget(cx, ids!(chapter_list))
            .area()
            .clipped_rect(cx);
        if self.compact_layout {
            viewport = reader_overlay_viewport(
                viewport,
                self.responsive_window_size,
                self.safe_area_insets,
            );
        }
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
        // A rotation can change the number of wrapped action rows. Measure
        // again after drawing at the new width before finalizing its position.
        if (old_width - width).abs() > 0.5 {
            self.pending_selection_popup_layout = true;
        }
        drop(inner);
        if changed {
            popup.redraw(cx);
        }
        changed
    }

    /// Card placement uses a font-scaled height estimate on its first frame.
    /// Repositioning after layout made the card jump under the pointer.
    fn position_word_card(&self, cx: &mut Cx) -> bool {
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
        let font_size = READER_FONT_SIZE
            .read()
            .unwrap()
            .unwrap_or(READER_FONT_DEFAULT);
        let card_height = WORD_CARD_ESTIMATED_HEIGHT
            + (font_size as f64 - READER_FONT_DEFAULT as f64).max(0.0) * 5.0;
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
        SELECTION_POPUP_POINTER.store(false, Ordering::Relaxed);
        self.selection_popup_anchor = None;
        self.pending_selection_popup_layout = false;
        self.reset_selection_action_hover(cx);
        self.ui.widget(cx, ids!(sel_bar)).set_visible(cx, false);
        let chapter_list = self.ui.widget(cx, ids!(chapter_list));
        if let Some(mut inner) = chapter_list.borrow_mut::<ChapterList>() {
            inner.cancel_reader_tap(cx);
            inner.reset_selection_gesture();
        }
        let list = chapter_list.portal_list(cx, ids!(list));
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
        self.bookmarks_open = false;
        self.reader_ai_open = false;
        self.search_open = false;
        self.sync_reader_panel_layout(cx);
        self.ui
            .widget(cx, ids!(note_composer))
            .set_visible(cx, true);
        self.ui
            .text_input(cx, ids!(note_quote))
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
        let selection_range = *SEL_RANGE.read().unwrap();
        let book_id = CURRENT_BOOK
            .read()
            .unwrap()
            .as_ref()
            .map(|book| book.id.clone());
        if let (Some(range), Some(book_id)) = (selection_range, book_id) {
            if self.selected_highlight_ids().is_empty() {
                spawn_add_highlight(book_id, self.chapter_idx, range);
            }
        }
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
        let gc = [ids!(gc0), ids!(gc1), ids!(gc2), ids!(gc3)];
        for (i, cell) in gc.into_iter().enumerate() {
            if self.ui.button(cx, cell).clicked(actions) {
                *GLOSS_COLOR.write().unwrap() = GLOSS_PRESETS[i].1;
                changed = true;
            }
        }
        let gb = [ids!(gb0), ids!(gb1), ids!(gb2), ids!(gb3)];
        for (i, cell) in gb.into_iter().enumerate() {
            if self.ui.button(cx, cell).clicked(actions) {
                *GLOSS_BG.write().unwrap() = BG_PRESETS[i].1;
                changed = true;
            }
        }
        let wc = [ids!(wc0), ids!(wc1), ids!(wc2), ids!(wc3)];
        for (i, cell) in wc.into_iter().enumerate() {
            if self.ui.button(cx, cell).clicked(actions) {
                *WORD_COLOR.write().unwrap() = WORD_COLOR_PRESETS[i].1;
                changed = true;
            }
        }
        let wb = [ids!(wb0), ids!(wb1), ids!(wb2), ids!(wb3)];
        for (i, cell) in wb.into_iter().enumerate() {
            if self.ui.button(cx, cell).clicked(actions) {
                *WORD_BG.write().unwrap() = BG_PRESETS[i].1;
                changed = true;
            }
        }
        let sc = [ids!(sc0), ids!(sc1), ids!(sc2), ids!(sc3)];
        for (i, cell) in sc.into_iter().enumerate() {
            if self.ui.button(cx, cell).clicked(actions) {
                let selected = *SELECTION_SLOT.read().unwrap();
                *SELECTION_SLOT.write().unwrap() = ColorKind::Selection.option_slot(i, selected);
                changed = true;
            }
        }
        let hc = [ids!(hc0), ids!(hc1), ids!(hc2), ids!(hc3)];
        for (i, cell) in hc.into_iter().enumerate() {
            if self.ui.button(cx, cell).clicked(actions) {
                let selected = *crate::mark_span::HL_SLOT.read().unwrap();
                *crate::mark_span::HL_SLOT.write().unwrap() =
                    ColorKind::Highlight.option_slot(i, selected);
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
        crate::reader_word::warm_up();
        self.app_in_foreground = true;
        let has_current_book = CURRENT_BOOK.read().unwrap().is_some();
        let page =
            restored_page_after_rebuild(ACTIVE_PAGE.load(Ordering::Relaxed), has_current_book);
        self.activate_page(cx, page);
        self.refresh_shelf_projection(cx);
        if has_current_book {
            self.restore_reader_after_rebuild(cx);
            if page == AppPage::Reader {
                self.start_reading_timer();
            }
        }
    }

    fn handle_foreground(&mut self, _cx: &mut Cx) {
        self.app_in_foreground = true;
        self.start_reading_timer();
    }

    fn handle_resume(&mut self, _cx: &mut Cx) {
        self.app_in_foreground = true;
        self.start_reading_timer();
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
        self.lan_transfer = None;
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
        if app_action_batch_is_passive(actions, self.on_reader_page) {
            return;
        }
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
            if self.compact_layout && self.reader_toolbar_visible {
                self.set_reader_toolbar_visible(cx, false);
            }
            self.close_word_card(cx);
            self.update_progress_label(cx);
            self.debounce_reader_progress_save(cx);
        }
        for action in actions {
            if let Some(transfer) = action.downcast_ref::<LanTransferAction>() {
                if self
                    .lan_transfer
                    .as_ref()
                    .is_some_and(|s| s.id == transfer.session_id)
                {
                    use crate::lan_transfer::TransferNotice;
                    self.lan_transfer_status = match &transfer.notice {
                        TransferNotice::Receiving(name) => {
                            if app_language() == AppLanguage::English {
                                format!("Receiving {name}…")
                            } else {
                                format!("正在接收：{name}…")
                            }
                        }
                        TransferNotice::Imported(title) => {
                            self.lan_import_count += 1;
                            if app_language() == AppLanguage::English {
                                format!("Added {title} · {} books received", self.lan_import_count)
                            } else {
                                format!(
                                    "已加入书架：{title} · 本次已传 {} 本",
                                    self.lan_import_count
                                )
                            }
                        }
                        TransferNotice::Failed(error) => error.clone(),
                        TransferNotice::Stopped => {
                            self.lan_transfer = None;
                            self.ui
                                .widget(cx, ids!(library_import_scroll))
                                .set_scroll_pos(cx, DVec2::default());
                            localized(
                                "传书已停止，请重新开启。",
                                "Transfer stopped. Start again to receive books.",
                            )
                            .into()
                        }
                    };
                    self.sync_library_import_dialog(cx);
                }
                continue;
            }
            if action.downcast_ref::<ReaderToolbarTap>().is_some() {
                if self.on_reader_page
                    && self.compact_layout
                    && !self.reader_theme_open
                    && !self.toc_open
                    && !self.bookmarks_open
                    && !self.note_open
                    && !self.search_open
                    && self.card.is_none()
                    && !READER_CUSTOMISE_OPEN.load(Ordering::Acquire)
                {
                    self.set_reader_toolbar_visible(cx, !self.reader_toolbar_visible);
                }
                continue;
            }
            if let Some(turn) = action.downcast_ref::<ReaderPageTurn>() {
                if self.on_reader_page && !READER_CUSTOMISE_OPEN.load(Ordering::Acquire) {
                    self.page_scroll(cx, turn.0);
                }
            }
            #[cfg(target_os = "macos")]
            if let Some(import_action) =
                action.downcast_ref::<crate::desktop_import::DesktopImportAction>()
            {
                match import_action {
                    crate::desktop_import::DesktopImportAction::Picked(paths) => {
                        self.ui.label(cx, ids!(library_hint)).set_text(
                            cx,
                            localized("正在导入所选 EPUB…", "Importing selected EPUB files…"),
                        );
                        spawn_import(paths.clone(), false);
                    }
                    crate::desktop_import::DesktopImportAction::Cancelled => {
                        self.ui
                            .label(cx, ids!(library_hint))
                            .set_text(cx, localized("已取消导入", "Import cancelled"));
                    }
                    crate::desktop_import::DesktopImportAction::Error(error) => {
                        self.ui.label(cx, ids!(library_hint)).set_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!("Could not open the file picker: {error}")
                            } else {
                                format!("无法打开文件选择器：{error}")
                            },
                        );
                    }
                }
            }
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
                        } else {
                            let load_statistics = self.statistics_after_pending_session;
                            self.statistics_after_pending_session = false;
                            spawn_finish_reading_session(id.clone(), load_statistics);
                            if self.app_in_foreground && self.on_reader_page && is_current_book {
                                self.start_reading_timer();
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
            if let Some(notes_action) = action.downcast_ref::<AllNotesAction>() {
                match notes_action {
                    AllNotesAction::Loaded(notes) => {
                        *ALL_NOTES.write().unwrap() = notes.clone();
                        self.sync_all_notes_status(cx);
                        self.ui
                            .widget(cx, ids!(all_notes_list))
                            .portal_list(cx, ids!(list))
                            .set_first_id_and_scroll(0, 0.0);
                        self.ui.widget(cx, ids!(all_notes_list)).redraw(cx);
                    }
                    AllNotesAction::Error(error) => self
                        .ui
                        .label(cx, ids!(all_notes_status))
                        .set_text(cx, error),
                }
                continue;
            }
            if let Some(statistics_action) = action.downcast_ref::<StatisticsAction>() {
                match statistics_action {
                    StatisticsAction::Loaded(snapshot) => {
                        *PROFILE_STATISTICS.write().unwrap() = Some(snapshot.clone());
                        self.apply_statistics(cx, snapshot);
                    }
                    StatisticsAction::Error(error) => {
                        self.ui.label(cx, ids!(profile_status)).set_text(cx, error);
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
                        let mut data = CHAT_DATA.write().unwrap();
                        let revision = data.revision.wrapping_add(1);
                        *data = ChatData {
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
                            revision,
                            ..Default::default()
                        };
                        drop(data);
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
                        self.set_selection_hint(cx, hint);
                        if *added {
                            spawn_load_vocab();
                        }
                    }
                    CardAction::MarkKnownSaved { lemma, known_count } => {
                        if let Some(known_count) = known_count {
                            self.project_known_word_count(cx, *known_count);
                        }
                        let changed = CURRENT_BOOK
                            .write()
                            .unwrap()
                            .as_mut()
                            .filter(|book| !book.known_words.contains(lemma))
                            .map(|book| book.mark_word_known(lemma))
                            .is_some();
                        if changed {
                            self.hide_current_word_gloss(cx, lemma);
                        }
                    }
                    CardAction::MarkKnownFailed {
                        lemma,
                        book_id,
                        error,
                    } => {
                        let restored = CURRENT_BOOK
                            .write()
                            .unwrap()
                            .as_mut()
                            .filter(|book| book_id.as_deref() == Some(book.id.as_str()))
                            .map(|book| book.unmark_word_known(lemma))
                            .is_some();
                        if restored {
                            self.refresh_current_chapter(cx);
                        }
                        self.ui.label(cx, ids!(status)).set_text(
                            cx,
                            &format!(
                                "{}: {error}",
                                localized("标记已认识失败", "Could not mark word known")
                            ),
                        );
                    }
                }
                continue;
            }
            if let Some(sel) = action.downcast_ref::<SelAction>() {
                match sel {
                    SelAction::Show { anchor } => {
                        if !self.on_reader_page || SEL_RANGE.read().unwrap().is_none() {
                            SELECTION_POPUP_POINTER.store(false, Ordering::Relaxed);
                            self.selection_popup_anchor = None;
                            self.pending_selection_popup_layout = false;
                            self.ui.widget(cx, ids!(sel_bar)).set_visible(cx, false);
                            self.ui.redraw(cx);
                            continue;
                        }
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
                        SELECTION_POPUP_POINTER.store(false, Ordering::Relaxed);
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
                    HlAction::Saved {
                        book_id,
                        highlight,
                        chapter,
                    } => {
                        if let Some(book) = CURRENT_BOOK
                            .write()
                            .unwrap()
                            .as_mut()
                            .filter(|book| book.id == *book_id)
                        {
                            book.highlights
                                .entry(*chapter)
                                .or_default()
                                .push(highlight.clone());
                        } else {
                            continue;
                        }
                        self.clear_reader_selection(cx);
                        self.redraw_reader_annotations(cx);
                        if self.note_open {
                            self.refresh_reader_highlights(cx);
                        }
                    }
                    HlAction::Deleted {
                        book_id,
                        chapter,
                        ids,
                    } => {
                        // A book reopened while the save was pending may have
                        // loaded the old rows. Reconcile only that same book.
                        let changed = CURRENT_BOOK
                            .write()
                            .unwrap()
                            .as_mut()
                            .filter(|book| book.id == *book_id)
                            .is_some_and(|book| !book.take_highlights(*chapter, ids).is_empty());
                        if changed && *chapter == self.chapter_idx {
                            self.redraw_reader_annotations(cx);
                        }
                        if changed && self.note_open {
                            self.refresh_reader_highlights(cx);
                        }
                    }
                    HlAction::DeleteFailed {
                        book_id,
                        chapter,
                        removed,
                        error,
                    } => {
                        let restored = CURRENT_BOOK
                            .write()
                            .unwrap()
                            .as_mut()
                            .filter(|book| book.id == *book_id)
                            .is_some_and(|book| book.restore_highlights(*chapter, removed));
                        if restored && *chapter == self.chapter_idx {
                            self.redraw_reader_annotations(cx);
                        }
                        if restored && self.note_open {
                            self.refresh_reader_highlights(cx);
                        }
                        self.ui.label(cx, ids!(status)).set_text(
                            cx,
                            &format!(
                                "{}: {error}",
                                localized("删除高亮失败", "Could not remove highlight")
                            ),
                        );
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
                        request_id,
                        resume,
                        bookmarks,
                        gloss_count,
                        parse_ms,
                        initial_gloss_ms,
                    } => {
                        if *request_id != OPEN_BOOK_REQUEST_ID.load(Ordering::Acquire)
                            || !requested_book_is_ready()
                        {
                            continue;
                        }
                        ::log::info!(
                            "Opened EPUB: parse={}ms, initial chapter gloss={}ms, lazy remaining chapters",
                            parse_ms,
                            initial_gloss_ms
                        );
                        *READER_BOOKMARKS.write().unwrap() = bookmarks.clone();
                        self.reader_page_span = 0;
                        // The reader can receive a TOC click while the first
                        // book is still loading. Project the panel again now
                        // that its chapter titles are available.
                        self.sync_reader_panel_layout(cx);
                        self.ui.widget(cx, ids!(toc_list)).redraw(cx);
                        if self.note_open {
                            self.refresh_reader_highlights(cx);
                        }
                        self.set_chapter_at(cx, resume.chapter, resume.block, resume.scroll, false);
                        if let Some(mut reader) = self
                            .ui
                            .widget(cx, ids!(chapter_list))
                            .borrow_mut::<ChapterList>()
                        {
                            reader.pager.requested_source =
                                resume.source.map(|source| (resume.block, source));
                        }
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
                        if let Some((book_id, key, _)) = reader_book_content_key() {
                            spawn_load_notes(book_id.clone());
                            self.reader_pagination_content_key = key;
                            spawn_build_reader_pagination(book_id, key);
                        }
                        self.start_reading_timer();
                    }
                    ReaderAction::OpenError { request_id, error } => {
                        if *request_id != OPEN_BOOK_REQUEST_ID.load(Ordering::Acquire) {
                            continue;
                        }
                        self.set_reader_title_text(
                            cx,
                            &if app_language() == AppLanguage::English {
                                format!("Failed to open: {error}")
                            } else {
                                format!("打开失败: {error}")
                            },
                        );
                    }
                }
                continue;
            }
            if let Some(bookmark_action) = action.downcast_ref::<BookmarkAction>() {
                let active_book_id = CURRENT_BOOK
                    .read()
                    .unwrap()
                    .as_ref()
                    .map(|book| book.id.clone());
                match bookmark_action {
                    BookmarkAction::Updated { book_id, bookmarks }
                        if active_book_id.as_deref() == Some(book_id.as_str()) =>
                    {
                        *READER_BOOKMARKS.write().unwrap() = bookmarks.clone();
                        self.refresh_bookmark_button(cx);
                        self.ui
                            .widget(cx, ids!(reader_bookmarks_hint))
                            .set_visible(cx, bookmarks.is_empty());
                        self.ui.widget(cx, ids!(reader_bookmark_list)).redraw(cx);
                    }
                    BookmarkAction::Error { book_id, error }
                        if active_book_id.as_deref() == Some(book_id.as_str()) =>
                    {
                        self.ui.label(cx, ids!(status)).set_text(
                            cx,
                            &format!("{}: {error}", localized("书签操作失败", "Bookmark failed")),
                        );
                    }
                    _ => {}
                }
                continue;
            }
            if let Some(ReaderSearchAction::Results {
                book_id,
                query,
                hits,
            }) = action.downcast_ref::<ReaderSearchAction>()
            {
                let active_book_id = CURRENT_BOOK
                    .read()
                    .unwrap()
                    .as_ref()
                    .map(|book| book.id.clone());
                if active_book_id.as_deref() == Some(book_id.as_str())
                    && self.reader_search_query == *query
                {
                    *READER_SEARCH_RESULTS.write().unwrap() = hits.clone();
                    self.ui.label(cx, ids!(reader_search_hint)).set_text(
                        cx,
                        &if app_language() == AppLanguage::English {
                            format!("{} results", hits.len())
                        } else {
                            format!("找到 {} 处", hits.len())
                        },
                    );
                    self.ui.widget(cx, ids!(reader_search_list)).redraw(cx);
                }
                continue;
            }
            if let Some(pagination) = action.downcast_ref::<ReaderPaginationAction>() {
                match pagination {
                    ReaderPaginationAction::Ready { book_id, content } => {
                        let is_current = CURRENT_BOOK
                            .read()
                            .unwrap()
                            .as_ref()
                            .is_some_and(|book| book.id == *book_id);
                        if is_current
                            && reader_book_content_key()
                                .is_some_and(|(_, key, _)| key == content.key)
                        {
                            let widget = self.ui.widget(cx, ids!(chapter_list));
                            if let Some(mut reader) = widget.borrow_mut::<ChapterList>() {
                                reader.book_pages.set_content(content.clone());
                            }
                            widget.redraw(cx);
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
                        self.shelf_ready = true;
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
                        if std::mem::take(&mut self.pending_last_read_open)
                            && self.active_page == AppPage::ReadingHome
                        {
                            if let Some(book) = Self::reading_home_book() {
                                self.open_book_for_reading(cx, book);
                            }
                        }
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
                } else {
                    let target = CURRENT_BOOK
                        .read()
                        .unwrap()
                        .as_ref()
                        .map(|book| resolve_reader_link(book, self.chapter_idx, &url));
                    match target {
                        Some(ReaderLinkTarget::External(url)) => {
                            cx.open_url(&url, OpenUrlInPlace::No)
                        }
                        Some(ReaderLinkTarget::Image(path)) => {
                            self.open_reader_image_viewer(cx, &path)
                        }
                        Some(ReaderLinkTarget::Chapter { index, fragment }) => {
                            let block = CURRENT_BOOK
                                .write()
                                .unwrap()
                                .as_mut()
                                .and_then(|book| build_chapter_chunks(book, index))
                                .map(|chunks| {
                                    fragment
                                        .as_deref()
                                        .map_or(0, |id| reader_anchor_block(&chunks, id))
                                })
                                .unwrap_or(0);
                            self.set_chapter_at(cx, index, block, 0.0, true);
                        }
                        _ => self
                            .ui
                            .label(cx, ids!(status))
                            .set_text(cx, localized("无法打开此链接", "Cannot open this link")),
                    }
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
                let font_size = READER_FONT_SIZE
                    .read()
                    .unwrap()
                    .unwrap_or(READER_FONT_DEFAULT);
                for id in [id!(card_word), id!(card_hint)] {
                    if let Some(mut label) = self.ui.label(cx, &[id]).borrow_mut() {
                        label.draw_text.text_style.font_size = font_size;
                    }
                }
                // Chinese gloss and action glyphs look larger than Latin body
                // text at the same numeric size.
                let action_font_size = (font_size * 0.8).max(8.0);
                if let Some(mut label) = self.ui.label(cx, ids!(card_gloss)).borrow_mut() {
                    label.draw_text.text_style.font_size = action_font_size;
                }
                for id in [id!(close_card_btn), id!(add_vocab_btn), id!(know_btn)] {
                    let mut button = self.ui.widget(cx, &[id]);
                    script_apply_eval!(cx, button, {
                        draw_text.text_style.font_size: #(action_font_size)
                    });
                }
                self.ui.label(cx, ids!(card_word)).set_text(cx, &word);
                self.ui.label(cx, ids!(card_gloss)).set_text(cx, &gloss);
                self.ui.label(cx, ids!(card_hint)).set_text(cx, "");
                self.ui.widget(cx, ids!(word_card)).set_visible(cx, true);
                self.word_card_anchor = Some(anchor);
                self.position_word_card(cx);
                self.pending_word_card_layout = false;
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
            .shelf_list(cx, ids!(list));
        for (item_id, item) in shelf_list.items_with_actions(actions) {
            if shelf_view_mode == LibraryViewMode::Grid {
                for (column, cell_id) in [
                    id!(grid_cell_0),
                    id!(grid_cell_1),
                    id!(grid_cell_2),
                    id!(grid_cell_3),
                    id!(grid_cell_4),
                    id!(grid_cell_5),
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
            self.open_library_import_dialog(cx);
        }
        if self
            .ui
            .button(cx, ids!(library_import_close))
            .clicked(actions)
            || self
                .ui
                .modal(cx, ids!(library_import_modal))
                .dismissed(actions)
        {
            self.close_library_import_dialog(cx);
        }
        if self
            .ui
            .button(cx, ids!(library_import_stop))
            .clicked(actions)
        {
            self.stop_lan_transfer(cx);
        }
        if self
            .ui
            .button(cx, ids!(library_import_lan))
            .clicked(actions)
            || self
                .ui
                .button(cx, ids!(library_import_restart))
                .clicked(actions)
        {
            self.start_lan_transfer(cx);
        }
        if self
            .ui
            .button(cx, ids!(library_import_copy))
            .clicked(actions)
        {
            if let Some(receiver) = &self.lan_transfer {
                cx.copy_to_clipboard(&receiver.urls[0]);
                self.lan_transfer_status = localized(
                    "网址已复制，请在同一 Wi-Fi 下的电脑浏览器打开。",
                    "Address copied. Open it in a computer browser on the same Wi-Fi.",
                )
                .into();
                self.sync_library_import_dialog(cx);
            }
        }
        if self
            .ui
            .button(cx, ids!(library_import_background))
            .clicked(actions)
        {
            #[cfg(target_os = "android")]
            {
                self.lan_transfer_status = match crate::android_lan_transfer::request_background_access() {
                    Ok(()) => localized("请在系统中允许后台运行；若仍暂停接收，请将本应用的耗电管理设为允许后台高耗电。", "Allow background operation in the system dialog. If receiving still pauses, allow unrestricted battery use for ReadWise.").into(),
                    Err(error) => error,
                };
                self.sync_library_import_dialog(cx);
            }
        }
        if self
            .ui
            .button(cx, ids!(library_import_local))
            .clicked(actions)
        {
            self.close_library_import_dialog(cx);
            #[cfg(target_os = "macos")]
            if self.import_picker_frame.is_none() {
                // The action is delivered inside Makepad's MouseUp dispatch,
                // before it clears the pressed finger. Present the native
                // sheet after that event has fully unwound.
                self.import_picker_frame = Some(cx.new_next_frame());
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
            || self.ui.button(cx, ids!(reading_browse)).clicked(actions)
        {
            if self.on_reader_page {
                self.stop_reading_timer(cx, false);
            }
            self.library_tag_filter = None;
            self.activate_page(cx, AppPage::Library);
            // The library page or its PortalList may only be instantiated when
            // entering it after a mobile responsive transition.
            self.refresh_shelf_projection(cx);
            if cfg!(any(target_os = "android", target_os = "ios")) {
                self.pending_ui_projection_after_draw = true;
            }
        }
        if self.ui.button(cx, ids!(reader_back_btn)).clicked(actions)
            || self.ui.button(cx, ids!(titlebar_back_btn)).clicked(actions)
        {
            self.stop_reading_timer(cx, false);
            self.activate_page(cx, AppPage::Library);
            self.refresh_shelf_projection(cx);
        }
        if self
            .ui
            .button(cx, ids!(reader_appearance_btn))
            .clicked(actions)
            || self
                .ui
                .button(cx, ids!(titlebar_appearance_btn))
                .clicked(actions)
        {
            self.reader_theme_open = !self.reader_theme_open;
            if self.reader_theme_open {
                self.close_reader_info_popup(cx);
            }
            self.ui
                .widget(cx, ids!(reader_theme_menu))
                .set_visible(cx, self.reader_theme_open);
            self.ui
                .widget(cx, ids!(reader_theme_arrow_row))
                .set_visible(cx, self.reader_theme_open);
            self.set_reader_toolbar_visible(cx, true);
            self.ui.widget(cx, ids!(reader_theme_layer)).redraw(cx);
        }
        if self
            .ui
            .button(cx, ids!(reader_bookmark_btn))
            .clicked(actions)
            || self
                .ui
                .button(cx, ids!(reader_bookmark_filled_btn))
                .clicked(actions)
            || self
                .ui
                .button(cx, ids!(titlebar_bookmark_btn))
                .clicked(actions)
            || self
                .ui
                .button(cx, ids!(titlebar_bookmark_filled_btn))
                .clicked(actions)
        {
            self.toggle_reader_bookmark(cx);
        }
        for (id, background) in [
            (id!(reader_mode_original), ReaderBackground::Original),
            (id!(reader_mode_quiet), ReaderBackground::Sepia),
            (id!(reader_mode_paper), ReaderBackground::Paper),
            (id!(reader_mode_bold), ReaderBackground::Bold),
            (id!(reader_mode_calm), ReaderBackground::Sage),
            (id!(reader_mode_focus), ReaderBackground::Night),
        ] {
            if self.ui.button(cx, &[id]).clicked(actions) {
                self.apply_reader_mode(cx, background);
            }
        }
        // Quick choices are committed preferences, never a layout edit or a
        // draft preview. Preserve the current page, caches and turn history.
        let mut quick_color_changed = false;
        for (ids, kind, slots) in [
            (
                [
                    id!(reader_quick_pen_vermilion),
                    id!(reader_quick_pen_cobalt),
                    id!(reader_quick_pen_moss),
                    id!(reader_quick_pen_graphite),
                ],
                ColorKind::Selection,
                crate::settings_swatch::PEN_SLOTS,
            ),
            (
                [
                    id!(reader_quick_highlight_yellow),
                    id!(reader_quick_highlight_green),
                    id!(reader_quick_highlight_blue),
                    id!(reader_quick_highlight_pink),
                ],
                ColorKind::Highlight,
                crate::mark_span::MARKER_SLOTS,
            ),
        ] {
            for (id, slot) in ids.into_iter().zip(slots) {
                if self.ui.button(cx, &[id]).clicked(actions) {
                    let lock = match kind {
                        ColorKind::Selection => &SELECTION_SLOT,
                        _ => &crate::mark_span::HL_SLOT,
                    };
                    let mut selected = lock.write().unwrap();
                    quick_color_changed |= *selected != slot;
                    *selected = slot;
                }
            }
        }
        if quick_color_changed {
            self.refresh_swatches(cx);
            self.ui.redraw(cx);
            self.persist_reader_settings(cx);
        }
        if self
            .ui
            .button(cx, ids!(reader_customise_btn))
            .clicked(actions)
        {
            self.reader_theme_open = false;
            self.ui
                .widget(cx, ids!(reader_theme_menu))
                .set_visible(cx, false);
            self.ui
                .widget(cx, ids!(reader_theme_arrow_row))
                .set_visible(cx, false);
            self.open_reader_customise(cx);
        }
        if self
            .ui
            .button(cx, ids!(reader_customise_close))
            .clicked(actions)
            || self
                .ui
                .button(cx, ids!(reader_customise_cancel))
                .clicked(actions)
            || self
                .ui
                .modal(cx, ids!(reader_customise_modal))
                .dismissed(actions)
        {
            self.close_reader_customise(cx);
        }
        if self
            .ui
            .button(cx, ids!(reader_customise_save))
            .clicked(actions)
        {
            self.save_reader_customise(cx);
        }
        if let Some(index) = self
            .ui
            .drop_down(cx, ids!(reader_customise_font_choice))
            .selected(actions)
        {
            self.reader_customise_open_dropdown = None;
            if let (Some(draft), Some(font)) = (
                self.reader_customise_draft.as_mut(),
                ReaderFont::from_index(index),
            ) {
                draft.font = font;
                self.load_reader_font(cx, font, true);
                self.sync_reader_customise_controls(cx);
            }
        }
        if let Some(bold) = self
            .ui
            .check_box(cx, ids!(reader_customise_bold))
            .changed(actions)
        {
            if let Some(draft) = self.reader_customise_draft.as_mut() {
                draft.bold_text = bold;
                self.sync_reader_customise_controls(cx);
            }
        }
        let mut highlight_changed = false;
        for (id, slot) in [
            id!(reader_customise_highlight_yellow),
            id!(reader_customise_highlight_green),
            id!(reader_customise_highlight_blue),
            id!(reader_customise_highlight_pink),
        ]
        .into_iter()
        .zip(crate::mark_span::MARKER_SLOTS)
        {
            if self.ui.button(cx, &[id]).clicked(actions) {
                if let Some(draft) = self.reader_customise_draft.as_mut() {
                    draft.highlight_color_slot = slot;
                    highlight_changed = true;
                }
            }
        }
        if highlight_changed {
            if let Some(draft) = self.reader_customise_draft.as_ref() {
                self.sync_reader_customise_highlighter(cx, draft);
            }
        }
        let mut gloss_preview_changed = false;
        if let Some(enabled) = self
            .ui
            .check_box(cx, ids!(reader_customise_gloss_enable))
            .changed(actions)
        {
            if let Some(draft) = self.reader_customise_draft.as_mut() {
                draft.gloss_enabled = enabled;
                gloss_preview_changed = true;
            }
        }
        for (id, slot) in [
            (id!(reader_customise_gloss_color_green), 0),
            (id!(reader_customise_gloss_color_blue), 1),
            (id!(reader_customise_gloss_color_orange), 2),
            (id!(reader_customise_gloss_color_gray), 3),
        ] {
            if self.ui.button(cx, &[id]).clicked(actions) {
                if let Some(draft) = self.reader_customise_draft.as_mut() {
                    draft.gloss_color_slot = slot;
                    gloss_preview_changed = true;
                }
            }
        }
        for (id, slot) in [
            (id!(reader_customise_gloss_bg_none), 0),
            (id!(reader_customise_gloss_bg_gray), 1),
            (id!(reader_customise_gloss_bg_green), 2),
            (id!(reader_customise_gloss_bg_blue), 3),
        ] {
            if self.ui.button(cx, &[id]).clicked(actions) {
                if let Some(draft) = self.reader_customise_draft.as_mut() {
                    draft.gloss_background_slot = slot;
                    gloss_preview_changed = true;
                }
            }
        }
        for (id, slot) in [
            (id!(reader_customise_word_color_default), 0),
            (id!(reader_customise_word_color_green), 1),
            (id!(reader_customise_word_color_blue), 2),
            (id!(reader_customise_word_color_orange), 3),
        ] {
            if self.ui.button(cx, &[id]).clicked(actions) {
                if let Some(draft) = self.reader_customise_draft.as_mut() {
                    draft.word_color_slot = slot;
                    gloss_preview_changed = true;
                }
            }
        }
        for (id, slot) in [
            (id!(reader_customise_word_bg_none), 0),
            (id!(reader_customise_word_bg_gray), 1),
            (id!(reader_customise_word_bg_green), 2),
            (id!(reader_customise_word_bg_blue), 3),
        ] {
            if self.ui.button(cx, &[id]).clicked(actions) {
                if let Some(draft) = self.reader_customise_draft.as_mut() {
                    draft.word_background_slot = slot;
                    gloss_preview_changed = true;
                }
            }
        }
        if gloss_preview_changed {
            self.sync_reader_customise_controls(cx);
        }
        let size_value = self
            .ui
            .slider(cx, ids!(reader_customise_size_slider))
            .slided(actions);
        let gloss_size_value = self
            .ui
            .slider(cx, ids!(reader_customise_gloss_size_slider))
            .slided(actions);
        let line_value = self
            .ui
            .slider(cx, ids!(reader_customise_line_slider))
            .slided(actions);
        let character_value = self
            .ui
            .slider(cx, ids!(reader_customise_character_slider))
            .slided(actions);
        let word_value = self
            .ui
            .slider(cx, ids!(reader_customise_word_slider))
            .slided(actions);
        let margin_value = self
            .ui
            .slider(cx, ids!(reader_customise_margin_slider))
            .slided(actions);
        // Makepad measures the drag against the full widget width, while the
        // visible thumb stops inside it. Snap that small edge gap to the limit.
        let snap_edge = |value: f64, min: f64, max: f64| {
            let edge = (max - min) * 0.1;
            if value <= min + edge {
                min
            } else if value >= max - edge {
                max
            } else {
                value
            }
        };
        if let Some(draft) = self.reader_customise_draft.as_mut() {
            let mut changed = false;
            if let Some(value) = size_value {
                // The drawn thumb stops short of the widget edge; keep both
                // endpoints reachable without swallowing the first step.
                let edge = (READER_FONT_MAX - READER_FONT_MIN) as f64 * 0.05;
                let size = if value <= READER_FONT_MIN as f64 + edge {
                    READER_FONT_MIN
                } else if value >= READER_FONT_MAX as f64 - edge {
                    READER_FONT_MAX
                } else {
                    value.round() as f32
                };
                draft.font_size = Some(size.clamp(READER_FONT_MIN, READER_FONT_MAX));
                changed = true;
            }
            if let Some(value) = gloss_size_value {
                let edge = (12.0 - 6.0) * 0.05;
                let size = if value <= 6.0 + edge {
                    6.0
                } else if value >= 12.0 - edge {
                    12.0
                } else {
                    (value * 2.0).round() / 2.0
                };
                draft.gloss_font_size = (size as f32).clamp(6.0, 12.0);
                changed = true;
            }
            if let Some(value) = line_value {
                draft.spacing = ReaderSpacing::from_value(
                    (snap_edge(value, 0.8, 2.5) * 100.0).round() as f32 / 100.0,
                );
                changed = true;
            }
            if let Some(value) = character_value {
                draft.character_spacing =
                    (snap_edge(value, -10.0, 10.0).round() as f32 / 100.0).clamp(-0.1, 0.1);
                changed = true;
            }
            if let Some(value) = word_value {
                draft.word_spacing =
                    (snap_edge(value, -20.0, 20.0).round() as f32 / 100.0).clamp(-0.2, 0.2);
                changed = true;
            }
            if let Some(value) = margin_value {
                draft.margin = (snap_edge(value, -10.0, 10.0).round() as f32).clamp(-10.0, 10.0);
                changed = true;
            }
            if changed {
                self.sync_reader_customise_controls(cx);
            }
        }
        if let Some(index) = self
            .ui
            .drop_down(cx, ids!(reader_customise_columns_choice))
            .selected(actions)
        {
            self.reader_customise_open_dropdown = None;
            if let Some(draft) = self.reader_customise_draft.as_mut() {
                draft.columns = match index {
                    0 => ReaderColumns::One,
                    1 => ReaderColumns::Two,
                    _ => ReaderColumns::Auto,
                };
                self.sync_reader_customise_controls(cx);
            }
        }
        if let Some(justify) = self
            .ui
            .check_box(cx, ids!(reader_customise_justify))
            .changed(actions)
        {
            if let Some(draft) = self.reader_customise_draft.as_mut() {
                draft.justify_text = justify;
                self.sync_reader_customise_controls(cx);
            }
        }
        if self
            .ui
            .button(cx, ids!(reader_customise_reset))
            .clicked(actions)
        {
            if let Some(draft) = self.reader_customise_draft.as_mut() {
                let default = ReaderSettings::default();
                draft.theme = default.theme;
                draft.background = default.background;
                draft.columns = default.columns;
                draft.font = default.font;
                draft.bold_text = default.bold_text;
                draft.spacing = default.spacing;
                draft.character_spacing = default.character_spacing;
                draft.word_spacing = default.word_spacing;
                draft.margin = default.margin;
                draft.justify_text = default.justify_text;
                draft.font_size = default.font_size;
                draft.gloss_enabled = default.gloss_enabled;
                draft.gloss_font_size = default.gloss_font_size;
                draft.gloss_color_slot = default.gloss_color_slot;
                draft.gloss_background_slot = default.gloss_background_slot;
                draft.word_color_slot = default.word_color_slot;
                draft.word_background_slot = default.word_background_slot;
                draft.highlight_color_slot = default.highlight_color_slot;
                self.sync_reader_customise_controls(cx);
            }
        }
        if self.ui.button(cx, ids!(nav_reader)).clicked(actions) {
            if let Some(book) = Self::reading_home_book() {
                self.pending_last_read_open = false;
                if CURRENT_BOOK
                    .read()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|current| current.id == book.id)
                {
                    self.activate_page(cx, AppPage::Reader);
                    self.start_reading_timer();
                } else {
                    self.open_book_for_reading(cx, book);
                }
            } else {
                self.pending_last_read_open = !self.shelf_ready;
                self.activate_page(cx, AppPage::ReadingHome);
            }
        }
        if self.ui.button(cx, ids!(mobile_nav_reader)).clicked(actions) {
            self.activate_page(cx, AppPage::ReadingHome);
        }
        if self.ui.button(cx, ids!(reading_continue)).clicked(actions) {
            if let Some(book) = Self::reading_home_book() {
                if CURRENT_BOOK
                    .read()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|current| current.id == book.id)
                {
                    self.activate_page(cx, AppPage::Reader);
                    self.start_reading_timer();
                } else {
                    self.open_book_for_reading(cx, book);
                }
            } else {
                self.activate_page(cx, AppPage::Library);
                self.refresh_shelf_projection(cx);
            }
        }
        if self
            .ui
            .button(cx, ids!(mobile_nav_profile))
            .clicked(actions)
            || self.ui.button(cx, ids!(profile_back)).clicked(actions)
        {
            if self.on_reader_page {
                self.stop_reading_timer(cx, true);
            } else {
                spawn_load_statistics();
            }
            self.activate_page(cx, AppPage::Profile);
        }
        if self
            .ui
            .widget(cx, ids!(profile_notes))
            .button(cx, ids!(metric_button))
            .clicked(actions)
        {
            self.activate_page(cx, AppPage::Notes);
            self.ui
                .label(cx, ids!(all_notes_status))
                .set_text(cx, localized("正在读取笔记…", "Loading notes…"));
            spawn_load_all_notes();
        }
        for (id, filter) in [
            (id!(profile_reading), READING_FILTER),
            (id!(profile_completed), COMPLETED_FILTER),
        ] {
            if self
                .ui
                .widget(cx, &[id])
                .button(cx, ids!(metric_button))
                .clicked(actions)
            {
                self.library_tag_filter = Some(filter.to_string());
                self.activate_page(cx, AppPage::Library);
                self.refresh_shelf_projection(cx);
            }
        }
        if self.ui.button(cx, ids!(nav_chat)).clicked(actions)
            || self.ui.button(cx, ids!(profile_chat)).clicked(actions)
        {
            if self.on_reader_page {
                self.stop_reading_timer(cx, false);
            }
            self.activate_page(cx, AppPage::Chat);
        }
        if self.ui.button(cx, ids!(nav_skills)).clicked(actions)
            || self.ui.button(cx, ids!(profile_skills)).clicked(actions)
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
                .widget(cx, ids!(profile_duration))
                .button(cx, ids!(metric_button))
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
            || self.ui.button(cx, ids!(profile_settings)).clicked(actions)
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
            .radio_button_set(
                cx,
                ids_array!(reader_columns_one, reader_columns_two, reader_columns_auto),
            )
            .selected(cx, actions)
        {
            let mode = match index {
                0 => ReaderColumns::One,
                1 => ReaderColumns::Two,
                _ => ReaderColumns::Auto,
            };
            self.set_reader_columns(cx, mode);
        }
        if let Some(index) = self
            .ui
            .radio_button_set(
                cx,
                ids_array!(reader_turn_curl, reader_turn_slide, reader_turn_scroll),
            )
            .selected(cx, actions)
        {
            let mode = match index {
                1 => ReaderTurnMode::Slide,
                2 => ReaderTurnMode::Scroll,
                _ => ReaderTurnMode::Curl,
            };
            *READER_TURN_MODE.write().unwrap() = mode;
            self.reader_swipe.cancel();
            self.clear_reader_selection(cx);
            self.close_word_card(cx);
            self.sync_reader_turn_controls(cx, mode);
            self.update_reader_page_footer(cx);
            self.invalidate_reader_pagination_layout(cx);
            self.persist_reader_settings(cx);
        }
        if let Some(index) = self
            .ui
            .drop_down(cx, ids!(reader_font_choice))
            .selected(actions)
        {
            if let Some(font) = ReaderFont::from_index(index) {
                self.set_reader_font(cx, font);
            }
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
                    let removed = CURRENT_BOOK
                        .write()
                        .unwrap()
                        .as_mut()
                        .filter(|book| book.id == book_id)
                        .map(|book| book.take_highlights(self.chapter_idx, &ids))
                        .unwrap_or_default();
                    self.clear_reader_selection(cx);
                    self.redraw_reader_annotations(cx);
                    spawn_delete_highlights(book_id, self.chapter_idx, ids, removed);
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
                self.close_word_card(cx);
                let book_id = CURRENT_BOOK.write().unwrap().as_mut().map(|book| {
                    book.mark_word_known(&lemma);
                    book.id.clone()
                });
                if book_id.is_some() {
                    self.hide_current_word_gloss(cx, &lemma);
                }
                spawn_mark_known(lemma, book_id);
            }
        }
        if self.ui.button(cx, ids!(toc_btn)).clicked(actions)
            || self.ui.button(cx, ids!(titlebar_toc_btn)).clicked(actions)
        {
            self.toc_open = !self.toc_open;
            self.pending_toc_first_draw = self.toc_open;
            if self.toc_open {
                self.note_open = false;
                self.reader_ai_open = false;
                self.bookmarks_open = false;
                self.search_open = false;
                self.ui.widget(cx, ids!(toc_list)).redraw(cx);
            }
            self.sync_reader_panel_layout(cx);
            self.ui.redraw(cx);
        }
        if self.ui.button(cx, ids!(toc_close_btn)).clicked(actions) {
            self.close_reader_info_popup(cx);
        }
        if self.ui.button(cx, ids!(notes_btn)).clicked(actions)
            || self
                .ui
                .button(cx, ids!(titlebar_notes_btn))
                .clicked(actions)
        {
            self.note_open = !self.note_open;
            if self.note_open {
                self.refresh_reader_highlights(cx);
                self.toc_open = false;
                self.reader_ai_open = false;
                self.bookmarks_open = false;
                self.search_open = false;
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
        if self
            .ui
            .button(cx, ids!(reader_bookmarks_btn))
            .clicked(actions)
            || self
                .ui
                .button(cx, ids!(titlebar_bookmarks_btn))
                .clicked(actions)
        {
            self.bookmarks_open = !self.bookmarks_open;
            if self.bookmarks_open {
                self.toc_open = false;
                self.note_open = false;
                self.reader_ai_open = false;
                self.search_open = false;
                self.ui.widget(cx, ids!(reader_bookmark_list)).redraw(cx);
            }
            self.sync_reader_panel_layout(cx);
            self.ui.redraw(cx);
        }
        if self
            .ui
            .button(cx, ids!(reader_bookmarks_close))
            .clicked(actions)
        {
            self.bookmarks_open = false;
            self.sync_reader_panel_layout(cx);
        }
        if self.ui.button(cx, ids!(reader_search_btn)).clicked(actions)
            || self
                .ui
                .button(cx, ids!(titlebar_search_btn))
                .clicked(actions)
        {
            self.search_open = !self.search_open;
            if self.search_open {
                self.toc_open = false;
                self.note_open = false;
                self.reader_ai_open = false;
                self.bookmarks_open = false;
                self.reader_theme_open = false;
                self.ui
                    .widget(cx, ids!(reader_theme_menu))
                    .set_visible(cx, false);
                self.ui
                    .widget(cx, ids!(reader_theme_arrow_row))
                    .set_visible(cx, false);
            }
            self.sync_reader_panel_layout(cx);
            self.ui.redraw(cx);
        }
        if self
            .ui
            .button(cx, ids!(reader_search_close))
            .clicked(actions)
        {
            self.search_open = false;
            self.sync_reader_panel_layout(cx);
        }
        let search_return = self
            .ui
            .text_input(cx, ids!(reader_search_input))
            .returned(actions);
        if self
            .ui
            .button(cx, ids!(reader_search_submit))
            .clicked(actions)
            || search_return.is_some()
        {
            let query = self
                .ui
                .text_input(cx, ids!(reader_search_input))
                .text()
                .trim()
                .to_string();
            self.reader_search_query = query.clone();
            READER_SEARCH_RESULTS.write().unwrap().clear();
            self.ui.widget(cx, ids!(reader_search_list)).redraw(cx);
            if let Some(book_id) = CURRENT_BOOK
                .read()
                .unwrap()
                .as_ref()
                .map(|book| book.id.clone())
            {
                self.ui
                    .label(cx, ids!(reader_search_hint))
                    .set_text(cx, localized("正在搜索…", "Searching…"));
                spawn_reader_search(book_id, query);
            }
        }
        let bookmark_list = self
            .ui
            .widget(cx, ids!(reader_bookmark_list))
            .portal_list(cx, ids!(list));
        for (item_id, item) in bookmark_list.items_with_actions(actions) {
            if item.button(cx, ids!(jump_btn)).clicked(actions) {
                let bookmark = READER_BOOKMARKS.read().unwrap().get(item_id).cloned();
                if let Some(bookmark) = bookmark {
                    let chapter_count = CURRENT_BOOK
                        .read()
                        .unwrap()
                        .as_ref()
                        .map_or(0, |book| book.chapters.len());
                    let location = ReaderLocation::decode(&bookmark.location, chapter_count);
                    self.set_chapter_at(
                        cx,
                        location.chapter,
                        location.block,
                        location.scroll,
                        true,
                    );
                    if let (Some(source), Some(mut reader)) = (
                        location.source,
                        self.ui
                            .widget(cx, ids!(chapter_list))
                            .borrow_mut::<ChapterList>(),
                    ) {
                        reader.pager.requested_source = Some((location.block, source));
                    }
                    self.bookmarks_open = false;
                    self.sync_reader_panel_layout(cx);
                }
            }
        }
        let search_list = self
            .ui
            .widget(cx, ids!(reader_search_list))
            .portal_list(cx, ids!(list));
        for (item_id, item) in search_list.items_with_actions(actions) {
            if item.button(cx, ids!(jump_btn)).clicked(actions) {
                let hit = READER_SEARCH_RESULTS.read().unwrap().get(item_id).cloned();
                if let Some(hit) = hit {
                    self.set_chapter_at(cx, hit.chapter, hit.block, 0.0, true);
                    self.search_open = false;
                    self.sync_reader_panel_layout(cx);
                }
            }
        }
        let highlight_list = self
            .ui
            .widget(cx, ids!(reader_highlight_list))
            .portal_list(cx, ids!(list));
        for (item_id, item) in highlight_list.items_with_actions(actions) {
            if item.button(cx, ids!(jump_btn)).clicked(actions) {
                let hit = READER_HIGHLIGHT_JUMPS.read().unwrap().get(item_id).cloned();
                if let Some(hit) = hit {
                    self.set_chapter_at(cx, hit.chapter, hit.block, 0.0, true);
                    self.note_open = false;
                    self.sync_reader_panel_layout(cx);
                }
            }
        }
        let toc_list = self
            .ui
            .widget(cx, ids!(toc_list))
            .shelf_list(cx, ids!(list));
        for (item_id, item) in toc_list.items_with_actions(actions) {
            if item.button(cx, ids!(toc_item_btn)).clicked(actions) {
                let target = CURRENT_BOOK
                    .read()
                    .unwrap()
                    .as_ref()
                    .and_then(|book| book.toc.get(item_id).cloned());
                if let Some(target) = target {
                    let block = CURRENT_BOOK
                        .write()
                        .unwrap()
                        .as_mut()
                        .and_then(|book| build_chapter_chunks(book, target.chapter))
                        .map(|chunks| {
                            target
                                .fragment
                                .as_deref()
                                .map_or(0, |fragment| reader_anchor_block(&chunks, fragment))
                        })
                        .unwrap_or(0);
                    self.set_chapter_at(cx, target.chapter, block, 0.0, true);
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
            if item.button(cx, ids!(note_save_btn)).clicked(actions) {
                let note = NOTES.read().unwrap().entry_for_item_id(item_id).cloned();
                let book_id = CURRENT_BOOK.read().unwrap().as_ref().map(|b| b.id.clone());
                if let (Some(note), Some(book_id)) = (note, book_id) {
                    let (quote, _) = split_note_content(&note.content);
                    let body = item.text_input(cx, ids!(note_body_input)).text();
                    let content = format_note_content(quote, &body);
                    self.ui
                        .label(cx, ids!(note_hint))
                        .set_text(cx, localized("保存中…", "Saving…"));
                    spawn_update_note(book_id, note.id, content);
                }
            }
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
                color_bg_container: #(surface)
                color_bg_even: #(surface)
                color_bg_odd: #(surface)
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

                color_inset: #(surface)
                color_inset_hover: #(surface)
                color_inset_down: #(pressed)
                color_inset_active: #(field)
                color_inset_focus: #(surface)
                color_inset_drag: #(surface)
                color_inset_disabled: #(surface)
                color_inset_empty: #(surface)
                color_inset_1: #(surface)
                color_inset_1_hover: #(surface)
                color_inset_1_down: #(pressed)
                color_inset_1_active: #(field)
                color_inset_1_focus: #(surface)
                color_inset_1_drag: #(surface)
                color_inset_1_disabled: #(surface)
                color_inset_1_empty: #(surface)
                color_inset_2: #(surface)
                color_inset_2_hover: #(surface)
                color_inset_2_down: #(pressed)
                color_inset_2_active: #(field)
                color_inset_2_focus: #(surface)
                color_inset_2_drag: #(surface)
                color_inset_2_disabled: #(surface)
                color_inset_2_empty: #(surface)

                color_outset: #(surface)
                color_outset_hover: #(hover)
                color_outset_down: #(pressed)
                color_outset_active: #(surface)
                color_outset_focus: #(surface)
                color_outset_drag: #(hover)
                color_outset_disabled: #(surface)
                color_outset_inactive: #(surface)
                color_outset_1: #(surface)
                color_outset_1_hover: #(hover)
                color_outset_1_down: #(pressed)
                color_outset_1_active: #(surface)
                color_outset_1_focus: #(surface)
                color_outset_1_drag: #(hover)
                color_outset_1_disabled: #(surface)
                color_outset_2: #(surface)
                color_outset_2_hover: #(hover)
                color_outset_2_down: #(pressed)
                color_outset_2_active: #(surface)
                color_outset_2_focus: #(surface)
                color_outset_2_drag: #(hover)
                color_outset_2_disabled: #(surface)

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
        crate::shelf_list::script_mod(vm);
        crate::ruby_word::script_mod(vm);
        crate::reader_hit::script_mod(vm);
        crate::reader_inline_code::script_mod(vm);
        crate::reader_link::script_mod(vm);
        crate::chapter_image::script_mod(vm);
        crate::reader_page_turn::script_mod(vm);
        crate::mark_span::script_mod(vm);
        crate::settings_swatch::script_mod(vm);
        self::script_mod(vm)
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        if self
            .selection_popup_layout_frame
            .as_ref()
            .and_then(|frame| frame.is_event(event))
            .is_some()
        {
            self.selection_popup_layout_frame = None;
            self.ui.widget(cx, ids!(sel_bar)).redraw(cx);
        }
        if self
            .reader_code_refresh_frame
            .as_ref()
            .and_then(|frame| frame.is_event(event))
            .is_some()
        {
            self.reader_code_refresh_frame = None;
            if self.on_reader_page {
                if let Some(mut reader) = self
                    .ui
                    .widget(cx, ids!(chapter_list))
                    .borrow_mut::<ChapterList>()
                {
                    reader.redraw_visible_code_views(cx);
                }
            }
        }
        #[cfg(target_os = "macos")]
        if self
            .import_picker_frame
            .as_ref()
            .and_then(|frame| frame.is_event(event))
            .is_some()
        {
            self.import_picker_frame = None;
            if let Err(error) = crate::desktop_import::open_epub_picker(
                localized("导入 EPUB", "Import EPUB"),
                localized("导入", "Import"),
            ) {
                self.ui.label(cx, ids!(library_hint)).set_text(
                    cx,
                    &if app_language() == AppLanguage::English {
                        format!("Could not open the file picker: {error}")
                    } else {
                        format!("无法打开文件选择器：{error}")
                    },
                );
            }
        }
        // The word card owns pointer input while open. A press outside closes
        // it immediately; a press inside must not start selection underneath.
        let word_card_press = match event {
            Event::MouseDown(e) => Some(e.abs),
            Event::TouchUpdate(e) => e
                .touches
                .iter()
                .find(|touch| touch.state == TouchState::Start)
                .map(|touch| touch.abs),
            _ => None,
        };
        let card_captures_pointer = if self.word_card_anchor.is_some() {
            if let Some(abs) = word_card_press {
                let card_rect =
                    current_clipped_rect(cx, self.ui.widget(cx, ids!(word_card)).area());
                if !card_rect.contains(abs) {
                    self.close_word_card(cx);
                    return;
                }
                true
            } else {
                SELECTION_POPUP_POINTER.load(Ordering::Relaxed)
            }
        } else {
            false
        };
        // Dismiss from the window-level owner, not only the shelf body's
        // transparent overlay. Consume this press so it cannot open a book or
        // activate a control underneath the disappearing menu.
        if self.library_action_book_id.is_some() {
            let press = match event {
                Event::MouseDown(e) => Some(e.abs),
                Event::TouchUpdate(e) => e
                    .touches
                    .iter()
                    .find(|touch| touch.state == TouchState::Start)
                    .map(|touch| touch.abs),
                _ => None,
            };
            if let Some(abs) = press {
                let menu =
                    current_clipped_rect(cx, self.ui.widget(cx, ids!(library_action_menu)).area());
                if !menu.contains(abs) {
                    self.close_library_action_menu(cx);
                    return;
                }
            }
        }
        if self.library_action_book_id.is_some()
            && (matches!(event, Event::KeyDown(key) if key.key_code == KeyCode::Escape)
                || event.back_pressed())
        {
            self.close_library_action_menu(cx);
        }
        let reader_swipe = self.track_reader_swipe(cx, event);
        let pointer_start = match event {
            Event::MouseDown(e) => Some(e.abs),
            Event::TouchUpdate(e) => e
                .touches
                .iter()
                .find(|touch| touch.state == TouchState::Start)
                .map(|touch| touch.abs),
            _ => None,
        };
        if let Some(abs) = pointer_start {
            if self.dismiss_reader_customise_dropdowns_outside(cx, abs) {
                return;
            }
        }
        if self.on_reader_page
            && (self.toc_open || self.bookmarks_open || self.note_open || self.search_open)
        {
            if matches!(event, Event::KeyDown(key) if key.key_code == KeyCode::Escape)
                || event.back_pressed()
            {
                self.close_reader_info_popup(cx);
                return;
            }
            if let Some(abs) = pointer_start {
                let panel_id = if self.toc_open {
                    id!(toc_panel)
                } else if self.bookmarks_open {
                    id!(reader_bookmarks_panel)
                } else if self.search_open {
                    id!(reader_search_panel)
                } else {
                    id!(note_panel)
                };
                let panel = current_clipped_rect(cx, self.ui.widget(cx, &[panel_id]).area());
                if !panel.contains(abs) {
                    let toolbar_id = if self.ui.widget(cx, ids!(reader_titlebar)).visible() {
                        id!(reader_titlebar)
                    } else {
                        id!(header)
                    };
                    let header = current_clipped_rect(cx, self.ui.widget(cx, &[toolbar_id]).area());
                    let info_trigger = header.contains(abs)
                        && [
                            id!(toc_btn),
                            id!(reader_bookmarks_btn),
                            id!(notes_btn),
                            id!(titlebar_toc_btn),
                            id!(titlebar_bookmarks_btn),
                            id!(titlebar_notes_btn),
                            id!(reader_search_btn),
                            id!(titlebar_search_btn),
                        ]
                        .into_iter()
                        .any(|id| {
                            current_clipped_rect(cx, self.ui.widget(cx, &[id]).area()).contains(abs)
                        });
                    if !info_trigger {
                        self.close_reader_info_popup(cx);
                        if !header.contains(abs) {
                            return;
                        }
                    }
                }
            }
        }
        if self.reader_theme_open {
            if let Some(abs) = pointer_start {
                let menu =
                    current_clipped_rect(cx, self.ui.widget(cx, ids!(reader_theme_menu)).area());
                let trigger_hit = [id!(reader_appearance_btn), id!(titlebar_appearance_btn)]
                    .into_iter()
                    .any(|id| {
                        current_clipped_rect(cx, self.ui.widget(cx, &[id]).area()).contains(abs)
                    });
                if !menu.contains(abs) && !trigger_hit {
                    self.reader_theme_open = false;
                    self.ui
                        .widget(cx, ids!(reader_theme_menu))
                        .set_visible(cx, false);
                    self.ui
                        .widget(cx, ids!(reader_theme_arrow_row))
                        .set_visible(cx, false);
                    self.ui.widget(cx, ids!(reader_theme_layer)).redraw(cx);
                }
            }
        }
        if let Some(abs) = pointer_start {
            let selection_popup_open = self.on_reader_page
                && self.selection_popup_anchor.is_some()
                && self.ui.widget(cx, ids!(sel_bar)).visible();
            let selection_exists = SEL_RANGE.read().unwrap().is_some();
            let popup_captures_pointer = selection_popup_captures_pointer(
                selection_popup_open,
                selection_exists,
                || current_clipped_rect(cx, self.ui.widget(cx, ids!(selection_popup)).area()),
                abs,
            );
            SELECTION_POPUP_POINTER.store(
                popup_captures_pointer
                    || card_captures_pointer
                    || (self.on_reader_page
                        && self.compact_layout
                        && self.reader_toolbar_visible
                        && current_clipped_rect(cx, self.ui.widget(cx, ids!(header)).area())
                            .contains(abs)),
                Ordering::Relaxed,
            );
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
                {
                    let mut data = CHAT_DATA.write().unwrap();
                    data.streaming_text.push_str(&stream_text);
                    data.streaming_revision = data.streaming_revision.wrapping_add(1);
                }
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
                            {
                                let mut data = CHAT_DATA.write().unwrap();
                                data.streaming_text.push_str(&text);
                                data.streaming_revision = data.streaming_revision.wrapping_add(1);
                            }
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
            Event::LiveEdit | Event::ScriptReapply => self.restore_after_script_reload(cx),
            Event::Startup => {
                let _ = APP_STARTED_AT.set(Instant::now());
                #[cfg(target_os = "macos")]
                crate::desktop_window::ensure_reader_minimum_content_width(
                    DESKTOP_MIN_WINDOW_WIDTH,
                );
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
                #[cfg(target_os = "macos")]
                {
                    let main_window = self.ui.window(cx, ids!(main_window));
                    if main_window.window_id() == Some(event.window_id) {
                        crate::desktop_window::ensure_reader_minimum_content_width(
                            DESKTOP_MIN_WINDOW_WIDTH,
                        );
                    }
                }
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
            Event::MouseMove(event) if self.on_reader_page && !self.compact_layout => {
                let (toolbar_id, toolbar_height) =
                    if self.ui.widget(cx, ids!(reader_titlebar)).visible() {
                        (id!(reader_titlebar), 36.0)
                    } else {
                        (id!(header), 54.0)
                    };
                let header = current_clipped_rect(cx, self.ui.widget(cx, &[toolbar_id]).area());
                // The caption view can retain a stale Area after a theme
                // reapply. Its top strip is still the toolbar hover target.
                let over_header = if header.size.y > 0.0 {
                    header.contains(event.abs)
                } else {
                    event.abs.y >= 0.0 && event.abs.y <= toolbar_height
                };
                let visible = over_header
                    || self.reader_theme_open
                    || self.toc_open
                    || self.bookmarks_open
                    || self.note_open
                    || self.search_open;
                if visible != self.reader_toolbar_visible {
                    self.set_reader_toolbar_visible(cx, visible);
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
        // ChapterList is drawn underneath the non-modal aA overlay. Mirror the
        // owner state before dispatch so its wheel/touch/key handlers cannot
        // turn pages or begin selection while the colour panel is scrolling.
        READER_APPEARANCE_OPEN.store(
            self.on_reader_page && self.reader_theme_open,
            Ordering::Release,
        );
        self.ui.handle_event(cx, event, &mut Scope::empty());
        if matches!(event, Event::Draw(_) | Event::WindowGeomChange(_)) {
            self.sync_reader_toolbar_placement(cx);
        }
        if let Event::WindowDragQuery(query) = event {
            // Makepad treats the whole caption bar as a window drag target.
            // The button groups disappear when the toolbar is idle, so their
            // Areas can be empty when the drag query arrives. Their enclosing
            // slots stay laid out and keep clicks out of titlebar drag handling.
            if self.on_reader_page
                && self.ui.widget(cx, ids!(reader_titlebar)).visible()
                && [id!(reader_titlebar_left), id!(reader_titlebar_right)]
                    .into_iter()
                    .any(|id| {
                        current_clipped_rect(cx, self.ui.widget(cx, &[id]).area())
                            .contains(query.abs)
                    })
            {
                query.response.set(WindowDragQueryResponse::Client);
            }
        }
        if matches!(event, Event::Draw(_)) {
            self.finish_toc_first_draw(cx);
            if self.on_reader_page {
                self.load_reader_font(cx, *READER_FONT.read().unwrap(), false);
            }
            if self.pending_customise_preview_after_draw {
                self.pending_customise_preview_after_draw = false;
                if self.reader_customise_draft.is_some() {
                    self.load_reader_font(
                        cx,
                        self.reader_customise_draft.as_ref().unwrap().font,
                        true,
                    );
                    self.sync_reader_customise_controls(cx);
                }
            }
            if self.pending_ui_projection_after_draw {
                self.pending_ui_projection_after_draw = false;
                self.restore_projected_ui_after_draw(cx);
            }
            self.resolve_reader_pagination_layout_after_draw(cx);
            if self.pending_selection_popup_layout {
                self.pending_selection_popup_layout = false;
                if self.position_selection_popup(cx, true) {
                    // Apply the measured position in a fresh frame. Updating
                    // a cached draw list after Draw leaves its pixels behind.
                    self.selection_popup_layout_frame = Some(cx.new_next_frame());
                }
            }
            if self.pending_word_card_layout {
                self.pending_word_card_layout = false;
                if self.position_word_card(cx) {
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
            if !self.virtual_keyboard_open
                && self.on_reader_page
                && !self.toc_open
                && !self.note_open
                && !self.reader_ai_open
                && selection_is_clear
            {
                self.page_scroll(cx, direction == ReaderSwipeDirection::Next);
            }
        }
        if matches!(event, Event::MouseUp(_))
            || matches!(event, Event::TouchUpdate(e) if e.touches.iter().any(|t| t.state == TouchState::Stop))
        {
            SELECTION_POPUP_POINTER.store(false, Ordering::Relaxed);
        }
    }
}
