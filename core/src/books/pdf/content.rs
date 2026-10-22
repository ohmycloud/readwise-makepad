//! Enrich the immutable text projection using PDF font and annotation geometry.
//! Never guess links from words, nor treat all adjacent monospace runs as code.
#[cfg(test)]
#[path = "content_tests.rs"]
pub(super) mod tests;
use super::headings::BlockTarget;
use crate::{books::epub_meta::split_blocks, vocab::annotate::rendered_text};
use pdf_oxide::{annotations::LinkAction, geometry::Rect, layout::TextSpan, PdfDocument};
use std::{collections::HashMap, path::Path};

pub(super) fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
pub(super) fn monospace(span: &TextSpan) -> bool {
    let name = span.font_name.to_ascii_lowercase();
    span.is_monospace
        || [
            "courier",
            "firacode",
            "fira-code",
            "jetbrainsmono",
            "sourcecodepro",
            "source-code-pro",
            "consolas",
            "menlo",
            "inconsolata",
            "monospace",
            "dejavusansmono",
            "liberationmono",
        ]
        .iter()
        .any(|n| name.contains(n))
}
pub(super) struct Glyph {
    pub ch: char,
    pub rect: Rect,
    pub mono: bool,
    pub size: f32,
    pub span: usize,
    pub link: Option<usize>,
}
pub(super) struct Geometry {
    pub glyphs: Vec<Glyph>,
    starts: HashMap<char, Vec<usize>>,
    urls: Vec<String>,
    code_rows: Vec<bool>,
}
impl Geometry {
    pub(super) fn new(spans: &[TextSpan], links: &[(Rect, String)]) -> Self {
        let mono: Vec<_> = spans.iter().map(monospace).collect();
        let code_rows = spans
            .iter()
            .enumerate()
            .map(|(i, span)| {
                mono[i]
                    && !spans.iter().enumerate().any(|(j, other)| {
                        !mono[j]
                            && !other.text.trim().is_empty()
                            && (span.bbox.y - other.bbox.y).abs()
                                < span.font_size.max(other.font_size) * 0.25
                    })
            })
            .collect();
        let mut glyphs = Vec::new();
        let mut starts: HashMap<char, Vec<usize>> = HashMap::new();
        for (index, span) in spans.iter().enumerate() {
            if span.wmode != 0
                || span.rotation_degrees.abs() > 0.1
                || !span.font_size.is_finite()
                || span.font_size <= 0.0
            {
                continue;
            }
            for c in span.to_chars() {
                // Use glyph centres, not whole-span overlap. The baseline alone
                // can sit just below a link rectangle, hence the small rise.
                let x = c.bbox.x + c.bbox.width * 0.5;
                let y = c.bbox.y + c.font_size * 0.3;
                let link = links.iter().position(|(r, _)| {
                    x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
                });
                let normalized = match c.char {
                    'ﬀ' => "ff".into(),
                    'ﬁ' => "fi".into(),
                    'ﬂ' => "fl".into(),
                    'ﬃ' => "ffi".into(),
                    'ﬄ' => "ffl".into(),
                    'ﬅ' | 'ﬆ' => "st".into(),
                    c => c.to_string(),
                };
                for ch in normalized.chars().filter(|c| !c.is_whitespace()) {
                    starts.entry(ch).or_default().push(glyphs.len());
                    glyphs.push(Glyph {
                        ch,
                        rect: c.bbox,
                        mono: mono[index],
                        size: span.font_size,
                        span: index,
                        link,
                    });
                }
            }
        }
        Self {
            glyphs,
            starts,
            urls: links.iter().map(|(_, url)| url.clone()).collect(),
            code_rows,
        }
    }
    /// Match complete text runs, allowing the HTML converter to have relocated
    /// inline fixed-font runs. Short/ambiguous matches fail closed. No character
    /// is rewritten, deleted or reordered in the stored source text.
    fn locate(&self, text: &str, hint: Option<usize>) -> Vec<Option<usize>> {
        let chars: Vec<_> = text.chars().collect();
        let nonspace: Vec<_> = chars
            .iter()
            .copied()
            .filter(|c| !c.is_whitespace())
            .collect();
        let mut found = vec![None; nonspace.len()];
        let mut pos = 0;
        let mut previous = hint;
        while pos < nonspace.len() {
            let mut best = (0, 0, usize::MAX);
            let mut tied = false;
            for &start in self.starts.get(&nonspace[pos]).into_iter().flatten() {
                let len = nonspace[pos..]
                    .iter()
                    .zip(&self.glyphs[start..])
                    .take_while(|(c, g)| **c == g.ch)
                    .count();
                let distance = previous.map_or(0, |p: usize| start.abs_diff(p));
                if len > best.0 || (len == best.0 && distance < best.2) {
                    best = (len, start, distance);
                    tied = false;
                } else if len == best.0 && distance == best.2 {
                    tied = true;
                }
            }
            let (len, start, _) = best;
            let follows = previous == Some(start);
            if !tied && len > 0 && (len >= 5 || follows || (pos == 0 && len == nonspace.len())) {
                for i in 0..len {
                    found[pos + i] = Some(start + i);
                }
                previous = Some(start + len);
                pos += len;
            } else {
                pos += 1;
                previous = None;
            }
        }
        let mut next = found.into_iter();
        chars
            .into_iter()
            .map(|c| {
                if c.is_whitespace() {
                    None
                } else {
                    next.next().flatten()
                }
            })
            .collect()
    }
}
pub(super) struct Block {
    pub html: String,
    pub text: String,
    pub glyphs: Vec<Option<usize>>,
    pub code: bool,
}
pub(super) fn links(doc: &PdfDocument, page: usize) -> Vec<(Rect, String)> {
    doc.get_annotations(page)
        .unwrap_or_default()
        .into_iter()
        .flat_map(|a| {
            let mut result = Vec::new();
            let Some(LinkAction::Uri(url)) = a.action else {
                return result;
            };
            if !(url.starts_with("https://")
                || url.starts_with("http://")
                || url.starts_with("mailto:"))
            {
                return result;
            }
            if let Some(quads) = a.quad_points.filter(|q| !q.is_empty()) {
                for q in quads {
                    let xs = [q[0], q[2], q[4], q[6]];
                    let ys = [q[1], q[3], q[5], q[7]];
                    let x0 = xs.into_iter().fold(f64::INFINITY, f64::min);
                    let x1 = xs.into_iter().fold(f64::NEG_INFINITY, f64::max);
                    let y0 = ys.into_iter().fold(f64::INFINITY, f64::min);
                    let y1 = ys.into_iter().fold(f64::NEG_INFINITY, f64::max);
                    result.push((
                        Rect::new(x0 as f32, y0 as f32, (x1 - x0) as f32, (y1 - y0) as f32),
                        url.clone(),
                    ));
                }
                return result;
            }
            let Some([x0, y0, x1, y1]) = a.rect else {
                return result;
            };
            // QuadPoints take precedence over the enclosing rectangle for multiline
            // links, otherwise unrelated text between the lines would become linked.
            let rect = Rect::new(
                x0.min(x1) as f32,
                y0.min(y1) as f32,
                (x1 - x0).abs() as f32,
                (y1 - y0).abs() as f32,
            );
            result.push((rect, url));
            result
        })
        .collect()
}
pub(super) fn prepare_blocks(html: &str, geo: &Geometry) -> Vec<Block> {
    let mut hint = None;
    split_blocks(html)
        .into_iter()
        .map(|html| {
            let text = rendered_text(&html);
            let glyphs = geo.locate(&text, hint);
            if let Some(last) = glyphs.iter().flatten().last() {
                hint = Some(last + 1);
            }
            let mapped: Vec<_> = text
                .chars()
                .zip(&glyphs)
                .filter(|(c, _)| !c.is_whitespace())
                .collect();
            let code = !mapped.is_empty()
                && !html.contains("<h")
                && !html.contains("<pre")
                && mapped.iter().all(|(_, g)| {
                    g.is_some_and(|g| {
                        let glyph = &geo.glyphs[g];
                        glyph.mono && geo.code_rows[glyph.span]
                    })
                });
            Block {
                html,
                text,
                glyphs,
                code,
            }
        })
        .collect()
}
fn continues(a: &Block, b: &Block, geo: &Geometry) -> bool {
    let Some(a) = a
        .glyphs
        .iter()
        .rev()
        .flatten()
        .next()
        .map(|g| &geo.glyphs[*g])
    else {
        return false;
    };
    let Some(b) = b.glyphs.iter().flatten().next().map(|g| &geo.glyphs[*g]) else {
        return false;
    };
    let dy = a.rect.y - b.rect.y;
    let em = a.size.max(b.size);
    let gap = b.rect.x - (a.rect.x + a.rect.width);
    (a.size - b.size).abs() < em * 0.15
        && dy >= -em * 0.25
        && dy < em * 2.5
        && (dy > em * 0.3 || (gap >= -em * 0.5 && gap < em * 2.0))
}
fn code(group: &[Block], geo: &Geometry, block: usize) -> (String, Vec<BlockTarget>) {
    let left = group
        .iter()
        .flat_map(|b| &b.glyphs)
        .flatten()
        .map(|g| geo.glyphs[*g].rect.x)
        .fold(f32::INFINITY, f32::min);
    let mut source = String::new();
    let mut maps = Vec::new();
    let mut previous: Option<&Glyph> = None;
    for b in group {
        let mut offsets = Vec::with_capacity(b.text.len() + 1);
        for (c, g) in b.text.chars().zip(&b.glyphs) {
            if let Some(g) = g.map(|g| &geo.glyphs[g]) {
                let newline = previous
                    .is_some_and(|p| (p.rect.y - g.rect.y).abs() > p.size.max(g.size) * 0.3);
                if newline && !source.ends_with('\n') {
                    source.push('\n');
                }
                if previous.is_none() || newline {
                    let cell = (g.rect.width).max(g.size * 0.4);
                    let indent = ((g.rect.x - left) / cell).round().clamp(0.0, 80.0) as usize;
                    let existing = source
                        .rsplit('\n')
                        .next()
                        .filter(|s| s.chars().all(|c| c == ' '))
                        .map_or(0, str::len);
                    source.extend(std::iter::repeat_n(' ', indent.saturating_sub(existing)));
                } else if let Some(p) = previous {
                    if p.span != g.span && !source.ends_with(char::is_whitespace) {
                        let gap = g.rect.x - (p.rect.x + p.rect.width);
                        if gap > g.size * 0.2 {
                            source.extend(std::iter::repeat_n(
                                ' ',
                                (gap / g.rect.width.max(g.size * 0.4))
                                    .round()
                                    .clamp(1.0, 80.0) as usize,
                            ));
                        }
                    }
                }
                previous = Some(g);
            }
            let start = source.len();
            offsets.extend((0..c.len_utf8()).map(|i| start + i));
            source.push(c);
        }
        offsets.push(source.len());
        maps.push(BlockTarget {
            block,
            prefix: offsets.first().copied().unwrap_or(source.len()),
            old_len: b.text.len(),
            offsets: Some(offsets),
            layout_changed: false,
        });
    }
    (format!("<pre><code>{}</code></pre>", escape(&source)), maps)
}

/// Replace only a/code wrappers. Keep heading/emphasis tags and original text
/// entities intact. Close wrappers at each tag to avoid crossing HTML scopes.
pub(super) fn inline(b: &Block, geo: &Geometry) -> String {
    if b.html.contains("<pre") {
        return b.html.clone();
    }
    let mut styles: Vec<_> = b
        .glyphs
        .iter()
        .map(|g| {
            g.map(|g| {
                let g = &geo.glyphs[g];
                // HtmlLink consumes a plain label. A nested custom code
                // widget would disappear; a plain link still suppresses ruby.
                (g.mono && g.link.is_none(), g.link)
            })
            .unwrap_or((false, None))
        })
        .collect();
    let chars: Vec<_> = b.text.chars().collect();
    for i in 0..styles.len() {
        if chars[i].is_whitespace() && i > 0 {
            let next = chars[i..]
                .iter()
                .position(|c| !c.is_whitespace())
                .map(|n| i + n);
            if next.is_some_and(|n| styles[n] == styles[i - 1]) {
                styles[i] = styles[i - 1];
            }
        }
    }
    let mut out = String::new();
    let mut rest = b.html.as_str();
    let mut character = 0;
    while !rest.is_empty() {
        if rest.starts_with('<') {
            let Some(end) = rest.find('>') else {
                out.push_str(rest);
                break;
            };
            let tag = &rest[1..end];
            let name = tag
                .trim_start_matches('/')
                .split([' ', '/', '\t', '\n'])
                .next()
                .unwrap_or("");
            if !matches!(name, "a" | "code") {
                out.push_str(&rest[..=end]);
            }
            if name == "br" && !tag.starts_with('/') {
                character += 1;
            }
            rest = &rest[end + 1..];
            continue;
        }
        let end = rest.find('<').unwrap_or(rest.len());
        let node = &rest[..end];
        let mut i = 0;
        let mut active = (false, None);
        while i < node.len() {
            let tail = &node[i..];
            let len = if tail.starts_with('&') {
                tail.find(';')
                    .filter(|n| *n <= 10)
                    .map(|n| n + 1)
                    .unwrap_or(1)
            } else {
                tail.chars().next().unwrap().len_utf8()
            };
            let raw = &tail[..len];
            let decoded = rendered_text(raw);
            let style = styles.get(character).copied().unwrap_or((false, None));
            if style != active {
                close(&mut out, active);
                if let Some(link) = style.1 {
                    out.push_str(&format!("<a href=\"{}\">", escape(&geo.urls[link])));
                }
                if style.0 {
                    out.push_str("<code>");
                }
                active = style;
            }
            out.push_str(raw);
            character += decoded.chars().count();
            i += len;
        }
        close(&mut out, active);
        rest = &rest[end..];
    }
    out
}
fn close(out: &mut String, style: (bool, Option<usize>)) {
    if style.0 {
        out.push_str("</code>");
    }
    if style.1.is_some() {
        out.push_str("</a>");
    }
}

/// Assets are written only to the application-owned directory. Relative paths
/// in snapshots keep a copied/moved application data directory usable.
fn images(
    doc: &PdfDocument,
    page: usize,
    directory: Option<&Path>,
    blocks: &[Block],
    geo: &Geometry,
) -> Result<Vec<(usize, String)>, String> {
    let Some(directory) = directory else {
        return Ok(Vec::new());
    };
    let mut result = Vec::new();
    for (index, image) in doc
        .extract_images(page)
        .map_err(|e| format!("提取 PDF 图片失败: {e}"))?
        .into_iter()
        .enumerate()
    {
        if image.width() < 8 || image.height() < 8 {
            continue;
        }
        let Some(rect) = image.bbox() else { continue };
        if ![rect.x, rect.y, rect.width, rect.height]
            .iter()
            .all(|x| x.is_finite())
            || rect.width < 8.0
            || rect.height < 8.0
        {
            continue;
        }
        // Place before the first paragraph below the picture's visual centre,
        // normally its caption. Never drop a caption or replace text with pixels.
        let center = rect.y + rect.height * 0.5;
        let at = blocks
            .iter()
            .position(|b| {
                b.glyphs
                    .iter()
                    .flatten()
                    .next()
                    .is_some_and(|g| geo.glyphs[*g].rect.y < center)
            })
            .unwrap_or(blocks.len());
        let relative = format!("pdf-assets-v3/page-{page}-{index}.png");
        let dest = directory.join(&relative);
        std::fs::create_dir_all(dest.parent().unwrap())
            .map_err(|e| format!("创建 PDF 图片目录失败: {e}"))?;
        if !dest.is_file() {
            if u64::from(image.width()) * u64::from(image.height()) > 32 * 1024 * 1024 {
                return Err("PDF 图片过大，无法安全解码".into());
            }
            let data = image
                .to_png_bytes()
                .map_err(|e| format!("解码 PDF 图片失败: {e}"))?;
            let tmp = dest.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
            if let Err(e) = std::fs::write(&tmp, data).and_then(|_| std::fs::rename(&tmp, &dest)) {
                let _ = std::fs::remove_file(&tmp);
                return Err(format!("保存 PDF 图片失败: {e}"));
            }
        }
        result.push((at, format!("<p><img src=\"{relative}\"/></p>")));
    }
    result.sort_by_key(|(at, _)| *at);
    Ok(result)
}

pub(super) fn repair(
    doc: &PdfDocument,
    page: usize,
    html: &str,
    directory: Option<&Path>,
) -> Result<(String, Vec<BlockTarget>), String> {
    let spans = doc
        .extract_spans(page)
        .map_err(|e| format!("读取 PDF 字体坐标失败: {e}"))?;
    let geo = Geometry::new(&spans, &links(doc, page));
    let blocks = prepare_blocks(html, &geo);
    let assets = images(doc, page, directory, &blocks, &geo)?;
    project(&blocks, &geo, assets)
}
fn project(
    blocks: &[Block],
    geo: &Geometry,
    assets: Vec<(usize, String)>,
) -> Result<(String, Vec<BlockTarget>), String> {
    let mut output = Vec::new();
    let mut maps = Vec::new();
    let mut i = 0;
    let mut assets = assets.into_iter().peekable();
    while i < blocks.len() {
        while assets.peek().is_some_and(|(at, _)| *at <= i) {
            output.push(assets.next().unwrap().1);
        }
        if blocks[i].code {
            let mut end = i + 1;
            while end < blocks.len()
                && blocks[end].code
                && !assets.peek().is_some_and(|(at, _)| *at <= end)
                && continues(&blocks[end - 1], &blocks[end], geo)
            {
                end += 1;
            }
            let (html, group) = code(&blocks[i..end], geo, output.len());
            output.push(html);
            maps.extend(group);
            i = end;
        } else {
            maps.push(BlockTarget {
                block: output.len(),
                prefix: 0,
                old_len: blocks[i].text.len(),
                offsets: None,
                layout_changed: false,
            });
            output.push(inline(&blocks[i], geo));
            i += 1;
        }
    }
    output.extend(assets.map(|(_, html)| html));
    Ok((output.join("\n"), maps))
}

/// Regenerate missing derived assets without re-extracting or changing text.
pub(super) fn ensure_assets(path: &Path, loaded: &super::LoadedChapters) -> Result<(), String> {
    let directory = path.parent().ok_or("PDF 目录无效")?;
    let mut missing = std::collections::BTreeSet::new();
    for chapter in &loaded.chapters {
        for rest in chapter.html.split("src=\"pdf-assets-v3/").skip(1) {
            let name = rest.split('"').next().unwrap_or("");
            let Some((page, image)) = name
                .strip_prefix("page-")
                .and_then(|s| s.strip_suffix(".png"))
                .and_then(|s| s.split_once('-'))
            else {
                return Err("PDF 图片路径无效".into());
            };
            let (Ok(page), Ok(image)) = (page.parse::<usize>(), image.parse::<usize>()) else {
                return Err("PDF 图片路径无效".into());
            };
            if name != format!("page-{page}-{image}.png") {
                return Err("PDF 图片路径无效".into());
            }
            if !directory.join("pdf-assets-v3").join(name).is_file() {
                missing.insert((page, image));
            }
        }
    }
    if !missing.is_empty() {
        let doc = super::open(super::read_pdf(path)?)?;
        for (page, index) in missing {
            let images = doc.extract_images(page).map_err(|e| e.to_string())?;
            let image = images
                .get(index)
                .ok_or("PDF 图片缺失，原正文快照保持不变")?;
            if u64::from(image.width()) * u64::from(image.height()) > 32 * 1024 * 1024 {
                return Err("PDF 图片过大，无法安全解码".into());
            }
            let destination = directory
                .join("pdf-assets-v3")
                .join(format!("page-{page}-{index}.png"));
            std::fs::create_dir_all(destination.parent().unwrap()).map_err(|e| e.to_string())?;
            let tmp = destination.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
            let bytes = image.to_png_bytes().map_err(|e| e.to_string())?;
            if let Err(e) =
                std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, destination))
            {
                let _ = std::fs::remove_file(tmp);
                return Err(e.to_string());
            }
        }
    }
    Ok(())
}

pub(super) fn resolve_assets(
    mut loaded: super::LoadedChapters,
    path: &Path,
) -> super::LoadedChapters {
    let base = escape(
        &path
            .parent()
            .unwrap_or(Path::new(""))
            .join("pdf-assets-v3")
            .to_string_lossy(),
    );
    for chapter in &mut loaded.chapters {
        chapter.html = chapter
            .html
            .replace("src=\"pdf-assets-v3/", &format!("src=\"{base}/"));
    }
    loaded
}
