//! Browser-safe projection of the core's canonical chapter blocks.
//! Never trust the Makepad HTML subset as a browser XSS sanitizer.
use readwise_core::{
    books::epub_meta::{LoadedChapters, TocEntry, split_blocks},
    vocab::{
        annotate::{RUBY_SEP, annotate_html},
        lemma::lemmatize,
    },
};
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

pub struct Chapter {
    pub title: String,
    pub blocks: Vec<String>,
    pub text: Vec<String>,
}

pub struct BookContent {
    pub chapters: Vec<Chapter>,
    pub toc: Vec<TocEntry>,
    pub media: Vec<PathBuf>,
    pub bytes: usize,
}

pub fn media_type(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

pub fn private_media(path: &Path, book_dir: &Path) -> Option<PathBuf> {
    let path = path.canonicalize().ok()?;
    let root = book_dir.canonicalize().ok()?;
    (path.starts_with(root) && path.is_file() && media_type(&path).is_some()).then_some(path)
}

fn archive_path(path: &Path) -> String {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::ParentDir => {
                parts.pop();
            }
            _ => {}
        }
    }
    parts.join("/")
}

pub fn prepare(loaded: LoadedChapters, book_id: &str, book_dir: &Path) -> BookContent {
    let sources: HashMap<String, usize> = loaded
        .chapters
        .iter()
        .enumerate()
        .map(|(i, c)| (archive_path(Path::new(&c.source_path)), i))
        .collect();
    let media = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
    let mut chapters = Vec::new();
    let mut bytes = 0;
    for chapter in loaded.chapters {
        let media = media.clone();
        let root = book_dir.to_owned();
        let id = book_id.to_owned();
        let sources = sources.clone();
        let source = PathBuf::from(&chapter.source_path);
        let mut cleaner = ammonia::Builder::default();
        cleaner
            .add_generic_attributes(&["id"])
            .add_tag_attributes("pre", &["class"])
            .add_tag_attributes("code", &["class"])
            .rm_tags(&["ruby", "rt", "rp"])
            .url_schemes(["http", "https"].into_iter().collect())
            .attribute_filter(move |tag, attr, value| {
                if matches!(tag, "pre" | "code") && attr == "class" {
                    return crate::syntax::language_class(value).map(Cow::Owned);
                }
                if attr == "id" || attr == "name" {
                    return Some(Cow::Owned(format!("content-{value}")));
                }
                if tag == "img" && attr == "src" {
                    let path = private_media(Path::new(value), &root)?;
                    let mut files = media.lock().unwrap();
                    let index = match files.iter().position(|p| p == &path) {
                        Some(index) => index,
                        None => {
                            files.push(path);
                            files.len() - 1
                        }
                    };
                    return Some(Cow::Owned(format!("/media/{id}/{index}")));
                }
                if tag == "a" && attr == "href" {
                    if let Some(fragment) = value.strip_prefix('#') {
                        return Some(Cow::Owned(format!("#content-{fragment}")));
                    }
                    if value.starts_with("https://") || value.starts_with("http://") {
                        return Some(Cow::Borrowed(value));
                    }
                    let (target, fragment) = value.split_once('#').unwrap_or((value, ""));
                    let target = source.parent().unwrap_or(Path::new("")).join(target);
                    let index = sources.get(&archive_path(&target))?;
                    let suffix = if fragment.is_empty() {
                        String::new()
                    } else {
                        format!("#content-{fragment}")
                    };
                    return Some(Cow::Owned(format!("/read/{id}/{index}{suffix}")));
                }
                Some(Cow::Borrowed(value))
            });
        // Split BEFORE browser cleanup so native block numbers remain stable.
        let blocks: Vec<String> = split_blocks(&chapter.html)
            .into_iter()
            .map(|block| cleaner.clean(&block).to_string())
            .collect();
        let text: Vec<String> = blocks
            .iter()
            .map(|block| plain_text(&annotate(block, &HashMap::new(), &HashSet::new())))
            .collect();
        bytes += blocks
            .iter()
            .chain(text.iter())
            .map(String::len)
            .sum::<usize>();
        chapters.push(Chapter {
            title: chapter.title,
            blocks,
            text,
        });
    }
    let media = Arc::try_unwrap(media).unwrap().into_inner().unwrap();
    BookContent {
        chapters,
        toc: loaded.toc,
        media,
        bytes,
    }
}

/// Canonical DOM text, without annotation nodes. Unlike native TextFlow offsets,
/// these coordinates count exactly the browser text nodes (including code LF).
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

pub fn annotate(block: &str, glosses: &HashMap<String, String>, known: &HashSet<String>) -> String {
    // The core emits Makepad's private ruby separator. Convert only that trusted
    // structure, escape glossary text before injection, and keep ALL ruby slots,
    // even for known words. Hiding rt must never change paragraph geometry.
    let annotated = annotate_html(block, glosses, &HashSet::new());
    let mut out = String::with_capacity(annotated.len() * 2);
    let mut rest = annotated.as_str();
    while let Some(start) = rest.find("<ruby>") {
        out.push_str(&rest[..start]);
        rest = &rest[start + "<ruby>".len()..];
        let Some(end) = rest.find("</ruby>") else {
            break;
        };
        let body = &rest[..end];
        if let Some((word, gloss)) = body.split_once(RUBY_SEP) {
            let lemma = lemmatize(word);
            let is_known = known.contains(&lemma);
            out.push_str(&format!(
                "<ruby tabindex=\"0\" role=\"button\" data-lemma=\"{}\" data-known=\"{is_known}\"><span class=\"word\">{word}</span><rt>{gloss}</rt></ruby>",
                escape(&lemma),
            ));
        } else {
            out.push_str(&escape(body));
        }
        rest = &rest[end + "</ruby>".len()..];
    }
    out.push_str(rest);
    let cleaned = ammonia::Builder::default()
        .add_tags(&["ruby", "rt", "rp"])
        .add_generic_attributes(&["id"])
        .add_tag_attributes("ruby", &["data-lemma", "data-known", "tabindex", "role"])
        .add_tag_attributes("span", &["class"])
        .add_tag_attributes("pre", &["class"])
        .add_tag_attributes("code", &["class"])
        .clean(&out)
        .to_string();
    crate::syntax::highlight(&cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use readwise_core::books::epub_meta::ChapterInfo;

    fn source(html: &str) -> LoadedChapters {
        LoadedChapters {
            chapters: vec![ChapterInfo {
                title: "Test".into(),
                html: html.into(),
                source_path: "text/ch1.xhtml".into(),
                figure_targets: HashMap::new(),
            }],
            toc: vec![],
        }
    }

    #[test]
    fn glossary_markup_is_text_and_known_words_keep_identical_ruby_slots() {
        let glosses =
            HashMap::from([("solitude".into(), escape("独处 <script>alert(1)</script>"))]);
        let unknown = annotate("<p>solitude</p>", &glosses, &HashSet::new());
        let known = annotate(
            "<p>solitude</p>",
            &glosses,
            &HashSet::from(["solitude".into()]),
        );
        assert!(!unknown.contains("<script>"));
        assert!(unknown.contains("&lt;script&gt;"));
        assert_eq!(
            unknown.replace("data-known=\"false\"", "data-known=\"true\""),
            known
        );
        assert!(known.contains("<rt>独处"));
    }

    #[test]
    fn browser_projection_removes_scripts_handlers_forms_and_untrusted_urls() {
        let root = tempfile::tempdir().unwrap();
        let html = r#"<p onclick="alert(1)" style="position:fixed">Safe<script>alert(2)</script><a href="javascript:alert(3)">link</a><img src="https://tracker.invalid/a.png" onerror="alert(4)"></p><iframe src="/api/known"></iframe><form action="/api/import"><input></form>"#;
        let book = prepare(source(html), "test", root.path());
        let html = book.chapters[0].blocks.join("");
        for bad in [
            "<script",
            "onclick",
            "onerror",
            "javascript:",
            "<iframe",
            "<form",
            "<input",
            "position:fixed",
            "tracker.invalid",
        ] {
            assert!(!html.contains(bad), "{bad}: {html}");
        }
    }

    #[test]
    fn code_keeps_language_but_never_publisher_styles_or_glosses() {
        let root = tempfile::tempdir().unwrap();
        let book = prepare(
            source(
                "<pre id=\"code\" class=\"fixed language-rust\" style=\"position:fixed\"><code>fn main() {\n    let solitude = &quot;&lt;script&gt;&quot;;\n}</code></pre>",
            ),
            "test",
            root.path(),
        );
        let raw = &book.chapters[0].blocks[0];
        assert!(raw.contains("class=\"language-rust\""));
        assert!(!raw.contains("position:fixed"));
        let result = annotate(
            raw,
            &HashMap::from([("solitude".into(), "独处".into())]),
            &HashSet::new(),
        );
        assert!(result.contains("style=\"color:"));
        assert!(result.contains("id=\"content-code\""));
        assert!(!result.contains("<ruby"));
        assert!(!result.contains("<script>"));
        assert_eq!(
            readwise_core::books::epub_meta::code_block_text(&result),
            readwise_core::books::epub_meta::code_block_text(raw)
        );
    }

    #[test]
    fn media_is_confined_to_the_owned_book_directory() {
        let root = tempfile::tempdir().unwrap();
        let book = root.path().join("book");
        std::fs::create_dir(&book).unwrap();
        std::fs::write(book.join("image.png"), b"png").unwrap();
        std::fs::write(root.path().join("outside.png"), b"secret").unwrap();
        assert!(private_media(&book.join("image.png"), &book).is_some());
        assert!(private_media(&root.path().join("outside.png"), &book).is_none());
        let html = format!("<p><img src=\"{}\"></p>", book.join("image.png").display());
        let prepared = prepare(source(&html), "book-id", &book);
        assert!(
            prepared.chapters[0]
                .blocks
                .join("")
                .contains("/media/book-id/0")
        );
        assert_eq!(prepared.media.len(), 1);
        assert!(
            !prepared.chapters[0]
                .blocks
                .join("")
                .contains(&root.path().display().to_string())
        );
    }
}
