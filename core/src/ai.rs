use serde::{Deserialize, Serialize};

pub const DEFAULT_OPENAI_ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AiSettings {
    pub endpoint: String,
    pub model: String,
    pub api_key: String,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            endpoint: DEFAULT_OPENAI_ENDPOINT.to_string(),
            model: String::new(),
            api_key: String::new(),
        }
    }
}

/// Accept either an SDK-style base URL or a complete Chat Completions URL.
///
/// Most OpenAI-compatible services expose `/v1/chat/completions`; DeepSeek's
/// official endpoint intentionally omits `/v1`.
pub fn normalize_chat_endpoint(input: &str) -> Result<String, String> {
    let endpoint = input.trim().trim_end_matches('/');
    if endpoint.is_empty() {
        return Err("AI 接口地址不能为空".to_string());
    }
    if !(endpoint.starts_with("https://") || endpoint.starts_with("http://")) {
        return Err("AI 接口地址必须以 http:// 或 https:// 开头".to_string());
    }

    let lower = endpoint.to_ascii_lowercase();
    if lower.ends_with("/chat/completions") {
        return Ok(endpoint.to_string());
    }
    if lower.ends_with("/v1") || lower.ends_with("/beta") {
        return Ok(format!("{endpoint}/chat/completions"));
    }

    let without_scheme = endpoint.split_once("://").map(|(_, rest)| rest).unwrap();
    let (host, path) = without_scheme
        .split_once('/')
        .map(|(host, path)| (host, format!("/{path}")))
        .unwrap_or((without_scheme, String::new()));
    if host.eq_ignore_ascii_case("api.deepseek.com") {
        return Ok(format!("{endpoint}/chat/completions"));
    }
    if path.is_empty() {
        return Ok(format!("{endpoint}/v1/chat/completions"));
    }
    Ok(format!("{endpoint}/chat/completions"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_common_openai_compatible_endpoints() {
        assert_eq!(
            normalize_chat_endpoint("https://api.deepseek.com").unwrap(),
            "https://api.deepseek.com/chat/completions"
        );
        assert_eq!(
            normalize_chat_endpoint("https://api.openai.com/v1/").unwrap(),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            normalize_chat_endpoint("http://localhost:8080").unwrap(),
            "http://localhost:8080/v1/chat/completions"
        );
        assert_eq!(
            normalize_chat_endpoint("https://example.com/openai/v1/chat/completions").unwrap(),
            "https://example.com/openai/v1/chat/completions"
        );
    }
}
