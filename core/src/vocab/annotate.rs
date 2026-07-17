use super::lemma::lemmatize;
use std::collections::{HashMap, HashSet};

/// 给章节 HTML 注入生词注音：文本节点里的英文词，若其原形在释义表中
/// 且不在已认识集合中，包成 <ruby t="释义">词</ruby>。
/// 跳过标签内部、code/pre 内容、HTML 实体（老项目 GlossAnnotator 的字符串版）。
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
            // code/pre：不注音；table：单元格行高不可控，注音会被裁，跳过
            if name == "code" || name == "pre" || name == "table" {
                code_depth += if is_close { -1 } else { 1 };
                code_depth = code_depth.max(0);
            }
            out.push_str(tag);
            i += gt_rel + 1;
            continue;
        }
        if rest.starts_with('&') {
            // HTML 实体整体拷贝，防止 &amp; 里的 "amp" 被当单词
            let end = rest.find(';').filter(|p| *p <= 10).map(|p| p + 1).unwrap_or(1);
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

/// 把列表拍平成"• "段落。TextFlow 的列表项换行是硬编码零间距（begin_list_item
/// 走 new_line_collapsed，无旋钮），注音在 li 里永远没有头顶空间；
/// 段落路径的间距（paragraph_margin）可配置，所以注音章节渲染前统一转换。
pub fn flatten_lists(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut li_depth: i32 = 0;
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
            ("ul", _) | ("ol", _) => {}
            ("li", false) => {
                li_depth += 1;
                out.push_str("<p>• ");
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
            let safe_gloss = gloss.replace('"', "'");
            out.push_str(&format!(
                "<ruby t=\"{}\">{}</ruby>",
                safe_gloss, word
            ));
            last = start + word.len();
        }
    }
    out.push_str(&text[last..]);
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
            "<p>The <ruby t=\"孤独\">solitude</ruby> and murmur.</p>"
        );
    }

    #[test]
    fn flattens_lists_to_bullet_paragraphs() {
        let src = "<ul><li>alpha</li><li>beta <b>x</b></li></ul>";
        assert_eq!(
            flatten_lists(src),
            "<p>• alpha</p><p>• beta <b>x</b></p>"
        );
    }

    #[test]
    fn flatten_strips_nested_p_inside_li() {
        let src = "<ul><li><p>alpha</p></li><li><p>beta</p><p>gamma</p></li></ul>";
        assert_eq!(
            flatten_lists(src),
            "<p>• alpha</p><p>• betagamma</p>"
        );
    }

    #[test]
    fn skips_tables() {
        let (g, k) = ctx();
        let out = annotate_html("<table><tr><td>solitude</td></tr></table><p>solitude</p>", &g, &k);
        assert!(out.starts_with("<table><tr><td>solitude</td></tr></table>"));
        assert!(out.contains("<ruby t=\"孤独\">solitude</ruby>"));
    }

    #[test]
    fn skips_code_and_entities() {
        let (g, k) = ctx();
        let out = annotate_html("<pre>solitude</pre>&amp; solitude", &g, &k);
        assert_eq!(
            out,
            "<pre>solitude</pre>&amp; <ruby t=\"孤独\">solitude</ruby>"
        );
    }
}
