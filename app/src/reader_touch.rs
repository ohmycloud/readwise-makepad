//! Touch selection starts with a whole word; a normal swipe remains scrolling.
use makepad_widgets::DVec2;

type Position = (usize, usize);

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
