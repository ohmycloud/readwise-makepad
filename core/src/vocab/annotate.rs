use super::lemma::lemmatize;
use std::collections::{HashMap, HashSet};

/// 词与释义在 <ruby> 内文中的分隔符（私有区字符，正文不可能出现）。
/// 释义随文本走而非存 widget 属性：Html 复用 widget 时 set_text 每帧刷新，
/// 属性只在创建时读一次——删词后释义会错位到下一个词上（实测 bug）。
pub const RUBY_SEP: char = '\u{E000}';

/// 给章节 HTML 注入生词注音：文本节点里的英文词，若其原形在释义表中
/// 且不在已认识集合中，包成 <ruby>词⌷释义</ruby>（⌷ 为 RUBY_SEP 私有区分隔符）。
/// 跳过标签内部、code/pre 和链接内容、HTML 实体（老项目 GlossAnnotator 的字符串版）。
pub fn annotate_html(
    html: &str,
    glosses: &HashMap<String, String>,
    known: &HashSet<String>,
) -> String {
    if glosses.is_empty() {
        return html.to_string();
    }
    let mut out = String::with_capacity(html.len() + html.len() / 4);
    let mut code_depth: i32 = 0;
    let mut i = 0;
    while i < html.len() {
        let rest = &html[i..];
        if rest.starts_with('<') {
            let Some(gt_rel) = rest.find('>') else {
                out.push_str(rest);
                break;
            };
            let tag = &rest[..=gt_rel];
            let inner = &tag[1..tag.len() - 1];
            let is_close = inner.starts_with('/');
            let name: String = inner
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_lowercase();
            // Code/table cannot host ruby; Makepad's HtmlLink also consumes its
            // text as one widget and drops nested ruby plus following link text.
            if matches!(name.as_str(), "code" | "pre" | "table" | "a") {
                code_depth += if is_close { -1 } else { 1 };
                code_depth = code_depth.max(0);
            }
            out.push_str(tag);
            i += gt_rel + 1;
            continue;
        }
        if rest.starts_with('&') {
            // HTML 实体整体拷贝，防止 &amp; 里的 "amp" 被当单词
            let end = rest
                .find(';')
                .filter(|p| *p <= 10)
                .map(|p| p + 1)
                .unwrap_or(1);
            out.push_str(&rest[..end]);
            i += end;
            continue;
        }
        let text_end = rest.find(['<', '&']).unwrap_or(rest.len());
        let text = &rest[..text_end];
        if code_depth > 0 {
            out.push_str(text);
        } else {
            annotate_text(text, glosses, known, &mut out);
        }
        i += text_end;
    }
    out
}

/// 把列表拍平成带文本标记的段落。TextFlow 的列表项换行是硬编码零间距
/// （begin_list_item 走 new_line_collapsed，无旋钮），注音在 li 里永远没有头顶空间；
/// 段落路径的间距（paragraph_margin）可配置。有序列表必须保留文本编号，
/// 不能与无序列表一样全部降级成项目符号。
pub fn flatten_lists(html: &str) -> String {
    enum ListKind {
        Unordered,
        Ordered(i32),
    }

    fn ordered_start(tag: &str) -> i32 {
        let lower = tag.to_ascii_lowercase();
        let Some(start) = lower.find("start") else {
            return 1;
        };
        let mut value = &tag[start + "start".len()..];
        value = value.trim_start();
        let Some(value) = value.strip_prefix('=') else {
            return 1;
        };
        let value = value.trim_start();
        let value = if let Some(rest) = value.strip_prefix(['"', '\'']) {
            &rest[..rest.find(['"', '\'']).unwrap_or(rest.len())]
        } else {
            &value[..value
                .find(|ch: char| ch.is_ascii_whitespace() || ch == '>')
                .unwrap_or(value.len())]
        };
        value.parse().unwrap_or(1)
    }

    let mut out = String::with_capacity(html.len());
    let mut li_depth: i32 = 0;
    let mut list_stack = Vec::new();
    let mut i = 0;
    while i < html.len() {
        let rest = &html[i..];
        let Some(lt_rel) = rest.find('<') else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..lt_rel]);
        let tag_start = i + lt_rel;
        let Some(gt_rel) = html[tag_start..].find('>') else {
            out.push_str(&html[tag_start..]);
            break;
        };
        let tag_end = tag_start + gt_rel;
        let inner = &html[tag_start + 1..tag_end];
        let is_close = inner.starts_with('/');
        let name: String = inner
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_lowercase();
        match (name.as_str(), is_close) {
            ("ul", false) => list_stack.push(ListKind::Unordered),
            ("ol", false) => {
                list_stack.push(ListKind::Ordered(ordered_start(&html[tag_start..=tag_end])))
            }
            ("ul", true) | ("ol", true) => {
                list_stack.pop();
            }
            ("li", false) => {
                li_depth += 1;
                out.push_str("<p>");
                match list_stack.last_mut() {
                    Some(ListKind::Ordered(next)) => {
                        out.push_str(&format!("{}. ", *next));
                        *next += 1;
                    }
                    _ => out.push_str("• "),
                }
            }
            ("li", true) => {
                li_depth = (li_depth - 1).max(0);
                out.push_str("</p>");
            }
            // li 内嵌的 <p>（Manning 等出版社常见）会造成 p 嵌套破坏段落间距，剥掉
            ("p", _) if li_depth > 0 => {}
            _ => out.push_str(&html[tag_start..=tag_end]),
        }
        i = tag_end + 1;
    }
    out
}

/// 原地消注：把词形还原后等于 lemma 的 <ruby> 标注剥回纯词。
/// "标为已认识"用——不重载书、不动滚动位置。
pub fn remove_ruby_for_lemma(html: &str, lemma: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    while let Some(pos) = html[i..].find("<ruby") {
        let tag_start = i + pos;
        let Some(open_end_rel) = html[tag_start..].find('>') else {
            break;
        };
        let content_start = tag_start + open_end_rel + 1;
        let Some(close_rel) = html[content_start..].find("</ruby>") else {
            break;
        };
        let content_end = content_start + close_rel;
        let content = &html[content_start..content_end];
        let word = content.split(RUBY_SEP).next().unwrap_or(content);

        out.push_str(&html[i..tag_start]);
        if super::lemma::lemmatize(word) == lemma {
            out.push_str(word);
        } else {
            out.push_str(&html[tag_start..content_end + "</ruby>".len()]);
        }
        i = content_end + "</ruby>".len();
    }
    out.push_str(&html[i..]);
    out
}

/// 剥掉全部注音标注（总开关关闭时原地还原纯文）。
pub fn remove_all_ruby(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    while let Some(pos) = html[i..].find("<ruby") {
        let tag_start = i + pos;
        let Some(open_end_rel) = html[tag_start..].find('>') else {
            break;
        };
        let content_start = tag_start + open_end_rel + 1;
        let Some(close_rel) = html[content_start..].find("</ruby>") else {
            break;
        };
        let content_end = content_start + close_rel;
        let content = &html[content_start..content_end];
        let word = content.split(RUBY_SEP).next().unwrap_or(content);
        out.push_str(&html[i..tag_start]);
        out.push_str(word);
        i = content_end + "</ruby>".len();
    }
    out.push_str(&html[i..]);
    out
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '\'' || c == '\u{2019}' || c == '-'
}

/// 纯文本段分词与注音（词形：字母开头，可含 '’-）。
fn annotate_text(
    text: &str,
    glosses: &HashMap<String, String>,
    known: &HashSet<String>,
    out: &mut String,
) {
    let mut chars = text.char_indices().peekable();
    let mut last = 0;
    while let Some((start, c)) = chars.next() {
        if !c.is_ascii_alphabetic() {
            continue;
        }
        let mut end = start + c.len_utf8();
        while let Some(&(pos, nc)) = chars.peek() {
            if is_word_char(nc) {
                end = pos + nc.len_utf8();
                chars.next();
            } else {
                break;
            }
        }
        let word = text[start..end].trim_end_matches(['\'', '\u{2019}', '-']);
        if word.len() < 2 {
            continue;
        }
        let lemma = lemmatize(word);
        if known.contains(&lemma) {
            continue;
        }
        if let Some(gloss) = glosses.get(&lemma) {
            out.push_str(&text[last..start]);
            out.push_str(&format!("<ruby>{}{}{}</ruby>", word, RUBY_SEP, gloss));
            last = start + word.len();
        }
    }
    out.push_str(&text[last..]);
}

/// 把 TextFlow UTF-8 字节区间 [start, end) 包成 <mark> 段（高亮渲染用）。
/// 计数规则与 TextFlow 选区一致：标签不计、实体按解码后文本字节数计、
/// <ruby> 内只计词部（SEP 前）。
/// 跨标签的区间按文本节点拆成多个 <mark> 段。
pub fn inject_marks(html: &str, ranges: &[(usize, usize)]) -> String {
    if ranges.is_empty() {
        return html.to_string();
    }
    let covered = |pos: usize| ranges.iter().any(|(s, e)| pos >= *s && pos < *e);
    let mut out = String::with_capacity(html.len() + 64);
    let mut text_pos: usize = 0;
    let mut in_mark = false;
    let mut i = 0;

    let close_mark = |out: &mut String, in_mark: &mut bool| {
        if *in_mark {
            out.push_str("</mark>");
            *in_mark = false;
        }
    };

    while i < html.len() {
        let rest = &html[i..];
        if rest.starts_with('<') {
            let Some(gt_rel) = rest.find('>') else {
                out.push_str(rest);
                break;
            };
            close_mark(&mut out, &mut in_mark);
            let tag = &rest[..=gt_rel];
            out.push_str(tag);
            // ruby 内文特殊：词部参与计数与高亮，SEP 后释义原样跳过
            let inner = &tag[1..tag.len() - 1];
            if inner
                .trim_start_matches('/')
                .to_lowercase()
                .starts_with("ruby")
                && !inner.starts_with('/')
            {
                i += gt_rel + 1;
                let rrest = &html[i..];
                if let Some(close_rel) = rrest.find("</ruby>") {
                    let content = &rrest[..close_rel];
                    // ruby 词部按整词判定（不拆分高亮段）
                    let word = content.split(RUBY_SEP).next().unwrap_or(content);
                    let word_len = word.len();
                    if word_len > 0 && covered(text_pos) {
                        // 整词高亮：mark 包不进 ruby 内部（widget 吃整段文本），跳过内部标记
                    }
                    text_pos += word_len;
                    out.push_str(content);
                    i += close_rel;
                }
                continue;
            }
            i += gt_rel + 1;
            continue;
        }
        if rest.starts_with('&') {
            let end = rest
                .find(';')
                .filter(|p| *p <= 10)
                .map(|p| p + 1)
                .unwrap_or(1);
            if covered(text_pos) && !in_mark {
                out.push_str("<mark>");
                in_mark = true;
            } else if !covered(text_pos) {
                close_mark(&mut out, &mut in_mark);
            }
            out.push_str(&rest[..end]);
            text_pos += decode_html_entity(&rest[..end]).len();
            i += end;
            continue;
        }
        let text_end = rest.find(['<', '&']).unwrap_or(rest.len());
        for (off, c) in rest[..text_end].char_indices() {
            let _ = off;
            if covered(text_pos) {
                if !in_mark {
                    out.push_str("<mark>");
                    in_mark = true;
                }
            } else {
                close_mark(&mut out, &mut in_mark);
            }
            out.push(c);
            text_pos += c.len_utf8();
        }
        i += text_end;
    }
    close_mark(&mut out, &mut in_mark);
    out
}

/// 永久高亮版本：把颜色名编码进 `<mark>` 内文，避免 Html 复用自定义 widget 时
/// 颜色属性成为创建时快照。MarkSpan 每帧通过 MARK_SEP 同时收到正文与颜色。
pub const MARK_SEP: char = '\u{E001}';

fn decode_html_entity(entity: &str) -> String {
    match entity {
        "&amp;" => "&".to_string(),
        "&lt;" => "<".to_string(),
        "&gt;" => ">".to_string(),
        "&quot;" => "\"".to_string(),
        "&apos;" | "&#39;" => "'".to_string(),
        "&nbsp;" => "\u{a0}".to_string(),
        _ if entity.starts_with("&#x") && entity.ends_with(';') => {
            u32::from_str_radix(&entity[3..entity.len() - 1], 16)
                .ok()
                .and_then(char::from_u32)
                .map(|c| c.to_string())
                .unwrap_or_else(|| "�".to_string())
        }
        _ if entity.starts_with("&#") && entity.ends_with(';') => entity[2..entity.len() - 1]
            .parse::<u32>()
            .ok()
            .and_then(char::from_u32)
            .map(|c| c.to_string())
            .unwrap_or_else(|| "�".to_string()),
        _ => "�".to_string(),
    }
}

/// 返回 Html/TextFlow 实际参与选择的纯文本。
///
/// 普通标签不计入；实体解码后计入；自定义 ruby/mark widget 的私有编码只保留
/// 用户看到的词或正文。它与高亮注入使用同一套 UTF-8 字节空间，供跨 PortalList 项选择
/// 在列表回收了离屏 widget 后仍能稳定取得完整文本。
pub fn rendered_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut i = 0usize;
    while i < html.len() {
        let rest = &html[i..];
        if rest.starts_with('<') {
            let Some(gt_rel) = rest.find('>') else { break };
            let tag = &rest[1..gt_rel];
            let name: String = tag
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_lowercase();
            i += gt_rel + 1;
            if !tag.starts_with('/') && name == "br" {
                out.push('\n');
            }
            if !tag.starts_with('/') && (name == "ruby" || name == "mark") {
                let close = format!("</{}>", name);
                let Some(close_rel) = html[i..].find(&close) else {
                    continue;
                };
                let content = &html[i..i + close_rel];
                let visible = if name == "ruby" {
                    content.split(RUBY_SEP).next().unwrap_or(content)
                } else {
                    content.split(MARK_SEP).next().unwrap_or(content)
                };
                out.push_str(visible);
                i += close_rel + close.len();
            }
            continue;
        }
        if rest.starts_with('&') {
            let end = rest
                .find(';')
                .filter(|p| *p <= 10)
                .map(|p| p + 1)
                .unwrap_or(1);
            out.push_str(&decode_html_entity(&rest[..end]));
            i += end;
            continue;
        }
        let text_end = rest.find(['<', '&']).unwrap_or(rest.len());
        out.push_str(&rest[..text_end]);
        i += text_end;
    }
    out
}

/// 从若干 Html 块中按 TextFlow UTF-8 字节下标导出跨块选区。
pub fn rendered_text_for_range(
    blocks: &[String],
    range: ((usize, usize), (usize, usize)),
) -> String {
    let ((start_block, start_char), (end_block, end_char)) = range;
    if start_block > end_block || start_block >= blocks.len() {
        return String::new();
    }
    let mut out = String::new();
    for (block_id, block) in blocks
        .iter()
        .enumerate()
        .take(end_block.min(blocks.len() - 1) + 1)
        .skip(start_block)
    {
        let text = rendered_text(block);
        let len = text.len();
        let from = if block_id == start_block {
            start_char.min(len)
        } else {
            0
        };
        let to = if block_id == end_block {
            end_char.min(len)
        } else {
            len
        };
        if to <= from {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        let mut from = from;
        while from > 0 && !text.is_char_boundary(from) {
            from -= 1;
        }
        let mut to = to;
        while to > from && !text.is_char_boundary(to) {
            to -= 1;
        }
        out.push_str(&text[from..to]);
    }
    out
}

pub fn inject_colored_marks(html: &str, ranges: &[(usize, usize, &str)]) -> String {
    if ranges.is_empty() {
        return html.to_string();
    }
    let color_at = |pos: usize| {
        ranges
            .iter()
            .find(|(start, end, _)| pos >= *start && pos < *end)
            .map(|(_, _, color)| *color)
    };
    let mut out = String::with_capacity(html.len() + 96);
    let mut text_pos = 0usize;
    let mut active_color: Option<&str> = None;
    let mut i = 0usize;

    fn transition<'a>(out: &mut String, active: &mut Option<&'a str>, next: Option<&'a str>) {
        if *active == next {
            return;
        }
        if let Some(color) = active.take() {
            out.push(MARK_SEP);
            out.push_str(color);
            out.push_str("</mark>");
        }
        if next.is_some() {
            out.push_str("<mark>");
        }
        *active = next;
    }

    while i < html.len() {
        let rest = &html[i..];
        if rest.starts_with('<') {
            let Some(gt_rel) = rest.find('>') else {
                transition(&mut out, &mut active_color, None);
                out.push_str(rest);
                break;
            };
            transition(&mut out, &mut active_color, None);
            let tag = &rest[..=gt_rel];
            out.push_str(tag);
            let inner = &tag[1..tag.len() - 1];
            if inner
                .trim_start_matches('/')
                .to_lowercase()
                .starts_with("ruby")
                && !inner.starts_with('/')
            {
                i += gt_rel + 1;
                let rrest = &html[i..];
                if let Some(close_rel) = rrest.find("</ruby>") {
                    let content = &rrest[..close_rel];
                    let word = content.split(RUBY_SEP).next().unwrap_or(content);
                    let word_len = word.len();
                    let color = (0..word_len).find_map(|offset| color_at(text_pos + offset));
                    out.push_str(content);
                    if let Some(color) = color {
                        out.push(MARK_SEP);
                        out.push_str(color);
                    }
                    text_pos += word_len;
                    i += close_rel;
                }
                continue;
            }
            i += gt_rel + 1;
            continue;
        }
        if rest.starts_with('&') {
            let end = rest
                .find(';')
                .filter(|p| *p <= 10)
                .map(|p| p + 1)
                .unwrap_or(1);
            transition(&mut out, &mut active_color, color_at(text_pos));
            out.push_str(&rest[..end]);
            text_pos += decode_html_entity(&rest[..end]).len();
            i += end;
            continue;
        }
        let text_end = rest.find(['<', '&']).unwrap_or(rest.len());
        for c in rest[..text_end].chars() {
            transition(&mut out, &mut active_color, color_at(text_pos));
            out.push(c);
            text_pos += c.len_utf8();
        }
        i += text_end;
    }
    transition(&mut out, &mut active_color, None);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> (HashMap<String, String>, HashSet<String>) {
        let mut g = HashMap::new();
        g.insert("solitude".to_string(), "孤独".to_string());
        g.insert("murmur".to_string(), "低语".to_string());
        let mut k = HashSet::new();
        k.insert("murmur".to_string());
        (g, k)
    }

    #[test]
    fn annotates_unknown_words_only() {
        let (g, k) = ctx();
        let out = annotate_html("<p>The solitude and murmur.</p>", &g, &k);
        assert_eq!(
            out,
            "<p>The <ruby>solitude\u{E000}孤独</ruby> and murmur.</p>"
        );
    }

    #[test]
    fn flattens_lists_to_bullet_paragraphs() {
        let src = "<ul><li>alpha</li><li>beta <b>x</b></li></ul>";
        assert_eq!(flatten_lists(src), "<p>• alpha</p><p>• beta <b>x</b></p>");
    }

    #[test]
    fn flattens_ordered_lists_to_crisp_text_numbers() {
        let src = "<ol start=\"3\"><li>alpha</li><li>beta <b>x</b></li></ol>";
        assert_eq!(flatten_lists(src), "<p>3. alpha</p><p>4. beta <b>x</b></p>");
    }

    #[test]
    fn flatten_strips_nested_p_inside_li() {
        let src = "<ul><li><p>alpha</p></li><li><p>beta</p><p>gamma</p></li></ul>";
        assert_eq!(flatten_lists(src), "<p>• alpha</p><p>• betagamma</p>");
    }

    #[test]
    fn injects_marks_across_tags() {
        // "abc def" 高亮 [2,5) → "ab<mark>c d</mark>ef"，跨 <b> 边界拆段
        let out = inject_marks("ab<b>c d</b>ef", &[(2, 5)]);
        assert_eq!(out, "ab<b><mark>c d</mark></b>ef");
        let out2 = inject_marks("abcdef", &[(1, 3)]);
        assert_eq!(out2, "a<mark>bc</mark>def");
    }

    #[test]
    fn injects_per_mark_colors_without_stale_widget_state() {
        let out = inject_colored_marks("abcdef", &[(1, 3, "blue"), (4, 6, "orange")]);
        assert_eq!(
            out,
            "a<mark>bc\u{E001}blue</mark>d<mark>ef\u{E001}orange</mark>"
        );
        let across = inject_colored_marks("a<b>bc</b>d", &[(0, 4, "teal")]);
        assert_eq!(
            across,
            "<mark>a\u{E001}teal</mark><b><mark>bc\u{E001}teal</mark></b><mark>d\u{E001}teal</mark>"
        );
        let ruby = inject_colored_marks("a<ruby>word\u{E000}释义</ruby>z", &[(2, 3, "blue")]);
        assert_eq!(ruby, "a<ruby>word\u{E000}释义\u{E001}blue</ruby>z");

        // TextFlow cursors are UTF-8 byte offsets: the em dash occupies [1, 4).
        let unicode = inject_colored_marks("A—B", &[(1, 4, "yellow")]);
        assert_eq!(unicode, "A<mark>—\u{E001}yellow</mark>B");
        let entity = inject_colored_marks("A&#x2014;B", &[(1, 4, "yellow")]);
        assert_eq!(entity, "A<mark>&#x2014;\u{E001}yellow</mark>B");
    }

    #[test]
    fn exports_rendered_text_and_cross_block_range() {
        let blocks = vec![
            "<p>A &amp; <ruby>word\u{E000}词\u{E001}blue</ruby>.</p>".to_string(),
            "<p><mark>Next\u{E001}yellow</mark> line</p>".to_string(),
        ];
        assert_eq!(rendered_text(&blocks[0]), "A & word.");
        assert_eq!(rendered_text(&blocks[1]), "Next line");
        assert_eq!(rendered_text("<pre>a<br>  b</pre>"), "a\n  b");
        assert_eq!(
            rendered_text_for_range(&blocks, ((0, 4), (1, 4))),
            "word.\nNext"
        );
        let unicode_blocks = vec!["<p>A—B</p>".to_string()];
        assert_eq!(
            rendered_text_for_range(&unicode_blocks, ((0, 1), (0, 4))),
            "—"
        );
    }

    #[test]
    fn removes_ruby_in_place_for_lemma() {
        let src =
            "<p>The <ruby>solitude\u{E000}孤独</ruby> and <ruby>murmurs\u{E000}低语</ruby>.</p>";
        let out = remove_ruby_for_lemma(src, "murmur");
        assert_eq!(
            out,
            "<p>The <ruby>solitude\u{E000}孤独</ruby> and murmurs.</p>"
        );
    }

    #[test]
    fn skips_tables() {
        let (g, k) = ctx();
        let out = annotate_html(
            "<table><tr><td>solitude</td></tr></table><p>solitude</p>",
            &g,
            &k,
        );
        assert!(out.starts_with("<table><tr><td>solitude</td></tr></table>"));
        assert!(out.contains("<ruby>solitude\u{E000}孤独</ruby>"));
    }

    #[test]
    fn skips_code_and_entities() {
        let (g, k) = ctx();
        let out = annotate_html("<pre>solitude</pre>&amp; solitude", &g, &k);
        assert_eq!(
            out,
            "<pre>solitude</pre>&amp; <ruby>solitude\u{E000}孤独</ruby>"
        );
    }

    #[test]
    fn leaves_link_text_intact_so_html_link_keeps_all_of_its_content() {
        let glosses = HashMap::from([("mutexes".to_string(), "互斥体".to_string())]);
        let html = "<p>See <a href=\"ch01.xhtml#mutexes\">Locking: Mutexes and RwLocks</a>, then mutexes.</p>";
        let annotated = annotate_html(html, &glosses, &HashSet::new());
        assert!(
            annotated.contains("<a href=\"ch01.xhtml#mutexes\">Locking: Mutexes and RwLocks</a>")
        );
        assert!(annotated.contains("then <ruby>mutexes\u{e000}互斥体</ruby>"));
    }
}
