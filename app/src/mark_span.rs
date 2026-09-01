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

const LPXS_PER_POINT: f64 = 96.0 / 72.0;
// Font line boxes vary slightly by family. This factor deliberately errs a
// little high so adjacent highlight rows overlap instead of leaving a hairline.
const HIGHLIGHT_LINE_BOX_EM: f64 = 1.25;
// TextFlow's InlineCode SDF draws one logical pixel inside its rect on every
// edge. Compensate that inset and overlap the anti-aliased edges by half a
// pixel, otherwise two vertically adjacent rects still show a fine seam.
const HIGHLIGHT_SDF_EDGE_OVERLAP: f64 = 1.5;

fn highlight_vertical_padding(font_size: f32, line_spacing: f32, font_scale: f32) -> f64 {
    let extra_spacing = (line_spacing - 1.0).max(0.0) as f64;
    let line_box = font_size.max(0.0) as f64
        * LPXS_PER_POINT
        * font_scale.max(0.0) as f64
        * HIGHLIGHT_LINE_BOX_EM;
    line_box * extra_spacing * 0.5 + HIGHLIGHT_SDF_EDGE_OVERLAP
}

fn active_text_line_metrics(tf: &TextFlow) -> (f32, f32) {
    let line_spacing = if tf.fixed.value() > 0 {
        tf.text_style_fixed.line_spacing
    } else if tf.bold.value() > 0 {
        if tf.italic.value() > 0 {
            tf.text_style_bold_italic.line_spacing
        } else {
            tf.text_style_bold.line_spacing
        }
    } else if tf.italic.value() > 0 {
        tf.text_style_italic.line_spacing
    } else {
        tf.text_style_normal.line_spacing
    };
    let font_size = *tf.font_sizes.last().unwrap_or(&tf.font_size);
    (font_size, line_spacing)
}

/// Draw an inline run through TextFlow's built-in background path. Temporarily
/// enabling the inline-code branch is intentional: that branch lays out each
/// wrapped row first, emits its background before the glyph batch, and still
/// participates in TextFlow selection. A permanent highlight expands into the
/// configured line gap; matching negative margins keep the reader's line layout
/// and wrapping unchanged.
pub(crate) fn draw_text_with_background(
    tf: &mut TextFlow,
    cx: &mut Cx2d,
    text: &str,
    color: Vec4f,
    fill_line_gap: bool,
) {
    let old_color = tf.draw_block.code_color;
    let old_padding = tf.inline_code_padding;
    let old_margin = tf.inline_code_margin;
    tf.draw_block.code_color = color;
    if fill_line_gap {
        let (font_size, line_spacing) = active_text_line_metrics(tf);
        let vertical = highlight_vertical_padding(font_size, line_spacing, tf.draw_text.font_scale);
        tf.inline_code_padding = Inset {
            top: vertical,
            bottom: vertical,
            left: 0.0,
            right: 0.0,
        };
        tf.inline_code_margin = Inset {
            top: -vertical,
            bottom: -vertical,
            left: 0.0,
            right: 0.0,
        };
    } else {
        tf.inline_code_padding = Inset::default();
        tf.inline_code_margin = Inset::default();
    }
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

/// 高亮段：文字和底色交给 TextFlow 在同一帧内联绘制（与正文同排版）。
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
        draw_text_with_background(tf, cx, &self.text, hl_color(self.color_slot), true);
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

#[cfg(test)]
mod tests {
    use super::highlight_vertical_padding;

    #[test]
    fn highlight_padding_fills_half_of_the_configured_line_gap() {
        // 12pt = 16 logical pixels; the conservative 1.25em line box is 20px.
        // At 1.5 line spacing each side grows by 5px plus the 1.5px SDF overlap.
        assert!((highlight_vertical_padding(12.0, 1.5, 1.0) - 6.5).abs() < f64::EPSILON);
    }

    #[test]
    fn highlight_padding_keeps_sdf_edges_overlapping_at_tight_spacing() {
        assert_eq!(highlight_vertical_padding(12.0, 1.0, 1.0), 1.5);
        assert_eq!(highlight_vertical_padding(12.0, 0.8, 1.0), 1.5);
    }
}
