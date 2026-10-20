use super::*;
use crate::books::epub_meta::code_block_text;

fn span(text: &str, x: f32, y: f32, mono: bool) -> TextSpan {
    let doc = super::super::open(super::super::tests::heading_fixture()).unwrap();
    let mut span = doc.extract_spans(0).unwrap().remove(0);
    span.text = text.into();
    span.font_name = if mono {
        "ABCDEF+FiraCode-Regular"
    } else {
        "Helvetica"
    }
    .into();
    // Explicitly simulate FiraCode PDFs with missing FixedPitch metadata.
    span.is_monospace = false;
    span.font_size = 10.0;
    span.bbox = Rect::new(x, y, text.chars().count() as f32 * 6.0, 10.0);
    span.char_widths.clear();
    span.char_x_offsets.clear();
    span.rotation_degrees = 0.0;
    span.page_rotation_applied = 0;
    span
}
fn run(html: &str, spans: &[TextSpan], links: &[(Rect, String)]) -> (String, Vec<BlockTarget>) {
    let geo = Geometry::new(spans, links);
    project(&prepare_blocks(html, &geo), &geo, vec![]).unwrap()
}
#[test]
fn link_rectangle_does_not_swallow_possessive_or_period_in_same_span() {
    let spans = [
        span("tarpaulin", 10.0, 100.0, true),
        span("’s README.", 64.0, 100.0, false),
    ];
    let link = [(
        Rect::new(82.0, 99.0, 36.0, 12.0),
        "https://example.test/readme?a=1&b=2".into(),
    )];
    let html = "<p>tarpaulin<a href=\"https://wrong.test\">’s README.</a></p>";
    let (fixed, map) = run(html, &spans, &link);
    assert_eq!(fixed, "<p><code>tarpaulin</code>’s <a href=\"https://example.test/readme?a=1&amp;b=2\">README</a>.</p>");
    assert_eq!(rendered_text(&fixed), rendered_text(html));
    assert_eq!(map[0].old_len, rendered_text(html).len());
}
#[test]
fn inline_code_is_not_promoted_to_a_listing_and_never_receives_ruby() {
    let spans = [
        span("Use ", 10.0, 100.0, false),
        span("cargo tarpaulin", 34.0, 100.0, true),
        span(" today.", 118.0, 100.0, false),
    ];
    let (fixed, _) = run("<p>Use cargo tarpaulin today.</p>", &spans, &[]);
    assert_eq!(fixed, "<p>Use <code>cargo tarpaulin</code> today.</p>");
    let gloss = std::collections::HashMap::from([("tarpaulin".into(), "柏油帆布".into())]);
    let annotated = crate::vocab::annotate::annotate_html(&fixed, &gloss, &Default::default());
    assert!(!annotated.contains("<ruby>"));
    assert_eq!(code_block_text(&fixed), None);
}
#[test]
fn code_listing_restores_lines_indents_and_maps_original_utf8_bytes() {
    let spans = [
        span("fn main() {", 10.0, 100.0, true),
        span("let reader = \"中文\";", 34.0, 86.0, true),
        span("}", 10.0, 72.0, true),
        span("Body text.", 10.0, 40.0, false),
    ];
    let html = "<p>fn main() {</p><p>let reader = &quot;中文&quot;;</p><p>}</p><p>Body text.</p>";
    let (fixed, maps) = run(html, &spans, &[]);
    let blocks = split_blocks(&fixed);
    assert_eq!(blocks.len(), 2, "{fixed}");
    assert_eq!(
        code_block_text(&blocks[0]).unwrap(),
        "fn main() {\n    let reader = \"中文\";\n}"
    );
    let before = split_blocks(html);
    let after = rendered_text(&blocks[0]);
    for (block, map) in before[..3].iter().zip(&maps) {
        let text = rendered_text(block);
        let offsets = map.offsets.as_ref().unwrap();
        for (i, byte) in text.bytes().enumerate() {
            assert_eq!(after.as_bytes()[offsets[i]], byte);
        }
        assert_eq!(map.block, 0);
    }
    assert_eq!(maps[3].block, 1);
    let gloss = std::collections::HashMap::from([("reader".into(), "读者".into())]);
    assert!(
        !crate::vocab::annotate::annotate_html(&fixed, &gloss, &Default::default())
            .contains("<ruby>")
    );
}
#[test]
fn different_columns_and_unmatched_prose_are_not_joined_as_code() {
    let spans = [
        span("cargo one", 10.0, 100.0, true),
        span("cargo two", 300.0, 100.0, true),
    ];
    let (fixed, _) = run(
        "<p>cargo one</p><p>cargo two</p><p>Unmatched prose.</p>",
        &spans,
        &[],
    );
    assert_eq!(split_blocks(&fixed).len(), 3);
    assert!(fixed.ends_with("<p>Unmatched prose.</p>"));
}
#[test]
fn linked_monospace_has_a_visible_plain_label_not_a_nested_custom_widget() {
    let spans = [
        span("cargo", 10.0, 100.0, true),
        span(" tool", 40.0, 100.0, false),
    ];
    let (fixed, _) = run(
        "<p>cargo tool</p>",
        &spans,
        &[(
            Rect::new(10.0, 99.0, 30.0, 12.0),
            "https://example.test".into(),
        )],
    );
    assert_eq!(
        fixed,
        "<p><a href=\"https://example.test\">cargo</a> tool</p>"
    );
    let gloss = std::collections::HashMap::from([("cargo".into(), "货物".into())]);
    assert!(
        !crate::vocab::annotate::annotate_html(&fixed, &gloss, &Default::default())
            .contains("<ruby>")
    );
}

#[test]
fn tag_scopes_entities_and_text_offsets_survive_inline_wrapping() {
    let spans = [
        span("Use ", 10.0, 100.0, false),
        span("a<b && c", 34.0, 100.0, true),
        span(" safely.", 82.0, 100.0, false),
    ];
    let html = "<p><em>Use a&lt;b &amp;&amp; c</em> safely.</p>";
    let (fixed, _) = run(html, &spans, &[]);
    assert_eq!(rendered_text(&fixed), rendered_text(html));
    assert!(
        fixed.contains("<em>Use <code>a&lt;b &amp;&amp; c</code></em>"),
        "{fixed}"
    );
}

pub(crate) fn image_fixture() -> Vec<u8> {
    let content = "BT /F1 12 Tf 20 270 Td (Readable text.) Tj 0 -150 Td (Figure 3-20. A diagram.) Tj ET q 64 0 0 64 20 140 cm /Im1 Do Q";
    let pixels = "2060A0".repeat(16 * 16) + ">";
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Resources << /Font << /F1 5 0 R >> /XObject << /Im1 6 0 R >> >> /Contents 4 0 R >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        format!("<< /Type /XObject /Subtype /Image /Width 16 /Height 16 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /ASCIIHexDecode /Length {} >>\nstream\n{pixels}\nendstream", pixels.len()),
    ];
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0];
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{obj}\nendobj\n", i + 1).as_bytes());
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
fn embedded_picture_precedes_caption_and_missing_assets_regenerate_without_text_changes() {
    let root = std::env::temp_dir().join(format!("readwise-pdf-images-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("book.pdf");
    std::fs::write(&path, image_fixture()).unwrap();
    let loaded = super::super::load_imported_pages(&path).unwrap();
    let html = &loaded.chapters[0].html;
    assert!(html.find("<img ").unwrap() < html.find("Figure 3-20").unwrap());
    assert!(html.contains(root.to_str().unwrap()));
    let snapshot = std::fs::read(root.join(super::super::SNAPSHOT_FILE)).unwrap();
    assert!(!String::from_utf8_lossy(&snapshot).contains(root.to_str().unwrap()));
    let image = root.join("pdf-assets-v3/page-0-0.png");
    let original = std::fs::read(&image).unwrap();
    assert_eq!(&original[..8], b"\x89PNG\r\n\x1a\n");
    std::fs::remove_file(&image).unwrap();
    let again = super::super::load_imported_pages(&path).unwrap();
    assert_eq!(again.chapters[0].html, *html);
    assert_eq!(std::fs::read(&image).unwrap(), original);
    assert_eq!(
        std::fs::read(root.join(super::super::SNAPSHOT_FILE)).unwrap(),
        snapshot
    );
    std::fs::remove_dir_all(&root).unwrap();
}
