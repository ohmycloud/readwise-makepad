//! Scroll geometry, a transient reading indicator, and paged gesture ownership.
//! Keep this state across redraws/chapter changes: inertia belongs to the old
//! gesture even after the incoming page is ready.
use crate::reader_columns::BlockMetrics;
use makepad_widgets::makepad_platform::event::ScrollEvent;

/// Native wheel/trackpad motion already contains its own timing. This only
/// controls the overlay's lifetime; it never filters or interpolates motion.
#[derive(Default)]
pub struct ScrollIndicator {
    last_motion: Option<f64>,
}

impl ScrollIndicator {
    pub fn moved(&mut self, now: f64) {
        self.last_motion = Some(now);
    }

    pub fn opacity(&self, now: f64) -> f32 {
        self.last_motion.map_or(0.0, |last| {
            (1.0 - ((now - last - 0.45) / 0.18).max(0.0)).clamp(0.0, 1.0) as f32
        })
    }
}

/// Use measured paragraph heights where available and their mean elsewhere,
/// like PortalList's height tree. Never instantiate crossed paragraphs merely
/// to consume a large pending wheel displacement.
pub fn scroll_extent(metrics: &[Option<BlockMetrics>], first: usize, scroll: f64) -> (f64, f64) {
    let (sum, count) = metrics
        .iter()
        .flatten()
        .fold((0.0, 0), |(sum, n), m| (sum + m.height.max(1.0), n + 1));
    let estimate = if count == 0 { 30.0 } else { sum / count as f64 };
    let height = |m: &Option<BlockMetrics>| m.as_ref().map_or(estimate, |m| m.height.max(1.0));
    let before: f64 = metrics.iter().take(first).map(height).sum();
    let total: f64 = metrics.iter().map(height).sum();
    (before - scroll, total)
}

pub fn seek_large_scroll(
    metrics: &[Option<BlockMetrics>],
    first: usize,
    scroll: f64,
    viewport: f64,
) -> Option<(usize, f64)> {
    if metrics.is_empty()
        || !scroll.is_finite()
        || !viewport.is_finite()
        || viewport <= 0.0
        || scroll.abs() <= viewport * 2.0
    {
        return None;
    }
    let (position, total) = scroll_extent(metrics, first, scroll);
    if scroll < 0.0
        && metrics.iter().any(Option::is_none)
        && position >= (total - viewport).max(0.0)
    {
        // An estimated last viewport is not the real bottom. Repeated large
        // wheel packets would keep landing at that same estimate, preventing
        // PortalList from drawing the tail and reporting at_end. Backfill
        // from the actual final block and let the list align its measured end.
        return Some((metrics.len() - 1, 0.0));
    }
    let target = position.clamp(0.0, (total - viewport).max(0.0));
    let (sum, count) = metrics
        .iter()
        .flatten()
        .fold((0.0, 0), |(sum, n), m| (sum + m.height.max(1.0), n + 1));
    let estimate = if count == 0 { 30.0 } else { sum / count as f64 };
    let mut remaining = target;
    for (id, metric) in metrics.iter().enumerate() {
        let height = metric.as_ref().map_or(estimate, |m| m.height.max(1.0));
        if remaining < height || id + 1 == metrics.len() {
            return Some((id, -remaining));
        }
        remaining -= height;
    }
    None
}

const TRACKPAD_THRESHOLD: f64 = 8.0;
const QUIET_GAP: f64 = 0.2;
const MOUSE_INTERVAL: f64 = 0.25;

#[derive(Clone, Copy)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
enum Phase {
    Begin,
    Update,
    End,
    Momentum,
}

#[derive(Default)]
pub struct ScrollGesture {
    last_event: Option<(f64, bool)>,
    last_mouse_turn: Option<f64>,
    distance: f64,
    consumed: bool,
}

/// The action belongs to the chapter whose boundary was actually painted.
/// Several input packets can arrive before the next draw; stale actions must
/// not skip more chapters or move a newly opened book.
#[derive(Clone, Copy, Debug)]
pub struct ScrollBoundaryTurn {
    pub chapter: usize,
    pub content_version: u64,
    pub forward: bool,
}

impl ScrollBoundaryTurn {
    pub fn target(&self, chapter: usize, content_version: u64, chapters: usize) -> Option<usize> {
        if self.chapter != chapter || self.content_version != content_version || chapter >= chapters
        {
            return None;
        }
        if self.forward {
            chapter.checked_add(1).filter(|target| *target < chapters)
        } else {
            chapter.checked_sub(1)
        }
    }
}

impl ScrollGesture {
    /// Scrolling is continuous across chapter/PDF source-page boundaries.
    /// Unlike paged reading, interior motion must not spend a page-turn token
    /// or make the user wait for the same gesture's momentum to finish.
    pub fn on_continuous_scroll(&mut self, event: &ScrollEvent, boundary: bool) -> Option<bool> {
        self.update_continuous(
            event.time,
            event.scroll.y,
            event.is_mouse,
            native_phase(event),
            boundary,
        )
    }

    fn update_continuous(
        &mut self,
        time: f64,
        delta: f64,
        is_mouse: bool,
        phase: Option<Phase>,
        boundary: bool,
    ) -> Option<bool> {
        if !time.is_finite() || !delta.is_finite() {
            return None;
        }
        let changed_device = self.last_event.is_none_or(|(_, mouse)| mouse != is_mouse);
        let quiet = self
            .last_event
            .is_none_or(|(last, _)| time < last || time - last >= QUIET_GAP);
        self.last_event = Some((time, is_mouse));
        if changed_device || matches!(phase, Some(Phase::Begin)) || (phase.is_none() && quiet) {
            self.consumed = false;
            self.distance = 0.0;
        }
        if !boundary {
            self.distance = 0.0;
            return None;
        }
        if is_mouse {
            return (delta.abs() >= 1.0).then_some(delta > 0.0);
        }
        // A gesture started inside an overlay remains owned by that overlay.
        if self.consumed || matches!(phase, Some(Phase::End)) {
            return None;
        }
        self.distance += delta;
        if self.distance.abs() < TRACKPAD_THRESHOLD {
            return None;
        }
        let forward = self.distance > 0.0;
        self.distance = 0.0;
        Some(forward)
    }

    pub fn on_scroll(&mut self, event: &ScrollEvent) -> Option<bool> {
        let delta = if event.scroll.y.abs() >= event.scroll.x.abs() {
            event.scroll.y
        } else {
            event.scroll.x
        };
        self.update(event.time, delta, event.is_mouse, native_phase(event))
    }

    pub fn block_scroll(&mut self, event: &ScrollEvent) {
        self.observe_blocked(event.time, event.is_mouse);
    }

    fn observe_blocked(&mut self, time: f64, is_mouse: bool) {
        // Only the stream/device timestamp matters here; blocked deltas must
        // neither accumulate distance nor advance the mouse-wheel repeat timer.
        if time.is_finite() {
            self.last_event = Some((time, is_mouse));
        }
        self.consume();
    }

    /// A gesture owned by a code/table scroller or another panel must not
    /// start turning pages when its pointer or momentum enters the text.
    pub fn consume(&mut self) {
        self.consumed = true;
        self.distance = 0.0;
    }

    fn update(
        &mut self,
        time: f64,
        delta: f64,
        is_mouse: bool,
        phase: Option<Phase>,
    ) -> Option<bool> {
        if !time.is_finite() || !delta.is_finite() {
            return None;
        }
        let changed_device = self.last_event.is_none_or(|(_, mouse)| mouse != is_mouse);
        let quiet = self
            .last_event
            .is_none_or(|(last, _)| time < last || time - last >= QUIET_GAP);
        // Observe every packet, including subpixel/zero inertia tails. Updating
        // only when a turn fires would repeat every 250ms during a single swipe.
        self.last_event = Some((time, is_mouse));
        if is_mouse {
            if changed_device {
                self.last_mouse_turn = None;
            }
            if delta.abs() >= 1.0
                && self
                    .last_mouse_turn
                    .is_none_or(|last| time < last || time - last >= MOUSE_INTERVAL)
            {
                self.last_mouse_turn = Some(time);
                return Some(delta > 0.0);
            }
            return None;
        }

        // Native phases allow a new deliberate swipe immediately, even before
        // the previous gesture's inertia or page animation has finished. On
        // platforms without phases, re-arm only after the event stream is quiet.
        let begin =
            changed_device || matches!(phase, Some(Phase::Begin)) || (phase.is_none() && quiet);
        if begin {
            self.distance = 0.0;
            self.consumed = false;
        }
        if matches!(phase, Some(Phase::End | Phase::Momentum)) {
            self.consume();
            return None;
        }
        if self.consumed {
            return None;
        }
        self.distance += delta;
        if self.distance.abs() < TRACKPAD_THRESHOLD {
            return None;
        }
        let forward = self.distance > 0.0;
        self.consume();
        Some(forward)
    }
}

#[cfg(any(target_os = "macos", test))]
fn phase_from_appkit(phase: u64, momentum: u64) -> Option<Phase> {
    // NSEventPhase flags from AppKit/NSEvent.h. Momentum uses its own phase;
    // momentum's Began is NOT the beginning of a new finger gesture.
    if momentum != 0 {
        Some(Phase::Momentum)
    } else if phase & (8 | 16) != 0 {
        Some(Phase::End)
    } else if phase & (1 | 32) != 0 {
        Some(Phase::Begin)
    } else if phase & (2 | 4) != 0 {
        Some(Phase::Update)
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn native_phase(event: &ScrollEvent) -> Option<Phase> {
    use makepad_widgets::makepad_platform::os::apple::apple_sys::*;
    if event.is_mouse {
        return None;
    }
    // The pinned Makepad sends NSEvent to NSApplication before synchronously
    // dispatching ScrollEvent, but drops phase/momentumPhase from ScrollEvent.
    // Read that current NSEvent without changing/consuming the native event.
    unsafe {
        let app: ObjcId = msg_send![class!(NSApplication), sharedApplication];
        if app == nil {
            return None;
        }
        let native: ObjcId = msg_send![app, currentEvent];
        if native == nil {
            return None;
        }
        let kind: u64 = msg_send![native, type];
        if kind != NSEventType::NSScrollWheel as u64 {
            return None;
        }
        let precise: BOOL = msg_send![native, hasPreciseScrollingDeltas];
        let dx: f64 = msg_send![native, scrollingDeltaX];
        let dy: f64 = msg_send![native, scrollingDeltaY];
        if precise != YES
            || (dx + event.scroll.x).abs() > 0.001
            || (dy + event.scroll.y).abs() > 0.001
        {
            // Synthetic Studio input or an unrelated native event: use the
            // same phase-less gesture detector as other desktop platforms.
            return None;
        }
        let phase: u64 = msg_send![native, phase];
        let momentum: u64 = msg_send![native, momentumPhase];
        phase_from_appkit(phase, momentum)
    }
}

#[cfg(not(target_os = "macos"))]
fn native_phase(_event: &ScrollEvent) -> Option<Phase> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolling_reaches_the_next_pdf_page_with_the_same_swipe_and_momentum() {
        let mut gesture = ScrollGesture::default();
        assert_eq!(
            gesture.update_continuous(1.0, 80.0, false, Some(Phase::Begin), false),
            None
        );
        assert_eq!(
            gesture.update_continuous(1.02, 30.0, false, Some(Phase::Update), false),
            None
        );
        assert_eq!(
            gesture.update_continuous(1.04, 12.0, false, Some(Phase::Momentum), true),
            Some(true)
        );
        // Reaching another short PDF source page does not need a new swipe.
        assert_eq!(
            gesture.update_continuous(1.06, 9.0, false, Some(Phase::Momentum), true),
            Some(true)
        );
        assert_eq!(
            gesture.update_continuous(1.08, -10.0, false, Some(Phase::Update), true),
            Some(false)
        );
    }

    #[test]
    fn continuous_mouse_boundaries_have_no_paged_repeat_delay() {
        let mut gesture = ScrollGesture::default();
        for time in [1.0, 1.01, 1.02] {
            assert_eq!(
                gesture.update_continuous(time, 30.0, true, None, true),
                Some(true)
            );
        }
        // Keep the 250ms rule in paged mode.
        assert_eq!(gesture.update(2.0, 30.0, true, None), Some(true));
        assert_eq!(gesture.update(2.01, 30.0, true, None), None);
    }

    #[test]
    fn continuous_scroll_still_respects_overlay_ownership_and_small_tails() {
        let mut gesture = ScrollGesture::default();
        gesture.observe_blocked(1.0, false);
        assert_eq!(
            gesture.update_continuous(1.02, 40.0, false, Some(Phase::Momentum), true),
            None
        );
        assert_eq!(
            gesture.update_continuous(1.04, 4.0, false, Some(Phase::Begin), true),
            None
        );
        assert_eq!(
            gesture.update_continuous(1.06, 4.0, false, Some(Phase::Update), true),
            Some(true)
        );
        assert_eq!(
            gesture.update_continuous(1.08, 0.1, false, Some(Phase::Momentum), true),
            None
        );
    }

    #[test]
    fn queued_boundary_actions_cannot_skip_a_page_or_wrap_the_book() {
        let turn = ScrollBoundaryTurn {
            chapter: 2,
            content_version: 17,
            forward: true,
        };
        assert_eq!(turn.target(2, 17, 5), Some(3));
        assert_eq!(turn.target(3, 17, 5), None);
        assert_eq!(turn.target(2, 18, 5), None);
        assert_eq!(turn.target(2, 17, 3), None);
        assert_eq!(
            ScrollBoundaryTurn {
                chapter: 0,
                forward: false,
                ..turn
            }
            .target(0, 17, 5),
            None
        );
    }

    #[test]
    fn scroll_indicator_hides_after_quiet_time_and_restarts_on_motion() {
        let mut indicator = ScrollIndicator::default();
        assert_eq!(indicator.opacity(10.0), 0.0);
        indicator.moved(10.0);
        assert_eq!(indicator.opacity(10.4), 1.0);
        assert!((indicator.opacity(10.54) - 0.5).abs() < 0.001);
        assert_eq!(indicator.opacity(10.7), 0.0);
        indicator.moved(10.8);
        assert_eq!(indicator.opacity(10.8), 1.0);
    }

    #[test]
    fn large_wheel_motion_skips_crossed_paragraphs_in_both_directions() {
        let metrics: Vec<_> = (0..10_000)
            .map(|_| {
                Some(BlockMetrics {
                    height: 100.0,
                    ..Default::default()
                })
            })
            .collect();
        assert_eq!(
            seek_large_scroll(&metrics, 0, -50_025.0, 600.0),
            Some((500, -25.0))
        );
        assert_eq!(
            seek_large_scroll(&metrics, 500, 40_025.0, 600.0),
            Some((99, -75.0))
        );
        assert_eq!(
            seek_large_scroll(&metrics, 0, -2_000_000.0, 600.0),
            Some((9_994, -0.0))
        );
        assert_eq!(
            seek_large_scroll(&metrics, 500, 60_000.0, 600.0),
            Some((0, -0.0))
        );
        assert_eq!(seek_large_scroll(&metrics, 500, -24.0, 600.0), None);
    }

    #[test]
    fn seeking_estimates_unknown_blocks_without_mutating_layout_metrics() {
        let mut metrics = vec![None; 100];
        metrics[0] = Some(BlockMetrics {
            height: 200.0,
            ..Default::default()
        });
        metrics[1] = Some(BlockMetrics {
            height: 100.0,
            ..Default::default()
        });
        assert_eq!(scroll_extent(&metrics, 2, -30.0), (330.0, 15_000.0));
        assert_eq!(
            seek_large_scroll(&metrics, 2, -5_000.0, 600.0),
            Some((35, -50.0))
        );
        assert_eq!(metrics.iter().flatten().count(), 2);
        assert_eq!(seek_large_scroll(&metrics, 0, f64::NAN, 600.0), None);
    }

    #[test]
    fn unknown_tail_uses_the_real_last_block_instead_of_stalling_at_an_estimate() {
        let mut metrics = vec![None; 100];
        metrics[0] = Some(BlockMetrics {
            height: 200.0,
            ..Default::default()
        });
        assert_eq!(
            seek_large_scroll(&metrics, 0, -100_000.0, 600.0),
            Some((99, 0.0))
        );
        assert_eq!(
            seek_large_scroll(&metrics, 96, -10_000.0, 600.0),
            Some((99, 0.0))
        );
        assert_eq!(
            seek_large_scroll(&metrics, 96, 100_000.0, 600.0),
            Some((0, -0.0))
        );
        assert_eq!(metrics.iter().flatten().count(), 1);
    }

    #[test]
    fn a_long_swipe_and_inertia_turn_only_one_spread() {
        for columns in [1, 2] {
            let mut gesture = ScrollGesture::default();
            let mut page = 255;
            for i in 0..180 {
                let delta = if i < 30 { 10.0 } else { 0.3 };
                if gesture.update(i as f64 / 60.0, delta, false, None) == Some(true) {
                    page += columns;
                }
            }
            assert_eq!(page, 255 + columns);
            assert_eq!(gesture.update(3.3, 10.0, false, None), Some(true));
        }
    }

    #[test]
    fn native_inertia_never_rearms_after_slow_layout_or_a_chapter_change() {
        let mut gesture = ScrollGesture::default();
        assert_eq!(
            gesture.update(1.0, 10.0, false, Some(Phase::Begin)),
            Some(true)
        );
        assert_eq!(gesture.update(2.0, 30.0, false, Some(Phase::Update)), None);
        assert_eq!(gesture.update(2.1, 0.0, false, Some(Phase::End)), None);
        for (time, momentum) in [(3.0, 1), (3.3, 4), (3.8, 8)] {
            assert_eq!(
                gesture.update(time, 20.0, false, phase_from_appkit(0, momentum)),
                None
            );
        }
    }

    #[test]
    fn a_new_native_swipe_can_start_without_waiting_for_an_animation_timeout() {
        let mut gesture = ScrollGesture::default();
        assert_eq!(
            gesture.update(1.0, 10.0, false, Some(Phase::Begin)),
            Some(true)
        );
        assert_eq!(
            gesture.update(1.03, 10.0, false, Some(Phase::Momentum)),
            None
        );
        assert_eq!(
            gesture.update(1.1, -10.0, false, Some(Phase::Begin)),
            Some(false)
        );
        assert_eq!(
            gesture.update(1.12, -30.0, false, Some(Phase::Update)),
            None
        );
    }

    #[test]
    fn subpixel_tail_and_direction_jitter_do_not_enqueue_another_turn() {
        let mut gesture = ScrollGesture::default();
        assert_eq!(gesture.update(0.0, 3.0, false, None), None);
        assert_eq!(gesture.update(0.02, 3.0, false, None), None);
        assert_eq!(gesture.update(0.04, 3.0, false, None), Some(true));
        for i in 1..40 {
            assert_eq!(
                gesture.update(0.04 + i as f64 * 0.1, -0.1, false, None),
                None
            );
        }
        assert_eq!(gesture.update(4.1, 10.0, false, None), None);
        assert_eq!(gesture.update(4.4, -10.0, false, None), Some(false));
    }

    #[test]
    fn scrolling_a_child_or_entering_from_outside_cannot_turn_the_page() {
        let mut gesture = ScrollGesture::default();
        assert_eq!(gesture.update(0.0, 1.0, false, None), None);
        gesture.consume();
        assert_eq!(gesture.update(0.1, 40.0, false, None), None);
        assert_eq!(gesture.update(0.4, 10.0, false, None), Some(true));
    }

    #[test]
    fn overlay_packets_cannot_rearm_a_trackpad_when_the_pointer_leaves() {
        let mut gesture = ScrollGesture::default();
        // Open a card after a normal reader gesture, then scroll it for long
        // enough that an unobserved packet would look like a new gesture.
        assert_eq!(gesture.update(0.0, 12.0, false, None), Some(true));
        for i in 1..=60 {
            gesture.observe_blocked(i as f64 * 0.1, false);
        }
        assert_eq!(gesture.update(6.05, 40.0, false, None), None);
        assert_eq!(gesture.update(6.1, -30.0, false, None), None);
        assert_eq!(gesture.update(6.4, 12.0, false, None), Some(true));
    }

    #[test]
    fn overlay_scrolling_before_the_first_reader_packet_still_owns_the_gesture() {
        let mut gesture = ScrollGesture::default();
        gesture.observe_blocked(1.0, false);
        assert_eq!(gesture.update(1.02, 40.0, false, None), None);
        assert_eq!(
            gesture.update(1.04, 40.0, false, Some(Phase::Momentum)),
            None
        );
        assert_eq!(
            gesture.update(1.06, 12.0, false, Some(Phase::Begin)),
            Some(true)
        );
        // A discrete mouse tick after leaving a card remains usable.
        gesture.observe_blocked(2.0, true);
        assert_eq!(gesture.update(2.02, -32.0, true, None), Some(false));
    }

    #[test]
    fn discrete_mouse_wheel_keeps_its_existing_repeat_behavior() {
        let mut gesture = ScrollGesture::default();
        assert_eq!(gesture.update(0.0, 32.0, true, None), Some(true));
        assert_eq!(gesture.update(0.1, 32.0, true, None), None);
        assert_eq!(gesture.update(0.3, -32.0, true, None), Some(false));
        assert_eq!(gesture.update(0.4, 10.0, false, None), Some(true));
    }

    #[test]
    fn cancelled_or_momentum_only_gestures_and_invalid_samples_do_not_turn() {
        let mut gesture = ScrollGesture::default();
        assert_eq!(gesture.update(0.0, f64::NAN, false, None), None);
        assert_eq!(gesture.update(f64::INFINITY, 30.0, false, None), None);
        assert_eq!(
            gesture.update(1.0, 30.0, false, phase_from_appkit(0, 1)),
            None
        );
        assert_eq!(
            gesture.update(1.2, 0.0, false, phase_from_appkit(1, 0)),
            None
        );
        assert_eq!(
            gesture.update(1.3, 30.0, false, phase_from_appkit(16, 0)),
            None
        );
    }
}
