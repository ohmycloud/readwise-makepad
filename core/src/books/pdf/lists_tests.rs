use super::*;
use pdf_oxide::geometry::Rect;

fn span(text: &str, x: f32, y: f32, font: &str) -> TextSpan {
    let doc = super::super::open(super::super::tests::heading_fixture()).unwrap();
    let mut span = doc.extract_spans(0).unwrap().remove(0);
    span.text = text.into();
    span.font_name = font.into();
    span.font_size = 10.0;
    span.is_monospace = false;
    span.bbox = Rect::new(x, y, text.chars().count() as f32 * 5.0, 10.0);
    span.char_widths.clear();
    span.char_x_offsets.clear();
    span.rotation_degrees = 0.0;
    span.page_rotation_applied = 0;
    span
}

#[test]
fn confirmed_list_restores_line_breaks_keeps_inline_styles_entities_and_utf8_offsets() {
    let spans = [
        span("\u{f0a1}", 20.0, 200.0, "ABCDEF+Wingdings2"),
        span("First & 中文.", 32.0, 200.0, "Helvetica"),
        span("\u{f0a1}", 20.0, 180.0, "Wingdings2"),
        span("Second item.", 32.0, 180.0, "Helvetica"),
    ];
    let html = "<p>\u{f0a1} <em>First &amp; 中文.</em> \u{f0a1} <a href=\"https://example.test/?x=1&amp;y=2\">Second item.</a></p>";
    let (fixed, maps) = project(html, &spans);
    assert_eq!(fixed, "<p>• <em>First &amp; 中文.</em><br/>• <a href=\"https://example.test/?x=1&amp;y=2\">Second item.</a></p>");
    let old = rendered_text(html);
    let new = rendered_text(&fixed);
    assert_eq!(new, "• First & 中文.\n• Second item.");
    assert_eq!(old.find("Second"), new.find("Second"));
    assert_eq!(old.len(), new.len());
    assert!(valid_text_change(&old, &new));
    assert!(maps[0].layout_changed);
    assert!(maps[0].offsets.is_none());
    assert_eq!(project(&fixed, &spans).0, fixed);
}

#[test]
fn wrong_fonts_inline_symbols_unknown_pua_and_code_are_not_rewritten() {
    for (html, spans) in [
        (
            "<p>\u{f0a1} First.</p>",
            vec![
                span("\u{f0a1}", 20.0, 200.0, "Helvetica"),
                span("First.", 32.0, 200.0, "Helvetica"),
            ],
        ),
        (
            "<p>Example \u{f0a1} First.</p>",
            vec![
                span("Example", 1.0, 200.0, "Helvetica"),
                span("\u{f0a1}", 45.0, 200.0, "Wingdings2"),
                span("First.", 57.0, 200.0, "Helvetica"),
            ],
        ),
        (
            "<p>\u{f012} First.</p>",
            vec![
                span("\u{f012}", 20.0, 200.0, "Wingdings2"),
                span("First.", 32.0, 200.0, "Helvetica"),
            ],
        ),
        (
            "<pre><code>\u{f0a1} First.</code></pre>",
            vec![
                span("\u{f0a1}", 20.0, 200.0, "Wingdings2"),
                span("First.", 32.0, 200.0, "Helvetica"),
            ],
        ),
    ] {
        let (fixed, maps) = project(html, &spans);
        assert_eq!(fixed, html);
        assert!(!maps[0].layout_changed);
    }
}

#[test]
fn projection_validation_does_not_allow_prose_changes_or_invalid_utf8_boundaries() {
    assert!(valid_text_change(
        "\u{f0a1} item. \u{f0a1} next.",
        "• item.\n• next."
    ));
    for (old, new) in [
        ("item", "lost"),
        ("a b", "a\nb"),
        ("中文", "éabcd"),
        ("\u{f012}", "•"),
    ] {
        assert!(!valid_text_change(old, new));
    }
}

pub(crate) const FIRST: &str = "First item describes which resources belong to the application.";
pub(crate) const SECOND: &str = "Second item starts those resources together for development.";

pub(crate) fn fixture() -> Vec<u8> {
    let content = format!("BT /F1 10 Tf 20 250 Td <01> Tj ET BT /F2 10 Tf 32 250 Td ({FIRST}) Tj ET BT /F1 10 Tf 20 230 Td <01> Tj ET BT /F2 10 Tf 32 230 Td ({SECOND}) Tj ET BT /F2 10 Tf 20 190 Td (Ordinary prose contains a separate full paragraph below the list.) Tj ET");
    let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /Test def /CMapType 2 def 1 begincodespacerange <00> <FF> endcodespacerange 1 beginbfchar <01> <F0A1> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 300] /Resources << /Font << /F1 5 0 R /F2 7 0 R >> >> /Contents 4 0 R >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream",content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Wingdings2 /ToUnicode 6 0 R >>".into(),
        format!("<< /Length {} >>\nstream\n{cmap}\nendstream",cmap.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ];
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let start = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in &offsets[1..] {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    bytes
}

#[test]
fn new_imports_convert_symbol_font_bullets() {
    let snapshot = super::super::extract(fixture(), "test".into()).unwrap();
    assert_eq!(snapshot.extractor, super::super::TOC_EXTRACTOR);
    assert!(
        !snapshot.pages[0].contains(PRIVATE_BULLET),
        "{}",
        snapshot.pages[0]
    );
    assert_eq!(snapshot.pages[0].matches('•').count(), 2);
}
