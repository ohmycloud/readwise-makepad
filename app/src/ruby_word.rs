use makepad_widgets::makepad_html::HtmlDoc;
use makepad_widgets::*;

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
    #[live(2.0)]
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
        self.word = v.to_string();
        self.redraw(cx);
    }

    fn text(&self) -> String {
        self.word.clone()
    }

    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, _walk: Walk) -> DrawStep {
        let Some(tf) = scope.data.get_mut::<TextFlow>() else {
            return DrawStep::done();
        };

        tf.areas_tracker.push_tracker();
        tf.draw_text(cx, &self.word);
        let (start, end) = tf.areas_tracker.pop_tracker();

        if self.drawn_areas.len() == end - start {
            for i in 0..end - start {
                self.drawn_areas[i] =
                    cx.update_area_refs(self.drawn_areas[i], tf.areas_tracker.areas[i + start]);
            }
        } else {
            self.drawn_areas = tf.areas_tracker.areas[start..end].to_vec();
        }

        if !self.gloss.is_empty() {
            if let Some(first) = self.drawn_areas.first() {
                let rect = first.rect(cx);
                let laidout =
                    self.draw_gloss
                        .layout(cx, 0.0, 0.0, None, false, Align::default(), &self.gloss);
                let gloss_w = laidout
                    .rows
                    .first()
                    .map(|r| (r.width_in_lpxs * self.draw_gloss.font_scale) as f64)
                    .unwrap_or(0.0);
                let gloss_h =
                    (self.draw_gloss.text_style.font_size * self.draw_gloss.font_scale) as f64;
                let x = rect.pos.x + (rect.size.x - gloss_w) * 0.5;
                let y = rect.pos.y - gloss_h - self.gloss_gap as f64;
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
