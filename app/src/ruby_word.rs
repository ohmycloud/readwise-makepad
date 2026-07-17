use makepad_widgets::makepad_html::HtmlDoc;
use makepad_widgets::*;
use std::sync::{LazyLock, RwLock};

/// 注音字号（设置页可调，6-12）。
pub static GLOSS_FONT_SIZE: LazyLock<RwLock<f32>> = LazyLock::new(|| RwLock::new(7.5));

/// 注音颜色 RGB（设置页预设可选）。
pub static GLOSS_COLOR: LazyLock<RwLock<(f32, f32, f32)>> =
    LazyLock::new(|| RwLock::new(GLOSS_PRESETS[0].1));

/// (名称, RGB) 预设。
pub const GLOSS_PRESETS: &[(&str, (f32, f32, f32))] = &[
    ("青", (0.498, 0.698, 0.651)),
    ("蓝", (0.384, 0.714, 1.0)),
    ("橙", (0.910, 0.529, 0.353)),
    ("灰", (0.604, 0.604, 0.604)),
];

/// GLOSS_DEBUG=1 时画出注音的锚定矩形/摆放盒/真实字盒。
static GLOSS_DEBUG: LazyLock<bool> = LazyLock::new(|| std::env::var("GLOSS_DEBUG").is_ok());

/// 注音背景色（None = 无背景）。
pub static GLOSS_BG: LazyLock<RwLock<Option<(f32, f32, f32, f32)>>> =
    LazyLock::new(|| RwLock::new(None));

/// 注音词字色（None = 跟随正文）。
pub static WORD_COLOR: LazyLock<RwLock<Option<(f32, f32, f32)>>> =
    LazyLock::new(|| RwLock::new(None));

/// 注音词背景色（None = 无背景）。
pub static WORD_BG: LazyLock<RwLock<Option<(f32, f32, f32, f32)>>> =
    LazyLock::new(|| RwLock::new(None));

/// 背景预设（深色主题友好的半透明填充）。
pub const BG_PRESETS: &[(&str, Option<(f32, f32, f32, f32)>)] = &[
    ("无", None),
    ("深灰", Some((0.30, 0.30, 0.30, 0.85))),
    ("深青", Some((0.16, 0.31, 0.27, 0.85))),
    ("深蓝", Some((0.16, 0.24, 0.38, 0.85))),
];

/// 词字色预设（首项 = 跟随正文）。
pub const WORD_COLOR_PRESETS: &[(&str, Option<(f32, f32, f32)>)] = &[
    ("默认", None),
    ("青", Some(GLOSS_PRESETS[0].1)),
    ("蓝", Some(GLOSS_PRESETS[1].1)),
    ("橙", Some(GLOSS_PRESETS[2].1)),
];

script_mod! {
    use mod.prelude.widgets.*

    mod.widgets.RubyWordBase = #(RubyWord::register_widget(vm))
    mod.widgets.RubyWord = set_type_default() do mod.widgets.RubyWordBase{
        width: Fit height: Fit
        gloss_gap: 2.0
        draw_gloss +: {
            color: #x7fb2a6
            text_style: theme.font_regular{
                font_size: 7.5
            }
        }
    }
}

/// 注音词的点击动作（M4 接释义弹窗/加生词本）。
#[derive(Clone, Debug, Default)]
pub enum RubyWordAction {
    #[default]
    None,
    Clicked {
        word: String,
        gloss: String,
    },
}

/// 注音词 widget：原词上方居中叠加小字释义。
/// 关键设计：原词不自己画，而是交给 TextFlow 的 `draw_text` 内联绘制——
/// 基线、字体、断行与正文完全一致（HtmlLink 同款画法）；
/// 释义用独立 DrawText 按绝对坐标画在词的第一段区域上方。
/// 由 Html 自定义标签机制实例化：<ruby t="释义">word</ruby>。
#[derive(Script, Widget)]
pub struct RubyWord {
    #[uid]
    uid: WidgetUid,
    #[source]
    source: ScriptObjectRef,
    #[redraw]
    #[area]
    area: Area,
    #[walk]
    walk: Walk,
    #[layout]
    layout: Layout,
    #[live]
    draw_gloss: DrawText,
    #[live]
    draw_debug: DrawColor,
    #[live]
    draw_fill: DrawColor,
    #[live(2.0f32)]
    gloss_gap: f32,
    #[rust]
    drawn_areas: Vec<Area>,
    #[rust]
    word: String,
    #[rust]
    gloss: String,
}

impl ScriptHook for RubyWord {
    fn on_after_new_scoped(&mut self, _vm: &mut ScriptVm, scope: &mut Scope) {
        if let Some(doc) = scope.props.get::<HtmlDoc>() {
            let mut walker = doc.new_walker_with_index(scope.index + 1);
            while let Some((lc, attr)) = walker.while_attr_lc() {
                if lc == live_id!(t) {
                    self.gloss = attr.into();
                    break;
                }
            }
        }
    }
}

impl Widget for RubyWord {
    fn set_text(&mut self, cx: &mut Cx, v: &str) {
        // 正式管线：内文 = 词 + RUBY_SEP + 释义（随文本每帧刷新，widget 复用不串位）；
        // 无分隔符时保留创建时从 t 属性读的释义（M0 demo 兼容）。
        match v.split_once(readwise_core::vocab::annotate::RUBY_SEP) {
            Some((word, gloss)) => {
                self.word = word.to_string();
                self.gloss = gloss.to_string();
            }
            None => self.word = v.to_string(),
        }
        self.redraw(cx);
    }

    fn text(&self) -> String {
        self.word.clone()
    }

    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, _walk: Walk) -> DrawStep {
        let Some(tf) = scope.data.get_mut::<TextFlow>() else {
            return DrawStep::done();
        };

        // 注音词背景：用上一帧的区域先铺底（首帧无区域，下一帧补上）
        if let Some((r, g, b, a)) = *WORD_BG.read().unwrap() {
            self.draw_fill.color = Vec4f { x: r, y: g, z: b, w: a };
            for area in self.drawn_areas.clone() {
                self.draw_fill.draw_abs(cx, area.rect(cx));
            }
        }

        let word_color = *WORD_COLOR.read().unwrap();
        if let Some((r, g, b)) = word_color {
            tf.font_colors.push(Vec4f { x: r, y: g, z: b, w: 1.0 });
        }
        tf.areas_tracker.push_tracker();
        tf.draw_text(cx, &self.word);
        let (start, end) = tf.areas_tracker.pop_tracker();
        if word_color.is_some() {
            tf.font_colors.pop();
        }

        if self.drawn_areas.len() == end - start {
            for i in 0..end - start {
                self.drawn_areas[i] =
                    cx.update_area_refs(self.drawn_areas[i], tf.areas_tracker.areas[i + start]);
            }
        } else {
            self.drawn_areas = tf.areas_tracker.areas[start..end].to_vec();
        }

        if !self.gloss.is_empty() {
            // 词在行尾折行时首段是个小残段，注音要锚在最宽的那段（词的主体）上
            let widest = self
                .drawn_areas
                .iter()
                .max_by(|a, b| {
                    let wa = a.rect(cx).size.x;
                    let wb = b.rect(cx).size.x;
                    wa.partial_cmp(&wb).unwrap_or(std::cmp::Ordering::Equal)
                })
                .copied();
            if let Some(first) = widest {
                // 设置页可配置的注音字号/颜色
                self.draw_gloss.text_style.font_size = *GLOSS_FONT_SIZE.read().unwrap();
                let (r, g, b) = *GLOSS_COLOR.read().unwrap();
                self.draw_gloss.color = Vec4f { x: r, y: g, z: b, w: 1.0 };
                let rect = first.rect(cx);
                let laidout =
                    self.draw_gloss
                        .layout(cx, 0.0, 0.0, None, false, Align::default(), &self.gloss);
                let row = laidout.rows.first();
                let gloss_w = row
                    .map(|r| (r.width_in_lpxs * self.draw_gloss.font_scale) as f64)
                    .unwrap_or(0.0);
                // 真实字盒高度 = (ascender - descender)，descender 为负
                let gloss_box_h = row
                    .map(|r| {
                        ((r.ascender_in_lpxs - r.descender_in_lpxs) * self.draw_gloss.font_scale)
                            as f64
                    })
                    .unwrap_or(self.draw_gloss.text_style.font_size as f64 * 1.4);
                let x = rect.pos.x + (rect.size.x - gloss_w) * 0.5;
                let y = rect.pos.y - gloss_box_h - self.gloss_gap as f64;
                if let Some((r, g, b, a)) = *GLOSS_BG.read().unwrap() {
                    self.draw_fill.color = Vec4f { x: r, y: g, z: b, w: a };
                    self.draw_fill.draw_abs(
                        cx,
                        Rect {
                            pos: dvec2(x - 2.0, y - 1.0),
                            size: dvec2(gloss_w + 4.0, gloss_box_h + 2.0),
                        },
                    );
                }
                if *GLOSS_DEBUG {
                    self.draw_debug.color = Vec4f { x: 0.2, y: 0.8, z: 0.3, w: 0.25 };
                    self.draw_debug.draw_abs(cx, rect);
                    self.draw_debug.color = Vec4f { x: 0.9, y: 0.15, z: 0.15, w: 0.35 };
                    self.draw_debug.draw_abs(
                        cx,
                        Rect {
                            pos: dvec2(x, y),
                            size: dvec2(gloss_w.max(2.0), gloss_box_h),
                        },
                    );
                }
                self.draw_gloss.draw_abs(cx, dvec2(x, y), &self.gloss);
            }
        }

        DrawStep::done()
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event, _scope: &mut Scope) {
        for area in self.drawn_areas.clone() {
            match event.hits(cx, area) {
                Hit::FingerHoverIn(_) => {
                    cx.set_cursor(MouseCursor::Hand);
                }
                Hit::FingerLongPress(_) => {
                    cx.widget_action(
                        self.widget_uid(),
                        RubyWordAction::Clicked {
                            word: self.word.clone(),
                            gloss: self.gloss.clone(),
                        },
                    );
                }
                Hit::FingerUp(fu) => {
                    if fu.is_over && fu.is_primary_hit() && fu.was_tap() {
                        cx.widget_action(
                            self.widget_uid(),
                            RubyWordAction::Clicked {
                                word: self.word.clone(),
                                gloss: self.gloss.clone(),
                            },
                        );
                    }
                }
                _ => {}
            }
        }
    }
}
