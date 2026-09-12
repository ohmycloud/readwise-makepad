//! Correct only inline-code selection geometry in Makepad 4f9ce7a.
//! TextFlow records its selection layout before walking code margin/padding
//! and forcing a wrap. Keep its drawing intact, then replace that run's stale
//! selection rectangles using the code boxes emitted by the real draw.
use makepad_widgets::makepad_draw::text::layouter::LaidoutText;
use makepad_widgets::text_flow::SelectionTracker;
use makepad_widgets::*;
use std::cell::RefCell;

#[derive(Default)]
struct Capture {
    range: Option<(usize, usize)>,
    corrections: Vec<Correction>,
}

struct Correction {
    native: Vec<Rect>,
    // An Area is resolved only after the parent finishes aligning its rows.
    actual: Vec<(Area, Inset, f64, f64)>,
}

thread_local! {
    static CAPTURE: RefCell<Capture> = RefCell::new(Capture::default());
}

pub fn begin_capture(range: Option<(usize, usize)>) {
    CAPTURE.with(|capture| {
        *capture.borrow_mut() = Capture {
            range,
            corrections: Vec::new(),
        }
    });
}

fn same_rect(a: Rect, b: Rect) -> bool {
    (a.pos - b.pos).length() < 0.05 && (a.size - b.size).length() < 0.05
}

fn replace_native_run(
    rects: &mut Vec<Rect>,
    native: &[Rect],
    actual: impl IntoIterator<Item = Rect>,
) {
    // Match a whole contiguous native run. Never union an unrelated row
    // or append out-of-order rectangles to the line normalizer.
    let count = native.len();
    if count == 0 || count > rects.len() {
        return;
    }
    let Some(index) = rects
        .windows(count)
        .position(|window| window.iter().zip(native).all(|(a, b)| same_rect(*a, *b)))
    else {
        return;
    };
    rects.splice(index..index + count, actual);
}

fn selected_glyph_rect(rect: Rect, padding: Inset, left: f64, right: f64) -> Rect {
    Rect {
        pos: rect.pos + padding.left_top() + dvec2(left, -2.0),
        size: dvec2(right - left, rect.size.y - padding.size().y + 4.0),
    }
}

pub fn correct_rects(cx: &Cx, rects: &mut Vec<Rect>) {
    let capture = CAPTURE.with(|capture| std::mem::take(&mut *capture.borrow_mut()));
    for correction in capture.corrections {
        let actual = correction
            .actual
            .into_iter()
            .map(|(area, padding, left, right)| {
                selected_glyph_rect(area.rect(cx), padding, left, right)
            });
        replace_native_run(rects, &correction.native, actual);
    }
}

/// Imported reader HTML uses canonical <code> tags. Wrap only plain code
/// contents; keep structured/nested code on the native path.
pub fn wrap_html(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(start) = rest.find("<code>") {
        out.push_str(&rest[..start]);
        rest = &rest[start + 6..];
        let Some(end) = rest.find("</code>") else {
            out.push_str("<code>");
            break;
        };
        let code = &rest[..end];
        if !code.contains('<') {
            out.push_str("<reader_code>");
            out.push_str(code);
            out.push_str("</reader_code>");
        } else {
            out.push_str("<code>");
            out.push_str(code);
            out.push_str("</code>");
        }
        rest = &rest[end + 7..];
    }
    out.push_str(rest);
    out
}

// Offsets inside the actually painted row, relative to its first glyph.
fn selected_row_spans(layout: &LaidoutText, start: usize, end: usize) -> Vec<(usize, f64, f64)> {
    layout
        .rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let from = start.max(row.text.start_in_parent());
            let to = end.min(row.text.end_in_parent());
            if from >= to {
                return None;
            }
            let first_x = row.glyphs.first()?.origin_in_lpxs.x;
            let left = row.index_to_x_in_lpxs(from - row.text.start_in_parent()) - first_x;
            let right = row.index_to_x_in_lpxs(to - row.text.start_in_parent()) - first_x;
            Some((index, left as f64, right as f64))
        })
        .collect()
}

script_mod! {
    use mod.prelude.widgets.*
    mod.widgets.ReaderInlineCodeBase = #(ReaderInlineCode::register_widget(vm))
    mod.widgets.ReaderInlineCode = set_type_default() do mod.widgets.ReaderInlineCodeBase{}
}

#[derive(Script, WidgetRegister)]
pub struct ReaderInlineCode {
    #[uid]
    uid: WidgetUid,
    #[source]
    source: ScriptObjectRef,
    #[rust]
    text: String,
}
impl ScriptHook for ReaderInlineCode {}
impl WidgetNode for ReaderInlineCode {
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
    fn point_hits_area(&self, _cx: &Cx, _point: DVec2) -> bool {
        false
    }
}
impl Widget for ReaderInlineCode {
    fn set_text(&mut self, _cx: &mut Cx, text: &str) {
        self.text = text.into();
    }
    fn text(&self) -> String {
        self.text.clone()
    }
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, _walk: Walk) -> DrawStep {
        let Some(tf) = scope.data.get_mut::<TextFlow>() else {
            return DrawStep::done();
        };
        // Match Html's code-tag scope. Putting the custom tag *inside* <code>
        // would make Html draw empty code nodes around it as extra padded boxes.
        tf.push_size_rel_scale(0.85);
        tf.combine_spaces.push(false);
        tf.fixed.push();
        tf.inline_code.push();
        let range = CAPTURE.with(|capture| capture.borrow().range);
        let start = tf.text_len();
        let pos = cx.turtle().pos();
        let rect = cx.turtle().inner_rect();
        let row_height = cx.turtle().next_row_offset() as f32;
        let wrap = matches!(cx.turtle().layout().flow, Flow::Right { wrap: true, .. });
        tf.areas_tracker.push_tracker();
        tf.draw_text(cx, &self.text);
        let (area_start, area_end) = tf.areas_tracker.pop_tracker();
        let selected = range.filter(|(from, to)| *from < tf.text_len() && *to > start);
        let capture_hit = tf.selectable && crate::reader_hit::is_capturing();
        if tf.text_len() > start && (selected.is_some() || capture_hit) {
            let text = tf.get_text_for_range(start, tf.text_len());
            let dt = &tf.draw_text;
            // Margin walks have zero height; the remaining tracked rectangles
            // are precisely the inline code backgrounds, one per visual row.
            let areas: Vec<_> = tf.areas_tracker.areas[area_start..area_end]
                .iter()
                .copied()
                .filter(|area| area.rect(cx).size.y > 0.0)
                .collect();
            if let Some(first) = areas.first() {
                let padding = tf.inline_code_padding;
                let first_rect = first.rect(cx);
                let content_left = first_rect.pos.x + padding.left;
                let left = rect.pos.x + padding.left;
                let width = rect.size.x - padding.left - padding.right;
                let actual = dt.layout(
                    cx,
                    (content_left - left) as f32,
                    row_height,
                    (!width.is_nan()).then_some(width as f32),
                    wrap,
                    dt.layout_align,
                    &text,
                );
                if capture_hit {
                    crate::reader_hit::capture_code_rows(
                        actual.clone(),
                        &areas,
                        padding,
                        dt.font_scale,
                        start,
                    );
                }
                if let Some((from, to)) = selected.filter(|_| actual.rows.len() == areas.len()) {
                    let from = from.saturating_sub(start);
                    let to = (to - start).min(text.len());
                    let native = dt.layout(
                        cx,
                        (pos.x - rect.pos.x) as f32,
                        row_height,
                        (!rect.size.x.is_nan()).then_some(rect.size.x as f32),
                        wrap,
                        dt.layout_align,
                        &text,
                    );
                    let mut tracker = SelectionTracker::default();
                    tracker.push_text(native, dvec2(rect.pos.x, pos.y), dt.font_scale, &text);
                    let native = tracker.selection_rects(from, to);
                    let actual = selected_row_spans(&actual, from, to)
                        .into_iter()
                        .map(|(i, left, right)| {
                            (
                                areas[i],
                                padding,
                                left * dt.font_scale as f64,
                                right * dt.font_scale as f64,
                            )
                        })
                        .collect();
                    CAPTURE.with(|capture| {
                        capture
                            .borrow_mut()
                            .corrections
                            .push(Correction { native, actual })
                    });
                }
            } else if capture_hit {
                crate::reader_hit::cancel_capture();
            }
        }
        tf.inline_code.pop();
        tf.fixed.pop();
        tf.combine_spaces.pop();
        tf.font_sizes.pop();
        DrawStep::done()
    }
    fn handle_event(&mut self, _cx: &mut Cx, _event: &Event, _scope: &mut Scope) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            pos: dvec2(x, y),
            size: dvec2(width, height),
        }
    }

    #[test]
    fn code_adapter_preserves_text_offsets_and_owns_the_code_scope() {
        let source = "<p>Sets <code>vec</code>'s length to <code>new_len</code>. \
            <em>中文</em> <ruby>word</ruby> <code>Clone &amp; T&lt;U&gt;</code></p>";
        let wrapped = wrap_html(source);
        assert_eq!(
            readwise_core::vocab::annotate::rendered_text(source),
            readwise_core::vocab::annotate::rendered_text(&wrapped),
        );
        assert_eq!(wrapped.matches("<reader_code>").count(), 3);
        assert!(wrapped.contains("<reader_code>new_len</reader_code>"));
        // Keeping <code> outside the adapter would draw extra empty padded
        // code boxes, changing both the text position and the paragraph wrap.
        assert!(!wrapped.contains("<code>"));
        assert!(wrapped.contains("<em>中文</em> <ruby>word</ruby>"));
    }

    #[test]
    fn unsupported_or_unterminated_code_keeps_original_html() {
        for source in [
            "plain &lt;code&gt; text",
            "<p><code><em>structured</em></code></p>",
            "<p><code>unfinished",
            "<p><code class='custom'>code</code></p>",
        ] {
            assert_eq!(wrap_html(source), source);
        }
        assert_eq!(
            wrap_html("<code>first</code><code>unfinished"),
            "<reader_code>first</reader_code><code>unfinished",
        );
    }

    #[test]
    fn final_glyph_uses_painted_code_origin_including_padding() {
        let padding = Inset {
            left: 3.0,
            right: 3.0,
            top: 2.0,
            bottom: 2.0,
        };
        // Each code run is painted 5 px after its native selection origin:
        // 2 px margin + 3 px padding. Using the old origin leaves the tail off.
        for width in [24.0, 56.0, 40.0] {
            // vec, new_len, Clone
            let code_box = rect(102.0, 18.0, width + 6.0, 24.0);
            let native = rect(100.0, 18.0, width, 24.0);
            let actual = selected_glyph_rect(code_box, padding, 0.0, width);
            let mut selected = vec![native];
            replace_native_run(&mut selected, &[native], [actual]);
            assert_eq!(selected, vec![rect(105.0, 18.0, width, 24.0)]);
            assert_eq!(selected[0].pos.x + selected[0].size.x, 105.0 + width);
        }
    }

    #[test]
    fn partial_selection_and_backtracking_do_not_fill_the_whole_code_box() {
        let code_box = rect(102.0, 18.0, 62.0, 24.0);
        let padding = Inset {
            left: 3.0,
            right: 3.0,
            top: 2.0,
            bottom: 2.0,
        };
        // Only use offsets of selected glyphs, not the entire background Area.
        let expanded = selected_glyph_rect(code_box, padding, 8.0, 48.0);
        let retracted = selected_glyph_rect(code_box, padding, 8.0, 24.0);
        assert_eq!(expanded, rect(113.0, 18.0, 40.0, 24.0));
        assert_eq!(retracted, rect(113.0, 18.0, 16.0, 24.0));
    }

    #[test]
    fn corrected_wrap_keeps_neighboring_runs_in_source_order() {
        let before = rect(20.0, 0.0, 80.0, 24.0);
        let native = [rect(100.0, 0.0, 20.0, 24.0), rect(20.0, 30.0, 20.0, 24.0)];
        let after = rect(70.0, 30.0, 120.0, 24.0);
        let actual = rect(23.0, 30.0, 40.0, 24.0);
        let mut selected = vec![before, native[0], native[1], after];
        replace_native_run(&mut selected, &native, [actual]);
        assert_eq!(selected, vec![before, actual, after]);
    }

    #[test]
    fn partial_or_stale_native_match_cannot_modify_other_runs() {
        let a = rect(20.0, 0.0, 80.0, 24.0);
        let b = rect(20.0, 30.0, 80.0, 24.0);
        let unchanged = vec![a, b];
        let mut selected = unchanged.clone();
        let replacement = rect(25.0, 0.0, 80.0, 24.0);
        for native in [
            vec![],
            vec![a, b, a],
            vec![a, a],
            vec![rect(20.0, 1.0, 80.0, 24.0)],
        ] {
            replace_native_run(&mut selected, &native, [replacement]);
            assert_eq!(selected, unchanged);
        }
    }
}
