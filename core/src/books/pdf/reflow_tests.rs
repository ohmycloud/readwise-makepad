use super::*;
use crate::books::epub_meta::code_block_text;
use pdf_oxide::{geometry::Rect, layout::FontWeight};

fn span(text: &str, x: f32, y: f32, mono: bool, size: f32) -> TextSpan {
    let doc = super::super::open(super::super::tests::heading_fixture()).unwrap();
    let mut s = doc.extract_spans(0).unwrap().remove(0);
    s.text = text.into();
    s.font_name = if mono {
        "Subset+FiraCode-Regular"
    } else {
        "Helvetica"
    }
    .into();
    s.font_size = size;
    s.bbox = Rect::new(x, y, text.chars().count() as f32 * 6.0, size);
    s.font_weight = FontWeight::Normal;
    s.is_italic = false;
    s.is_monospace = false;
    s.char_widths.clear();
    s.char_x_offsets.clear();
    s.rotation_degrees = 0.0;
    s.page_rotation_applied = 0;
    s
}
fn code(text: &str, x: f32, y: f32) -> TextSpan {
    span(text, x, y, true, 10.0)
}
fn body(text: &str, x: f32, y: f32) -> TextSpan {
    span(text, x, y, false, 12.0)
}
fn verify(repair: &Repair) {
    let target: Vec<_> = split_blocks(&repair.html)
        .iter()
        .map(|b| rendered_text(b))
        .collect();
    let mut used = std::collections::HashSet::new();
    for (text, points) in repair.source.iter().zip(&repair.points) {
        assert_eq!(points.len(), text.len() + 1);
        for (byte, ch) in text.char_indices().filter(|(_, c)| !c.is_whitespace()) {
            let (b, to) = points[byte];
            assert!(used.insert((b, to)), "duplicate glyph");
            assert_eq!(
                &target[b][to..to + ch.len_utf8()],
                &text[byte..byte + ch.len_utf8()]
            );
        }
    }
    for (b, text) in target.iter().enumerate() {
        for (byte, ch) in text.char_indices() {
            if !ch.is_whitespace() {
                assert!(used.contains(&(b, byte)));
            }
        }
    }
}
#[test]
fn relocated_header_and_diagnostic_gutters_rejoin_without_losing_repeated_bars() {
    let spans = vec![
        body("CHAPTER SIX", 20.0, 300.0),
        body("Compiler says", 20.0, 270.0),
        code("error: denied", 20.0, 240.0),
        code("|", 32.0, 224.0),
        code("^^^^", 92.0, 224.0),
        code("|", 32.0, 208.0),
        code("private constructor", 92.0, 208.0),
        code("|", 32.0, 192.0),
        body("Continue reading.", 20.0, 160.0),
    ];
    let html="<p>Compiler says</p><pre><code>error: denied\n|\n|\n|</code></pre><p>Continue reading.</p><p>CHAPTER SIX</p><pre><code>^^^^\nprivate constructor</code></pre>";
    let r = project(html, &spans, &[]).unwrap();
    verify(&r);
    let b = split_blocks(&r.html);
    assert_eq!(b.len(), 4, "{}", r.html);
    assert_eq!(rendered_text(&b[0]), "CHAPTER SIX");
    assert_eq!(
        code_block_text(&b[2]).unwrap(),
        "error: denied\n  |         ^^^^\n  |         private constructor\n  |"
    );
    assert_eq!(rendered_text(&b[3]), "Continue reading.");
}
#[test]
fn blank_lines_stay_inside_one_listing_and_font_changes_do_not_split_a_line() {
    let mut bold = code("true", 80.0, 152.0);
    bold.font_name = "Subset+FiraCode-SemiBold".into();
    bold.font_weight = FontWeight::Bold;
    let spans = vec![
        code("fn main() {", 20.0, 200.0),
        code("let ok = ", 44.0, 184.0),
        code("true;", 98.0, 184.0),
        code("// gap", 44.0, 152.0),
        bold,
        code("}", 20.0, 136.0),
    ];
    let html =
        "<pre><code>fn main() {\nlet ok = true;</code></pre><pre><code>// gaptrue\n}</code></pre>";
    let r = project(html, &spans, &[]).unwrap();
    verify(&r);
    assert_eq!(split_blocks(&r.html).len(), 1);
    let code = code_block_text(&r.html).unwrap();
    assert!(code.contains("\n\n    // gap"), "{code}");
    assert!(code.contains("let ok = true;"), "{code}");
}
#[test]
fn fragmented_mixed_size_heading_is_one_line_and_inline_code_stays_in_prose() {
    let mut spans = vec![
        body("Normal body establishes size.", 20.0, 260.0),
        span("6.7", 20.0, 230.0, false, 16.0),
        span("Error", 44.0, 230.0, false, 16.0),
        span("As", 80.0, 230.0, false, 16.0),
        span("Values -", 98.0, 230.0, false, 16.0),
        span("Result", 152.0, 230.0, true, 12.0),
        body("Use ", 20.0, 200.0),
        code("Result", 44.0, 200.0),
        body(" safely.", 80.0, 200.0),
    ];
    for s in &mut spans[1..6] {
        s.font_weight = FontWeight::Bold;
    }
    let html="<p>Normal body establishes size.</p><p>6.7</p><h3>Error</h3><h3>As</h3><h3>Values - Result</h3><p>Use Result safely.</p>";
    let r = project(html, &spans, &[]).unwrap();
    verify(&r);
    let b = split_blocks(&r.html);
    assert_eq!(b.len(), 3, "{}", r.html);
    assert_eq!(rendered_text(&b[1]), "6.7 Error As Values - Result");
    assert!(b[1].contains("<code>Result</code>"));
    assert!(b[1].starts_with("<h3>"));
    assert_eq!(b[2], "<p>Use <code>Result</code> safely.</p>");
}
#[test]
fn one_old_block_can_map_to_prose_on_both_sides_of_a_listing() {
    let spans = vec![
        body("Before", 20.0, 240.0),
        code("fn test() {}", 20.0, 200.0),
        body("After", 20.0, 160.0),
    ];
    let html = "<p>Before After</p><pre><code>fn test() {}</code></pre>";
    let r = project(html, &spans, &[]).unwrap();
    verify(&r);
    assert_eq!(split_blocks(&r.html).len(), 3);
    assert_eq!(r.points[0][0], (0, 0));
    assert_eq!(r.points[0][7], (2, 0));
}
#[test]
fn columns_images_explicit_line_breaks_and_unmatched_text_fail_closed() {
    let spans = vec![
        code("left call();", 20.0, 200.0),
        code("right call();", 340.0, 200.0),
    ];
    assert!(project(
        "<pre><code>left call();</code></pre><pre><code>right call();</code></pre>",
        &spans,
        &[]
    )
    .is_none());
    let spans = vec![code("fn test() {}", 20.0, 200.0)];
    for html in [
        "<p><img src=\"a.png\"/></p><p>fn test() {}</p>",
        "<p>fn test() {}<br/>end</p>",
        "<p>unknown text</p>",
    ] {
        assert!(project(html, &spans, &[]).is_none());
    }
}
#[test]
fn different_examples_with_intervening_prose_never_merge() {
    let spans = vec![
        code("let first = 1;", 20.0, 240.0),
        body("Next example:", 20.0, 210.0),
        code("let next = 2;", 20.0, 180.0),
    ];
    let html="<pre><code>let first = 1;</code></pre><p>Next example:</p><pre><code>let next = 2;</code></pre>";
    if let Some(r) = project(html, &spans, &[]) {
        verify(&r);
        let b = split_blocks(&r.html);
        assert_eq!(b.len(), 3);
        assert!(b[1].starts_with("<p>"));
    }
}

#[test]
fn source_emphasis_links_entities_and_multibyte_offsets_survive_geometric_reordering() {
    let mut bold = body("café", 50.0, 220.0);
    bold.font_weight = FontWeight::Bold;
    let mut italic = body("雪", 80.0, 220.0);
    italic.is_italic = true;
    let spans = vec![
        body("Read", 20.0, 220.0),
        bold,
        italic,
        body(" & docs.", 86.0, 220.0),
        code("fn test() {}", 20.0, 180.0),
    ];
    let links = vec![(
        Rect::new(104.0, 220.0, 24.0, 12.0),
        "https://example.org/docs".into(),
    )];
    let r = project(
        "<p>Read café 雪 &amp; docs.</p><pre><code>fn test() {}</code></pre>",
        &spans,
        &links,
    )
    .unwrap();
    verify(&r);
    assert!(r.html.contains("<strong> café</strong>"), "{}", r.html);
    assert!(r.html.contains("<em> 雪</em>"), "{}", r.html);
    assert!(
        r.html.contains("href=\"https://example.org/docs\""),
        "{}",
        r.html
    );
}
#[test]
fn raised_footnote_labels_stay_with_their_own_wrapped_paragraph_not_the_next_note() {
    let spans = vec![
        code("fn test() {}", 20.0, 260.0),
        span("7", 20.0, 203.0, false, 6.0),
        span("First note has a normal body.", 27.0, 200.0, false, 9.0),
        span("8", 20.0, 181.0, false, 6.0),
        span("Second note stays separate.", 27.0, 178.0, false, 9.0),
    ];
    let r=project("<pre><code>fn test() {}</code></pre><p>7 First note has a normal body.</p><p>8 Second note stays separate.</p>",&spans,&[]).unwrap();
    verify(&r);
    let b = split_blocks(&r.html);
    assert_eq!(b.len(), 3, "{}", r.html);
    assert_eq!(rendered_text(&b[1]), "7 First note has a normal body.");
    assert_eq!(rendered_text(&b[2]), "8 Second note stays separate.");
}
#[test]
fn two_blank_code_lines_are_not_a_block_boundary() {
    let spans = vec![
        code("// first", 20.0, 260.0),
        code("fn test() {", 20.0, 244.0),
        code("call();", 44.0, 196.0),
        code("}", 20.0, 180.0),
    ];
    let r = project(
        "<pre><code>// first\nfn test() {</code></pre><pre><code>call();\n}</code></pre>",
        &spans,
        &[],
    )
    .unwrap();
    verify(&r);
    assert_eq!(split_blocks(&r.html).len(), 1);
    assert!(code_block_text(&r.html)
        .unwrap()
        .contains("{\n\n\n    call();"));
}
#[test]
fn narrow_source_spaces_do_not_turn_prose_words_into_concatenations() {
    let spans = vec![
        body("Read", 20.0, 220.0),
        body("words", 44.5, 220.0),
        code("fn test() {}", 20.0, 180.0),
    ];
    let r = project(
        "<p>Read words</p><pre><code>fn test() {}</code></pre>",
        &spans,
        &[],
    )
    .unwrap();
    verify(&r);
    assert_eq!(rendered_text(&split_blocks(&r.html)[0]), "Read words");
}
#[test]
fn a_new_first_line_indent_restores_a_paragraph_instead_of_merging_it() {
    let first = "A sufficiently long previous paragraph can still end with a nearly full line.";
    let spans = vec![
        body(first, 20.0, 240.0),
        body("Next paragraph.", 36.0, 225.0),
        code("fn test() {}", 20.0, 180.0),
    ];
    let r = project(
        &format!("<p>{first} Next paragraph.</p><pre><code>fn test() {{}}</code></pre>"),
        &spans,
        &[],
    )
    .unwrap();
    verify(&r);
    let b = split_blocks(&r.html);
    assert_eq!(b.len(), 3, "{}", r.html);
    assert_eq!(rendered_text(&b[0]), first);
    assert_eq!(rendered_text(&b[1]), "Next paragraph.");
}

#[test]
fn three_digit_running_page_numbers_at_either_margin_are_not_body_columns() {
    for number_on_left in [true, false] {
        let (first, last) = if number_on_left {
            ("172", "CHAPTER SIX")
        } else {
            ("CHAPTER SIX", "173")
        };
        let spans = vec![
            body(first, 20.0, 280.0),
            body(last, 300.0, 280.0),
            code("fn test() {}", 20.0, 240.0),
        ];
        let html = format!("<pre><code>fn test() {{}}</code></pre><p>{first} {last}</p>");
        let r = project(&html, &spans, &[]).unwrap();
        verify(&r);
        let b = split_blocks(&r.html);
        assert_eq!(rendered_text(&b[0]), format!("{first} {last}"));
        assert!(b[1].starts_with("<pre>"));
    }
}

#[test]
fn short_independent_columns_also_keep_the_old_projection() {
    let spans = vec![code("cat", 20.0, 200.0), code("dog", 340.0, 200.0)];
    assert!(project(
        "<pre><code>cat</code></pre><pre><code>dog</code></pre>",
        &spans,
        &[]
    )
    .is_none());
}

#[test]
fn fresh_imports_use_the_same_code_grouping_as_old_snapshot_repairs() {
    let bytes = super::super::tests::pdf_with_font(
        &[
            r"BT /F1 10 Tf 20 260 Td (fn main\(\) {) Tj 24 -16 Td (let value = 1;) Tj -24 -32 Td (}) Tj ET",
        ],
        false,
        "Courier",
    );
    let snapshot = super::super::extract(bytes, "test".into()).unwrap();
    assert_eq!(snapshot.extractor, super::super::EXTENDED_REFLOW_EXTRACTOR);
    assert_eq!(
        split_blocks(&snapshot.pages[0]).len(),
        1,
        "{}",
        snapshot.pages[0]
    );
    let code = code_block_text(&snapshot.pages[0]).unwrap();
    assert_eq!(code, "fn main() {\n    let value = 1;\n\n}");
}

#[test]
#[ignore = "requires an explicit local PDF and source snapshot; never writes user data"]
fn audit_local_projection() {
    let path = std::path::PathBuf::from(
        std::env::var_os("READWISE_PDF_AUDIT_SOURCE").expect("set PDF path"),
    );
    let snapshot = std::path::PathBuf::from(
        std::env::var_os("READWISE_PDF_AUDIT_SNAPSHOT").expect("set snapshot path"),
    );
    let out = std::path::PathBuf::from(
        std::env::var_os("READWISE_PDF_AUDIT_OUTPUT").expect("set isolated output"),
    );
    std::fs::create_dir_all(&out).unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(snapshot).unwrap()).unwrap();
    let pages = value.get("snapshot").unwrap_or(&value)["pages"]
        .as_array()
        .unwrap();
    let doc = super::super::open(std::fs::read(path).unwrap()).unwrap();
    let mut changed = Vec::new();
    for (page, html) in pages.iter().enumerate() {
        let html = html.as_str().unwrap();
        if let Some(r) = super::repair(&doc, page, html).unwrap() {
            verify(&r);
            std::fs::write(out.join(format!("{page}.html")), &r.html).unwrap();
            changed.push(page);
        }
    }
    std::fs::write(
        out.join("changed.json"),
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    println!("Audited {} pages, changed {}", pages.len(), changed.len());
}
