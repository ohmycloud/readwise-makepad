//! Touch selection starts with a whole word; a normal swipe remains scrolling.
use makepad_widgets::DVec2;

type Position = (usize, usize);

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
