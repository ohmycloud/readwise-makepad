use super::*;
use crate::vocab::annotate::rendered_text;
const FILE_V8: &str = "pdf-reflow-v8.json";
fn case(v6: bool) -> Case {
    let mut case = Case::with_bytes(super::super::super::tests::pdf_with_font(
        &[
            r"BT /F1 10 Tf 20 260 Td (fn main\(\) {) Tj 24 -16 Td (let value = 1;) Tj -24 -16 Td (}) Tj ET",
        ],
        false,
        "Courier",
    ));
    let mut snapshot: Snapshot = serde_json::from_slice(&case.original).unwrap();
    snapshot.extractor = if v6 {
        super::super::super::TOC_EXTRACTOR
    } else {
        super::super::super::REFLOW_EXTRACTOR
    }
    .into();
    snapshot.pages=vec!["<ul></ul><pre><code>fn main() {</code></pre><pre><code>let value = 1;</code></pre><pre><code>}</code></pre>".into()];
    case.original = serde_json::to_vec(&snapshot).unwrap();
    std::fs::write(
        case.path.parent().unwrap().join(SNAPSHOT_FILE),
        &case.original,
    )
    .unwrap();
    case
}
fn point(case: &Case) -> (usize, usize) {
    let s: Snapshot = serde_json::from_slice(&case.original).unwrap();
    split_blocks(&s.pages[0])
        .iter()
        .enumerate()
        .find_map(|(b, h)| rendered_text(h).find("value").map(|i| (b, i)))
        .unwrap()
}
#[test]
fn v8_continues_from_native_v7_and_from_committed_v7_without_rewriting_old_files() {
    for v6 in [false, true] {
        let case = case(v6);
        let (b, i) = point(&case);
        let note = case
            .db
            .add_highlight(&case.id, 0, (b, i, b, i + 5), "cyan")
            .unwrap();
        case.db
            .add_bookmark(&case.id, &format!("v3:0:{b}:{i}"), "value")
            .unwrap();
        case.db
            .update_progress(&case.id, 1, 1, &format!("v3:0:{b}:{i}"))
            .unwrap();
        let loaded = case.db.load_book_chapters(&case.id, &case.path).unwrap();
        assert_eq!(split_blocks(&loaded.chapters[0].html).len(), 1);
        let text = rendered_text(&loaded.chapters[0].html);
        assert!(text.contains("\n    let value = 1;\n}"));
        let byte = text.find("value").unwrap();
        let check = || {
            let h = case.db.get_highlights_by_chapter(&case.id).unwrap();
            let h = h[&0].iter().find(|h| h.id == note).unwrap();
            assert_eq!(
                (h.start_block, h.start_char, h.end_block, h.end_char),
                (0, byte, 0, byte + 5)
            );
            assert_eq!(
                case.db.get_bookmarks(&case.id).unwrap()[0].location,
                format!("v3:0:0:{byte}")
            );
            assert_eq!(
                case.db.get_status(&case.id).unwrap().unwrap().location,
                format!("v3:0:0:{byte}")
            );
            assert_eq!(
                std::fs::read(case.path.parent().unwrap().join(SNAPSHOT_FILE)).unwrap(),
                case.original
            );
        };
        check();
        let path = case.path.parent().unwrap().join(FILE_V8);
        let bytes = std::fs::read(&path).unwrap();
        let old = case.path.parent().unwrap().join("pdf-reflow-v7.json");
        let old_bytes = std::fs::read(&old).ok();
        assert_eq!(old_bytes.is_some(), v6);
        case.db.load_book_chapters(&case.id, &case.path).unwrap();
        check();
        std::fs::remove_file(&path).unwrap();
        case.db.load_book_chapters(&case.id, &case.path).unwrap();
        check();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read(&old).ok(), old_bytes);
        let mut projection: Projection = serde_json::from_slice(&bytes).unwrap();
        projection.snapshot.pages[0] = projection.snapshot.pages[0].replace("value", "stole");
        std::fs::write(&path, serde_json::to_vec(&projection).unwrap()).unwrap();
        assert!(case.db.load_book_chapters(&case.id, &case.path).is_err());
        check();
    }
}
#[test]
fn failed_v8_transaction_keeps_v7_notes_pixels_and_bookmarks_then_retries_once() {
    let case = case(false);
    let (b, i) = point(&case);
    case.db
        .add_highlight(&case.id, 0, (b, i, b, i + 5), "cyan")
        .unwrap();
    case.db
        .add_bookmark(&case.id, &format!("v3:0:{b}:{i}"), "value")
        .unwrap();
    case.db
        .update_progress(&case.id, 1, 1, &format!("v2:0:{b}:24"))
        .unwrap();
    case.sql("CREATE TRIGGER fail_v8 BEFORE UPDATE OF metadata ON book_status WHEN NEW.metadata LIKE '%pdf_reflow_projection_v8%' BEGIN SELECT RAISE(ABORT,'test failure'); END;");
    assert!(case.db.load_book_chapters(&case.id, &case.path).is_err());
    let h = case.db.get_highlights_by_chapter(&case.id).unwrap();
    assert_eq!((h[&0][0].start_block, h[&0][0].start_char), (b, i));
    assert_eq!(
        case.db.get_bookmarks(&case.id).unwrap()[0].location,
        format!("v3:0:{b}:{i}")
    );
    assert_eq!(
        case.db.get_status(&case.id).unwrap().unwrap().location,
        format!("v2:0:{b}:24")
    );
    case.sql("DROP TRIGGER fail_v8;");
    case.db.load_book_chapters(&case.id, &case.path).unwrap();
    assert!(case
        .db
        .get_status(&case.id)
        .unwrap()
        .unwrap()
        .location
        .starts_with("v3:0:0:"));
}
#[test]
fn v8_uses_packed_multiblock_utf8_points_not_legacy_single_block_offsets() {
    let original = Snapshot {
        version: 1,
        fingerprint: "test".into(),
        extractor: super::super::super::REFLOW_EXTRACTOR.into(),
        pages: vec!["<p>é雪</p>".into()],
        toc: vec![],
    };
    let mut snapshot = original.clone();
    snapshot.extractor = super::super::super::EXTENDED_REFLOW_EXTRACTOR.into();
    snapshot.pages = vec!["<p>雪</p><p>é</p>".into()];
    let p = Projection {
        source_snapshot: "test".into(),
        snapshot,
        maps: vec![vec![BlockTarget {
            block: 1,
            prefix: 0,
            old_len: 5,
            offsets: Some(vec![1_000_000, 1_000_001, 0, 1, 2, 3]),
            layout_changed: true,
        }]],
        source_texts: Some(vec![Some(vec!["é雪".into()])]),
    };
    p.validate(&original).unwrap();
    assert_eq!(p.point(0, 0, 0).unwrap(), (1, 0));
    assert_eq!(p.reordered_range(0, 0, 2).unwrap(), (PACK, PACK + 2));
    assert_eq!(p.reordered_range(0, 2, 5).unwrap(), (0, 3));
    assert!(p.reordered_range(0, 1, 2).is_err());
}
