use epub::doc::EpubDoc;
use std::path::Path;

/// 从 EPUB 解析出的元数据（M2 只取书架需要的）。
pub struct EpubMeta {
    pub title: String,
    pub author: String,
    pub language: String,
    /// 封面图字节 + MIME（无封面为 None）。
    pub cover: Option<(Vec<u8>, String)>,
}

/// 一章的渲染数据。
pub struct ChapterInfo {
    pub title: String,
    pub html: String,
}

/// 加载全书章节（已消毒 + 提取标题，阅读器可直接渲染）。
pub fn load_chapters<P: AsRef<Path>>(path: P) -> Result<Vec<ChapterInfo>, String> {
    let mut doc = EpubDoc::new(path).map_err(|e| format!("打开 EPUB 失败: {}", e))?;
    let n = doc.get_num_chapters();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        doc.set_current_chapter(i);
        let html = doc.get_current_str().map(|(s, _mime)| s).unwrap_or_default();
        let body = sanitize_for_reader(&extract_body(&html));
        if body.trim().is_empty() {
            continue;
        }
        let title = first_heading_text(&body).unwrap_or_else(|| format!("第 {} 节", out.len() + 1));
        out.push(ChapterInfo { title, html: body });
    }
    if out.is_empty() {
        return Err("未解析出任何章节".to_string());
    }
    Ok(out)
}

/// 取消毒后 HTML 中第一个 h1-h3 的纯文本作为章节标题。
fn first_heading_text(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    for tag in ["<h1", "<h2", "<h3"] {
        if let Some(start) = lower.find(tag) {
            let content_start = html[start..].find('>').map(|p| start + p + 1)?;
            let close = format!("</{}", &tag[1..]);
            let end = lower[content_start..].find(&close).map(|p| content_start + p)?;
            let mut text = String::new();
            let mut in_tag = false;
            for c in html[content_start..end].chars() {
                match c {
                    '<' => in_tag = true,
                    '>' => in_tag = false,
                    _ if !in_tag => text.push(c),
                    _ => {}
                }
            }
            let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if !t.is_empty() {
                return Some(if t.chars().count() > 40 {
                    t.chars().take(40).collect::<String>() + "…"
                } else {
                    t
                });
            }
        }
    }
    None
}

/// Makepad Html widget 认识的标签（不在此列的标签会被它连内容一起丢弃，
/// 所以渲染前必须"去壳留瓤"）。
const KEEP_TAGS: &[&str] = &[
    "p", "h1", "h2", "h3", "h4", "h5", "h6", "b", "strong", "i", "em", "blockquote", "code",
    "pre", "sub", "sup", "ul", "ol", "li", "u", "del", "s", "strike", "table", "thead", "tbody",
    "tr", "th", "td", "br", "hr",
];

/// 这些标签连同内容一起丢（对阅读无意义或有害）。
const DROP_TAGS: &[&str] = &["script", "style", "head", "svg", "math", "rt", "rp", "template"];

/// 面向 Html widget 的消毒：白名单标签保留原样；未知标签（div/section/span/a/figure…）
/// 去掉标签本身、保留内部内容；DROP_TAGS 连内容删除；注释/声明剔除。
pub fn sanitize_for_reader(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let lower = html.to_lowercase();
    let mut i = 0;
    while i < html.len() {
        let Some(lt_rel) = html[i..].find('<') else {
            out.push_str(&html[i..]);
            break;
        };
        let lt = i + lt_rel;
        out.push_str(&html[i..lt]);

        let rest = &html[lt..];
        if rest.starts_with("<!--") {
            i = match html[lt..].find("-->") {
                Some(p) => lt + p + 3,
                None => html.len(),
            };
            continue;
        }
        let Some(gt_rel) = rest.find('>') else { break };
        let gt = lt + gt_rel;
        let inner = &html[lt + 1..gt];
        if inner.starts_with('!') || inner.starts_with('?') {
            i = gt + 1;
            continue;
        }
        let is_close = inner.starts_with('/');
        let name: String = inner
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_lowercase();

        if DROP_TAGS.contains(&name.as_str()) {
            if is_close {
                i = gt + 1;
            } else if inner.ends_with('/') {
                i = gt + 1;
            } else {
                let close = format!("</{}", name);
                i = match lower[gt..].find(&close) {
                    Some(p) => {
                        let cstart = gt + p;
                        match html[cstart..].find('>') {
                            Some(g) => cstart + g + 1,
                            None => html.len(),
                        }
                    }
                    None => gt + 1,
                };
            }
            continue;
        }

        if KEEP_TAGS.contains(&name.as_str()) {
            out.push_str(&html[lt..=gt]);
        }
        // 未知标签：不输出标签本身，内容自然流过（去壳留瓤）
        i = gt + 1;
    }
    out
}

/// 按块级结构安全分块（PortalList 虚拟化用）：只在标签嵌套深度回到 0 时切，
/// 不会把 blockquote/表格/列表从中间切断。
pub fn split_blocks(html: &str) -> Vec<String> {
    const VOID: &[&str] = &["br", "hr"];
    let mut blocks = Vec::new();
    let mut buf = String::new();
    let mut depth: i32 = 0;
    let mut i = 0;
    while i < html.len() {
        let Some(lt_rel) = html[i..].find('<') else {
            buf.push_str(&html[i..]);
            break;
        };
        let lt = i + lt_rel;
        buf.push_str(&html[i..lt]);
        let Some(gt_rel) = html[lt..].find('>') else {
            buf.push_str(&html[lt..]);
            break;
        };
        let gt = lt + gt_rel;
        let inner = &html[lt + 1..gt];
        let is_close = inner.starts_with('/');
        let self_close = inner.ends_with('/');
        let name: String = inner
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_lowercase();

        buf.push_str(&html[lt..=gt]);
        if !VOID.contains(&name.as_str()) && !self_close && !name.is_empty() {
            depth += if is_close { -1 } else { 1 };
        }
        if depth <= 0 && is_close {
            depth = 0;
            let block = buf.trim();
            if !block.is_empty() {
                blocks.push(block.to_string());
            }
            buf.clear();
        }
        i = gt + 1;
    }
    let tail = buf.trim();
    if !tail.is_empty() {
        blocks.push(tail.to_string());
    }
    if blocks.is_empty() {
        blocks.push(html.to_string());
    }
    blocks
}

/// 取 <body> 内部内容；解析失败原样返回。
fn extract_body(xhtml: &str) -> String {
    let lower = xhtml.to_lowercase();
    if let Some(bstart) = lower.find("<body") {
        if let Some(tag_end) = xhtml[bstart..].find('>') {
            let content_start = bstart + tag_end + 1;
            let end = lower.rfind("</body>").unwrap_or(xhtml.len());
            if end > content_start {
                return xhtml[content_start..end].to_string();
            }
        }
    }
    xhtml.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_unwraps_containers_keeps_content() {
        let src = r##"<section class="chapter"><div id="x"><h1>Title</h1><p>Hello <span class="k">world</span> <a href="#f1">link</a></p></div></section>"##;
        let out = sanitize_for_reader(src);
        assert_eq!(out, "<h1>Title</h1><p>Hello world link</p>");
    }

    #[test]
    fn sanitize_drops_script_style_rt_and_comments() {
        let src = r#"<p>a<!-- c --><script>bad()</script><style>.x{}</style></p><p><ruby>词<rt>ci</rt></ruby></p>"#;
        let out = sanitize_for_reader(src);
        assert_eq!(out, "<p>a</p><p>词</p>");
    }

    #[test]
    fn split_blocks_respects_nesting() {
        let src = "<h2>T</h2><blockquote><p>a</p><p>b</p></blockquote><p>c</p>";
        let blocks = split_blocks(src);
        assert_eq!(
            blocks,
            vec![
                "<h2>T</h2>".to_string(),
                "<blockquote><p>a</p><p>b</p></blockquote>".to_string(),
                "<p>c</p>".to_string()
            ]
        );
    }
}

/// 用纯 Rust 的 epub crate 解析元数据（替代老项目的 foliate-js）。
pub fn parse_meta<P: AsRef<Path>>(path: P) -> Result<EpubMeta, String> {
    let mut doc = EpubDoc::new(path).map_err(|e| format!("打开 EPUB 失败: {}", e))?;

    let title = doc
        .mdata("title")
        .map(|item| item.value.clone())
        .unwrap_or_else(|| "未知书名".to_string());
    let author = doc
        .mdata("creator")
        .or_else(|| doc.mdata("author"))
        .map(|item| item.value.clone())
        .unwrap_or_else(|| "未知作者".to_string());
    let language = doc
        .mdata("language")
        .map(|item| item.value.clone())
        .unwrap_or_else(|| "unknown".to_string());

    let cover = doc.get_cover();

    Ok(EpubMeta {
        title,
        author,
        language,
        cover,
    })
}
