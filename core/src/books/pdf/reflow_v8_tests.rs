use super::*;
use crate::books::epub_meta::code_block_text;
use pdf_oxide::{geometry::Rect, layout::FontWeight};
fn span(text: &str, x: f32, y: f32, mono: bool) -> TextSpan {
    let doc = super::super::open(super::super::tests::heading_fixture()).unwrap();
    let mut s = doc.extract_spans(0).unwrap().remove(0);
    s.text = text.into();
    s.font_name = if mono {
        "Subset+FiraCode-Regular"
    } else {
        "Helvetica"
    }
    .into();
    s.font_size = if mono { 10.0 } else { 12.0 };
    s.bbox = Rect::new(x, y, text.chars().count() as f32 * 6.0, s.font_size);
    s.font_weight = FontWeight::Normal;
    s.is_italic = false;
    s.is_monospace = false;
    s.char_widths.clear();
    s.char_x_offsets.clear();
    s.rotation_degrees = 0.0;
    s.page_rotation_applied = 0;
    s
}
fn code(t: &str, x: f32, y: f32) -> TextSpan {
    span(t, x, y, true)
}
fn body(t: &str, x: f32, y: f32) -> TextSpan {
    span(t, x, y, false)
}
fn italic(t: &str, x: f32, y: f32) -> TextSpan {
    let mut s = body(t, x, y);
    s.is_italic = true;
    s
}
fn verify(r: &Repair) {
    let target: Vec<_> = split_blocks(&r.html)
        .iter()
        .map(|s| rendered_text(s))
        .collect();
    let mut used = HashSet::new();
    for (text, points) in r.source.iter().zip(&r.points) {
        assert_eq!(points.len(), text.len() + 1);
        for (byte, ch) in text.char_indices().filter(|(_, c)| !c.is_whitespace()) {
            let p = points[byte];
            assert!(used.insert(p));
            assert_eq!(
                &target[p.0][p.1..p.1 + ch.len_utf8()],
                &text[byte..byte + ch.len_utf8()]
            );
        }
    }
    for (b, t) in target.iter().enumerate() {
        for (byte, ch) in t.char_indices() {
            if !ch.is_whitespace() {
                assert!(used.contains(&(b, byte)));
            }
        }
    }
}
#[test]
fn empty_list_wrappers_do_not_disable_code_grouping_for_the_whole_page() {
    let spans = vec![
        body("Next to your code:", 20.0, 260.0),
        code("// test code", 20.0, 230.0),
        code("mod tests {", 20.0, 198.0),
        code("use super::*;", 44.0, 182.0),
        code("// my tests", 44.0, 150.0),
        code("}", 20.0, 134.0),
    ];
    let html="<ul></ul><p>Next to your code:</p><pre><code>// test code</code></pre><pre><code>mod tests {\nuse super::*;</code></pre><pre><code>// my tests\n}</code></pre>";
    assert!(super::super::reflow::project(html, &spans, &[]).is_none());
    let r = project(html, &spans, &[]).unwrap();
    verify(&r);
    let b = split_blocks(&r.html);
    assert_eq!(b.len(), 2, "{}", r.html);
    assert!(code_block_text(&b[1])
        .unwrap()
        .contains("// test code\n\nmod tests {\n    use super::*;\n\n    // my tests\n}"));
}
#[test]
fn a_running_head_misread_as_an_ordered_list_rejoins_at_the_top() {
    let spans = vec![
        body("32", 20.0, 300.0),
        italic("CHAPTER", 180.0, 300.0),
        italic("SIGN", 228.0, 300.0),
        italic("UP", 258.0, 300.0),
        body("We read", 20.0, 270.0),
        body("a file.", 68.0, 270.0),
        code("fn config() {", 20.0, 240.0),
        code("load();", 44.0, 224.0),
        code("}", 20.0, 208.0),
    ];
    let html="<p>32</p><p>We read</p><pre><code>fn config() {\nload();</code></pre><p><em>CHAPTER</em></p><p>a file.</p><pre><code>}</code></pre><ol><li><em>SIGN</em></li></ol><p><em>UP</em></p>";
    let r = project(html, &spans, &[]).unwrap();
    verify(&r);
    let b = split_blocks(&r.html);
    assert_eq!(b.len(), 3, "{}", r.html);
    assert_eq!(rendered_text(&b[0]), "32 CHAPTER SIGN UP");
    assert!(!r.html.contains("<ol"));
    assert_eq!(rendered_text(&b[1]), "We read a file.");
    assert_eq!(
        code_block_text(&b[2]).unwrap(),
        "fn config() {\n    load();\n}"
    );
}
#[test]
fn real_semantic_lists_are_opaque_and_stop_code_merging() {
    let spans = vec![
        code("first();", 20.0, 260.0),
        body("café & first", 36.0, 230.0),
        body("second", 36.0, 214.0),
        code("next();", 20.0, 180.0),
    ];
    let list="<ul><li><strong>café</strong> &amp; first</li><li><a href=\"https://example.org\">second</a></li></ul>";
    let html = format!("<pre><code>first();</code></pre>{list}<pre><code>next();</code></pre>");
    let r = project(&html, &spans, &[]).unwrap();
    verify(&r);
    let b = split_blocks(&r.html);
    assert_eq!(b.len(), 3);
    assert_eq!(b[1], list);
    let index = r.source[1].find("second").unwrap();
    assert_eq!(r.points[1][index], (1, index));
}
#[test]
fn a_real_italic_list_is_not_unwrapped_without_a_running_page_counter() {
    let spans = vec![italic("item", 20.0, 260.0), code("next();", 20.0, 210.0)];
    let html = "<ol><li><em>item</em></li></ol><pre><code>next();</code></pre>";
    if let Some(r) = project(html, &spans, &[]) {
        verify(&r);
        assert!(r.html.starts_with("<ol><li><em>item</em></li></ol>"));
    }
}
#[test]
fn numbered_compiler_gutters_are_not_independent_columns() {
    let spans = vec![
        code("error[E0277]: cannot clone", 20.0, 300.0),
        code("10 | ||", 20.0, 280.0),
        code(".route(health)", 140.0, 280.0),
        code("11 | ||", 20.0, 260.0),
        code(".route(subscribe)", 140.0, 260.0),
        code("56 |", 20.0, 240.0),
        code("F: Fn() + Clone", 140.0, 240.0),
        code("|", 38.0, 220.0),
        body("A server expects a clone.", 20.0, 180.0),
    ];
    let html="<pre><code>error[E0277]: cannot clone\n10 | ||\n11 | ||\n56 |\n|</code></pre><pre><code>.route(health)\n.route(subscribe)\nF: Fn() + Clone</code></pre><p>A server expects a clone.</p>";
    assert!(super::super::reflow::project(html, &spans, &[]).is_none());
    let r = project(html, &spans, &[]).unwrap();
    verify(&r);
    let b = split_blocks(&r.html);
    assert_eq!(b.len(), 2, "{}", r.html);
    let c = code_block_text(&b[0]).unwrap();
    assert!(c.contains("10 | ||             .route(health)"), "{c}");
    assert!(c.contains("56 |                F: Fn() + Clone"), "{c}");
}
#[test]
fn diagnostic_context_does_not_authorise_merging_unrelated_code_columns() {
    let spans = vec![
        code("error[E0277]", 20.0, 300.0),
        code("left();", 20.0, 280.0),
        code("right();", 300.0, 280.0),
    ];
    assert!(project(
        "<pre><code>error[E0277]\nleft();</code></pre><pre><code>right();</code></pre>",
        &spans,
        &[]
    )
    .is_none());
}
#[test]
fn inline_error_word_in_prose_is_not_a_diagnostic_context_for_unrelated_columns() {
    let spans = vec![
        body("An ", 20.0, 300.0),
        code("error:", 50.0, 300.0),
        body(" word is not a diagnostic.", 90.0, 300.0),
        code("123|", 20.0, 260.0),
        code("let other_column = 1;", 260.0, 260.0),
    ];
    assert!(project("<p>An <code>error:</code> word is not a diagnostic.</p><pre><code>123|let other_column = 1;</code></pre>",&spans,&[]).is_none());
}

#[test]
fn different_source_files_do_not_merge_just_because_only_a_blank_line_separates_them() {
    let spans = vec![
        code("//! tests/check.rs", 20.0, 280.0),
        code("fn test() {}", 20.0, 264.0),
        code("#! Cargo.toml", 20.0, 232.0),
        code("[dev-dependencies]", 20.0, 216.0),
    ];
    let r=project("<pre><code>//! tests/check.rs\nfn test() {}\n\n#! Cargo.toml\n[dev-dependencies]</code></pre>",&spans,&[]).unwrap();
    verify(&r);
    let b = split_blocks(&r.html);
    assert_eq!(b.len(), 2, "{}", r.html);
    assert!(code_block_text(&b[0])
        .unwrap()
        .starts_with("//! tests/check.rs"));
    assert!(code_block_text(&b[1]).unwrap().starts_with("#! Cargo.toml"));
    assert_eq!(
        r.points[0][r.source[0].find("#! Cargo.toml").unwrap()],
        (1, 0)
    );
}
#[test]
fn a_raised_footnote_label_has_spacing_even_before_a_smaller_inline_code_font() {
    let mut label = body("4", 20.0, 263.0);
    label.font_size = 6.0;
    let mut word = code("App", 27.0, 260.0);
    word.font_size = 6.6;
    let mut prose = body(" is generic.", 45.0, 260.0);
    prose.font_size = 9.0;
    let r = project(
        "<p>4App is generic.</p><pre><code>next();</code></pre>",
        &[label, word, prose, code("next();", 20.0, 220.0)],
        &[],
    )
    .unwrap();
    verify(&r);
    assert!(
        rendered_text(&split_blocks(&r.html)[0]).starts_with("4 App"),
        "{}",
        r.html
    );
}

#[test]
fn original_character_origins_prevent_interleaving_letters_from_bad_span_offsets() {
    let mut wrong = body("write a function", 20.0, 260.0);
    let code = code("next();", 20.0, 220.0);
    let mut raw = wrong.to_chars();
    raw.extend(code.to_chars());
    for c in &mut raw {
        c.origin_x = c.bbox.x;
        c.origin_y = c.bbox.y;
        c.advance_width = c.bbox.width;
    }
    wrong.char_x_offsets = (0..wrong.text.chars().count())
        .map(|i| 20.0 + i as f32 * 6.0 + if i < 5 { 30.0 } else { 0.0 })
        .collect();
    let r = project_with_chars(
        "<p>write a function</p><pre><code>next();</code></pre>",
        &[wrong, code],
        &[],
        &raw,
    )
    .unwrap();
    verify(&r);
    assert_eq!(rendered_text(&split_blocks(&r.html)[0]), "write a function");
}

#[test]
fn image_table_and_printed_toc_projections_are_untouched() {
    let spans = vec![code("next();", 20.0, 260.0)];
    for html in [
        "<p><img src=\"a.png\"/></p><pre><code>next();</code></pre>",
        "<table><tr><td>a</td></tr></table><pre><code>next();</code></pre>",
        "<p>Contents<br/>Topic 1</p><pre><code>next();</code></pre>",
    ] {
        assert!(project(html, &spans, &[]).is_none());
    }
}

#[test]
fn fresh_v8_imports_keep_file_boundaries_and_use_original_character_positions() {
    let bytes = super::super::tests::pdf_with_font(
        &[
            r"BT /F1 10 Tf 20 260 Td (//! src/main.rs) Tj 0 -16 Td (fn main\(\) {}) Tj 0 -32 Td (#! Cargo.toml) Tj 0 -16 Td ([package]) Tj ET",
        ],
        false,
        "Courier",
    );
    let snapshot = super::super::extract(bytes, "test".into()).unwrap();
    assert_eq!(snapshot.extractor, super::super::EXTENDED_REFLOW_EXTRACTOR);
    let blocks = split_blocks(&snapshot.pages[0]);
    assert_eq!(blocks.len(), 2, "{}", snapshot.pages[0]);
    assert_eq!(
        code_block_text(&blocks[0]).unwrap(),
        "//! src/main.rs\nfn main() {}"
    );
    assert_eq!(
        code_block_text(&blocks[1]).unwrap(),
        "#! Cargo.toml\n[package]"
    );
}

#[test]
#[ignore = "requires explicit local PDF/snapshot/output; assert required pages, not just successful mappings"]
fn audit_local_v8_projection() {
    let path =
        std::path::PathBuf::from(std::env::var_os("READWISE_PDF_AUDIT_SOURCE").expect("PDF path"));
    let snapshot = std::path::PathBuf::from(
        std::env::var_os("READWISE_PDF_AUDIT_SNAPSHOT").expect("snapshot path"),
    );
    let out = std::path::PathBuf::from(
        std::env::var_os("READWISE_PDF_AUDIT_OUTPUT").expect("isolated output"),
    );
    let required: HashSet<usize> = std::env::var("READWISE_PDF_AUDIT_REQUIRED")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().unwrap())
        .collect();
    let expected: HashMap<usize, usize> = std::env::var("READWISE_PDF_AUDIT_EXPECT_CODE_COUNTS")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let (p, n) = s.split_once(':').unwrap();
            (p.parse().unwrap(), n.parse().unwrap())
        })
        .collect();
    std::fs::create_dir_all(&out).unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(snapshot).unwrap()).unwrap();
    let pages = value.get("snapshot").unwrap_or(&value)["pages"]
        .as_array()
        .unwrap();
    let doc = super::super::open(std::fs::read(path).unwrap()).unwrap();
    let mut report = Vec::new();
    let mut fixed = HashSet::new();
    for (page, html) in pages.iter().enumerate() {
        let html = html.as_str().unwrap();
        let (changed, after) = if let Some(r) = super::repair(&doc, page, html).unwrap() {
            verify(&r);
            std::fs::write(out.join(format!("{page}.html")), &r.html).unwrap();
            fixed.insert(page);
            (true, r.html)
        } else {
            (false, html.to_string())
        };
        if let Some(count) = expected.get(&page) {
            assert_eq!(
                after.matches("<pre>").count(),
                *count,
                "Code grouping is still wrong on required page {page}"
            );
        }
        report.push(serde_json::json!({"page":page,"changed":changed,"code_before":html.matches("<pre>").count(),"code_after":after.matches("<pre>").count(),"required":required.contains(&page)}));
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    assert!(
        required.is_subset(&fixed),
        "Required pages NOT repaired: {:?}",
        required.difference(&fixed).collect::<Vec<_>>()
    );
    println!(
        "Audited {} pages; changed {}; required {:?} all repaired",
        pages.len(),
        fixed.len(),
        required
    );
}
