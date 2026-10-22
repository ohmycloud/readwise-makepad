//! Reading difficulty is a local frequency approximation, not an official CEFR
//! word list. Custom membership is independent of difficulty and learning status.
use crate::settings::EnglishLevel;
use std::collections::HashMap;
use std::sync::LazyLock;

static VOCABULARY: LazyLock<HashMap<String, (u8, String)>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("data/reader-vocabulary.json"))
        .expect("checked-in reading vocabulary must be valid")
});

#[derive(Default)]
pub struct ReaderLexicon {
    pub standard: HashMap<String, String>,
    pub custom: HashMap<String, String>,
}

impl ReaderLexicon {
    pub fn builtin() -> Self {
        Self {
            standard: VOCABULARY
                .iter()
                .map(|(word, (_, gloss))| (word.clone(), gloss.clone()))
                .collect(),
            custom: HashMap::new(),
        }
    }

    pub fn select(&self, level: EnglishLevel) -> HashMap<String, String> {
        let Some(threshold) = level.annotation_threshold() else {
            return self.custom.clone();
        };
        self.standard
            .iter()
            .filter(|(word, gloss)| !gloss.trim().is_empty() && reading_band(word) > threshold)
            .map(|(word, gloss)| (word.clone(), gloss.clone()))
            .collect()
    }

    pub fn is_builtin(word: &str) -> bool {
        VOCABULARY.contains_key(word)
    }
}

pub fn reading_band(word: &str) -> u8 {
    VOCABULARY.get(word).map_or(7, |(band, _)| *band)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn higher_levels_have_strictly_fewer_glosses_and_custom_is_independent() {
        let mut lexicon = ReaderLexicon::builtin();
        lexicon.custom.insert("forest".into(), "自定义森林".into());
        lexicon
            .custom
            .insert("customonly".into(), "自定义词".into());
        let mut previous_words = lexicon.standard.clone();
        let mut previous = previous_words.len();
        for level in EnglishLevel::ALL.into_iter().take(6) {
            let selected = lexicon.select(level);
            assert!(!selected.is_empty());
            assert!(
                selected.len() < previous,
                "{level:?}: {} >= {previous}",
                selected.len()
            );
            assert!(!selected.contains_key("customonly"));
            assert!(selected
                .keys()
                .all(|word| previous_words.contains_key(word)));
            previous = selected.len();
            previous_words = selected;
        }
        assert_eq!(lexicon.select(EnglishLevel::C3), lexicon.custom);
        assert!(!lexicon.select(EnglishLevel::C3).contains_key("solitude"));
    }

    #[test]
    fn local_band_examples_and_unknown_word_fallback() {
        assert_eq!(reading_band("good"), 1);
        assert_eq!(reading_band("forest"), 2);
        assert_eq!(reading_band("murmur"), 3);
        assert_eq!(reading_band("solitude"), 4);
        assert_eq!(reading_band("vigil"), 5);
        assert_eq!(reading_band("abjure"), 6);
        assert_eq!(reading_band("uncataloguedword"), 7);
    }
}
