//! Book page numbers come from the same measured pages used for turning.
use crate::reader_columns::{BlockMetrics, PageAnchor, PageIndex};
use std::sync::Arc;

#[derive(Debug)]
pub struct ChapterContent {
    pub paragraphs: Vec<String>,
    pub codes: Vec<Option<String>>,
}

#[derive(Debug)]
pub struct BookContent {
    pub key: u64,
    pub chapters: Vec<ChapterContent>,
}

#[derive(Default)]
struct ChapterPages {
    metrics: Vec<Option<BlockMetrics>>,
    index: PageIndex,
}

#[derive(Default)]
pub struct BookPages {
    pub content_key: u64,
    layout_key: u64,
    content: Option<Arc<BookContent>>,
    chapters: Vec<ChapterPages>,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Summary {
    pub current: usize,
    pub total: Option<usize>,
    pub pages_left_in_chapter: Option<usize>,
}

impl BookPages {
    pub fn configure(&mut self, content_key: u64, layout_key: u64, chapters: usize) {
        let content_changed = self.content_key != content_key || self.chapters.len() != chapters;
        if content_changed {
            self.content = None;
            self.content_key = content_key;
        }
        if content_changed || self.layout_key != layout_key {
            self.layout_key = layout_key;
            self.chapters = (0..chapters).map(|_| ChapterPages::default()).collect();
            self.size_metrics();
            self.revision += 1;
        }
    }

    pub fn set_content(&mut self, content: Arc<BookContent>) {
        if content.key != self.content_key {
            // The UI validates the active book before accepting this result.
            // Preparation can finish before that book's first draw.
            self.configure(content.key, self.layout_key, content.chapters.len());
        }
        self.content = Some(content);
        self.size_metrics();
        self.revision += 1;
    }

    fn size_metrics(&mut self) {
        if let Some(content) = &self.content {
            for (chapter, content) in self.chapters.iter_mut().zip(&content.chapters) {
                if chapter.metrics.len() != content.paragraphs.len() {
                    chapter.metrics = vec![None; content.paragraphs.len()];
                    chapter.index = PageIndex::default();
                }
            }
        }
    }

    pub fn restore(
        &self,
        chapter: usize,
        blocks: usize,
    ) -> Option<(Vec<Option<BlockMetrics>>, PageIndex)> {
        let cached = self.chapters.get(chapter)?;
        (cached.metrics.len() == blocks).then(|| (cached.metrics.clone(), cached.index.clone()))
    }

    pub fn record_current(
        &mut self,
        chapter: usize,
        metrics: &[Option<BlockMetrics>],
        index: &PageIndex,
    ) {
        let Some(cached) = self.chapters.get_mut(chapter) else {
            return;
        };
        if cached.metrics.len() != metrics.len() {
            cached.metrics = vec![None; metrics.len()];
            cached.index = PageIndex::default();
        }
        for (saved, measured) in cached.metrics.iter_mut().zip(metrics) {
            if saved.is_none() && measured.is_some() {
                *saved = measured.clone();
            }
        }
        if cached.index.starts.len() != index.starts.len()
            || cached.index.complete != index.complete
        {
            cached.index = index.clone();
            self.revision += 1;
        }
    }

    /// Earlier chapters establish the exact global offset before later ones.
    pub fn next_work(
        &self,
        current: usize,
        preceding_only: bool,
    ) -> Option<(usize, usize, Arc<BookContent>)> {
        let content = self.content.as_ref()?;
        self.chapters
            .iter()
            .enumerate()
            .filter(|(chapter, _)| *chapter != current && (!preceding_only || *chapter < current))
            .find_map(|(chapter, pages)| {
                let block = pages.metrics.iter().position(Option::is_none)?;
                Some((chapter, block, content.clone()))
            })
    }

    pub fn record_block(
        &mut self,
        chapter: usize,
        block: usize,
        metric: BlockMetrics,
        height: f64,
    ) {
        let pages = &mut self.chapters[chapter];
        let previous = (pages.index.starts.len(), pages.index.complete);
        pages.metrics[block] = Some(metric);
        pages.index.advance(&pages.metrics, height);
        if previous != (pages.index.starts.len(), pages.index.complete) {
            self.revision += 1;
        }
    }

    pub fn complete(&self) -> bool {
        !self.chapters.is_empty() && self.chapters.iter().all(|chapter| chapter.index.complete)
    }

    pub fn summary(
        &self,
        chapter: usize,
        current: &PageIndex,
        anchor: PageAnchor,
        columns: usize,
    ) -> Option<Summary> {
        if chapter >= self.chapters.len()
            || self.chapters[..chapter]
                .iter()
                .any(|chapter| !chapter.index.complete)
        {
            return None;
        }
        let page = current.page_number(anchor)?;
        let before: usize = self.chapters[..chapter]
            .iter()
            .map(|chapter| chapter.index.starts.len())
            .sum();
        Some(Summary {
            current: before + page + 1,
            total: self.complete().then(|| {
                self.chapters
                    .iter()
                    .map(|chapter| chapter.index.starts.len())
                    .sum()
            }),
            pages_left_in_chapter: current
                .complete
                .then(|| current.starts.len().saturating_sub(page + columns)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measured_pages(count: usize) -> (Vec<Option<BlockMetrics>>, PageIndex) {
        let metrics = vec![
            Some(BlockMetrics {
                height: 100.0,
                boundaries: vec![],
                keep_together: false
            });
            count
        ];
        let mut index = PageIndex::default();
        index.advance(&metrics, 100.0);
        (metrics, index)
    }

    #[test]
    fn next_chapter_continues_at_26_despite_different_paragraph_density() {
        let mut book = BookPages::default();
        book.configure(1, 10, 2);
        let (first_metrics, first_index) = measured_pages(25);
        book.record_current(0, &first_metrics, &first_index);
        let (second_metrics, second_index) = measured_pages(16);
        book.record_current(1, &second_metrics, &second_index);
        assert_eq!(
            book.summary(1, &second_index, PageAnchor::default(), 2),
            Some(Summary {
                current: 26,
                total: Some(41),
                pages_left_in_chapter: Some(14),
            })
        );
    }

    #[test]
    fn current_page_does_not_wait_for_the_rest_of_the_chapter_or_book() {
        let mut book = BookPages::default();
        book.configure(1, 10, 2);
        let mut metrics = vec![None; 100];
        metrics[0] = Some(BlockMetrics {
            height: 100.0,
            boundaries: vec![],
            keep_together: false,
        });
        let mut index = PageIndex::default();
        index.advance(&metrics, 100.0);
        assert_eq!(
            book.summary(0, &index, PageAnchor::default(), 1),
            Some(Summary {
                current: 1,
                total: None,
                pages_left_in_chapter: None,
            })
        );
    }

    #[test]
    fn layout_change_invalidates_exact_counts_and_chapter_switch_does_not() {
        let mut book = BookPages::default();
        book.configure(1, 10, 1);
        let (metrics, index) = measured_pages(25);
        book.record_current(0, &metrics, &index);
        book.configure(1, 10, 1);
        assert!(book.complete());
        assert!(book.restore(0, 25).unwrap().1.complete);
        book.configure(1, 11, 1);
        assert!(!book.complete());
    }

    #[test]
    fn preparation_before_first_draw_survives_layout_configuration() {
        let mut book = BookPages::default();
        book.set_content(Arc::new(BookContent {
            key: 1,
            chapters: vec![ChapterContent {
                paragraphs: vec!["<p>Text</p>".into()],
                codes: vec![None],
            }],
        }));
        book.configure(1, 10, 1);
        assert_eq!(
            book.next_work(1, true)
                .map(|(chapter, block, _)| (chapter, block)),
            Some((0, 0))
        );
        let (metrics, index) = measured_pages(1);
        book.record_current(0, &metrics, &index);
        book.configure(2, 10, 1);
        assert!(!book.complete());
        assert!(book.next_work(1, true).is_none());
    }

    #[test]
    fn a_late_chapter_waits_for_measured_offsets_and_reuses_them_when_going_back() {
        let mut book = BookPages::default();
        book.configure(1, 10, 3);
        let (metrics, index) = measured_pages(4);
        book.record_current(2, &metrics, &index);
        assert!(book.summary(2, &index, PageAnchor::default(), 2).is_none());
        let (first, first_index) = measured_pages(3);
        let (second, second_index) = measured_pages(2);
        book.record_current(0, &first, &first_index);
        book.record_current(1, &second, &second_index);
        assert_eq!(
            book.summary(2, &index, PageAnchor::default(), 2)
                .unwrap()
                .current,
            6
        );
        let (_, restored) = book.restore(1, 2).unwrap();
        assert_eq!(
            book.summary(1, &restored, PageAnchor::default(), 2)
                .unwrap()
                .current,
            4
        );
    }
}
