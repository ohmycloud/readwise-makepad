//! Display-only first-line indentation. Never inserts whitespace into source offsets.
use makepad_widgets::*;

pub fn display_html(source: &str, indented: bool) -> String {
    if !indented {
        return source.to_owned();
    }
    let mut out = String::with_capacity(source.len());
    let mut stack = Vec::<String>::new();
    let mut offset = 0;
    while offset < source.len() {
        let rest = &source[offset..];
        if !rest.starts_with('<') {
            let len = rest.find('<').unwrap_or(rest.len());
            out.push_str(&rest[..len]);
            offset += len;
            continue;
        }
        if rest.starts_with("<!--") {
            let Some(end) = rest.find("-->") else {
                return source.to_owned();
            };
            out.push_str(&rest[..end + 3]);
            offset += end + 3;
            continue;
        }
        let mut quote = None;
        let Some(end) = rest.char_indices().skip(1).find_map(|(i, ch)| {
            if let Some(q) = quote {
                if ch == q {
                    quote = None;
                }
            } else if ch == '\'' || ch == '"' {
                quote = Some(ch);
            } else if ch == '>' {
                return Some(i);
            }
            None
        }) else {
            return source.to_owned();
        };
        let inside = rest[1..end].trim();
        let closing = inside.starts_with('/');
        let name = inside
            .trim_start_matches('/')
            .split(|ch: char| ch.is_whitespace() || ch == '/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        out.push_str(&rest[..=end]);
        if closing {
            if stack.pop().as_deref() != Some(&name) {
                return source.to_owned();
            }
        } else {
            // Do not change list markers, quotations, captions, tables or code.
            if name == "p"
                && !stack.iter().any(|name| {
                    matches!(
                        name.as_str(),
                        "li" | "blockquote" | "table" | "pre" | "figure" | "figcaption"
                    )
                })
            {
                out.push_str("<reader_indent></reader_indent>");
            }
            if !inside.ends_with('/')
                && !matches!(
                    name.as_str(),
                    "br" | "hr" | "img" | "input" | "meta" | "link"
                )
            {
                stack.push(name);
            }
        }
        offset += end + 1;
    }
    if stack.is_empty() {
        out
    } else {
        source.to_owned()
    }
}

script_mod! {
    use mod.prelude.widgets.*
    mod.widgets.ReaderParagraphIndentBase = #(ReaderParagraphIndent::register_widget(vm))
    mod.widgets.ReaderParagraphIndent = set_type_default() do mod.widgets.ReaderParagraphIndentBase{}
}

#[derive(Script, WidgetRegister)]
pub struct ReaderParagraphIndent {
    #[uid]
    uid: WidgetUid,
    #[source]
    source: ScriptObjectRef,
}
impl ScriptHook for ReaderParagraphIndent {}
impl WidgetNode for ReaderParagraphIndent {
    fn widget_uid(&self) -> WidgetUid {
        self.uid
    }
    fn walk(&mut self, _cx: &mut Cx) -> Walk {
        Walk::default()
    }
    fn area(&self) -> Area {
        Area::Empty
    }
    fn redraw(&mut self, _cx: &mut Cx) {}
    fn point_hits_area(&self, _cx: &Cx, _point: DVec2) -> bool {
        false
    }
}
impl Widget for ReaderParagraphIndent {
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, _walk: Walk) -> DrawStep {
        if let Some(tf) = scope.data.get_mut::<TextFlow>() {
            // A zero-height layout walk changes only the first line's x origin;
            // no synthetic character enters selection, copy or saved highlights.
            cx.walk_turtle(Walk {
                width: Size::Fixed(tf.font_size as f64 * 2.0),
                height: Size::Fixed(0.0),
                ..Walk::default()
            });
        }
        DrawStep::done()
    }
    fn handle_event(&mut self, _cx: &mut Cx, _event: &Event, _scope: &mut Scope) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use readwise_core::vocab::annotate::rendered_text;

    #[test]
    fn indentation_preserves_text_ruby_slots_and_source_offsets() {
        let source = "<h2>Title</h2><p title='a > b'>An <ruby t='词'>example</ruby> &amp; <em>more</em>.</p><p>Next.</p>";
        let display = display_html(source, true);
        assert_eq!(display.matches("<reader_indent>").count(), 2);
        assert_eq!(rendered_text(source), rendered_text(&display));
        assert!(crate::reader_hit::tracked_html(&display).is_some());
        assert_eq!(display_html(source, false), source);
    }

    #[test]
    fn special_blocks_and_malformed_markup_are_unchanged() {
        for source in [
            "<ul><li><p>Item</p></li></ul>",
            "<blockquote><p>Quote</p></blockquote>",
            "<table><tr><td><p>Cell</p></td></tr></table>",
            "<figure><p>Caption</p></figure>",
            "<p>Unclosed",
            "<p title='bad>Text</p>",
        ] {
            assert_eq!(display_html(source, true), source);
        }
    }
}
