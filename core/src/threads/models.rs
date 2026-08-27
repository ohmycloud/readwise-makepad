use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThreadRole {
    User,
    Assistant,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ThreadMessage {
    pub role: ThreadRole,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thread {
    pub id: String,
    pub book_id: Option<String>,
    pub metadata: String,
    pub title: String,
    pub messages: Vec<ThreadMessage>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_messages_round_trip_json() {
        let messages = vec![
            ThreadMessage {
                role: ThreadRole::User,
                content: "What does this mean?".into(),
            },
            ThreadMessage {
                role: ThreadRole::Assistant,
                content: "It is a metaphor.".into(),
            },
        ];
        let json = serde_json::to_string(&messages).unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<ThreadMessage>>(&json).unwrap(),
            messages
        );
    }
}
