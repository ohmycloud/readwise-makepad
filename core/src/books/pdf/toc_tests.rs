use super::*;
use pdf_oxide::{geometry::Rect, layout::FontWeight};

fn span(text: &str, x: f32, y: f32, size: f32) -> TextSpan {
    let doc = super::super::open(super::super::tests::heading_fixture()).unwrap();
    let mut span = doc.extract_spans(0).unwrap().remove(0);
    span.text = text.into();
    span.font_name = "Helvetica".into();
    span.font_size = size;
    span.bbox = Rect::new(x, y, text.chars().count() as f32 * 5.0, size);
    span.char_widths.clear();
    span.char_x_offsets.clear();
    span.is_monospace = false;
    span.font_weight = FontWeight::Normal;
    span.rotation_degrees = 0.0;
    span.page_rotation_applied = 0;
    span
}

#[test]
fn contents_rows_and_mixed_size_title_order_come_from_geometry_not_bad_html_order() {
    let mut spans = vec![
        span("Contents", 20.0, 390.0, 20.0),
        span("th distributed systems", 140.0, 350.0, 13.0),
        span("3", 260.0, 350.0, 13.0),
        span("1", 20.0, 342.8, 27.0),
        span("Getting started wi", 50.0, 350.0, 13.0),
        span("1.1 Alpha topic 4", 40.0, 320.0, 11.0),
        span("1.2 Beta topic 6", 40.0, 300.0, 11.0),
        span("1.3 Gamma topic 8", 40.0, 280.0, 11.0),
        span("1.4 Delta topic 9", 40.0, 260.0, 11.0),
    ];
    let chars: Vec<_> = spans.iter().flat_map(TextSpan::to_chars).collect();
    spans[4].bbox.y = 342.8; // The converter merged this run with the large numeral.
    let html="<h1>Contents</h1><h4>th distributed systems</h4><p>31</p><h4>Getting started wi</h4><p>1.1 Alpha topic 4 1.2 Beta topic 6 1.3 Gamma topic 8 1.4 Delta topic 9</p>";
    let (fixed, maps) = project(html, &spans, &chars).unwrap();
    let text = rendered_text(&fixed);
    assert!(
        text.contains("1 Getting started with distributed systems 3"),
        "{text}"
    );
    assert!(
        text.contains("1.1 Alpha topic 4\n1.2 Beta topic 6\n1.3 Gamma topic 8\n1.4 Delta topic 9"),
        "{text}"
    );
    assert_eq!(maps.len(), 5);
    assert_eq!(maps[1].block, 0);
    assert!(
        maps[1].prefix > maps[3].prefix,
        "reordered fragments need non-monotonic source maps"
    );
}

#[test]
fn prose_and_ambiguous_or_incomplete_pages_keep_their_existing_reflow() {
    let spans = [span(
        "The contents of this ordinary paragraph are reflowable prose.",
        20.0,
        300.0,
        12.0,
    )];
    assert!(project(
        "<p>The contents of this ordinary paragraph are reflowable prose.</p>",
        &spans,
        &[]
    )
    .is_none());
    let spans = [
        span("Contents", 20.0, 300.0, 20.0),
        span("One entry 1", 20.0, 250.0, 12.0),
    ];
    assert!(project("<h1>Contents</h1><p>One entry 1</p>", &spans, &[]).is_none());
    let spans = [span("Contents", 20.0, 300.0, 20.0)];
    assert!(project("<h1>Contents</h1><p>Missing source text</p>", &spans, &[]).is_none());
}

#[test]
fn two_column_contents_read_down_each_column_and_short_continuations_still_break_rows() {
    let mut spans = vec![span("Contents", 20.0, 390.0, 20.0)];
    let mut html = "<h1>Contents</h1><p>".to_string();
    for (x, prefix) in [(20.0, "1"), (300.0, "2")] {
        for i in 1..=3 {
            let text = format!("{prefix}.{i} Topic {i}");
            spans.push(span(&text, x, 360.0 - i as f32 * 20.0, 11.0));
            html.push_str(&text);
            html.push(' ');
        }
    }
    html.push_str("</p>");
    let chars: Vec<_> = spans.iter().flat_map(TextSpan::to_chars).collect();
    let (fixed, _) = project(&html, &spans, &chars).unwrap();
    let text = rendered_text(&fixed);
    assert!(
        text.find("1.3").unwrap() < text.find("2.1").unwrap(),
        "{text}"
    );
    let spans = [
        span("Contents", 20.0, 300.0, 15.0),
        span("11.11 Epilogue 581", 20.0, 270.0, 11.0),
    ];
    let (fixed, _) = project("<p>Contents 11.11 Epilogue 581</p>", &spans, &[]).unwrap();
    assert_eq!(rendered_text(&fixed), "Contents\n11.11 Epilogue 581");
}

#[test]
fn relocated_page_number_column_reattaches_repeated_numbers_in_order_and_keeps_code() {
    let mut spans = vec![span("Contents", 20.0, 390.0, 20.0)];
    let mut html = "<h1>Contents</h1><p>".to_string();
    for (i, number) in [8, 8, 9, 10].iter().enumerate() {
        let text = format!("1.{} Topic", i + 1);
        let y = 360.0 - i as f32 * 20.0;
        spans.push(span(&text, 20.0, y, 11.0));
        spans.push(span(&number.to_string(), 240.0, y, 11.0));
        html.push_str(&text);
        html.push(' ');
    }
    spans.push(span("....", 180.0, 320.0, 11.0));
    html.push_str("</p><p>8 8 .... 9 10</p>");
    spans[1].font_name = "Courier".into();
    let chars: Vec<_> = spans.iter().flat_map(TextSpan::to_chars).collect();
    let (fixed, _) = project(&html, &spans, &chars).unwrap();
    assert!(fixed.contains("<code>1.1 Topic</code>"), "{fixed}");
    assert!(
        rendered_text(&fixed).contains("1.1 Topic 8\n1.2 Topic 8\n1.3 Topic .... 9\n1.4 Topic 10")
    );
}

pub(crate) fn fixture() -> Vec<u8> {
    super::super::tests::pdf_with_font(&[
        "BT /F1 20 Tf 20 280 Td (Contents) Tj ET BT /F1 27 Tf 20 232.8 Td (1) Tj ET BT /F2 13 Tf 50 240 Td (Getting started with systems) Tj 0 0 Td ET BT /F2 13 Tf 240 240 Td (3) Tj ET BT /F2 11 Tf 40 210 Td (1.1 Alpha topic 4) Tj 0 -25 Td (1.2 Beta topic 6) Tj 0 -25 Td (1.3 Gamma topic 8) Tj 0 -25 Td (1.4 Delta topic 9) Tj ET"
    ],false,"Helvetica")
}

#[test]
fn new_imports_preserve_contents_rows_before_the_prose_pipeline() {
    let snapshot = super::super::extract(fixture(), "test".into()).unwrap();
    assert_eq!(snapshot.extractor, super::super::EXTENDED_REFLOW_EXTRACTOR);
    let text = rendered_text(&snapshot.pages[0]);
    assert!(
        text.contains("1.1 Alpha topic 4\n1.2 Beta topic 6"),
        "{text}"
    );
}

/// Opt-in regression for copyrighted/local PDFs that cannot be checked in.
#[test]
#[ignore]
fn local_pdf_snapshot_contents_rows() {
    struct Logger;
    impl log::Log for Logger {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, r: &log::Record) {
            eprintln!("{}", r.args());
        }
        fn flush(&self) {}
    }
    static LOGGER: Logger = Logger;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Warn);
    let root =
        std::path::PathBuf::from(std::env::var("READWISE_TOC_QA_BOOK").expect("book directory"));
    let page: usize = std::env::var("READWISE_TOC_QA_PAGE")
        .unwrap()
        .parse()
        .unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("pdf-lists-v5.json")).unwrap()).unwrap();
    let html = value["snapshot"]["pages"][page].as_str().unwrap();
    let doc = super::super::open(super::super::read_pdf(&root.join("book.pdf")).unwrap()).unwrap();
    let spans = doc.extract_spans(page).unwrap();
    let chars = doc.extract_chars(page).unwrap();
    let (html, _) = project(html, &spans, &chars).expect("complete, unambiguous contents mapping");
    println!("{}", rendered_text(&html));
}
