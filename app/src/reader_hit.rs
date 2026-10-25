//! Hit testing from the same per-run layout used to paint reader text.
//! TextFlow's bounding-box hit includes the empty part of a wrapped run;
//! another run on that visual row must not be hidden behind that empty box.
use makepad_widgets::makepad_draw::text::layouter::LaidoutText;
use makepad_widgets::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone)]
pub struct HitRun {
    layout: Rc<LaidoutText>,
    origin: DVec2,
    scale: f64,
    start: usize,
    // Inline code may be explicitly moved to a new line by TextFlow. Each
    // painted row then has its own origin instead of the pre-draw run origin.
    row: Option<usize>,
    /// Resolved TextFlow row areas after Makepad's per-row alignment.
    drawn_rows: Vec<(usize, Rect)>,
    ink_rows: Vec<(usize, Rect)>,
    atomic: Option<AtomicGeometry>,
}
#[derive(Clone, Copy)]
struct AtomicGeometry {
    len: usize,
    hidden: bool,
    center: f64,
    display: bool,
}

struct PendingRun {
    run: HitRun,
    painted_row: Option<(Area, Inset)>,
    row_areas: Vec<(Area, DVec2)>,
    ink_rows: Vec<Option<Rect>>,
}

thread_local! {
    static RUNS: RefCell<Option<Vec<PendingRun>>> = const { RefCell::new(None) };
    static OVERLAYS: RefCell<Option<Vec<(Area, DVec2, Rect, usize)>>> = const { RefCell::new(None) };
    static CAPTURE_INK: Cell<bool> = const { Cell::new(false) };
}

pub fn begin_capture(measure: bool) {
    RUNS.with(|runs| *runs.borrow_mut() = Some(Vec::new()));
    OVERLAYS.with(|overlays| *overlays.borrow_mut() = Some(Vec::new()));
    CAPTURE_INK.with(|capture| capture.set(measure));
}

pub fn is_capturing() -> bool {
    RUNS.with(|runs| runs.borrow().is_some())
}

pub fn cancel_capture() {
    RUNS.with(|runs| *runs.borrow_mut() = None);
    OVERLAYS.with(|overlays| *overlays.borrow_mut() = None);
    CAPTURE_INK.with(|capture| capture.set(false));
}

pub fn capture_overlay(cx: &Cx, anchor: Area, rect: Rect, source: usize) {
    OVERLAYS.with(|overlays| {
        if let Some(overlays) = overlays.borrow_mut().as_mut() {
            overlays.push((anchor, anchor.rect(cx).pos, rect, source));
        }
    });
}

pub fn end_capture(cx: &Cx, origin: DVec2) -> (Vec<HitRun>, Vec<(Rect, usize)>) {
    CAPTURE_INK.with(|capture| capture.set(false));
    let runs = RUNS
        .with(|runs| runs.borrow_mut().take().unwrap_or_default())
        .into_iter()
        .filter_map(|pending| {
            let mut run = pending.run;
            if pending.ink_rows.len() == run.layout.rows.len() {
                run.ink_rows = pending
                    .ink_rows
                    .iter()
                    .enumerate()
                    .filter_map(|(index, ink)| {
                        let mut rect = (*ink)?;
                        if let Some((area, original_pos)) = pending.row_areas.get(index) {
                            rect.pos += area.rect(cx).pos - *original_pos;
                        }
                        rect.pos -= origin;
                        Some((index, rect))
                    })
                    .collect();
            }
            if pending.row_areas.len() == run.layout.rows.len() {
                run.drawn_rows = pending
                    .row_areas
                    .iter()
                    .enumerate()
                    .map(|(index, (area, _))| {
                        let mut rect = area.rect(cx);
                        rect.pos -= origin;
                        (index, rect)
                    })
                    .collect();
            }
            if let Some((area, padding)) = pending.painted_row {
                // Resolve after parent row alignment, just like inline-code
                // highlight geometry. No Area survives in the hit cache.
                let row = &run.layout.rows[run.row?];
                let first = row.glyphs.first()?;
                run.origin = area.rect(cx).pos + padding.left_top()
                    - dvec2(
                        first.origin_in_lpxs.x as f64 * run.scale,
                        (row.origin_in_lpxs.y - row.ascender_in_lpxs) as f64 * run.scale,
                    );
                let mut rect = area.rect(cx);
                rect.pos += padding.left_top() - origin;
                rect.size -= padding.size();
                run.drawn_rows.push((run.row?, rect));
            }
            run.origin -= origin;
            Some(run)
        })
        .collect();
    let overlays = OVERLAYS
        .with(|overlays| overlays.borrow_mut().take().unwrap_or_default())
        .into_iter()
        .map(|(anchor, original_pos, mut rect, source)| {
            rect.pos += anchor.rect(cx).pos - original_pos - origin;
            (rect, source)
        })
        .collect();
    (runs, overlays)
}

/// Visible glyph bounds after row alignment. TextFlow's tracked row area is a
/// line box: some fonts paint their glyphs well above that box, so its gaps
/// are not necessarily safe page cuts.
pub fn painted_rows(runs: &[HitRun]) -> Vec<(Rect, usize)> {
    runs.iter()
        .flat_map(|run| {
            if let Some(math) = run.atomic {
                return if math.hidden {
                    Vec::new()
                } else {
                    run.drawn_rows
                        .iter()
                        .map(|(_, rect)| (*rect, run.start))
                        .collect()
                };
            }
            let rows = if run.ink_rows.is_empty() {
                &run.drawn_rows
            } else {
                &run.ink_rows
            };
            if !rows.is_empty() {
                return rows
                    .iter()
                    .filter_map(|(index, rect)| {
                        let row = run.layout.rows.get(*index)?;
                        (!row.glyphs.is_empty())
                            .then_some((*rect, run.start + row.text.start_in_parent()))
                    })
                    .collect::<Vec<_>>();
            }
            run.layout
                .rows
                .iter()
                .enumerate()
                .filter_map(|(index, row)| {
                    if run.row.is_some_and(|painted_row| painted_row != index) {
                        return None;
                    }
                    let first = row.glyphs.first()?;
                    let left = run.origin.x + first.origin_in_lpxs.x as f64 * run.scale;
                    let right = run.origin.x + row.width_in_lpxs as f64 * run.scale;
                    let baseline = run.origin.y + row.origin_in_lpxs.y as f64 * run.scale;
                    let top = baseline - row.ascender_in_lpxs as f64 * run.scale;
                    let bottom = baseline - row.descender_in_lpxs as f64 * run.scale;
                    Some((
                        Rect {
                            pos: dvec2(left, top),
                            size: dvec2((right - left).max(0.0), (bottom - top).max(0.0)),
                        },
                        run.start + row.text.start_in_parent(),
                    ))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

pub fn capture_code_rows(
    layout: Rc<LaidoutText>,
    areas: &[Area],
    padding: Inset,
    scale: f32,
    start: usize,
) {
    if layout.rows.len() != areas.len() {
        // Never advertise an incomplete set of runs as a whole paragraph.
        cancel_capture();
        return;
    }
    RUNS.with(|runs| {
        if let Some(runs) = runs.borrow_mut().as_mut() {
            for (row, area) in areas.iter().copied().enumerate() {
                runs.push(PendingRun {
                    run: HitRun {
                        layout: layout.clone(),
                        origin: DVec2::default(),
                        scale: scale as f64,
                        start,
                        row: Some(row),
                        drawn_rows: Vec::new(),
                        ink_rows: Vec::new(),
                        atomic: None,
                    },
                    painted_row: Some((area, padding)),
                    row_areas: Vec::new(),
                    ink_rows: Vec::new(),
                });
            }
        }
    });
}

/// The actual TextFlow draw remains authoritative. Reusing its cached layout
/// afterwards records glyph/UTF-8 boundaries without reflowing the paragraph.
pub fn draw_text(tf: &mut TextFlow, cx: &mut Cx2d, text: &str) {
    let capture = tf.selectable && is_capturing();
    let pos = cx.turtle().pos();
    let rect = cx.turtle().inner_rect();
    let row_height = cx.turtle().next_row_offset() as f32;
    let wrap = matches!(cx.turtle().layout().flow, Flow::Right { wrap: true, .. });
    let start = tf.text_len();
    let area_start = tf.areas_tracker.areas.len();
    let capture_ink = capture && CAPTURE_INK.with(Cell::get);
    if capture {
        tf.draw_text.captured_row_areas = Some(Vec::new());
    }
    if capture_ink {
        tf.draw_text.captured_ink_rows = Some(Vec::new());
    }
    tf.draw_text(cx, text);
    let painted_areas = if capture {
        tf.draw_text.captured_row_areas.take().unwrap_or_default()
    } else {
        Vec::new()
    };
    let ink_rows = if capture_ink {
        tf.draw_text.captured_ink_rows.take().unwrap_or_default()
    } else {
        Vec::new()
    };
    if !capture || tf.text_len() == start {
        return;
    }
    // Use the exact text TextFlow accepted (including its whitespace trimming).
    let text = tf.get_text_for_range(start, tf.text_len());
    let dt = &tf.draw_text;
    let layout = dt.layout(
        cx,
        (pos.x - rect.pos.x) as f32,
        row_height,
        (!rect.size.x.is_nan()).then_some(rect.size.x as f32),
        wrap,
        dt.layout_align,
        &text,
    );
    let shift = layout
        .rows
        .first()
        .and_then(|row| row.glyphs.first())
        .map(|glyph| glyph.font_size_in_lpxs * dt.temp_y_shift)
        .unwrap_or(0.0);
    let areas: &[Area] = if painted_areas.is_empty() {
        &tf.areas_tracker.areas[area_start..]
    } else {
        &painted_areas
    };
    let row_areas = areas
        .iter()
        .map(|area| (*area, area.rect(cx).pos))
        .collect();
    RUNS.with(|runs| {
        if let Some(runs) = runs.borrow_mut().as_mut() {
            runs.push(PendingRun {
                run: HitRun {
                    layout,
                    origin: dvec2(rect.pos.x, pos.y + shift as f64),
                    scale: dt.font_scale as f64,
                    start,
                    row: None,
                    drawn_rows: Vec::new(),
                    ink_rows: Vec::new(),
                    atomic: None,
                },
                painted_row: None,
                row_areas,
                ink_rows,
            });
        }
    });
}

pub fn capture_atomic(
    layout: Rc<LaidoutText>,
    area: Area,
    start: usize,
    len: usize,
    hidden: bool,
    center: f64,
    display: bool,
) {
    RUNS.with(|runs| {
        if let Some(runs) = runs.borrow_mut().as_mut() {
            runs.push(PendingRun {
                run: HitRun {
                    layout,
                    origin: DVec2::default(),
                    scale: 1.,
                    start,
                    row: None,
                    drawn_rows: Vec::new(),
                    ink_rows: Vec::new(),
                    atomic: Some(AtomicGeometry {
                        len,
                        hidden,
                        center,
                        display,
                    }),
                },
                painted_row: None,
                row_areas: vec![(area, DVec2::default())],
                ink_rows: Vec::new(),
            });
        }
    });
}

fn axis_distance(value: f64, low: f64, high: f64) -> f64 {
    (low - value).max(value - high).max(0.0)
}

pub fn contains_row(rect: Rect, y: f64) -> bool {
    rect.size.y > 0.0 && y >= rect.pos.y && y < rect.pos.y + rect.size.y
}

fn nearest_row<T>(rows: impl Iterator<Item = (Rect, T)>, point: DVec2) -> Option<T> {
    let mut best: Option<(f64, f64, T)> = None;
    for (rect, value) in rows {
        let dy = axis_distance(point.y, rect.pos.y, rect.pos.y + rect.size.y);
        let dx = axis_distance(point.x, rect.pos.x, rect.pos.x + rect.size.x);
        // Prefer the visual row, then the nearest occupied run on it.
        if best
            .as_ref()
            .is_none_or(|(by, bx, _)| dy < by - 0.5 || ((dy - by).abs() <= 0.5 && dx < *bx))
        {
            best = Some((dy, dx, value));
        }
    }
    best.map(|(_, _, value)| value)
}

pub fn hit_index(runs: &[HitRun], point: DVec2) -> Option<usize> {
    hit_text_index(runs, point, false)
}

/// Word gestures need the glyph under the pointer, not the nearest caret:
/// the right half of a word's last letter must still select that word.
pub fn hit_word_index(runs: &[HitRun], point: DVec2) -> Option<usize> {
    hit_text_index(runs, point, true)
}

fn grapheme_at_x(text: &str, left: f32, right: f32, x: f32) -> Option<usize> {
    if x < left || x >= right {
        return None;
    }
    let count = text.graphemes(true).count();
    let index = ((x - left) / (right - left) * count as f32) as usize;
    text.grapheme_indices(true)
        .nth(index)
        .map(|(index, _)| index)
}

fn hit_text_index(runs: &[HitRun], point: DVec2, word: bool) -> Option<usize> {
    let rows = runs.iter().flat_map(|run| {
        run.layout
            .rows
            .iter()
            .enumerate()
            .filter_map(move |(index, row)| {
                if run.atomic.is_some_and(|math| math.hidden)
                    || run.row.is_some_and(|painted_row| painted_row != index)
                {
                    return None;
                }
                let rect = run.row_rect(index, row)?;
                Some((rect, (run, index, row)))
            })
    });
    let (run, index, row) = nearest_row(rows, point)?;
    if let Some(math) = run.atomic {
        let rect = run.row_rect(index, row)?;
        return (!word).then_some(
            run.start
                + if point.x < rect.pos.x + rect.size.x * 0.5 {
                    0
                } else {
                    math.len
                },
        );
    }
    let x = run.row_x_in_lpxs(index, row, point.x);
    if word {
        let mut groups = row
            .glyphs
            .chunk_by(|a, b| a.cluster == b.cluster)
            .peekable();
        while let Some(group) = groups.next() {
            let start = group[0].cluster;
            let end = groups.peek().map_or(row.text.len(), |next| next[0].cluster);
            let left = group[0].origin_in_lpxs.x;
            let right = groups
                .peek()
                .map_or(row.width_in_lpxs, |next| next[0].origin_in_lpxs.x);
            if let Some(index) = grapheme_at_x(&row.text[start..end], left, right, x) {
                return Some(run.start + row.text.start_in_parent() + start + index);
            }
        }
        return None;
    }
    Some(run.start + row.text.start_in_parent() + row.x_in_lpxs_to_index(x))
}

impl HitRun {
    fn row_start_x(
        row: &makepad_widgets::makepad_draw::text::layouter::LaidoutRow,
        index: usize,
    ) -> f64 {
        if index == 0 {
            row.glyphs
                .first()
                .map_or(0.0, |glyph| glyph.origin_in_lpxs.x as f64)
        } else {
            0.0
        }
    }

    fn row_rect(
        &self,
        index: usize,
        row: &makepad_widgets::makepad_draw::text::layouter::LaidoutRow,
    ) -> Option<Rect> {
        if let Some((_, rect)) = self.drawn_rows.iter().find(|(row, _)| *row == index) {
            return Some(*rect);
        }
        let first = row.glyphs.first()?;
        let left = self.origin.x + first.origin_in_lpxs.x as f64 * self.scale;
        let right = self.origin.x + row.width_in_lpxs as f64 * self.scale;
        let baseline = self.origin.y + row.origin_in_lpxs.y as f64 * self.scale;
        let top = baseline - row.ascender_in_lpxs as f64 * self.scale;
        let bottom = baseline - row.descender_in_lpxs as f64 * self.scale;
        Some(Rect {
            pos: dvec2(left, top),
            size: dvec2(right - left, bottom - top),
        })
    }

    fn row_x_in_lpxs(
        &self,
        index: usize,
        row: &makepad_widgets::makepad_draw::text::layouter::LaidoutRow,
        x: f64,
    ) -> f32 {
        let left = self
            .row_rect(index, row)
            .map_or(self.origin.x, |rect| rect.pos.x);
        painted_row_local_x(left, Self::row_start_x(row, index), self.scale, x)
    }
}

fn painted_row_local_x(left: f64, row_start: f64, scale: f64, x: f64) -> f32 {
    ((x - left) / scale + row_start) as f32
}

fn painted_row_screen_x(left: f64, row_start: f64, scale: f64, x: f32) -> f64 {
    left + (x as f64 - row_start) * scale
}

/// TextFlow records selection against the layout's predicted rows. On macOS
/// its multi-row draw follows the turtle's *actual* row positions instead.
/// Rebuild tracked rich-text rectangles from the painted rows so a long block
/// cannot accumulate a vertical offset between text, hit testing and color.
pub fn selection_rects(runs: &[HitRun], from: usize, to: usize) -> Vec<Rect> {
    let mut rects = Vec::new();
    let mut formulas = Vec::new();
    for run in runs {
        if let Some(math) = run.atomic {
            if from < run.start + math.len && to > run.start {
                if let Some((_, rect)) = run.drawn_rows.first() {
                    let height = run.layout.rows.first().map_or(rect.size.y, |row| {
                        (row.ascender_in_lpxs - row.descender_in_lpxs) as f64 * run.scale
                    });
                    formulas.push(MathSelection {
                        rect: Rect {
                            pos: rect.pos - dvec2(0., 2.),
                            size: rect.size + dvec2(0., 4.),
                        },
                        center: rect.pos.y + math.center,
                        tolerance: height * 0.45,
                        display: math.display,
                    });
                }
            }
            continue;
        }
        for (index, row) in run.layout.rows.iter().enumerate() {
            if run.row.is_some_and(|painted| painted != index) {
                continue;
            }
            let start = run.start + row.text.start_in_parent();
            let end = start + row.text.len();
            let a = from.max(start);
            let b = to.min(end);
            if a >= b {
                continue;
            }
            let Some(actual) = run.row_rect(index, row) else {
                continue;
            };
            let row_x = HitRun::row_start_x(row, index);
            let left = painted_row_screen_x(
                actual.pos.x,
                row_x,
                run.scale,
                row.index_to_x_in_lpxs(a - start),
            );
            let right = painted_row_screen_x(
                actual.pos.x,
                row_x,
                run.scale,
                row.index_to_x_in_lpxs(b - start),
            );
            if right > left {
                rects.push(Rect {
                    pos: dvec2(left, actual.pos.y - 2.0),
                    size: dvec2(right - left, actual.size.y + 4.0),
                });
            }
        }
    }
    if formulas.is_empty() {
        rects
    } else {
        math_selection_rows(rects, formulas)
    }
}

struct MathSelection {
    rect: Rect,
    center: f64,
    tolerance: f64,
    display: bool,
}
/// Use the PROSE baseline, not an asymmetric fraction's ink centre. Fold
/// displaced proxies into the same visual row before the existing continuous
/// line-band painter runs; source/selection order itself is never changed.
fn math_selection_rows(mut prose: Vec<Rect>, formulas: Vec<MathSelection>) -> Vec<Rect> {
    prose.sort_by(|a, b| {
        (a.pos.y + a.size.y * 0.5)
            .total_cmp(&(b.pos.y + b.size.y * 0.5))
            .then(a.pos.x.total_cmp(&b.pos.x))
    });
    let mut rows: Vec<(MathSelection, Vec<Rect>)> = Vec::new();
    for fragment in prose
        .into_iter()
        .map(|rect| MathSelection {
            center: rect.pos.y + rect.size.y * 0.5,
            tolerance: rect.size.y * 0.45,
            rect,
            display: false,
        })
        .chain(formulas)
    {
        if rows
            .iter()
            .any(|(_, pieces)| pieces.contains(&fragment.rect))
        {
            continue;
        }
        let target = (!fragment.display)
            .then(|| {
                rows.iter()
                    .enumerate()
                    .filter(|(_, (r, _))| {
                        !r.display
                            && (r.center - fragment.center).abs()
                                <= r.tolerance.min(fragment.tolerance)
                    })
                    .min_by(|(_, (a, _)), (_, (b, _))| {
                        (a.center - fragment.center)
                            .abs()
                            .total_cmp(&(b.center - fragment.center).abs())
                    })
                    .map(|(i, _)| i)
            })
            .flatten();
        if let Some(i) = target {
            rows[i].1.push(fragment.rect);
            let r = &mut rows[i].0.rect;
            let right = (r.pos.x + r.size.x).max(fragment.rect.pos.x + fragment.rect.size.x);
            let bottom = (r.pos.y + r.size.y).max(fragment.rect.pos.y + fragment.rect.size.y);
            r.pos = dvec2(
                r.pos.x.min(fragment.rect.pos.x),
                r.pos.y.min(fragment.rect.pos.y),
            );
            r.size = dvec2(right - r.pos.x, bottom - r.pos.y);
        } else {
            let rect = fragment.rect;
            rows.push((fragment, vec![rect]));
        }
    }
    rows.sort_by(|(a, _), (b, _)| {
        a.center
            .total_cmp(&b.center)
            .then(a.rect.pos.x.total_cmp(&b.rect.pos.x))
    });
    rows.into_iter()
        .flat_map(|(row, mut pieces)| {
            pieces.sort_by(|a, b| a.pos.x.total_cmp(&b.pos.x));
            let mut spans: Vec<Rect> = Vec::new();
            for mut piece in pieces {
                piece.pos.y = row.rect.pos.y;
                piece.size.y = row.rect.size.y;
                if let Some(last) = spans
                    .last_mut()
                    .filter(|last| piece.pos.x <= last.pos.x + last.size.x + 2.)
                {
                    last.size.x = last.size.x.max(piece.pos.x + piece.size.x - last.pos.x);
                } else {
                    spans.push(piece);
                }
            }
            spans
        })
        .collect()
}

/// Intercept text whose runs we can record completely, including paragraphs
/// preceded by a callout image. Images retain their own drawing/click handling;
/// code must already use the adapter that records its actually painted rows.
/// Structured tables and code blocks keep their native hit testing. Link text
/// stays inside ReaderLink, which records its paint while retaining its own
/// click handling. Superscript and subscript keep their Html baseline shifts.
pub fn tracked_html(source: &str) -> Option<String> {
    let mut result = String::with_capacity(source.len() * 2);
    let mut offset = 0;
    let mut custom_depth = 0usize;
    while offset < source.len() {
        let rest = &source[offset..];
        if rest.starts_with('<') {
            let mut quote = None;
            let end = rest.char_indices().skip(1).find_map(|(i, ch)| {
                if let Some(q) = quote {
                    if ch == q {
                        quote = None;
                    }
                } else if ch == '\'' || ch == '"' {
                    quote = Some(ch);
                } else if ch == '>' {
                    return Some(i);
                }
                None
            })?;
            let tag = &rest[..=end];
            let inside = rest[1..end].trim();
            let closing = inside.starts_with('/');
            let name = inside
                .trim_start_matches('/')
                .split(|c: char| c.is_whitespace() || c == '/')
                .next()?
                .to_ascii_lowercase();
            if !matches!(
                name.as_str(),
                "p" | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
                    | "b"
                    | "strong"
                    | "i"
                    | "em"
                    | "sup"
                    | "sub"
                    | "br"
                    | "ruby"
                    | "mark"
                    | "img"
                    | "a"
                    | "reader_code"
                    | "reader_indent"
                    | "reader_math"
                    | "reader_math_hidden"
                    | "readeralignon"
                    | "readeralignoff"
            ) {
                return None;
            }
            if matches!(
                name.as_str(),
                "ruby" | "mark" | "reader_code" | "a" | "reader_math" | "reader_math_hidden"
            ) {
                if closing {
                    custom_depth = custom_depth.saturating_sub(1);
                } else if !inside.ends_with('/') {
                    custom_depth += 1;
                }
            }
            result.push_str(tag);
            offset += end + 1;
        } else {
            let len = rest.find('<').unwrap_or(rest.len());
            if custom_depth == 0 {
                result.push_str("<reader_run>");
            }
            result.push_str(&rest[..len]);
            if custom_depth == 0 {
                result.push_str("</reader_run>");
            }
            offset += len;
        }
    }
    (custom_depth == 0).then_some(result)
}

script_mod! {
    use mod.prelude.widgets.*
    mod.widgets.ReaderRunBase = #(ReaderRun::register_widget(vm))
    mod.widgets.ReaderRun = set_type_default() do mod.widgets.ReaderRunBase{}
    mod.widgets.ReaderTextAlignBase = #(ReaderTextAlign::register_widget(vm))
    mod.widgets.ReaderTextAlignOn = set_type_default() do mod.widgets.ReaderTextAlignBase{center: true}
    mod.widgets.ReaderTextAlignOff = set_type_default() do mod.widgets.ReaderTextAlignBase{center: false}
}

#[derive(Script, WidgetRegister)]
pub struct ReaderTextAlign {
    #[uid]
    uid: WidgetUid,
    #[source]
    source: ScriptObjectRef,
    #[live]
    center: bool,
}
impl ScriptHook for ReaderTextAlign {}
impl WidgetNode for ReaderTextAlign {
    fn widget_uid(&self) -> WidgetUid {
        self.uid
    }
    fn walk(&mut self, _cx: &mut Cx) -> Walk {
        Walk::default()
    }
    fn area(&self) -> Area {
        Area::Empty
    }
    fn redraw(&mut self, _cx: &mut Cx) {}
}
impl Widget for ReaderTextAlign {
    fn draw_walk(&mut self, _cx: &mut Cx2d, scope: &mut Scope, _walk: Walk) -> DrawStep {
        if let Some(tf) = scope.data.get_mut::<TextFlow>() {
            tf.cell_text_align_x = if self.center { 0.5 } else { 0.0 };
        }
        DrawStep::done()
    }
    fn handle_event(&mut self, _cx: &mut Cx, _event: &Event, _scope: &mut Scope) {}
}

#[derive(Script, WidgetRegister)]
pub struct ReaderRun {
    #[uid]
    uid: WidgetUid,
    #[source]
    source: ScriptObjectRef,
    #[rust]
    text: String,
}
impl ScriptHook for ReaderRun {}
impl WidgetNode for ReaderRun {
    fn widget_uid(&self) -> WidgetUid {
        self.uid
    }
    fn walk(&mut self, _cx: &mut Cx) -> Walk {
        Walk::default()
    }
    fn area(&self) -> Area {
        Area::Empty
    }
    fn redraw(&mut self, _cx: &mut Cx) {}
}
impl Widget for ReaderRun {
    fn set_text(&mut self, _cx: &mut Cx, text: &str) {
        self.text.clear();
        self.text.push_str(text);
    }
    fn text(&self) -> String {
        self.text.clone()
    }
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, _walk: Walk) -> DrawStep {
        if let Some(tf) = scope.data.get_mut::<TextFlow>() {
            draw_text(tf, cx, &self.text);
        }
        DrawStep::done()
    }
    fn handle_event(&mut self, _cx: &mut Cx, _event: &Event, _scope: &mut Scope) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use makepad_widgets::makepad_html::parse_html;

    #[test]
    fn captured_row_field_preserves_draw_text_instance_alignment() {
        assert_eq!(std::mem::size_of::<DrawText>() % 16, 0);
    }

    #[test]
    fn ruby_reservation_moves_only_its_row_and_shares_space_between_words() {
        let mut cx = Cx::new(Box::new(|_, _| {}));
        let list = DrawList2d::new(&mut cx);
        let event = DrawEvent::default();
        let mut draw = CxDraw::new(&mut cx, &event);
        draw.draw_lists[list.id()].redraw_id = draw.redraw_id;
        draw.draw_list_stack.push(list.id());
        let mut cx = Cx2d::new(&mut draw);
        cx.begin_root_turtle(
            dvec2(300.0, 400.0),
            Layout {
                flow: Flow::right_wrap(),
                ..Layout::default()
            },
        );
        let mut rows = Vec::new();
        for annotated in [false, true, false, false] {
            let start = cx.align_list_len();
            let rect = Rect {
                pos: cx.turtle().pos(),
                size: dvec2(100.0, 20.0),
            };
            let mut area = Area::Empty;
            cx.add_aligned_rect_area(&mut area, rect);
            cx.turtle_mut().allocate_height(20.0);
            cx.emit_turtle_walk(rect, start);
            if annotated {
                // The ruby appears after preceding plain text on this row.
                cx.reserve_turtle_row_top(18.0);
                cx.reserve_turtle_row_top(18.0);
                cx.reserve_turtle_row_top(14.0);
                assert_eq!(cx.turtle().pos().y, rect.pos.y + 18.0);
                assert_eq!(area.rect(&cx).pos.y, cx.turtle().pos().y);
            }
            rows.push(area);
            cx.turtle_new_line_with_spacing(8.0);
        }
        let tops: Vec<_> = rows.iter().map(|area| area.rect(&cx).pos.y).collect();
        assert_eq!(tops, vec![0.0, 46.0, 74.0, 102.0]);
        // Normal rows resume their configured 28px pitch immediately.
        assert_eq!(cx.turtle().pos().y, 130.0);
    }

    #[test]
    fn later_painted_rows_keep_hit_and_selection_coordinates_inverse() {
        // A right-column row may be aligned 18 px away from the run origin.
        // Its caret and highlight must use the same resolved horizontal base.
        let painted_left = 42.0;
        let predicted_left = 24.0;
        let first_glyph = 0.0;
        let scale = 1.25;
        let caret = 96.0_f32;
        let screen = painted_row_screen_x(painted_left, first_glyph, scale, caret);
        assert_eq!(screen, 162.0);
        assert_eq!(
            painted_row_local_x(painted_left, first_glyph, scale, screen),
            caret
        );
        assert_ne!(
            painted_row_local_x(predicted_left, first_glyph, scale, screen),
            caret
        );
    }

    #[test]
    fn word_hit_keeps_the_right_half_of_a_glyph_and_ligature() {
        assert_eq!(grapheme_at_x("读", 0.0, 20.0, 19.0), Some(0));
        assert_eq!(grapheme_at_x("fi", 0.0, 20.0, 19.0), Some(1));
        assert_eq!(grapheme_at_x("e\u{301}", 0.0, 20.0, 19.0), Some(0));
        assert_eq!(grapheme_at_x("读", 0.0, 20.0, 20.0), None);
        assert_eq!(grapheme_at_x("读", 0.0, 20.0, -1.0), None);
    }

    #[test]
    fn centered_caption_keeps_tracked_text_offsets() {
        let source = "<h6><readeralignon></readeralignon>Figure 3-25. A centered caption<readeralignoff></readeralignoff></h6>";
        let tracked = tracked_html(source).expect("caption must retain row hit testing");
        assert!(tracked.contains("<reader_run>Figure 3-25. A centered caption</reader_run>"));
        assert_eq!(
            readwise_core::vocab::annotate::rendered_text(source),
            readwise_core::vocab::annotate::rendered_text(&tracked)
        );
    }

    #[test]
    fn line_margins_stay_in_the_same_block_instead_of_selecting_earlier_text() {
        let block = Rect {
            pos: dvec2(182.0, 120.0),
            size: dvec2(860.0, 240.0),
        };
        for x in [160.0, 180.0, 182.0, 760.0, 1060.0] {
            let pointer = dvec2(x, 334.0);
            assert!(contains_row(block, pointer.y));
        }
        assert!(!contains_row(block, 119.0));
        assert!(!contains_row(block, 360.0));
    }
    #[test]
    fn wrapped_run_empty_space_cannot_hide_the_next_run() {
        // Run A starts late on row 1 and ends early on row 2. Its enclosing
        // box includes B, but neither of A's actual occupied rows does.
        let rows = [
            (
                Rect {
                    pos: dvec2(300.0, 0.0),
                    size: dvec2(200.0, 20.0),
                },
                "A1",
            ),
            (
                Rect {
                    pos: dvec2(0.0, 36.0),
                    size: dvec2(60.0, 20.0),
                },
                "A2",
            ),
            (
                Rect {
                    pos: dvec2(60.0, 36.0),
                    size: dvec2(400.0, 20.0),
                },
                "B",
            ),
            (
                Rect {
                    pos: dvec2(0.0, 0.0),
                    size: dvec2(300.0, 20.0),
                },
                "previous",
            ),
        ];
        assert_eq!(nearest_row(rows.into_iter(), dvec2(240.0, 45.0)), Some("B"));
        assert_eq!(
            nearest_row(rows.into_iter(), dvec2(80.0, 10.0)),
            Some("previous")
        );
        // Reverse movement and line-end / inter-line whitespace use the same
        // geometry; no previous cursor or direction-dependent clamping.
        assert_eq!(nearest_row(rows.into_iter(), dvec2(20.0, 45.0)), Some("A2"));
        assert_eq!(nearest_row(rows.into_iter(), dvec2(550.0, 45.0)), Some("B"));
        assert_eq!(nearest_row(rows.into_iter(), dvec2(240.0, 32.0)), Some("B"));
    }
    #[test]
    fn wrapping_preserves_text_and_rich_tags() {
        let source = "<p>first <em>italic</em> <ruby t='a > b'>word</ruby> 中文 &amp; end</p>";
        let wrapped = tracked_html(source).unwrap();
        assert!(wrapped.contains("<em><reader_run>italic</reader_run></em>"));
        assert!(wrapped.contains("<ruby t='a > b'>word</ruby>"));
        assert_eq!(
            readwise_core::vocab::annotate::rendered_text(source),
            readwise_core::vocab::annotate::rendered_text(&wrapped)
        );
    }

    #[test]
    fn wrapping_preserves_link_widget_and_tracks_surrounding_prose() {
        let source = "<p>Before <a href='next.xhtml'>linked words</a> after the link.</p>";
        let tracked = tracked_html(source).unwrap();
        assert!(tracked.contains("<reader_run>Before </reader_run>"));
        assert!(tracked.contains("<a href='next.xhtml'>linked words</a>"));
        assert!(tracked.contains("<reader_run> after the link.</reader_run>"));
        assert_eq!(
            readwise_core::vocab::annotate::rendered_text(source),
            readwise_core::vocab::annotate::rendered_text(&tracked),
        );
    }

    #[test]
    fn epub_destination_anchor_does_not_capture_the_next_link_or_prose() {
        let source = r#"<p>Before <a id="ix_term"/>word <a href="note.xhtml">note <em>title</em></a> after.</p>"#;
        let sanitized = readwise_core::books::epub_meta::sanitize_for_reader(source);
        let display = tracked_html(&sanitized).unwrap();
        let mut errors = Some(Vec::new());
        let doc = parse_html(&display, &mut errors, InternLiveId::No);
        assert!(errors.unwrap().is_empty());

        let mut walker = doc.new_walker();
        let mut links = Vec::new();
        while !walker.done() {
            if walker.open_tag_lc() == Some(live_id!(a)) {
                links.push((
                    walker.find_attr_lc(live_id!(href)).map(str::to_owned),
                    walker.find_text().map(str::to_owned),
                ));
                walker.jump_to_close();
            }
            walker.walk();
        }
        assert_eq!(
            links,
            vec![
                (None, Some(String::new())),
                (Some("note.xhtml".into()), Some("note title".into()))
            ]
        );
    }

    #[test]
    fn superscript_footnote_preserves_link_and_prose_capture() {
        let source = concat!(
            "<p>度而言，值得庆幸的是：那时美国绝大部分人居住在乡下，也就是今天经济",
            "学家所称的“货币经济”<sup>\n",
            "<a id=\"ch75\" href=\"text00012.html#ch75-back\">[75]</a>\n",
            "</sup> 之外，感受不到经济崩溃带来的苦痛。</p>"
        );
        let tracked = tracked_html(source).expect("footnotes must keep painted-row measurement");
        assert!(tracked.contains("<sup>"));
        assert!(tracked.contains("<a id=\"ch75\" href=\"text00012.html#ch75-back\">[75]</a>"));
        assert!(tracked.contains("<reader_run> 之外，感受不到"));
        assert_eq!(
            readwise_core::vocab::annotate::rendered_text(source),
            readwise_core::vocab::annotate::rendered_text(&tracked),
        );
        assert!(tracked_html("<p>H<sub>2</sub>O</p>").is_some());
    }

    #[test]
    fn callout_code_and_gloss_paragraph_captures_all_text_runs() {
        // Async Rust: the image and the paragraph are one stored block. The
        // old whitelist rejected it, leaving selection stuck before quickly.
        let source = concat!(
            "<img src='1.png' alt='1' width='12' height='12'/>\n<p>Exit as ",
            "<ruby>quickly\u{e000}很快地</ruby> as possible if the <code>HEAT_ON</code>",
            " flag is off because nothing is going to happen. We want to release the ",
            "future from the executor as <ruby>quickly\u{e000}很快地</ruby>",
            " as possible to avoid blocking other <ruby>futures\u{e000}未来</ruby>.</p>",
        );
        let adapted = crate::reader_inline_code::wrap_html(source);
        let tracked = tracked_html(&adapted).expect("mixed paragraph must use row hit testing");
        assert!(tracked.starts_with("<img src='1.png' alt='1' width='12' height='12'/>"));
        assert!(tracked.contains("<reader_code>HEAT_ON</reader_code>"));
        assert!(tracked.contains("<ruby>quickly\u{e000}很快地</ruby>"));
        assert!(tracked.contains("from the executor as </reader_run>"));
        assert!(tracked.contains("<reader_run> as possible to avoid blocking other </reader_run>"));
        assert!(tracked.contains("<reader_run>.</reader_run></p>"));
        assert_eq!(
            readwise_core::vocab::annotate::rendered_text(source),
            readwise_core::vocab::annotate::rendered_text(&tracked),
        );
    }

    #[test]
    fn wrapped_row_tail_does_not_capture_following_gloss_and_suffix() {
        let rows = [
            (
                Rect {
                    pos: dvec2(1030.0, 190.0),
                    size: dvec2(10.0, 22.0),
                },
                "wrapped head",
            ),
            (
                Rect {
                    pos: dvec2(182.0, 227.0),
                    size: dvec2(153.0, 22.0),
                },
                "executor as",
            ),
            (
                Rect {
                    pos: dvec2(335.0, 227.0),
                    size: dvec2(54.0, 22.0),
                },
                "quickly",
            ),
            (
                Rect {
                    pos: dvec2(389.0, 227.0),
                    size: dvec2(255.0, 22.0),
                },
                "suffix",
            ),
            (
                Rect {
                    pos: dvec2(644.0, 227.0),
                    size: dvec2(54.0, 22.0),
                },
                "futures",
            ),
        ];
        // Both forward dragging and backtracking must reach every occupied
        // run on row two, not the bounding box of the earlier wrapped run.
        for (x, expected) in [
            (330.0, "executor as"),
            (360.0, "quickly"),
            (520.0, "suffix"),
            (697.0, "futures"),
            (520.0, "suffix"),
            (360.0, "quickly"),
            (330.0, "executor as"),
        ] {
            assert_eq!(
                nearest_row(rows.into_iter(), dvec2(x, 237.0)),
                Some(expected)
            );
        }
    }

    #[test]
    fn unsupported_code_still_disables_complete_run_capture() {
        for source in [
            "<p>before <code class='custom'>value</code> after</p>",
            "<p>before <code><b>nested</b></code> after</p>",
            "<reader_code>unfinished",
        ] {
            assert!(tracked_html(&crate::reader_inline_code::wrap_html(source)).is_none());
        }
    }

    #[test]
    fn typst_formula_hits_are_atomic_and_displaced_source_paints_the_owner() {
        use makepad_widgets::makepad_draw::text::{
            geom::{Point, Size},
            layouter::LaidoutRow,
        };
        let layout = Rc::new(LaidoutText {
            text: "M".into(),
            size_in_lpxs: Size::new(12., 16.),
            is_truncated: false,
            rows: vec![LaidoutRow {
                origin_in_lpxs: Point::new(0., 12.),
                text: "M".into(),
                newline: false,
                width_in_lpxs: 12.,
                ascender_in_lpxs: 12.,
                descender_in_lpxs: -4.,
                line_gap_in_lpxs: 0.,
                line_spacing_scale: 1.,
                glyphs: Vec::new(),
            }],
        });
        let rect = Rect {
            pos: dvec2(90., 130.),
            size: dvec2(100., 45.),
        };
        let highlight = Rect {
            pos: rect.pos - dvec2(0., 2.),
            size: rect.size + dvec2(0., 4.),
        };
        let run = HitRun {
            layout,
            origin: rect.pos,
            scale: 1.,
            start: 12,
            row: Some(0),
            drawn_rows: vec![(0, rect)],
            ink_rows: vec![(0, rect)],
            atomic: Some(AtomicGeometry {
                len: "x√d_k".len(),
                hidden: false,
                center: 20.,
                display: false,
            }),
        };
        assert_eq!(
            hit_index(std::slice::from_ref(&run), dvec2(100., 150.)),
            Some(12)
        );
        assert_eq!(
            hit_index(std::slice::from_ref(&run), dvec2(180., 150.)),
            Some(12 + "x√d_k".len())
        );
        assert_eq!(
            hit_word_index(std::slice::from_ref(&run), dvec2(100., 150.)),
            None
        );
        assert_eq!(
            selection_rects(std::slice::from_ref(&run), 13, 14),
            vec![highlight]
        );
        assert_eq!(
            selection_rects(std::slice::from_ref(&run), 0, 12),
            Vec::new()
        );
        let hidden = HitRun {
            atomic: Some(AtomicGeometry {
                len: 1,
                hidden: true,
                center: 20.,
                display: false,
            }),
            ..run
        };
        assert_eq!(
            hit_index(std::slice::from_ref(&hidden), dvec2(100., 150.)),
            None
        );
        assert!(painted_rows(std::slice::from_ref(&hidden)).is_empty());
        assert_eq!(selection_rects(&[hidden], 12, 13), vec![highlight]);
    }

    #[test]
    fn paragraph_math_uses_prose_rows_and_deduplicates_out_of_order_script_proxies() {
        let formula = Rect {
            pos: dvec2(45., 28.),
            size: dvec2(25., 24.),
        };
        let rects = math_selection_rows(
            vec![
                Rect {
                    pos: dvec2(10., 70.),
                    size: dvec2(80., 14.),
                },
                Rect {
                    pos: dvec2(10., 20.),
                    size: dvec2(35., 14.),
                },
                Rect {
                    pos: dvec2(70., 20.),
                    size: dvec2(30., 14.),
                },
            ],
            vec![
                MathSelection {
                    rect: formula,
                    center: 27.,
                    tolerance: 7.,
                    display: false,
                },
                MathSelection {
                    rect: formula,
                    center: 27.,
                    tolerance: 7.,
                    display: false,
                },
            ],
        );
        assert_eq!(
            rects,
            vec![
                Rect {
                    pos: dvec2(10., 20.),
                    size: dvec2(90., 32.)
                },
                Rect {
                    pos: dvec2(10., 70.),
                    size: dvec2(80., 14.)
                }
            ]
        );
    }

    #[test]
    fn displaced_formula_proxy_does_not_colour_unselected_prose_between_fragments() {
        let rects = math_selection_rows(
            vec![Rect {
                pos: dvec2(10., 20.),
                size: dvec2(30., 14.),
            }],
            vec![MathSelection {
                rect: Rect {
                    pos: dvec2(200., 16.),
                    size: dvec2(20., 24.),
                },
                center: 27.,
                tolerance: 7.,
                display: false,
            }],
        );
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0].pos.x + rects[0].size.x, 40.);
        assert_eq!(rects[1].pos.x, 200.);
        assert_eq!(rects[0].pos.y, rects[1].pos.y);
        assert_eq!(rects[0].size.y, rects[1].size.y);
    }

    #[test]
    fn structured_and_uncaptured_runs_keep_native_path() {
        for html in [
            "<table><tr><td>x</td></tr></table>",
            "<p><code>x</code></p>",
            "<ruby>x",
            "<p title='unfinished>",
        ] {
            assert!(tracked_html(html).is_none(), "{html}");
        }
    }
}
