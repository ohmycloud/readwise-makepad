//! Word selection uses rendered UTF-8 offsets, independent of layout and ruby.
use jieba_rs::{Jieba, TokenizeMode};
use std::sync::LazyLock;
use unicode_segmentation::UnicodeSegmentation;

static CHINESE: LazyLock<Jieba> = LazyLock::new(Jieba::new);

pub fn warm_up() {
    // Parse the embedded dictionary before the first reading gesture.
    std::thread::spawn(|| {
        LazyLock::force(&CHINESE);
    });
}

fn is_han(ch: char) -> bool {
    matches!(ch as u32, 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff
        | 0x20000..=0x2ffff | 0x30000..=0x323af)
}

pub fn word_range_at(text: &str, byte_index: usize) -> Option<(usize, usize)> {
    // A word hit is a character, not a caret clamped to the preceding word.
    if byte_index >= text.len() {
        return None;
    }
    let graphemes = text.grapheme_indices(true).collect::<Vec<_>>();
    let index = graphemes.partition_point(|(offset, _)| *offset <= byte_index) - 1;
    let (offset, selected) = graphemes[index];
    let first = selected.chars().next()?;
    if first.is_whitespace() {
        return None;
    }
    if is_han(first) {
        let mut start = index;
        let mut end = index + 1;
        while start > 0 && graphemes[start - 1].1.chars().next().is_some_and(is_han) {
            start -= 1;
        }
        while end < graphemes.len() && graphemes[end].1.chars().next().is_some_and(is_han) {
            end += 1;
        }
        let run_start = graphemes[start].0;
        let run_end = graphemes.get(end).map_or(text.len(), |g| g.0);
        return CHINESE
            .tokenize(&text[run_start..run_end], TokenizeMode::Default, true)
            .into_iter()
            .find(|token| (token.byte_start..token.byte_end).contains(&(offset - run_start)))
            .map(|token| (run_start + token.byte_start, run_start + token.byte_end));
    }
    let letter = |i: usize| {
        graphemes[i]
            .1
            .chars()
            .next()
            .is_some_and(|ch| ch.is_alphanumeric() && !is_han(ch))
    };
    let word = |i: usize| {
        letter(i)
            || (matches!(graphemes[i].1, "'" | "’" | "-" | "‐" | "‑")
                && i > 0
                && i + 1 < graphemes.len()
                && letter(i - 1)
                && letter(i + 1))
    };
    if !word(index) {
        return Some((offset, offset + selected.len()));
    }
    let mut start = index;
    let mut end = index + 1;
    while start > 0 && word(start - 1) {
        start -= 1;
    }
    while end < graphemes.len() && word(end) {
        end += 1;
    }
    Some((
        graphemes[start].0,
        graphemes.get(end).map_or(text.len(), |g| g.0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selected(text: &str, index: usize) -> Option<&str> {
        word_range_at(text, index).map(|(a, b)| &text[a..b])
    }

    #[test]
    fn english_keeps_internal_connectors_and_unicode_graphemes() {
        for word in ["can't", "can’t", "well-known", "café", "cafe\u{301}"] {
            let text = format!("‘{word}’, next");
            for index in 3..3 + word.len() {
                assert_eq!(selected(&text, index), Some(word));
            }
        }
        assert_eq!(selected("read wise", 4), None);
        assert_eq!(selected("'read'", 2), Some("read"));
        assert_eq!(selected("read--wise", 8), Some("wise"));
    }

    #[test]
    fn chinese_selects_words_and_preserves_utf8_byte_offsets() {
        let text = "我喜欢阅读中文书籍。";
        for word in ["喜欢", "阅读", "中文", "书籍"] {
            let start = text.find(word).unwrap();
            for index in start..start + word.len() {
                assert_eq!(selected(text, index), Some(word));
            }
        }
        assert_eq!(selected("阅读", 3), Some("阅读"));
        assert_eq!(selected("𠮷。", 1), Some("𠮷"));
    }

    #[test]
    fn mixed_languages_do_not_merge_and_ruby_glosses_are_excluded() {
        let html =
            "<p>学习<em>Makepad</em>和<ruby word='reading' gloss='阅读'>reading</ruby>技巧。</p>";
        let text = readwise_core::vocab::annotate::rendered_text(html);
        assert_eq!(
            selected(&text, text.find("学习").unwrap() + 3),
            Some("学习")
        );
        assert_eq!(
            selected(&text, text.find("Makepad").unwrap()),
            Some("Makepad")
        );
        assert_eq!(
            selected(&text, text.find("reading").unwrap()),
            Some("reading")
        );
        assert!(!text.contains("阅读"));
        assert_eq!(selected("中文English阅读", 6), Some("English"));
    }

    #[test]
    fn whitespace_end_and_punctuation_never_select_a_neighbor() {
        assert_eq!(selected("", 0), None);
        assert_eq!(selected("read", 4), None);
        assert_eq!(selected("read", usize::MAX), None);
        assert_eq!(selected("阅读。", 6), Some("。"));
        assert_eq!(selected("é—word", 2), Some("—"));
        assert_eq!(selected("👩‍💻", 5), Some("👩‍💻"));
    }
}
