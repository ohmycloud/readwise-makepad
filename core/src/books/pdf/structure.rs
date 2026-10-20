//! v4: conservative source-typography repair. v3 extraction stays frozen because
//! its projection hash is already committed alongside users' note coordinates.
use super::{
    content::{self, Block, Geometry, Glyph},
    headings::BlockTarget,
};
use crate::vocab::annotate::rendered_text;
use pdf_oxide::{
    geometry::Rect,
    layout::{TextChar, TextSpan},
    PdfDocument,
};
use std::collections::BTreeMap;

#[cfg(test)]
#[path = "structure_tests.rs"]
mod tests;

fn contains(rect: Rect, x: f32, y: f32) -> bool {
    x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
}
fn color(c: &TextChar) -> [u8; 3] {
    [c.color.r, c.color.g, c.color.b].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}
/// A wrapped annotation without QuadPoints may bound entire lines, including
/// unrelated black prose. Use actual glyph colors (span merging loses them),
/// and let smaller annotations win. Never invent a word-based link boundary.
fn refine_links(links: &[(Rect, String)], chars: &[TextChar]) -> Vec<(Rect, String)> {
    let mut colors = BTreeMap::<[u8; 3], usize>::new();
    for c in chars.iter().filter(|c| !c.char.is_whitespace()) {
        *colors.entry(color(c)).or_default() += 1;
    }
    let dominant = colors
        .iter()
        .max_by(|(a, ac), (b, bc)| ac.cmp(bc).then_with(|| b.cmp(a)))
        .map(|(c, _)| *c);
    let mut result = Vec::new();
    for (index, (rect, url)) in links.iter().enumerate() {
        let inside: Vec<_> = chars
            .iter()
            .filter(|c| {
                !c.char.is_whitespace()
                    && c.rotation_degrees.abs() < 0.1
                    && contains(
                        *rect,
                        c.bbox.x + c.bbox.width * 0.5,
                        c.bbox.y + c.font_size * 0.3,
                    )
            })
            .collect();
        let mut rows: Vec<f32> = Vec::new();
        for c in &inside {
            if !rows
                .iter()
                .any(|y| (*y - c.bbox.y).abs() < c.font_size * 0.3)
            {
                rows.push(c.bbox.y);
            }
        }
        let colored = inside
            .iter()
            .any(|c| dominant.is_some_and(|d| color(c) != d));
        if !colored {
            // A tight, single-line black link is unambiguous. A multi-line
            // bounding box alone cannot identify its first/last linked glyph.
            if rows.len() == 1 {
                result.push((*rect, url.clone()));
            }
            continue;
        }
        let mut regions: Vec<Rect> = Vec::new();
        let mut adjacent = false;
        for c in inside {
            if dominant == Some(color(c)) {
                adjacent = false;
                continue;
            }
            let x = c.bbox.x + c.bbox.width * 0.5;
            let y = c.bbox.y + c.font_size * 0.3;
            if links.iter().enumerate().any(|(other, (r, _))| {
                other != index
                    && r.width * r.height < rect.width * rect.height
                    && contains(*r, x, y)
            }) {
                adjacent = false;
                continue;
            }
            let glyph = Rect::new(
                c.bbox.x,
                c.bbox.y - c.font_size * 0.1,
                c.bbox.width,
                c.font_size * 1.1,
            );
            if let Some(last) = regions.last_mut().filter(|r| {
                adjacent
                    && (r.y - glyph.y).abs() < c.font_size * 0.2
                    && glyph.x >= r.x
                    && glyph.x - (r.x + r.width) <= c.font_size * 0.8
            }) {
                *last = last.union(&glyph);
            } else {
                regions.push(glyph);
            }
            adjacent = true;
        }
        result.extend(regions.into_iter().map(|r| (r, url.clone())));
    }
    result.sort_by(|(a, _), (b, _)| (a.width * a.height).total_cmp(&(b.width * b.height)));
    result
}
fn heading_body(html: &str) -> Option<&str> {
    let h = html.trim();
    let tag = h.as_bytes();
    if tag.len() < 4
        || tag[0] != b'<'
        || tag[1] != b'h'
        || !(b'1'..=b'6').contains(&tag[2])
        || tag[3] != b'>'
    {
        return None;
    }
    h[4..].strip_suffix(&format!("</h{}>", tag[2] as char))
}
fn paragraph_body(html: &str) -> Option<&str> {
    html.trim().strip_prefix("<p>")?.strip_suffix("</p>")
}
fn glyphs<'a>(block: &'a Block, geo: &'a Geometry) -> impl Iterator<Item = &'a Glyph> {
    block.glyphs.iter().flatten().map(|g| &geo.glyphs[*g])
}
fn fully_mapped(block: &Block) -> bool {
    block
        .text
        .chars()
        .zip(&block.glyphs)
        .all(|(c, g)| c.is_whitespace() || g.is_some())
        && block.glyphs.iter().any(Option::is_some)
}
/// Establish prose sizes from existing normal paragraphs, not the page-wide
/// mean (code/footnotes skew it) or the largest size (a pull quote can skew it).
/// Suspect headings and short labels cannot establish their own body style.
fn body_sizes(blocks: &[Block], geo: &Geometry, spans: &[TextSpan]) -> Vec<f32> {
    let mut sizes = BTreeMap::<i32, usize>::new();
    for b in blocks
        .iter()
        .filter(|b| paragraph_body(&b.html).is_some() && b.text.chars().count() >= 40)
    {
        for g in glyphs(b, geo)
            .filter(|g| !g.mono && !spans[g.span].font_weight.is_bold() && !spans[g.span].is_italic)
        {
            *sizes.entry((g.size * 10.0).round() as i32).or_default() += 1;
        }
    }
    sizes
        .into_iter()
        .filter(|(_, n)| *n >= 40)
        .map(|(s, _)| s as f32 / 10.0)
        .collect()
}
struct Part {
    block: Block,
    demoted: bool,
    changed: bool,
}
fn parts(html: &str, geo: &Geometry, spans: &[TextSpan]) -> Vec<Part> {
    let blocks = content::prepare_blocks(html, geo);
    let body_sizes = body_sizes(&blocks, geo, spans);
    blocks
        .into_iter()
        .map(|mut block| {
            let demoted = heading_body(&block.html).is_some()
                && fully_mapped(&block)
                && body_sizes.iter().any(|size| {
                    glyphs(&block, geo).all(|g| {
                        (if g.mono {
                            g.size <= size * 1.06
                        } else {
                            (g.size - size).abs() <= size * 0.06
                        }) && !spans[g.span].font_weight.is_bold()
                            && spans[g.span].heading_level.is_none()
                    })
                });
            let old = block.html.clone();
            if demoted {
                let body = heading_body(&block.html).unwrap();
                block.html = format!("<p>{body}</p>");
            }
            // A spurious heading must not leave inferred bold wrappers behind.
            if fully_mapped(&block)
                && glyphs(&block, geo).all(|g| !spans[g.span].font_weight.is_bold())
                && !block.html.contains("<pre")
            {
                for tag in ["<strong>", "</strong>", "<b>", "</b>"] {
                    block.html = block.html.replace(tag, "");
                }
            }
            block.html = content::inline(&block, geo);
            let changed = block.html != old;
            Part {
                block,
                demoted,
                changed,
            }
        })
        .collect()
}
/// Only bridge boundaries introduced by demoted headings, or converter-created
/// fragments on the SAME source line. Ordinary paragraphs, code, images, lists,
/// columns and genuine headings retain their boundaries.
fn bridge(a: &Part, b: &Part, geo: &Geometry, in_repaired_paragraph: bool) -> Option<&'static str> {
    if paragraph_body(&a.block.html).is_none()
        || paragraph_body(&b.block.html).is_none()
        || a.block.html.contains("<img")
        || b.block.html.contains("<img")
        || !fully_mapped(&a.block)
        || !fully_mapped(&b.block)
    {
        return None;
    }
    let last = glyphs(&a.block, geo).last()?;
    let first = glyphs(&b.block, geo).next()?;
    let em = last.size.max(first.size);
    if (last.size - first.size).abs() > em * 0.08 {
        return None;
    }
    let dy = last.rect.y - first.rect.y;
    if dy.abs() < em * 0.25 {
        let gap = first.rect.x - (last.rect.x + last.rect.width);
        if gap < -em * 0.25 || gap > em * 0.8 {
            return None;
        }
        return Some(if gap > em * 0.15 { " " } else { "" });
    }
    if !(a.demoted || b.demoted || in_repaired_paragraph) || dy < em * 0.8 || dy > em * 1.4 {
        return None;
    }
    let left = glyphs(&a.block, geo)
        .filter(|g| (g.rect.y - last.rect.y).abs() < em * 0.25)
        .map(|g| g.rect.x)
        .fold(f32::INFINITY, f32::min);
    if (first.rect.x - left).abs() > em * 0.6 {
        return None;
    }
    Some(" ")
}
fn project(html: &str, spans: &[TextSpan], links: &[(Rect, String)]) -> (String, Vec<BlockTarget>) {
    let geo = Geometry::new(spans, links);
    let parts = parts(html, &geo, spans);
    let mut output = Vec::new();
    let mut maps = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let mut body = paragraph_body(&parts[i].block.html).map(str::to_owned);
        let mut end = i + 1;
        let mut repaired = parts[i].demoted;
        maps.push(BlockTarget {
            block: output.len(),
            prefix: 0,
            old_len: parts[i].block.text.len(),
            offsets: None,
            layout_changed: parts[i].changed,
        });
        if let Some(content) = body.as_mut() {
            while end < parts.len() {
                let Some(space) = bridge(&parts[end - 1], &parts[end], &geo, repaired) else {
                    break;
                };
                if !rendered_text(content).ends_with(char::is_whitespace)
                    && !parts[end].block.text.starts_with(char::is_whitespace)
                {
                    content.push_str(space);
                }
                maps.push(BlockTarget {
                    block: output.len(),
                    prefix: rendered_text(content).len(),
                    old_len: parts[end].block.text.len(),
                    offsets: None,
                    layout_changed: true,
                });
                content.push_str(paragraph_body(&parts[end].block.html).unwrap());
                repaired |= parts[end].demoted;
                end += 1;
            }
        }
        output.push(if end > i + 1 {
            format!("<p>{}</p>", body.unwrap())
        } else {
            parts[i].block.html.clone()
        });
        i = end;
    }
    (output.join("\n"), maps)
}
pub(super) fn repair(
    doc: &PdfDocument,
    page: usize,
    html: &str,
) -> Result<(String, Vec<BlockTarget>), String> {
    let spans = doc
        .extract_spans(page)
        .map_err(|e| format!("读取 PDF 正文样式失败: {e}"))?;
    let links = content::links(doc, page);
    let links = if links.is_empty() {
        links
    } else {
        let chars = doc
            .extract_chars(page)
            .map_err(|e| format!("读取 PDF 逐字链接样式失败: {e}"))?;
        refine_links(&links, &chars)
    };
    Ok(project(html, &spans, &links))
}
