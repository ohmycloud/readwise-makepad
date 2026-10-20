use super::*;
use crate::books::epub_meta::split_blocks;
use pdf_oxide::layout::{Color, FontWeight};
fn span(text: &str, x: f32, y: f32, size: f32, bold: bool) -> TextSpan {
    let doc = super::super::open(super::super::tests::heading_fixture()).unwrap();
    let mut s = doc.extract_spans(0).unwrap().remove(0);
    s.text = text.into();
    s.font_name = "Helvetica".into();
    s.font_size = size;
    s.font_weight = if bold {
        FontWeight::Bold
    } else {
        FontWeight::Normal
    };
    s.is_italic = false;
    s.is_monospace = false;
    s.heading_level = None;
    s.bbox = Rect::new(x, y, text.chars().count() as f32 * 6.0, size);
    s.char_widths.clear();
    s.char_x_offsets.clear();
    s.color = Color::black();
    s.rotation_degrees = 0.0;
    s.page_rotation_applied = 0;
    s
}
#[test]
fn multiline_bounding_box_uses_glyph_color_and_does_not_hide_smaller_link() {
    let first = "Verifiers are ordinary prose before self-";
    let second = "consistency and Best-of-N samples. More ordinary prose.";
    let spans = [
        span(first, 10.0, 100.0, 12.0, false),
        span(second, 10.0, 85.0, 12.0, false),
    ];
    // Span-level colors are all black after pdf_oxide merges same-font runs.
    let mut chars: Vec<_> = spans.iter().flat_map(TextSpan::to_chars).collect();
    let best = second.find("Best-of-N").unwrap();
    for (i, c) in chars.iter_mut().enumerate() {
        let linked = if i < first.len() {
            i >= first.len() - 5
        } else {
            let i = i - first.len();
            i < "consistency".len() || (i >= best && i < best + "Best-of-N samples".len())
        };
        if linked {
            c.color = Color {
                r: 0.55,
                g: 0.0,
                b: 0.07,
            };
        }
    }
    let broad = Rect::new(10.0, 83.0, spans[1].bbox.width, 30.0);
    let small = Rect::new(
        10.0 + best as f32 * 6.0,
        83.0,
        "Best-of-N samples".len() as f32 * 6.0,
        13.0,
    );
    let links = refine_links(
        &[
            (broad, "https://example.test/self-consistency".into()),
            (small, "https://example.test/best-of-n".into()),
        ],
        &chars,
    );
    let html = format!("<p><a href=\"https://wrong.test\">{first} {second}</a></p>");
    let (fixed, _) = project(&html, &spans, &links);
    assert!(fixed.contains("Verifiers are ordinary prose before <a href=\"https://example.test/self-consistency\">self-</a>"), "{fixed}");
    assert!(
        fixed.contains("href=\"https://example.test/self-consistency\">consistency</a> and "),
        "{fixed}"
    );
    assert!(
        fixed.contains(
            "href=\"https://example.test/best-of-n\">Best-of-N samples</a>. More ordinary prose."
        ),
        "{fixed}"
    );
    assert_eq!(rendered_text(&fixed), rendered_text(&html));
}
#[test]
fn ambiguous_uncolored_wrapped_rectangle_fails_closed_but_single_line_link_stays() {
    let spans = [
        span("Plain prose and a link", 10.0, 100.0, 12.0, false),
        span("continuation and more prose", 10.0, 85.0, 12.0, false),
    ];
    let chars: Vec<_> = spans.iter().flat_map(TextSpan::to_chars).collect();
    let links = refine_links(
        &[
            (
                Rect::new(10.0, 83.0, 200.0, 30.0),
                "https://example.test/unknown".into(),
            ),
            (
                Rect::new(10.0, 99.0, 30.0, 13.0),
                "https://example.test/plain".into(),
            ),
        ],
        &chars,
    );
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].1, "https://example.test/plain");
}
#[test]
fn ordinary_body_is_not_bold_just_because_code_or_footnotes_lower_page_mean_size() {
    let heading =
        "two categories of modifying the proposal distribution, namely supervised fine-tuning";
    let prose = "We ended the chapter by exploring upcoming fields in reasoning LLMs, including new methods.";
    let spans = [
        span(heading, 10.0, 200.0, 12.0, false),
        span(prose, 10.0, 170.0, 12.0, false),
        span("Summary", 10.0, 250.0, 16.8, true),
    ];
    let html = format!("<h2>Summary</h2><h3><strong>{heading}</strong></h3><p>{prose}</p>");
    let (fixed, map) = project(&html, &spans, &[]);
    let blocks = split_blocks(&fixed);
    assert_eq!(blocks[0], "<h2>Summary</h2>");
    assert_eq!(blocks[1], format!("<p>{heading}</p>"));
    assert!(map[1].layout_changed);
    assert_eq!(map[1].old_len, heading.len());
}
#[test]
fn spurious_heading_fragments_become_one_paragraph_without_losing_inline_code_or_utf8() {
    let a = "Let’s explore this by integrating a Memory module into your TinyAgent. The module is quite";
    let b = "straightforward and is merely a list of dictionaries that track the role and its content, exactly as";
    let c = "shown in Figure 4-5:";
    let d = "A separate real paragraph must remain separate after a genuine paragraph gap.";
    let spans = [
        span(a, 10.0, 300.0, 12.0, false),
        span(b, 10.0, 284.4, 12.0, false),
        span(c, 10.0, 269.4, 12.0, false),
        span(d, 10.0, 240.0, 12.0, false),
    ];
    let html = format!("<h3>{a}</h3><p>{b}</p><h3>{c}</h3><p>{d}</p>");
    let (fixed, map) = project(&html, &spans, &[]);
    let blocks = split_blocks(&fixed);
    assert_eq!(blocks.len(), 2, "{fixed}");
    assert_eq!(rendered_text(&blocks[0]), format!("{a} {b} {c}"));
    assert_eq!((map[1].block, map[1].prefix), (0, a.len() + 1));
    assert_eq!((map[2].block, map[2].prefix), (0, a.len() + b.len() + 2));
    assert_eq!(map[3].block, 1);
}
#[test]
fn native_heading_bold_text_images_code_columns_and_real_paragraphs_are_preserved() {
    let body =
        "A sufficiently long normal body paragraph establishes the document prose font size.";
    let quote =
        "A long large-font pull quote must not make a smaller genuine heading look like prose.";
    let mut tagged = span("Native tagged heading", 10.0, 400.0, 12.0, false);
    tagged.heading_level = Some(3);
    let spans = [
        span("Medium heading", 10.0, 500.0, 16.0, false),
        span(quote, 10.0, 450.0, 20.0, false),
        tagged,
        span("Genuine heading", 10.0, 300.0, 12.0, true),
        span(body, 10.0, 270.0, 12.0, false),
        span("Bold words", 10.0, 240.0, 12.0, true),
        span("Left", 10.0, 200.0, 12.0, false),
        span("Right", 300.0, 200.0, 12.0, false),
    ];
    let html = format!("<h2>Medium heading</h2><p>{quote}</p><h3>Native tagged heading</h3><h3>Genuine heading</h3><p>{body}</p><p><strong>Bold words</strong></p><p><img src=\"pdf-assets-v3/page-0-0.png\"/></p><pre><code>let x = 1;</code></pre><p>Left</p><p>Right</p>");
    let (fixed, _) = project(&html, &spans, &[]);
    assert_eq!(fixed, split_blocks(&html).join("\n"));
}
#[test]
fn same_source_line_inline_code_fragments_do_not_force_a_paragraph_break() {
    let mut code = span("Memory", 70.0, 200.0, 12.0, false);
    code.is_monospace = true;
    let spans = [
        span("Use this (", 10.0, 200.0, 12.0, false),
        code,
        span(") safely.", 106.0, 200.0, 12.0, false),
    ];
    let html = "<p>Use this (</p><p><code>Memory</code></p><p>) safely.</p>";
    let (fixed, _) = project(html, &spans, &[]);
    assert_eq!(fixed, "<p>Use this (<code>Memory</code>) safely.</p>");
}
