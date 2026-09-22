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
    /// Path of this spine document inside the EPUB archive, for relative links.
    pub source_path: String,
    /// Fragment IDs inside figures, mapped to the extracted original image.
    pub figure_targets: HashMap<String, String>,
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
        let source_path = doc
            .get_current_path()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let html = doc
            .get_current_str()
            .map(|(s, _mime)| s)
            .unwrap_or_default();
        let body = extract_body(&html);
        let figure_targets = figure_image_targets(&body, &images);
        let body = sanitize_for_reader(&body);
        if body.trim().is_empty() {
            continue;
        }
        let body = rewrite_img_src(&body, &images);
        let title = first_heading_text(&body).unwrap_or_else(|| format!("第 {} 节", out.len() + 1));
        out.push(ChapterInfo {
            title,
            html: body,
            source_path,
            figure_targets,
        });
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

/// Figure cross-references may point to an ID on the outer `<div>` or on a
/// caption anchor. Those wrappers are removed for Makepad Html, so capture the
/// relationship before sanitizing the chapter.
fn figure_image_targets(html: &str, images: &HashMap<String, String>) -> HashMap<String, String> {
    let lower = html.to_ascii_lowercase();
    let mut targets = HashMap::new();
    let mut cursor = 0;
    while let Some(relative) = lower[cursor..].find("<figure") {
        let start = cursor + relative;
        let after_name = start + "<figure".len();
        if !lower
            .as_bytes()
            .get(after_name)
            .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b'>')
        {
            cursor = after_name;
            continue;
        }
        let Some(open_end) = lower[start..].find('>').map(|offset| start + offset) else {
            break;
        };
        let Some(close_start) = lower[open_end + 1..]
            .find("</figure>")
            .map(|offset| open_end + 1 + offset)
        else {
            cursor = open_end + 1;
            continue;
        };
        let figure = &html[start..close_start];
        let mut ids = Vec::new();
        let mut image = None;
        let mut tag_cursor = 0;
        while let Some(relative) = figure[tag_cursor..].find('<') {
            let tag_start = tag_cursor + relative;
            let Some(tag_end) = figure[tag_start..]
                .find('>')
                .map(|offset| tag_start + offset)
            else {
                break;
            };
            let tag = &figure[tag_start..=tag_end];
            let inner = tag[1..tag.len() - 1].trim_start();
            if !inner.starts_with('/') {
                let name = inner
                    .chars()
                    .take_while(|ch| ch.is_ascii_alphanumeric())
                    .collect::<String>()
                    .to_ascii_lowercase();
                if let Some(id) = quoted_attr(tag, "id") {
                    ids.push(id.to_string());
                }
                if name == "a" {
                    if let Some(name) = quoted_attr(tag, "name") {
                        ids.push(name.to_string());
                    }
                }
                if name == "img" && image.is_none() {
                    image = quoted_attr(tag, "src")
                        .map(|src| percent_decode(src.rsplit('/').next().unwrap_or(src)))
                        .and_then(|file| images.get(&file).cloned());
                }
            }
            tag_cursor = tag_end + 1;
        }
        if let Some(image) = image {
            for id in ids {
                targets.insert(id, image.clone());
            }
        }
        cursor = close_start + "</figure>".len();
    }
    targets
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
    "a",
];

/// 这些标签连同内容一起丢（对阅读无意义或有害）。
const DROP_TAGS: &[&str] = &[
    "script", "style", "head", "svg", "math", "rt", "rp", "template",
];

/// EPUB 表格标题通常写在 `<table><caption>…</caption>` 内。Makepad 的 Html
/// 能绘制表格，却没有 caption 的块级布局；直接去掉 caption 标签会让标题文字
/// 和第一行表格共用同一个文本流。这里先把标题提升到 table 前的独立段落，既保留
/// 语义，也让后续 `split_blocks` 能把标题和表格分别交给 PortalList。
fn hoist_table_captions(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() + 32);
    let mut cursor = 0;

    while let Some(table_rel) = lower[cursor..].find("<table") {
        let table_start = cursor + table_rel;
        let name_end = table_start + "<table".len();
        if lower
            .as_bytes()
            .get(name_end)
            .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'>' && *byte != b'/')
        {
            out.push_str(&html[cursor..name_end]);
            cursor = name_end;
            continue;
        }
        let Some(open_end_rel) = lower[table_start..].find('>') else {
            break;
        };
        let open_end = table_start + open_end_rel;
        let Some(close_rel) = lower[open_end + 1..].find("</table>") else {
            break;
        };
        let table_end = open_end + 1 + close_rel + "</table>".len();
        let table_lower = &lower[open_end + 1..table_end];
        let Some(caption_rel) = table_lower.find("<caption") else {
            out.push_str(&html[cursor..table_end]);
            cursor = table_end;
            continue;
        };
        let caption_start = open_end + 1 + caption_rel;
        let caption_name_end = caption_start + "<caption".len();
        if lower
            .as_bytes()
            .get(caption_name_end)
            .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'>' && *byte != b'/')
        {
            out.push_str(&html[cursor..table_end]);
            cursor = table_end;
            continue;
        }
        let Some(caption_open_end_rel) = lower[caption_start..table_end].find('>') else {
            out.push_str(&html[cursor..table_end]);
            cursor = table_end;
            continue;
        };
        let caption_open_end = caption_start + caption_open_end_rel;
        let Some(caption_close_rel) = lower[caption_open_end + 1..table_end].find("</caption>")
        else {
            out.push_str(&html[cursor..table_end]);
            cursor = table_end;
            continue;
        };
        let caption_close = caption_open_end + 1 + caption_close_rel;
        let caption_end = caption_close + "</caption>".len();
        let caption = html[caption_open_end + 1..caption_close].trim();

        out.push_str(&html[cursor..table_start]);
        if !caption.is_empty() {
            out.push_str("<p><strong>");
            out.push_str(caption);
            out.push_str("</strong></p>");
        }
        out.push_str(&html[table_start..caption_start]);
        out.push_str(&html[caption_end..table_end]);
        cursor = table_end;
    }
    out.push_str(&html[cursor..]);
    out
}

/// 面向 Html widget 的消毒：白名单标签保留原样；未知标签（div/section/span/figure…）
/// 去掉标签本身、保留内部内容；DROP_TAGS 连内容删除；注释/声明剔除。
pub fn sanitize_for_reader(html: &str) -> String {
    let html = hoist_table_captions(html);
    let html = html.as_str();
    let mut out = String::with_capacity(html.len());
    let lower = html.to_lowercase();
    let mut i = 0;
    // An EPUB may use <a id="…"/> as a zero-width destination. It must not
    // become an unclosed HtmlLink, which would consume the following prose.
    let mut link_stack = Vec::new();
    let mut figure_depth = 0usize;
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

        if name == "figure" {
            if is_close {
                figure_depth = figure_depth.saturating_sub(1);
            } else if !inner.trim_end().ends_with('/') {
                figure_depth += 1;
            }
        }

        if link_stack.last() == Some(&true)
            && matches!(
                name.as_str(),
                "b" | "strong" | "i" | "em" | "code" | "sub" | "sup" | "u" | "del" | "s" | "strike"
            )
        {
            // HtmlLink consumes one text node and skips nested markup. Flatten
            // inline formatting so every word remains in the link's text.
        } else if name == "figcaption" {
            if is_close {
                out.push_str("<readeralignoff></readeralignoff></em></p>");
            } else {
                out.push_str("<p><em><readeralignon></readeralignon>");
            }
        } else if name == "h6" && figure_depth > 0 {
            if is_close {
                out.push_str("<readeralignoff></readeralignoff></h6>");
            } else {
                out.push_str(&html[lt..=gt]);
                out.push_str("<readeralignon></readeralignon>");
            }
        } else if name == "a" && is_close {
            if link_stack.pop().unwrap_or(false) {
                out.push_str("</a>");
            }
        } else if name == "a" {
            let tag = &html[lt..=gt];
            let is_link = quoted_attr(tag, "href").is_some_and(safe_reader_href);
            let self_closing = inner.trim_end().ends_with('/');
            let keep_link = is_link && !self_closing;
            let has_marker = quoted_attr(tag, "id").is_some() || quoted_attr(tag, "name").is_some();
            if keep_link || has_marker {
                out.push_str("<a");
                for attr in if keep_link {
                    &["href", "id", "name"][..]
                } else {
                    &["id", "name"][..]
                } {
                    if let Some((start, end)) = quoted_attr_range(tag, attr) {
                        let value = &tag[start..end];
                        out.push(' ');
                        out.push_str(attr);
                        out.push_str("=\"");
                        out.push_str(&value.replace('"', "&quot;"));
                        out.push('"');
                    }
                }
                out.push_str(if keep_link { ">" } else { "></a>" });
            }
            if !self_closing {
                link_stack.push(keep_link);
            }
        } else if KEEP_TAGS.contains(&name.as_str()) {
            out.push_str(&html[lt..=gt]);
        }
        // 未知标签：不输出标签本身，内容自然流过（去壳留瓤）
        i = gt + 1;
    }
    out
}

fn safe_reader_href(href: &str) -> bool {
    let href = href.trim();
    if href.is_empty() || href.chars().any(char::is_control) {
        return false;
    }
    let lower = href.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
    {
        return true;
    }
    !href.starts_with('/')
        && !href.starts_with('\\')
        && !href.split('/').next().unwrap_or("").contains(':')
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

        // A top-level table must start with a fresh layout turtle. EPUB figure/caption
        // wrappers are removed by sanitization, so defensive splitting here also fixes
        // malformed books that leave plain title text immediately before `<table>`.
        if !is_close && name == "table" && depth == 0 {
            let prefix = buf.trim();
            if !prefix.is_empty() {
                blocks.push(prefix.to_string());
            }
            buf.clear();
        }
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
        assert_eq!(
            out,
            "<h1>Title</h1><p>Hello world <a href=\"#f1\">link</a></p>"
        );
    }

    #[test]
    fn sanitize_keeps_relative_links_and_named_anchors_but_not_active_attributes() {
        let html = r##"<p><a id="part" name="old" onclick="evil()">Heading</a> <a href="../next.xhtml#part" onmouseover="evil()">Next</a> <a href="javascript:evil()">Unsafe</a></p>"##;
        assert_eq!(
            sanitize_for_reader(html),
            "<p><a id=\"part\" name=\"old\"></a>Heading <a href=\"../next.xhtml#part\">Next</a> Unsafe</p>"
        );
    }

    #[test]
    fn figure_targets_include_wrapper_and_caption_anchors() {
        let images = HashMap::from([
            ("first.png".to_string(), "/books/res/first.png".to_string()),
            (
                "second.png".to_string(),
                "/books/res/second.png".to_string(),
            ),
        ]);
        let html = concat!(
            r##"<p><a href="#fig0301">Figure 3-1</a></p>"##,
            r#"<figure><div class="figure" id="fig0301"><img src="assets/first.png"/>"#,
            r#"<h6>Figure 3-1</h6></div></figure>"#,
            r#"<figure><img src="images/second.png"/><figcaption>"#,
            r#"<a id="figure3-2">Figure 3-2</a></figcaption></figure>"#,
        );
        let targets = figure_image_targets(html, &images);
        assert_eq!(
            targets.get("fig0301").map(String::as_str),
            Some("/books/res/first.png")
        );
        assert_eq!(
            targets.get("figure3-2").map(String::as_str),
            Some("/books/res/second.png")
        );
        assert!(!targets.contains_key("missing"));
        assert!(!sanitize_for_reader(html).contains("id=\"fig0301\""));
    }

    #[test]
    fn self_closing_anchor_remains_zero_width_before_paragraph_text() {
        let html = r#"<p>First <a data-type="indexterm" id="ix_first"/>word <a id="ix_second" />and the rest of the paragraph.</p>"#;
        let sanitized = sanitize_for_reader(html);
        assert_eq!(
            sanitized,
            "<p>First <a id=\"ix_first\"></a>word <a id=\"ix_second\"></a>and the rest of the paragraph.</p>"
        );
        assert_eq!(split_blocks(&sanitized), vec![sanitized.clone()],);
        assert_eq!(
            crate::vocab::annotate::rendered_text(&sanitized),
            "First word and the rest of the paragraph."
        );
    }

    #[test]
    fn link_with_inline_formatting_keeps_its_complete_text() {
        let html =
            r#"<p>See <a href="next.xhtml"><em>first</em> and <code>second</code></a> after.</p>"#;
        assert_eq!(
            sanitize_for_reader(html),
            "<p>See <a href=\"next.xhtml\">first and second</a> after.</p>"
        );
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
    fn table_caption_is_hoisted_and_table_gets_its_own_block() {
        let src = concat!(
            "<figure><table><caption>Table 1. Results</caption>",
            "<thead><tr><th>Name</th><th>Score</th></tr></thead>",
            "<tbody><tr><td>Ada</td><td>10</td></tr></tbody></table></figure>"
        );
        let sanitized = sanitize_for_reader(src);
        assert_eq!(
            split_blocks(&sanitized),
            vec![
                "<p><strong>Table 1. Results</strong></p>".to_string(),
                concat!(
                    "<table><thead><tr><th>Name</th><th>Score</th></tr></thead>",
                    "<tbody><tr><td>Ada</td><td>10</td></tr></tbody></table>"
                )
                .to_string(),
            ]
        );
    }

    #[test]
    fn figure_caption_is_a_separate_reading_paragraph() {
        let sanitized = sanitize_for_reader(
            "<figure><figcaption>Table 2. Details</figcaption><table><tr><td>x</td></tr></table></figure>",
        );
        assert_eq!(
            split_blocks(&sanitized),
            vec![
                "<p><em><readeralignon></readeralignon>Table 2. Details<readeralignoff></readeralignoff></em></p>".to_string(),
                "<table><tr><td>x</td></tr></table>".to_string(),
            ]
        );
    }

    #[test]
    fn figure_heading_caption_is_centered_without_changing_visible_text() {
        let html = "<figure><div><img src=\"figure.png\"/><h6><span>Figure 3-25. </span>Self-consistency samples many thoughts and answers and will choose the most frequent answer</h6></div></figure><h6>Next heading</h6>";
        let sanitized = sanitize_for_reader(html);
        assert!(sanitized.contains("<h6><readeralignon></readeralignon>Figure 3-25. Self-consistency samples many thoughts and answers and will choose the most frequent answer<readeralignoff></readeralignoff></h6>"));
        assert!(sanitized.ends_with("<h6>Next heading</h6>"));
        assert_eq!(
            crate::vocab::annotate::rendered_text(&sanitized),
            crate::vocab::annotate::rendered_text(html)
        );
    }

    #[test]
    fn top_level_table_is_split_from_unwrapped_leading_text() {
        assert_eq!(
            split_blocks("Table 3<table><tr><td>x</td></tr></table>tail"),
            vec![
                "Table 3".to_string(),
                "<table><tr><td>x</td></tr></table>".to_string(),
                "tail".to_string(),
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
