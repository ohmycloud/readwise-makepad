use super::*;
use readwise_core::{books::epub_meta::split_blocks, vocab::annotate::rendered_text};

#[test]
fn session_reuses_document_across_pages_without_changing_projection_or_source() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<PdfSession>(); // Native workers share the same session.
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../web/tests/fixtures/vector-figure.pdf");
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("book.pdf");
    let original = std::fs::read(source).unwrap();
    std::fs::write(&path, &original).unwrap();
    let loaded = readwise_core::books::pdf::load_pages(&path).unwrap();
    let session = PdfSession::new(&path);
    assert!(
        session.document.get().is_none(),
        "opening is lazy/off-thread"
    );
    let mut before = Vec::new();
    for (i, chapter) in loaded.chapters.iter().enumerate() {
        let blocks = split_blocks(&chapter.html);
        let text: Vec<_> = blocks.iter().map(|s| rendered_text(s)).collect();
        let cold = MathPage::pdf(&path, i, &blocks, &text);
        let warm = session.page(i, &blocks, &text);
        let render = |p: &MathPage| {
            blocks
                .iter()
                .enumerate()
                .map(|(b, html)| p.annotate(b, html, str::to_owned))
                .collect::<Vec<_>>()
        };
        assert_eq!(render(&cold), render(&warm));
        for (html, source) in render(&warm).iter().zip(&text) {
            assert_eq!(rendered_text(html), *source);
        }
        let rendered = render(&warm);
        before.push((blocks, text, rendered));
    }
    assert_eq!(std::fs::read(&path).unwrap(), original);
    // Hide the source path: an uncached destination must still use the already
    // open document, rather than secretly reopening it on a page-cache miss.
    std::fs::rename(&path, temp.path().join("moved.pdf")).unwrap();
    // Ordinary page avoids the figure rasterizer's independent file access.
    let (blocks, text, expected) = &before[1];
    let page = session.page(1, blocks, text);
    let actual: Vec<_> = blocks
        .iter()
        .enumerate()
        .map(|(b, html)| page.annotate(b, html, str::to_owned))
        .collect();
    assert_eq!(actual, *expected);
    assert!(session.document.get().unwrap().is_some());
    // A fresh session must not inherit another book's stale parser.
    let fresh = PdfSession::new(&path);
    fresh.page(1, blocks, text);
    assert!(fresh.document.get().unwrap().is_none());

    // A non-empty formula projection makes the no-reopen check observable:
    // fallback plain text must not accidentally pass as a cache hit.
    let path = temp.path().join("formula.pdf");
    std::fs::write(
        &path,
        include_bytes!("../../web/tests/fixtures/math-scattered.pdf"),
    )
    .unwrap();
    let loaded = readwise_core::books::pdf::load_pages(&path).unwrap();
    let blocks = split_blocks(&loaded.chapters[0].html);
    let texts: Vec<_> = blocks.iter().map(|b| rendered_text(b)).collect();
    let session = PdfSession::new(&path);
    let before = session.page(0, &blocks, &texts);
    assert!(before.bytes > 0);
    std::fs::rename(&path, temp.path().join("moved-formula.pdf")).unwrap();
    let after = session.page(0, &blocks, &texts);
    assert_eq!(before.bytes, after.bytes);
    for (i, html) in blocks.iter().enumerate() {
        assert_eq!(
            before.annotate(i, html, str::to_owned),
            after.annotate(i, html, str::to_owned)
        );
    }
    assert_eq!(PdfSession::new(&path).page(0, &blocks, &texts).bytes, 0);
}

#[test]
fn non_pdf_and_rejected_inputs_never_open_a_document_and_failures_are_cached() {
    let temp = tempfile::tempdir().unwrap();
    let epub = PdfSession::new(temp.path().join("book.epub"));
    assert_eq!(epub.page(0, &[], &[]).bytes, 0);
    assert!(epub.document.get().is_none());
    let path = temp.path().join("missing.pdf");
    let session = PdfSession::new(&path);
    assert_eq!(session.page(0, &[], &["a".repeat(512 * 1024 + 1)]).bytes, 0);
    assert!(session.document.get().is_none());
    assert_eq!(session.page(0, &[], &[]).bytes, 0);
    assert!(session.document.get().unwrap().is_none());
    std::fs::write(
        &path,
        include_bytes!("../../web/tests/fixtures/equations.pdf"),
    )
    .unwrap();
    session.page(0, &[], &[]);
    assert!(
        session.document.get().unwrap().is_none(),
        "failure is not retried in a pagination loop"
    );
    let fresh = PdfSession::new(&path);
    fresh.page(0, &[], &[]);
    assert!(
        fresh.document.get().unwrap().is_some(),
        "reopening the book retries"
    );
}
