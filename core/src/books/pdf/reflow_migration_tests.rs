use super::*;
use crate::vocab::annotate::rendered_text;
const FILE_V7: &str = "pdf-reflow-v7.json";
fn case() -> Case {
    let mut case = Case::with_bytes(super::super::super::tests::pdf_with_font(
        &[
            r"BT /F1 10 Tf 20 260 Td (fn main\(\) {) Tj 24 -16 Td (let value = 1;) Tj -24 -16 Td (}) Tj ET",
        ],
        false,
        "Courier",
    ));
    let mut snapshot: Snapshot = serde_json::from_slice(&case.original).unwrap();
    snapshot.extractor = super::super::super::TOC_EXTRACTOR.into();
    snapshot.pages=vec!["<pre><code>fn main() {</code></pre><pre><code>let value = 1;</code></pre><pre><code>}</code></pre>".into()];
    case.original = serde_json::to_vec(&snapshot).unwrap();
    std::fs::write(
        case.path.parent().unwrap().join(SNAPSHOT_FILE),
        &case.original,
    )
    .unwrap();
    case
}
fn point(case: &Case, needle: &str) -> (usize, usize) {
    let snapshot: Snapshot = serde_json::from_slice(&case.original).unwrap();
    split_blocks(&snapshot.pages[0])
        .iter()
        .enumerate()
        .find_map(|(b, h)| rendered_text(h).find(needle).map(|i| (b, i)))
        .unwrap()
}
#[test]
fn code_reflow_migrates_notes_bookmarks_progress_once_and_recreates_the_same_projection() {
    let case = case();
    let (b, i) = point(&case, "value");
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
    let text = rendered_text(&loaded.chapters[0].html);
    assert_eq!(split_blocks(&loaded.chapters[0].html).len(), 1);
    assert!(text.contains("\n    let value = 1;\n}"), "{text}");
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
    };
    check();
    let path = case.path.parent().unwrap().join(FILE_V7);
    let bytes = std::fs::read(&path).unwrap();
    case.db.load_book_chapters(&case.id, &case.path).unwrap();
    check();
    std::fs::remove_file(&path).unwrap();
    case.db.load_book_chapters(&case.id, &case.path).unwrap();
    check();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(
        std::fs::read(case.path.parent().unwrap().join(SNAPSHOT_FILE)).unwrap(),
        case.original
    );
    let mut p: Projection = serde_json::from_slice(&bytes).unwrap();
    p.snapshot.pages[0] = p.snapshot.pages[0].replace("value", "stole");
    std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
    assert!(case.db.load_book_chapters(&case.id, &case.path).is_err());
    check();
}
#[test]
fn code_reflow_commit_failure_rolls_back_then_retry_preserves_the_source_anchor() {
    let case = case();
    let (b, i) = point(&case, "value");
    case.db
        .add_highlight(&case.id, 0, (b, i, b, i + 5), "cyan")
        .unwrap();
    case.db
        .update_progress(&case.id, 1, 1, &format!("v2:0:{b}:24"))
        .unwrap();
    case.sql("CREATE TRIGGER fail_reflow BEFORE UPDATE OF metadata ON book_status WHEN NEW.metadata LIKE '%pdf_reflow_projection_v7%' BEGIN SELECT RAISE(ABORT,'test failure'); END;");
    assert!(case.db.load_book_chapters(&case.id, &case.path).is_err());
    let h = case.db.get_highlights_by_chapter(&case.id).unwrap();
    assert_eq!((h[&0][0].start_block, h[&0][0].start_char), (b, i));
    assert_eq!(
        case.db.get_status(&case.id).unwrap().unwrap().location,
        format!("v2:0:{b}:24")
    );
    case.sql("DROP TRIGGER fail_reflow;");
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
fn reordered_multibyte_ranges_use_utf8_bytes_and_reject_partial_codepoints() {
    let original = Snapshot {
        version: 1,
        fingerprint: "test".into(),
        extractor: super::super::super::TOC_EXTRACTOR.into(),
        pages: vec!["<p>é雪</p>".into()],
        toc: vec![],
    };
    let mut snapshot = original.clone();
    snapshot.extractor = super::super::super::REFLOW_EXTRACTOR.into();
    snapshot.pages = vec!["<p>雪é</p>".into()];
    let p = Projection {
        source_snapshot: "test".into(),
        snapshot,
        maps: vec![vec![BlockTarget {
            block: 0,
            prefix: 3,
            old_len: 5,
            offsets: Some(vec![3, 4, 0, 1, 2, 3]),
            layout_changed: true,
        }]],
        source_texts: Some(vec![Some(vec!["é雪".into()])]),
    };
    p.validate(&original).unwrap();
    assert_eq!(p.reordered_range(0, 0, 2).unwrap(), (3, 5));
    assert_eq!(p.reordered_range(0, 2, 5).unwrap(), (0, 3));
    assert!(p.reordered_range(0, 1, 2).is_err());
    assert!(p.reordered_range(0, 2, 4).is_err());
}

#[test]
fn unchanged_pages_are_exact_identities_not_unmarked_legacy_offset_maps() {
    let original = Snapshot {
        version: 1,
        fingerprint: "test".into(),
        extractor: super::super::super::TOC_EXTRACTOR.into(),
        pages: vec!["<p>same</p>".into()],
        toc: vec![],
    };
    let mut snapshot = original.clone();
    snapshot.extractor = super::super::super::REFLOW_EXTRACTOR.into();
    let mut p = Projection {
        source_snapshot: "test".into(),
        snapshot,
        maps: vec![vec![BlockTarget {
            block: 0,
            prefix: 0,
            old_len: 4,
            offsets: None,
            layout_changed: false,
        }]],
        source_texts: Some(vec![None]),
    };
    p.validate(&original).unwrap();
    p.maps[0][0].offsets = Some(vec![0, 1, 2, 3, 4]);
    assert!(p.validate(&original).is_err());
    p.maps[0][0].offsets = None;
    p.maps[0][0].layout_changed = true;
    assert!(p.validate(&original).is_err());
    p.maps[0][0].layout_changed = false;
    p.snapshot.pages[0] = "<p><strong>same</strong></p>".into();
    assert!(p.validate(&original).is_err());
}

#[test]
fn a_split_source_range_cannot_gain_unselected_code_between_its_fragments() {
    let original = Snapshot {
        version: 1,
        fingerprint: "test".into(),
        extractor: super::super::super::TOC_EXTRACTOR.into(),
        pages: vec!["<p>AB</p><p>X</p>".into()],
        toc: vec![],
    };
    let mut snapshot = original.clone();
    snapshot.extractor = super::super::super::REFLOW_EXTRACTOR.into();
    snapshot.pages = vec!["<p>A</p><pre><code>X</code></pre><p>B</p>".into()];
    let mut p = Projection {
        source_snapshot: "test".into(),
        snapshot,
        maps: vec![vec![
            BlockTarget {
                block: 0,
                prefix: 0,
                old_len: 2,
                offsets: Some(vec![0, 2_000_000, 2_000_001]),
                layout_changed: true,
            },
            BlockTarget {
                block: 1,
                prefix: 0,
                old_len: 1,
                offsets: Some(vec![1_000_000, 1_000_001]),
                layout_changed: true,
            },
        ]],
        source_texts: Some(vec![Some(vec!["AB".into(), "X".into()])]),
    };
    p.validate(&original).unwrap();
    assert!(p.reordered_range(0, 0, 2).is_err());
    assert_eq!(
        p.reordered_range(0, 0, PACK + 1).unwrap(),
        (0, 2 * PACK + 1)
    );
    p.maps[0][1].offsets.as_mut().unwrap()[0] = 0;
    assert!(p.validate(&original).is_err());
}
