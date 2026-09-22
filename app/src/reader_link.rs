//! Reader links use color instead of HtmlLink's built-in underline.
use makepad_widgets::html::HtmlLinkAction;
use makepad_widgets::makepad_html::HtmlDoc;
use makepad_widgets::*;

script_mod! {
    use mod.prelude.widgets.*

    mod.widgets.ReaderLinkBase = #(ReaderLink::register_widget(vm))
    mod.widgets.ReaderLink = set_type_default() do mod.widgets.ReaderLinkBase{
        width: Fit height: Fit
    }
}

#[derive(Script, Widget)]
pub struct ReaderLink {
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
    #[rust]
    drawn_areas: Vec<Area>,
    #[rust]
    hovered: bool,
    #[rust]
    pressed: bool,
    #[live(true)]
    grab_key_focus: bool,
    #[live]
    text: ArcStringMut,
    #[live]
    url: String,
}

impl ScriptHook for ReaderLink {
    fn on_after_new_scoped(&mut self, _vm: &mut ScriptVm, scope: &mut Scope) {
        if let Some(doc) = scope.props.get::<HtmlDoc>() {
            let mut walker = doc.new_walker_with_index(scope.index + 1);
            while let Some((name, value)) = walker.while_attr_lc() {
                if name == live_id!(href) {
                    self.url = value.into();
                    break;
                }
            }
        }
    }
}

impl ReaderLink {
    fn redraw_text(&self, cx: &mut Cx, scope: &mut Scope) {
        if let Some(tf) = scope.data.get_mut::<TextFlow>() {
            tf.redraw(cx);
        } else {
            for area in &self.drawn_areas {
                area.redraw(cx);
            }
        }
    }
}

impl Widget for ReaderLink {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        let mut redraw = false;
        for area in self.drawn_areas.clone() {
            match event.hits(cx, area) {
                Hit::FingerDown(fe) if fe.is_primary_hit() => {
                    if self.grab_key_focus {
                        cx.set_key_focus(self.area());
                    }
                    self.pressed = true;
                    redraw = true;
                }
                Hit::FingerDown(fe) if fe.mouse_button().is_some_and(|b| b.is_secondary()) => {
                    cx.widget_action(
                        self.widget_uid(),
                        HtmlLinkAction::SecondaryClicked {
                            url: self.url.clone(),
                            key_modifiers: fe.modifiers,
                        },
                    );
                }
                Hit::FingerHoverIn(_) => {
                    cx.set_cursor(MouseCursor::Hand);
                    self.hovered = true;
                    redraw = true;
                }
                Hit::FingerHoverOut(_) => {
                    self.hovered = false;
                    redraw = true;
                }
                Hit::FingerLongPress(_) => {
                    cx.widget_action(
                        self.widget_uid(),
                        HtmlLinkAction::SecondaryClicked {
                            url: self.url.clone(),
                            key_modifiers: Default::default(),
                        },
                    );
                }
                Hit::FingerUp(fu) => {
                    self.pressed = false;
                    self.hovered = fu.is_over;
                    redraw = true;
                    if fu.is_over {
                        cx.set_cursor(MouseCursor::Hand);
                    }
                    if fu.is_over && fu.is_primary_hit() && fu.was_tap() {
                        cx.widget_action(
                            self.widget_uid(),
                            HtmlLinkAction::Clicked {
                                url: self.url.clone(),
                                key_modifiers: fu.modifiers,
                            },
                        );
                    }
                }
                _ => {}
            }
        }
        if redraw {
            self.redraw_text(cx, scope);
        }
    }

    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, _walk: Walk) -> DrawStep {
        let Some(tf) = scope.data.get_mut::<TextFlow>() else {
            return DrawStep::done();
        };

        tf.areas_tracker.push_tracker();
        tf.font_colors.push(crate::app::current_reader_link_color(
            self.hovered,
            self.pressed,
        ));
        tf.draw_text(cx, self.text.as_ref());
        tf.font_colors.pop();
        let (start, end) = tf.areas_tracker.pop_tracker();

        if self.drawn_areas.len() == end - start {
            for index in 0..self.drawn_areas.len() {
                self.drawn_areas[index] = cx.update_area_refs(
                    self.drawn_areas[index],
                    tf.areas_tracker.areas[start + index],
                );
            }
        } else {
            self.drawn_areas = tf.areas_tracker.areas[start..end].to_vec();
        }
        DrawStep::done()
    }

    fn text(&self) -> String {
        self.text.as_ref().to_string()
    }

    fn set_text(&mut self, cx: &mut Cx, value: &str) {
        self.text.as_mut_empty().push_str(value);
        self.redraw(cx);
    }
}
