use super::super::super::lists::tests::{FIRST, SECOND};
use super::*;
use crate::vocab::annotate::rendered_text;

fn case() -> Case {
    let mut case = Case::with_bytes(super::super::super::lists::tests::fixture());
    let mut snapshot: Snapshot = serde_json::from_slice(&case.original).unwrap();
    snapshot.extractor = super::super::super::STRUCTURE_EXTRACTOR.into();
    snapshot.pages = vec![format!(
        "<p>\u{f0a1} {FIRST} \u{f0a1} {SECOND}</p>\n<p>Ordinary prose.</p>"
    )];
    case.original = serde_json::to_vec(&snapshot).unwrap();
    std::fs::write(
        case.path.parent().unwrap().join(SNAPSHOT_FILE),
        &case.original,
    )
    .unwrap();
    case
}

#[test]
fn v5_keeps_prose_coordinates_notes_progress_and_original_snapshot_and_reopens_once() {
    let case = case();
    let text = rendered_text(
        &split_blocks(
            &serde_json::from_slice::<Snapshot>(&case.original)
                .unwrap()
                .pages[0],
        )[0],
    );
    let start = text.find("Second").unwrap();
    let id = case
        .db
        .add_highlight(&case.id, 0, (0, start, 0, start + 6), "cyan")
        .unwrap();
    case.db
        .add_bookmark(&case.id, &format!("v3:0:0:{start}"), "second")
        .unwrap();
    case.db
        .update_progress(&case.id, 1, 1, &format!("v3:0:0:{start}"))
        .unwrap();
    let assert_positions = || {
        let notes = case.db.get_highlights_by_chapter(&case.id).unwrap();
        let note = notes[&0].iter().find(|n| n.id == id).unwrap();
        assert_eq!(
            (
                note.start_block,
                note.start_char,
                note.end_block,
                note.end_char
            ),
            (0, start, 0, start + 6)
        );
        assert_eq!(
            case.db.get_bookmarks(&case.id).unwrap()[0].location,
            format!("v3:0:0:{start}")
        );
        assert_eq!(
            case.db.get_status(&case.id).unwrap().unwrap().location,
            format!("v3:0:0:{start}")
        );
    };
    let loaded = case.db.load_book_chapters(&case.id, &case.path).unwrap();
    assert_eq!(
        rendered_text(&split_blocks(&loaded.chapters[0].html)[0]),
        format!("• {FIRST}\n• {SECOND}")
    );
    assert_positions();
    let path = case.path.parent().unwrap().join(LIST_FILE);
    let bytes = std::fs::read(&path).unwrap();
    case.db.load_book_chapters(&case.id, &case.path).unwrap();
    assert_positions();
    std::fs::remove_file(&path).unwrap();
    case.db.load_book_chapters(&case.id, &case.path).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_positions();
    assert_eq!(
        std::fs::read(case.path.parent().unwrap().join(SNAPSHOT_FILE)).unwrap(),
        case.original
    );
    let mut projection: Projection = serde_json::from_slice(&bytes).unwrap();
    projection.snapshot.pages[0] = projection.snapshot.pages[0].replace("Second", "Stolen");
    std::fs::write(&path, serde_json::to_vec(&projection).unwrap()).unwrap();
    assert!(case.db.load_book_chapters(&case.id, &case.path).is_err());
    assert_positions();
}

#[test]
fn v5_failed_commit_preserves_pixels_and_notes_then_retry_uses_text_anchor() {
    let case = case();
    case.db
        .update_progress(&case.id, 1, 1, "v2:0:0:28")
        .unwrap();
    let id = case
        .db
        .add_highlight(&case.id, 0, (1, 0, 1, 8), "yellow")
        .unwrap();
    case.sql("CREATE TRIGGER fail_lists BEFORE UPDATE OF metadata ON book_status WHEN NEW.metadata LIKE '%pdf_list_projection_v5%' BEGIN SELECT RAISE(ABORT, 'test failure'); END;");
    assert!(case.db.load_book_chapters(&case.id, &case.path).is_err());
    assert_eq!(
        case.db.get_status(&case.id).unwrap().unwrap().location,
        "v2:0:0:28"
    );
    case.sql("DROP TRIGGER fail_lists;");
    case.db.load_book_chapters(&case.id, &case.path).unwrap();
    assert_eq!(
        case.db.get_status(&case.id).unwrap().unwrap().location,
        "v3:0:0:0"
    );
    let notes = case.db.get_highlights_by_chapter(&case.id).unwrap();
    let note = notes[&0].iter().find(|n| n.id == id).unwrap();
    assert_eq!(
        (
            note.start_block,
            note.start_char,
            note.end_block,
            note.end_char
        ),
        (1, 0, 1, 8)
    );
}
