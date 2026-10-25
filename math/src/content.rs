//! Literal source helpers; neither frontend's annotation UI belongs in the engine.
use std::collections::HashSet;
pub fn plain_text(html: &str) -> String {
    let text = ammonia::Builder::default()
        .tags(HashSet::new())
        .link_rel(None)
        .clean(html)
        .to_string();
    html_escape::decode_html_entities(&text).into_owned()
}
pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
