//! Bounded CPU-only preparation for steady scrolling. Native TextFlow still
//! owns glyph layout, selection, ruby slots and all measured geometry.
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::Arc,
};

pub struct PreparedParagraph {
    pub html: String,
    pub text: String,
    pub tracked: bool,
}

#[derive(Default)]
pub struct ParagraphCache {
    entries: HashMap<usize, (u64, Arc<PreparedParagraph>)>,
    bytes: usize,
}

impl ParagraphCache {
    const MAX_BYTES: usize = 2 * 1024 * 1024;
    const MAX_ENTRIES: usize = 64;

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    pub fn get(&mut self, block: usize, html: &str, indent: bool) -> Arc<PreparedParagraph> {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        html.hash(&mut hash);
        indent.hash(&mut hash);
        let key = hash.finish();
        if let Some((_, entry)) = self.entries.get(&block).filter(|(old, _)| *old == key) {
            return entry.clone();
        }
        let display = crate::reader_inline_code::wrap_html(&crate::reader_paragraph::display_html(
            &crate::reader_lists::normalize_html(html),
            indent,
        ));
        let tracked = crate::reader_hit::tracked_html(&display);
        let entry = Arc::new(PreparedParagraph {
            tracked: tracked.is_some(),
            html: tracked.unwrap_or(display),
            text: readwise_core::vocab::annotate::rendered_text(html),
        });
        let size = entry.html.len() + entry.text.len();
        if let Some((_, old)) = self.entries.remove(&block) {
            self.bytes -= old.html.len() + old.text.len();
        }
        if size <= Self::MAX_BYTES {
            if self.entries.len() >= Self::MAX_ENTRIES || self.bytes + size > Self::MAX_BYTES {
                self.clear();
            }
            self.bytes += size;
            self.entries.insert(block, (key, entry.clone()));
        }
        entry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_preparation_is_identical_to_the_existing_projection() {
        let mut cache = ParagraphCache::default();
        for (block, html) in [
            "<p>A <ruby>reader\u{e000}读者</ruby> returns.</p>",
            "<p><code>let value = 2;</code> and <a href=\"#note\">a note</a>.</p>",
            "<ul><li>one</li><li>雪</li></ul>",
            "<reader_math_hidden m=\"book/0/pdf-figure-x\">unchanged labels</reader_math_hidden>",
        ]
        .into_iter()
        .enumerate()
        {
            for indent in [false, true] {
                let a = cache.get(block, html, indent);
                let b = cache.get(block, html, indent);
                assert!(Arc::ptr_eq(&a, &b));
                let old =
                    crate::reader_inline_code::wrap_html(&crate::reader_paragraph::display_html(
                        &crate::reader_lists::normalize_html(html),
                        indent,
                    ));
                let tracked = crate::reader_hit::tracked_html(&old);
                assert_eq!(a.tracked, tracked.is_some());
                assert_eq!(a.html, tracked.unwrap_or(old));
                assert_eq!(a.text, readwise_core::vocab::annotate::rendered_text(html));
            }
        }
    }

    #[test]
    fn changed_ruby_indent_and_reused_block_ids_never_reuse_stale_markup() {
        let mut cache = ParagraphCache::default();
        let a = cache.get(0, "<p><ruby>reader\u{e000}读者</ruby></p>", false);
        let known = cache.get(0, "<p><ruby>reader\u{e000}</ruby></p>", false);
        assert!(!Arc::ptr_eq(&a, &known));
        assert_eq!(a.text, known.text);
        assert!(!known.html.contains("读者"));
        let indented = cache.get(0, "<p><ruby>reader\u{e000}</ruby></p>", true);
        assert!(!Arc::ptr_eq(&known, &indented));
        assert_eq!(cache.get(0, "<p>Other book</p>", false).text, "Other book");
        cache.clear();
        assert!(cache.entries.is_empty());
        assert_eq!(cache.bytes, 0);
    }

    #[test]
    fn cache_is_bounded_and_does_not_retain_an_oversized_paragraph() {
        let mut cache = ParagraphCache::default();
        for i in 0..512 {
            cache.get(i, &format!("<p>Paragraph {i}</p>"), false);
            assert!(cache.entries.len() <= ParagraphCache::MAX_ENTRIES);
            assert!(cache.bytes <= ParagraphCache::MAX_BYTES);
        }
        let html = format!("<p>{}</p>", "a".repeat(ParagraphCache::MAX_BYTES));
        assert_eq!(
            cache.get(9999, &html, false).text.len(),
            ParagraphCache::MAX_BYTES
        );
        assert!(!cache.entries.contains_key(&9999));
    }
}
