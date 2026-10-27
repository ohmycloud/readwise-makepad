//! Render only a confirmed diagram region, never its surrounding prose.
//! Text outside the crop remains ordinary source HTML (including mixed blocks).
use super::{Figure, Fragment, MathPage, Projection, content, text_positions};
use base64::{Engine, engine::general_purpose::STANDARD};
use hayro::{
    RenderCache, RenderSettings,
    hayro_interpret::InterpreterSettings,
    hayro_syntax::Pdf,
    vello_cpu::{Pixmap, color::palette::css::WHITE},
};
use pdf_oxide::{PdfDocument, layout::TextChar};
use std::{
    collections::HashMap,
    io::Read,
    ops::Range,
    path::Path,
    sync::{Arc, Mutex},
};

const MAX_EDGE: f32 = 1800.;
const MAX_FILE: u64 = 200 * 1024 * 1024;
static RENDER_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug)]
struct Bounds {
    left: f32,
    bottom: f32,
    right: f32,
    top: f32,
}
impl Bounds {
    fn empty() -> Self {
        Self {
            left: f32::INFINITY,
            bottom: f32::INFINITY,
            right: f32::NEG_INFINITY,
            top: f32::NEG_INFINITY,
        }
    }
    fn include(&mut self, b: Self) {
        self.left = self.left.min(b.left);
        self.bottom = self.bottom.min(b.bottom);
        self.right = self.right.max(b.right);
        self.top = self.top.max(b.top);
    }
    fn padded(self, n: f32) -> Self {
        Self {
            left: self.left - n,
            bottom: self.bottom - n,
            right: self.right + n,
            top: self.top + n,
        }
    }
}
#[derive(Debug)]
struct Region {
    bounds: Bounds,
    media: (f32, f32),
}

fn rotated(c: &TextChar) -> bool {
    (c.rotation_degrees.abs() - 90.).abs() < 1.
}
fn figure_region(doc: &PdfDocument, page: usize) -> Option<Region> {
    // Unsupported transforms must not silently fall back to a whole-page image.
    if doc.get_page_rotation(page).ok()? != 0 {
        return None;
    }
    let (x, y, w, h) = doc.get_page_media_box(page).ok()?;
    if x != 0. || y != 0. || !w.is_finite() || !h.is_finite() || w <= 0. || h <= 0. {
        return None;
    }
    // Rotated labels are mandatory evidence. Check the cached character stream
    // first; ordinary prose must not parse vector paths merely to reject them.
    let chars = doc.extract_chars(page).ok()?;
    if chars.len() > 50_000 {
        return None;
    }
    let chars = chars
        .iter()
        .filter(|c| !c.char.is_whitespace())
        .collect::<Vec<_>>();
    let labels = chars.iter().filter(|c| rotated(c)).collect::<Vec<_>>();
    if labels.len() < 24 || labels.len() * 5 < chars.len() {
        return None;
    }
    let paths = doc.extract_paths(page).ok()?;
    if !(32..=20_000).contains(&paths.len()) {
        return None;
    }
    let valid = paths
        .iter()
        .filter(|p| {
            [
                p.bbox.x,
                p.bbox.y,
                p.bbox.width,
                p.bbox.height,
                p.stroke_width,
            ]
            .iter()
            .all(|n| n.is_finite())
                && (p.fill_color.is_some() || p.stroke_color.is_some())
                && (p.bbox.width.abs() > 2. || p.bbox.height.abs() > 2.)
        })
        .collect::<Vec<_>>();
    if valid.len() < 32 {
        return None;
    }
    let mut seed = Bounds::empty();
    for c in labels {
        if ![c.origin_x, c.origin_y, c.font_size]
            .iter()
            .all(|v| v.is_finite())
            || !(1. ..=100.).contains(&c.font_size)
        {
            return None;
        }
        // The extractor reports zero advance/bbox width for some rotated fonts.
        // Use each original origin and em, not a flattened span's fake width.
        seed.include(Bounds {
            left: c.origin_x - c.font_size,
            right: c.origin_x + c.font_size,
            bottom: c.origin_y - c.font_size,
            top: c.origin_y + c.font_size,
        });
    }
    let mut corridor = Bounds {
        left: 0.,
        bottom: 0.,
        right: w,
        top: h,
    };
    for c in chars.iter().filter(|c| !rotated(c)) {
        if c.rotation_degrees.abs() > 0.1
            || ![c.origin_x, c.origin_y, c.advance_width, c.ascent, c.descent]
                .iter()
                .all(|v| v.is_finite())
        {
            return None;
        }
        let b = Bounds {
            left: c.origin_x,
            right: c.origin_x + c.advance_width.max(0.),
            bottom: c.origin_y + c.descent,
            top: c.origin_y + c.ascent,
        };
        // Every horizontal word stays text. Constrain the crop on its nearest
        // side, or reject overlapping/ambiguous mixed layouts altogether.
        if b.bottom > seed.top {
            corridor.top = corridor.top.min(b.bottom - 1.);
        } else if b.top < seed.bottom {
            corridor.bottom = corridor.bottom.max(b.top + 1.);
        } else if b.right < seed.left {
            corridor.left = corridor.left.max(b.right + 1.);
        } else if b.left > seed.right {
            corridor.right = corridor.right.min(b.left - 1.);
        } else {
            return None;
        }
    }
    let mut bounds = seed;
    for p in valid {
        let pad = if p.stroke_color.is_some() {
            p.stroke_width.abs() * 0.5
        } else {
            0.
        };
        bounds.include(
            Bounds {
                left: p.bbox.x,
                bottom: p.bbox.y,
                right: p.bbox.x + p.bbox.width,
                top: p.bbox.y + p.bbox.height,
            }
            .padded(pad),
        );
    }
    let bounds = bounds.padded(2.);
    let bounds = Bounds {
        left: bounds.left.max(corridor.left),
        bottom: bounds.bottom.max(corridor.bottom),
        right: bounds.right.min(corridor.right),
        top: bounds.top.min(corridor.top),
    };
    if bounds.left >= seed.left
        || bounds.right <= seed.right
        || bounds.bottom >= seed.bottom
        || bounds.top <= seed.top
    {
        return None;
    }
    Some(Region {
        bounds,
        media: (w, h),
    })
}

fn expand(c: char) -> String {
    match c {
        'ﬀ' => "ff".into(),
        'ﬁ' => "fi".into(),
        'ﬂ' => "fl".into(),
        'ﬃ' => "ffi".into(),
        'ﬄ' => "ffl".into(),
        c => c.to_string(),
    }
}
fn normalized(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .map(expand)
        .collect()
}

/// Protect every outside span by an unambiguous match in the EXISTING blocks.
/// Order, wrappers and even mixed caption/diagram/footer blocks remain intact.
fn source_ranges(
    doc: &PdfDocument,
    page: usize,
    texts: &[String],
) -> Option<Vec<Vec<Range<usize>>>> {
    let spans = doc.extract_spans(page).ok()?;
    if spans.len() > 5000
        || spans
            .len()
            .saturating_mul(texts.iter().map(String::len).sum())
            > 32 * 1024 * 1024
    {
        return None;
    }
    let sources = texts
        .iter()
        .map(|text| {
            let mut normalized = String::new();
            let mut positions = Vec::new();
            for (start, c) in text.char_indices().filter(|(_, c)| !c.is_whitespace()) {
                for ch in expand(c).chars() {
                    positions.push((normalized.len(), start, start + c.len_utf8()));
                    normalized.push(ch);
                }
            }
            (normalized, positions)
        })
        .collect::<Vec<_>>();
    let mut protected = texts
        .iter()
        .map(|t| vec![false; t.len()])
        .collect::<Vec<_>>();
    for span in spans
        .iter()
        .filter(|s| s.rotation_degrees.abs() < 0.1 && !s.text.trim().is_empty())
    {
        let wanted = normalized(&span.text);
        if wanted.is_empty() {
            continue;
        }
        let mut candidates = Vec::new();
        for (block, (source, positions)) in sources.iter().enumerate() {
            for (at, _) in source.match_indices(&wanted) {
                let end = at + wanted.len();
                let (_, start, _) = positions.iter().find(|p| p.0 == at)?;
                let (_, _, finish) = positions.iter().rev().find(|p| p.0 < end)?;
                // A page number '4' must not match the suffix of 'label4', nor
                // a heading 'Attention' the prefix of 'Attentions' in a caption.
                let text = &texts[block];
                let before = text[..*start].chars().next_back();
                let after = text[*finish..].chars().next();
                if wanted.chars().next()?.is_alphanumeric()
                    && before.is_some_and(char::is_alphanumeric)
                    || wanted.chars().next_back()?.is_alphanumeric()
                        && after.is_some_and(char::is_alphanumeric)
                {
                    continue;
                }
                candidates.push((block, *start, *finish));
            }
        }
        let [(block, start, end)] = candidates.as_slice() else {
            return None;
        };
        protected[*block][*start..*end].fill(true);
    }
    let mut available = HashMap::<char, usize>::new();
    for c in doc
        .extract_chars(page)
        .ok()?
        .iter()
        .filter(|c| rotated(c) && !c.char.is_whitespace())
    {
        for ch in expand(c.char).chars() {
            *available.entry(ch).or_default() += 1;
        }
    }
    let mut ranges = Vec::new();
    for (text, protected) in texts.iter().zip(&protected) {
        let mut parts = Vec::new();
        let mut start = None;
        for (at, c) in text.char_indices() {
            if protected[at] {
                if let Some(start) = start.take()
                    && !text[start..at].trim().is_empty()
                {
                    parts.push(start..at);
                }
            } else {
                start.get_or_insert(at);
                if !c.is_whitespace() {
                    for ch in expand(c).chars() {
                        let n = available.get_mut(&ch)?;
                        *n = n.checked_sub(1)?;
                    }
                }
            }
        }
        if let Some(start) = start
            && !text[start..].trim().is_empty()
        {
            parts.push(start..text.len());
        }
        ranges.push(parts);
    }
    Some(ranges)
}

fn raster(path: &Path, page: usize, region: &Region) -> Option<(Vec<u8>, u16, u16)> {
    let _guard = RENDER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_FILE {
        return None;
    }
    std::panic::catch_unwind(|| {
        let pdf = Pdf::new(bytes).ok()?;
        let page = pdf.pages().get(page)?;
        let crop = page.intersected_crop_box();
        let (w, h) = page.render_dimensions();
        if crop.x0 != 0.
            || crop.y0 != 0.
            || (w - region.media.0).abs() > 0.1
            || (h - region.media.1).abs() > 0.1
        {
            return None;
        }
        let scale = (MAX_EDGE / w.max(h)).min(3.);
        let settings = RenderSettings {
            x_scale: scale,
            y_scale: scale,
            width: Some((w * scale).ceil().clamp(1., MAX_EDGE) as u16),
            height: Some((h * scale).ceil().clamp(1., MAX_EDGE) as u16),
            bg_color: WHITE,
        };
        let pixels = hayro::render(
            page,
            &RenderCache::new(),
            &InterpreterSettings::default(),
            &settings,
        );
        let width = pixels.width() as usize;
        let height = pixels.height() as usize;
        let b = region.bounds;
        let left = (b.left * scale).ceil().clamp(0., width as f32) as usize;
        let right = (b.right * scale).floor().clamp(0., width as f32) as usize;
        let top = ((h - b.top) * scale).ceil().clamp(0., height as f32) as usize;
        let bottom = ((h - b.bottom) * scale).floor().clamp(0., height as f32) as usize;
        if right <= left + 2 || bottom <= top + 2 {
            return None;
        }
        let data = pixels.data_as_u8_slice().as_chunks::<4>().0;
        let mut x0 = right;
        let mut y0 = bottom;
        let mut x1 = left;
        let mut y1 = top;
        for y in top..bottom {
            for x in left..right {
                if data[y * width + x][..3].iter().any(|c| *c < 248) {
                    // No visible ink may be sliced at a prose/figure boundary.
                    if x == left || x == right - 1 || y == top || y == bottom - 1 {
                        return None;
                    }
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x + 1);
                    y1 = y1.max(y + 1);
                }
            }
        }
        if x0 >= x1 || y0 >= y1 {
            return None;
        }
        x0 = x0.saturating_sub(3).max(left);
        y0 = y0.saturating_sub(3).max(top);
        x1 = (x1 + 3).min(right);
        y1 = (y1 + 3).min(bottom);
        let (w, h) = ((x1 - x0) as u16, (y1 - y0) as u16);
        let mut cropped = Pixmap::new(w, h);
        for row in 0..h as usize {
            cropped.data_mut()[row * w as usize..(row + 1) * w as usize]
                .copy_from_slice(&pixels.data()[(y0 + row) * width + x0..(y0 + row) * width + x1]);
        }
        let png = cropped.into_png().ok()?;
        (png.len() <= 4 * 1024 * 1024).then_some((png, w, h))
    })
    .ok()
    .flatten()
}

pub(super) fn project(
    doc: &PdfDocument,
    path: &Path,
    page: usize,
    blocks: &[String],
    texts: &[String],
) -> Option<MathPage> {
    let region = figure_region(doc, page)?;
    if blocks.len() != texts.len()
        || blocks.len() > 1024
        || blocks
            .iter()
            .zip(texts)
            .any(|(b, t)| text_positions(b).0 != *t)
    {
        return None;
    }
    let ranges = source_ranges(doc, page, texts)?;
    // Prefer an entire label block after the (still textual) heading. Never
    // anchor the image to a caption simply because the old extractor put it first.
    let heading = blocks
        .iter()
        .enumerate()
        .rfind(|(i, b)| b.trim_start().starts_with("<h") && ranges[*i].is_empty())
        .map(|(i, _)| i);
    let full = |i: usize| ranges[i].len() == 1 && ranges[i][0] == (0..texts[i].len());
    let anchor = (0..texts.len())
        .find(|&i| full(i) && heading.is_none_or(|h| i > h))
        .or_else(|| (0..texts.len()).find(|&i| full(i)))
        .or_else(|| ranges.iter().position(|r| !r.is_empty()))?;
    let (png, width, height) = raster(path, page, &region)?;
    let figure = Figure {
        id: format!("pdf-figure-{page}"),
        png: Arc::new(png),
        width,
        height,
    };
    let image = format!(
        "data:image/png;base64,{}",
        STANDARD.encode(figure.png.as_slice())
    );
    let mut fragments = HashMap::new();
    for (block, parts) in ranges.into_iter().enumerate() {
        let mut list = Vec::new();
        for (part, range) in parts.into_iter().enumerate() {
            let source = content::escape(&texts[block][range.clone()]);
            let (projection, html) = if block == anchor && part == 0 {
                (
                    Projection::Figure(figure.clone()),
                    format!(
                        "<span class=\"typst-math math-display pdf-region-figure\" data-math-renderer=\"pdf-graphics\" data-math-id=\"{}\" role=\"img\" aria-label=\"PDF figure on page {}\" tabindex=\"0\" style=\"width:100%\"><span class=\"math-canvas\" style=\"aspect-ratio:{width}/{height}\"><img src=\"{image}\" alt=\"\"/><span class=\"math-source\">{source}</span></span></span>",
                        figure.id,
                        page + 1
                    ),
                )
            } else {
                (
                    Projection::Hidden {
                        owner: figure.id.clone(),
                    },
                    format!(
                        "<span class=\"math-remote-source\" data-math-owner=\"{}\">{source}</span>",
                        figure.id
                    ),
                )
            };
            list.push(Fragment {
                start: range.start,
                end: range.end,
                html,
                projection,
            });
        }
        if !list.is_empty() {
            fragments.insert(block, list);
        }
    }
    let bytes = fragments.values().flatten().map(Fragment::bytes).sum();
    Some(MathPage { fragments, bytes })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/tests/fixtures/vector-figure.pdf")
    }
    #[test]
    fn diagram_region_keeps_prose_and_every_original_source_block_and_byte() {
        let path = fixture();
        let before = std::fs::read(&path).unwrap();
        let loaded = readwise_core::books::pdf::load_pages(&path).unwrap();
        let blocks = readwise_core::books::epub_meta::split_blocks(&loaded.chapters[0].html);
        let texts = blocks
            .iter()
            .map(|s| readwise_core::vocab::annotate::rendered_text(s))
            .collect::<Vec<_>>();
        let page = MathPage::pdf(&path, 0, &blocks, &texts);
        let mut images = 0;
        let mut prose = String::new();
        for (i, block) in blocks.iter().enumerate() {
            let html = page.annotate(i, block, |s| {
                readwise_core::vocab::annotate::annotate_html(
                    s,
                    &[
                        ("diagram".into(), "图".into()),
                        ("figure".into(), "图示".into()),
                        ("forest".into(), "森林".into()),
                    ]
                    .into_iter()
                    .collect(),
                    &Default::default(),
                )
            });
            assert_eq!(
                readwise_core::vocab::annotate::rendered_text(&html),
                texts[i]
            );
            if texts[i].contains("Diagram overview")
                || texts[i].contains("Figure 1:")
                || texts[i].contains("Ordinary forest")
            {
                assert!(
                    html.contains("<ruby"),
                    "outside text must remain annotatable: {html}"
                );
                prose.push_str(&html);
            }
            images += html.matches("data:image/png;base64,").count();
        }
        assert_eq!(images, 1);
        assert!(page.bytes < 8 * 1024 * 1024);
        assert!(
            prose.contains("<h1>")
                && readwise_core::vocab::annotate::rendered_text(&prose).contains("Figure 1:"),
            "prose: {prose}"
        );
        let figure = page
            .fragments
            .values()
            .flatten()
            .find_map(|f| {
                if let Projection::Figure(f) = &f.projection {
                    Some(f)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(&figure.png[..8], b"\x89PNG\r\n\x1a\n");
        assert!(
            figure.width > 100
                && figure.height > 100
                && figure.width <= 1800
                && figure.height <= 1800
        );
        let doc = PdfDocument::open(&path).unwrap();
        let region = figure_region(&doc, 0).unwrap();
        assert!(
            region.bounds.top < 280. && region.bounds.bottom > 35.,
            "title and caption must be outside the bitmap: {region:?}"
        );
        for (block, parts) in &page.fragments {
            for part in parts {
                let text = &texts[*block][part.start..part.end];
                assert!(
                    !text.contains("Diagram overview")
                        && !text.contains("Figure 1:")
                        && !text.contains("Ordinary forest")
                );
            }
        }
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    #[test]
    fn detached_numeric_footer_stays_text_and_outside_the_diagram() {
        let path = fixture();
        let doc = PdfDocument::open(&path).unwrap();
        let region = figure_region(&doc, 3).unwrap();
        let (_, width, height) = raster(&path, 3, &region).unwrap();
        assert!(height < width);
    }
    #[test]
    fn overlapping_prose_and_ambiguous_label_matches_are_not_rasterized() {
        let path = fixture();
        let doc = PdfDocument::open(&path).unwrap();
        let loaded = readwise_core::books::pdf::load_pages(&path).unwrap();
        for page in [4, 5] {
            let blocks = readwise_core::books::epub_meta::split_blocks(&loaded.chapters[page].html);
            let texts = blocks
                .iter()
                .map(|b| readwise_core::vocab::annotate::rendered_text(b))
                .collect::<Vec<_>>();
            assert!(
                project(&doc, &path, page, &blocks, &texts).is_none(),
                "ambiguous text cannot be hidden or captured on page {page}"
            );
            let display = MathPage::pdf(&path, page, &blocks, &texts);
            for (i, block) in blocks.iter().enumerate() {
                let html = display.annotate(i, block, str::to_owned);
                assert!(!html.contains("pdf-region-figure"));
                assert_eq!(
                    readwise_core::vocab::annotate::rendered_text(&html),
                    texts[i]
                );
            }
        }
    }
    #[test]
    fn ordinary_text_and_sparse_rotated_watermarks_do_not_become_images() {
        let doc = PdfDocument::open(fixture()).unwrap();
        assert!(figure_region(&doc, 0).is_some());
        assert!(figure_region(&doc, 1).is_none());
        assert!(figure_region(&doc, 2).is_none());
    }
}
