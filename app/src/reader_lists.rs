//! Display-only adaptation for EPUB lists in Makepad's Html renderer.
//! begin_list_item already draws the marker on the first line; opening a
//! paragraph immediately afterwards would move its body to the next line.
//! Keep source blocks/offsets intact and unwrap only that first paragraph.

#[derive(Default)]
struct Element {
    name: String,
    has_content: bool,
    unwrap: bool,
}

pub fn normalize_html(source: &str) -> String {
    if !source.to_ascii_lowercase().contains("<li") {
        return source.to_string();
    }
    let mut out = String::with_capacity(source.len());
    let mut stack: Vec<Element> = Vec::new();
    let mut offset = 0;
    while offset < source.len() {
        let rest = &source[offset..];
        if !rest.starts_with('<') {
            let len = rest.find('<').unwrap_or(rest.len());
            let text = &rest[..len];
            if !text.trim().is_empty() {
                if let Some(parent) = stack.last_mut() {
                    parent.has_content = true;
                }
            }
            out.push_str(text);
            offset += len;
            continue;
        }
        if rest.starts_with("<!--") {
            let Some(end) = rest.find("-->") else {
                return source.to_string();
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
            return source.to_string();
        };
        let inside = rest[1..end].trim();
        let closing = inside.starts_with('/');
        let name = inside
            .trim_start_matches('/')
            .split(|ch: char| ch.is_whitespace() || ch == '/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let unwrap = if closing {
            let Some(element) = stack.pop().filter(|element| element.name == name) else {
                // Do not partially rewrite malformed markup.
                return source.to_string();
            };
            element.unwrap
        } else {
            let unwrap = stack
                .last()
                .is_some_and(|parent| parent.name == "li" && !parent.has_content && name == "p");
            if let Some(parent) = stack.last_mut() {
                parent.has_content = true;
            }
            if !inside.ends_with('/') && !matches!(name.as_str(), "br" | "hr" | "img") {
                stack.push(Element {
                    name,
                    unwrap,
                    ..Default::default()
                });
            }
            unwrap
        };
        if !unwrap {
            out.push_str(&rest[..=end]);
        }
        offset += end + 1;
    }
    if stack.is_empty() {
        out
    } else {
        source.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use readwise_core::vocab::annotate::{annotate_html, flatten_lists, rendered_text};
    use std::collections::{HashMap, HashSet};

    #[test]
    fn epub_list_markers_and_first_paragraph_share_a_line() {
        for tag in ["ol", "ul"] {
            let html = format!(
                "<{tag}>\n<li>\n<p>First <code>item</code>.</p>\n</li>\n<li>\n<p>Next <em>item</em>.</p>\n</li>\n</{tag}>"
            );
            let display = normalize_html(&html);
            assert_eq!(display, html.replace("<p>", "").replace("</p>", ""));
            assert_eq!(rendered_text(&html), rendered_text(&display));
            assert_eq!(normalize_html(&display), display);
        }
    }

    #[test]
    fn keeps_numbering_nesting_and_subsequent_paragraphs() {
        let html = "<p>Before.</p><ol start='3' type='A'><li value='7'><p>First.</p><p>Second.</p><ul><li><p>Nested.</p></li></ul><p>Last.</p></li><li>Inline.<p>Separate.</p></li></ol><p>After.</p>";
        let expected = "<p>Before.</p><ol start='3' type='A'><li value='7'>First.<p>Second.</p><ul><li>Nested.</li></ul><p>Last.</p></li><li>Inline.<p>Separate.</p></li></ol><p>After.</p>";
        assert_eq!(normalize_html(html), expected);
        assert_eq!(rendered_text(html), rendered_text(expected));
    }

    #[test]
    fn keeps_inline_code_ruby_entities_and_source_selection_offsets() {
        let html = "<ol>\n<li>\n<p>A <ruby>consumer\u{e000}消费者</ruby> calls <code>park()</code> &amp; waits.</p>\n</li></ol>";
        let source = rendered_text(html);
        let display = normalize_html(html);
        assert_eq!(source, rendered_text(&display));
        let code = crate::reader_inline_code::wrap_html(&display);
        assert_eq!(source, rendered_text(&code));
        // The rendered buffer no longer has a paragraph break after "1.".
        // Persisted ranges still refer to the ORIGINAL HTML, not the adapter.
        let native = "\n1.A consumer calls park() & waits.\n";
        let map = crate::reader_text::TextMap::from_html(&source, html, native.into()).unwrap();
        for phrase in ["consumer", "park()", "& waits."] {
            let start = source.find(phrase).unwrap();
            let (a, b) = map.native_range((start, start + phrase.len())).unwrap();
            assert_eq!(&native[a..b], phrase);
            assert_eq!(map.source_index(a), start);
            assert_eq!(map.source_index(b), start + phrase.len());
        }
    }

    #[test]
    fn already_flattened_gloss_lists_are_unchanged() {
        let html = "<ul>\n<li>\n<p>A consumer waits.</p>\n</li></ul>";
        let glosses = HashMap::from([("consumer".to_string(), "消费者".to_string())]);
        let glossed = annotate_html(&flatten_lists(html), &glosses, &HashSet::new());
        assert!(glossed.contains("<ruby>consumer"));
        assert_eq!(normalize_html(&glossed), glossed);
    }

    #[test]
    fn handles_attributes_comments_void_tags_and_malformed_input() {
        let html = "<UL><LI><!-- > --><P title='a > b'>Text<br/>next<img src='x'/></P></LI></UL>";
        assert_eq!(
            normalize_html(html),
            "<UL><LI><!-- > -->Text<br/>next<img src='x'/></LI></UL>"
        );
        for html in [
            "<li><p>Unclosed",
            "<li><p>Mismatch</li></p>",
            "<li><p title='unterminated>Text</p></li>",
            "<li><!-- unclosed",
            "<li><img src='x'/><p>After image.</p></li>",
            "<p>Ordinary paragraph.</p>",
        ] {
            assert_eq!(normalize_html(html), html);
        }
    }
}
