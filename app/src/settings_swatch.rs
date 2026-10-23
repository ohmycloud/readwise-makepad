//! Shared colour presentation for settings, the aA popover and its draft editor.
//! Colour/slot semantics stay in the existing reader settings and presets.
use makepad_widgets::*;
use readwise_core::settings::AppLanguage;

/// New Pen colours use separate slots so a saved classic selection keeps its ink.
pub const PEN_SLOTS: [usize; 4] = [4, 5, 6, 7];
const SELECTION_RGBA: [u32; 8] = [
    0x7fb2_a688,
    0x4a90_d988,
    0xe887_5a88,
    0xd9c9_6a88,
    0xc140_1b88,
    0x2c58_c988,
    0x3b7a_3d88,
    0x4b4a_4788,
];

/// The actual text-selection paint; swatches use the same RGB with preview alpha.
pub fn selection_color(slot: usize) -> Vec4f {
    Vec4f::from_u32(SELECTION_RGBA[slot.min(7)])
}

#[derive(Clone, Copy, Debug)]
pub enum ColorKind {
    Gloss,
    Background,
    Word,
    Selection,
    Highlight,
}

impl ColorKind {
    /// Keep the settings page on the active palette without remapping a saved
    /// classic colour. Choosing an aA marker switches that row to the new set.
    pub fn option_slot(self, option: usize, selected_slot: usize) -> usize {
        let offset = if matches!(self, Self::Selection | Self::Highlight) && selected_slot >= 4 {
            4
        } else {
            0
        };
        offset + option.min(3)
    }

    pub fn name(self, slot: usize, language: AppLanguage) -> &'static str {
        if matches!(self, Self::Selection) && slot >= 4 {
            return crate::i18n::text(
                language,
                ["vermilion", "cobalt", "moss", "graphite"][slot.min(7) - 4],
            );
        }
        if matches!(self, Self::Highlight) && slot >= 4 {
            return crate::i18n::text(
                language,
                ["yellow", "green", "blue", "pink"][slot.min(7) - 4],
            );
        }
        let keys = match self {
            Self::Gloss => ["cyan", "blue", "orange", "gray"],
            Self::Background => ["none", "dark_gray", "dark_cyan", "dark_blue"],
            Self::Word => ["default", "cyan", "blue", "orange"],
            Self::Selection => ["cyan", "blue", "orange", "yellow"],
            Self::Highlight => ["yellow", "cyan", "blue", "orange"],
        };
        crate::i18n::text(language, keys[slot.min(3)])
    }

    pub fn color(self, slot: usize, text_color: Vec4f) -> Vec4f {
        use crate::ruby_word::{BG_PRESETS, GLOSS_PRESETS, WORD_COLOR_PRESETS};
        let slot = slot.min(if matches!(self, Self::Selection | Self::Highlight) {
            7
        } else {
            3
        });
        let rgb = |(x, y, z)| Vec4f { x, y, z, w: 0.8 };
        match self {
            Self::Gloss => rgb(GLOSS_PRESETS[slot].1),
            Self::Background => BG_PRESETS[slot]
                .1
                .map(|(x, y, z, w)| Vec4f { x, y, z, w })
                .unwrap_or_default(),
            Self::Word => WORD_COLOR_PRESETS[slot].1.map(rgb).unwrap_or(Vec4f {
                w: 0.8,
                ..text_color
            }),
            Self::Selection => {
                let mut color = selection_color(slot);
                color.w = 0.8;
                color
            }
            Self::Highlight => {
                let mut color = crate::mark_span::hl_color(slot);
                color.w = 0.8;
                color
            }
        }
    }
}

script_mod! {
    use mod.prelude.widgets.*

    mod.widgets.SettingsColorSwatch = ButtonFlat{
        width: 44 height: 44
        margin: 0 padding: 0 text: ""
        // Colour rows also live inside scroll views: a drag/hold is not a pick.
        enable_long_press: true
        draw_bg +: {
            color: #d9c96a
            border_size: 0.0
            border_color: theme.color_label_inner_inactive
            pixel: fn() {
                let center = self.rect_size * 0.5
                let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                // Compact annotation rows narrow the hit box, not its 44px
                // height. Scale the dab and ring together so neither is clipped.
                sdf.scale(max(1.0, 44.0 / min(self.rect_size.x, self.rect_size.y)), center.x, center.y)
                // Only selection draws a ring. Focus can survive closing and
                // reopening a modal, so it must not ring a different old choice.
                // Both hover and keyboard focus emphasise the ink dab instead.
                let emphasis = max(self.hover, self.focus)
                let ring_width = self.border_size
                if ring_width > 0.0 {
                    sdf.circle(center.x, center.y, 18.0)
                    // Sdf2d.stroke takes a half-width; keep the visible ring thin.
                    sdf.stroke(self.border_color, ring_width * 0.5)
                }
                sdf.rotate(-0.2094395, center.x, center.y)
                let width = 22.0 - self.down * 2.0
                let height = 13.0 - self.down
                sdf.box(center.x - width * 0.5, center.y - height * 0.5, width, height, 3.0)
                if self.color.w < 0.01 {
                    // No background: an outlined, crossed-out ink dab rather
                    // than an invisible hit target or a misleading gray fill.
                    sdf.stroke(vec4(self.border_color.xyz, 0.65 + emphasis * 0.2), 1.0)
                    sdf.move_to(center.x - 9.0, center.y + 5.0)
                    sdf.line_to(center.x + 9.0, center.y - 5.0)
                    sdf.stroke(vec4(self.border_color.xyz, 0.65 + emphasis * 0.2), 1.0)
                } else {
                    sdf.fill(vec4(self.color.xyz, min(1.0, self.color.w + emphasis * 0.15)))
                }
                return sdf.result
            }
        }
    }

    mod.widgets.SettingsColorPicker = View{
        width: Fit height: 44
        flow: Right spacing: 4 align: Align{y: 0.5}
    }
    mod.widgets.SettingsColorTitle = Label{
        width: 88 height: Fit
        padding: 0
        draw_text.color: theme.color_label_inner_inactive
    }
    mod.widgets.SettingsColorName = Label{
        width: 68 height: Fit
        padding: 0
        draw_text.text_style.font_size: 11
        draw_text.color: theme.color_label_inner_inactive
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_preserve_all_existing_slot_orders_in_both_languages() {
        for (kind, chinese, english) in [
            (
                ColorKind::Gloss,
                ["青", "蓝", "橙", "灰"],
                ["Cyan", "Blue", "Orange", "Gray"],
            ),
            (
                ColorKind::Background,
                ["无", "深灰", "深青", "深蓝"],
                ["None", "Dark Gray", "Dark Cyan", "Dark Blue"],
            ),
            (
                ColorKind::Word,
                ["默认", "青", "蓝", "橙"],
                ["Default", "Cyan", "Blue", "Orange"],
            ),
            (
                ColorKind::Selection,
                ["青", "蓝", "橙", "黄"],
                ["Cyan", "Blue", "Orange", "Yellow"],
            ),
            (
                ColorKind::Highlight,
                ["黄", "青", "蓝", "橙"],
                ["Yellow", "Cyan", "Blue", "Orange"],
            ),
        ] {
            for slot in 0..4 {
                assert_eq!(kind.name(slot, AppLanguage::Chinese), chinese[slot]);
                assert_eq!(kind.name(slot, AppLanguage::English), english[slot]);
            }
            let last = match kind {
                ColorKind::Highlight => "Pink",
                ColorKind::Selection => "Graphite",
                _ => english[3],
            };
            assert_eq!(kind.name(usize::MAX, AppLanguage::English), last);
        }
    }

    #[test]
    fn no_background_is_distinct_from_following_the_text_colour() {
        let ink = Vec4f::from_u32(0x302f_2aff);
        assert_eq!(ColorKind::Background.color(0, ink).w, 0.0);
        let default = ColorKind::Word.color(0, ink);
        assert_eq!((default.x, default.y, default.z), (ink.x, ink.y, ink.z));
        assert!(default.w > 0.0);
        let white = Vec4f::from_u32(0xf7f7_f7ff);
        assert_ne!(ColorKind::Word.color(0, white).x, default.x);
    }

    #[test]
    fn swatches_use_the_real_annotation_presets_without_mutating_them() {
        let ink = Vec4f::from_u32(0x302f_2aff);
        for slot in 0..4 {
            let gloss = ColorKind::Gloss.color(slot, ink);
            assert_eq!(
                (gloss.x, gloss.y, gloss.z),
                crate::ruby_word::GLOSS_PRESETS[slot].1
            );
            let highlight = ColorKind::Highlight.color(slot, ink);
            let actual = crate::mark_span::hl_color(slot);
            assert_eq!(
                (highlight.x, highlight.y, highlight.z),
                (actual.x, actual.y, actual.z)
            );
            assert_eq!(
                crate::mark_span::hl_slot_for_name(crate::mark_span::HL_COLOR_NAMES[slot]),
                slot
            );
        }
    }

    #[test]
    fn marker_names_and_settings_projection_keep_legacy_slots_distinct() {
        for (index, (chinese, english)) in [
            ("黄", "Yellow"),
            ("绿", "Green"),
            ("蓝", "Blue"),
            ("粉", "Pink"),
        ]
        .into_iter()
        .enumerate()
        {
            let slot = crate::mark_span::MARKER_SLOTS[index];
            assert_eq!(
                ColorKind::Highlight.name(slot, AppLanguage::Chinese),
                chinese
            );
            assert_eq!(
                ColorKind::Highlight.name(slot, AppLanguage::English),
                english
            );
            assert_eq!(ColorKind::Highlight.option_slot(index, slot), slot);
            assert_eq!(ColorKind::Highlight.option_slot(index, 3), index);
        }
        assert_eq!(ColorKind::Highlight.name(3, AppLanguage::English), "Orange");
        assert_eq!(ColorKind::Gloss.option_slot(1, 7), 1);
    }

    #[test]
    fn pen_slots_keep_classic_selection_ink_and_match_the_real_selection_paint() {
        let ink = Vec4f::from_u32(0x302f_2aff);
        for (slot, rgba) in SELECTION_RGBA.into_iter().enumerate() {
            assert_eq!(selection_color(slot), Vec4f::from_u32(rgba));
            let preview = ColorKind::Selection.color(slot, ink);
            let actual = selection_color(slot);
            assert_eq!(
                (preview.x, preview.y, preview.z),
                (actual.x, actual.y, actual.z)
            );
            assert_eq!(actual.w, 136.0 / 255.0);
        }
        for (index, (zh, en)) in [
            ("朱红", "Vermilion"),
            ("钴蓝", "Cobalt"),
            ("苔绿", "Moss"),
            ("石墨", "Graphite"),
        ]
        .into_iter()
        .enumerate()
        {
            let slot = PEN_SLOTS[index];
            assert_eq!(ColorKind::Selection.name(slot, AppLanguage::Chinese), zh);
            assert_eq!(ColorKind::Selection.name(slot, AppLanguage::English), en);
            assert_eq!(ColorKind::Selection.option_slot(index, slot), slot);
            assert_eq!(ColorKind::Selection.option_slot(index, 0), index);
        }
    }

    #[test]
    fn colour_index_clamps_like_normalized_settings() {
        let ink = Vec4f::from_u32(0x302f_2aff);
        let settings = readwise_core::settings::ReaderSettings {
            highlight_color_slot: usize::MAX,
            ..Default::default()
        }
        .normalized();
        for kind in [
            ColorKind::Gloss,
            ColorKind::Background,
            ColorKind::Word,
            ColorKind::Selection,
            ColorKind::Highlight,
        ] {
            assert_eq!(
                kind.color(usize::MAX, ink),
                kind.color(settings.highlight_color_slot, ink)
            );
        }
    }
}
