//! Convert a scroll gesture into one page-turn intent, not one per animation.
//! Keep this state across redraws/chapter changes: inertia belongs to the old
//! gesture even after the incoming page is ready.
use makepad_widgets::makepad_platform::event::ScrollEvent;

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

impl ScrollGesture {
    pub fn on_scroll(&mut self, event: &ScrollEvent) -> Option<bool> {
        let delta = if event.scroll.y.abs() >= event.scroll.x.abs() {
            event.scroll.y
        } else {
            event.scroll.x
        };
        self.update(event.time, delta, event.is_mouse, native_phase(event))
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
