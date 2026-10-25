//! Some old PDF snapshots put scripts/operators in a later extraction row.
//! Bind through a UNIQUE complete paragraph in the original span stream, not
//! guessed variable names. Paint at the base, retain displaced source bytes in
//! zero-width proxies in their original DOM positions (no coordinate migration).
use super::{Fragment, Glyph, MathPage, Rule, content, fragment, pdf::rules_for};
use pdf_oxide::PdfDocument;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug)]
pub(super) struct Location {
    block: usize,
    start: usize,
    end: usize,
}
fn expand(ch: char) -> String {
    match ch {
        'ﬀ' => "ff".into(),
        'ﬁ' => "fi".into(),
        'ﬂ' => "fl".into(),
        'ﬃ' => "ffi".into(),
        'ﬄ' => "ffl".into(),
        c => c.to_string(),
    }
}
pub(super) fn locations(
    doc: &PdfDocument,
    page: usize,
    glyphs: &[Glyph],
    texts: &[String],
) -> Vec<Option<Location>> {
    let mut locations = vec![None; glyphs.len()];
    if glyphs.len() > 8192 {
        return locations;
    }
    let Ok(spans) = doc.extract_spans(page) else {
        return locations;
    };
    if spans.len() > 5000 {
        return locations;
    }
    let mut stream = String::new();
    let mut references = Vec::new();
    let mut used = HashSet::new();
    for span in spans {
        if span.text.len() > 65536 || stream.len() + span.text.len() > 512 * 1024 {
            return vec![None; glyphs.len()];
        }
        let candidates: Vec<_> = glyphs
            .iter()
            .filter(|g| {
                g.x >= span.bbox.x - 0.5
                    && g.x <= span.bbox.x + span.bbox.width + 0.5
                    && (g.y - span.bbox.y).abs() < span.font_size * 0.8
            })
            .collect();
        let mut cursor = span.bbox.x;
        for ch in span
            .text
            .chars()
            .flat_map(|ch| expand(ch).chars().collect::<Vec<_>>())
            .filter(|c| !c.is_whitespace())
        {
            let best = candidates
                .iter()
                .filter(|g| {
                    g.ch == ch
                        && !used.contains(&g.id)
                        && (g.x - cursor).abs() < span.font_size * 1.3
                })
                .min_by(|a, b| {
                    ((a.x - cursor).abs() + (a.y - span.bbox.y).abs() * 0.25)
                        .total_cmp(&((b.x - cursor).abs() + (b.y - span.bbox.y).abs() * 0.25))
                });
            let id = best.map(|g| {
                used.insert(g.id);
                cursor = g.x + g.width;
                g.id
            });
            if id.is_none() {
                cursor += span.font_size * 0.5;
            }
            references.push((stream.len(), id));
            stream.push(ch);
        }
    }
    let mut counts = HashMap::<String, usize>::new();
    for text in texts {
        *counts
            .entry(text.chars().filter(|c| !c.is_whitespace()).collect())
            .or_default() += 1;
    }
    for (block, text) in texts.iter().enumerate() {
        let chars: Vec<_> = text
            .char_indices()
            .filter(|(_, ch)| !ch.is_whitespace())
            .collect();
        let wanted: String = chars.iter().map(|(_, ch)| ch).collect();
        if wanted.len() < 2 || counts.get(&wanted) != Some(&1) {
            continue;
        }
        let Some(offset) = stream.find(&wanted) else {
            continue;
        };
        if stream[offset + wanted.chars().next().unwrap().len_utf8()..].contains(&wanted) {
            continue;
        }
        let Ok(index) = references.binary_search_by_key(&offset, |(at, _)| *at) else {
            continue;
        };
        let ids: Option<Vec<_>> = references[index..]
            .iter()
            .take(chars.len())
            .map(|(_, id)| *id)
            .collect();
        let Some(ids) = ids else {
            continue;
        };
        if ids.len() != chars.len() || ids.iter().any(|id| locations[*id].is_some()) {
            continue;
        }
        for ((start, ch), id) in chars.into_iter().zip(ids) {
            locations[id] = Some(Location {
                block,
                start,
                end: start + ch.len_utf8(),
            });
        }
    }
    locations
}
pub(super) fn exclude_block(locations: &mut [Option<Location>], block: usize) {
    for loc in locations {
        if loc.is_some_and(|l| l.block == block) {
            *loc = None;
        }
    }
}
fn root(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}
fn join(parent: &mut [usize], a: usize, b: usize) {
    let a = root(parent, a);
    let b = root(parent, b);
    parent[a] = b;
}
fn clusters(glyphs: &[Glyph], rules: &[Rule]) -> Vec<Vec<Glyph>> {
    let math: Vec<_> = glyphs.iter().filter(|g| g.math).collect();
    if math.len() > 2048 {
        return Vec::new();
    }
    let selected: Vec<_> = glyphs
        .iter()
        .filter(|g| {
            g.math
                || math.iter().any(|m| {
                    (g.size < m.size * 0.85 || (g.ch.is_ascii_digit() && g.size <= m.size * 1.05))
                        && (g.y - m.y).abs() < m.size
                        && g.x >= m.x - m.size * 0.2
                        && g.x < m.x + m.width + m.size * 2.
                })
        })
        .cloned()
        .collect();
    if selected.len() > 2048 {
        return Vec::new();
    }
    let mut parent: Vec<_> = (0..selected.len()).collect();
    // Attach scripts to their larger, immediately preceding base. Nested scripts
    // prefer the intermediate base, not a different nearby body-text line.
    for (i, g) in selected.iter().enumerate() {
        let best = selected
            .iter()
            .enumerate()
            .filter(|(j, b)| {
                *j != i
                    && b.size > g.size * 1.15
                    && g.x >= b.x - b.size * 0.1
                    && g.x - b.x - b.width < b.size * 0.65
                    && (g.y - b.y).abs() < b.size * 0.95
            })
            .min_by(|(_, a), (_, b)| {
                let score = |p: &Glyph| (g.x - p.x - p.width).abs() + 0.6 * (g.y - p.y).abs();
                score(a).total_cmp(&score(b))
            });
        if let Some((j, _)) = best {
            join(&mut parent, i, j);
        }
    }
    for (i, g) in selected.iter().enumerate() {
        for (j, h) in selected.iter().enumerate().skip(i + 1) {
            // A common baseline joins operators and operands, never distant
            // scripts that happen to lie near the NEXT visual body line.
            if (g.size - h.size).abs() < g.size * 0.12
                && (g.y - h.y).abs() < g.size * 0.23
                && (g.x - h.x - h.width).max(h.x - g.x - g.width) < g.size * 0.8
            {
                join(&mut parent, i, j);
            }
        }
        if g.paint == '∑' {
            for (j, h) in selected.iter().enumerate() {
                if h.size >= g.size * 0.85
                    && (h.y - g.y).abs() < g.size * 0.25
                    && h.x > g.x
                    && h.x - g.x - g.width < g.size * 2.
                {
                    join(&mut parent, i, j);
                }
            }
        }
        if g.paint == '√'
            && let Some(bar) = rules.iter().find(|r| {
                (r.x - g.x - g.width).abs() < g.size * 0.35 && (r.y - g.y).abs() < g.size * 0.3
            })
        {
            for (j, h) in selected.iter().enumerate() {
                if h.x >= bar.x - 0.1
                    && h.x < bar.x + bar.width
                    && h.y < bar.y
                    && h.y > bar.y - g.size * 1.5
                {
                    join(&mut parent, i, j);
                }
            }
        }
    }
    // A fraction bar, unlike a common text baseline, explicitly binds the
    // numerator and denominator. This also captures small roman numerals.
    for rule in rules {
        let above: Vec<_> = selected
            .iter()
            .enumerate()
            .filter(|(_, g)| {
                g.x >= rule.x - 0.1
                    && g.x < rule.x + rule.width
                    && g.y > rule.y
                    && g.y - rule.y < g.size * 1.5
                    && rule.width < g.size * 5.
            })
            .map(|(i, _)| i)
            .collect();
        let below: Vec<_> = selected
            .iter()
            .enumerate()
            .filter(|(_, g)| {
                g.x >= rule.x - 0.1
                    && g.x < rule.x + rule.width
                    && g.y < rule.y
                    && rule.y - g.y < g.size * 1.8
                    && rule.width < g.size * 5.
            })
            .map(|(i, _)| i)
            .collect();
        if let (Some(a), Some(_)) = (above.first(), below.first()) {
            for b in above.iter().chain(&below) {
                join(&mut parent, *a, *b);
            }
        }
    }
    let mut groups = HashMap::<usize, Vec<Glyph>>::new();
    for (i, g) in selected.into_iter().enumerate() {
        groups.entry(root(&mut parent, i)).or_default().push(g);
    }
    let mut groups: Vec<_> = groups.into_values().filter(|g| g.len() >= 2).collect();
    // Deterministic names/cache/rendering, independent of HashMap random seed.
    groups.sort_by(|a, b| {
        a.iter()
            .map(|g| g.id)
            .min()
            .cmp(&b.iter().map(|g| g.id).min())
    });
    groups
}

pub(super) fn project(
    glyphs: &[Glyph],
    rules: &[Rule],
    locations: &[Option<Location>],
    blocks: &[String],
    texts: &[String],
    page: usize,
    result: &mut MathPage,
) -> usize {
    let mut used = 0;
    for (index, cluster) in clusters(glyphs, rules).into_iter().enumerate() {
        let Some(locs) = cluster
            .iter()
            .map(|g| locations[g.id])
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let block = locs[0].block;
        if locs.iter().any(|l| l.block != block)
            || ["<pre", "<code", "<table", "<a "]
                .iter()
                .any(|tag| blocks[block].contains(tag))
        {
            continue;
        }
        if locs.iter().any(|l| {
            result
                .fragments
                .get(&block)
                .is_some_and(|fs| fs.iter().any(|f| l.start < f.end && l.end > f.start))
        }) {
            continue;
        }
        // Must carry real script/operator geometry, not ordinary italic words.
        let max = cluster.iter().map(|g| g.size).fold(0., f32::max);
        if !cluster
            .iter()
            .any(|g| g.size < max * 0.85 || matches!(g.paint, '√' | '∑' | '∫'))
        {
            continue;
        }
        let mut ordered: Vec<_> = locs.iter().enumerate().collect();
        ordered.sort_by_key(|(_, l)| l.start);
        let mut ranges = Vec::<(usize, usize, Vec<usize>)>::new();
        for (i, loc) in ordered {
            if let Some(last) = ranges
                .last_mut()
                .filter(|r| texts[block][r.1..loc.start].trim().is_empty())
            {
                last.1 = loc.end;
                last.2.push(i);
            } else {
                ranges.push((loc.start, loc.end, vec![i]));
            }
        }
        let (_, positions) = super::text_positions(&blocks[block]);
        if ranges.iter().any(|(start, end, _)| {
            super::html_range(&blocks[block], &positions, *start, *end).is_none()
        }) {
            continue;
        }
        let anchor = ranges
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                let weight = |r: &(usize, usize, Vec<usize>)| {
                    r.2.iter().map(|i| cluster[*i].size).sum::<f32>()
                };
                weight(a).total_cmp(&weight(b))
            })
            .map(|(i, _)| i)
            .unwrap();
        let (start, end, ids) = &ranges[anchor];
        let mut visual = cluster.clone();
        for (i, g) in visual.iter_mut().enumerate() {
            g.source = ids.contains(&i);
        }
        let prefix = format!("math-{page}-scattered-{index}");
        let display =
            texts[block][..*start].trim().is_empty() && texts[block][*end..].trim().is_empty();
        let placement = super::Placement::resolve(&visual, glyphs, display);
        let Some(visual) = fragment(
            visual,
            rules_for(&cluster, rules),
            &texts[block][*start..*end],
            *start,
            *end,
            &prefix,
            placement,
        ) else {
            continue;
        };
        let mut additions = vec![visual];
        for (i, (start, end, _)) in ranges.into_iter().enumerate() {
            if i != anchor {
                additions.push(Fragment {
                    start,
                    end,
                    projection: super::Projection::Hidden {
                        owner: prefix.clone(),
                    },
                    html: format!(
                        "<span class=\"math-remote-source\" data-math-owner=\"{prefix}\">{}</span>",
                        content::escape(&texts[block][start..end])
                    ),
                });
            }
        }
        let bytes = additions.iter().map(Fragment::bytes).sum::<usize>();
        if result.bytes + bytes > 8 * 1024 * 1024
            || used + cluster.len() > 2048
            || result
                .fragments
                .values()
                .flatten()
                .filter(|f| f.html.contains("data-math-renderer="))
                .count()
                >= 64
            || result.fragments.values().map(Vec::len).sum::<usize>() + additions.len() > 128
        {
            continue;
        }
        used += cluster.len();
        result.bytes += bytes;
        result.fragments.entry(block).or_default().extend(additions);
    }
    used
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scattered_scripts_and_cmex_keep_every_source_byte_and_reject_duplicate_paragraphs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scattered.pdf");
        let bytes = include_bytes!("../../web/tests/fixtures/math-scattered.pdf");
        std::fs::write(&path, bytes).unwrap();
        let loaded = readwise_core::books::pdf::load_pages(&path).unwrap();
        let blocks = readwise_core::books::epub_meta::split_blocks(&loaded.chapters[0].html);
        let texts: Vec<_> = blocks.iter().map(|b| content::plain_text(b)).collect();
        let page = MathPage::pdf(&path, 0, &blocks, &texts);
        let projected: Vec<_> = blocks
            .iter()
            .enumerate()
            .map(|(i, b)| page.annotate(i, b, str::to_owned))
            .collect();
        for (html, source) in projected.iter().zip(&texts) {
            assert_eq!(content::plain_text(html), *source);
        }
        let html = projected.join("");
        assert!(html.contains("data-math-symbols=\"∑\""), "{:#?}", texts);
        assert!(
            html.contains("math-remote-source"),
            "fixture must really exercise dislocated extraction rows"
        );
        assert!(html.matches("data-math-renderer=\"typst\"").count() >= 3);
        assert!(
            texts.iter().any(|t| t.contains('P')),
            "source P must not be rewritten to Unicode sum"
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let doc = PdfDocument::open(&path).unwrap();
        let glyphs = super::super::pdf::glyphs(&doc, 0).unwrap();
        assert!(
            html.matches("data-math-renderer=\"typst\"").count() >= 4,
            "small-font inline fraction must be projected too"
        );
        let text = texts.iter().find(|t| t.contains("Keys")).unwrap();
        assert!(
            locations(&doc, 0, &glyphs, &[text.clone(), text.clone()])
                .iter()
                .all(Option::is_none)
        );
    }
}
