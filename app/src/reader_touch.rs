//! Touch selection starts with a whole word; a normal swipe remains scrolling.
use makepad_widgets::makepad_platform::event::finger::TAP_COUNT_DISTANCE;
use makepad_widgets::makepad_platform::event::TouchState;
use makepad_widgets::{DVec2, Event, MouseButton, Rect};

type Position = (usize, usize);

/// A scrolling list must remember movement even if the pointer returns to its origin.
#[derive(Default)]
pub struct ListTap {
    origin: Option<(u64, DVec2)>,
}

impl ListTap {
    pub fn cancel(&mut self) {
        self.origin = None;
    }

    fn move_to(&mut self, uid: u64, abs: DVec2) {
        if self.origin.is_some_and(|(origin_uid, origin)| {
            uid != origin_uid || (abs - origin).length() >= TAP_COUNT_DISTANCE
        }) {
            self.cancel();
        }
    }

    fn release(&mut self, uid: u64, abs: DVec2, rect: Rect) -> bool {
        self.move_to(uid, abs);
        self.origin.take().is_some() && rect.contains(abs)
    }

    /// Gate child click actions while still forwarding every release to clear capture.
    pub fn allows_click(&mut self, event: &Event, rect: Rect) -> bool {
        match event {
            Event::TouchUpdate(event) => {
                if event.touches.len() != 1 {
                    self.cancel();
                    return false;
                }
                let touch = &event.touches[0];
                match touch.state {
                    TouchState::Start => {
                        self.origin = rect.contains(touch.abs).then_some((touch.uid, touch.abs));
                    }
                    TouchState::Move | TouchState::Stable => self.move_to(touch.uid, touch.abs),
                    TouchState::Stop => return self.release(touch.uid, touch.abs, rect),
                }
                false
            }
            Event::MouseDown(event) => {
                self.origin = (event.button == MouseButton::PRIMARY && rect.contains(event.abs))
                    .then_some((0, event.abs));
                false
            }
            Event::MouseMove(event) => {
                self.move_to(0, event.abs);
                false
            }
            Event::MouseUp(event) => {
                let primary = event.button == MouseButton::PRIMARY;
                self.release(0, event.abs, rect) && primary
            }
            Event::LongPress(_)
            | Event::Scroll(_)
            | Event::MouseLeave(_)
            | Event::WindowLostFocus(_)
            | Event::LiveEdit
            | Event::ScriptReapply => {
                self.cancel();
                false
            }
            _ => true,
        }
    }
}

/// A toolbar tap must never be inferred from a swipe that returns to its origin.
#[derive(Default)]
pub struct ToolbarTap {
    origin: Option<(u64, DVec2, f64)>,
}

impl ToolbarTap {
    pub fn begin(&mut self, uid: u64, abs: DVec2, time: f64) {
        self.origin = Some((uid, abs, time));
    }

    pub fn cancel(&mut self) {
        self.origin = None;
    }

    pub fn move_to(&mut self, uid: u64, abs: DVec2) {
        if self.origin.is_some_and(|(origin_uid, origin, _)| {
            uid != origin_uid || (abs - origin).length() > 10.0
        }) {
            self.cancel();
        }
    }

    pub fn end(&mut self, uid: u64, abs: DVec2, time: f64) -> bool {
        self.origin
            .take()
            .is_some_and(|(origin_uid, origin, started)| {
                uid == origin_uid
                    && (abs - origin).length() <= 10.0
                    && (0.0..=0.4).contains(&(time - started))
            })
    }
}

#[derive(Debug)]
pub struct TouchSelection {
    pub uid: u64,
    origin: DVec2,
    word: (Position, Position),
    dragging: bool,
}

impl TouchSelection {
    pub fn new(uid: u64, origin: DVec2, word: (Position, Position)) -> Self {
        Self {
            uid,
            origin,
            word,
            dragging: false,
        }
    }

    pub fn range_at(&mut self, abs: DVec2, hit: Position) -> (Position, Position) {
        self.dragging |= (abs - self.origin).length() > 8.0;
        if !self.dragging || (self.word.0..=self.word.1).contains(&hit) {
            return self.word;
        }
        if hit < self.word.0 {
            (hit, self.word.1)
        } else {
            (self.word.0, hit)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use makepad_widgets::dvec2;

    fn list_touch(uid: u64, state: TouchState, abs: DVec2) -> Event {
        use makepad_widgets::makepad_platform::event::{TouchPoint, TouchUpdateEvent};
        use makepad_widgets::WindowId;
        Event::TouchUpdate(TouchUpdateEvent {
            time: 1.0,
            window_id: WindowId(0, 0),
            modifiers: Default::default(),
            touches: vec![TouchPoint {
                state,
                abs,
                time: 1.0,
                uid,
                rotation_angle: 0.0,
                force: 1.0,
                radius: dvec2(1.0, 1.0),
                handled: Default::default(),
                sweep_lock: Default::default(),
            }],
        })
    }

    fn list_rect() -> Rect {
        Rect {
            pos: dvec2(0.0, 0.0),
            size: dvec2(300.0, 600.0),
        }
    }

    #[test]
    fn list_scroll_and_backtracking_never_activate_the_released_row() {
        for end in [dvec2(100.0, 170.0), dvec2(100.0, 200.0)] {
            let mut tap = ListTap::default();
            tap.allows_click(
                &list_touch(1, TouchState::Start, dvec2(100.0, 200.0)),
                list_rect(),
            );
            tap.allows_click(
                &list_touch(1, TouchState::Move, dvec2(100.0, 170.0)),
                list_rect(),
            );
            assert!(!tap.allows_click(&list_touch(1, TouchState::Stop, end), list_rect()));
        }
    }

    #[test]
    fn list_tap_tolerates_jitter_and_recovers_after_cancelled_scroll() {
        let mut tap = ListTap::default();
        tap.allows_click(
            &list_touch(1, TouchState::Start, dvec2(100.0, 200.0)),
            list_rect(),
        );
        tap.allows_click(
            &list_touch(1, TouchState::Move, dvec2(100.0, 195.0)),
            list_rect(),
        );
        assert!(!tap.allows_click(
            &list_touch(1, TouchState::Stop, dvec2(100.0, 200.0)),
            list_rect()
        ));
        tap.allows_click(
            &list_touch(2, TouchState::Start, dvec2(100.0, 200.0)),
            list_rect(),
        );
        tap.allows_click(
            &list_touch(2, TouchState::Move, dvec2(102.0, 201.0)),
            list_rect(),
        );
        assert!(tap.allows_click(
            &list_touch(2, TouchState::Stop, dvec2(103.0, 201.0)),
            list_rect()
        ));
        assert!(!tap.allows_click(
            &list_touch(2, TouchState::Stop, dvec2(103.0, 201.0)),
            list_rect()
        ));
    }

    #[test]
    fn list_touch_that_stops_a_flick_does_not_activate_a_chapter() {
        let mut tap = ListTap::default();
        let start = list_touch(1, TouchState::Start, dvec2(100.0, 200.0));
        let stop = list_touch(1, TouchState::Stop, dvec2(100.0, 200.0));
        tap.allows_click(&start, list_rect());
        // PortalList reports was_scrolling on the down event that stops a flick.
        tap.cancel();
        assert!(!tap.allows_click(&stop, list_rect()));
        tap.allows_click(&start, list_rect());
        assert!(tap.allows_click(&stop, list_rect()));
    }

    #[test]
    fn list_multi_touch_and_layout_reload_cancel_pending_activation() {
        let mut tap = ListTap::default();
        let start = list_touch(1, TouchState::Start, dvec2(100.0, 200.0));
        let stop = list_touch(1, TouchState::Stop, dvec2(100.0, 200.0));
        tap.allows_click(&start, list_rect());
        let mut multi = list_touch(2, TouchState::Start, dvec2(150.0, 200.0));
        if let (Event::TouchUpdate(multi), Event::TouchUpdate(start)) = (&mut multi, &start) {
            multi.touches.push(start.touches[0].clone());
        }
        assert!(!tap.allows_click(&multi, list_rect()));
        assert!(!tap.allows_click(&stop, list_rect()));
        tap.allows_click(&start, list_rect());
        tap.allows_click(&Event::LiveEdit, list_rect());
        assert!(!tap.allows_click(&stop, list_rect()));
    }

    #[test]
    fn ordinary_tap_tolerates_jitter_but_does_not_accept_long_press() {
        let mut tap = ToolbarTap::default();
        tap.begin(1, dvec2(100.0, 200.0), 1.0);
        tap.move_to(1, dvec2(102.0, 201.0));
        assert!(tap.end(1, dvec2(103.0, 201.0), 1.2));
        assert!(!tap.end(1, dvec2(103.0, 201.0), 1.3));
        tap.begin(1, dvec2(100.0, 200.0), 2.0);
        assert!(!tap.end(1, dvec2(100.0, 200.0), 2.5));
    }

    #[test]
    fn swipe_backtracking_other_fingers_and_cancel_are_not_toolbar_taps() {
        let mut tap = ToolbarTap::default();
        tap.begin(1, dvec2(100.0, 200.0), 1.0);
        tap.move_to(1, dvec2(130.0, 200.0));
        assert!(!tap.end(1, dvec2(100.0, 200.0), 1.2));
        tap.begin(1, dvec2(100.0, 200.0), 1.0);
        assert!(!tap.end(2, dvec2(100.0, 200.0), 1.2));
        tap.begin(1, dvec2(100.0, 200.0), 1.0);
        tap.cancel();
        assert!(!tap.end(1, dvec2(100.0, 200.0), 1.2));
    }

    #[test]
    fn release_and_finger_jitter_keep_the_whole_word() {
        let word = ((3, 10), (3, 19));
        let mut selection = TouchSelection::new(7, dvec2(100.0, 200.0), word);
        assert_eq!(selection.range_at(dvec2(100.0, 200.0), (3, 10)), word);
        assert_eq!(selection.range_at(dvec2(102.0, 201.0), (3, 13)), word);
        assert_eq!(selection.range_at(dvec2(102.0, 201.0), (3, 20)), word);
    }

    #[test]
    fn drag_extends_in_both_directions_and_across_paragraphs() {
        let mut selection = TouchSelection::new(7, dvec2(100.0, 200.0), ((3, 10), (3, 19)));
        assert_eq!(
            selection.range_at(dvec2(140.0, 200.0), (3, 25)),
            ((3, 10), (3, 25))
        );
        assert_eq!(
            selection.range_at(dvec2(60.0, 200.0), (3, 5)),
            ((3, 5), (3, 19))
        );
        assert_eq!(
            selection.range_at(dvec2(100.0, 250.0), (4, 6)),
            ((3, 10), (4, 6))
        );
        assert_eq!(
            selection.range_at(dvec2(100.0, 150.0), (2, 8)),
            ((2, 8), (3, 19))
        );
        assert_eq!(
            selection.range_at(dvec2(100.0, 200.0), (3, 15)),
            ((3, 10), (3, 19))
        );
    }
}
