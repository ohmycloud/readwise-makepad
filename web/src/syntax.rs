//! Server-side, offline syntax highlighting, applied ONLY after HTML cleanup.
//! Style attributes come from a built-in theme, never from the EPUB itself.
use std::sync::LazyLock;
use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, ThemeSet},
    parsing::{SyntaxReference, SyntaxSet},
    util::LinesWithEndings,
};

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

/// Only language metadata survives the untrusted class attribute.
pub fn language_class(value: &str) -> Option<String> {
    value.split_whitespace().find_map(|class| {
        let token = class
            .strip_prefix("language-")
            .or_else(|| class.strip_prefix("lang-"))?;
        (token.len() <= 40
            && token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '+' | '#')))
        .then(|| format!("language-{token}"))
    })
}

// Walk sanitized HTML tags without treating > or < inside quoted IDs as tags.
fn tag_end(html: &str) -> Option<usize> {
    let mut quote = None;
    for (index, byte) in html.bytes().enumerate().skip(1) {
        match (quote, byte) {
            (Some(current), value) if current == value => quote = None,
            (None, b'\'' | b'"') => quote = Some(byte),
            (None, b'>') => return Some(index),
            _ => {}
        }
    }
    None
}

fn tags(html: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut offset = 0;
    std::iter::from_fn(move || {
        let start = offset + html[offset..].find('<')?;
        let end = start + tag_end(&html[start..])? + 1;
        offset = end;
        Some((start, end))
    })
}

fn code_text(html: &str) -> Option<String> {
    let start = tag_end(html)? + 1;
    let end = html.rfind("</pre>")?;
    let inner = &html[start..end];
    let mut output = String::new();
    let mut offset = 0;
    for (start, end) in tags(inner) {
        output.push_str(&inner[offset..start]);
        if &inner[start..end] == "<br>" {
            output.push('\n');
        }
        offset = end;
    }
    output.push_str(&inner[offset..]);
    // Do not use native code_block_text: it deliberately trims edge newlines.
    Some(html_escape::decode_html_entities(&output).into_owned())
}

fn syntax<'a>(html: &str, text: &str, set: &'a SyntaxSet) -> &'a SyntaxReference {
    // Examine just opening pre/code tags, never a class mentioned inside source.
    for (start, end) in tags(html) {
        let opening = &html[start..end - 1];
        if (opening == "<pre"
            || opening.starts_with("<pre ")
            || opening == "<code"
            || opening.starts_with("<code "))
            && let Some(start) = opening.find("class=\"")
        {
            let classes = opening[start + 7..].split('"').next().unwrap_or("");
            if let Some(class) = language_class(classes)
                && let Some(syntax) = set.find_syntax_by_token(&class[9..])
            {
                return syntax;
            }
        }
    }
    if let Some(syntax) = text
        .lines()
        .next()
        .and_then(|line| set.find_syntax_by_first_line(line))
    {
        return syntax;
    }
    let token = if [
        "fn ",
        "let mut ",
        "pub ",
        "impl ",
        "println!",
        "use std::",
        "#[derive",
        "trait ",
        "struct ",
        "enum ",
        "match ",
        "unsafe ",
    ]
    .iter()
    .any(|needle| text.contains(needle))
    {
        "rs"
    } else if text
        .lines()
        .any(|line| line.trim_start().starts_with("def ") || line.trim_start().starts_with("from "))
        || text.contains("import ") && text.contains("print(")
    {
        "py"
    } else if text.contains("function ")
        || text.contains("const ")
        || text.contains("console.log")
        || text.contains("=>")
    {
        "js"
    } else if text.contains("let ") {
        "rs"
    } else if serde_json::from_str::<serde_json::Value>(text).is_ok() {
        "json"
    } else {
        "txt"
    };
    set.find_syntax_by_token(token)
        .unwrap_or_else(|| set.find_syntax_plain_text())
}

// Keep both palettes in the same spans: theme changes never replace code nodes.
fn token_html(style: syntect::highlighting::Style, text: &str) -> String {
    let color = style.foreground;
    let channels = [color.r, color.g, color.b];
    let sum =
        f32::from(*channels.iter().min().unwrap()) + f32::from(*channels.iter().max().unwrap());
    let blend = if sum >= 368.0 {
        0.0
    } else {
        (368.0 - sum) / (510.0 - sum)
    };
    let [r, g, b] = channels.map(|c| (f32::from(c) + (255.0 - f32::from(c)) * blend).round() as u8);
    let light = format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b);
    let dark = format!("#{r:02x}{g:02x}{b:02x}");
    let mut css = format!("color:{light};color:light-dark({light},{dark});");
    for (flag, declaration) in [
        (FontStyle::BOLD, "font-weight:bold;"),
        (FontStyle::ITALIC, "font-style:italic;"),
        (FontStyle::UNDERLINE, "text-decoration:underline;"),
    ] {
        if style.font_style.contains(flag) {
            css.push_str(declaration);
        }
    }
    format!(
        "<span style=\"{css}\">{}</span>",
        html_escape::encode_text(text)
    )
}

fn block(html: &str) -> Option<String> {
    let text = code_text(html)?;
    // Avoid unbounded parser work on minified/very large publisher listings.
    if text.len() > 128 * 1024 || text.lines().any(|line| line.len() > 8192) {
        return None;
    }
    let set = &*SYNTAXES;
    let syntax = syntax(html, &text, set);
    let mut highlighter = HighlightLines::new(syntax, &THEMES.themes["InspiredGitHub"]);
    let opening_end = tag_end(html)?;
    let opening = &html[..opening_end];
    let mut output = format!("{opening} data-syntax=\"true\"><code>");
    for line in LinesWithEndings::from(&text) {
        let regions = highlighter.highlight_line(line, set).ok()?;
        for (style, token) in regions {
            output.push_str(&token_html(style, token));
        }
    }
    output.push_str("</code></pre>");
    Some(output)
}

pub fn highlight(html: &str) -> String {
    let mut output = String::with_capacity(html.len());
    let mut offset = 0;
    let mut scanner = tags(html);
    while let Some((start, end)) = scanner.next() {
        let opening = &html[start..end];
        if opening != "<pre>" && !opening.starts_with("<pre ") {
            continue;
        }
        let Some((_, end)) = scanner.find(|(start, end)| &html[*start..*end] == "</pre>") else {
            break;
        };
        output.push_str(&html[offset..start]);
        let original = &html[start..end];
        output.push_str(&block(original).unwrap_or_else(|| original.to_owned()));
        offset = end;
    }
    output.push_str(&html[offset..]);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_and_python_are_coloured_without_altering_source_or_whitespace() {
        for source in [
            "<pre class=\"language-rust\"><code>fn main() {\n    // 森林 🌿\n    println!(&quot;&lt;script&gt;&amp;&quot;);\n}\n</code></pre>",
            "<pre><code>def greet(name):\n    print(\"Hello\", name)\n</code></pre>",
            "<pre><code>let answer = 42;\n</code></pre>",
        ] {
            let result = highlight(source);
            assert!(result.contains("data-syntax=\"true\""));
            assert!(result.contains("style=\"color:"));
            assert!(result.matches("<span").count() > 1);
            assert_eq!(code_text(&result), code_text(source));
            assert!(code_text(&result).unwrap().ends_with('\n'));
            assert!(!result.contains("<script>"));
        }
    }

    #[test]
    fn quoted_delimiters_entities_and_edge_whitespace_are_preserved() {
        let before = "<p id=\"content-<pre >\">untouched</p>";
        let code = "<pre id=\"content->x\" class=\"language-rust\"><code>\n\nfn main() { // &copy; &#x1F33F;\n}<br>\n\n</code></pre>";
        let result = highlight(&format!("{before}{code}"));
        assert!(result.starts_with(before));
        let result = &result[before.len()..];
        assert!(result.starts_with("<pre id=\"content->x\""));
        assert_eq!(
            code_text(result),
            Some("\n\nfn main() { // © 🌿\n}\n\n\n".into())
        );
    }

    #[test]
    fn dual_theme_tokens_never_rewrite_literal_source_or_need_dom_replacement() {
        let source = "<pre class=\"language-rust\"><code>let color = \"color:#abcdef\"; // 颜色 🌿\n</code></pre>";
        let result = highlight(source);
        assert!(result.contains("color:light-dark(#"));
        assert_eq!(code_text(&result), code_text(source));
        let style = syntect::highlighting::Style {
            foreground: syntect::highlighting::Color {
                r: 3,
                g: 47,
                b: 98,
                a: 255,
            },
            ..Default::default()
        };
        let token = token_html(style, "&lt;");
        assert!(token.contains("color:#032f62;color:light-dark(#032f62,#"));
        assert!(token.contains("&amp;lt;"));
        assert!(!token.contains("light-dark(#032f62,#032f62)"));
    }

    #[test]
    fn metadata_is_restricted_and_large_code_falls_back_to_safe_original() {
        assert_eq!(
            language_class("button language-rust fixed"),
            Some("language-rust".into())
        );
        assert_eq!(language_class("onclick=alert(1)"), None);
        let source = format!("<pre>{}</pre>", "x".repeat(9000));
        assert_eq!(highlight(&source), source);
        assert_eq!(
            highlight("<p>inline <code>fn main()</code></p>"),
            "<p>inline <code>fn main()</code></p>"
        );
    }
}
