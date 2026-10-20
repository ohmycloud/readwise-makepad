//! Repair span-by-span heading classification using ORIGINAL PDF line geometry.
//! Never join adjacent headings merely because their tags / font sizes match.
use super::super::epub_meta::split_blocks;
use pdf_oxide::layout::TextSpan;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct BlockTarget {
    pub block: usize,
    pub prefix: usize,
    pub old_len: usize,
    /// Optional insertion-only UTF-8 byte map for restored code whitespace.
    /// Omit on legacy projections so their serialized migration token is stable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offsets: Option<Vec<usize>>,
    /// Omit on older projections so their committed token remains stable.
    #[serde(default, skip_serializing_if = "is_false")]
    pub layout_changed: bool,
}
fn is_false(value: &bool) -> bool {
    !*value
}

fn normalized(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn lines(spans: &[TextSpan]) -> Vec<String> {
    let mut spans: Vec<_> = spans
        .iter()
        .filter(|s| {
            s.font_size.is_finite()
                && s.font_size > 0.0
                && s.bbox.x.is_finite()
                && s.bbox.y.is_finite()
                && s.rotation_degrees.abs() < 0.1
                && s.wmode == 0
        })
        .collect();
    spans.sort_by(|a, b| {
        b.bbox
            .y
            .total_cmp(&a.bbox.y)
            .then(a.bbox.x.total_cmp(&b.bbox.x))
    });
    let mut lines = Vec::new();
    let mut text = String::new();
    let mut previous: Option<&TextSpan> = None;
    for span in spans {
        let same = previous.is_some_and(|p| {
            let em = p.font_size.max(span.font_size);
            let gap = span.bbox.x - (p.bbox.x + p.bbox.width);
            (p.bbox.y - span.bbox.y).abs() <= em * 0.2
                && (p.font_size - span.font_size).abs() <= em * 0.08
                && gap >= -em * 0.25
                && gap <= em * 0.8
        });
        if !same && !text.is_empty() {
            lines.push(normalized(&text));
            text.clear();
        } else if same
            && !text.ends_with(char::is_whitespace)
            && !span.text.starts_with(char::is_whitespace)
        {
            let p = previous.unwrap();
            if span.bbox.x - (p.bbox.x + p.bbox.width) > span.font_size * 0.15 {
                text.push(' ');
            }
        }
        text.push_str(&span.text);
        previous = Some(span);
    }
    if !text.is_empty() {
        lines.push(normalized(&text));
    }
    lines
}

fn inner(block: &str) -> Option<(&str, Option<u8>)> {
    let block = block.trim();
    let end = block.find('>')?;
    let tag = &block[1..end];
    let heading = match tag.as_bytes() {
        [b'h', n @ b'1'..=b'6'] => Some(n - b'0'),
        _ if tag == "p" => None,
        _ => return None,
    };
    let tail = format!("</{tag}>");
    Some((block[end + 1..].strip_suffix(&tail)?, heading))
}

/// Return unchanged HTML byte-for-byte if no complete geometric line matches.
/// Each old reader block maps to its exact UTF-8 prefix in the repaired block.
pub(super) fn repair(html: &str, spans: &[TextSpan]) -> (String, Vec<BlockTarget>) {
    let blocks = split_blocks(html);
    let texts: Vec<_> = blocks
        .iter()
        .map(|b| crate::vocab::annotate::rendered_text(b))
        .collect();
    let lines = lines(spans);
    let mut output = Vec::new();
    let mut targets = Vec::new();
    let mut i = 0;
    let mut changed = false;
    while i < blocks.len() {
        let mut replacement = None;
        for line in &lines {
            if texts[i].trim().is_empty() || !line.starts_with(texts[i].trim()) {
                continue;
            }
            let mut cursor = 0;
            let mut content = String::new();
            let mut level = None;
            let mut group = Vec::new();
            for j in i..blocks.len() {
                let Some((body, heading)) = inner(&blocks[j]) else {
                    break;
                };
                // Normalizing source text inside a block could invalidate its
                // offsets. We only relocate intact blocks, never edit their text.
                let text = &texts[j];
                if text != text.trim() || normalized(text) != *text {
                    break;
                }
                let remaining = &line[cursor..];
                let trimmed = remaining.trim_start();
                let space = remaining.len() - trimmed.len();
                if !text.is_empty() && !trimmed.starts_with(text.as_str()) {
                    break;
                }
                if !text.is_empty() {
                    if space > 0 {
                        content.push(' ');
                    }
                    cursor += space + text.len();
                }
                let prefix = crate::vocab::annotate::rendered_text(&content).len();
                group.push(BlockTarget {
                    block: output.len(),
                    prefix,
                    old_len: text.len(),
                    offsets: None,
                    layout_changed: false,
                });
                content.push_str(body);
                level = level.or(heading);
                if cursor == line.len() {
                    if j > i {
                        if let Some(level) = level {
                            replacement =
                                Some((j + 1, format!("<h{level}>{content}</h{level}>"), group));
                        }
                    }
                    break;
                }
            }
            if replacement.is_some() {
                break;
            }
        }
        if let Some((next, html, group)) = replacement {
            output.push(html);
            targets.extend(group);
            i = next;
            changed = true;
        } else {
            targets.push(BlockTarget {
                block: output.len(),
                prefix: 0,
                old_len: texts[i].len(),
                offsets: None,
                layout_changed: false,
            });
            output.push(blocks[i].clone());
            i += 1;
        }
    }
    (
        if changed {
            output.join("\n")
        } else {
            html.to_owned()
        },
        targets,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn span(text: &str, x: f32, y: f32, size: f32) -> TextSpan {
        let doc = super::super::open(super::super::tests::heading_fixture()).unwrap();
        let mut s = doc.extract_spans(0).unwrap().remove(0);
        s.text = text.into();
        s.bbox.x = x;
        s.bbox.y = y;
        s.bbox.width = 60.0;
        s.bbox.height = size;
        s.font_size = size;
        s
    }
    #[test]
    fn separate_lines_columns_and_body_styles_are_not_merged() {
        let html = "<h1>Alpha</h1>\n<h1>Beta</h1>";
        for spans in [
            vec![
                span("Alpha", 20.0, 200.0, 24.0),
                span("Beta", 20.0, 160.0, 24.0),
            ],
            vec![
                span("Alpha", 20.0, 200.0, 24.0),
                span("Beta", 300.0, 200.0, 24.0),
            ],
            vec![
                span("Alpha", 20.0, 200.0, 24.0),
                span("Beta", 85.0, 200.0, 10.0),
            ],
        ] {
            assert_eq!(repair(html, &spans).0, html);
        }
    }
    #[test]
    fn joined_heading_prefixes_are_utf8_bytes_and_empty_blocks_are_mapped() {
        let html = "<h2>入门</h2><p></p><p><strong>1</strong></p><p>Body</p>";
        let spans = vec![
            span("入门", 20.0, 200.0, 24.0),
            span("1", 85.0, 200.0, 24.0),
        ];
        let (fixed, map) = repair(html, &spans);
        assert_eq!(fixed, "<h2>入门 <strong>1</strong></h2>\n<p>Body</p>");
        assert_eq!((map[2].block, map[2].prefix), (0, "入门 ".len()));
        assert_eq!((map[3].block, map[3].prefix), (1, 0));
        assert_eq!(map.len(), 4);
    }
}
