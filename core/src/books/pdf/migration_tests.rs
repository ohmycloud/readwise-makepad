use super::*;
use crate::{books::epub_meta::split_blocks, Db};
use std::path::PathBuf;

#[path = "structure_migration_tests.rs"]
mod structure;

struct Case {
    root: PathBuf,
    path: PathBuf,
    id: String,
    db: Db,
    original: Vec<u8>,
    last: usize,
}
impl Case {
    fn new() -> Self {
        Self::with_bytes(super::super::tests::heading_fixture())
    }
    fn with_bytes(bytes: Vec<u8>) -> Self {
        let root = std::env::temp_dir().join(format!(
            "readwise-heading-migration-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("source.pdf");
        std::fs::write(&source, &bytes).unwrap();
        let db = Db::open_in(root.join("data")).unwrap();
        let book = db.import_book(&source).unwrap();
        let path = root.join("data").join(&book.file_path);
        let doc = open(bytes.clone()).unwrap();
        let html = super::super::super::epub_meta::sanitize_for_reader(
            &doc.to_html(0, &Default::default()).unwrap(),
        );
        let last = split_blocks(&html).len() - 1;
        let snapshot = Snapshot {
            version: super::super::SNAPSHOT_VERSION,
            fingerprint: fingerprint(&bytes),
            extractor: LEGACY_EXTRACTOR.into(),
            pages: vec![html],
            toc: vec![],
        };
        let original = serde_json::to_vec(&snapshot).unwrap();
        std::fs::write(path.parent().unwrap().join(SNAPSHOT_FILE), &original).unwrap();
        Self {
            root,
            path,
            id: book.id,
            db,
            original,
            last,
        }
    }
    fn sql(&self, sql: &str) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let url = format!(
                "sqlite:{}",
                self.root.join("data/database/app.db").display()
            );
            let pool = SqlitePool::connect(&url).await.unwrap();
            sqlx::raw_sql(sql).execute(&pool).await.unwrap();
            pool.close().await;
        });
    }
}
impl Drop for Case {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn legacy_heading_migration_preserves_notes_bookmarks_progress_and_is_idempotent() {
    let case = Case::new();
    assert!(
        case.last > 2,
        "fixture must actually reproduce fragmented headings"
    );
    let number = case
        .db
        .add_highlight(&case.id, 0, (1, 0, 1, 1), "cyan")
        .unwrap();
    let body = case
        .db
        .add_highlight(&case.id, 0, (case.last, 2, case.last, 8), "yellow")
        .unwrap();
    case.db
        .add_bookmark(&case.id, "v3:0:4:2", "Started quote")
        .unwrap();
    case.db
        .update_progress(&case.id, 1, 1, &format!("v3:0:{}:5", case.last))
        .unwrap();
    case.db.set_book_want_to_read(&case.id, true).unwrap();
    let loaded = case.db.load_book_chapters(&case.id, &case.path).unwrap();
    let blocks = split_blocks(&loaded.chapters[0].html);
    assert_eq!(
        crate::vocab::annotate::rendered_text(&blocks[0]),
        "Chapter 1"
    );
    assert_eq!(blocks[1], "<h1>Getting Started</h1>");
    assert_eq!(blocks.len(), 3);
    let assert_coordinates = || {
        let highlights = case.db.get_highlights_by_chapter(&case.id).unwrap();
        let h = highlights[&0].iter().find(|h| h.id == number).unwrap();
        assert_eq!(
            (h.start_block, h.start_char, h.end_block, h.end_char),
            (0, 8, 0, 9)
        );
        let h = highlights[&0].iter().find(|h| h.id == body).unwrap();
        assert_eq!(
            (h.start_block, h.start_char, h.end_block, h.end_char),
            (2, 2, 2, 8)
        );
        assert_eq!(
            case.db.get_status(&case.id).unwrap().unwrap().location,
            "v3:0:2:5"
        );
        assert_eq!(
            case.db.get_bookmarks(&case.id).unwrap()[0].location,
            "v3:0:1:10"
        );
        assert!(case.db.get_shelf_items().unwrap()[0].want_to_read);
    };
    assert_coordinates();
    case.db.load_book_chapters(&case.id, &case.path).unwrap();
    assert_coordinates();
    // Recovery after sidecar loss regenerates EXACTLY the same projection;
    // its committed token prevents a second rebasing of coordinates.
    std::fs::remove_file(case.path.parent().unwrap().join(FILE)).unwrap();
    let reopened = Db::open_in(case.root.join("data")).unwrap();
    reopened.load_book_chapters(&case.id, &case.path).unwrap();
    assert_coordinates();
    let original_path = case.path.parent().unwrap().join(SNAPSHOT_FILE);
    assert_eq!(std::fs::read(&original_path).unwrap(), case.original);
    std::fs::remove_file(&original_path).unwrap();
    assert!(case
        .db
        .load_book_chapters(&case.id, &case.path)
        .err()
        .unwrap()
        .contains("快照缺失"));
    assert!(
        !original_path.exists(),
        "never regenerate a committed coordinate source"
    );
    assert_coordinates();
}

#[test]
fn pixel_locations_keep_body_scroll_but_joined_headings_use_text_anchors() {
    let case = Case::new();
    case.db
        .update_progress(&case.id, 1, 1, &format!("v2:0:{}:32.5", case.last))
        .unwrap();
    case.db
        .add_bookmark(&case.id, "v2:0:4:12", "legacy heading")
        .unwrap();
    case.db.load_book_chapters(&case.id, &case.path).unwrap();
    assert_eq!(
        case.db.get_status(&case.id).unwrap().unwrap().location,
        "v2:0:2:32.5"
    );
    assert_eq!(
        case.db.get_bookmarks(&case.id).unwrap()[0].location,
        "v3:0:1:8"
    );
}

#[test]
fn tampered_projection_is_rejected_without_touching_committed_highlights() {
    let case = Case::new();
    case.db
        .add_highlight(&case.id, 0, (1, 0, 1, 1), "cyan")
        .unwrap();
    case.db.load_book_chapters(&case.id, &case.path).unwrap();
    let path = case.path.parent().unwrap().join(FILE);
    let mut p: Projection = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    p.snapshot.pages[0] = p.snapshot.pages[0].replace("Started", "Changed");
    std::fs::write(path, serde_json::to_vec(&p).unwrap()).unwrap();
    assert!(case.db.load_book_chapters(&case.id, &case.path).is_err());
    assert_eq!(
        case.db.get_highlights_by_chapter(&case.id).unwrap()[&0][0].start_char,
        8
    );
}

#[test]
fn failed_heading_migration_rolls_back_every_coordinate_and_retries_once() {
    let case = Case::new();
    case.db
        .add_highlight(&case.id, 0, (case.last, 2, case.last, 8), "yellow")
        .unwrap();
    let old_location = format!("v3:0:{}:5", case.last);
    case.db
        .update_progress(&case.id, 1, 1, &old_location)
        .unwrap();
    case.sql("CREATE TRIGGER fail_heading_commit BEFORE UPDATE OF metadata ON book_status WHEN NEW.metadata LIKE '%pdf_heading_projection_v2%' BEGIN SELECT RAISE(ABORT, 'test failure'); END;");
    assert!(case.db.load_book_chapters(&case.id, &case.path).is_err());
    assert_eq!(
        case.db.get_highlights_by_chapter(&case.id).unwrap()[&0][0].start_block,
        case.last
    );
    assert_eq!(
        case.db.get_status(&case.id).unwrap().unwrap().location,
        old_location
    );
    assert!(
        case.path.parent().unwrap().join(FILE).exists(),
        "uncommitted projection is reusable but not active"
    );
    case.sql("DROP TRIGGER fail_heading_commit;");
    case.db.load_book_chapters(&case.id, &case.path).unwrap();
    assert_eq!(
        case.db.get_highlights_by_chapter(&case.id).unwrap()[&0][0].start_block,
        2
    );
}

fn code_case() -> Case {
    let mut case = Case::with_bytes(super::super::tests::pdf_with_font(
        &[
            r"BT /F1 10 Tf 20 260 Td (fn main\(\) {) Tj 24 -14 Td (let reader = 1;) Tj -24 -14 Td (}) Tj /F2 10 Tf 0 -30 Td (Body text.) Tj ET",
        ],
        false,
        "Courier",
    ));
    let mut snapshot: Snapshot = serde_json::from_slice(&case.original).unwrap();
    snapshot.extractor = EXTRACTOR.into();
    snapshot.pages =
        vec!["<p>fn main() {</p><p>let reader = 1;</p><p>}</p><p>Body text.</p>".into()];
    case.original = serde_json::to_vec(&snapshot).unwrap();
    std::fs::write(
        case.path.parent().unwrap().join(SNAPSHOT_FILE),
        &case.original,
    )
    .unwrap();
    case
}

#[test]
fn content_migration_rebases_code_highlights_byte_ranges_and_recovers_identically() {
    let case = code_case();
    let h = case
        .db
        .add_highlight(&case.id, 0, (1, 4, 1, 10), "cyan")
        .unwrap();
    case.db
        .add_bookmark(&case.id, "v3:0:1:4", "reader")
        .unwrap();
    case.db
        .update_progress(&case.id, 1, 1, "v2:0:1:20")
        .unwrap();
    let loaded = case.db.load_book_chapters(&case.id, &case.path).unwrap();
    let blocks = split_blocks(&loaded.chapters[0].html);
    let source = crate::books::epub_meta::code_block_text(&blocks[0]).unwrap();
    assert_eq!(source, "fn main() {\n    let reader = 1;\n}");
    let expected = source.find("reader").unwrap();
    let assert_positions = || {
        let highlights = case.db.get_highlights_by_chapter(&case.id).unwrap();
        let h = highlights[&0].iter().find(|n| n.id == h).unwrap();
        assert_eq!(
            (h.start_block, h.start_char, h.end_block, h.end_char),
            (0, expected, 0, expected + 6)
        );
        assert_eq!(
            case.db.get_bookmarks(&case.id).unwrap()[0].location,
            format!("v3:0:0:{expected}")
        );
        assert_eq!(
            case.db.get_status(&case.id).unwrap().unwrap().location,
            "v3:0:0:16"
        );
    };
    assert_positions();
    case.db.load_book_chapters(&case.id, &case.path).unwrap();
    assert_positions();
    std::fs::remove_file(case.path.parent().unwrap().join(CONTENT_FILE)).unwrap();
    case.db.load_book_chapters(&case.id, &case.path).unwrap();
    assert_positions();
    assert_eq!(
        std::fs::read(case.path.parent().unwrap().join(SNAPSHOT_FILE)).unwrap(),
        case.original
    );
}

#[test]
fn content_migration_failure_rolls_back_and_tampering_is_rejected() {
    let case = code_case();
    case.db
        .add_highlight(&case.id, 0, (1, 4, 1, 10), "cyan")
        .unwrap();
    case.db.update_progress(&case.id, 1, 1, "v3:0:1:4").unwrap();
    case.sql("CREATE TRIGGER fail_content_commit BEFORE UPDATE OF metadata ON book_status WHEN NEW.metadata LIKE '%pdf_content_projection_v3%' BEGIN SELECT RAISE(ABORT, 'test failure'); END;");
    assert!(case.db.load_book_chapters(&case.id, &case.path).is_err());
    assert_eq!(
        case.db.get_highlights_by_chapter(&case.id).unwrap()[&0][0].start_block,
        1
    );
    assert_eq!(
        case.db.get_status(&case.id).unwrap().unwrap().location,
        "v3:0:1:4"
    );
    case.sql("DROP TRIGGER fail_content_commit;");
    case.db.load_book_chapters(&case.id, &case.path).unwrap();
    let cache = case.path.parent().unwrap().join(CONTENT_FILE);
    let mut projection: Projection =
        serde_json::from_slice(&std::fs::read(&cache).unwrap()).unwrap();
    projection.maps[0][1].offsets.as_mut().unwrap()[4] += 1;
    std::fs::write(cache, serde_json::to_vec(&projection).unwrap()).unwrap();
    assert!(case.db.load_book_chapters(&case.id, &case.path).is_err());
    assert_eq!(
        case.db.get_highlights_by_chapter(&case.id).unwrap()[&0][0].start_block,
        0
    );
}

#[test]
fn code_range_end_excludes_inserted_newline_and_indent_but_start_includes_them() {
    let mut case = code_case();
    let mut snapshot: Snapshot = serde_json::from_slice(&case.original).unwrap();
    snapshot.pages = vec!["<p>fn main() {let reader = 1;}</p><p>Body text.</p>".into()];
    case.original = serde_json::to_vec(&snapshot).unwrap();
    std::fs::write(
        case.path.parent().unwrap().join(SNAPSHOT_FILE),
        &case.original,
    )
    .unwrap();
    let first = case
        .db
        .add_highlight(&case.id, 0, (0, 0, 0, 11), "cyan")
        .unwrap();
    let next = case
        .db
        .add_highlight(&case.id, 0, (0, 11, 0, 14), "yellow")
        .unwrap();
    case.db.add_bookmark(&case.id, "v3:0:0:11", "let").unwrap();
    case.db.load_book_chapters(&case.id, &case.path).unwrap();
    let highlights = case.db.get_highlights_by_chapter(&case.id).unwrap();
    assert_eq!(
        highlights[&0]
            .iter()
            .find(|h| h.id == first)
            .unwrap()
            .end_char,
        11
    );
    let h = highlights[&0].iter().find(|h| h.id == next).unwrap();
    assert_eq!((h.start_char, h.end_char), (16, 19));
    assert_eq!(
        case.db.get_bookmarks(&case.id).unwrap()[0].location,
        "v3:0:0:16"
    );
}

#[test]
fn inserting_picture_rebases_caption_coordinates_without_changing_quote_text() {
    let mut case = Case::with_bytes(super::super::content::tests::image_fixture());
    let mut snapshot: Snapshot = serde_json::from_slice(&case.original).unwrap();
    snapshot.extractor = EXTRACTOR.into();
    snapshot.pages = vec!["<p>Readable text.</p><p>Figure 3-20. A diagram.</p>".into()];
    case.original = serde_json::to_vec(&snapshot).unwrap();
    std::fs::write(
        case.path.parent().unwrap().join(SNAPSHOT_FILE),
        &case.original,
    )
    .unwrap();
    case.db
        .add_highlight(&case.id, 0, (1, 0, 1, 11), "cyan")
        .unwrap();
    case.db
        .add_bookmark(&case.id, "v3:0:1:0", "Figure 3-20")
        .unwrap();
    let loaded = case.db.load_book_chapters(&case.id, &case.path).unwrap();
    let blocks = split_blocks(&loaded.chapters[0].html);
    assert!(blocks[1].contains("<img "));
    assert_eq!(
        crate::vocab::annotate::rendered_text(&blocks[2]),
        "Figure 3-20. A diagram."
    );
    let highlights = case.db.get_highlights_by_chapter(&case.id).unwrap();
    let h = &highlights[&0][0];
    assert_eq!(
        (h.start_block, h.start_char, h.end_block, h.end_char),
        (2, 0, 2, 11)
    );
    assert_eq!(
        case.db.get_bookmarks(&case.id).unwrap()[0].location,
        "v3:0:2:0"
    );
    case.db.load_book_chapters(&case.id, &case.path).unwrap();
    assert_eq!(
        std::fs::read(case.path.parent().unwrap().join(SNAPSHOT_FILE)).unwrap(),
        case.original
    );
}
