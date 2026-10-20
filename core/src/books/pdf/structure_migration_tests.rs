use super::*;
const A: &str = "Let us explore this by integrating a Memory module.";
const B: &str = "The module is straightforward and keeps the conversation history in dictionaries.";
const C: &str = "This is shown in Figure 4-5:";
const D: &str = "Another ordinary paragraph remains separate after the real paragraph gap.";
fn case() -> Case {
    let content = format!(
        "BT /F1 10 Tf 20 260 Td ({A}) Tj 0 -12 Td ({B}) Tj 0 -12 Td ({C}) Tj 0 -30 Td ({D}) Tj ET"
    );
    let mut case = Case::with_bytes(super::super::super::tests::pdf_with_font(
        &[&content],
        false,
        "Helvetica",
    ));
    let mut snapshot: Snapshot = serde_json::from_slice(&case.original).unwrap();
    snapshot.extractor = super::super::super::CONTENT_EXTRACTOR.into();
    snapshot.pages = vec![format!("<h3>{A}</h3><p>{B}</p><h3>{C}</h3><p>{D}</p>")];
    case.original = serde_json::to_vec(&snapshot).unwrap();
    std::fs::write(
        case.path.parent().unwrap().join(SNAPSHOT_FILE),
        &case.original,
    )
    .unwrap();
    case
}
#[test]
fn v4_rebases_v3_annotations_and_progress_once_and_recovers_same_projection() {
    let case = case();
    let highlight = case
        .db
        .add_highlight(&case.id, 0, (1, 0, 1, 10), "cyan")
        .unwrap();
    case.db
        .add_bookmark(&case.id, "v3:0:2:3", "figure")
        .unwrap();
    case.db
        .update_progress(&case.id, 1, 1, "v2:0:0:12")
        .unwrap();
    let loaded = case.db.load_book_chapters(&case.id, &case.path).unwrap();
    let blocks = split_blocks(&loaded.chapters[0].html);
    assert_eq!(blocks.len(), 2);
    assert_eq!(
        crate::vocab::annotate::rendered_text(&blocks[0]),
        format!("{A} {B} {C}")
    );
    let assert_positions = || {
        let notes = case.db.get_highlights_by_chapter(&case.id).unwrap();
        let h = notes[&0].iter().find(|h| h.id == highlight).unwrap();
        assert_eq!(
            (h.start_block, h.start_char, h.end_block, h.end_char),
            (0, A.len() + 1, 0, A.len() + 11)
        );
        assert_eq!(
            case.db.get_bookmarks(&case.id).unwrap()[0].location,
            format!("v3:0:0:{}", A.len() + B.len() + 5)
        );
        assert_eq!(
            case.db.get_status(&case.id).unwrap().unwrap().location,
            "v3:0:0:0"
        );
    };
    assert_positions();
    let cache = case.path.parent().unwrap().join(STRUCTURE_FILE);
    let data = std::fs::read(&cache).unwrap();
    case.db.load_book_chapters(&case.id, &case.path).unwrap();
    assert_positions();
    std::fs::remove_file(&cache).unwrap();
    case.db.load_book_chapters(&case.id, &case.path).unwrap();
    assert_positions();
    assert_eq!(std::fs::read(&cache).unwrap(), data);
    assert_eq!(
        std::fs::read(case.path.parent().unwrap().join(SNAPSHOT_FILE)).unwrap(),
        case.original
    );
}
#[test]
fn v4_failed_commit_rolls_back_and_style_tampering_is_rejected_after_commit() {
    let case = case();
    case.db
        .add_highlight(&case.id, 0, (1, 0, 1, 10), "cyan")
        .unwrap();
    case.db.update_progress(&case.id, 1, 1, "v3:0:1:4").unwrap();
    case.sql("CREATE TRIGGER fail_structure BEFORE UPDATE OF metadata ON book_status WHEN NEW.metadata LIKE '%pdf_structure_projection_v4%' BEGIN SELECT RAISE(ABORT, 'test failure'); END;");
    assert!(case.db.load_book_chapters(&case.id, &case.path).is_err());
    assert_eq!(
        case.db.get_highlights_by_chapter(&case.id).unwrap()[&0][0].start_block,
        1
    );
    assert_eq!(
        case.db.get_status(&case.id).unwrap().unwrap().location,
        "v3:0:1:4"
    );
    case.sql("DROP TRIGGER fail_structure;");
    case.db.load_book_chapters(&case.id, &case.path).unwrap();
    let cache = case.path.parent().unwrap().join(STRUCTURE_FILE);
    let mut p: Projection = serde_json::from_slice(&std::fs::read(&cache).unwrap()).unwrap();
    p.snapshot.pages[0] = p.snapshot.pages[0]
        .replace("<p>", "<p><strong>")
        .replace("</p>", "</strong></p>");
    std::fs::write(cache, serde_json::to_vec(&p).unwrap()).unwrap();
    assert!(case.db.load_book_chapters(&case.id, &case.path).is_err());
    assert_eq!(
        case.db.get_highlights_by_chapter(&case.id).unwrap()[&0][0].start_char,
        A.len() + 1
    );
}
#[test]
fn old_projection_map_serialization_stays_identical_without_v4_fields() {
    let bytes = br#"{"block":2,"prefix":3,"old_len":9}"#;
    let target: BlockTarget = serde_json::from_slice(bytes).unwrap();
    assert!(!target.layout_changed);
    assert_eq!(serde_json::to_vec(&target).unwrap(), bytes);
}
#[test]
fn style_only_change_uses_text_anchor_instead_of_reusing_old_heading_pixel_scroll() {
    let mut case = case();
    let mut snapshot: Snapshot = serde_json::from_slice(&case.original).unwrap();
    snapshot.pages = vec![format!("<h3>{A}</h3><p>{B}</p><p>{C}</p><p>{D}</p>")];
    // Place the next paragraph far enough away that demotion does not join it.
    let source = format!(
        "BT /F1 10 Tf 20 260 Td ({A}) Tj 0 -30 Td ({B}) Tj 0 -30 Td ({C}) Tj 0 -30 Td ({D}) Tj ET"
    );
    let bytes = super::super::super::tests::pdf_with_font(&[&source], false, "Helvetica");
    std::fs::write(&case.path, &bytes).unwrap();
    snapshot.fingerprint = fingerprint(&bytes);
    case.original = serde_json::to_vec(&snapshot).unwrap();
    std::fs::write(
        case.path.parent().unwrap().join(SNAPSHOT_FILE),
        &case.original,
    )
    .unwrap();
    case.db
        .update_progress(&case.id, 1, 1, "v2:0:0:35")
        .unwrap();
    let loaded = case.db.load_book_chapters(&case.id, &case.path).unwrap();
    assert_eq!(split_blocks(&loaded.chapters[0].html).len(), 4);
    assert_eq!(
        case.db.get_status(&case.id).unwrap().unwrap().location,
        "v3:0:0:0"
    );
}
