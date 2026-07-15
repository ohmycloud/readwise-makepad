use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BookWord {
    pub lemma: String,
    #[serde(rename = "contextSentence")]
    pub context_sentence: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WordGloss {
    pub lemma: String,
    pub gloss: String,
    pub level: Option<i64>,
}

#[derive(Deserialize, Debug)]
pub struct GlossInput {
    pub lemma: String,
    pub gloss: String,
    pub level: Option<i64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct VocabBookEntry {
    pub lemma: String,
    pub word: String,
    pub gloss: Option<String>,
    pub level: Option<i64>,
    pub context_sentence: Option<String>,
    pub book_id: Option<String>,
    pub status: String,
    pub review_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct VocabBookAddInput {
    pub lemma: String,
    pub word: String,
    pub gloss: Option<String>,
    pub level: Option<i64>,
    pub context_sentence: Option<String>,
    pub book_id: Option<String>,
}
