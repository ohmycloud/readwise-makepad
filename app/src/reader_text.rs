//! Translate persisted source offsets to TextFlow's actual selection buffer.
//! Block breaks and object placeholders belong to TextFlow, not to the EPUB
//! text. A single leading-newline adjustment cannot represent this mapping.
use makepad_widgets::TextFlow;
use std::ops::Range;

#[derive(Debug)]
struct Span {
    source: Range<usize>,
    native: Range<usize>,
}

#[derive(Debug)]
pub struct TextMap {
    source: String,
    native: String,
    spans: Vec<Span>,
}

impl TextMap {
    pub fn new(source: &str, native: String) -> Option<Self> {
        Self::with_list_items(source, native, &[])
    }

    pub fn from_html(source: &str, html: &str, native: String) -> Option<Self> {
        let items = list_item_offsets(html);
        if items.is_empty() {
            Self::new(source, native)
        } else {
            Self::with_list_items(source, native, &items)
        }
    }

    fn with_list_items(source: &str, native: String, items: &[usize]) -> Option<Self> {
        let mut source_chars = source.char_indices().peekable();
        let mut native_chars = native.char_indices().peekable();
        let mut items = items.iter().peekable();
        let mut spans: Vec<Span> = Vec::new();
        loop {
            // TextFlow::begin_list_item paints/registers its generated marker
            // as text. Exclude it only at an actual <li> source boundary, never
            // by guessing that punctuation or numbers in prose are markers.
            let source_offset = source_chars.peek().map(|(i, _)| *i).unwrap_or(source.len());
            while items.peek().is_some_and(|offset| **offset == source_offset) {
                while native_chars
                    .peek()
                    .is_some_and(|(_, ch)| ch.is_whitespace())
                {
                    native_chars.next();
                }
                let n = native_chars.peek()?.0;
                let len = list_marker_len(&native[n..])?;
                while native_chars.peek().is_some_and(|(i, _)| *i < n + len) {
                    native_chars.next();
                }
                items.next();
            }
            match (source_chars.peek().copied(), native_chars.peek().copied()) {
                (Some((s, a)), Some((n, b))) if a == b => {
                    let source_end = s + a.len_utf8();
                    let native_end = n + b.len_utf8();
                    if let Some(last) = spans
                        .last_mut()
                        .filter(|last| last.source.end == s && last.native.end == n)
                    {
                        last.source.end = source_end;
                        last.native.end = native_end;
                    } else {
                        spans.push(Span {
                            source: s..source_end,
                            native: n..native_end,
                        });
                    }
                    source_chars.next();
                    native_chars.next();
                }
                // Only whitespace normalization and non-text objects may differ.
                // Never search ahead for repeated words or hide missing letters.
                (_, Some((_, b))) if b.is_whitespace() || b == '\u{fffc}' => {
                    native_chars.next();
                }
                (Some((_, a)), _) if a.is_whitespace() => {
                    source_chars.next();
                }
                (None, None) if items.peek().is_none() => break,
                _ => return None,
            }
        }
        Some(Self {
            source: source.into(),
            native,
            spans,
        })
    }

    pub fn matches_native(&self, native: &str) -> bool {
        self.native == native
    }

    pub fn source_index(&self, native: usize) -> usize {
        let native = self
            .native
            .floor_char_boundary(native.min(self.native.len()));
        let next = self.spans.partition_point(|span| span.native.end < native);
        if let Some(span) = self.spans.get(next) {
            span.source.start + native.saturating_sub(span.native.start)
        } else {
            self.source.len()
        }
    }

    pub fn native_range(&self, range: (usize, usize)) -> Option<(usize, usize)> {
        let from = self
            .source
            .floor_char_boundary(range.0.min(self.source.len()));
        let to = self
            .source
            .floor_char_boundary(range.1.min(self.source.len()));
        if from >= to {
            return None;
        }
        // Start after inserted separators, end before them. This affinity is
        // important when two source paragraphs have no literal newline between.
        let first = self
            .spans
            .get(self.spans.partition_point(|span| span.source.end <= from))?;
        let last = self.spans.get(
            self.spans
                .partition_point(|span| span.source.start < to)
                .checked_sub(1)?,
        )?;
        let start = first.native.start + from.saturating_sub(first.source.start);
        let end = last.native.start + (to.min(last.source.end) - last.source.start);
        (start < end).then_some((start, end))
    }
}

fn list_marker_len(text: &str) -> Option<usize> {
    // Pinned Html defaults: bullets "•" / "-", decimal/alpha/Roman ordinals
    // followed by '.', and '#' for unsupported numbering depth.
    if let Some(dot) = text.find('.').filter(|dot| *dot > 0 && *dot <= 32) {
        let ordinal = text[..dot].trim_start_matches('-');
        if !ordinal.is_empty() && ordinal.chars().all(|ch| ch.is_ascii_alphanumeric()) {
            return Some(dot + 1);
        }
    }
    let ch = text.chars().next()?;
    matches!(ch, '•' | '-' | '#').then_some(ch.len_utf8())
}

fn list_item_offsets(html: &str) -> Vec<usize> {
    let mut offsets = Vec::new();
    let mut cursor = 0;
    while let Some(start) = html[cursor..].find('<').map(|i| cursor + i) {
        let mut quote = None;
        let Some(end) = html[start + 1..].char_indices().find_map(|(i, ch)| {
            if let Some(q) = quote {
                if ch == q {
                    quote = None;
                }
            } else if ch == '\'' || ch == '"' {
                quote = Some(ch);
            } else if ch == '>' {
                return Some(start + 1 + i);
            }
            None
        }) else {
            break;
        };
        let tag = html[start + 1..end]
            .split_ascii_whitespace()
            .next()
            .unwrap_or("");
        if tag.eq_ignore_ascii_case("li") {
            offsets.push(readwise_core::vocab::annotate::rendered_text(&html[..start]).len());
        }
        cursor = end + 1;
    }
    offsets
}

/// The public full-text API strips U+FFFC, but selection offsets still count
/// its three UTF-8 bytes. Preserve those holes without accessing private fields.
/// Most paragraphs have no objects and take the single-string fast path.
pub fn native_text(flow: &TextFlow) -> Option<String> {
    let visible = flow.get_full_text();
    let len = flow.text_len();
    restore_object_gaps(visible, len, |start, end| {
        flow.get_text_for_range(start, end)
    })
}

fn restore_object_gaps(
    visible: String,
    len: usize,
    read: impl Fn(usize, usize) -> String,
) -> Option<String> {
    if visible.len() == len {
        return Some(visible);
    }
    let mut native = String::with_capacity(len);
    let mut offset = 0;
    for ch in visible.chars() {
        let width = ch.len_utf8();
        while read(offset, offset + width) != ch.to_string() {
            if offset + 3 > len || !read(offset, offset + 3).is_empty() {
                return None;
            }
            native.push('\u{fffc}');
            offset += 3;
        }
        native.push(ch);
        offset += width;
    }
    while offset + 3 <= len {
        native.push('\u{fffc}');
        offset += 3;
    }
    (offset == len).then_some(native)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_selection(source: &str, native: &str, phrase: &str) {
        let map = TextMap::new(source, native.into()).expect("all body letters must match");
        let from = source.find(phrase).unwrap();
        let to = from + phrase.len();
        let (a, b) = map.native_range((from, to)).unwrap();
        assert_eq!(&native[a..b], phrase);
        assert_eq!(map.source_index(a), from);
        assert_eq!(map.source_index(b), to);
        // Character-by-character forward selection and backtracking use the
        // same bijection, including UTF-8 punctuation and the last character.
        for offset in phrase.char_indices().map(|(i, _)| i).chain([phrase.len()]) {
            assert_eq!(map.source_index(a + offset), from + offset);
            if offset > 0 {
                assert_eq!(
                    map.native_range((from, from + offset)),
                    Some((a, a + offset))
                );
            }
        }
    }

    #[test]
    fn sample_heading_and_multiple_paragraphs_keep_last_line_offsets() {
        let html = crate::sample_chapter::build_glossed_paragraph();
        let source = readwise_core::vocab::annotate::rendered_text(&html);
        let title_end = source.find("下面").unwrap();
        let body_start = source.find("The lighthouse").unwrap();
        let native = format!(
            "\n{}\n\n{}\n\n{}\n",
            &source[..title_end],
            &source[title_end..body_start],
            &source[body_start..]
        );
        assert_selection(
            &source,
            &native,
            "tempered only by the resilience he had earned from years of keeping vigil alone.",
        );
    }

    #[test]
    fn single_paragraph_and_callout_mixed_paragraph_keep_offsets() {
        assert_selection(
            "Exit as quickly as possible if HEAT_ON is off.",
            "\nExit as quickly as possible if HEAT_ON is off.\n",
            "HEAT_ON is off.",
        );
        assert_selection(
            "\nExit as quickly as possible.",
            "\u{fffc}\nExit as quickly as possible.\n",
            "quickly as possible.",
        );
    }

    #[test]
    fn headings_quotes_lists_and_unicode_use_all_block_breaks() {
        assert_selection(
            "Title\nFirst paragraph.引文。item oneitem two",
            "\nTitle\n\nFirst paragraph.\n\n引文。\n\nitem one\nitem two\n",
            "引文。",
        );
        assert_selection(
            "Title\nFirst paragraph.引文。item oneitem two",
            "\nTitle\n\nFirst paragraph.\n\n引文。\n\nitem one\nitem two\n",
            "item two",
        );
        assert_selection(
            "before中文，é🙂alone.after",
            "\nbefore\n\n中文，é🙂alone.\n\nafter\n",
            "中文，é🙂alone.",
        );
    }

    #[test]
    fn range_edges_exclude_inserted_breaks_and_collapsed_spaces() {
        let map = TextMap::new("one   two", "\none two\n".into()).unwrap();
        assert_eq!(map.native_range((6, 9)), Some((5, 8)));
        assert_eq!(map.native_range((0, 9)), Some((1, 8)));
        assert_eq!(map.native_range((3, 3)), None);
        let map = TextMap::new("onetwo", "\none\n\ntwo\n".into()).unwrap();
        assert_eq!(map.native_range((0, 3)), Some((1, 4)));
        assert_eq!(map.native_range((3, 6)), Some((6, 9)));
    }

    #[test]
    fn missing_letters_or_reordered_words_are_not_silently_aligned() {
        assert!(TextMap::new("alone.", "\nalo".into()).is_none());
        assert!(TextMap::new("one two one", "one one two".into()).is_none());
        assert!(TextMap::new("标题正文", "\n正文".into()).is_none());
    }

    #[test]
    fn generated_list_markers_are_skipped_only_at_html_list_item_boundaries() {
        let html = "<h2>Title</h2><ul><li>• real bullet</li><li><p>第二项</p><ol type='A'><li>A. real text</li></ol></li></ul><p>alone.</p>";
        let source = readwise_core::vocab::annotate::rendered_text(html);
        let native = "\nTitle\n\n•• real bullet\n\n•\n第二项\n\nA.A. real text\n\n\nalone.\n";
        let map = TextMap::from_html(&source, html, native.into()).unwrap();
        for phrase in ["• real bullet", "第二项", "A. real text", "alone."] {
            let start = source.find(phrase).unwrap();
            let (a, b) = map.native_range((start, start + phrase.len())).unwrap();
            assert_eq!(&native[a..b], phrase);
            assert_eq!(map.source_index(a), start);
        }
        assert!(TextMap::new("real text", "1.real text".into()).is_none());
        assert!(
            TextMap::from_html("real text", "<p>real text</p>", "1.real text".into()).is_none()
        );
    }

    #[test]
    fn object_gaps_keep_three_byte_offsets_around_unicode_and_trailing_images() {
        for native in [
            "plain",
            "\u{fffc}\n中文é🙂alone.\u{fffc}",
            "a\u{fffc}\u{fffc}b",
            "\u{fffc}",
        ] {
            let visible = native.replace('\u{fffc}', "");
            let restored = restore_object_gaps(visible, native.len(), |a, b| {
                native.get(a..b).unwrap_or("").replace('\u{fffc}', "")
            })
            .unwrap();
            assert_eq!(restored, native);
        }
    }
}
