//! Geometric PDF projection, not an equation-name/flat-text replacement table.
//! A formula is accepted only when its original glyph multiset matches ONE
//! unmodified source range. Ambiguous/unsupported content stays readable text.
use super::{Fragment, Glyph, MathPage, Rule, fragment, html_range, signature, text_positions};
use pdf_oxide::PdfDocument;
use std::{collections::HashMap, path::Path};

fn math_font(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    ["cmmi", "cmsy", "cmex", "msam", "msbm", "math", "stix"]
        .iter()
        .any(|v| name.contains(v))
}
// CMEX's standard TeX slots are not Latin letters. Keep the decoded source
// character for immutable anchors; correct only its painted glyph.
fn legacy_symbol(font: &str, ch: char) -> char {
    let name = font.rsplit('+').next().unwrap_or(font).to_ascii_lowercase();
    if name
        .strip_prefix("cmex")
        .is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        && matches!(ch, 'P' | 'X')
    {
        '∑'
    } else {
        ch
    }
}
pub(super) fn glyphs(doc: &PdfDocument, page: usize) -> Option<Vec<Glyph>> {
    let chars = doc.extract_chars(page).ok()?;
    if chars.len() > 50_000 {
        return None;
    }
    let mut result = Vec::new();
    for c in chars {
        if c.char.is_whitespace()
            || c.rotation_degrees.abs() > 0.1
            || c.char.is_control()
            || ![c.origin_x, c.origin_y, c.font_size, c.advance_width]
                .iter()
                .all(|v| v.is_finite())
            || !(1.0..=100.0).contains(&c.font_size)
        {
            continue;
        }
        let text = match c.char {
            'ﬀ' => "ff".into(),
            'ﬁ' => "fi".into(),
            'ﬂ' => "fl".into(),
            'ﬃ' => "ffi".into(),
            'ﬄ' => "ffl".into(),
            c => c.to_string(),
        };
        let width = c.advance_width.max(c.font_size * 0.1) / text.chars().count() as f32;
        for (i, ch) in text.chars().enumerate() {
            let math = math_font(&c.font_name)
                || matches!(
                    ch,
                    '=' | '∈'
                        | '∉'
                        | '×'
                        | '÷'
                        | '√'
                        | '∑'
                        | '∏'
                        | '∫'
                        | '≤'
                        | '≥'
                        | '≈'
                        | '±'
                        | '∞'
                        | '−'
                        | '·'
                );
            result.push(Glyph {
                ch,
                paint: legacy_symbol(&c.font_name, ch),
                id: result.len(),
                source: true,
                roman: c
                    .font_name
                    .rsplit('+')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .strip_prefix("cmr")
                    .is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())),
                x: c.origin_x + i as f32 * width,
                y: c.origin_y,
                size: c.font_size,
                width,
                math,
                italic: c.is_italic || c.font_name.to_ascii_lowercase().contains("cmmi"),
                bold: c.font_weight.is_bold(),
            });
        }
    }
    Some(result)
}

fn display_candidate(row: &Band) -> bool {
    // A one-line prose paragraph containing a symbol is not a display equation.
    // CMR is roman mathematical text here; do not turn it into an inline seed.
    row.glyphs.iter().filter(|g| g.math || g.roman).count() * 2 >= row.glyphs.len()
}
struct Band {
    glyphs: Vec<Glyph>,
    baseline: f32,
    size: f32,
}
fn bands(glyphs: &[Glyph]) -> Vec<Band> {
    let mut sizes = HashMap::<i32, usize>::new();
    for g in glyphs {
        *sizes.entry((g.size * 10.).round() as i32).or_default() += 1;
    }
    let body = sizes
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map_or(10., |(s, _)| s as f32 / 10.);
    let mut large: Vec<_> = glyphs
        .iter()
        .filter(|g| g.size >= body * 0.85)
        .cloned()
        .collect();
    large.sort_by(|a, b| b.y.total_cmp(&a.y).then(a.x.total_cmp(&b.x)));
    let mut bands: Vec<Band> = Vec::new();
    for g in large {
        if let Some(row) = bands
            .iter_mut()
            .find(|r| (r.baseline - g.y).abs() < g.size.max(r.size) * 0.22)
        {
            row.size = row.size.max(g.size);
            row.glyphs.push(g);
        } else {
            bands.push(Band {
                baseline: g.y,
                size: g.size,
                glyphs: vec![g],
            });
        }
    }
    for g in glyphs.iter().filter(|g| g.size < body * 0.85) {
        let nearest = bands
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                (r.baseline - g.y).abs() < r.size * 0.95
                    && r.glyphs
                        .iter()
                        .any(|other| (other.x - g.x).abs() < r.size * 2.)
            })
            .min_by(|(_, a), (_, b)| {
                (a.baseline - g.y)
                    .abs()
                    .total_cmp(&(b.baseline - g.y).abs())
            })
            .map(|(i, _)| i);
        if let Some(i) = nearest {
            bands[i].glyphs.push(g.clone());
        }
    }
    // Fractions have several full-size baselines less than one em apart.
    // Merge those only where both rows carry mathematical evidence.
    let mut merged: Vec<Band> = Vec::new();
    for band in bands {
        if let Some(previous) = merged.last_mut().filter(|r| {
            (r.baseline - band.baseline).abs() < r.size.max(band.size) * 0.9
                && r.glyphs.iter().any(|g| g.math)
                && band.glyphs.iter().any(|g| g.math)
        }) {
            previous.baseline = band.baseline;
            previous.glyphs.extend(band.glyphs);
        } else {
            merged.push(band);
        }
    }
    merged
}

fn matches(
    texts: &[String],
    glyphs: &[Glyph],
    existing: &HashMap<usize, Vec<Fragment>>,
) -> Vec<(usize, usize, usize)> {
    let wanted = signature(glyphs.iter().map(|g| g.ch));
    if wanted.len() < 2 || wanted.len() > 512 {
        return Vec::new();
    }
    let mut matches = Vec::new();
    for (block, text) in texts.iter().enumerate() {
        if text.len() > 64 * 1024 {
            continue;
        }
        let chars: Vec<_> = text
            .char_indices()
            .filter(|(_, c)| !c.is_whitespace())
            .collect();
        if chars.len() < wanted.len() {
            continue;
        }
        let mut counts = HashMap::<char, i32>::new();
        for ch in &wanted {
            *counts.entry(*ch).or_default() -= 1;
        }
        let change = |counts: &mut HashMap<char, i32>, ch: char, delta| {
            let value = counts.entry(ch).or_default();
            *value += delta;
            if *value == 0 {
                counts.remove(&ch);
            }
        };
        for (i, (_, ch)) in chars.iter().enumerate() {
            change(&mut counts, *ch, 1);
            if i >= wanted.len() {
                change(&mut counts, chars[i - wanted.len()].1, -1);
            }
            if i + 1 >= wanted.len() && counts.is_empty() {
                let start = chars[i + 1 - wanted.len()].0;
                let end = chars[i].0 + ch.len_utf8();
                if !existing
                    .get(&block)
                    .is_some_and(|ranges| ranges.iter().any(|r| start < r.end && end > r.start))
                {
                    matches.push((block, start, end));
                }
            }
        }
    }
    matches
}

fn cuts_word(text: &str, start: usize, end: usize) -> bool {
    (text[start..]
        .chars()
        .next()
        .is_some_and(char::is_alphanumeric)
        && text[..start]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric))
        || (text[..end]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric)
            && text[end..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric))
}

pub(super) fn project(
    path: &Path,
    page: usize,
    blocks: &[String],
    texts: &[String],
) -> Option<MathPage> {
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
    {
        return None;
    }
    if texts.iter().map(String::len).sum::<usize>() > 512 * 1024 {
        return None;
    }
    let doc = PdfDocument::open(path).ok()?;
    let mut glyphs = glyphs(&doc, page)?;
    let mut locations = super::scattered::locations(&doc, page, &glyphs, texts);
    for g in &mut glyphs {
        if g.paint != g.ch {
            g.y -= g.size * 0.8;
        }
    }
    let rules: Vec<Rule> = doc
        .extract_paths(page)
        .ok()?
        .iter()
        .filter(|p| {
            p.bbox.height.abs() < 0.05
                && p.bbox.width > 0.
                && p.stroke_color.is_some()
                && p.stroke_width > 0.
                && p.stroke_width <= 2.
        })
        .map(|p| Rule {
            x: p.bbox.x,
            y: p.bbox.y,
            width: p.bbox.width,
            dy: 0.,
            thickness: p.stroke_width,
        })
        .collect();
    let rows = bands(&glyphs);
    // Reserve whole display rows for the existing complete-equation renderer.
    // Repair proven scattered scripts BEFORE permissive inline multiset runs
    // can accidentally consume an unrelated trailing script with the same name.
    for row in &rows {
        if !display_candidate(row) {
            continue;
        }
        let found = matches(texts, &row.glyphs, &HashMap::new());
        if let [(block, start, end)] = found.as_slice()
            && texts[*block][..*start].trim().is_empty()
            && texts[*block][*end..].trim().is_empty()
        {
            super::scattered::exclude_block(&mut locations, *block);
        }
    }
    let mut initial = MathPage {
        fragments: HashMap::new(),
        bytes: 0,
    };
    let mut glyph_budget = super::scattered::project(
        &glyphs,
        &rules,
        &locations,
        blocks,
        texts,
        page,
        &mut initial,
    );
    let mut fragments = initial.fragments;
    let mut bytes = initial.bytes;
    let mut count = fragments
        .values()
        .flatten()
        .filter(|f| f.html.contains("data-math-renderer="))
        .count();
    let page_glyphs = &glyphs;
    let mut add = |glyphs: Vec<Glyph>,
                   display_only: bool,
                   fragments: &mut HashMap<usize, Vec<Fragment>>|
     -> bool {
        if count >= 64 || glyph_budget + glyphs.len() > 2048 || bytes > 4 * 1024 * 1024 {
            return false;
        }
        let found = matches(texts, &glyphs, fragments);
        if found.len() != 1 {
            return false;
        }
        let (block, start, end) = found[0];
        // A short atom must not consume the `d v` inside "and values" after
        // its real owner was used. Keep display-row reservation unchanged:
        // incomplete PDF row slices can end inside legitimate source math.
        if glyphs.len() <= 4 && cuts_word(&texts[block], start, end) {
            return false;
        }
        if blocks[block].contains("<pre")
            || blocks[block].contains("<code")
            || blocks[block].contains("<table")
            || blocks[block].contains("<a ")
        {
            return false;
        }
        let display =
            texts[block][..start].trim().is_empty() && texts[block][end..].trim().is_empty();
        if display_only && !display {
            return false;
        }
        let (_, positions) = text_positions(&blocks[block]);
        if html_range(&blocks[block], &positions, start, end).is_none() {
            return false;
        }
        let formula_rules = rules_for(&glyphs, &rules);
        let n = glyphs.len();
        let placement = super::Placement::resolve(&glyphs, page_glyphs, display);
        let Some(fragment) = fragment(
            glyphs,
            formula_rules,
            &texts[block][start..end],
            start,
            end,
            &format!("math-{page}-{count}"),
            placement,
        ) else {
            return false;
        };
        if bytes.saturating_add(fragment.html.len()) > 8 * 1024 * 1024 {
            return false;
        }
        bytes += fragment.bytes();
        glyph_budget += n;
        fragments.entry(block).or_default().push(fragment);
        count += 1;
        true
    };
    for row in rows {
        if !row.glyphs.iter().any(|g| g.math) {
            continue;
        }
        if display_candidate(&row) && add(row.glyphs.clone(), true, &mut fragments) {
            continue;
        }
        // Inline runs: mathematical fonts/operators plus adjacent scripts.
        // Plain body words, hyperlinks, code and tables are never made formulas.
        let mut ordered = row.glyphs.clone();
        ordered.sort_by(|a, b| a.x.total_cmp(&b.x).then(b.y.total_cmp(&a.y)));
        let mut flags: Vec<_> = ordered
            .iter()
            .map(|g| {
                g.math || g.size < row.size * 0.85 && (g.y - row.baseline).abs() > row.size * 0.1
            })
            .collect();
        for i in 0..ordered.len() {
            if !flags[i]
                && (ordered[i].ch.is_ascii_digit() || "()[],._".contains(ordered[i].ch))
                && ((i > 0 && flags[i - 1]) || (i + 1 < flags.len() && flags[i + 1]))
            {
                flags[i] = true;
            }
        }
        let mut run: Vec<Glyph> = Vec::new();
        for (g, yes) in ordered.into_iter().zip(flags).chain(std::iter::once((
            Glyph {
                ch: ' ',
                paint: ' ',
                id: usize::MAX,
                source: true,
                roman: false,
                x: 0.,
                y: 0.,
                size: 1.,
                width: 0.,
                math: false,
                italic: false,
                bold: false,
            },
            false,
        ))) {
            let separated = run
                .last()
                .is_some_and(|prev| g.x - (prev.x + prev.width) > row.size * 0.9);
            if !yes || separated {
                if run.len() >= 2
                    && run.iter().any(|g| g.math)
                    && run
                        .iter()
                        .any(|g| g.size < row.size * 0.85 || matches!(g.ch, '√' | '∑' | '∫'))
                {
                    let run = std::mem::take(&mut run);
                    if !add(run.clone(), false, &mut fragments) {
                        // A PDF converter may split one visual equation between
                        // non-adjacent source blocks. Do not join/reorder them:
                        // independently match each base-with-scripts atom.
                        let mut atom = Vec::<Glyph>::new();
                        let mut flush = |atom: &mut Vec<Glyph>| {
                            if atom.len() >= 2 && atom.iter().any(|g| g.size < row.size * 0.85) {
                                add(std::mem::take(atom), false, &mut fragments);
                            }
                            atom.clear();
                        };
                        for glyph in run {
                            if glyph.size >= row.size * 0.85 {
                                flush(&mut atom);
                            }
                            atom.push(glyph);
                        }
                        flush(&mut atom);
                    }
                }
                run.clear();
            }
            if yes {
                run.push(g);
            }
        }
    }
    let mut result = MathPage { fragments, bytes };
    for ranges in result.fragments.values_mut() {
        ranges.sort_by_key(|r| r.start);
    }
    Some(result)
}

pub(super) fn rules_for(glyphs: &[Glyph], rules: &[Rule]) -> Vec<Rule> {
    let left = glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min);
    let right = glyphs
        .iter()
        .map(|g| g.x + g.width)
        .fold(f32::NEG_INFINITY, f32::max);
    // Fraction rules deliberately overhang small numerator/denominator ink.
    // A script-sized quarter-em clipped the owned fixture's real PDF bar even
    // though it had already supplied the evidence for that fraction cluster.
    let margin = glyphs.iter().map(|g| g.size).fold(0f32, f32::max) * 0.6;
    let bottom = glyphs
        .iter()
        .map(|g| g.y - g.size * 0.4)
        .fold(f32::INFINITY, f32::min);
    let top = glyphs
        .iter()
        .map(|g| g.y + g.size)
        .fold(f32::NEG_INFINITY, f32::max);
    rules
        .iter()
        .filter(|r| {
            r.x >= left - margin && r.x + r.width <= right + margin && r.y >= bottom && r.y <= top
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cmex_sum_slots_change_only_paint_not_roman_letters() {
        assert_eq!(legacy_symbol("ABCDEF+CMEX9", 'P'), '∑');
        assert_eq!(legacy_symbol("CMEX10", 'X'), '∑');
        for font in ["CMR10", "Times-Roman", "CMEXample"] {
            assert_eq!(legacy_symbol(font, 'P'), 'P');
        }
    }
    #[test]
    fn ambiguous_text_is_not_guessed_and_whitespace_does_not_change_identity() {
        let g = |ch| Glyph {
            ch,
            paint: ch,
            id: 0,
            source: true,
            roman: false,
            x: 0.,
            y: 0.,
            size: 10.,
            width: 5.,
            math: true,
            italic: false,
            bold: false,
        };
        let glyphs = vec![g('x'), g('2')];
        assert_eq!(
            matches(&["x 2".into()], &glyphs, &HashMap::new()),
            [(0, 0, 3)]
        );
        assert_eq!(
            matches(&["x2 and x2".into()], &glyphs, &HashMap::new()).len(),
            2
        );
        assert!(matches(&["x3".into()], &glyphs, &HashMap::new()).is_empty());
        let dimension = vec![g('d'), g('v')];
        let prose = "and values come from the encoder";
        let found = matches(&[prose.into()], &dimension, &HashMap::new());
        let [(block, start, end)] = found.as_slice() else {
            panic!("fixture")
        };
        assert_eq!(*block, 0);
        assert!(cuts_word(prose, *start, *end));
        assert!(!cuts_word("dv is a dimension", 0, 2));
        assert!(cuts_word("prefixx2suffix", 6, 8));
    }
}
