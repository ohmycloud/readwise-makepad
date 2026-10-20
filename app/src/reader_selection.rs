//! Selection popovers deliberately keep their warm, dark ink in every reader theme.
use makepad_widgets::*;

pub fn question_prompt(selected: &str, question: &str, english: bool) -> Option<String> {
    let selected = selected.trim();
    let question = question.trim();
    if selected.is_empty() || question.is_empty() {
        return None;
    }
    Some(if english {
        format!("About this selected text:\n\n<selection>\n{selected}\n</selection>\n\n{question}")
    } else {
        format!("关于以下选中文字：\n\n<selection>\n{selected}\n</selection>\n\n{question}")
    })
}

/// Keep the second popover disjoint from the toolbar, even on a short screen.
/// The caller gives its content a scroll viewport of the returned height.
pub fn more_menu_geometry(viewport: Rect, toolbar: Rect, content_height: f64) -> Rect {
    let inset = 10.0_f64.min(viewport.size.x.min(viewport.size.y).max(0.0) * 0.25);
    let width = 288.0_f64.min((viewport.size.x - inset * 2.0).max(1.0));
    let top = viewport.pos.y + inset;
    let bottom = viewport.pos.y + viewport.size.y - inset;
    let above = (toolbar.pos.y - 8.0 - top).max(0.0);
    let below = (bottom - toolbar.pos.y - toolbar.size.y - 8.0).max(0.0);
    let on_top = content_height <= above || above >= below;
    let available = if on_top { above } else { below };
    let height = content_height.max(1.0).min(available.max(1.0));
    let x = (toolbar.pos.x + toolbar.size.x - width).clamp(
        viewport.pos.x + inset,
        (viewport.pos.x + viewport.size.x - inset - width).max(viewport.pos.x + inset),
    );
    let y = if on_top {
        toolbar.pos.y - 8.0 - height
    } else {
        toolbar.pos.y + toolbar.size.y + 8.0
    };
    Rect {
        pos: dvec2(x, y),
        size: dvec2(width, height),
    }
}

pub fn citation_quote(
    selected: &str,
    title: &str,
    chapter: usize,
    paragraph: usize,
    english: bool,
) -> String {
    if english {
        format!(
            "“{}”\n— {}, Chapter {}, paragraph {}",
            selected.trim(),
            title,
            chapter + 1,
            paragraph + 1
        )
    } else {
        format!(
            "“{}”\n——《{}》，第 {} 章，第 {} 段",
            selected.trim(),
            title,
            chapter + 1,
            paragraph + 1
        )
    }
}

script_mod! {
    use mod.prelude.widgets.*

    mod.widgets.SelectionAction = ButtonFlat{
        width: Fit height: 32
        margin: 0 padding: Inset{left: 8 right: 8 top: 4 bottom: 4}
        spacing: 5
        icon_walk: Walk{width: 16 height: 16}
        draw_bg +: {
            color: #0000
            color_hover: #x493b29
            color_down: #x574733
            color_focus: #x493b29
            border_size: 0.0
            border_radius: 8.0
            pixel: fn() {
                let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                sdf.box(0.0, 0.0, self.rect_size.x, self.rect_size.y, self.border_radius)
                sdf.fill(self.color.mix(self.color_focus, self.focus).mix(self.color_hover, self.hover).mix(self.color_down, self.down))
                return sdf.result
            }
        }
        draw_text +: {
            text_style.font_size: 11
            color: #xf3ead8 color_hover: #xfff7e8 color_down: #xfff7e8 color_focus: #xfff7e8
        }
        draw_icon +: {
            color: #xb3a184 color_hover: #xf3ead8 color_down: #xf3ead8 color_focus: #xf3ead8
        }
    }
    mod.widgets.SelectionSwatch = mod.widgets.SelectionAction{
        width: 32 height: 32 padding: 0 text: ""
        draw_bg +: {
            color: #xf2c438
            border_color: #xf3ead8
            border_size: 0.0
            pixel: fn() {
                let center = self.rect_size * 0.5
                let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                if self.border_size > 0.0 {
                    sdf.circle(center.x, center.y, 13.0)
                    sdf.stroke(self.border_color, 1.0)
                }
                sdf.circle(center.x, center.y, 9.0 - self.down)
                sdf.fill(self.color)
                return sdf.result
            }
        }
    }
    mod.widgets.SelectionMenuItem = View{
        width: Fill height: 58 flow: Overlay new_batch: true
        hit := mod.widgets.SelectionAction{
            width: Fill height: Fill text: "" padding: 0
            enable_long_press: true
        }
        content := View{
            width: Fill height: Fill new_batch: true
            padding: Inset{left: 10 right: 10 top: 9 bottom: 7}
            flow: Right spacing: 10
            icon := Icon{
                width: 17 height: 18
                icon_walk: Walk{width: 17 height: 17}
                draw_icon.color: #xb3a184
            }
            labels := View{
                width: Fill height: Fit flow: Down spacing: 3
                menu_title := Label{width: Fill height: Fit padding: 0 text: "" draw_text.color: #xf3ead8 draw_text.text_style.font_size: 12}
                menu_subtitle := Label{width: Fill height: Fit padding: 0 text: "" draw_text.color: #xaa9779 draw_text.text_style.font_size: 10}
            }
        }
    }
    mod.widgets.SelectionQuestionInput = TextInput{
        width: Fill height: 36 margin: 0 padding: Inset{left: 8 right: 6 top: 8 bottom: 6}
        empty_text: "Ask about this word"
        draw_bg +: {
            pixel: fn() { return vec4(0.0) }
        }
        draw_text +: {
            text_style.font_size: 12
            color: #xf3ead8 color_hover: #xf3ead8 color_focus: #xf3ead8
            color_empty: #xaa9779 color_empty_hover: #xaa9779 color_empty_focus: #xaa9779
        }
        draw_cursor.color: #xf3ead8
        draw_selection.color: #x65563d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn more_menu_avoids_toolbar_and_scrolls_in_short_viewports() {
        for (size, toolbar) in [
            (
                dvec2(1100.0, 900.0),
                Rect {
                    pos: dvec2(300.0, 700.0),
                    size: dvec2(440.0, 44.0),
                },
            ),
            (
                dvec2(320.0, 568.0),
                Rect {
                    pos: dvec2(12.0, 200.0),
                    size: dvec2(296.0, 80.0),
                },
            ),
            (
                dvec2(844.0, 240.0),
                Rect {
                    pos: dvec2(200.0, 100.0),
                    size: dvec2(440.0, 44.0),
                },
            ),
        ] {
            let viewport = Rect {
                pos: dvec2(0.0, 0.0),
                size,
            };
            let menu = more_menu_geometry(viewport, toolbar, 360.0);
            assert!(menu.pos.x >= 0.0 && menu.pos.y >= 0.0);
            assert!(menu.pos.x + menu.size.x <= size.x);
            assert!(menu.pos.y + menu.size.y <= size.y);
            assert!(
                menu.pos.y + menu.size.y <= toolbar.pos.y
                    || menu.pos.y >= toolbar.pos.y + toolbar.size.y
            );
            assert!(menu.size.y <= 360.0);
        }
    }

    #[test]
    fn copied_quote_includes_title_and_one_based_source_location() {
        for english in [true, false] {
            let quote = citation_quote(" a quoted sentence ", "Test Book", 1, 6, english);
            assert!(quote.starts_with("“a quoted sentence”"));
            assert!(quote.contains("Test Book"));
            assert!(quote.contains('2') && quote.contains('7'));
        }
    }

    #[test]
    fn question_requires_both_selection_and_question() {
        assert!(question_prompt(" ", "Why?", true).is_none());
        assert!(question_prompt("word", "\n", false).is_none());
    }

    #[test]
    fn question_retains_selected_context_and_user_question() {
        for english in [true, false] {
            let prompt =
                question_prompt("  a particular word  ", "  Why here?  ", english).unwrap();
            assert!(prompt.contains("<selection>\na particular word\n</selection>"));
            assert!(prompt.ends_with("\n\nWhy here?"));
        }
    }
}
