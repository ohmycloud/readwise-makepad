//! A closed, offline Typst world. Only app-generated glyph/line placement enters
//! the compiler: PDF text is always a quoted string, never evaluated as Typst.
use super::{Glyph, Rule};
use std::sync::LazyLock;
use typst::{
    Library, LibraryExt, World,
    diag::{FileError, FileResult},
    foundations::{Bytes, Datetime, Duration},
    syntax::{FileId, Source},
    text::{Font, FontBook},
    utils::LazyHash,
};

struct Fonts {
    fonts: Vec<Font>,
    book: LazyHash<FontBook>,
    library: LazyHash<Library>,
}
static FONTS: LazyLock<Fonts> = LazyLock::new(|| {
    let fonts: Vec<_> = typst_assets::fonts()
        .flat_map(|bytes| Font::iter(Bytes::new(bytes)))
        .collect();
    Fonts {
        book: LazyHash::new(FontBook::from_fonts(fonts.iter())),
        fonts,
        library: LazyHash::new(Library::default()),
    }
});
struct ClosedWorld(Source);
impl World for ClosedWorld {
    fn library(&self) -> &LazyHash<Library> {
        &FONTS.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        &FONTS.book
    }
    fn main(&self) -> FileId {
        self.0.id()
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main() {
            Ok(self.0.clone())
        } else {
            Err(FileError::AccessDenied)
        }
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if id == self.main() {
            Ok(Bytes::from_string(self.0.text().to_owned()))
        } else {
            Err(FileError::AccessDenied)
        }
    }
    fn font(&self, index: usize) -> Option<Font> {
        FONTS.fonts.get(index).cloned()
    }
    fn today(&self, _: Option<Duration>) -> Option<Datetime> {
        None
    }
}

pub(super) struct Rendered {
    pub xml: String,
    pub ink: Option<[f32; 4]>,
}
fn ink_bounds(frame: &typst::layout::Frame) -> Option<[f32; 4]> {
    use typst::layout::{FrameItem, Point};
    fn visit(frame: &typst::layout::Frame, offset: Point, bounds: &mut [f32; 4]) -> Option<()> {
        for (at, item) in frame.items() {
            let at = offset + *at;
            let bbox = match item {
                FrameItem::Group(group) => {
                    if !group.transform.is_identity() || group.clip.is_some() {
                        return None;
                    }
                    visit(&group.frame, at, bounds)?;
                    continue;
                }
                FrameItem::Text(text) => text.bbox(),
                FrameItem::Shape(shape, _) => shape.bbox(true),
                FrameItem::Tag(_) => continue,
                _ => return None,
            };
            let values = [
                (bbox.min.x + at.x).to_pt() as f32,
                (bbox.min.y + at.y).to_pt() as f32,
                (bbox.max.x + at.x).to_pt() as f32,
                (bbox.max.y + at.y).to_pt() as f32,
            ];
            if !values.iter().all(|v| v.is_finite())
                || values[0] == values[2]
                || values[1] == values[3]
            {
                continue;
            }
            bounds[0] = bounds[0].min(values[0].min(values[2]));
            bounds[1] = bounds[1].min(values[1].min(values[3]));
            bounds[2] = bounds[2].max(values[0].max(values[2]));
            bounds[3] = bounds[3].max(values[1].max(values[3]));
        }
        Some(())
    }
    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    visit(frame, Point::zero(), &mut bounds)?;
    (bounds.iter().all(|v| v.is_finite()) && bounds[2] > bounds[0] && bounds[3] > bounds[1])
        .then_some(bounds)
}

pub(super) fn svg(glyphs: &[Glyph], rules: &[Rule], width: f32, height: f32) -> Option<Rendered> {
    // Finite, bounded coordinates, a fixed page, no loops, imports, files,
    // packages, system fonts, clock or network. Glyph/rule counts are bounded.
    if glyphs.is_empty()
        || glyphs.len() > 512
        || rules.len() > 64
        || !width.is_finite()
        || !height.is_finite()
        || !(1.0..=4096.0).contains(&width)
        || !(1.0..=1024.0).contains(&height)
    {
        return None;
    }
    if glyphs.iter().any(|g| {
        !FONTS
            .fonts
            .iter()
            .any(|font| font.info().coverage.contains(g.paint as u32))
    }) {
        return None;
    }
    let mut source = format!(
        "#set page(width: {width}pt, height: {height}pt, margin: 0pt, fill: none)\n#set text(font: (\"New Computer Modern\", \"New Computer Modern Math\"), fill: black)\n"
    );
    for glyph in glyphs {
        if glyph.paint == '√'
            && rules
                .iter()
                .any(|r| r.dy != 0.0 && (r.x - glyph.x).abs() < 0.01)
        {
            continue;
        }
        let literal = serde_json::to_string(&glyph.paint.to_string()).ok()?;
        let style = if glyph.italic { "italic" } else { "normal" };
        let weight = if glyph.bold { "bold" } else { "regular" };
        source.push_str(&format!("#place(top + left, dx: {:.4}pt, dy: {:.4}pt, text(size: {:.4}pt, style: \"{style}\", weight: \"{weight}\", top-edge: \"baseline\", bottom-edge: \"baseline\", {literal}))\n", glyph.x, glyph.y, glyph.size));
    }
    for rule in rules {
        source.push_str(&format!("#place(top + left, dx: {:.4}pt, dy: {:.4}pt, line(start: (0pt, 0pt), end: ({:.4}pt, {:.4}pt), stroke: {:.4}pt))\n", rule.x, rule.y, rule.width, rule.dy, rule.thickness));
    }
    // Ensure an otherwise out-of-flow page is emitted.
    source.push_str("#box(width: 0pt, height: 0pt)\n");
    let world = ClosedWorld(Source::detached(source));
    let result = typst::compile::<typst_layout::PagedDocument>(&world)
        .output
        .ok()?;
    if result.pages().len() != 1 {
        return None;
    }
    let ink = ink_bounds(&result.pages()[0].frame);
    let svg = typst_svg::svg(&result.pages()[0], &typst_svg::SvgOptions::default())
        .replace("fill=\"#000000\"", "fill=\"currentColor\"")
        .replace("stroke=\"#000000\"", "stroke=\"currentColor\"");
    drop(result);
    drop(world);
    typst::comemo::evict(2);
    (svg.len() <= 1024 * 1024).then_some(Rendered { xml: svg, ink })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closed_world_has_no_files_packages_clock_or_unquoted_code() {
        let world = ClosedWorld(Source::detached(""));
        use typst::syntax::{RootedPath, VirtualPath, VirtualRoot};
        let other = RootedPath::new(
            VirtualRoot::Project,
            VirtualPath::new("secret.typ").unwrap(),
        )
        .intern();
        assert!(world.source(other).is_err());
        assert!(world.file(other).is_err());
        assert!(world.today(None).is_none());
        let glyphs = vec![Glyph {
            ch: '#',
            paint: '#',
            id: 0,
            source: true,
            roman: false,
            x: 2.,
            y: 12.,
            size: 10.,
            width: 6.,
            math: true,
            italic: false,
            bold: false,
        }];
        let output = svg(&glyphs, &[], 20., 20.).expect("real embedded Typst compilation");
        assert!(output.ink.is_some());
        let output = output.xml;
        assert!(output.contains("<svg") && output.contains("<path"));
        assert!(!output.contains("<text") && !output.contains("<script"));
        assert!(output.contains("currentColor"));
        let mut unsupported = glyphs;
        unsupported[0].paint = '\u{10ffff}';
        assert!(svg(&unsupported, &[], 20., 20.).is_none());
        assert!(svg(&[], &[], f32::NAN, 20.).is_none());
    }
}
