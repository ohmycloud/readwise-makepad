use super::*;
use crate::vocab::annotate::rendered_text;

fn case() -> Case {
    let mut case = Case::with_bytes(super::super::super::toc::tests::fixture());
    let mut snapshot: Snapshot = serde_json::from_slice(&case.original).unwrap();
    snapshot.extractor = super::super::super::LIST_EXTRACTOR.into();
    case.original = serde_json::to_vec(&snapshot).unwrap();
    std::fs::write(
        case.path.parent().unwrap().join(SNAPSHOT_FILE),
        &case.original,
    )
    .unwrap();
    case
}

fn source_point(case: &Case, needle: &str) -> (usize, usize) {
    let source: Snapshot = serde_json::from_slice(&case.original).unwrap();
    split_blocks(&source.pages[0])
        .iter()
        .enumerate()
        .find_map(|(b, html)| rendered_text(html).find(needle).map(|i| (b, i)))
        .unwrap()
}

#[test]
fn toc_migration_keeps_highlights_bookmarks_and_content_then_reopens_and_recovers_once() {
    let case = case();
    let (block, byte) = source_point(&case, "Alpha");
    let id = case
        .db
        .add_highlight(&case.id, 0, (block, byte, block, byte + 5), "cyan")
        .unwrap();
    let (heading, hbyte) = source_point(&case, "Getting");
    case.db
        .add_bookmark(&case.id, &format!("v3:0:{heading}:{hbyte}"), "chapter")
        .unwrap();
    case.db
        .update_progress(&case.id, 1, 1, &format!("v3:0:{block}:{byte}"))
        .unwrap();
    let loaded = case.db.load_book_chapters(&case.id, &case.path).unwrap();
    let text = rendered_text(&loaded.chapters[0].html);
    let alpha = text.find("Alpha").unwrap();
    let getting = text.find("Getting").unwrap();
    assert!(
        text.contains("1.1 Alpha topic 4\n1.2 Beta topic 6"),
        "{text}"
    );
    let check = || {
        let notes = case.db.get_highlights_by_chapter(&case.id).unwrap();
        let note = notes[&0].iter().find(|n| n.id == id).unwrap();
        assert_eq!(
            (
                note.start_block,
                note.start_char,
                note.end_block,
                note.end_char
            ),
            (0, alpha, 0, alpha + 5)
        );
        assert_eq!(
            case.db.get_bookmarks(&case.id).unwrap()[0].location,
            format!("v3:0:0:{getting}")
        );
        assert_eq!(
            case.db.get_status(&case.id).unwrap().unwrap().location,
            format!("v3:0:0:{alpha}")
        );
    };
    check();
    let path = case.path.parent().unwrap().join(TOC_FILE);
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
    p.snapshot.pages[0] = p.snapshot.pages[0].replace("Alpha", "Stole");
    std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
    assert!(case.db.load_book_chapters(&case.id, &case.path).is_err());
    check();
}

#[test]
fn toc_failed_commit_rolls_back_all_coordinates_and_retry_anchors_the_same_text() {
    let case = case();
    let (block, byte) = source_point(&case, "Alpha");
    case.db
        .add_highlight(&case.id, 0, (block, byte, block, byte + 5), "cyan")
        .unwrap();
    case.db
        .update_progress(&case.id, 1, 1, &format!("v2:0:{block}:24"))
        .unwrap();
    case.sql("CREATE TRIGGER fail_toc BEFORE UPDATE OF metadata ON book_status WHEN NEW.metadata LIKE '%pdf_toc_projection_v6%' BEGIN SELECT RAISE(ABORT, 'test failure'); END;");
    assert!(case.db.load_book_chapters(&case.id, &case.path).is_err());
    assert_eq!(
        case.db.get_status(&case.id).unwrap().unwrap().location,
        format!("v2:0:{block}:24")
    );
    let h = case.db.get_highlights_by_chapter(&case.id).unwrap();
    assert_eq!((h[&0][0].start_block, h[&0][0].start_char), (block, byte));
    case.sql("DROP TRIGGER fail_toc;");
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
fn reordered_ranges_never_include_unselected_text_between_moved_fragments() {
    let original = Snapshot {
        version: 1,
        fingerprint: "test".into(),
        extractor: super::super::super::LIST_EXTRACTOR.into(),
        pages: vec!["<p>AB</p><p>X</p>".into()],
        toc: vec![],
    };
    let mut snapshot = original.clone();
    snapshot.extractor = super::super::super::TOC_EXTRACTOR.into();
    snapshot.pages = vec!["<p>A X B</p>".into()];
    let projection = Projection {
        source_snapshot: "test".into(),
        snapshot,
        maps: vec![vec![
            BlockTarget {
                block: 0,
                prefix: 0,
                old_len: 2,
                offsets: Some(vec![0, 4, 5]),
                layout_changed: true,
            },
            BlockTarget {
                block: 0,
                prefix: 2,
                old_len: 1,
                offsets: Some(vec![2, 3]),
                layout_changed: true,
            },
        ]],
        source_texts: Some(vec![Some(vec!["AB".into(), "X".into()])]),
    };
    projection.validate(&original).unwrap();
    assert!(projection.reordered_range(0, 0, 2).is_err());
    assert_eq!(projection.reordered_range(0, 0, PACK + 1).unwrap(), (0, 5));
}
