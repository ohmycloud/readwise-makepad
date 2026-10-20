//! Presentation data for word cards. Never mutates reader text or pagination.
#[derive(Debug)]
pub struct Content {
    pub phonetic: String,
    pub part_of_speech: String,
    pub meanings: String,
}

pub fn content(word: &str, lemma: &str, gloss: &str) -> Content {
    let entry = readwise_core::vocab::dictionary::lookup(word, lemma);
    let mut part_of_speech = Vec::new();
    let translation = entry.as_ref().map_or(gloss, |entry| entry.translation);
    let mut meanings = translation
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (pos, meaning) = line.split_once(". ").unwrap_or(("", line));
            let has_pos = matches!(
                pos,
                "n" | "v"
                    | "vt"
                    | "vi"
                    | "adj"
                    | "adv"
                    | "prep"
                    | "pron"
                    | "conj"
                    | "num"
                    | "art"
                    | "aux"
                    | "abbr"
                    | "int"
                    | "a"
            );
            if has_pos {
                let pos = format!("{pos}.");
                if !part_of_speech.contains(&pos) {
                    part_of_speech.push(pos);
                }
                Some(meaning.to_string())
            } else {
                Some(line.to_string())
            }
        })
        .collect::<Vec<_>>();
    // Book/AI glosses can be context-specific. Enrich them, never silently
    // replace them with a more general dictionary interpretation.
    let gloss = gloss.trim();
    if !gloss.is_empty() && !translation.contains(gloss) {
        meanings.insert(0, gloss.to_string());
    }
    let meanings = meanings
        .into_iter()
        .enumerate()
        .map(|(index, meaning)| format!("{}.  {meaning}", index + 1))
        .collect::<Vec<_>>()
        .join("\n\n");
    let phonetic = entry
        .as_ref()
        .filter(|entry| !entry.phonetic.is_empty())
        .map_or_else(String::new, |entry| {
            if entry.headword.eq_ignore_ascii_case(word) {
                format!("/{}/", entry.phonetic)
            } else {
                format!("{}  /{}/", entry.headword, entry.phonetic)
            }
        });
    Content {
        phonetic,
        part_of_speech: part_of_speech.join(" / "),
        meanings,
    }
}

/// Extract the actual sentence at the rendered-text byte offset. Bound very
/// long/unpunctuated paragraphs around the hit instead of taking another match.
pub fn context_sentence(text: &str, offset: usize) -> String {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let boundary = |ch: char| matches!(ch, '.' | '!' | '?' | '。' | '！' | '？' | '\n');
    let start = text[..offset]
        .rfind(boundary)
        .map_or(0, |i| i + text[i..].chars().next().unwrap().len_utf8());
    let end = text[offset..].find(boundary).map_or(text.len(), |i| {
        let i = offset + i;
        i + text[i..].chars().next().unwrap().len_utf8()
    });
    let left = text[start..offset]
        .char_indices()
        .rev()
        .nth(140)
        .map_or(start, |(i, _)| start + i);
    let right = text[offset..end]
        .char_indices()
        .nth(220)
        .map_or(end, |(i, _)| offset + i);
    format!(
        "{}{}{}",
        if left > start { "…" } else { "" },
        text[left..right].trim(),
        if right < end { "…" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dictionary_and_fallback_do_not_invent_fields() {
        let cited = content("cited", "cite", "引用");
        assert!(cited.phonetic.starts_with("cite  /"));
        assert!(cited.part_of_speech.contains("vt."));
        assert!(cited.meanings.contains("引用"));
        let custom = content("unknown-xyz", "unknown-xyz", "自定义释义");
        assert!(custom.phonetic.is_empty());
        assert!(custom.part_of_speech.is_empty());
        assert_eq!(custom.meanings, "1.  自定义释义");
    }
    #[test]
    fn book_specific_gloss_is_preserved_before_dictionary_senses() {
        let entry = content("vigil", "vigil", "守望");
        assert!(entry.meanings.starts_with("1.  守望"));
        assert!(entry.meanings.contains("2."));
    }
    #[test]
    fn context_uses_the_clicked_occurrence_not_the_first_match() {
        let text = "The paper was cited. Later it was cited again! 下一句。";
        assert_eq!(
            context_sentence(text, text.rfind("cited").unwrap()),
            "Later it was cited again!"
        );
        assert_eq!(
            context_sentence(text, text.find("下").unwrap() + 1),
            "下一句。"
        );
    }
    #[test]
    fn long_unicode_context_stays_bounded_around_the_hit() {
        let text = "文".repeat(1000);
        let sentence = context_sentence(&text, 1500);
        assert!(sentence.starts_with('…') && sentence.ends_with('…'));
        assert!(sentence.chars().count() < 370);
    }
}
