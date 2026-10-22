//! Classify printed contents pages before prose reflow, and restore their rows
//! from glyph geometry. PDF outlines (navigation) are a separate data source.
use super::{
    content::{self, Geometry},
    headings::BlockTarget,
};
use crate::vocab::annotate::rendered_text;
use pdf_oxide::{
    layout::{TextChar, TextSpan},
    PdfDocument,
};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy)]
struct Position {
    x: f32,
    y: f32,
    width: f32,
    size: f32,
}

struct Row {
    baseline: f32,
    size: f32,
    glyphs: Vec<usize>,
}

fn position(geo: &Geometry, id: usize, chars: &[TextChar]) -> Position {
    let g = &geo.glyphs[id];
    // Span merging can give the beginning of a mixed-size title the lowered
    // baseline of its large chapter number. Recover each original char origin.
    let raw = chars
        .iter()
        .filter(|c| {
            c.char == g.ch
                && (c.origin_x - g.rect.x).abs() < g.size * 0.6
                && (c.origin_y - g.rect.y).abs() < g.size * 0.85
        })
        .min_by(|a, b| {
            ((a.origin_x - g.rect.x).abs() * 2.0 + (a.origin_y - g.rect.y).abs())
                .total_cmp(&((b.origin_x - g.rect.x).abs() * 2.0 + (b.origin_y - g.rect.y).abs()))
        });
    raw.map_or(
        Position {
            x: g.rect.x,
            y: g.rect.y,
            width: g.rect.width,
            size: g.size,
        },
        |c| Position {
            x: c.origin_x,
            y: c.origin_y,
            width: c.advance_width,
            size: c.font_size,
        },
    )
}

fn rows(ids: &HashSet<usize>, positions: &[Position]) -> Vec<Row> {
    let mut ids: Vec<_> = ids.iter().copied().collect();
    ids.sort_by(|a, b| {
        positions[*b]
            .y
            .total_cmp(&positions[*a].y)
            .then(positions[*a].x.total_cmp(&positions[*b].x))
            .then(a.cmp(b))
    });
    let mut rows: Vec<Row> = Vec::new();
    for id in ids {
        let p = positions[id];
        if let Some(row) = rows
            .last_mut()
            .filter(|r| (r.baseline - p.y).abs() <= r.size.max(p.size) * 0.4)
        {
            row.size = row.size.max(p.size);
            row.glyphs.push(id);
        } else {
            rows.push(Row {
                baseline: p.y,
                size: p.size,
                glyphs: vec![id],
            });
        }
    }
    for row in &mut rows {
        row.glyphs
            .sort_by(|a, b| positions[*a].x.total_cmp(&positions[*b].x).then(a.cmp(b)));
    }
    rows
}

fn spaced(_geo: &Geometry, positions: &[Position], a: usize, b: usize) -> bool {
    let (a, b) = (positions[a], positions[b]);
    b.x - (a.x + a.width) > a.size.min(b.size) * 0.14
}

fn row_text(row: &Row, geo: &Geometry, positions: &[Position]) -> String {
    let mut text = String::new();
    for (i, id) in row.glyphs.iter().enumerate() {
        if i > 0 && spaced(geo, positions, row.glyphs[i - 1], *id) {
            text.push(' ');
        }
        text.push(geo.glyphs[*id].ch);
    }
    text
}

fn page_number(text: &str) -> bool {
    let token = text.split_whitespace().last().unwrap_or("");
    !token.is_empty()
        && token.len() <= 8
        && (token.chars().all(|c| c.is_ascii_digit())
            || token.chars().all(|c| "ivxlcdmIVXLCDM".contains(c)))
}

fn columns(rows: Vec<Row>, geo: &Geometry, positions: &[Position]) -> Vec<Row> {
    let mut anchors: Vec<(f32, usize, f32)> = Vec::new();
    for row in &rows {
        let mut i = 0;
        while i < row.glyphs.len() {
            let start = i;
            while i + 1 < row.glyphs.len()
                && !spaced(geo, positions, row.glyphs[i], row.glyphs[i + 1])
            {
                i += 1;
            }
            let end = i + 1;
            let token: String = row.glyphs[start..end]
                .iter()
                .map(|id| geo.glyphs[*id].ch)
                .collect();
            let p = positions[row.glyphs[start]];
            if token.contains('.')
                && token.chars().all(|c| c.is_ascii_digit() || c == '.')
                && token.chars().any(|c| c.is_ascii_digit())
                && end < row.glyphs.len()
                && geo.glyphs[row.glyphs[end]].ch.is_alphabetic()
                && (start == 0
                    || p.x
                        - (positions[row.glyphs[start - 1]].x
                            + positions[row.glyphs[start - 1]].width)
                        > p.size * 3.0)
            {
                if let Some(a) = anchors
                    .iter_mut()
                    .find(|a| (a.0 - p.x).abs() < p.size * 0.6)
                {
                    a.1 += 1;
                    a.2 = a.2.max(row.baseline);
                } else {
                    anchors.push((p.x, 1, row.baseline));
                }
            }
            i = end;
        }
    }
    anchors.retain(|a| a.1 >= 3);
    anchors.sort_by(|a, b| a.0.total_cmp(&b.0));
    let Some(left) = anchors.first().copied() else {
        return rows;
    };
    let min_x = positions.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
    let max_x = positions
        .iter()
        .map(|p| p.x + p.width)
        .fold(f32::NEG_INFINITY, f32::max);
    let Some(right) = anchors
        .iter()
        .copied()
        .find(|a| a.0 - left.0 > (max_x - min_x) * 0.3)
    else {
        return rows;
    };
    let boundary = right.0 - 2.0;
    let top = left.2.max(right.2);
    let mut header = Vec::new();
    let mut first = Vec::new();
    let mut second = Vec::new();
    let mut footer = Vec::new();
    for row in rows {
        if row.baseline > top + row.size * 0.5 {
            header.push(row);
            continue;
        }
        let text = row_text(&row, geo, positions);
        if text
            .chars()
            .all(|c| c.is_whitespace() || c.is_ascii_digit() || "ivxlcdmIVXLCDM".contains(c))
        {
            footer.push(row);
            continue;
        }
        let (a, b): (Vec<_>, Vec<_>) = row
            .glyphs
            .into_iter()
            .partition(|id| positions[*id].x < boundary);
        if !a.is_empty() {
            first.push(Row {
                baseline: row.baseline,
                size: row.size,
                glyphs: a,
            });
        }
        if !b.is_empty() {
            second.push(Row {
                baseline: row.baseline,
                size: row.size,
                glyphs: b,
            });
        }
    }
    header.extend(first);
    header.extend(second);
    header.extend(footer);
    header
}

pub(super) fn is_contents(spans: &[TextSpan]) -> bool {
    spans.iter().any(|s| {
        matches!(
            s.text.trim().to_lowercase().as_str(),
            "contents" | "table of contents" | "目录" | "目錄"
        )
    })
}

fn complete_mapping(blocks: &mut [content::Block], geo: &Geometry) -> Option<HashSet<usize>> {
    let mut used = HashSet::new();
    for block in blocks.iter() {
        for id in block.glyphs.iter().flatten() {
            if !used.insert(*id) {
                log::warn!("PDF 目录重复字形映射 {id}");
                return None;
            }
        }
    }
    // Extraction can relocate a short page number ahead of a whole title.
    // The prose mapper deliberately rejects such short matches. Recover only
    // complete, UNIQUE, still-unused runs, never guess an individual digit.
    for block in blocks.iter_mut() {
        let chars: Vec<_> = block.text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i].is_whitespace() || block.glyphs[i].is_some() {
                i += 1;
                continue;
            }
            let start = i;
            while i < chars.len() && (chars[i].is_whitespace() || block.glyphs[i].is_none()) {
                i += 1;
            }
            let indices: Vec<_> = (start..i).filter(|j| !chars[*j].is_whitespace()).collect();
            let candidates: Vec<_> = (0..geo.glyphs.len())
                .filter(|g| {
                    indices.iter().enumerate().all(|(k, j)| {
                        geo.glyphs
                            .get(g + k)
                            .is_some_and(|glyph| glyph.ch == chars[*j] && !used.contains(&(g + k)))
                    })
                })
                .collect();
            if candidates.len() != 1
                && chars[start..i]
                    .iter()
                    .all(|c| c.is_whitespace() || c.is_ascii_digit() || *c == '.')
            {
                // Some converters move an aligned column of page numbers to
                // the end of the page. Reattach the complete numeric tokens
                // down that single column, preserving duplicate number order.
                let mut cursor = start;
                while cursor < i {
                    if chars[cursor].is_whitespace() {
                        cursor += 1;
                        continue;
                    }
                    if chars[cursor] == '.' {
                        let begin = cursor;
                        while cursor < i && (chars[cursor] == '.' || chars[cursor].is_whitespace())
                        {
                            cursor += 1;
                        }
                        let dots: Vec<_> = (begin..cursor).filter(|j| chars[*j] == '.').collect();
                        let options: Vec<_> = (0..geo.glyphs.len())
                            .filter(|g| {
                                (0..dots.len()).all(|k| {
                                    geo.glyphs
                                        .get(g + k)
                                        .is_some_and(|v| v.ch == '.' && !used.contains(&(g + k)))
                                }) && g
                                    .checked_sub(1)
                                    .and_then(|j| geo.glyphs.get(j).map(|v| (j, v)))
                                    .is_none_or(|(j, v)| v.ch != '.' || used.contains(&j))
                                    && geo.glyphs.get(g + dots.len()).is_none_or(|v| {
                                        v.ch != '.' || used.contains(&(g + dots.len()))
                                    })
                            })
                            .collect();
                        if options.len() != 1 {
                            return None;
                        }
                        for (k, j) in dots.into_iter().enumerate() {
                            let id = options[0] + k;
                            block.glyphs[j] = Some(id);
                            used.insert(id);
                        }
                        continue;
                    }
                    let begin = cursor;
                    while cursor < i && chars[cursor].is_ascii_digit() {
                        cursor += 1;
                    }
                    let token = &chars[begin..cursor];
                    let mut options: Vec<_> = (0..geo.glyphs.len())
                        .filter(|g| {
                            token.iter().enumerate().all(|(k, ch)| {
                                geo.glyphs
                                    .get(g + k)
                                    .is_some_and(|v| v.ch == *ch && !used.contains(&(g + k)))
                            }) && (0..token.len())
                                .all(|k| geo.glyphs[*g + k].span == geo.glyphs[*g].span)
                                && g.checked_sub(1).and_then(|j| geo.glyphs.get(j)).is_none_or(
                                    |v| v.span != geo.glyphs[*g].span || !v.ch.is_ascii_digit(),
                                )
                                && geo.glyphs.get(g + token.len()).is_none_or(|v| {
                                    v.span != geo.glyphs[*g].span || !v.ch.is_ascii_digit()
                                })
                        })
                        .collect();
                    let min = options
                        .iter()
                        .map(|g| geo.glyphs[*g].rect.x)
                        .fold(f32::INFINITY, f32::min);
                    let max = options
                        .iter()
                        .map(|g| geo.glyphs[*g].rect.x)
                        .fold(f32::NEG_INFINITY, f32::max);
                    if options.is_empty() || max - min > geo.glyphs[options[0]].size * 4.0 {
                        return None;
                    }
                    options.sort_by(|a, b| {
                        geo.glyphs[*b]
                            .rect
                            .y
                            .total_cmp(&geo.glyphs[*a].rect.y)
                            .then(a.cmp(b))
                    });
                    for k in 0..token.len() {
                        let id = options[0] + k;
                        block.glyphs[begin + k] = Some(id);
                        used.insert(id);
                    }
                }
                continue;
            }
            if candidates.len() != 1 {
                log::warn!(
                    "PDF 目录未对应片段 {:?}，候选 {}",
                    indices.iter().map(|j| chars[*j]).collect::<String>(),
                    candidates.len()
                );
                return None;
            }
            for (k, j) in indices.iter().enumerate() {
                let id = candidates[0] + k;
                block.glyphs[*j] = Some(id);
                used.insert(id);
            }
        }
    }
    Some(used)
}

/// Return None unless the page has a contents heading, several numbered rows,
/// and an exact one-to-one mapping of EVERY source non-whitespace character.
pub(super) fn project(
    html: &str,
    spans: &[TextSpan],
    chars: &[TextChar],
) -> Option<(String, Vec<BlockTarget>)> {
    if !is_contents(spans) || html.contains("<img") || html.contains("<pre") {
        return None;
    }
    let geo = Geometry::new(spans, &[]);
    let mut blocks = content::prepare_blocks(html, &geo);
    let Some(used) = complete_mapping(&mut blocks, &geo) else {
        log::warn!("PDF 目录字形无法唯一对应，保留原排版以保护标注");
        return None;
    };
    let positions: Vec<_> = (0..geo.glyphs.len())
        .map(|id| position(&geo, id, chars))
        .collect();
    let rows = columns(rows(&used, &positions), &geo, &positions);
    let texts: Vec<_> = rows.iter().map(|r| row_text(r, &geo, &positions)).collect();
    let numbered = texts.iter().filter(|s| page_number(s)).count();
    let sections = texts
        .iter()
        .filter(|s| {
            let token = s.split_whitespace().next().unwrap_or("");
            token.contains('.')
                && token.chars().all(|c| c.is_ascii_digit() || c == '.')
                && token.chars().any(|c| c.is_ascii_digit())
        })
        .count();
    if numbered < 4 && sections < 3 && !(sections > 0 && used.len() < 250) {
        return None;
    }
    let mut body = String::new();
    let mut text = String::new();
    let mut destinations = HashMap::new();
    for (row_index, row) in rows.iter().enumerate() {
        if row_index > 0 {
            body.push_str("<br/>");
            text.push('\n');
        }
        let mut active = (false, false, false);
        for (index, id) in row.glyphs.iter().enumerate() {
            let span = &spans[geo.glyphs[*id].span];
            let style = (
                span.font_weight.is_bold(),
                span.is_italic,
                geo.glyphs[*id].mono,
            );
            if style != active {
                if active.2 {
                    body.push_str("</code>");
                }
                if active.1 {
                    body.push_str("</em>");
                }
                if active.0 {
                    body.push_str("</strong>");
                }
                if style.0 {
                    body.push_str("<strong>");
                }
                if style.1 {
                    body.push_str("<em>");
                }
                if style.2 {
                    body.push_str("<code>");
                }
                active = style;
            }
            if index > 0 && spaced(&geo, &positions, row.glyphs[index - 1], *id) {
                body.push(' ');
                text.push(' ');
            }
            destinations.insert(*id, text.len());
            let ch = geo.glyphs[*id].ch;
            body.push_str(&content::escape(&ch.to_string()));
            text.push(ch);
        }
        if active.2 {
            body.push_str("</code>");
        }
        if active.1 {
            body.push_str("</em>");
        }
        if active.0 {
            body.push_str("</strong>");
        }
    }
    let mut maps = Vec::new();
    let mut last_anchor = 0;
    for block in &blocks {
        let mut offsets = vec![0; block.text.len() + 1];
        let anchor = block
            .glyphs
            .iter()
            .flatten()
            .find_map(|id| destinations.get(id))
            .copied()
            .unwrap_or(last_anchor);
        let mut current = anchor;
        for ((byte, ch), id) in block.text.char_indices().zip(&block.glyphs) {
            if let Some(id) = id {
                current = *destinations.get(id)?;
            }
            for k in 0..ch.len_utf8() {
                offsets[byte + k] = current
                    + k.min(if ch.is_whitespace() {
                        0
                    } else {
                        ch.len_utf8() - 1
                    });
            }
            offsets[byte + ch.len_utf8()] = if ch.is_whitespace() {
                current
            } else {
                current + ch.len_utf8()
            };
        }
        // Whitespace-only source blocks still anchor near their preceding run.
        if block.text.is_empty() {
            offsets[0] = anchor;
        }
        last_anchor = offsets[block.text.len()];
        maps.push(BlockTarget {
            block: 0,
            prefix: offsets[0],
            old_len: block.text.len(),
            offsets: Some(offsets),
            layout_changed: true,
        });
    }
    let result = format!("<p>{body}</p>");
    debug_assert_eq!(rendered_text(&result), text);
    Some((result, maps))
}

pub(super) fn repair(
    doc: &PdfDocument,
    page: usize,
    html: &str,
) -> Result<Option<(String, Vec<BlockTarget>)>, String> {
    let text = rendered_text(html).to_lowercase();
    if !text.contains("contents") && !text.contains("目录") && !text.contains("目錄") {
        return Ok(None);
    }
    let spans = doc
        .extract_spans(page)
        .map_err(|e| format!("读取 PDF 目录行失败: {e}"))?;
    if !is_contents(&spans) {
        return Ok(None);
    }
    let chars = doc
        .extract_chars(page)
        .map_err(|e| format!("读取 PDF 目录字形失败: {e}"))?;
    Ok(project(html, &spans, &chars))
}

#[cfg(test)]
#[path = "toc_tests.rs"]
pub(super) mod tests;
