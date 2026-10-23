//! Reader geometry is a projection: page slices retain the original block IDs.
use readwise_core::settings::ReaderColumns;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ColumnLayout {
    pub count: usize,
    pub width: f64,
    pub height: f64,
    pub left: f64,
    pub top: f64,
    pub gap: f64,
}

impl ColumnLayout {
    #[cfg(test)]
    pub fn resolve(
        mode: ReaderColumns,
        width: f64,
        height: f64,
        font: f64,
        previous: usize,
    ) -> Self {
        Self::resolve_with_margin(mode, width, height, font, previous, 28.0)
    }

    pub fn resolve_with_margin(
        mode: ReaderColumns,
        width: f64,
        height: f64,
        font: f64,
        previous: usize,
        margin: f64,
    ) -> Self {
        let width = if width.is_finite() {
            width.max(1.0)
        } else {
            1.0
        };
        let height = if height.is_finite() {
            height.max(1.0)
        } else {
            1.0
        };
        let font = if font.is_finite() {
            font.clamp(8.0, 20.0)
        } else {
            12.0
        };
        let edge = if width < 600.0 {
            (margin * (24.0 / 28.0)).min(width * 0.15)
        } else {
            margin
        }
        .clamp(12.0, 80.0);
        let top = (margin - 20.0).clamp(8.0, 60.0);
        let gap = 40.0;
        let minimum = (font * 25.0).max(300.0);
        let usable = (width - edge * 2.0).max(1.0);
        let fits = usable >= minimum * 2.0 + gap && height >= 280.0;
        let auto_threshold = (font * 36.0).max(420.0) * 2.0 + gap;
        let count = match mode {
            ReaderColumns::Two if fits => 2,
            ReaderColumns::Auto
                if fits && usable >= auto_threshold + if previous == 2 { -40.0 } else { 40.0 } =>
            {
                2
            }
            _ => 1,
        };
        // Use 56px per side at the default setting for a desktop single column.
        // Narrow windows retain compact margins; font size never caps the width.
        let column_width = if count == 1 {
            let single_edge = if width < 600.0 {
                edge
            } else {
                (margin * 2.0).clamp(12.0, 80.0)
            };
            (width - single_edge * 2.0).max(1.0)
        } else {
            ((usable - gap) / 2.0).min(font * 60.0)
        };
        Self {
            count,
            width: column_width,
            height: (height - top * 2.0).max(1.0),
            left: (width - column_width * count as f64 - (count - 1) as f64 * gap) * 0.5,
            top,
            gap,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PageAnchor {
    pub block: usize,
    /// Display offset within the original (unsplit) block.
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineBoundary {
    pub y: f64,
    pub source: usize,
}

#[derive(Clone, Debug, Default)]
pub struct BlockMetrics {
    pub height: f64,
    /// Safe boundaries between complete rendered lines, including ruby space.
    pub boundaries: Vec<LineBoundary>,
    pub keep_together: bool,
}

impl BlockMetrics {
    pub fn source_at(&self, y: f64) -> usize {
        self.boundaries
            .iter()
            .rev()
            .find(|b| b.y <= y + 0.5)
            .map_or(0, |b| b.source)
    }

    pub fn y_at(&self, source: usize) -> f64 {
        self.boundaries
            .iter()
            .rev()
            .find(|b| b.source <= source)
            .map_or(0.0, |b| b.y)
    }
}

#[derive(Clone, Debug)]
pub struct PageFragment {
    pub block: usize,
    pub from: f64,
    pub to: f64,
    pub top: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Page {
    pub fragments: Vec<PageFragment>,
    pub next: PageAnchor,
}

/// Returns the missing block instead of guessing heights. Measurements always
/// come from the same Html/CodeView templates that paint the reader.
pub fn compose_page(
    metrics: &[Option<BlockMetrics>],
    start: PageAnchor,
    height: f64,
) -> Result<Page, usize> {
    let height = if height.is_finite() {
        height.max(1.0)
    } else {
        1.0
    };
    let mut page = Page {
        next: start,
        ..Page::default()
    };
    let mut top = 0.0;
    while page.next.block < metrics.len() {
        let block = page.next.block;
        let metric = metrics[block].as_ref().ok_or(block)?;
        let block_height = if metric.height.is_finite() {
            metric.height.max(0.0)
        } else {
            0.0
        };
        let from = page.next.y.max(0.0).min(block_height);
        let remaining = (block_height - from).max(0.0);
        if remaining < 0.01 {
            page.next = PageAnchor {
                block: block + 1,
                y: 0.0,
            };
            continue;
        }
        let available = height - top;
        if metric.keep_together
            && from == 0.0
            && top > 0.0
            && remaining > available
            && remaining <= height
        {
            break;
        }
        let to = if remaining <= available + 0.01 {
            block_height
        } else if let Some(boundary) = metric
            .boundaries
            .iter()
            .rev()
            .find(|b| b.y > from + 0.5 && b.y <= from + available)
        {
            boundary.y
        } else if top > 0.0 {
            break; // Keep an image/table/short block intact on the next page.
        } else {
            // Oversized atomic media has no line boundary. Still make progress;
            // its original remains available in the image viewer.
            (from + available).min(block_height)
        };
        page.fragments.push(PageFragment {
            block,
            from,
            to,
            top,
        });
        top += to - from;
        if to >= block_height - 0.01 {
            page.next = PageAnchor {
                block: block + 1,
                y: 0.0,
            };
            top += 6.0;
        } else {
            page.next = PageAnchor { block, y: to };
            break;
        }
        if top >= height - 0.5 {
            break;
        }
    }
    Ok(page)
}

/// A chapter index advances from its measured frontier. Unmeasured pages are
/// never counted, and a redraw does not recompose all the preceding pages.
#[derive(Clone, Default)]
pub struct PageIndex {
    pub starts: Vec<PageAnchor>,
    pub complete: bool,
    next: PageAnchor,
}

impl PageIndex {
    pub fn page_number(&self, anchor: PageAnchor) -> Option<usize> {
        let start = self.containing(anchor)?;
        if !self.complete && start == self.next {
            return Some(self.starts.len());
        }
        self.starts.iter().position(|page| *page == start)
    }

    pub fn advance(&mut self, metrics: &[Option<BlockMetrics>], height: f64) {
        if self.complete {
            return;
        }
        while self.next.block < metrics.len() {
            let Ok(page) = compose_page(metrics, self.next, height) else {
                return;
            };
            if page.next == self.next {
                return;
            }
            self.starts.push(self.next);
            self.next = page.next;
        }
        self.complete = true;
    }

    /// Restoring a location needs only the prefix up to that location, not the
    /// entire chapter. The frontier itself is already a canonical page start.
    pub fn containing(&self, anchor: PageAnchor) -> Option<PageAnchor> {
        if anchor == self.next {
            return Some(anchor);
        }
        if !self.complete
            && (anchor.block > self.next.block
                || (anchor.block == self.next.block && anchor.y > self.next.y))
        {
            return None;
        }
        self.starts
            .iter()
            .rev()
            .find(|page| {
                page.block < anchor.block
                    || (page.block == anchor.block && page.y <= anchor.y + 0.5)
            })
            .copied()
    }
}

#[cfg(test)]
fn page_starts(metrics: &[Option<BlockMetrics>], height: f64) -> (Vec<PageAnchor>, bool) {
    let mut index = PageIndex::default();
    index.advance(metrics, height);
    (index.starts, index.complete)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_presets_change_actual_mobile_and_desktop_text_area() {
        use readwise_core::settings::{ReaderSettings, ReaderTextWidth};
        for viewport in [320.0, 390.0, 1100.0] {
            let widths = [
                ReaderTextWidth::Narrow,
                ReaderTextWidth::Even,
                ReaderTextWidth::Wide,
            ]
            .map(|width| {
                ColumnLayout::resolve_with_margin(
                    ReaderColumns::One,
                    viewport,
                    700.0,
                    11.0,
                    1,
                    ReaderSettings::text_margin_pixels(width, 0.0) as f64,
                )
                .width
            });
            assert!(
                widths[0] < widths[1] && widths[1] < widths[2],
                "{viewport}: {widths:?}"
            );
        }
    }

    #[test]
    fn larger_reader_margins_reduce_page_area_without_changing_column_mode() {
        let narrow =
            ColumnLayout::resolve_with_margin(ReaderColumns::Two, 1200.0, 800.0, 14.0, 2, 16.0);
        let wide =
            ColumnLayout::resolve_with_margin(ReaderColumns::Two, 1200.0, 800.0, 14.0, 2, 64.0);
        assert_eq!(narrow.count, 2);
        assert_eq!(wide.count, 2);
        assert!(wide.left > narrow.left);
        assert!(wide.top > narrow.top);
        assert!(wide.width < narrow.width);
        assert!(wide.height < narrow.height);
    }

    #[test]
    fn unfinished_index_does_not_block_a_measured_spread() {
        for columns in [1, 2] {
            let mut metrics = vec![None; 1000];
            metrics[0] = block(100);
            let mut index = PageIndex::default();
            index.advance(&metrics, 100.0);
            assert!(!index.complete);
            let mut anchor = PageAnchor::default();
            for _ in 0..4 {
                assert_eq!(index.containing(anchor), Some(anchor));
                for _ in 0..columns {
                    let page = compose_page(&metrics, anchor, 100.0).unwrap();
                    assert_ne!(page.next, anchor);
                    anchor = page.next;
                }
            }
        }
    }

    #[test]
    fn incremental_index_matches_full_layout_without_counting_incomplete_pages() {
        let full = vec![block(8), block(1), block(14), block(3)];
        for height in [50.0, 105.0, 230.0, 900.0] {
            let mut metrics = vec![None; full.len()];
            let mut index = PageIndex::default();
            index.advance(&metrics, height);
            assert!(index.starts.is_empty());
            assert!(!index.complete);
            for block in [2, 0, 3, 1] {
                let prefix = index.starts.clone();
                metrics[block] = full[block].clone();
                index.advance(&metrics, height);
                assert!(index.starts.starts_with(&prefix));
                let starts = index.starts.clone();
                index.advance(&metrics, height);
                assert_eq!(index.starts, starts, "redraw must not duplicate pages");
            }
            assert!(index.complete);
            assert_eq!(index.starts, page_starts(&full, height).0);
        }
    }

    #[test]
    fn restoring_an_anchor_requires_only_its_prefix() {
        let metrics = [block(15), None, None];
        let mut index = PageIndex::default();
        index.advance(&metrics, 100.0);
        assert!(!index.complete);
        assert_eq!(
            index.containing(PageAnchor::default()),
            Some(PageAnchor::default())
        );
        assert_eq!(
            index.containing(PageAnchor { block: 0, y: 142.0 }),
            Some(PageAnchor { block: 0, y: 100.0 })
        );
        assert_eq!(index.containing(PageAnchor { block: 2, y: 0.0 }), None);
    }

    #[test]
    fn short_code_moves_whole_but_long_code_breaks_only_at_line_boundaries() {
        let mut code = block(4).unwrap();
        code.keep_together = true;
        let first = compose_page(
            &[block(3), Some(code.clone())],
            PageAnchor::default(),
            100.0,
        )
        .unwrap();
        assert_eq!(first.next, PageAnchor { block: 1, y: 0.0 });
        code.height = 180.0;
        let split = compose_page(&[Some(code)], PageAnchor::default(), 55.0).unwrap();
        assert_eq!(split.next, PageAnchor { block: 0, y: 40.0 });
    }

    #[test]
    fn malformed_geometry_cannot_make_the_page_index_loop_forever() {
        let metrics = [
            Some(BlockMetrics {
                height: f64::INFINITY,
                ..Default::default()
            }),
            block(1),
        ];
        let (pages, complete) = page_starts(&metrics, f64::NAN);
        assert!(complete);
        assert!(pages.len() <= 21);
    }
    fn block(lines: usize) -> Option<BlockMetrics> {
        Some(BlockMetrics {
            height: lines as f64 * 20.0,
            boundaries: (1..lines)
                .map(|i| LineBoundary {
                    y: i as f64 * 20.0,
                    source: i * 10,
                })
                .collect(),
            keep_together: false,
        })
    }
    #[test]
    fn auto_uses_available_width_and_has_hysteresis() {
        assert_eq!(
            ColumnLayout::resolve(ReaderColumns::Auto, 1100.0, 700.0, 12.0, 1).count,
            2
        );
        assert_eq!(
            ColumnLayout::resolve(ReaderColumns::Auto, 700.0, 700.0, 12.0, 2).count,
            1
        );
        assert_eq!(
            ColumnLayout::resolve(ReaderColumns::Auto, 970.0, 700.0, 12.0, 1).count,
            1
        );
        assert_eq!(
            ColumnLayout::resolve(ReaderColumns::Auto, 970.0, 700.0, 12.0, 2).count,
            2
        );
    }
    #[test]
    fn narrow_two_columns_falls_back_without_changing_the_preference() {
        assert_eq!(
            ColumnLayout::resolve(ReaderColumns::Two, 390.0, 700.0, 12.0, 2).count,
            1
        );
        assert_eq!(
            ColumnLayout::resolve(ReaderColumns::Two, 900.0, 700.0, 12.0, 1).count,
            2
        );
        assert_eq!(
            ColumnLayout::resolve(ReaderColumns::Two, 900.0, 700.0, 20.0, 2).count,
            1
        );
    }
    #[test]
    fn single_column_keeps_configured_margins_at_small_and_large_fonts() {
        for font in [11.0, 20.0] {
            let layout = ColumnLayout::resolve(ReaderColumns::One, 1400.0, 700.0, font, 2);
            assert_eq!(layout.count, 1);
            assert_eq!(layout.width, 1288.0);
            assert_eq!(layout.left, 56.0);
        }
    }
    #[test]
    fn long_paragraph_continues_at_exact_same_boundary() {
        let metrics = vec![block(15), block(2)];
        let first = compose_page(&metrics, PageAnchor::default(), 105.0).unwrap();
        assert_eq!(first.next, PageAnchor { block: 0, y: 100.0 });
        let second = compose_page(&metrics, first.next, 105.0).unwrap();
        assert_eq!(second.fragments[0].from, first.fragments[0].to);
        assert_eq!(second.next.y, 200.0);
        let (starts, complete) = page_starts(&metrics, 105.0);
        assert!(complete);
        assert_eq!(starts.len(), 4);
    }
    #[test]
    fn media_moves_whole_when_it_fits_a_fresh_page() {
        let metrics = vec![
            block(2),
            Some(BlockMetrics {
                height: 80.0,
                ..Default::default()
            }),
        ];
        let first = compose_page(&metrics, PageAnchor::default(), 100.0).unwrap();
        assert_eq!(first.next, PageAnchor { block: 1, y: 0.0 });
        assert_eq!(first.fragments.len(), 1);
    }
    #[test]
    fn unknown_metrics_are_not_treated_as_empty_and_oversized_blocks_progress() {
        assert_eq!(
            compose_page(&[None], PageAnchor::default(), 100.0).unwrap_err(),
            0
        );
        let metrics = vec![Some(BlockMetrics {
            height: 301.0,
            ..Default::default()
        })];
        assert_eq!(page_starts(&metrics, 100.0).0.len(), 4);
        assert!(!page_starts(&[block(2), None], 100.0).1);
    }

    #[test]
    fn page_fragments_cover_the_original_blocks_once_in_source_order() {
        let metrics = vec![block(8), block(1), block(14), block(3)];
        for height in [50.0, 105.0, 230.0, 900.0] {
            let mut anchor = PageAnchor::default();
            let mut covered = vec![0.0; metrics.len()];
            while anchor.block < metrics.len() {
                let page = compose_page(&metrics, anchor, height).unwrap();
                assert_ne!(anchor, page.next);
                for fragment in page.fragments {
                    assert_eq!(fragment.from, covered[fragment.block]);
                    assert!(fragment.top + fragment.to - fragment.from <= height + 0.01);
                    covered[fragment.block] = fragment.to;
                }
                anchor = page.next;
            }
            for (height, metric) in covered.into_iter().zip(&metrics) {
                assert_eq!(height, metric.as_ref().unwrap().height);
            }
        }
    }

    #[test]
    fn reflow_maps_a_source_position_to_the_new_containing_line() {
        let old = block(10).unwrap();
        let new = BlockMetrics {
            height: 200.0,
            boundaries: vec![
                LineBoundary {
                    y: 40.0,
                    source: 20,
                },
                LineBoundary {
                    y: 80.0,
                    source: 40,
                },
                LineBoundary {
                    y: 120.0,
                    source: 60,
                },
            ],
            keep_together: false,
        };
        let source = old.source_at(100.0);
        assert_eq!(source, 50);
        assert_eq!(new.y_at(source), 80.0);
    }
}
