use makepad_widgets::*;
use std::sync::{LazyLock, RwLock};

/// 高亮颜色槽位：0=黄 1=青 2=蓝 3=橙（设置页可选，MarkSpan 每帧读取）。
pub static HL_SLOT: LazyLock<RwLock<usize>> = LazyLock::new(|| RwLock::new(0));

/// 槽位 → 半透明底色。
pub fn hl_color(slot: usize) -> Vec4f {
    match slot {
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
pub const HL_COLOR_NAMES: &[&str] = &["yellow", "teal", "blue", "orange"];

/// Draw an inline run through TextFlow's built-in background path. Temporarily
/// enabling the inline-code branch is intentional: that branch lays out each
/// wrapped row first, emits its background before the glyph batch, and still
/// participates in TextFlow selection. Zero padding/margin keeps ordinary
/// reader typography unchanged.
pub(crate) fn draw_text_with_background(
    tf: &mut TextFlow,
    cx: &mut Cx2d,
    text: &str,
    color: Vec4f,
) {
    let old_color = tf.draw_block.code_color;
    let old_padding = tf.inline_code_padding;
    let old_margin = tf.inline_code_margin;
    tf.draw_block.code_color = color;
    tf.inline_code_padding = Inset::default();
    tf.inline_code_margin = Inset::default();
    tf.inline_code.push();
    tf.draw_text(cx, text);
    tf.inline_code.pop();
    tf.draw_block.code_color = old_color;
    tf.inline_code_padding = old_padding;
    tf.inline_code_margin = old_margin;
}

script_mod! {
    use mod.prelude.widgets.*

    mod.widgets.MarkSpanBase = #(MarkSpan::register_widget(vm))
    mod.widgets.MarkSpan = set_type_default() do mod.widgets.MarkSpanBase{}
}

/// 高亮段：文字交给 TextFlow 内联绘制（与正文同排版），
/// 底色用上一帧的绘制区域铺在文字下面（荧光笔效果）。
/// 不自报命中（point_hits_area=false），选区与点词从其上正常穿过。
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
    #[rust]
    color_slot: usize,
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
        let (text, color) = v
            .rsplit_once(readwise_core::vocab::annotate::MARK_SEP)
            .unwrap_or((v, "yellow"));
        let color_slot = match color {
            "teal" | "green" => 1,
            "blue" => 2,
            "orange" | "red" | "violet" => 3,
            _ => 0,
        };
        if self.text != text || self.color_slot != color_slot {
            self.text = text.to_string();
            self.color_slot = color_slot;
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
        draw_text_with_background(tf, cx, &self.text, hl_color(self.color_slot));
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
