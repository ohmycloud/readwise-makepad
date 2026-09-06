//! Hit testing from the same per-run layout used to paint reader text.
//! TextFlow's bounding-box hit includes the empty part of a wrapped run;
//! another run on that visual row must not be hidden behind that empty box.
use makepad_widgets::makepad_draw::text::layouter::LaidoutText;
use makepad_widgets::*;
use std::{cell::RefCell, rc::Rc};

#[derive(Clone)]
pub struct HitRun {
    layout: Rc<LaidoutText>,
    origin: DVec2,
    scale: f64,
    start: usize,
}

thread_local! {
    static RUNS: RefCell<Option<Vec<HitRun>>> = const { RefCell::new(None) };
}

pub fn begin_capture() {
    RUNS.with(|runs| *runs.borrow_mut() = Some(Vec::new()));
}

pub fn end_capture(origin: DVec2) -> Vec<HitRun> {
    let mut runs = RUNS.with(|runs| runs.borrow_mut().take().unwrap_or_default());
    for run in &mut runs {
        run.origin -= origin;
    }
    runs
}

/// The actual TextFlow draw remains authoritative. Reusing its cached layout
/// afterwards records glyph/UTF-8 boundaries without reflowing the paragraph.
pub fn draw_text(tf: &mut TextFlow, cx: &mut Cx2d, text: &str) {
    let capture = tf.selectable && RUNS.with(|runs| runs.borrow().is_some());
    let pos = cx.turtle().pos();
    let rect = cx.turtle().inner_rect();
    let row_height = cx.turtle().next_row_offset() as f32;
    let wrap = matches!(cx.turtle().layout().flow, Flow::Right { wrap: true, .. });
    let start = tf.text_len();
    tf.draw_text(cx, text);
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
    RUNS.with(|runs| {
        if let Some(runs) = runs.borrow_mut().as_mut() {
            runs.push(HitRun {
                layout,
                origin: dvec2(rect.pos.x, pos.y + shift as f64),
                scale: dt.font_scale as f64,
                start,
            });
        }
    });
}

fn axis_distance(value: f64, low: f64, high: f64) -> f64 {
    (low - value).max(value - high).max(0.0)
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
    let rows = runs.iter().flat_map(|run| {
        run.layout.rows.iter().filter_map(move |row| {
            let first = row.glyphs.first()?;
            let left = run.origin.x + first.origin_in_lpxs.x as f64 * run.scale;
            let right = run.origin.x + row.width_in_lpxs as f64 * run.scale;
            let baseline = run.origin.y + row.origin_in_lpxs.y as f64 * run.scale;
            let top = baseline - row.ascender_in_lpxs as f64 * run.scale;
            let bottom = baseline - row.descender_in_lpxs as f64 * run.scale;
            Some((
                Rect {
                    pos: dvec2(left, top),
                    size: dvec2(right - left, bottom - top),
                },
                (run, row),
            ))
        })
    });
    let (run, row) = nearest_row(rows, point)?;
    Some(
        run.start
            + row.text.start_in_parent()
            + row.x_in_lpxs_to_index(((point.x - run.origin.x) / run.scale) as f32),
    )
}

/// Only intercept continuous text whose runs we can record completely. Other
/// structures keep their existing native hit testing (tables/code/links etc.).
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
                    | "br"
                    | "ruby"
                    | "mark"
            ) {
                return None;
            }
            if matches!(name.as_str(), "ruby" | "mark") {
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
    fn structured_and_uncaptured_runs_keep_native_path() {
        for html in [
            "<table><tr><td>x</td></tr></table>",
            "<p><code>x</code></p>",
            "<p><a href='x'>link</a></p>",
            "<ruby>x",
            "<p title='unfinished>",
        ] {
            assert!(tracked_html(html).is_none(), "{html}");
        }
    }
}
