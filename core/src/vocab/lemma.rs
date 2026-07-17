use std::collections::HashMap;
use std::sync::LazyLock;

/// 变形 → 原形表（自老项目 lemmas-en.json，构建期嵌入）。
static LEMMAS: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("./data/lemmas-en.json")).unwrap_or_default()
});

/// 词 → 词表等级（1-4）。
static LEVELS: LazyLock<HashMap<String, i64>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("./data/word-levels.json")).unwrap_or_default()
});

/// 词表外单词的虚拟等级（高于所有读者等级）。
pub const OFF_LIST_LEVEL: i64 = 5;

/// 统一弯引号并剥离缩写后缀（don't -> do, it's -> it）。逻辑照抄老项目 vocab-service。
pub fn normalize_token(raw: &str) -> String {
    let mut s = raw.to_lowercase().replace('\u{2019}', "'");
    if s.ends_with("n't") {
        s.truncate(s.len() - 3);
        return s;
    }
    for suffix in ["'s", "'ll", "'ve", "'re", "'d", "'m", "'t"] {
        if s.ends_with(suffix) {
            s.truncate(s.len() - suffix.len());
            return s;
        }
    }
    s
}

/// 变形还原为原形；不在表中则原样返回。
pub fn lemmatize(word: &str) -> String {
    let normalized = normalize_token(word);
    LEMMAS.get(&normalized).cloned().unwrap_or(normalized)
}

/// 词的等级：1-4；词表外为 OFF_LIST_LEVEL。表面形优先（data 不被还原成 datum）。
pub fn word_level(word: &str) -> i64 {
    let normalized = normalize_token(word);
    if let Some(l) = LEVELS.get(&normalized) {
        return *l;
    }
    let lemma = LEMMAS.get(&normalized).cloned().unwrap_or(normalized);
    LEVELS.get(&lemma).copied().unwrap_or(OFF_LIST_LEVEL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_contractions() {
        assert_eq!(normalize_token("Don't"), "do");
        assert_eq!(normalize_token("it\u{2019}s"), "it");
        assert_eq!(normalize_token("Reader"), "reader");
    }

    #[test]
    fn lemma_data_loaded() {
        assert!(LEMMAS.len() > 1000, "lemmas 数据应已嵌入");
        assert!(LEVELS.len() > 1000, "词级数据应已嵌入");
    }
}
