//! Restore confirmed PDF symbol-font bullets without changing block/byte coordinates.
//! F0A1 (Wingdings2) and U+2022 are both three UTF-8 bytes; an existing ASCII
//! separator becomes a <br/> (one rendered newline byte). No prose is removed.
use super::{content, headings::BlockTarget};
use crate::vocab::annotate::rendered_text;
use pdf_oxide::{layout::TextSpan, PdfDocument};
use std::collections::BTreeMap;

const PRIVATE_BULLET: char = '\u{f0a1}';
const BULLET: char = '•';

fn symbol_font(name: &str) -> bool {
    name.rsplit('+')
        .next()
        .unwrap_or(name)
        .eq_ignore_ascii_case("Wingdings2")
}

fn rewrite(html: &str, edits: &BTreeMap<usize, &str>) -> String {
    let mut result = String::with_capacity(html.len());
    let mut index = 0;
    let mut byte = 0;
    while index < html.len() {
        let rest = &html[index..];
        let length = if rest.starts_with('<') {
            rest.find('>').map_or(rest.len(), |i| i + 1)
        } else if rest.starts_with('&') {
            rest.find(';').filter(|i| *i <= 10).map_or(1, |i| i + 1)
        } else {
            rest.chars().next().unwrap().len_utf8()
        };
        let token = &rest[..length];
        let visible = rendered_text(token);
        if let Some(edit) = (!visible.is_empty()).then(|| edits.get(&byte)).flatten() {
            result.push_str(edit);
        } else {
            result.push_str(token);
        }
        byte += visible.len();
        index += length;
    }
    result
}

pub(super) fn project(html: &str, spans: &[TextSpan]) -> (String, Vec<BlockTarget>) {
    let geo = content::Geometry::new(spans, &[]);
    let blocks = content::prepare_blocks(html, &geo);
    let mut output = Vec::with_capacity(blocks.len());
    let mut maps = Vec::with_capacity(blocks.len());
    let mut changed = false;
    for (index, block) in blocks.iter().enumerate() {
        let mut edits = BTreeMap::new();
        // Keep existing lists, code listings and unrelated glyphs untouched.
        if block.html.starts_with("<p>") && !block.html.contains("<pre") {
            let chars: Vec<_> = block.text.char_indices().collect();
            for (i, ((byte, ch), glyph)) in chars.iter().zip(&block.glyphs).enumerate() {
                if *ch != PRIVATE_BULLET {
                    continue;
                }
                let Some(glyph) = glyph.map(|id| &geo.glyphs[id]) else {
                    continue;
                };
                if !symbol_font(&spans[glyph.span].font_name) || glyph.mono {
                    continue;
                }
                let next = block.glyphs[i + 1..]
                    .iter()
                    .flatten()
                    .next()
                    .map(|id| &geo.glyphs[*id]);
                let Some(next) = next else { continue };
                let em = glyph.size.max(next.size);
                // Only an actual source-line list marker, followed by text on
                // that line. Inline symbol examples/legends are not list starts.
                if (glyph.rect.y - next.rect.y).abs() > em * 0.25
                    || next.rect.x < glyph.rect.x + glyph.rect.width * 0.5
                    || next.rect.x - glyph.rect.x > em * 4.0
                    || geo.glyphs.iter().any(|other| {
                        other.rect.x < glyph.rect.x - em * 0.1
                            && (other.rect.y - glyph.rect.y).abs() < em * 0.25
                    })
                {
                    continue;
                }
                edits.insert(*byte, "•");
                if let Some((previous_byte, previous)) = i.checked_sub(1).map(|p| chars[p]) {
                    if previous.is_ascii_whitespace() {
                        edits.insert(previous_byte, "<br/>");
                    }
                }
            }
        }
        let fixed = rewrite(&block.html, &edits);
        let layout_changed = fixed != block.html;
        debug_assert!(valid_text_change(&block.text, &rendered_text(&fixed)));
        changed |= layout_changed;
        maps.push(BlockTarget {
            block: index,
            prefix: 0,
            old_len: block.text.len(),
            offsets: None,
            layout_changed,
        });
        output.push(fixed);
    }
    (
        if changed {
            output.join("\n")
        } else {
            html.to_owned()
        },
        maps,
    )
}

/// The v5 migration may change ONLY confirmed marker spelling and its adjacent
/// separator. Every prose byte, UTF-8 boundary and block index stays identical.
pub(super) fn valid_text_change(old: &str, new: &str) -> bool {
    if old.len() != new.len() {
        return false;
    }
    old.char_indices().all(|(byte, ch)| {
        let Some(next) = new.get(byte..).and_then(|s| s.chars().next()) else {
            return false;
        };
        ch == next
            || (ch == PRIVATE_BULLET && next == BULLET)
            || (ch.is_ascii_whitespace()
                && next == '\n'
                && old[byte + 1..].starts_with(PRIVATE_BULLET)
                && new[byte + 1..].starts_with(BULLET))
    })
}

pub(super) fn repair(
    doc: &PdfDocument,
    page: usize,
    html: &str,
) -> Result<(String, Vec<BlockTarget>), String> {
    let spans = if html.contains(PRIVATE_BULLET) {
        doc.extract_spans(page)
            .map_err(|e| format!("读取 PDF 列表字体失败: {e}"))?
    } else {
        Vec::new()
    };
    Ok(project(html, &spans))
}

#[cfg(test)]
#[path = "lists_tests.rs"]
pub(super) mod tests;
