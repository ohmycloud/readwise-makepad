//! Native frontend for the SAME offline Typst projection used by the Web.
//! Only generated outlines enter DrawSvg. Source text still enters TextFlow in
//! its original byte order, including zero-width displaced-script proxies.
use makepad_widgets::image::DrawImage;
use makepad_widgets::makepad_draw::text::layouter::LaidoutText;
use makepad_widgets::makepad_html::HtmlDoc;
use makepad_widgets::*;
use readwise_math::{Figure, Formula, MathPage, Projection};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
    sync::{Arc, LazyLock, RwLock},
};

static COMPILE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The import worker prepares the first display before publishing the shelf
/// result. Kept out of core: database/text extraction does not depend on Typst.
pub fn prepare_import(db: &readwise_core::Db, id: &str, relative: &str) {
    let path = db.data_dir().join(relative);
    if !path
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("pdf"))
    {
        return;
    }
    match db.load_book_chapters(id, &path) {
        Ok(loaded) => {
            if let Some(first) = loaded.chapters.first() {
                let mut book = BookMath::new(&path.to_string_lossy(), id, 0);
                book.prepare_initial(0, &first.html);
            }
        }
        Err(error) => ::log::warn!("PDF initial presentation preparation skipped: {error}"),
    }
}
enum Visual {
    Formula(Formula),
    Figure(Figure),
}
static FORMULAS: LazyLock<RwLock<HashMap<String, Arc<Visual>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
#[derive(Debug)]
pub struct Ready {
    pub token: String,
    pub chapter: usize,
    pub page: Arc<MathPage>,
}
#[derive(Default)]
pub struct BookMath {
    session: Option<Arc<readwise_math::PdfSession>>,
    token: String,
    pages: VecDeque<(usize, Arc<MathPage>)>,
    pending: HashSet<usize>,
    navigation: Option<(usize, usize, f64, bool)>,
    current: usize,
    warmed: HashSet<usize>,
    pagination_changes: HashSet<usize>,
}
impl BookMath {
    pub fn new(path: &str, id: &str, request: u64) -> Self {
        let path = std::path::PathBuf::from(path);
        Self {
            session: path
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("pdf"))
                .then(|| {
                    // Only app-owned copies get disk writes, never arbitrary
                    // sources/fixtures passed to the standalone projection API.
                    let managed = path.file_name().is_some_and(|s| s == "book.pdf")
                        && path
                            .parent()
                            .and_then(std::path::Path::parent)
                            .and_then(std::path::Path::file_name)
                            .is_some_and(|s| s == "books");
                    Arc::new(if managed {
                        let directory = path.parent().unwrap().join("pdf-visual-cache");
                        readwise_math::PdfSession::with_cache(path, directory)
                    } else {
                        readwise_math::PdfSession::new(path)
                    })
                }),
            token: format!("{id}/{request}/"),
            pages: VecDeque::new(),
            pending: HashSet::new(),
            navigation: None,
            current: 0,
            warmed: HashSet::new(),
            pagination_changes: HashSet::new(),
        }
    }
    fn compile(session: &readwise_math::PdfSession, chapter: usize, raw: &str) -> Arc<MathPage> {
        let blocks = readwise_core::books::epub_meta::split_blocks(raw);
        let texts = blocks
            .iter()
            .map(|b| readwise_core::vocab::annotate::rendered_text(b))
            .collect::<Vec<_>>();
        let page = Arc::new(
            session
                .cached_page(chapter, &blocks, &texts)
                .unwrap_or_else(|| {
                    let _compile = COMPILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
                    // Recheck inside page(): a different worker may have prepared it
                    // while we were waiting. Cache hits never wait for neighbor work.
                    session.page(chapter, &blocks, &texts)
                }),
        );
        // PDF source pages often contain large PNG diagrams. Prepare their
        // unchanged theme projection here, never in the UI's pagination probes.
        let (bg, fg) = crate::app::current_reader_image_palette();
        crate::reader_image_cache::prepare_chapter(raw, bg, fg);
        page
    }
    /// Only called by the existing book-opening BACKGROUND worker.
    pub fn prepare_initial(&mut self, chapter: usize, raw: &str) {
        self.current = chapter;
        if let Some(session) = &self.session {
            self.pages
                .push_back((chapter, Self::compile(session, chapter, raw)));
        }
    }
    pub fn page(&mut self, chapter: usize, raw: &str) -> Option<Arc<MathPage>> {
        let session = self.session.clone()?;
        if let Some((_, page)) = self.pages.iter().find(|(i, _)| *i == chapter) {
            return Some(page.clone());
        }
        // One in-flight compile: a foreground jump must not queue behind two
        // speculative pages. Its latest destination is retried on completion.
        if !self.pending.is_empty() {
            return None;
        }
        if self.pending.insert(chapter) {
            let raw = raw.to_owned();
            let token = self.token.clone();
            std::thread::spawn(move || {
                Cx::post_action(Ready {
                    token,
                    chapter,
                    page: Self::compile(&session, chapter, &raw),
                })
            });
        }
        None
    }
    fn next_neighbor(&self, chapters: usize) -> Option<usize> {
        if self.session.is_none() || !self.pending.is_empty() || self.navigation.is_some() {
            return None;
        }
        [self.current.checked_add(1), self.current.checked_sub(1)]
            .into_iter()
            .flatten()
            .find(|i| {
                *i < chapters
                    && !self.warmed.contains(i)
                    && !self.pages.iter().any(|(cached, _)| cached == i)
            })
    }
    /// Warm only the adjacent original pages, not the entire PDF. Keep the
    /// existing six-page/8 MiB budget and never reload visible content. Remember
    /// attempts until the next actual navigation so large pages cannot cause an
    /// endless prefetch/eviction cycle. A requested destination always wins.
    pub fn warm_neighbors(&mut self, chapters: &[String]) {
        if let Some(chapter) = self.next_neighbor(chapters.len()) {
            self.warmed.insert(chapter);
            self.page(chapter, &chapters[chapter]);
        }
    }
    pub fn defer_navigation(
        &mut self,
        chapter: usize,
        raw: &str,
        block: usize,
        scroll: f64,
        persist: bool,
    ) -> bool {
        if self.session.is_none() || self.pages.iter().any(|(i, _)| *i == chapter) {
            self.navigation = None;
            if self.current != chapter {
                self.warmed.clear();
            }
            self.current = chapter;
            return false;
        }
        self.navigation = Some((chapter, block, scroll, persist));
        self.page(chapter, raw);
        true
    }
    pub fn take_pagination_change(&mut self, chapter: usize) -> bool {
        self.pagination_changes.remove(&chapter)
    }
    pub fn cancel_navigation(&mut self) {
        self.navigation = None;
    }
    pub fn waiting_navigation(&self) -> Option<usize> {
        self.navigation.map(|(i, _, _, _)| i)
    }
    pub fn take_navigation(&mut self, chapter: usize) -> Option<(usize, usize, f64, bool)> {
        if self.navigation.is_some_and(|(i, _, _, _)| i == chapter) {
            self.navigation.take()
        } else {
            None
        }
    }
    pub fn accept(&mut self, ready: &Ready) -> Option<Vec<usize>> {
        if ready.token != self.token || !self.pending.remove(&ready.chapter) {
            return None;
        }
        let mut removed = Vec::new();
        while self.pages.len() >= 6
            || self.pages.iter().map(|(_, p)| p.bytes).sum::<usize>() + ready.page.bytes
                > 8 * 1024 * 1024
        {
            let Some(index) = self
                .pages
                .iter()
                .position(|(i, _)| *i != self.current && *i != ready.chapter)
            else {
                break;
            };
            let (chapter, _) = self.pages.remove(index).unwrap();
            self.pagination_changes.remove(&chapter);
            let prefix = format!("{}{chapter}/", self.token);
            FORMULAS
                .write()
                .unwrap()
                .retain(|key, _| !key.starts_with(&prefix));
            removed.push(chapter);
        }
        if ready.page.bytes > 0 {
            self.pagination_changes.insert(ready.chapter);
        }
        self.pages.push_back((ready.chapter, ready.page.clone()));
        Some(removed)
    }
    pub fn project(
        &self,
        page: &MathPage,
        chapter: usize,
        block: usize,
        html: &str,
        annotate: impl FnOnce(&str) -> String,
    ) -> String {
        page.project_with(block, html, annotate, |part, text, _| {
            let (id, tag) = match part {
                Projection::Formula(f) => {
                    let key = format!("{}{chapter}/{}", self.token, f.id);
                    FORMULAS
                        .write()
                        .unwrap()
                        .entry(key.clone())
                        .or_insert_with(|| Arc::new(Visual::Formula(f.clone())));
                    (key, "reader_math")
                }
                Projection::Figure(f) => {
                    let key = format!("{}{chapter}/{}", self.token, f.id);
                    FORMULAS
                        .write()
                        .unwrap()
                        .entry(key.clone())
                        .or_insert_with(|| Arc::new(Visual::Figure(f.clone())));
                    (key, "reader_math")
                }
                Projection::Hidden { owner } => (
                    format!("{}{chapter}/{owner}", self.token),
                    "reader_math_hidden",
                ),
            };
            format!("<{tag} m=\"{id}\">{}</{tag}>", escape(text))
        })
    }
}
/// Only diagram-label source proxies have no independent visual layout. Ordinary
/// scattered-formula proxies retain their established paragraph geometry.
pub fn is_figure_source(html: &str) -> bool {
    html.starts_with("<reader_math_hidden m=\"") && html.contains("/pdf-figure-")
}

pub fn figure_owner_block(blocks: &[String], block: usize) -> Option<usize> {
    is_figure_source(blocks.get(block)?).then(|| {
        blocks
            .iter()
            .position(|html| html.starts_with("<reader_math m=\"") && html.contains("/pdf-figure-"))
    })?
}

impl Drop for BookMath {
    fn drop(&mut self) {
        if !self.token.is_empty() {
            FORMULAS
                .write()
                .unwrap()
                .retain(|key, _| !key.starts_with(&self.token));
        }
    }
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

struct Atomic {
    key: String,
    start: usize,
    len: usize,
    layout: Rc<LaidoutText>,
    hidden: bool,
    center: f64,
    display: bool,
}
#[derive(Default)]
struct Capture {
    areas: HashMap<String, Area>,
    text: Vec<Atomic>,
}
thread_local! {static CAPTURE:RefCell<Capture>=RefCell::new(Capture::default());}
pub fn begin_capture() {
    CAPTURE.with(|c| *c.borrow_mut() = Capture::default());
}
pub fn finish_capture() {
    let capture = CAPTURE.with(|c| std::mem::take(&mut *c.borrow_mut()));
    for a in capture.text {
        if let Some(area) = capture.areas.get(&a.key) {
            crate::reader_hit::capture_atomic(
                a.layout, *area, a.start, a.len, a.hidden, a.center, a.display,
            );
        }
    }
}
script_mod! {
    use mod.prelude.widgets.*
    mod.widgets.ReaderMathBase = #(ReaderMath::register_widget(vm))
    mod.widgets.ReaderMath = set_type_default() do mod.widgets.ReaderMathBase {draw_svg +: {..mod.draw.DrawSvg}}
    mod.widgets.ReaderMathHidden = set_type_default() do mod.widgets.ReaderMathBase {hidden: true draw_svg +: {..mod.draw.DrawSvg}}
}
#[derive(Script, WidgetRegister)]
pub struct ReaderMath {
    #[uid]
    uid: WidgetUid,
    #[source]
    source: ScriptObjectRef,
    #[live]
    draw_svg: DrawSvg,
    #[live]
    hidden: bool,
    #[live]
    draw_image: DrawImage,
    #[rust]
    raster_texture: Option<Texture>,
    #[rust]
    press_origin: Option<DVec2>,
    #[rust]
    area: Area,
    #[rust]
    key: String,
    #[rust]
    text: String,
    #[rust]
    formula: Option<Arc<Visual>>,
    #[rust]
    tessellation_size: DVec2,
}

// DrawSvg normally tessellates at the SVG's intrinsic (PDF point) size, then
// scales the mesh, including its physical AA fringe, to the reader's em size.
// A 1-unit fringe around a tiny script can become several screen pixels and
// visually embolden it. Tessellate at device resolution instead; only the
// final GPU transform converts device pixels to logical pixels. The Typst
// viewBox, layout rect, source text and hit/selection geometry stay unchanged.
fn prepare_math_svg(draw: &mut DrawSvg, size: DVec2, dpi: f64, cached: &mut DVec2) {
    let pixels = size * dpi;
    if *cached == pixels {
        return;
    }
    if let Some(doc) = draw.svg_doc.as_mut() {
        set_math_svg_viewport(doc, pixels);
        draw.content_bounds = (0., 0., pixels.x as f32, pixels.y as f32);
        draw.cache_valid = false;
        *cached = pixels;
    }
}
fn set_math_svg_viewport(doc: &mut makepad_widgets::makepad_draw::svg::SvgDocument, pixels: DVec2) {
    doc.width = Some(pixels.x as f32);
    doc.height = Some(pixels.y as f32);
}

impl ScriptHook for ReaderMath {
    fn on_after_new_scoped(&mut self, _vm: &mut ScriptVm, scope: &mut Scope) {
        if let Some(doc) = scope.props.get::<HtmlDoc>() {
            let mut walker = doc.new_walker_with_index(scope.index + 1);
            while let Some((lc, attr)) = walker.while_attr_lc() {
                if lc == live_id!(m) {
                    self.key = attr.into();
                    break;
                }
            }
        }
    }
}
impl WidgetNode for ReaderMath {
    fn widget_uid(&self) -> WidgetUid {
        self.uid
    }
    fn walk(&mut self, _cx: &mut Cx) -> Walk {
        Walk::default()
    }
    fn area(&self) -> Area {
        self.area
    }
    fn redraw(&mut self, cx: &mut Cx) {
        self.area.redraw(cx)
    }
}
impl Widget for ReaderMath {
    fn set_text(&mut self, _cx: &mut Cx, text: &str) {
        self.text = text.into();
    }
    fn text(&self) -> String {
        self.text.clone()
    }
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, _scope: &mut Scope) {
        let Some(visual) = &self.formula else { return };
        let Visual::Figure(figure) = visual.as_ref() else {
            return;
        };
        if self.hidden {
            return;
        }
        match event.hits(cx, self.area) {
            Hit::FingerDown(e) => self.press_origin = Some(e.abs),
            Hit::FingerMove(e) => {
                if self.press_origin.is_some_and(|p| (p - e.abs).length() > 8.) {
                    self.press_origin = None;
                }
            }
            Hit::FingerUp(e) => {
                if self
                    .press_origin
                    .take()
                    .is_some_and(|p| e.is_over && e.was_tap() && (p - e.abs).length() <= 8.)
                {
                    cx.widget_action(
                        self.uid,
                        crate::chapter_image::ChapterImageAction::OpenData(figure.png.clone()),
                    );
                }
            }
            Hit::FingerHoverIn(_) | Hit::FingerHoverOver(_) => cx.set_cursor(MouseCursor::Hand),
            _ => {}
        }
    }
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, _walk: Walk) -> DrawStep {
        let Some(tf) = scope.data.get_mut::<TextFlow>() else {
            return DrawStep::done();
        };
        if self.formula.is_none() {
            self.formula = FORMULAS.read().unwrap().get(&self.key).cloned();
            if !self.hidden {
                match self.formula.as_deref() {
                    Some(Visual::Formula(f)) => {
                        self.draw_svg.load_from_str(&f.svg);
                        if let Some(doc) = &self.draw_svg.svg_doc {
                            let (w, h) = doc.logical_size();
                            self.draw_svg.content_bounds = (0., 0., w, h);
                        }
                    }
                    Some(Visual::Figure(f)) => {
                        self.raster_texture = ImageBuffer::from_png(&f.png)
                            .ok()
                            .map(|b| b.into_new_texture(cx));
                    }
                    None => {}
                }
            }
        }
        let Some(visual) = &self.formula else {
            crate::reader_hit::draw_text(tf, cx, &self.text);
            return DrawStep::done();
        };
        if !self.hidden
            && matches!(visual.as_ref(), Visual::Figure(_))
            && self.raster_texture.is_none()
        {
            crate::reader_hit::draw_text(tf, cx, &self.text);
            return DrawStep::done();
        }
        let old = tf.draw_text.text_style.clone();
        tf.draw_text.text_style = tf.text_style_normal.clone();
        tf.draw_text.text_style.font_size = *tf.font_sizes.last().unwrap_or(&tf.font_size);
        let probe = tf
            .draw_text
            .layout(cx, 0., 0., None, false, Align::default(), "M");
        tf.draw_text.text_style = old;
        let Some(row) = probe.rows.first() else {
            return DrawStep::done();
        };
        let scale = tf.draw_text.font_scale as f64;
        let em = row
            .glyphs
            .first()
            .map_or(tf.font_size as f64, |g| g.font_size_in_lpxs as f64)
            * scale;
        let asc = row.ascender_in_lpxs as f64 * scale;
        let desc = -row.descender_in_lpxs as f64 * scale;
        let available = cx.turtle().inner_rect().size.x.max(1.);
        let (width_em, height_em, align_em, display) = match visual.as_ref() {
            Visual::Formula(f) => (f.width_em, f.height_em, f.align_em, f.display),
            Visual::Figure(f) => {
                let size = crate::chapter_image::inline_image_display_size(
                    (f.width as usize, f.height as usize),
                    available,
                    cx.current_dpi_factor(),
                );
                (size.x / em, size.y / em, 0., true)
            }
        };
        let em = em * (available / (width_em * em)).min(1.);
        if !self.hidden {
            if display {
                tf.new_line_collapsed(cx);
            }
            let width = width_em * em;
            let height = height_em * em;
            let math_asc = (height_em + align_em) * em;
            let math_desc = -align_em * em;
            if !display && width > cx.turtle().unused_inner_width_for_current_row() {
                tf.new_line_collapsed(cx);
            }
            if !display {
                cx.reserve_turtle_row_top((math_asc - asc).max(0.));
            }
            let walk = Walk {
                width: Size::Fixed(width),
                height: Size::Fixed(if display {
                    height
                } else {
                    asc + desc.max(math_desc)
                }),
                metrics: Metrics {
                    descender: if display { 0. } else { desc.max(math_desc) },
                    line_gap: 0.,
                    line_scale: 1.,
                },
                margin: if display {
                    Inset {
                        left: (available - width) * 0.5,
                        right: (available - width) * 0.5,
                        top: em * 0.25,
                        bottom: em * 0.25,
                    }
                } else {
                    Inset::default()
                },
                ..Walk::default()
            };
            let mut rect = cx.walk_turtle(walk);
            if !display {
                rect.pos.y += asc - math_asc;
                rect.size.y = height;
            }
            if let Some(texture) = &self.raster_texture {
                self.draw_image.draw_vars.set_texture(0, texture);
                self.draw_image.draw_abs(cx, rect);
            } else {
                prepare_math_svg(
                    &mut self.draw_svg,
                    rect.size,
                    cx.current_dpi_factor(),
                    &mut self.tessellation_size,
                );
                self.draw_svg.color = *tf.font_colors.last().unwrap_or(&tf.font_color);
                self.draw_svg.draw_abs(cx, rect);
            }
            cx.add_aligned_rect_area(&mut self.area, rect);
            tf.areas_tracker.track_rect(cx, rect);
            CAPTURE.with(|c| {
                c.borrow_mut().areas.insert(self.key.clone(), self.area);
            });
            tf.first_thing_on_a_line = false;
        }
        let start = tf.text_len();
        // No source glyphs are repainted: widget text is the original UTF-8
        // selection segment. Actual atomic areas are captured after alignment.
        tf.push_widget_text_for_selection(WidgetRef::empty(), &self.text);
        CAPTURE.with(|c| {
            c.borrow_mut().text.push(Atomic {
                key: self.key.clone(),
                start,
                len: self.text.len(),
                layout: probe,
                hidden: self.hidden,
                center: (height_em + align_em) * em - (asc - desc) * 0.5,
                display,
            })
        });
        if display && !self.hidden {
            tf.new_line_collapsed(cx);
        }
        DrawStep::done()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn imported_first_page_reopens_without_waiting_for_a_speculative_compile() {
        let root =
            std::env::temp_dir().join(format!("readwise-visual-import-{}", uuid::Uuid::new_v4()));
        let db = readwise_core::Db::open_in(root.clone()).unwrap();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../web/tests/fixtures/math-scattered.pdf");
        let book = db.import_book(&source).unwrap();
        prepare_import(&db, &book.id, &book.file_path);
        let path = root.join(&book.file_path);
        let raw = db.load_book_chapters(&book.id, &path).unwrap().chapters[0]
            .html
            .clone();
        let lock = COMPILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut next = BookMath::new(&path.to_string_lossy(), &book.id, 99);
            next.prepare_initial(0, &raw);
            tx.send(next.pages[0].1.bytes).unwrap();
        });
        let bytes = rx.recv_timeout(std::time::Duration::from_secs(2));
        drop(lock);
        worker.join().unwrap();
        assert!(bytes.expect("cache hit must bypass the global compilation lock") > 0);
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn both_import_workers_prepare_the_initial_pdf_presentation_before_publishing() {
        let source = include_str!("app.rs");
        assert_eq!(
            source
                .matches("crate::reader_math::prepare_import(db, &book.id, &book.file_path)")
                .count(),
            2
        );
        let worker = source
            .split("fn spawn_open_book(")
            .nth(1)
            .unwrap()
            .split("fn build_chapter_chunks(")
            .next()
            .unwrap();
        assert!(
            worker
                .matches("OPEN_BOOK_REQUEST_ID.load(Ordering::Acquire) != request_id")
                .count()
                >= 3
        );
    }

    fn fixture() -> (BookMath, Vec<String>) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../web/tests/fixtures/math-scattered.pdf");
        let loaded = readwise_core::books::pdf::load_pages(&path).unwrap();
        let raw = &loaded.chapters[0].html;
        let blocks = readwise_core::books::epub_meta::split_blocks(raw);
        let mut math = BookMath::new(path.to_str().unwrap(), &uuid::Uuid::new_v4().to_string(), 1);
        math.prepare_initial(0, raw);
        (math, blocks)
    }
    #[test]
    fn vector_figure_is_one_native_image_with_unchanged_atomic_source_segments() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../web/tests/fixtures/vector-figure.pdf");
        let loaded = readwise_core::books::pdf::load_pages(&path).unwrap();
        let raw = &loaded.chapters[0].html;
        let blocks = readwise_core::books::epub_meta::split_blocks(raw);
        let mut book = BookMath::new(path.to_str().unwrap(), &uuid::Uuid::new_v4().to_string(), 1);
        book.prepare_initial(0, raw);
        let page = &book.pages[0].1;
        let mut images = 0;
        let mut display = Vec::new();
        for (i, html) in blocks.iter().enumerate() {
            let projected = book.project(page, 0, i, html, str::to_owned);
            images += projected.matches("<reader_math ").count();
            let tracked = crate::reader_hit::tracked_html(&projected).unwrap();
            assert_eq!(
                readwise_core::vocab::annotate::rendered_text(&tracked),
                readwise_core::vocab::annotate::rendered_text(html)
            );
            let text = readwise_core::vocab::annotate::rendered_text(html);
            if text.contains("Figure 1:") || text.contains("Diagram overview") {
                assert!(
                    tracked.contains("<reader_run>"),
                    "surrounding prose must remain native text"
                );
                assert!(!is_figure_source(&projected));
            }
            display.push(projected);
        }
        assert_eq!(images, 1);
        let owner = display
            .iter()
            .position(|b| b.contains("<reader_math m="))
            .unwrap();
        for block in 0..display.len() {
            if is_figure_source(&display[block]) {
                assert_eq!(figure_owner_block(&display, block), Some(owner));
            }
        }
        assert!(!is_figure_source(
            "<reader_math_hidden m=\"book/0/math-1\">x</reader_math_hidden>"
        ));
        let metrics = display
            .iter()
            .enumerate()
            .map(|(i, html)| {
                Some(crate::reader_columns::BlockMetrics {
                    height: if is_figure_source(html) {
                        0.
                    } else if i == owner {
                        400.
                    } else {
                        40.
                    },
                    ..Default::default()
                })
            })
            .collect::<Vec<_>>();
        let composed =
            crate::reader_columns::compose_page(&metrics, Default::default(), 800.).unwrap();
        assert_eq!(
            composed.next.block,
            display.len(),
            "source labels cannot create an extra blank page"
        );
        let registry = FORMULAS.read().unwrap();
        assert_eq!(
            registry
                .iter()
                .filter(
                    |(k, v)| k.starts_with(&book.token) && matches!(v.as_ref(), Visual::Figure(_))
                )
                .count(),
            1
        );
    }

    #[test]
    fn native_math_tessellates_at_device_resolution_without_scaling_ink_or_source() {
        let svg = "<svg width=\"12\" height=\"8\" viewBox=\"0 0 12 8\"><path d=\"M1 1L11 1L11 7L1 7Z\"/></svg>";
        let mut doc = makepad_widgets::makepad_draw::svg::parse_svg(svg);
        let original = format!("{:?}{:?}", doc.viewbox, doc.root);
        for dpi in [1., 1.25, 2., 3.] {
            let logical = dvec2(36., 24.);
            set_math_svg_viewport(&mut doc, logical * dpi);
            let (w, h) = doc.logical_size();
            assert_eq!((w as f64 / dpi, h as f64 / dpi), (logical.x, logical.y));
            assert_eq!(format!("{:?}{:?}", doc.viewbox, doc.root), original);
        }
    }

    #[test]
    fn native_projection_uses_typst_vectors_without_changing_source_or_hit_offsets() {
        assert_eq!(
            include_bytes!("../resources/third-party/typst-assets/LICENSE"),
            include_bytes!("../../web/third-party/typst-assets/LICENSE")
        );
        assert_eq!(
            include_bytes!("../resources/third-party/typst-assets/NOTICE"),
            include_bytes!("../../web/third-party/typst-assets/NOTICE")
        );
        let (math, blocks) = fixture();
        let page = &math.pages[0].1;
        let mut count = 0;
        for (i, html) in blocks.iter().enumerate() {
            let projected = math.project(page, 0, i, html, str::to_owned);
            count += projected.matches("<reader_math ").count();
            assert!(!projected.contains("<svg"));
            assert_eq!(
                readwise_core::vocab::annotate::rendered_text(&projected),
                readwise_core::vocab::annotate::rendered_text(html)
            );
            let tracked = crate::reader_hit::tracked_html(&projected).unwrap();
            assert_eq!(
                readwise_core::vocab::annotate::rendered_text(&tracked),
                readwise_core::vocab::annotate::rendered_text(html)
            );
            for tag in ["reader_math", "reader_math_hidden"] {
                for part in tracked.split(&format!("<{tag} ")).skip(1) {
                    let inner = part
                        .split_once('>')
                        .unwrap()
                        .1
                        .split_once(&format!("</{tag}>"))
                        .unwrap()
                        .0;
                    assert!(
                        !inner.contains("<reader_run>"),
                        "formula source must remain an atomic native selection segment"
                    );
                }
            }
        }
        assert!(count >= 4);
        let registry = FORMULAS.read().unwrap();
        let formulas = registry
            .iter()
            .filter(|(k, _)| k.starts_with(&math.token))
            .filter_map(|(key, visual)| match visual.as_ref() {
                Visual::Formula(formula) => Some((key, formula)),
                Visual::Figure(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(formulas.len(), count);
        assert!(
            formulas
                .iter()
                .any(|(_, f)| f.svg.contains("stroke=\"currentColor\"")),
            "the small-font fraction retains its original overhanging PDF rule"
        );
        assert!(formulas.iter().all(|(_, f)| f.svg.contains("<path")
            && !f.svg.contains("<text")
            && f.width_em > 0.
            && f.height_em > 0.));
        drop(registry);
        let token = math.token.clone();
        drop(math);
        assert!(!FORMULAS
            .read()
            .unwrap()
            .keys()
            .any(|k| k.starts_with(&token)));
    }
    #[test]
    fn warming_is_adjacent_bounded_and_yields_to_the_latest_navigation() {
        let mut math = BookMath::new("book.pdf", "warm-test", 1);
        assert_eq!(math.next_neighbor(1000), Some(1));
        math.warmed.insert(1);
        assert_eq!(math.next_neighbor(1000), None);
        math.current = 500;
        math.warmed.clear();
        assert_eq!(math.next_neighbor(1000), Some(501));
        math.pending.insert(501);
        assert_eq!(
            math.next_neighbor(1000),
            None,
            "only one compiler at a time"
        );
        math.pending.clear();
        math.warmed.insert(501);
        assert_eq!(math.next_neighbor(1000), Some(499));
        math.navigation = Some((900, 0, 0., true));
        assert_eq!(math.next_neighbor(1000), None, "foreground jump wins");
        math.cancel_navigation();
        math.warmed.insert(499);
        assert_eq!(
            math.next_neighbor(1000),
            None,
            "do not recompile evicted speculative pages"
        );
        math.current = 999;
        math.warmed.clear();
        assert_eq!(math.next_neighbor(1000), Some(998));
        let epub = BookMath::new("book.epub", "epub", 1);
        assert_eq!(epub.next_neighbor(1000), None);
    }

    #[test]
    fn native_page_cache_rejects_stale_jobs_protects_current_geometry_and_bounds_residency() {
        let (mut math, blocks) = fixture();
        let session = math.session.clone().unwrap();
        let page = math.pages[0].1.clone();
        math.project(&page, 0, 1, &blocks[1], str::to_owned);
        math.pending.insert(1);
        assert!(math
            .accept(&Ready {
                token: format!("{}stale", math.token),
                chapter: 1,
                page: page.clone()
            })
            .is_none());
        assert!(math.pending.contains(&1));
        for chapter in 1..=8 {
            math.pending.insert(chapter);
            assert!(math
                .accept(&Ready {
                    token: math.token.clone(),
                    chapter,
                    page: page.clone()
                })
                .is_some());
            assert!(math.pages.len() <= 6);
            assert!(math.pages.iter().any(|(i, _)| *i == 0));
        }
        assert!(!math.pages.iter().any(|(i, _)| *i == 1));
        assert!(
            !math.take_pagination_change(1),
            "evicted speculation does not change book geometry"
        );
        assert!(math.take_pagination_change(8));
        assert!(
            !math.take_pagination_change(8),
            "consume only on actual display, once"
        );
        assert!(
            Arc::ptr_eq(&session, math.session.as_ref().unwrap()),
            "page eviction must not discard the book's font/parser cache"
        );
        assert!(FORMULAS
            .read()
            .unwrap()
            .keys()
            .any(|k| k.starts_with(&format!("{}0/", math.token))));
        math.navigation = Some((8, 3, 4.5, false));
        assert!(math.take_navigation(7).is_none());
        assert_eq!(math.take_navigation(8), Some((8, 3, 4.5, false)));
        assert!(!math.defer_navigation(8, "", 3, 4.5, false));
        assert_eq!(math.current, 8);
        math.navigation = Some((7, 0, 0., true));
        math.cancel_navigation();
        assert_eq!(math.waiting_navigation(), None);
    }
}
