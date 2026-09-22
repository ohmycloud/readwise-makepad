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
    // Inline code may be explicitly moved to a new line by TextFlow. Each
    // painted row then has its own origin instead of the pre-draw run origin.
    row: Option<usize>,
}

struct PendingRun {
    run: HitRun,
    painted_row: Option<(Area, Inset)>,
}

thread_local! {
    static RUNS: RefCell<Option<Vec<PendingRun>>> = const { RefCell::new(None) };
}

pub fn begin_capture() {
    RUNS.with(|runs| *runs.borrow_mut() = Some(Vec::new()));
}

pub fn is_capturing() -> bool {
    RUNS.with(|runs| runs.borrow().is_some())
}

pub fn cancel_capture() {
    RUNS.with(|runs| *runs.borrow_mut() = None);
}

pub fn end_capture(cx: &Cx, origin: DVec2) -> Vec<HitRun> {
    RUNS.with(|runs| runs.borrow_mut().take().unwrap_or_default())
        .into_iter()
        .filter_map(|pending| {
            let mut run = pending.run;
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
            }
            run.origin -= origin;
            Some(run)
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
                    },
                    painted_row: Some((area, padding)),
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
            runs.push(PendingRun {
                run: HitRun {
                    layout,
                    origin: dvec2(rect.pos.x, pos.y + shift as f64),
                    scale: dt.font_scale as f64,
                    start,
                    row: None,
                },
                painted_row: None,
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
    let rows = runs.iter().flat_map(|run| {
        run.layout
            .rows
            .iter()
            .enumerate()
            .filter_map(move |(index, row)| {
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

/// Intercept text whose runs we can record completely, including paragraphs
/// preceded by a callout image. Images retain their own drawing/click handling;
/// code must already use the adapter that records its actually painted rows.
/// Structured tables and code blocks keep their native hit testing. Link text
/// stays inside HtmlLink so it remains clickable; surrounding prose can still
/// use exact painted-run hit testing.
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
                    | "img"
                    | "a"
                    | "reader_code"
                    | "readeralignon"
                    | "readeralignoff"
            ) {
                return None;
            }
            if matches!(name.as_str(), "ruby" | "mark" | "reader_code" | "a") {
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
