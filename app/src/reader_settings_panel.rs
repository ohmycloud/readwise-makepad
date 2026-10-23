//! Presentation and section ownership for the reader's three-way aA editor.
//! The App keeps a live editing snapshot; control changes apply immediately.
use makepad_widgets::*;
use readwise_core::settings::ReaderBackground;

pub fn theme_modes() -> [(LiveId, ReaderBackground); 6] {
    [
        (id!(reader_mode_original), ReaderBackground::Original),
        (id!(reader_mode_quiet), ReaderBackground::Quiet),
        (id!(reader_mode_paper), ReaderBackground::Paper),
        (id!(reader_mode_sepia), ReaderBackground::Sepia),
        (id!(reader_mode_calm), ReaderBackground::Sage),
        (id!(reader_mode_snow), ReaderBackground::Snow),
    ]
}

/// Only shortcut an isolated theme click. Mixed data, navigation or multiple
/// clicks must continue through the normal action router without losing work.
pub fn theme_button_choice(
    actions: &Actions,
    buttons: &[(WidgetUid, ReaderBackground)],
) -> Option<ReaderBackground> {
    let mut choice = None;
    for action in actions {
        let widget = action.as_widget_action()?;
        if matches!(
            widget.action.downcast_ref::<ViewAction>(),
            Some(
                ViewAction::FingerDown(_)
                    | ViewAction::FingerMove(_)
                    | ViewAction::FingerHoverIn(_)
                    | ViewAction::FingerHoverOut(_)
            )
        ) {
            continue;
        }
        let (_, mode) = buttons
            .iter()
            .find(|(uid, _)| uid.0 != 0 && *uid == widget.widget_uid)?;
        if !matches!(
            widget.action.downcast_ref::<ButtonAction>(),
            Some(ButtonAction::Clicked(_))
        ) || choice.is_some()
        {
            return None;
        }
        choice = Some(*mode);
    }
    choice
}

#[cfg(test)]
mod theme_action_tests {
    use super::*;
    fn clicked(uid: u64) -> Action {
        Box::new(WidgetAction {
            data: None,
            action: Box::new(ButtonAction::Clicked(Default::default())),
            widget_uid: WidgetUid(uid),
            group: None,
        })
    }
    #[test]
    fn theme_choices_are_unique_and_include_sepia_and_snow() {
        let modes = theme_modes();
        for (index, (id, background)) in modes.iter().enumerate() {
            assert!(!modes[..index].iter().any(|(other_id, other_background)| {
                id == other_id || background == other_background
            }));
        }
        assert_eq!(modes[3], (id!(reader_mode_sepia), ReaderBackground::Sepia));
        assert_eq!(modes[5], (id!(reader_mode_snow), ReaderBackground::Snow));
        for (key, zh, en) in [
            ("reader_mode_sepia", "复古", "Sepia"),
            ("reader_mode_snow", "雪白", "Snow"),
        ] {
            assert_eq!(
                crate::i18n::text(readwise_core::settings::AppLanguage::Chinese, key),
                zh
            );
            assert_eq!(
                crate::i18n::text(readwise_core::settings::AppLanguage::English, key),
                en
            );
        }
    }

    #[test]
    fn isolated_theme_click_uses_short_path_but_mixed_actions_do_not() {
        let buttons = [(WidgetUid(1), ReaderBackground::Night)];
        assert_eq!(
            theme_button_choice(&vec![clicked(1)], &buttons),
            Some(ReaderBackground::Night)
        );
        assert_eq!(theme_button_choice(&vec![clicked(2)], &buttons), None);
        assert_eq!(
            theme_button_choice(
                &vec![clicked(1), Box::new("data notice".to_string())],
                &buttons
            ),
            None
        );
        assert_eq!(
            theme_button_choice(&vec![clicked(1), clicked(1)], &buttons),
            None
        );
        assert_eq!(theme_button_choice(&vec![], &buttons), None);
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ReaderSettingsTab {
    #[default]
    Theme,
    Text,
    Annotations,
}

impl ReaderSettingsTab {
    pub const ALL: [Self; 3] = [Self::Theme, Self::Text, Self::Annotations];

    pub fn nominal_height(self) -> f64 {
        match self {
            Self::Theme => POPOVER_HEIGHT,
            Self::Text => 416.0,
            Self::Annotations => 580.0,
        }
    }

    pub fn button(self) -> LiveId {
        match self {
            Self::Theme => id!(reader_settings_tab_theme),
            Self::Text => id!(reader_settings_tab_text),
            Self::Annotations => id!(reader_settings_tab_annotations),
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            Self::Theme => "reader_settings_theme",
            Self::Text => "text_section",
            Self::Annotations => "reader_settings_annotations",
        }
    }
}

/// Keep the compact width; longer sections get enough height for their controls.
pub const POPOVER_WIDTH: f64 = 316.0;
pub const POPOVER_HEIGHT: f64 = 400.0;
pub const ARROW_HEIGHT: f64 = 10.0;

/// Annotation labels, four swatches and the selected name share one row.
/// Keep room for English names and allow its longer labels to wrap internally,
/// rather than moving the entire picker below the label on compact cards.
#[derive(Clone, Copy, Debug)]
pub struct AnnotationRowLayout {
    pub label_width: f64,
    pub name_width: f64,
    pub swatch_width: f64,
}

impl AnnotationRowLayout {
    pub const GAP: f64 = 6.0;
    pub const SWATCH_GAP: f64 = 2.0;

    pub fn new(content_width: f64, english: bool) -> Self {
        let label_width = if english { 80.0 } else { 64.0 };
        let name_width = if english { 68.0 } else { 32.0 };
        let swatch_width =
            ((content_width - label_width - Self::GAP - name_width - Self::SWATCH_GAP * 4.0) / 4.0)
                .clamp(1.0, 34.0);
        Self {
            label_width,
            name_width,
            swatch_width,
        }
    }
}

pub fn annotation_rows() -> [(LiveId, LiveId, &'static str); 5] {
    [
        (
            id!(reader_customise_gloss_size_row),
            id!(reader_customise_gloss_size_label),
            "reader_annotation_size",
        ),
        (
            id!(reader_customise_gloss_color_row),
            id!(reader_customise_gloss_color_label),
            "reader_annotation_color",
        ),
        (
            id!(reader_customise_gloss_bg_row),
            id!(reader_customise_gloss_bg_label),
            "reader_annotation_background",
        ),
        (
            id!(reader_customise_word_color_row),
            id!(reader_customise_word_color_label),
            "word_color",
        ),
        (
            id!(reader_customise_word_bg_row),
            id!(reader_customise_word_bg_label),
            "word_background",
        ),
    ]
}

pub fn annotation_title(
    language: readwise_core::settings::AppLanguage,
    key: &'static str,
) -> String {
    let title = crate::i18n::text(language, key);
    // Explicit breaks avoid clipping long English titles inside the label
    // column, without wrapping the picker itself onto another row.
    if language == readwise_core::settings::AppLanguage::English {
        title.replace(' ', "\n")
    } else {
        title.to_string()
    }
}

/// Card bounds and arrow centre in card-local coordinates. Only viewport/safe
/// area constraints may shrink the preferred height; overflow remains scrollable.
pub fn popover_layout(
    size: DVec2,
    insets: SafeAreaInsets,
    trigger: Option<Rect>,
    tab: ReaderSettingsTab,
) -> (Rect, f64) {
    let left = insets.left + 12.0;
    let right = (size.x - insets.right - if size.x < 700.0 { 12.0 } else { 18.0 }).max(left + 1.0);
    let bottom = (size.y - insets.bottom - 12.0).max(insets.top + 1.0);
    let width = POPOVER_WIDTH.min((right - left).max(1.0));
    let anchor = trigger.filter(|r| {
        r.size.x > 0.0
            && r.size.y > 0.0
            && r.pos.x >= 0.0
            && r.pos.x + r.size.x <= size.x + 1.0
            && r.pos.y + r.size.y < bottom
    });
    let (centre, top) = anchor
        .map(|r| {
            (
                r.pos.x + r.size.x * 0.5,
                r.pos.y + r.size.y + 2.0 + ARROW_HEIGHT,
            )
        })
        .unwrap_or((right - 80.0, insets.top + 54.0 + ARROW_HEIGHT));
    let top = top
        .max(insets.top + 12.0 + ARROW_HEIGHT)
        .min((bottom - 1.0).max(insets.top));
    let rect = Rect {
        pos: dvec2(right - width, top),
        size: dvec2(width, tab.nominal_height().min((bottom - top).max(1.0))),
    };
    let guard = 16.0_f64.min(width * 0.5);
    (
        rect,
        (centre - rect.pos.x).clamp(guard, (width - guard).max(guard)),
    )
}

/// Native Button::was_tap only compares the final position. Keep the whole
/// gesture, but leave release events flowing to widgets to clear capture.
#[derive(Default)]
pub struct FontMenuTap {
    gesture: crate::reader_touch::DoubleTap,
    allowed: bool,
}

impl FontMenuTap {
    pub fn cancel(&mut self) {
        self.gesture.cancel();
        self.allowed = false;
    }

    pub fn track(&mut self, event: &Event, rect: Rect) {
        let outcome = self.gesture.track(event, rect);
        match event {
            Event::KeyDown(_) | Event::KeyUp(_) => self.allowed = true,
            Event::TouchUpdate(_)
            | Event::MouseDown(_)
            | Event::MouseMove(_)
            | Event::MouseUp(_)
            | Event::LongPress(_)
            | Event::Scroll(_)
            | Event::MouseLeave(_)
            | Event::WindowLostFocus(_)
            | Event::WindowGeomChange(_)
            | Event::LiveEdit
            | Event::ScriptReapply => {
                self.allowed = !matches!(outcome, crate::reader_touch::TapOutcome::None)
            }
            _ => {} // Actions/Draw must preserve the preceding release verdict.
        }
    }

    pub fn allows_click(&self) -> bool {
        self.allowed
    }
}

/// A short specimen demonstrates all four annotation paints without clipping
/// a multi-sentence chapter excerpt in the narrow settings column.
pub fn gloss_preview(enabled: bool) -> String {
    if enabled {
        "<p>Be <ruby t=\"沉默寡言的\">taciturn</ruby>.</p>"
    } else {
        "<p>Be taciturn.</p>"
    }
    .to_owned()
}

pub fn font_option_ids() -> [LiveId; 14] {
    [
        id!(reader_font_canela),
        id!(reader_font_original),
        id!(reader_font_athelas),
        id!(reader_font_avenir),
        id!(reader_font_charter),
        id!(reader_font_georgia),
        id!(reader_font_iowan),
        id!(reader_font_palatino),
        id!(reader_font_proxima),
        id!(reader_font_publico),
        id!(reader_font_san_francisco),
        id!(reader_font_new_york),
        id!(reader_font_seravek),
        id!(reader_font_times),
    ]
}

/// Always unfold below the field, independent of the selected font. In short
/// viewports shrink the scroll viewport, never flip the menu above the field.
pub fn font_menu_rect(size: DVec2, insets: SafeAreaInsets, input: Rect) -> Rect {
    let left = insets.left + 12.0;
    let right = (size.x - insets.right - 12.0).max(left + 1.0);
    let top = insets.top + 12.0;
    let bottom = (size.y - insets.bottom - 12.0).max(top + 1.0);
    let width = 180.0_f64.min(right - left);
    let menu_top = input.pos.y + input.size.y + 4.0;
    let height = 264.0_f64.min((bottom - menu_top).max(1.0));
    Rect {
        pos: dvec2(
            (input.pos.x + input.size.x - width).clamp(left, right - width),
            menu_top,
        ),
        size: dvec2(width, height),
    }
}

/// Only top-level scroll children belong here. Nested colour controls follow
/// their section owner; enabling annotations must not expose them in other tabs.
pub fn section_widgets() -> [(LiveId, ReaderSettingsTab); 18] {
    use ReaderSettingsTab::*;
    [
        (id!(reader_customise_theme_section), Theme),
        (id!(reader_customise_font_row), Text),
        (id!(reader_customise_size_row), Text),
        (id!(reader_customise_line_row), Text),
        (id!(reader_customise_margin_row), Text),
        (id!(reader_customise_columns_row), Text),
        (id!(reader_customise_align_row), Text),
        (id!(reader_customise_paragraph_row), Text),
        (id!(reader_customise_text_reset), Text),
        (id!(reader_preview), Annotations),
        (id!(reader_customise_gloss_title), Annotations),
        (id!(reader_customise_gloss_enable), Annotations),
        (id!(reader_customise_gloss_size_row), Annotations),
        (id!(reader_customise_gloss_color_row), Annotations),
        (id!(reader_customise_gloss_bg_row), Annotations),
        (id!(reader_customise_word_color_row), Annotations),
        (id!(reader_customise_word_bg_row), Annotations),
        (id!(reader_customise_annotations_reset), Annotations),
    ]
}

/// Stable choices shared by action handling, localization and selected paints.
#[derive(Clone, Copy, Debug)]
pub enum TextChoice {
    Spacing(f32),
    Width(readwise_core::settings::ReaderTextWidth),
    Justify(bool),
    Indent(bool),
    Columns(readwise_core::settings::ReaderColumns),
}

impl TextChoice {
    pub fn selected(self, settings: &readwise_core::settings::ReaderSettings) -> bool {
        match self {
            Self::Spacing(value) => (settings.spacing.plain_line_spacing() - value).abs() < 0.005,
            Self::Width(width) => settings.text_width == width,
            Self::Justify(justify) => settings.justify_text == justify,
            Self::Indent(indent) => settings.indent_paragraphs == indent,
            Self::Columns(columns) => settings.columns == columns,
        }
    }

    pub fn apply(self, settings: &mut readwise_core::settings::ReaderSettings) {
        match self {
            Self::Spacing(value) => {
                settings.spacing = readwise_core::settings::ReaderSpacing::from_value(value)
            }
            Self::Width(width) => settings.text_width = width,
            Self::Justify(justify) => settings.justify_text = justify,
            Self::Indent(indent) => settings.indent_paragraphs = indent,
            Self::Columns(columns) => settings.columns = columns,
        }
    }
}

pub fn text_choices() -> [(LiveId, &'static str, TextChoice); 13] {
    use readwise_core::settings::ReaderTextWidth::*;
    use TextChoice::*;
    [
        (
            id!(reader_spacing_snug),
            "reader_spacing_snug",
            Spacing(1.3),
        ),
        (id!(reader_spacing_even), "reader_text_even", Spacing(1.55)),
        (
            id!(reader_spacing_airy),
            "reader_spacing_airy",
            Spacing(1.85),
        ),
        (
            id!(reader_width_narrow),
            "reader_width_narrow",
            Width(Narrow),
        ),
        (id!(reader_width_even), "reader_text_even", Width(Even)),
        (id!(reader_width_wide), "reader_width_wide", Width(Wide)),
        (id!(reader_align_left), "reader_align_left", Justify(false)),
        (
            id!(reader_align_justified),
            "reader_align_justified",
            Justify(true),
        ),
        (
            id!(reader_paragraph_spaced),
            "reader_paragraph_spaced",
            Indent(false),
        ),
        (
            id!(reader_paragraph_indented),
            "reader_paragraph_indented",
            Indent(true),
        ),
        (
            id!(reader_customise_columns_one),
            "reader_columns_one",
            Columns(readwise_core::settings::ReaderColumns::One),
        ),
        (
            id!(reader_customise_columns_two),
            "reader_columns_two",
            Columns(readwise_core::settings::ReaderColumns::Two),
        ),
        (
            id!(reader_customise_columns_auto),
            "reader_columns_auto",
            Columns(readwise_core::settings::ReaderColumns::Auto),
        ),
    ]
}

pub fn section_visible(
    tab: ReaderSettingsTab,
    owner: ReaderSettingsTab,
    id: LiveId,
    gloss_enabled: bool,
) -> bool {
    tab == owner
        && (gloss_enabled
            || ![
                id!(reader_customise_gloss_size_row),
                id!(reader_customise_gloss_color_row),
                id!(reader_customise_gloss_bg_row),
                id!(reader_customise_word_color_row),
                id!(reader_customise_word_bg_row),
            ]
            .contains(&id))
}

script_mod! {
    use mod.prelude.widgets.*

    mod.widgets.ReaderFontMenuItem = ButtonFlat{
        width: Fill height: 24 margin: 0
        align: Align{x: 0.0 y: 0.5}
        padding: Inset{left: 15 right: 3 top: 3 bottom: 3}
        enable_long_press: true
        draw_bg +: {
            // Match PopupMenuItem's checkmark and row highlight. Reuse the
            // existing border_size uniform as the persistent selection flag.
            border_size: 0.0
            pixel: fn() {
                let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                sdf.box(0.0, 0.0, self.rect_size.x, self.rect_size.y, 3.0)
                sdf.fill_keep(self.color)
                let highlight = mix(self.color_hover, self.color_down, self.down)
                sdf.fill(vec4(highlight.xyz, highlight.w * max(self.border_size, self.hover)))
                if self.border_size > 0.5 {
                    let cy = self.rect_size.y * 0.5
                    sdf.move_to(6.0, cy)
                    sdf.line_to(8.0, cy + 3.0)
                    sdf.line_to(11.0, cy - 3.0)
                    sdf.stroke(self.border_color, 1.0)
                }
                return sdf.result
            }
        }
    }

    mod.widgets.ReaderFontCaret = View{
        width: 10 height: 10 show_bg: true
        draw_bg +: {
            color: #666
            pixel: fn() {
                let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                sdf.move_to(2.0, 3.5)
                sdf.line_to(5.0, 6.5)
                sdf.line_to(8.0, 3.5)
                sdf.stroke(self.color, 0.7)
                return sdf.result
            }
        }
    }

    mod.widgets.ReaderSettingsSegment = ButtonFlat{
        width: Fill height: 44 margin: 0 padding: 0
        enable_long_press: true
        draw_text.text_style.font_size: 13
        draw_bg +: {
            // Reuse DrawButton's input as the capsule flag; ButtonFlat's
            // inherited shader instance layout is already frozen.
            border_size: 0.0
            color: #fff
            pixel: fn() {
                let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                let width = self.rect_size.x - 4.0
                let height = self.rect_size.y - 4.0
                if self.border_size > 0.5 {
                    sdf.box(2.0, 3.0, width, height, 9.0)
                    sdf.fill(vec4(0.0, 0.0, 0.0, 0.08))
                    sdf.box(2.0, 1.0, width, height, 9.0)
                    sdf.fill(self.color)
                } else {
                    sdf.box(2.0, 1.0, width, height, 9.0)
                    sdf.fill(vec4(self.border_color.xyz, max(self.hover, self.focus) * 0.07))
                }
                if self.focus > 0.0 {
                    sdf.box(2.0, 1.0, width, height, 9.0)
                    sdf.stroke(vec4(self.border_color.xyz, self.focus * 0.4), 0.5)
                }
                return sdf.result
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use makepad_widgets::makepad_platform::event::TouchState;

    fn touch(time: f64, state: TouchState, x: f64) -> Event {
        use makepad_widgets::makepad_platform::event::{TouchPoint, TouchUpdateEvent};
        Event::TouchUpdate(TouchUpdateEvent {
            time,
            window_id: WindowId(0, 0),
            modifiers: Default::default(),
            touches: vec![TouchPoint {
                state,
                abs: dvec2(x, 20.0),
                time,
                uid: 1,
                rotation_angle: 0.0,
                force: 1.0,
                radius: dvec2(1.0, 1.0),
                handled: Default::default(),
                sweep_lock: Default::default(),
            }],
        })
    }

    #[test]
    fn font_actions_reject_backtracking_and_recover_on_the_next_real_tap() {
        let rect = Rect {
            pos: dvec2(0.0, 0.0),
            size: dvec2(180.0, 264.0),
        };
        let mut tap = FontMenuTap::default();
        for event in [
            touch(1.0, TouchState::Start, 20.0),
            touch(1.05, TouchState::Move, 80.0),
            touch(1.1, TouchState::Move, 20.0),
            touch(1.2, TouchState::Stop, 20.0),
        ] {
            tap.track(&event, rect);
        }
        assert!(!tap.allows_click());
        tap.track(&touch(2.0, TouchState::Start, 20.0), rect);
        tap.track(&touch(2.1, TouchState::Stop, 21.0), rect);
        assert!(tap.allows_click());
        tap.track(&Event::Actions(Default::default()), rect);
        assert!(tap.allows_click());
    }

    #[test]
    fn font_actions_reject_long_press_multi_touch_and_reload() {
        let rect = Rect {
            pos: dvec2(0.0, 0.0),
            size: dvec2(180.0, 264.0),
        };
        for (cancel, end) in [
            (None, 1.9),
            (Some(Event::ScriptReapply), 1.1),
            (Some(Event::LiveEdit), 1.1),
        ] {
            let mut tap = FontMenuTap::default();
            tap.track(&touch(1.0, TouchState::Start, 20.0), rect);
            if let Some(event) = cancel {
                tap.track(&event, rect);
            }
            tap.track(&touch(end, TouchState::Stop, 20.0), rect);
            assert!(!tap.allows_click());
        }
        let mut tap = FontMenuTap::default();
        tap.track(&touch(2.0, TouchState::Start, 20.0), rect);
        let mut multi = touch(2.05, TouchState::Move, 20.0);
        if let Event::TouchUpdate(event) = &mut multi {
            let mut second = event.touches[0].clone();
            second.uid = 2;
            event.touches.push(second);
        }
        tap.track(&multi, rect);
        tap.track(&touch(2.1, TouchState::Stop, 20.0), rect);
        assert!(!tap.allows_click());
    }

    #[test]
    fn section_heights_fit_contents_while_preserving_compact_width_and_pointer() {
        let trigger = Rect {
            pos: dvec2(966.0, 9.0),
            size: dvec2(36.0, 36.0),
        };
        let (r, arrow) = popover_layout(
            dvec2(1100.0, 1000.0),
            Default::default(),
            Some(trigger),
            ReaderSettingsTab::Theme,
        );
        assert_eq!(r.size, dvec2(316.0, 400.0));
        assert_eq!(r.pos, dvec2(766.0, 57.0));
        assert_eq!(r.pos.x + arrow, trigger.pos.x + trigger.size.x * 0.5);
        for tab in ReaderSettingsTab::ALL {
            let frame = popover_layout(
                dvec2(1100.0, 1000.0),
                Default::default(),
                Some(trigger),
                tab,
            )
            .0;
            assert_eq!(frame.pos, r.pos);
            assert_eq!(frame.size, dvec2(316.0, tab.nominal_height()));
        }
        assert!(
            ReaderSettingsTab::Text.nominal_height() > ReaderSettingsTab::Theme.nominal_height()
        );
        assert!(
            ReaderSettingsTab::Annotations.nominal_height()
                > ReaderSettingsTab::Text.nominal_height()
        );
    }

    #[test]
    fn annotation_reset_is_last_and_available_when_annotations_are_disabled() {
        let reset = id!(reader_customise_annotations_reset);
        assert!(section_widgets().contains(&(reset, ReaderSettingsTab::Annotations)));
        assert!(section_visible(
            ReaderSettingsTab::Annotations,
            ReaderSettingsTab::Annotations,
            reset,
            false
        ));
        assert!(!section_visible(
            ReaderSettingsTab::Text,
            ReaderSettingsTab::Annotations,
            reset,
            true
        ));
        for language in [
            readwise_core::settings::AppLanguage::Chinese,
            readwise_core::settings::AppLanguage::English,
        ] {
            assert_ne!(
                crate::i18n::text(language, "reader_annotations_reset"),
                "reader_annotations_reset"
            );
        }
        let source = include_str!("app.rs");
        let background = source.find("reader_customise_word_bg_row :=").unwrap();
        let reset = source
            .find("reader_customise_annotations_reset :=")
            .unwrap();
        let next = source.find("reader_customise_line_row :=").unwrap();
        assert!(background < reset && reset < next);
    }

    #[test]
    fn runtime_popover_layout_uses_rust_values_not_unbound_dsl_symbols() {
        let source = include_str!("app.rs");
        let layout = source
            .split("fn configure_reader_customise_layout(")
            .nth(1)
            .unwrap()
            .split("fn close_reader_customise_dropdowns(")
            .next()
            .unwrap();
        for call in layout.split("script_apply_eval!").skip(1) {
            let code = call.split("});").next().unwrap();
            // Interpolated Rust values are safe; inspect only evaluated Splash.
            let mut evaluated = String::new();
            let mut chars = code.chars().peekable();
            while let Some(ch) = chars.next() {
                if ch == '#' && chars.peek() == Some(&'(') {
                    chars.next();
                    let mut depth = 1;
                    for ch in chars.by_ref() {
                        if ch == '(' {
                            depth += 1;
                        }
                        if ch == ')' {
                            depth -= 1;
                        }
                        if depth == 0 {
                            break;
                        }
                    }
                } else {
                    evaluated.push(ch);
                }
            }
            for token in evaluated.split(|c: char| !c.is_alphanumeric() && c != '_') {
                assert!(
                    !["Fit", "Fill", "Flow", "Right", "Down", "Align", "Inset"].contains(&token),
                    "unbound runtime DSL symbol {token}: {code}"
                );
            }
        }
    }

    #[test]
    fn annotation_titles_remain_readable_after_language_projection() {
        use readwise_core::settings::AppLanguage;
        let rows = annotation_rows();
        for (index, (row, label, key)) in rows.iter().enumerate() {
            assert!(!rows[..index].iter().any(|(r, l, _)| r == row || l == label));
            let chinese = annotation_title(AppLanguage::Chinese, key);
            let english = annotation_title(AppLanguage::English, key);
            assert!(!chinese.contains('\n'));
            assert_eq!(english.lines().count(), 2);
            assert_eq!(
                english.replace('\n', " "),
                crate::i18n::text(AppLanguage::English, key)
            );
        }
    }

    #[test]
    fn annotation_rows_fit_beside_labels_in_both_languages() {
        for (size, safe) in [
            (dvec2(1100.0, 1000.0), SafeAreaInsets::default()),
            (
                dvec2(320.0, 568.0),
                SafeAreaInsets {
                    left: 8.0,
                    right: 8.0,
                    top: 24.0,
                    bottom: 16.0,
                },
            ),
            (dvec2(300.0, 568.0), SafeAreaInsets::default()),
            (
                dvec2(844.0, 390.0),
                SafeAreaInsets {
                    left: 44.0,
                    right: 44.0,
                    bottom: 20.0,
                    ..Default::default()
                },
            ),
        ] {
            let (card, _) = popover_layout(size, safe, None, ReaderSettingsTab::Annotations);
            let content = card.size.x - 32.0;
            for english in [false, true] {
                let row = AnnotationRowLayout::new(content, english);
                let used = row.label_width
                    + AnnotationRowLayout::GAP
                    + row.swatch_width * 4.0
                    + AnnotationRowLayout::SWATCH_GAP * 4.0
                    + row.name_width;
                assert!(
                    used <= content + 0.01,
                    "{size:?}, english={english}: {used} > {content}"
                );
                assert!(row.swatch_width >= 20.0 && row.swatch_width <= 34.0);
                // A useful slider remains after the label, value and gaps.
                assert!(content - row.label_width - 28.0 - 14.0 >= 100.0);
            }
        }
    }

    #[test]
    fn compact_popover_stays_inside_all_safe_edges() {
        for (size, safe, trigger) in [
            (
                dvec2(320.0, 568.0),
                SafeAreaInsets {
                    left: 8.0,
                    right: 8.0,
                    top: 24.0,
                    bottom: 16.0,
                },
                Rect {
                    pos: dvec2(185.0, 68.0),
                    size: dvec2(36.0, 36.0),
                },
            ),
            (
                dvec2(844.0, 390.0),
                SafeAreaInsets {
                    left: 44.0,
                    right: 44.0,
                    top: 0.0,
                    bottom: 20.0,
                },
                Rect {
                    pos: dvec2(620.0, 44.0),
                    size: dvec2(44.0, 44.0),
                },
            ),
        ] {
            let (r, arrow) =
                popover_layout(size, safe, Some(trigger), ReaderSettingsTab::Annotations);
            assert!(r.pos.x >= safe.left + 12.0 && r.pos.y - ARROW_HEIGHT >= safe.top);
            assert!(
                r.pos.x + r.size.x <= size.x - safe.right - 12.0
                    && r.pos.y + r.size.y <= size.y - safe.bottom - 12.0
            );
            assert!(
                r.size.y <= ReaderSettingsTab::Annotations.nominal_height() && r.size.y >= 200.0
            );
            assert!(arrow >= 16.0 && arrow <= r.size.x - 16.0);
        }
    }

    #[test]
    fn stale_trigger_after_rotation_cannot_push_the_popover_offscreen() {
        let stale = Rect {
            pos: dvec2(966.0, 9.0),
            size: dvec2(36.0, 36.0),
        };
        let (r, arrow) = popover_layout(
            dvec2(300.0, 568.0),
            Default::default(),
            Some(stale),
            ReaderSettingsTab::Theme,
        );
        assert!(r.pos.x >= 12.0 && r.pos.x + r.size.x <= 288.0);
        assert!(arrow >= 16.0 && arrow <= r.size.x - 16.0);
    }

    #[test]
    fn font_menu_is_bounded_and_has_every_font_once() {
        let ids = font_option_ids();
        assert_eq!(ids.len(), readwise_core::settings::ReaderFont::ALL.len());
        for (i, id) in ids.iter().enumerate() {
            assert!(!ids[..i].contains(id));
        }
        let safe = SafeAreaInsets {
            left: 44.0,
            right: 44.0,
            top: 0.0,
            bottom: 20.0,
        };
        for y in [173.0, 320.0] {
            let r = font_menu_rect(
                dvec2(844.0, 390.0),
                safe,
                Rect {
                    pos: dvec2(586.0, y),
                    size: dvec2(180.0, 32.0),
                },
            );
            assert_eq!(r.pos.y, y + 32.0 + 4.0);
            assert!(r.pos.y + r.size.y <= 358.0);
            assert!(r.pos.x >= 56.0 && r.pos.x + r.size.x <= 788.0);
            assert!(r.size.y < 14.0 * 44.0); // overflowing options belong in the scroll view
        }
    }

    #[test]
    fn every_native_dropdown_unfolds_below_input_not_selected_item() {
        let source = include_str!("app.rs");
        let mut controls = Vec::new();
        for block in source.split(" := DropDown{").skip(1) {
            let properties = block.split("labels:").next().unwrap();
            assert!(
                properties
                    .contains("popup_menu_position: mod.widgets.PopupMenuPosition.BelowInput"),
                "every dropdown must opt out of Makepad's OnSelected default"
            );
            controls.push(properties);
        }
        assert_eq!(controls.len(), 3, "audit newly added dropdowns too");
    }

    #[test]
    fn compact_annotation_specimen_keeps_text_when_gloss_is_disabled() {
        assert!(gloss_preview(true).contains("<ruby t=\"沉默寡言的\">taciturn</ruby>"));
        assert_eq!(gloss_preview(false), "<p>Be taciturn.</p>");
    }

    #[test]
    fn live_settings_have_no_commit_buttons_and_apply_control_actions() {
        let source = include_str!("app.rs");
        assert!(!source.contains("reader_customise_cancel"));
        assert!(!source.contains("reader_customise_save"));
        assert!(!source.contains("reader_customise_actions"));
        let card = source
            .split("reader_customise_card := RoundedShadowView{")
            .nth(1)
            .unwrap();
        let header = card.split("reader_customise_scroll :=").next().unwrap();
        assert!(
            header.contains("event_order: #(EventOrder::Down)"),
            "scrolled-out content must not capture the fixed segment header"
        );
        assert!(source.contains("self.apply_reader_customise_live(cx);"));
    }

    #[test]
    fn segments_have_stable_order_unique_ids_and_localized_names() {
        assert_eq!(ReaderSettingsTab::default(), ReaderSettingsTab::Theme);
        assert_eq!(
            ReaderSettingsTab::ALL,
            [
                ReaderSettingsTab::Theme,
                ReaderSettingsTab::Text,
                ReaderSettingsTab::Annotations
            ]
        );
        let mut ids = std::collections::HashSet::new();
        for (tab, zh, en) in [
            (ReaderSettingsTab::Theme, "主题", "Theme"),
            (ReaderSettingsTab::Text, "文本", "Text"),
            (ReaderSettingsTab::Annotations, "注音", "Glosses"),
        ] {
            assert!(ids.insert(tab.button()));
            assert_eq!(
                crate::i18n::text(
                    readwise_core::settings::AppLanguage::Chinese,
                    tab.label_key()
                ),
                zh
            );
            assert_eq!(
                crate::i18n::text(
                    readwise_core::settings::AppLanguage::English,
                    tab.label_key()
                ),
                en
            );
        }
    }

    #[test]
    fn text_choices_apply_and_select_exactly_one_option_per_group() {
        let mut settings = readwise_core::settings::ReaderSettings {
            margin: -2.0,
            character_spacing: 0.04,
            word_spacing: 0.1,
            ..Default::default()
        };
        let choices = text_choices();
        for (_, key, choice) in choices {
            for language in [
                readwise_core::settings::AppLanguage::Chinese,
                readwise_core::settings::AppLanguage::English,
            ] {
                assert_ne!(crate::i18n::text(language, key), key);
            }
            choice.apply(&mut settings);
            assert!(choice.selected(&settings));
            assert_eq!(
                (
                    settings.margin,
                    settings.character_spacing,
                    settings.word_spacing
                ),
                (-2.0, 0.04, 0.1)
            );
            for group in [
                &choices[..3],
                &choices[3..6],
                &choices[6..8],
                &choices[8..10],
                &choices[10..],
            ] {
                assert_eq!(
                    group
                        .iter()
                        .filter(|(_, _, choice)| choice.selected(&settings))
                        .count(),
                    1
                );
            }
        }
    }

    #[test]
    fn text_tab_removes_advanced_spacing_controls_but_keeps_width_and_columns() {
        let sections = section_widgets();
        for id in [
            id!(reader_customise_margin_row),
            id!(reader_customise_columns_row),
        ] {
            assert!(sections.contains(&(id, ReaderSettingsTab::Text)));
        }
        let source = include_str!("app.rs");
        for removed in [
            "reader_customise_bold",
            "reader_customise_page_margin",
            "reader_customise_character",
            "reader_customise_word_slider",
            "reader_customise_word_value",
            "reader_customise_word_row",
            "reader_customise_word_label",
            "reader_customise_layout_title",
            "reader_customise_columns_choice",
        ] {
            assert!(!source.contains(removed), "stale aA control: {removed}");
        }
    }

    #[test]
    fn columns_follow_paragraphs_as_matching_capsules_before_reset() {
        let source = include_str!("app.rs");
        let paragraph = source.find("reader_customise_paragraph_row :=").unwrap();
        let columns = source.find("reader_customise_columns_row :=").unwrap();
        let reset = source.find("reader_customise_text_reset :=").unwrap();
        assert!(paragraph < columns && columns < reset);
        assert_eq!(source[paragraph..columns].matches("_row :=").count(), 1);
        let group = &source[columns..reset];
        assert!(group.contains("reader_customise_columns_controls := RoundedView{"));
        for id in [
            "reader_customise_columns_one",
            "reader_customise_columns_two",
            "reader_customise_columns_auto",
        ] {
            assert!(group.contains(&format!("{id} := ReaderSettingsSegment{{height: 32")));
        }
    }

    #[test]
    fn every_scroll_child_has_exactly_one_section_owner() {
        let mut ids = std::collections::HashSet::new();
        for (id, owner) in section_widgets() {
            assert!(ids.insert(id));
            for tab in ReaderSettingsTab::ALL {
                assert_eq!(section_visible(tab, owner, id, true), tab == owner);
            }
        }
        assert_eq!(ids.len(), 18);
    }

    #[test]
    fn disabling_annotations_keeps_toggle_and_preview_but_not_colour_rows() {
        use ReaderSettingsTab::*;
        assert!(section_visible(
            Annotations,
            Annotations,
            id!(reader_customise_gloss_enable),
            false
        ));
        assert!(section_visible(
            Annotations,
            Annotations,
            id!(reader_preview),
            false
        ));
        assert!(!section_visible(
            Annotations,
            Annotations,
            id!(reader_customise_gloss_bg_row),
            false
        ));
        assert!(!section_visible(
            Text,
            Annotations,
            id!(reader_customise_gloss_bg_row),
            true
        ));
    }
}
