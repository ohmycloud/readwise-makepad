/// M0 验证用的模拟 EPUB 章节，覆盖常见排版元素：
/// 标题层级、中英混排段落、加粗/斜体、引用、列表、ruby 注音。
pub const SAMPLE_HEAD: &str = r#"
<h1>第一章 山月大记</h1>
<h2>一</h2>
<p>陇西的李征博学多才，天宝末年，年纪轻轻便名登虎榜。随后被补任江南尉，但他生性狷介，自视甚高，不甘于卑贱的官职。</p>
<p>This paragraph mixes <b>bold English</b> and <i>italic text</i> with 中文标点、引号"如此"，以及数字 1234 和符号 —— 用于验证中英混排的断行与字距。</p>
<blockquote>不敢发露的珍珠般的自尊心，与不肯藏拙的怯懦的羞耻心——这两者在我心中，都化作了猛虎。</blockquote>
<h2>二</h2>
<p>注音验证：<ruby>山月<rt>さんげつ</rt></ruby>照着<ruby>旷野<rt>こうや</rt></ruby>，<ruby>虎<rt>とら</rt></ruby>在草丛中长啸。若 ruby 标签不被支持，此处会退化为普通文本。</p>
<ul>
<li>无序列表项一：验证列表缩进</li>
<li>无序列表项二：验证换行与项目符号</li>
</ul>
<ol>
<li>有序列表项，编号渲染</li>
<li>第二项</li>
</ol>
"#;

/// Ruby 注音 demo：英文正文 + 生词中文释义（None 表示普通词）。
/// 模拟真实场景：不是全文翻译，只给生词标注，帮助快速阅读英文书。
const GLOSSED_TEXT: &[(&str, Option<&str>)] = &[
    ("The", None), ("lighthouse", Some("灯塔")), ("keeper", None), ("was", None),
    ("a", None), ("taciturn", Some("沉默寡言的")), ("man", None), ("with", None),
    ("a", None), ("weather-beaten", Some("饱经风霜的")), ("face.", None),
    ("He", None), ("had", None), ("grown", None), ("used", None), ("to", None),
    ("solitude,", Some("孤独")), ("to", None), ("the", None),
    ("murmur", Some("低语")), ("of", None), ("waves", None), ("against", None),
    ("the", None), ("rocks.", None), ("At", None), ("dusk", Some("黄昏")),
    ("the", None), ("lamp", None), ("gleamed", Some("闪烁")), ("across", None),
    ("the", None), ("water,", None), ("and", None), ("a", None), ("strange", None),
    ("melancholy", Some("忧郁")), ("settled", None), ("over", None), ("him,", None),
    ("tempered", Some("冲淡；调和")), ("only", None), ("by", None), ("the", None),
    ("resilience", Some("韧性")), ("he", None), ("had", None), ("earned", None),
    ("from", None), ("years", None), ("of", None), ("keeping", None),
    ("vigil", Some("守夜")), ("alone.", None),
];

/// 把 (词, 释义) 序列拼成带 <ruby t="..."> 的 HTML 段落。
pub fn build_glossed_paragraph() -> String {
    let mut html = String::from(
        "<h2>Ruby 注音验证</h2>\
         <p><b>下面段落里带绿色小字注音的生词可以点击，点击后看右上角状态栏：</b></p><p>",
    );
    for (word, gloss) in GLOSSED_TEXT {
        match gloss {
            Some(g) => html.push_str(&format!("<ruby t=\"{}\">{}</ruby> ", g, word)),
            None => {
                html.push_str(word);
                html.push(' ');
            }
        }
    }
    html.push_str("</p>");
    html
}

/// 生成章节的段落列表（PortalList 虚拟化按段渲染）。
/// 正文中的词全部包成 <a href="w:词"> 链接，验证点词命中。
pub fn build_paragraphs(paragraphs: usize) -> Vec<String> {
    let mut items = Vec::with_capacity(paragraphs + 3);
    items.push(build_glossed_paragraph());
    items.push(strip_ruby(SAMPLE_HEAD));
    items.push(
        "<p><b>点词验证：以下正文中每个词都可点击，点击后看窗口右上角状态栏。</b></p>".to_string(),
    );
    let para = "月光如水，虎影徘徊。他想起自己曾经的诗篇，那些未能传世的句子，\
                如今只能在旷野的风里独自吟诵。Grammar and rhythm survive even when the poet does not. \
                这一段用于填充长文，验证 PortalList 虚拟化下的滚动流畅度。";
    let wrapped = wrap_words(para);
    for i in 1..=paragraphs {
        items.push(format!("<p>第 {} 段——{}</p>", i, wrapped));
    }
    items
}

/// Html widget 会整体丢弃未知标签（含 <ruby>）的内容，
/// 渲染前把 <rt>…</rt> 剔除、保留正文，避免丢字。
pub fn strip_ruby(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("<rt>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</rt>") {
            Some(end) => rest = &rest[start + end + "</rt>".len()..],
            None => {
                rest = &rest[start + "<rt>".len()..];
                break;
            }
        }
    }
    out.push_str(rest);
    out.replace("<ruby>", "").replace("</ruby>", "")
}

/// 英文按单词、CJK 按单字包成链接（真实产品会换成 readwise 已有的分词器）。
pub fn wrap_words(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_alphanumeric() {
            let mut word = String::new();
            word.push(c);
            while let Some(&n) = chars.peek() {
                if n.is_ascii_alphanumeric() || n == '\'' {
                    word.push(n);
                    chars.next();
                } else {
                    break;
                }
            }
            out.push_str(&format!("<a href=\"w:{word}\">{word}</a>"));
        } else if is_cjk(c) {
            out.push_str(&format!("<a href=\"w:{c}\">{c}</a>"));
        } else {
            out.push(c);
        }
    }
    out
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x3040..=0x30FF)
}
