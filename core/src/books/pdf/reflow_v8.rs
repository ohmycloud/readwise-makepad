//! v8 extends geometric reflow to pages with empty/converter-generated lists
//! and compiler gutter columns. Intentionally forked from the frozen v7 engine:
//! regenerating an older committed projection must produce identical bytes.
//! Real lists remain opaque semantic blocks; do not flatten or discard them.
use super::{
    content::{self, Geometry},
    headings::BlockTarget,
};
use crate::{books::epub_meta::split_blocks, vocab::annotate::rendered_text};
use pdf_oxide::{
    geometry::Rect,
    layout::{TextChar, TextSpan},
    PdfDocument,
};
use std::collections::{BTreeMap, HashMap, HashSet};

pub(super) type Point = (usize, usize);
pub(super) const POINT_PACK: usize = 1_000_000;
pub(super) struct Repair {
    pub html: String,
    pub maps: Vec<BlockTarget>,
    pub source: Vec<String>,
    #[cfg(test)]
    pub points: Vec<Vec<Point>>,
}

// Assign long anchors first, then complete short runs using UNUSED glyphs and
// neighbouring anchors. The old locator can reuse the same `|`/`}` repeatedly;
// that is not a bijection and cannot support reordering persisted highlights.
fn locate(source: &[String], geo: &Geometry) -> Option<Vec<Vec<Option<usize>>>> {
    let texts: Vec<Vec<char>> = source.iter().map(|s| s.chars().collect()).collect();
    let indices: Vec<Vec<usize>> = texts
        .iter()
        .map(|s| {
            s.iter()
                .enumerate()
                .filter_map(|(i, c)| (!c.is_whitespace()).then_some(i))
                .collect()
        })
        .collect();
    let mut result: Vec<Vec<Option<usize>>> = texts.iter().map(|s| vec![None; s.len()]).collect();
    let mut used = vec![false; geo.glyphs.len()];
    let mut starts = HashMap::<char, Vec<usize>>::new();
    for (i, g) in geo.glyphs.iter().enumerate() {
        starts.entry(g.ch).or_default().push(i);
    }
    for (block, ids) in indices.iter().enumerate() {
        let mut at = 0;
        while at < ids.len() {
            let mut best = (0, 0);
            for &g in starts.get(&texts[block][ids[at]]).into_iter().flatten() {
                let len = ids[at..]
                    .iter()
                    .zip(geo.glyphs[g..].iter())
                    .enumerate()
                    .take_while(|(k, (i, glyph))| !used[g + k] && texts[block][**i] == glyph.ch)
                    .count();
                if len > best.0 {
                    best = (len, g);
                }
            }
            if best.0 >= 5 {
                for k in 0..best.0 {
                    result[block][ids[at + k]] = Some(best.1 + k);
                    used[best.1 + k] = true;
                }
                at += best.0;
            } else {
                at += 1;
            }
        }
    }
    for (block, ids) in indices.iter().enumerate() {
        let mut at = 0;
        while at < ids.len() {
            if result[block][ids[at]].is_some() {
                at += 1;
                continue;
            }
            let begin = at;
            while at < ids.len() && result[block][ids[at]].is_none() {
                at += 1;
            }
            let run = &ids[begin..at];
            let before = indices[..=block]
                .iter()
                .enumerate()
                .rev()
                .find_map(|(b, prev)| {
                    prev.iter()
                        .rev()
                        .filter(|i| b < block || **i < run[0])
                        .find_map(|i| result[b][*i])
                });
            let after = indices[block..]
                .iter()
                .enumerate()
                .find_map(|(delta, next)| {
                    let b = block + delta;
                    next.iter()
                        .filter(|i| b > block || **i > *run.last().unwrap())
                        .find_map(|i| result[b][*i])
                });
            let mut candidates: Vec<_> = starts
                .get(&texts[block][run[0]])
                .into_iter()
                .flatten()
                .copied()
                .filter_map(|g| {
                    let len = run
                        .iter()
                        .enumerate()
                        .take_while(|(k, i)| {
                            geo.glyphs
                                .get(g + k)
                                .is_some_and(|v| !used[g + k] && v.ch == texts[block][**i])
                        })
                        .count();
                    (len > 0).then_some((g, len))
                })
                .collect();
            let longest = candidates.iter().map(|(_, len)| *len).max()?;
            candidates.retain(|(_, len)| *len == longest);
            // A complete run between two stable anchors has an ordered origin.
            // Otherwise select the first unused occurrence in PDF stream order.
            candidates.sort_by_key(|(g, len)| {
                let inside = before
                    .zip(after)
                    .is_some_and(|(lo, hi)| lo < *g && *g + len <= hi);
                (!inside, before.map_or(*g, |lo| g.abs_diff(lo + 1)), *g)
            });
            let (g, len) = *candidates.first()?;
            for (k, i) in run[..len].iter().enumerate() {
                result[block][*i] = Some(g + k);
                used[g + k] = true;
            }
            at = begin + len;
        }
    }
    Some(result)
}

struct Row {
    ids: Vec<usize>,
    y: f32,
    size: f32,
    code: bool,
    heading: bool,
    opaque: Option<usize>,
}
fn rows(
    geo: &Geometry,
    spans: &[TextSpan],
    used: &[bool],
    heading_ids: &[bool],
    owners: &[usize],
    protected: &[bool],
) -> Option<Vec<Row>> {
    let mut ids: Vec<_> = (0..geo.glyphs.len()).filter(|i| used[*i]).collect();
    ids.sort_by(|a, b| {
        geo.glyphs[*b]
            .rect
            .y
            .total_cmp(&geo.glyphs[*a].rect.y)
            .then(geo.glyphs[*a].rect.x.total_cmp(&geo.glyphs[*b].rect.x))
            .then(a.cmp(b))
    });
    let mut rows: Vec<Row> = Vec::new();
    for id in ids {
        let g = &geo.glyphs[id];
        if let Some(row) = rows
            .last_mut()
            .filter(|r| (r.y - g.rect.y).abs() < r.size.max(g.size) * 0.55)
        {
            row.ids.push(id);
            if g.size > row.size {
                row.y = g.rect.y;
            }
            row.size = row.size.max(g.size);
        } else {
            rows.push(Row {
                ids: vec![id],
                y: g.rect.y,
                size: g.size,
                code: false,
                heading: false,
                opaque: None,
            });
        }
    }
    let mut sizes = BTreeMap::<i32, usize>::new();
    for row in &rows {
        for id in &row.ids {
            let g = &geo.glyphs[*id];
            if !g.mono && !spans[g.span].font_weight.is_bold() {
                *sizes.entry((g.size * 10.0).round() as i32).or_default() += 1;
            }
        }
    }
    let body = sizes
        .iter()
        .max_by_key(|(_, n)| *n)
        .map(|(s, _)| *s as f32 / 10.0);
    let diagnostic = rows.iter().any(|row| {
        if !row.ids.iter().all(|id| geo.glyphs[*id].mono) {
            return false;
        }
        let mut ids = row.ids.clone();
        ids.sort_by(|a, b| {
            geo.glyphs[*a]
                .rect
                .x
                .total_cmp(&geo.glyphs[*b].rect.x)
                .then(a.cmp(b))
        });
        let text: String = ids.iter().map(|id| geo.glyphs[*id].ch).collect();
        ["error[", "error:", "-->", "=note:"]
            .iter()
            .any(|p| text.starts_with(p))
    });
    let row_count = rows.len();
    for (row_index, row) in rows.iter_mut().enumerate() {
        row.ids.sort_by(|a, b| {
            geo.glyphs[*a]
                .rect
                .x
                .total_cmp(&geo.glyphs[*b].rect.x)
                .then(a.cmp(b))
        });
        // Do not sort unrelated columns into a single line. Large compiler
        // diagnostic gutters have only a short `|`/line number on the left.
        for k in 1..row.ids.len() {
            let (a, b) = (&geo.glyphs[row.ids[k - 1]], &geo.glyphs[row.ids[k]]);
            if b.rect.x - (a.rect.x + a.rect.width) > row.size * 5.0
                && k >= 3
                && row.ids.len() - k >= 3
            {
                // Running heads put the page number at the opposite margin.
                // Three-digit counters must not make every later book page
                // look like a two-column body. Require an outermost row and
                // a non-code, all-numeric side; short code columns stay unsafe.
                let counter = |ids: &[usize]| {
                    ids.iter().all(|id| {
                        let g = &geo.glyphs[*id];
                        !g.mono && g.ch.is_ascii_digit()
                    })
                };
                let running_head = (row_index == 0 || row_index + 1 == row_count)
                    && (counter(&row.ids[..k]) || counter(&row.ids[k..]));
                let gutter = diagnostic
                    && row.ids.iter().all(|id| geo.glyphs[*id].mono)
                    && row.ids[..k].iter().any(|id| geo.glyphs[*id].ch == '|')
                    && row.ids[..k].iter().all(|id| {
                        let c = geo.glyphs[*id].ch;
                        c.is_ascii_digit() || matches!(c, '|' | '_' | '^' | '-' | '+' | ':')
                    });
                if !running_head && !gutter {
                    log::debug!("PDF v8: ambiguous columns at y={}", row.y);
                    return None;
                }
            }
        }
        let lists: HashSet<_> = row
            .ids
            .iter()
            .map(|id| owners[*id])
            .filter(|o| protected[*o])
            .collect();
        // A converter may misread `3. SIGN` in a running head as <ol><li>.
        // Unwrap ONLY a short italic fragment sharing the outermost row with
        // other text. A real list (even one item) keeps its markup and offsets.
        let running_list = (row_index == 0 || row_index + 1 == row_count)
            && !lists.is_empty()
            && row.ids.iter().all(|id| !geo.glyphs[*id].mono)
            && row
                .ids
                .iter()
                .any(|id| !protected[owners[*id]] && geo.glyphs[*id].ch.is_ascii_digit())
            && lists.iter().all(|o| {
                let ids: Vec<_> = (0..geo.glyphs.len())
                    .filter(|id| used[*id] && owners[*id] == *o)
                    .collect();
                ids.len() <= 32
                    && ids
                        .iter()
                        .all(|id| row.ids.contains(id) && spans[geo.glyphs[*id].span].is_italic)
            });
        if !lists.is_empty() && !running_list {
            if lists.len() != 1 || row.ids.iter().any(|id| !protected[owners[*id]]) {
                return None;
            }
            row.opaque = lists.iter().copied().next();
        }
        row.code = row.opaque.is_none() && row.ids.iter().all(|id| geo.glyphs[*id].mono);
        row.heading = row.opaque.is_none()
            && !row.code
            && row.ids.iter().any(|id| heading_ids[*id])
            && (body.is_none_or(|s| row.size > s * 1.12)
                || row
                    .ids
                    .iter()
                    .all(|id| spans[geo.glyphs[*id].span].font_weight.is_bold()));
    }
    Some(rows)
}

fn style(body: &mut String, current: &mut (bool, bool), next: (bool, bool)) {
    if *current == next {
        return;
    }
    if current.1 {
        body.push_str("</em>");
    }
    if current.0 {
        body.push_str("</strong>");
    }
    if next.0 {
        body.push_str("<strong>");
    }
    if next.1 {
        body.push_str("<em>");
    }
    *current = next;
}
fn listing_file(row: &Row, geo: &Geometry) -> Option<String> {
    let text: String = row.ids.iter().map(|id| geo.glyphs[*id].ch).collect();
    if !text.starts_with("//") && !text.starts_with('#') {
        return None;
    }
    let path = text.trim_start_matches(['/', '#', '!']);
    (path
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.'))
        && [
            ".rs", ".toml", ".py", ".json", ".yaml", ".yml", ".js", ".ts", ".sh",
        ]
        .iter()
        .any(|ext| path.ends_with(ext)))
    .then(|| path.to_string())
}
fn gap(geo: &Geometry, a: usize, b: usize) -> f32 {
    let (a, b) = (&geo.glyphs[a], &geo.glyphs[b]);
    b.rect.x - (a.rect.x + a.rect.width)
}
fn prose_join(a: &Row, b: &Row, geo: &Geometry, owners: &[usize]) -> bool {
    if a.heading || b.heading || a.code || b.code || a.opaque.is_some() || b.opaque.is_some() {
        return false;
    }
    let lead = &geo.glyphs[b.ids[0]];
    if lead.ch.is_ascii_digit() && lead.size < b.size * 0.8 {
        return false;
    }
    let first = &geo.glyphs[b.ids[0]];
    let left = geo.glyphs[a.ids[0]].rect.x;
    let dy = a.y - b.y;
    if dy < a.size * 0.7
        || dy > a.size * 1.5
        || (a.size - b.size).abs() > a.size * 0.15
        // A new first-line indent is a paragraph boundary. A hanging indent
        // returning LEFT to the margin (e.g. footnotes) is still a soft wrap.
        || first.rect.x - left > a.size * 0.6
        || (first.rect.x - left).abs() > a.size * 2.1
    {
        return false;
    }
    // Preserve intentional paragraph boundaries unless they share an original
    // paragraph, or the previous physical line is clearly a wrapped full line.
    a.ids
        .iter()
        .any(|id| b.ids.iter().any(|next| owners[*id] == owners[*next]))
        || (a.ids.len() > 55
            && geo.glyphs[*a.ids.last().unwrap()].rect.x - first.rect.x > a.size * 28.0)
}

// extract_spans() can merge a word into an earlier run while leaving its
// char_x_offsets shifted (e.g. `write a function` then interleaves letters).
// Match the complete span glyph stream against original TextChar origins;
// glyph identity stays bijective, but positions/advances come from the PDF.
fn original_positions(
    geo: &mut Geometry,
    chars: &[TextChar],
    links: &[(Rect, String)],
) -> Option<()> {
    let mut raw = Geometry::new(&[], &[]);
    for c in chars {
        let expanded = match c.char {
            'ﬀ' => "ff".into(),
            'ﬁ' => "fi".into(),
            'ﬂ' => "fl".into(),
            'ﬃ' => "ffi".into(),
            'ﬄ' => "ffl".into(),
            'ﬅ' | 'ﬆ' => "st".into(),
            ch => ch.to_string(),
        };
        for ch in expanded.chars().filter(|c| !c.is_whitespace()) {
            raw.glyphs.push(content::Glyph {
                ch,
                rect: Rect::new(
                    c.origin_x,
                    c.origin_y,
                    c.advance_width.max(0.01),
                    c.font_size,
                ),
                mono: false,
                size: c.font_size,
                span: 0,
                link: None,
            });
        }
    }
    let text: String = geo.glyphs.iter().map(|g| g.ch).collect();
    let ids = locate(&[text], &raw)?;
    for (g, id) in geo.glyphs.iter_mut().zip(&ids[0]) {
        let original = &raw.glyphs[(*id)?];
        g.rect = original.rect;
        g.size = original.size;
        let x = g.rect.x + g.rect.width * 0.5;
        let y = g.rect.y + g.size * 0.3;
        g.link = links
            .iter()
            .position(|(r, _)| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height);
    }
    Some(())
}

#[cfg(test)]
pub(super) fn project(html: &str, spans: &[TextSpan], links: &[(Rect, String)]) -> Option<Repair> {
    project_with_chars(html, spans, links, &[])
}
fn project_with_chars(
    html: &str,
    spans: &[TextSpan],
    links: &[(pdf_oxide::geometry::Rect, String)],
    chars: &[TextChar],
) -> Option<Repair> {
    // Keep image/table/explicit-break projections. Lists no longer disable an
    // entire page: empty wrappers have no text, real lists remain opaque, and
    // a geometrically proven running-head pseudo-list may be unwrapped.
    if ["<img", "<br", "<table"]
        .iter()
        .any(|tag| html.contains(tag))
    {
        return None;
    }
    let blocks = split_blocks(html);
    let source: Vec<_> = blocks.iter().map(|b| rendered_text(b)).collect();
    let protected: Vec<_> = blocks
        .iter()
        .map(|b| b.contains("<ul") || b.contains("<ol") || b.contains("<li"))
        .collect();
    let mut geo = Geometry::new(spans, links);
    if !chars.is_empty() {
        original_positions(&mut geo, chars, links)?;
    }
    let locations = locate(&source, &geo).or_else(|| {
        log::debug!("PDF v8: unmatched source glyphs");
        None
    })?;
    // Keep confirmed source word boundaries even when narrow/justified spaces
    // fall below the geometric gap threshold. Apply only to actual neighbours
    // in a restored prose row, never across relocated fragments or code cells.
    let mut spaces = HashSet::new();
    for (text, ids) in source.iter().zip(&locations) {
        let (mut previous, mut whitespace) = (None, false);
        for (ch, id) in text.chars().zip(ids) {
            if ch.is_whitespace() {
                whitespace = true;
            }
            if let Some(id) = id {
                if whitespace {
                    if let Some(previous) = previous {
                        spaces.insert((previous, *id));
                    }
                }
                previous = Some(*id);
                whitespace = false;
            }
        }
    }
    let mut used = vec![false; geo.glyphs.len()];
    let mut owners = vec![usize::MAX; geo.glyphs.len()];
    let mut heading_ids = vec![false; geo.glyphs.len()];
    for (block, ids) in locations.iter().enumerate() {
        for id in ids.iter().flatten() {
            if used[*id] {
                return None;
            }
            used[*id] = true;
            owners[*id] = block;
            heading_ids[*id] = blocks[block].starts_with("<h");
        }
    }
    let rows = rows(&geo, spans, &used, &heading_ids, &owners, &protected).or_else(|| {
        log::debug!("PDF v8: unsupported row structure");
        None
    })?;
    if !rows
        .iter()
        .any(|r| r.code || (r.heading && r.ids.iter().any(|i| owners[*i] != owners[r.ids[0]])))
    {
        return None;
    }
    let mut pitches: Vec<f32> = rows
        .windows(2)
        .filter(|r| r[0].code && r[1].code)
        .map(|r| (r[0].y - r[1].y, r[0].size))
        .filter(|(dy, size)| *dy > size * 0.8 && *dy < size * 2.4)
        .map(|(dy, _)| dy)
        .collect();
    pitches.sort_by(f32::total_cmp);
    let pitch = pitches.get(pitches.len() / 2).copied();
    let margin = rows
        .iter()
        .flat_map(|r| &r.ids)
        .map(|id| geo.glyphs[*id].rect.x)
        .fold(f32::INFINITY, f32::min);
    let mut output = Vec::new();
    let mut destinations = vec![None; geo.glyphs.len()];
    let mut opaque_dest = vec![None; source.len()];
    let mut source_byte = vec![0; geo.glyphs.len()];
    for (text, ids) in source.iter().zip(&locations) {
        for ((byte, _), id) in text.char_indices().zip(ids) {
            if let Some(id) = id {
                source_byte[*id] = byte;
            }
        }
    }
    let mut i = 0;
    while i < rows.len() {
        let row = &rows[i];
        let mut end = i + 1;
        if let Some(block) = row.opaque {
            if opaque_dest[block].is_some() {
                return None;
            }
            opaque_dest[block] = Some(output.len());
            while end < rows.len() && rows[end].opaque == Some(block) {
                end += 1;
            }
            for r in &rows[i..end] {
                for id in &r.ids {
                    destinations[*id] = Some((output.len(), source_byte[*id]));
                }
            }
            output.push(blocks[block].clone());
            i = end;
            continue;
        }
        if row.code {
            while end < rows.len()
                && rows[end].code
                // An explicit source-file comment starts a different listing,
                // even if no prose separates two files on the printed page.
                && listing_file(&rows[end],&geo).is_none()
                && rows[end - 1].y - rows[end].y <= pitch.unwrap_or(row.size * 1.7) * 4.1
                && (geo.glyphs[rows[end].ids[0]].rect.x - geo.glyphs[row.ids[0]].rect.x).abs()
                    < row.size * 32.0
            {
                end += 1;
            }
        } else if !row.heading {
            while end < rows.len() && prose_join(&rows[end - 1], &rows[end], &geo, &owners) {
                end += 1;
            }
        }
        let left = rows[i..end]
            .iter()
            .flat_map(|r| r.ids.iter())
            .map(|id| geo.glyphs[*id].rect.x)
            .fold(f32::INFINITY, f32::min);
        let left = if row.code && left - margin >= row.size * 0.9 && left - margin < row.size * 8.0
        {
            margin
        } else {
            left
        };
        let mut body = String::new();
        let mut text = String::new();
        let mut emphasis = (false, false);
        let mut widths: Vec<_> = rows[i..end]
            .iter()
            .flat_map(|r| &r.ids)
            .map(|id| &geo.glyphs[*id])
            .filter(|g| g.rect.width > g.size * 0.3)
            .map(|g| g.rect.width)
            .collect();
        widths.sort_by(f32::total_cmp);
        let cell = widths
            .get(widths.len() / 2)
            .copied()
            .unwrap_or(row.size * 0.6);
        for (line, r) in rows[i..end].iter().enumerate() {
            if line > 0 {
                let spaces = if row.code {
                    "\n"
                } else if text.ends_with('-') {
                    ""
                } else {
                    " "
                };
                body.push_str(spaces);
                text.push_str(spaces);
                if row.code {
                    let blanks = ((rows[i + line - 1].y - r.y) / pitch.unwrap_or(row.size * 1.7))
                        .round()
                        .clamp(1.0, 4.0) as usize
                        - 1;
                    body.extend(std::iter::repeat_n('\n', blanks));
                    text.extend(std::iter::repeat_n('\n', blanks));
                }
            }
            if row.code {
                let indent = ((geo.glyphs[r.ids[0]].rect.x - left) / cell)
                    .round()
                    .clamp(0.0, 80.0) as usize;
                body.extend(std::iter::repeat_n(' ', indent));
                text.extend(std::iter::repeat_n(' ', indent));
            }
            for (k, id) in r.ids.iter().enumerate() {
                let g = &geo.glyphs[*id];
                if !row.code {
                    let font = &spans[g.span];
                    style(
                        &mut body,
                        &mut emphasis,
                        (font.font_weight.is_bold() && !row.heading, font.is_italic),
                    );
                }
                let footnote_gap = k > 0 && k <= 2 && !row.code && {
                    let prev = &geo.glyphs[r.ids[k - 1]];
                    prev.ch.is_ascii_digit() && prev.size < r.size * 0.8
                };
                let word_space = k > 0 && !row.code && spaces.contains(&(r.ids[k - 1], *id));
                if k > 0
                    && (gap(&geo, r.ids[k - 1], *id) > g.size * 0.14 || footnote_gap || word_space)
                {
                    let n = if row.code {
                        (gap(&geo, r.ids[k - 1], *id) / cell)
                            .round()
                            .clamp(1.0, 160.0) as usize
                    } else {
                        1
                    };
                    body.extend(std::iter::repeat_n(' ', n));
                    text.extend(std::iter::repeat_n(' ', n));
                }
                destinations[*id] = Some((output.len(), text.len()));
                body.push_str(&content::escape(&g.ch.to_string()));
                text.push(g.ch);
            }
        }
        style(&mut body, &mut emphasis, (false, false));
        if row.code {
            output.push(format!("<pre><code>{body}</code></pre>"));
        } else {
            // Reapply source font/URI styles after restoring reading order.
            let g: Vec<_> = rows[i..end].iter().flat_map(|r| &r.ids).copied().collect();
            let mut next = g.into_iter();
            let glyphs = text
                .chars()
                .map(|c| if c.is_whitespace() { None } else { next.next() })
                .collect();
            let level = row
                .ids
                .iter()
                .filter_map(|id| {
                    let b = &blocks[owners[*id]];
                    b.strip_prefix("<h")
                        .and_then(|s| s.chars().next())
                        .filter(|c| matches!(c, '1'..='6'))
                })
                .min()
                .unwrap_or('3');
            let heading = format!("h{level}");
            let tag = if row.heading { heading.as_str() } else { "p" };
            let block = content::Block {
                html: format!("<{tag}>{body}</{tag}>"),
                text,
                glyphs,
                code: false,
            };
            output.push(content::inline(&block, &geo));
        }
        i = end;
    }
    #[cfg(test)]
    let mut points = Vec::new();
    let mut maps = Vec::new();
    let mut previous = (0, 0);
    for (block, (text, ids)) in source.iter().zip(&locations).enumerate() {
        let mut map: Vec<Point> = vec![(0, 0); text.len() + 1];
        let mut anchor = ids
            .iter()
            .flatten()
            .find_map(|id| destinations[*id])
            .unwrap_or(previous);
        for ((byte, ch), id) in text.char_indices().zip(ids) {
            if let Some(id) = id {
                anchor = destinations[*id]?;
            }
            for k in 0..ch.len_utf8() {
                map[byte + k] = (anchor.0, anchor.1 + if ch.is_whitespace() { 0 } else { k });
            }
            map[byte + ch.len_utf8()] = (
                anchor.0,
                anchor.1 + if ch.is_whitespace() { 0 } else { ch.len_utf8() },
            );
        }
        if let Some(target) = opaque_dest[block] {
            for (byte, point) in map.iter_mut().enumerate() {
                *point = (target, byte);
            }
        } else if text.is_empty() {
            map[0] = anchor;
        }
        previous = *map.last()?;
        if map.iter().any(|(_, byte)| *byte >= POINT_PACK) {
            return None;
        }
        maps.push(BlockTarget {
            block: map[0].0,
            prefix: map[0].1,
            old_len: text.len(),
            offsets: Some(
                map.iter()
                    .map(|(block, byte)| block * POINT_PACK + byte)
                    .collect(),
            ),
            layout_changed: true,
        });
        #[cfg(test)]
        points.push(map);
    }
    let fixed = output.join("\n");
    (fixed != html).then_some(Repair {
        html: fixed,
        maps,
        source,
        #[cfg(test)]
        points,
    })
}

pub(super) fn repair(doc: &PdfDocument, page: usize, html: &str) -> Result<Option<Repair>, String> {
    let spans = doc
        .extract_spans(page)
        .map_err(|e| format!("读取 PDF 代码行失败: {e}"))?;
    if !spans.iter().any(content::monospace) {
        return Ok(None);
    }
    let chars = doc
        .extract_chars(page)
        .map_err(|e| format!("读取 PDF 原始字形失败: {e}"))?;
    let links = content::links(doc, page);
    let links = if links.is_empty() {
        links
    } else {
        super::structure::refine_links(&links, &chars)
    };
    Ok(project_with_chars(html, &spans, &links, &chars))
}

#[cfg(test)]
#[path = "reflow_v8_tests.rs"]
mod tests;
