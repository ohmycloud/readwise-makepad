//! Shared offline Typst/PDF visual projection over immutable source text.
//! Do not infer TeX from flattened words or rewrite native/Web source offsets.
mod cache;
mod content;
mod figures;
mod pdf;
mod scattered;
#[cfg(test)]
mod session_tests;
mod typeset;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug)]
struct Glyph {
    ch: char,
    paint: char,
    id: usize,
    source: bool,
    roman: bool,
    x: f32,
    y: f32,
    size: f32,
    width: f32,
    math: bool,
    italic: bool,
    bold: bool,
}
#[derive(Clone, Debug)]
struct Rule {
    x: f32,
    y: f32,
    width: f32,
    dy: f32,
    thickness: f32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MathPage {
    fragments: HashMap<usize, Vec<Fragment>>,
    pub bytes: usize,
}
#[derive(Debug, Serialize, Deserialize)]
struct Fragment {
    start: usize,
    end: usize,
    html: String,
    projection: Projection,
}
impl Fragment {
    fn bytes(&self) -> usize {
        self.html.len()
            + match &self.projection {
                Projection::Formula(f) => f.svg.len(),
                Projection::Figure(f) => f.png.len(),
                Projection::Hidden { owner } => owner.len(),
            }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Formula {
    pub id: String,
    pub svg: String,
    pub width_em: f64,
    pub height_em: f64,
    /// CSS-style displacement of the canvas bottom relative to the text baseline.
    pub align_em: f64,
    pub display: bool,
}
/// Original PDF graphics rendered off-thread. Never replaces persisted text.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Figure {
    pub id: String,
    #[serde(with = "cache::png")]
    pub png: std::sync::Arc<Vec<u8>>,
    pub width: u16,
    pub height: u16,
}
#[derive(Debug, Serialize, Deserialize)]
pub enum Projection {
    Formula(Formula),
    Figure(Figure),
    Hidden { owner: String },
}

/// One immutable book's visual-projection session. Keep it for the lifetime of
/// the open book, NOT one page: pdf_oxide caches embedded fonts, CMaps and page
/// geometry on PdfDocument. Reopening per page discards all of those caches.
/// Creation is cheap; opening/parsing occurs on the caller's background worker.
/// Do not reuse a session for a different source file or book revision.
pub struct PdfSession {
    path: std::path::PathBuf,
    document: std::sync::OnceLock<Option<pdf_oxide::PdfDocument>>,
    cache: Option<cache::PageCache>,
}
impl PdfSession {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            path: path.into(),
            document: std::sync::OnceLock::new(),
            cache: None,
        }
    }
    /// Opt-in disposable presentation cache. The caller owns this PRIVATE
    /// directory; never pass the user's source directory as the cache root.
    pub fn with_cache(
        path: impl Into<std::path::PathBuf>,
        directory: impl Into<std::path::PathBuf>,
    ) -> Self {
        let path = path.into();
        let cache = cache::PageCache::new(&path, directory.into());
        Self {
            path,
            document: Default::default(),
            cache,
        }
    }
    /// Keep immutable presentation-cache identity, but not a parsed PDF/font
    /// document in a dormant frontend reader. Does not read/write the source.
    pub fn without_document(&self) -> Self {
        Self {
            path: self.path.clone(),
            document: Default::default(),
            cache: self.cache.clone(),
        }
    }
    /// Cheap lookup before a frontend's compilation lock. A hit must not open
    /// the PDF, load fonts, run Typst or wait behind speculative neighbor work.
    pub fn cached_page(&self, page: usize, blocks: &[String], text: &[String]) -> Option<MathPage> {
        let cache = self.cache.as_ref()?;
        cache.read(page, blocks, text)
    }
    pub fn page(&self, page: usize, blocks: &[String], text: &[String]) -> MathPage {
        if !self
            .path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
            || text.iter().map(String::len).sum::<usize>() > 512 * 1024
        {
            return MathPage::empty();
        }
        if let Some(cached) = self.cached_page(page, blocks, text) {
            return cached;
        }
        // Cache document-open failures in memory, not as permanent empty
        // presentations. Successful empty (ordinary prose) pages are reusable.
        let projected = self
            .document
            .get_or_init(|| pdf_oxide::PdfDocument::open(&self.path).ok())
            .as_ref()
            .and_then(|doc| pdf::project(doc, &self.path, page, blocks, text));
        if let (Some(cache), Some(projected)) = (&self.cache, &projected) {
            let _ = cache.write(page, blocks, text, projected);
        }
        projected.unwrap_or_else(MathPage::empty)
    }
}

impl MathPage {
    fn empty() -> Self {
        Self {
            fragments: HashMap::new(),
            bytes: 0,
        }
    }
    /// One-shot convenience. Readers must retain a PdfSession across pages.
    pub fn pdf(path: &std::path::Path, page: usize, blocks: &[String], text: &[String]) -> Self {
        PdfSession::new(path).page(page, blocks, text)
    }
    pub fn annotate(
        &self,
        block: usize,
        html: &str,
        annotate: impl FnOnce(&str) -> String,
    ) -> String {
        self.project_with(block, html, annotate, |_, _, web| web.to_owned())
    }
    pub fn project_with(
        &self,
        block: usize,
        html: &str,
        annotate: impl FnOnce(&str) -> String,
        render: impl Fn(&Projection, &str, &str) -> String,
    ) -> String {
        let Some(fragments) = self.fragments.get(&block) else {
            return annotate(html);
        };
        let (text, positions) = text_positions(html);
        // Only complete diagram-label blocks lose their display wrappers.
        // Captions, headings, mixed blocks and ordinary formulas keep theirs.
        if let [fragment] = fragments.as_slice()
            && fragment.start == 0
            && fragment.end == text.len()
            && match &fragment.projection {
                Projection::Figure(_) => true,
                Projection::Hidden { owner } => owner.starts_with("pdf-figure-"),
                Projection::Formula(_) => false,
            }
        {
            return render(&fragment.projection, &text, &fragment.html);
        }
        let mut protected = String::new();
        let mut cursor = 0;
        for (index, fragment) in fragments.iter().enumerate() {
            let Some((start, end)) = html_range(html, &positions, fragment.start, fragment.end)
            else {
                continue;
            };
            if start < cursor {
                continue;
            }
            protected.push_str(&html[cursor..start]);
            protected.push_str(&format!(
                "<code class=\"rw-math-{index}\">{}</code>",
                content::escape(&text[fragment.start..fragment.end])
            ));
            cursor = end;
        }
        protected.push_str(&html[cursor..]);
        if content::plain_text(&protected) != text {
            return annotate(html);
        }
        let mut result = annotate(&protected);
        for (index, fragment) in fragments.iter().enumerate() {
            let open = format!("<code class=\"rw-math-{index}\">");
            let Some(start) = result.find(&open) else {
                continue;
            };
            let Some(end) = result[start..].find("</code>") else {
                continue;
            };
            let rendered = render(
                &fragment.projection,
                &text[fragment.start..fragment.end],
                &fragment.html,
            );
            result.replace_range(start..start + end + 7, &rendered);
        }
        result
    }
}

#[derive(Debug)]
struct Position {
    text: usize,
    start: usize,
    end: usize,
}
fn text_positions(html: &str) -> (String, Vec<Position>) {
    let mut text = String::new();
    let mut positions = Vec::new();
    let mut at = 0;
    while at < html.len() {
        if html.as_bytes()[at] == b'<' {
            let Some(end) = html[at..].find('>') else {
                break;
            };
            at += end + 1;
            continue;
        }
        let start = at;
        let mut value = html[at..].chars().next().unwrap().to_string();
        at += value.len();
        if value == "&"
            && let Some(end) = html[start..].find(';').filter(|n| *n <= 32)
        {
            let entity = &html[start..start + end + 1];
            let decoded = html_escape::decode_html_entities(entity);
            if decoded != entity {
                value = decoded.into_owned();
                at = start + end + 1;
            }
        }
        for ch in value.chars() {
            positions.push(Position {
                text: text.len(),
                start,
                end: at,
            });
            text.push(ch);
        }
    }
    (text, positions)
}
fn html_range(
    html: &str,
    positions: &[Position],
    start: usize,
    end: usize,
) -> Option<(usize, usize)> {
    let begin = positions.iter().find(|p| p.text == start)?.start;
    let mut finish = positions.iter().rev().find(|p| p.text < end)?.end;
    // Never cut across a link/emphasis/code container. Balanced markup wholly
    // inside a math range can safely become literal protected source text.
    let mut stack = Vec::new();
    let mut rest = &html[begin..finish];
    while let Some(open) = rest.find('<') {
        let close = rest[open..].find('>')? + open;
        let tag = rest[open + 1..close].trim();
        let name = tag
            .trim_start_matches('/')
            .split_ascii_whitespace()
            .next()?
            .trim_end_matches('/');
        if !["br", "img", "hr", "wbr"].contains(&name) {
            if tag.starts_with('/') {
                if stack.pop() != Some(name) {
                    return None;
                }
            } else if !tag.ends_with('/') {
                stack.push(name);
            }
        }
        rest = &rest[close + 1..];
    }
    while let Some(name) = stack.pop() {
        let close = format!("</{name}>");
        if !html[finish..].starts_with(&close) {
            return None;
        }
        finish += close.len();
    }
    Some((begin, finish))
}

fn signature(text: impl Iterator<Item = char>) -> Vec<char> {
    let mut chars: Vec<_> = text.filter(|c| !c.is_whitespace()).collect();
    chars.sort_unstable();
    chars
}

#[derive(Clone, Copy, Debug)]
struct Placement {
    display: bool,
    em: f32,
    baseline: f32,
}
impl Placement {
    fn resolve(glyphs: &[Glyph], page: &[Glyph], display: bool) -> Self {
        let em = glyphs.iter().map(|g| g.size).fold(0f32, f32::max);
        let baseline = glyphs
            .iter()
            .filter(|g| (g.size - em).abs() < 0.1)
            .map(|g| g.y)
            .fold(f32::INFINITY, f32::min);
        let fallback = Self {
            display,
            em,
            baseline,
        };
        if display {
            return fallback;
        }
        let left = glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min);
        let right = glyphs
            .iter()
            .map(|g| g.x + g.width)
            .fold(f32::NEG_INFINITY, f32::max);
        let center = (glyphs.iter().map(|g| g.y).fold(f32::INFINITY, f32::min)
            + glyphs.iter().map(|g| g.y).fold(f32::NEG_INFINITY, f32::max))
            * 0.5;
        // Numerators/denominators are script-size in the PDF. They must NOT be
        // enlarged to the surrounding reader's full em, nor treated as its
        // baseline. Resolve both from adjacent prose on the original text line.
        let ids: std::collections::HashSet<_> = glyphs.iter().map(|g| g.id).collect();
        let prose = page
            .iter()
            .filter(|g| !g.math && !ids.contains(&g.id))
            .filter_map(|g| {
                let gap = (left - g.x - g.width).max(g.x - right).max(0.);
                let dy = (g.y - center).abs();
                (gap <= g.size * 4. && dy <= g.size * 1.5).then_some((gap + dy * 4., g))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        prose.map_or(fallback, |(_, g)| Self {
            display,
            em: g.size,
            baseline: g.y,
        })
    }
}

fn fragment(
    mut glyphs: Vec<Glyph>,
    mut rules: Vec<Rule>,
    source: &str,
    start: usize,
    end: usize,
    prefix: &str,
    placement: Placement,
) -> Option<Fragment> {
    if glyphs.len() < 2 || glyphs.len() > 512 || source.len() > 8192 {
        return None;
    }
    let base = placement.em;
    let display = placement.display;
    // PDF symbol fonts often put the radical's baseline at its TOP bar,
    // unlike Unicode text fonts. Rebuild only the radical stroke from its
    // measured overbar and enclosed glyphs; do not shift or guess its operand.
    let mut radicals = Vec::new();
    for g in glyphs.iter().filter(|g| g.ch == '√') {
        let Some(bar) = rules.iter().find(|r| {
            r.dy == 0.0
                && (r.x - g.x - g.width).abs() < g.size * 0.35
                && (r.y - g.y).abs() < g.size * 0.3
        }) else {
            continue;
        };
        let bottom = glyphs
            .iter()
            .filter(|child| {
                child.x >= bar.x - 0.1
                    && child.x < bar.x + bar.width
                    && child.y < bar.y
                    && child.y > bar.y - g.size * 1.5
            })
            .map(|child| child.y - child.size * 0.18)
            .min_by(f32::total_cmp)
            .unwrap_or(bar.y - g.size * 0.9);
        let height = bar.y - bottom;
        let points = [
            (g.x, bar.y - height * 0.55),
            (g.x + g.width * 0.22, bar.y - height * 0.4),
            (g.x + g.width * 0.46, bottom),
            (bar.x, bar.y),
        ];
        for pair in points.windows(2) {
            radicals.push(Rule {
                x: pair[0].0,
                y: pair[0].1,
                width: pair[1].0 - pair[0].0,
                dy: pair[1].1 - pair[0].1,
                thickness: bar.thickness,
            });
        }
    }
    rules.extend(radicals);
    // Remove only large empty horizontal gutters (not mathematical spacing),
    // e.g. a page-edge equation number, so narrow readers need less scrolling.
    let mut intervals: Vec<_> = glyphs.iter().map(|g| (g.x, g.x + g.width)).collect();
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut edge = intervals[0].1;
    let mut gaps = Vec::new();
    for (left, right) in intervals {
        if left - edge > base * 3. {
            gaps.push((left, left - edge - base * 1.2));
        }
        edge = edge.max(right);
    }
    let shift = |x: f32| {
        x - gaps
            .iter()
            .filter(|(left, _)| x >= *left - 0.01)
            .map(|(_, gap)| gap)
            .sum::<f32>()
    };
    for g in &mut glyphs {
        g.x = shift(g.x);
    }
    for r in &mut rules {
        r.x = shift(r.x);
    }
    let left = glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min) - base * 0.12;
    let top = glyphs
        .iter()
        .filter(|g| {
            !(g.paint == '√' && rules.iter().any(|r| r.dy != 0. && (r.x - g.x).abs() < 0.01))
        })
        .map(|g| g.y + g.size * 0.95)
        .fold(f32::NEG_INFINITY, f32::max);
    let bottom = glyphs
        .iter()
        .map(|g| g.y - g.size * 0.35)
        .fold(f32::INFINITY, f32::min);
    let right = glyphs
        .iter()
        .map(|g| g.x + g.width)
        .fold(f32::NEG_INFINITY, f32::max)
        .max(
            rules
                .iter()
                .map(|r| r.x + r.width)
                .fold(f32::NEG_INFINITY, f32::max),
        )
        + base * 0.2;
    let mut width = right - left;
    let mut height = top - bottom;
    let mut baseline_down = top - placement.baseline;
    for g in &mut glyphs {
        g.x -= left;
        g.y = top - g.y;
    }
    for r in &mut rules {
        r.x -= left;
        r.y = top - r.y;
        r.dy = -r.dy;
    }
    let rendered = typeset::svg(&glyphs, &rules, width, height)?;
    let mut svg = rendered.xml;
    if !display && let Some([x, y, right, bottom]) = rendered.ink {
        // Trim only unused font-metric padding, never the actual glyphs/rules.
        // The compiler's ink bounds include radical and fraction stroke widths.
        let pad = base * 0.04;
        let x = x - pad;
        let y = y - pad;
        width = right + pad - x;
        height = bottom + pad - y;
        baseline_down -= y;
        for g in &mut glyphs {
            g.x -= x;
            g.y -= y;
        }
        let at = svg.find("viewBox=\"")? + 9;
        let end = at + svg[at..].find('"')?;
        svg.replace_range(at..end, &format!("{x:.5} {y:.5} {width:.5} {height:.5}"));
        // Native SVG logical size must have the same aspect as the cropped
        // viewBox too; browser width/height CSS alone cannot provide that.
        for (name, value) in [("width", width), ("height", height)] {
            let at = svg.find(&format!("{name}=\""))? + name.len() + 2;
            let end = at + svg[at..].find('"')?;
            svg.replace_range(at..end, &format!("{value:.5}pt"));
        }
    }
    // Typst emits glyph outlines, not browser text or publisher-controlled SVG.
    // Namespace reusable glyph IDs so neighboring formulas never collide.
    let svg = svg
        .replace("id=\"", &format!("id=\"{prefix}-"))
        .replace("href=\"#", &format!("href=\"#{prefix}-"))
        .replace("url(#", &format!("url(#{prefix}-"))
        .replacen("<svg ", "<svg aria-hidden=\"true\" focusable=\"false\" ", 1);
    let mut layer = String::new();
    let mut used = vec![false; glyphs.len()];
    // DOM text stays byte-for-byte identical to the existing source, even when
    // its PDF extraction order differs from visual order (fractions/scripts).
    for ch in source.chars() {
        if let Some((i, g)) = glyphs
            .iter()
            .enumerate()
            .find(|(i, g)| g.source && !used[*i] && g.ch == ch)
        {
            used[i] = true;
            // Percent positions use the canvas, not the smaller script's em.
            // Otherwise superscript/subscript selection rectangles drift.
            layer.push_str(&format!(
                "<span style=\"left:{:.5}%;top:{:.5}%;width:{:.5}%;font-size:{:.5}em\">{}</span>",
                g.x / width * 100.,
                (g.y - g.size * 0.8) / height * 100.,
                g.width / width * 100.,
                g.size / base,
                content::escape(&ch.to_string())
            ));
        } else if ch.is_whitespace() {
            layer.push(ch);
        } else {
            return None;
        }
    }
    if used.iter().zip(&glyphs).any(|(used, g)| g.source && !used) {
        return None;
    }
    let symbols: String = glyphs
        .iter()
        .filter(|g| g.paint != g.ch)
        .map(|g| g.paint)
        .collect();
    let symbols = content::escape(&symbols);
    let kind = if display { " math-display" } else { "" };
    let html = format!(
        "<span class=\"typst-math{kind}\" data-math-renderer=\"typst\" data-math-id=\"{prefix}\" data-math-symbols=\"{symbols}\" role=\"math\" tabindex=\"0\" style=\"width:{:.5}em;vertical-align:{:.5}em\"><span class=\"math-canvas\" style=\"width:{:.5}em;height:{:.5}em\">{svg}<span class=\"math-source\">{layer}</span></span></span>",
        width / base,
        (baseline_down - height) / base,
        width / base,
        height / base
    );
    Some(Fragment {
        start,
        end,
        html,
        projection: Projection::Formula(Formula {
            id: prefix.into(),
            svg,
            width_em: (width / base) as f64,
            height_em: (height / base) as f64,
            align_em: ((baseline_down - height) / base) as f64,
            display,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inline_fraction_uses_prose_em_and_baseline_and_trims_only_ink_padding() {
        let glyph = |ch, id, x, y, size, math: bool| Glyph {
            ch,
            paint: ch,
            id,
            source: true,
            roman: !math,
            x,
            y,
            size,
            width: size * 0.5,
            math,
            italic: math,
            bold: false,
        };
        let formula = vec![
            glyph('1', 0, 100., 94., 8., false),
            glyph('d', 1, 100., 86., 8., true),
            glyph('k', 2, 105., 84.5, 6., true),
        ];
        // Also supports CMR prose; roman is not exclusively a math font.
        let mut page = formula.clone();
        page.push(glyph('e', 3, 91., 90., 12., false));
        page.push(glyph('x', 4, 91., 110., 12., false));
        let placement = Placement::resolve(&formula, &page, false);
        assert_eq!(placement.em, 12.);
        assert_eq!(placement.baseline, 90.);
        let display = Placement::resolve(&formula, &page, true);
        assert_eq!(display.em, 8.);
        assert_eq!(display.baseline, 86.);
        assert_eq!(Placement::resolve(&formula, &[], false).em, 8.);
        let f = fragment(
            formula,
            vec![Rule {
                x: 99.,
                y: 91.,
                width: 12.,
                dy: 0.,
                thickness: 0.4,
            }],
            "1dk",
            0,
            3,
            "spacing",
            placement,
        )
        .unwrap();
        assert_eq!(content::plain_text(&f.html), "1dk");
        let canvas = &f.html[f.html.find("class=\"math-canvas\"").unwrap()..];
        let height: f32 = canvas
            .split("height:")
            .nth(1)
            .unwrap()
            .split("em")
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            height < 1.4,
            "fraction should fit ordinary leading, got {height}em"
        );
        assert!(f.html.contains("font-size:0.66667em"));
        assert!(!f.html.contains("viewBox=\"0 0 "));
    }
    #[test]
    fn radical_uses_original_overbar_geometry_and_keeps_literal_utf8_source() {
        let glyphs = vec![
            Glyph {
                ch: '√',
                paint: '√',
                id: 0,
                source: true,
                roman: false,
                x: 0.,
                y: 12.,
                size: 10.,
                width: 8.,
                math: true,
                italic: false,
                bold: false,
            },
            Glyph {
                ch: 'd',
                paint: 'd',
                id: 1,
                source: true,
                roman: false,
                x: 8.,
                y: 5.,
                size: 10.,
                width: 5.,
                math: true,
                italic: true,
                bold: false,
            },
        ];
        let formula = fragment(
            glyphs,
            vec![Rule {
                x: 8.,
                y: 12.2,
                width: 8.,
                dy: 0.,
                thickness: 0.4,
            }],
            "√d",
            0,
            "√d".len(),
            "test-root",
            Placement {
                display: false,
                em: 10.,
                baseline: 5.,
            },
        )
        .unwrap();
        assert_eq!(content::plain_text(&formula.html), "√d");
        assert!(formula.html.matches("stroke=\"currentColor\"").count() >= 4);
        assert!(!formula.html.contains("<text"));
    }

    #[test]
    fn literal_ranges_preserve_entities_utf8_and_balanced_markup() {
        let html = "<p>中文 x&lt;y &amp; <em>z</em> end</p>";
        let (text, map) = text_positions(html);
        assert_eq!(text, content::plain_text(html));
        let start = text.find('x').unwrap();
        let end = text.find(" end").unwrap();
        assert!(html_range(html, &map, start, end).is_some());
        let crossing = text.find('z').unwrap();
        assert!(html_range(html, &map, crossing, text.len()).is_none());
    }
}
