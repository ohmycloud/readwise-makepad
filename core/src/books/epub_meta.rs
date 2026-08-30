use epub::doc::EpubDoc;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek};
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

/// 加载全书章节（已消毒 + 提取标题 + 图片落盘并改写 src，阅读器可直接渲染）。
pub fn load_chapters<P: AsRef<Path>>(path: P) -> Result<Vec<ChapterInfo>, String> {
    let path = path.as_ref();
    let mut doc = EpubDoc::new(path).map_err(|e| format!("打开 EPUB 失败: {}", e))?;
    let images = extract_images(&mut doc, path);
    let n = doc.get_num_chapters();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        doc.set_current_chapter(i);
        let html = doc
            .get_current_str()
            .map(|(s, _mime)| s)
            .unwrap_or_default();
        let body = sanitize_for_reader(&extract_body(&html));
        if body.trim().is_empty() {
            continue;
        }
        let body = rewrite_img_src(&body, &images);
        let title = first_heading_text(&body).unwrap_or_else(|| format!("第 {} 节", out.len() + 1));
        out.push(ChapterInfo { title, html: body });
    }
    if out.is_empty() {
        return Err("未解析出任何章节".to_string());
    }
    Ok(out)
}

/// 把包内图片资源提取到书目录的 res/（幂等），返回 文件名 → 绝对路径。
fn extract_images<R: Read + Seek>(
    doc: &mut EpubDoc<R>,
    epub_path: &Path,
) -> HashMap<String, String> {
    let res_dir = epub_path.parent().unwrap_or(Path::new(".")).join("res");
    let items: Vec<(String, String)> = doc
        .resources
        .iter()
        .filter(|(_, item)| item.mime.starts_with("image/"))
        .filter_map(|(id, item)| {
            item.path
                .file_name()
                .map(|n| (id.clone(), n.to_string_lossy().into_owned()))
        })
        .collect();
    let mut map = HashMap::new();
    if items.is_empty() {
        return map;
    }
    let _ = fs::create_dir_all(&res_dir);
    for (id, name) in items {
        let target = res_dir.join(&name);
        if !target.exists() {
            if let Some((data, _mime)) = doc.get_resource(&id) {
                let _ = fs::write(&target, data);
            }
        }
        map.insert(name, target.to_string_lossy().into_owned());
    }
    map
}

/// 最小 %XX 解码（图片文件名常见空格等转义）。
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 把 <img src="包内相对路径"> 改写为提取后的绝对路径（按文件名匹配）。
fn rewrite_img_src(html: &str, images: &HashMap<String, String>) -> String {
    if !html.to_ascii_lowercase().contains("<img") {
        return html.to_string();
    }
    let mut out = String::with_capacity(html.len());
    let mut code_depth = 0_i32;
    let mut i = 0;
    while let Some(pos) = html[i..].find('<') {
        let tag_start = i + pos;
        let Some(gt_rel) = html[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + gt_rel;
        out.push_str(&html[i..tag_start]);
        let tag = &html[tag_start..=tag_end];
        let inner = &tag[1..tag.len() - 1];
        let is_close = inner.trim_start().starts_with('/');
        let name = inner
            .trim_start()
            .trim_start_matches('/')
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();

        if name == "img" && !is_close {
            out.push_str(&rewrite_one_img(tag, images, code_depth > 0));
        } else {
            out.push_str(tag);
        }
        if name == "code" || name == "pre" {
            code_depth += if is_close { -1 } else { 1 };
            code_depth = code_depth.max(0);
        }
        i = tag_end + 1;
    }
    out.push_str(&html[i..]);
    out
}

fn quoted_attr_range(tag: &str, attr: &str) -> Option<(usize, usize)> {
    let lower = tag.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let needle = attr.as_bytes();
    let mut offset = 0;
    while let Some(found) = lower[offset..].find(attr) {
        let start = offset + found;
        let boundary_before =
            start == 0 || bytes[start - 1].is_ascii_whitespace() || bytes[start - 1] == b'<';
        let mut cursor = start + needle.len();
        if !boundary_before {
            offset = cursor;
            continue;
        }
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'=') {
            offset = cursor;
            continue;
        }
        cursor += 1;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        let quote = *bytes.get(cursor)?;
        if quote != b'"' && quote != b'\'' {
            offset = cursor + 1;
            continue;
        }
        let value_start = cursor + 1;
        let value_end = tag[value_start..].find(quote as char)? + value_start;
        return Some((value_start, value_end));
    }
    None
}

fn quoted_attr<'a>(tag: &'a str, attr: &str) -> Option<&'a str> {
    let (start, end) = quoted_attr_range(tag, attr)?;
    Some(&tag[start..end])
}

fn callout_label(tag: &str, src: &str) -> Option<String> {
    fn clean(value: &str) -> Option<String> {
        let value = value.trim();
        let unwrapped = value
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
            .unwrap_or(value);
        if !unwrapped.is_empty() && unwrapped.chars().all(|ch| ch.is_ascii_digit()) {
            return Some(unwrapped.to_string());
        }
        if unwrapped.chars().count() == 1
            && unwrapped
                .chars()
                .next()
                .is_some_and(|ch| ('\u{2460}'..='\u{2473}').contains(&ch))
        {
            return Some(unwrapped.to_string());
        }
        None
    }

    if let Some(label) = quoted_attr(tag, "alt").and_then(clean) {
        return Some(label);
    }
    if let Some(label) = quoted_attr(tag, "title").and_then(clean) {
        return Some(label);
    }

    let filename = percent_decode(src.rsplit('/').next().unwrap_or(src));
    let stem = filename
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(&filename)
        .to_ascii_lowercase();
    if let Some(label) = clean(&stem) {
        return Some(label);
    }
    for prefix in [
        "callout-", "callout_", "callout", "number-", "number_", "num-",
    ] {
        if let Some(label) = stem.strip_prefix(prefix).and_then(clean) {
            return Some(label);
        }
    }
    None
}

fn rewrite_one_img(tag: &str, images: &HashMap<String, String>, in_code: bool) -> String {
    let Some((val_start, val_end)) = quoted_attr_range(tag, "src") else {
        return tag.to_string();
    };
    let val = &tag[val_start..val_end];
    if in_code {
        if let Some(label) = callout_label(tag, val) {
            return format!("[{label}]");
        }
    }
    let name = percent_decode(val.rsplit('/').next().unwrap_or(val));
    if let Some(abs) = images.get(&name) {
        return format!("{}{}{}", &tag[..val_start], abs, &tag[val_end..]);
    }
    tag.to_string()
}

/// 取消毒后 HTML 中第一个 h1-h3 的纯文本作为章节标题。
fn first_heading_text(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    for tag in ["<h1", "<h2", "<h3"] {
        if let Some(start) = lower.find(tag) {
            let content_start = html[start..].find('>').map(|p| start + p + 1)?;
            let close = format!("</{}", &tag[1..]);
            let end = lower[content_start..]
                .find(&close)
                .map(|p| content_start + p)?;
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
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "b",
    "strong",
    "i",
    "em",
    "blockquote",
    "code",
    "pre",
    "sub",
    "sup",
    "ul",
    "ol",
    "li",
    "u",
    "del",
    "s",
    "strike",
    "table",
    "thead",
    "tbody",
    "tr",
    "th",
    "td",
    "br",
    "hr",
    "img",
];

/// 这些标签连同内容一起丢（对阅读无意义或有害）。
const DROP_TAGS: &[&str] = &[
    "script", "style", "head", "svg", "math", "rt", "rp", "template",
];

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
    const VOID: &[&str] = &["br", "hr", "img"];
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

/// 从一个顶层 `<pre>` 块中恢复供原生代码视图使用的纯文本。
/// EPUB 常用 span 包裹语法 token，并用实体表示源码中的尖括号；这些
/// 展示标记应被剥掉，但缩进、空格、显式换行和注释文本必须原样保留。
pub fn code_block_text(html: &str) -> Option<String> {
    let trimmed = html.trim();
    let lower = trimmed.to_ascii_lowercase();
    let pre_start = lower.find("<pre")?;
    if !trimmed[..pre_start].trim().is_empty() {
        return None;
    }
    let open_end = lower[pre_start..].find('>')? + pre_start;
    let close_start = lower.rfind("</pre>")?;
    if close_start <= open_end || !trimmed[close_start + 6..].trim().is_empty() {
        return None;
    }

    fn decode_entity(entity: &str) -> Option<char> {
        match entity {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            value if value.starts_with("#x") || value.starts_with("#X") => {
                u32::from_str_radix(&value[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            value if value.starts_with('#') => {
                value[1..].parse::<u32>().ok().and_then(char::from_u32)
            }
            _ => None,
        }
    }

    let fragment = &trimmed[open_end + 1..close_start];
    let mut text = String::with_capacity(fragment.len());
    let mut skip_mark_metadata = false;
    let mut i = 0;
    while i < fragment.len() {
        let rest = &fragment[i..];
        if rest.starts_with('<') {
            let Some(gt) = rest.find('>') else {
                text.push_str(rest);
                break;
            };
            let tag = &rest[1..gt];
            let name = tag
                .trim_start_matches('/')
                .split(|ch: char| ch.is_ascii_whitespace() || ch == '/')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !tag.starts_with('/') && name == "br" {
                text.push('\n');
            }
            if tag.starts_with('/') && name == "mark" {
                skip_mark_metadata = false;
            }
            i += gt + 1;
            continue;
        }
        if rest.starts_with('&') {
            if let Some(end) = rest.find(';').filter(|end| *end <= 12) {
                if let Some(decoded) = decode_entity(&rest[1..end]) {
                    if !skip_mark_metadata {
                        text.push(decoded);
                    }
                    i += end + 1;
                    continue;
                }
            }
        }
        let Some(ch) = rest.chars().next() else {
            break;
        };
        if ch == crate::vocab::annotate::MARK_SEP {
            skip_mark_metadata = true;
        } else if !skip_mark_metadata {
            text.push(ch);
        }
        i += ch.len_utf8();
    }

    let mut text = text.replace("\r\n", "\n").replace('\r', "\n");
    if text.starts_with('\n') {
        text.remove(0);
    }
    if text.ends_with('\n') {
        text.pop();
    }
    Some(text)
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

    #[test]
    fn code_block_text_preserves_source_layout_and_decodes_entities() {
        let html = concat!(
            "<pre class=\"language-rust\"><code>",
            "<span class=\"kw\">fn</span> main() {\r\n",
            "    <span class=\"comment\">// keep this comment</span><br>",
            "    println!(&quot;answer: {}&quot;, 6 &lt;&lt; 1);\n",
            "}</code></pre>"
        );
        assert_eq!(
            code_block_text(html).as_deref(),
            Some("fn main() {\n    // keep this comment\n    println!(\"answer: {}\", 6 << 1);\n}")
        );
        assert_eq!(code_block_text("<p>inline <code>x</code></p>"), None);
        assert_eq!(
            code_block_text("<pre><mark>let x = 1;\u{E001}blue</mark></pre>").as_deref(),
            Some("let x = 1;")
        );
    }

    #[test]
    fn code_callout_images_become_crisp_text_markers() {
        let images = HashMap::from([
            (
                "callout1.png".to_string(),
                "/book/res/callout1.png".to_string(),
            ),
            ("figure.png".to_string(), "/book/res/figure.png".to_string()),
        ]);
        let html = concat!(
            "<pre><code>value = run() ",
            "<img class=\"callout\" src=\"img/callout1.png\" alt=\"1\">",
            "</code></pre>",
            "<p><IMG SRC='img/figure.png' ALT='1'></p>"
        );

        let rewritten = rewrite_img_src(html, &images);

        assert!(rewritten.contains("value = run() [1]"));
        assert!(!rewritten.contains("/book/res/callout1.png"));
        assert!(rewritten.contains("SRC='/book/res/figure.png'"));
    }

    #[test]
    fn code_callout_filename_is_used_when_alt_is_missing() {
        let images = HashMap::from([(
            "number-23.png".to_string(),
            "/book/res/number-23.png".to_string(),
        )]);
        assert_eq!(
            rewrite_img_src("<pre><img src=\"icons/number-23.png\"></pre>", &images),
            "<pre>[23]</pre>"
        );
        assert_eq!(
            rewrite_img_src(
                "<code><img src=\"missing.png\" alt=\"7\"></code>",
                &HashMap::new()
            ),
            "<code>[7]</code>"
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
