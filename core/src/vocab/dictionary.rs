//! Read-only offline dictionary used by reader word cards, not ruby layout.
use std::{collections::HashMap, sync::LazyLock};

static ENTRIES: LazyLock<HashMap<String, (String, String)>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("data/word-dictionary.json"))
        .expect("checked-in word dictionary must be valid")
});

pub fn warm_up() {
    LazyLock::force(&ENTRIES);
}

pub struct Entry<'a> {
    pub headword: &'a str,
    pub phonetic: &'a str,
    pub translation: &'a str,
}

pub fn lookup<'a>(word: &'a str, lemma: &'a str) -> Option<Entry<'a>> {
    // Lemma data has the complete senses; inflected forms often only say
    // "past participle of …". Do not present lemma IPA as inflected-word IPA.
    let (headword, (phonetic, translation)) = ENTRIES
        .get_key_value(lemma)
        .or_else(|| ENTRIES.get_key_value(&word.to_lowercase()))?;
    Some(Entry {
        headword,
        phonetic,
        translation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inflections_use_full_lemma_senses_and_identify_the_ipa_headword() {
        let entry = lookup("cited", "cite").unwrap();
        assert_eq!(entry.headword, "cite");
        assert!(!entry.phonetic.is_empty());
        assert!(entry.translation.contains("引用"));
    }

    #[test]
    fn missing_words_are_not_fabricated() {
        assert!(lookup("not-a-dictionary-word", "not-a-dictionary-word").is_none());
        assert!(lookup("solitude", "solitude")
            .unwrap()
            .translation
            .contains("孤独"));
    }
}
