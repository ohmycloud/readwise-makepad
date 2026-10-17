use makepad_widgets::*;
use std::sync::{LazyLock, RwLock};

/// 0..=3 retain the original colours and saved preferences. The aA picker uses
/// 4..=7 (yellow/green/blue/pink); never reuse a legacy slot or note colour name.
pub static HL_SLOT: LazyLock<RwLock<usize>> = LazyLock::new(|| RwLock::new(4));
pub const MARKER_SLOTS: [usize; 4] = [4, 5, 6, 7];
pub const MARKER_RGB: [u32; 4] = [0xf2c4_38ff, 0x7cc6_6fff, 0x6aa8_f0ff, 0xf084_acff];

/// 槽位 → 半透明底色。
pub fn hl_color(slot: usize) -> Vec4f {
    match slot {
        4..=7 => {
            let mut color = Vec4f::from_u32(MARKER_RGB[slot - 4]);
            color.w = 0.35;
            color
        }
        1 => Vec4f {
            x: 0.498,
            y: 0.698,
            z: 0.651,
            w: 0.35,
        },
        2 => Vec4f {
            x: 0.290,
            y: 0.565,
            z: 0.850,
            w: 0.35,
        },
        3 => Vec4f {
            x: 0.910,
            y: 0.529,
            z: 0.353,
            w: 0.35,
        },
        _ => Vec4f {
            x: 0.851,
            y: 0.788,
            z: 0.416,
            w: 0.35,
        },
    }
}

/// 槽位 → 存库颜色名（book_notes.color）。
pub const HL_COLOR_NAMES: &[&str] = &[
    "yellow",
    "teal",
    "blue",
    "orange",
    "marker-yellow",
    "marker-green",
    "marker-blue",
    "marker-pink",
];

pub fn hl_slot_for_name(color: &str) -> usize {
    match color {
        "marker-yellow" => 4,
        "marker-green" => 5,
        "marker-blue" => 6,
        "marker-pink" => 7,
        "teal" | "green" => 1,
        "blue" => 2,
        "orange" | "red" | "violet" => 3,
        _ => 0,
    }
}

#[cfg(test)]
mod palette_tests {
    use super::*;

    #[test]
    fn legacy_note_colours_and_aliases_keep_their_original_rgba() {
        for (name, slot, expected) in [
            ("yellow", 0, (0.851, 0.788, 0.416)),
            ("teal", 1, (0.498, 0.698, 0.651)),
            ("blue", 2, (0.290, 0.565, 0.850)),
            ("orange", 3, (0.910, 0.529, 0.353)),
        ] {
            assert_eq!(hl_slot_for_name(name), slot);
            let color = hl_color(slot);
            assert_eq!((color.x, color.y, color.z), expected);
            assert_eq!(color.w, 0.35);
        }
        for (alias, slot) in [("green", 1), ("red", 3), ("violet", 3), ("unknown", 0)] {
            assert_eq!(hl_slot_for_name(alias), slot);
        }
    }

    #[test]
    fn all_marker_colours_round_trip_without_colliding_with_legacy_names() {
        for (slot, name) in HL_COLOR_NAMES.iter().enumerate() {
            assert_eq!(hl_slot_for_name(name), slot);
        }
        for (index, slot) in MARKER_SLOTS.into_iter().enumerate() {
            let color = hl_color(slot);
            let expected = Vec4f::from_u32(MARKER_RGB[index]);
            assert_eq!(
                (color.x, color.y, color.z),
                (expected.x, expected.y, expected.z)
            );
            assert_eq!(color.w, 0.35);
        }
        assert_ne!(hl_color(2), hl_color(6));
        assert_ne!(hl_color(3), hl_color(7));
        let default = readwise_core::settings::ReaderSettings::default();
        assert_eq!(default.highlight_color_slot, MARKER_SLOTS[0]);
        assert_eq!(
            readwise_core::settings::ReaderSettings {
                highlight_color_slot: usize::MAX,
                ..default
            }
            .normalized()
            .highlight_color_slot,
            *MARKER_SLOTS.last().unwrap()
        );
    }
}

script_mod! {
    use mod.prelude.widgets.*

    mod.widgets.MarkSpanBase = #(MarkSpan::register_widget(vm))
    mod.widgets.MarkSpan = set_type_default() do mod.widgets.MarkSpanBase{}
}

/// 兼容旧 `<mark>` 文档的纯文字 run。永久高亮底色由 ChapterList 在正文
/// 排版完成后按数据库区间绘制；这里绝不进入 InlineCode 或改变 turtle 布局。
#[derive(Script, WidgetRef, WidgetSet, WidgetRegister)]
pub struct MarkSpan {
    #[uid]
    uid: WidgetUid,
    #[source]
    source: ScriptObjectRef,
    #[rust]
    area: Area,
    #[rust]
    drawn_areas: Vec<Area>,
    #[rust]
    text: String,
}

impl ScriptHook for MarkSpan {}

impl WidgetNode for MarkSpan {
    fn widget_uid(&self) -> WidgetUid {
        self.uid
    }
    fn walk(&mut self, _cx: &mut Cx) -> Walk {
        Walk::default()
    }
    fn area(&self) -> Area {
        self.area
    }
    fn redraw(&mut self, cx: &mut Cx) {
        if self.drawn_areas.is_empty() {
            self.area.redraw(cx);
        } else {
            self.drawn_areas.iter().for_each(|area| area.redraw(cx));
        }
    }
    fn point_hits_area(&self, _cx: &Cx, _point: DVec2) -> bool {
        false
    }
}

impl Widget for MarkSpan {
    fn set_text(&mut self, cx: &mut Cx, v: &str) {
        let (text, _) = v
            .rsplit_once(readwise_core::vocab::annotate::MARK_SEP)
            .unwrap_or((v, "yellow"));
        if self.text != text {
            self.text = text.to_string();
            self.redraw(cx);
        }
    }

    fn text(&self) -> String {
        self.text.clone()
    }

    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, _walk: Walk) -> DrawStep {
        let Some(tf) = scope.data.get_mut::<TextFlow>() else {
            return DrawStep::done();
        };
        tf.areas_tracker.push_tracker();
        crate::reader_hit::draw_text(tf, cx, &self.text);
        let (start, end) = tf.areas_tracker.pop_tracker();
        if self.drawn_areas.len() == end - start {
            for i in 0..end - start {
                self.drawn_areas[i] =
                    cx.update_area_refs(self.drawn_areas[i], tf.areas_tracker.areas[i + start]);
            }
        } else {
            self.drawn_areas = tf.areas_tracker.areas[start..end].to_vec();
        }
        DrawStep::done()
    }

    fn handle_event(&mut self, _cx: &mut Cx, _event: &Event, _scope: &mut Scope) {}
}
