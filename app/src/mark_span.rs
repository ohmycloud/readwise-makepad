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

pub fn hl_slot_for_name(color: &str) -> usize {
    match color {
        "teal" | "green" => 1,
        "blue" => 2,
        "orange" | "red" | "violet" => 3,
        _ => 0,
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
        tf.draw_text(cx, &self.text);
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
