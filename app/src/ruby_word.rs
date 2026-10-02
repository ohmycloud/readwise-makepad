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

#[derive(Clone, Copy)]
struct RubyAppearance {
    font_size: f32,
    color: (f32, f32, f32),
    background: Option<(f32, f32, f32, f32)>,
    word_color: Option<(f32, f32, f32)>,
    word_background: Option<(f32, f32, f32, f32)>,
}

static PREVIEW_APPEARANCE: LazyLock<RwLock<RubyAppearance>> = LazyLock::new(|| {
    RwLock::new(RubyAppearance {
        font_size: 7.5,
        color: GLOSS_PRESETS[0].1,
        background: None,
        word_color: None,
        word_background: None,
    })
});

pub fn set_preview_appearance(settings: &readwise_core::settings::ReaderSettings) {
    *PREVIEW_APPEARANCE.write().unwrap() = RubyAppearance {
        font_size: settings.gloss_font_size,
        color: GLOSS_PRESETS
            .get(settings.gloss_color_slot)
            .unwrap_or(&GLOSS_PRESETS[0])
            .1,
        background: BG_PRESETS
            .get(settings.gloss_background_slot)
            .unwrap_or(&BG_PRESETS[0])
            .1,
        word_color: WORD_COLOR_PRESETS
            .get(settings.word_color_slot)
            .unwrap_or(&WORD_COLOR_PRESETS[0])
            .1,
        word_background: BG_PRESETS
            .get(settings.word_background_slot)
            .unwrap_or(&BG_PRESETS[0])
            .1,
    };
}

/// 注音词自身的可选背景仍沿用 TextFlow 的既有样式。永久高亮绝不走这条
/// 排版分支，而是在 ChapterList 完成正文排版后统一覆盖绘制。
fn draw_word_with_background(tf: &mut TextFlow, cx: &mut Cx2d, text: &str, color: Vec4f) {
    let old_color = tf.draw_block.code_color;
    let old_padding = tf.inline_code_padding;
    let old_margin = tf.inline_code_margin;
    tf.draw_block.code_color = color;
    tf.inline_code_padding = Inset::default();
    tf.inline_code_margin = Inset::default();
    tf.inline_code.push();
    crate::reader_hit::draw_text(tf, cx, text);
    tf.inline_code.pop();
    tf.draw_block.code_color = old_color;
    tf.inline_code_padding = old_padding;
    tf.inline_code_margin = old_margin;
}

script_mod! {
    use mod.prelude.widgets.*

    mod.widgets.RubyWordBase = #(RubyWord::register_widget(vm))
    mod.widgets.RubyWord = set_type_default() do mod.widgets.RubyWordBase{
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
        /// 点击/长按发生时的屏幕坐标，供阅读器覆盖层就近摆放释义卡片。
        anchor: DVec2,
    },
}

/// 注音词 widget：原词上方居中叠加小字释义。
/// 关键设计：原词不自己画，而是交给 TextFlow 的 `draw_text` 内联绘制——
/// 基线、字体、断行与正文完全一致（HtmlLink 同款画法）；
/// 释义用独立 DrawText 按绝对坐标画在词的第一段区域上方。
/// 由 Html 自定义标签机制实例化：<ruby t="释义">word</ruby>。
#[derive(Script, WidgetRef, WidgetSet, WidgetRegister)]
pub struct RubyWord {
    #[uid]
    uid: WidgetUid,
    #[source]
    source: ScriptObjectRef,
    #[rust]
    area: Area,
    #[live]
    draw_gloss: DrawText,
    #[live]
    draw_debug: DrawColor,
    #[live]
    draw_fill: DrawColor,
    #[live(2.0f32)]
    gloss_gap: f32,
    #[live]
    preview: bool,
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

impl WidgetNode for RubyWord {
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
    /// 命中区域 = 词的各行绘制段。词卡点击和正文拖选会同时收到按下事件，
    /// 最终由 FingerUp 的 `was_tap()` 区分单击弹卡片与拖动选择文字。
    fn point_hits_area(&self, cx: &Cx, point: DVec2) -> bool {
        self.drawn_areas
            .iter()
            .any(|a| a.is_valid(cx) && a.rect(cx).contains(point))
    }
}

impl Widget for RubyWord {
    fn set_text(&mut self, cx: &mut Cx, v: &str) {
        let v = v
            .rsplit_once(readwise_core::vocab::annotate::MARK_SEP)
            .map(|(text, _)| text)
            .unwrap_or(v);
        // 正式管线：内文 = 词 + RUBY_SEP + 释义（随文本每帧刷新，widget 复用不串位）；
        // 无分隔符时保留创建时从 t 属性读的释义（M0 demo 兼容）。
        let (word, gloss) = match v.split_once(readwise_core::vocab::annotate::RUBY_SEP) {
            Some((word, gloss)) => (word.to_string(), gloss.to_string()),
            None => (v.to_string(), self.gloss.clone()),
        };
        if self.word != word || self.gloss != gloss {
            self.word = word;
            self.gloss = gloss;
            self.redraw(cx);
        }
    }

    fn text(&self) -> String {
        self.word.clone()
    }

    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, _walk: Walk) -> DrawStep {
        let Some(tf) = scope.data.get_mut::<TextFlow>() else {
            return DrawStep::done();
        };

        let appearance = if self.preview {
            *PREVIEW_APPEARANCE.read().unwrap()
        } else {
            RubyAppearance {
                font_size: *GLOSS_FONT_SIZE.read().unwrap(),
                color: *GLOSS_COLOR.read().unwrap(),
                background: *GLOSS_BG.read().unwrap(),
                word_color: *WORD_COLOR.read().unwrap(),
                word_background: *WORD_BG.read().unwrap(),
            }
        };

        let word_color = if self.gloss.is_empty() {
            None
        } else {
            appearance.word_color
        };
        if let Some((r, g, b)) = word_color {
            tf.font_colors.push(Vec4f {
                x: r,
                y: g,
                z: b,
                w: 1.0,
            });
        }
        let word_source = tf.text_len();
        // Reserve only the rows occupied by this word. Measure a stable sample
        // even after the gloss is hidden, so marking it known preserves layout.
        self.draw_gloss.text_style.font_size = appearance.font_size;
        let slot = self
            .draw_gloss
            .layout(cx, 0.0, 0.0, None, false, Align::default(), "注音");
        let slot_height = slot
            .rows
            .first()
            .map(|row| {
                ((row.ascender_in_lpxs - row.descender_in_lpxs) * self.draw_gloss.font_scale) as f64
            })
            .unwrap_or(appearance.font_size as f64 * 1.4)
            + self.gloss_gap as f64
            + 2.0;
        let old_inset = tf.draw_text.row_top_inset;
        tf.draw_text.row_top_inset = slot_height;
        tf.areas_tracker.push_tracker();
        if let Some((r, g, b, a)) = appearance.word_background {
            draw_word_with_background(
                tf,
                cx,
                &self.word,
                if self.gloss.is_empty() {
                    Vec4f::default()
                } else {
                    Vec4f {
                        x: r,
                        y: g,
                        z: b,
                        w: a,
                    }
                },
            );
        } else {
            crate::reader_hit::draw_text(tf, cx, &self.word);
        }
        let (start, end) = tf.areas_tracker.pop_tracker();
        tf.draw_text.row_top_inset = old_inset;
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
            let word_rect = first.rect(cx);
            // Keep page-cut geometry identical when a known word loses its gloss.
            crate::reader_hit::capture_overlay(
                cx,
                first,
                Rect {
                    pos: word_rect.pos - dvec2(0.0, slot_height),
                    size: word_rect.size + dvec2(0.0, slot_height),
                },
                word_source,
            );
            if !self.gloss.is_empty() {
                // 设置页可配置的注音字号/颜色
                self.draw_gloss.text_style.font_size = appearance.font_size;
                let (r, g, b) = appearance.color;
                self.draw_gloss.color = Vec4f {
                    x: r,
                    y: g,
                    z: b,
                    w: 1.0,
                };
                let rect = first.rect(cx);
                let laidout = self.draw_gloss.layout(
                    cx,
                    0.0,
                    0.0,
                    None,
                    false,
                    Align::default(),
                    &self.gloss,
                );
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
                if let Some((r, g, b, a)) = appearance.background {
                    self.draw_fill.color = Vec4f {
                        x: r,
                        y: g,
                        z: b,
                        w: a,
                    };
                    self.draw_fill.draw_abs(
                        cx,
                        Rect {
                            pos: dvec2(x - 2.0, y - 1.0),
                            size: dvec2(gloss_w + 4.0, gloss_box_h + 2.0),
                        },
                    );
                }
                if *GLOSS_DEBUG {
                    self.draw_debug.color = Vec4f {
                        x: 0.2,
                        y: 0.8,
                        z: 0.3,
                        w: 0.25,
                    };
                    self.draw_debug.draw_abs(cx, rect);
                    self.draw_debug.color = Vec4f {
                        x: 0.9,
                        y: 0.15,
                        z: 0.15,
                        w: 0.35,
                    };
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
        if self.preview || self.gloss.is_empty() {
            return;
        }
        for area in self.drawn_areas.clone() {
            match event.hits(cx, area) {
                Hit::FingerHoverIn(_) => {
                    cx.set_cursor(MouseCursor::Hand);
                }
                Hit::FingerLongPress(lp) => {
                    cx.widget_action(
                        self.widget_uid(),
                        RubyWordAction::Clicked {
                            word: self.word.clone(),
                            gloss: self.gloss.clone(),
                            anchor: lp.abs,
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
                                anchor: fu.abs,
                            },
                        );
                    }
                }
                _ => {}
            }
        }
    }
}
