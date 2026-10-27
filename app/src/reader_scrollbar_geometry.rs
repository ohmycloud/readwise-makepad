/// One geometry for the native shader, thumb/rail hit testing and pointer
/// conversion. The content range and the list's height tree are never inflated.
#[derive(Default, Debug)]
pub(super) struct ThumbGeometry {
    track: f64,
    handle: f64,
    range: f64,
}
impl ThumbGeometry {
    pub(super) fn new(track: f64, total: f64, visible: f64, minimum: f64, maximum: f64) -> Self {
        if ![track, total, visible, minimum, maximum]
            .iter()
            .all(|v| v.is_finite())
            || track <= 0.
            || visible < 0.
            || total <= visible
        {
            return Self::default();
        }
        let cap = if maximum > 0. {
            maximum.min(track)
        } else {
            track
        };
        let handle = (track * visible / total).max(minimum.max(0.)).min(cap);
        Self {
            track,
            handle,
            range: total - visible,
        }
    }
    pub(super) fn normalized(&self, position: f64) -> (f64, f64) {
        if self.track <= 0. || self.range <= 0. {
            return (0., 0.);
        }
        let progress = (position / self.range).clamp(0., 1.);
        (
            (self.track - self.handle) / self.track * progress,
            self.handle / self.track,
        )
    }
    pub(super) fn content_from_finger(&self, top: f64) -> f64 {
        let travel = self.track - self.handle;
        if travel <= 0. || !top.is_finite() {
            return 0.;
        }
        (top / travel).clamp(0., 1.) * self.range
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn short_chapters_never_grow_a_screen_height_thumb() {
        for (track, total, visible) in [
            (994., 1100., 1000.),
            (694., 1400., 700.),
            (314., 640., 320.),
        ] {
            let g = ThumbGeometry::new(track, total, visible, 24., 40.);
            assert_eq!(g.handle, 40.);
            for progress in [0., 0.1, 0.5, 0.9, 1.] {
                let (start, size) = g.normalized(g.range * progress);
                assert!((size * track - 40.).abs() < 1e-9);
                assert!((g.content_from_finger(start * track) - g.range * progress).abs() < 1e-8);
            }
            let (start, size) = g.normalized(g.range);
            assert!((start + size - 1.).abs() < 1e-12);
        }
    }
    #[test]
    fn long_chapter_thumb_stays_usable_and_drag_reaches_both_ends() {
        let g = ThumbGeometry::new(994., 1_000_000., 1000., 24., 40.);
        assert_eq!(g.handle, 24.);
        assert_eq!(g.content_from_finger(-10.), 0.);
        assert_eq!(g.content_from_finger(2000.), 999_000.);
        assert!((g.content_from_finger(485.) - 499_500.).abs() < 1e-9);
    }
    #[test]
    fn no_cap_matches_the_native_proportion_and_minimum() {
        for total in [1100., 2000., 10_000., 1_000_000.] {
            let g = ThumbGeometry::new(994., total, 1000., 30., 0.);
            assert!((g.handle / 994. - (1000. / total).max(30. / 994.)).abs() < 1e-12);
        }
    }
    #[test]
    fn resize_hidden_and_tiny_tracks_never_make_nan_or_oversized_handles() {
        for track in [0., 1., 10., 24., 40., 314., 694., 994.] {
            let g = ThumbGeometry::new(track, 2000., 1000., 24., 40.);
            assert!(g.handle <= track.min(40.));
            assert!(g.normalized(500.).0.is_finite());
            assert!(g.content_from_finger(10.).is_finite());
        }
        for total in [0., 500., 1000.] {
            let g = ThumbGeometry::new(994., total, 1000., 24., 40.);
            assert_eq!(g.normalized(0.), (0., 0.));
        }
    }
}
