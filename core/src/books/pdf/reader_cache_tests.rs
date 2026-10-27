use super::super::reader_cache;
use super::*;
use std::{fs, path::Path};

fn open(case: &Case) -> LoadedChapters {
    case.db.load_book_chapters(&case.id, &case.path).unwrap()
}
fn cache_path(case: &Case) -> std::path::PathBuf {
    case.path.parent().unwrap().join(reader_cache::FILE)
}
fn hit(case: &Case) -> bool {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let url = format!(
            "sqlite:{}",
            case.root.join("data/database/app.db").display()
        );
        let pool = SqlitePool::connect(&url).await.unwrap();
        let deps = reader_cache::Dependencies::capture(&case.path).unwrap();
        let result = reader_cache::load(&pool, &case.id, &case.path, &deps).await;
        pool.close().await;
        result.is_some()
    })
}
fn warm(case: &Case) -> LoadedChapters {
    open(case); // A first legacy migration may create dependencies.
    let loaded = open(case);
    assert!(hit(case));
    loaded
}
fn html(loaded: &LoadedChapters) -> Vec<&str> {
    loaded.chapters.iter().map(|c| c.html.as_str()).collect()
}
fn native(bytes: Vec<u8>) -> Case {
    let root = std::env::temp_dir().join(format!("readwise-cache-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let source = root.join("source.pdf");
    fs::write(&source, bytes).unwrap();
    let db = Db::open_in(root.join("data")).unwrap();
    let book = db.import_book(&source).unwrap();
    let path = root.join("data").join(&book.file_path);
    let original = fs::read(path.parent().unwrap().join(SNAPSHOT_FILE)).unwrap();
    Case {
        root,
        path,
        db,
        id: book.id,
        original,
        last: 0,
    }
}
fn assert_no_temporary_files(root: &Path) {
    assert!(!fs::read_dir(root)
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with(".pdf-reader-")));
}

#[test]
fn imported_pdf_is_checkpointed_before_the_first_shelf_open() {
    let case = native(super::super::super::tests::fixture());
    assert!(cache_path(&case).is_file());
    assert!(hit(&case));
    let loaded = open(&case);
    assert_eq!(loaded.chapters.len(), 2);
    assert_eq!(loaded.chapters[0].source_path, "pdf-page-0");
    assert_eq!(
        fs::read(case.path.parent().unwrap().join(SNAPSHOT_FILE)).unwrap(),
        case.original
    );
}

#[test]
fn reopened_checkpoint_skips_legacy_transactions_and_preserves_notes_progress_and_flags() {
    let case = Case::new();
    case.db
        .add_highlight(&case.id, 0, (case.last, 2, case.last, 8), "cyan")
        .unwrap();
    case.db
        .add_bookmark(&case.id, &format!("v3:0:{}:5", case.last), "saved")
        .unwrap();
    case.db
        .update_progress(&case.id, 1, 1, &format!("v3:0:{}:5", case.last))
        .unwrap();
    let original = warm(&case);
    case.db.set_book_want_to_read(&case.id, true).unwrap();
    assert!(
        hit(&case),
        "unrelated metadata must not invalidate validated coordinates"
    );
    let location = case.db.get_status(&case.id).unwrap().unwrap().location;
    let bookmark = case.db.get_bookmarks(&case.id).unwrap()[0].location.clone();
    let old = case.db.get_highlights_by_chapter(&case.id).unwrap();
    let h = &old[&0][0];
    let span = (
        h.start_block,
        h.start_char,
        h.end_block,
        h.end_char,
        h.color.clone(),
    );
    case.sql("CREATE TRIGGER forbid_pdf_replay BEFORE UPDATE OF metadata ON book_status BEGIN SELECT RAISE(ABORT,'must not replay migration'); END;");
    let db = Db::open_in(case.root.join("data")).unwrap();
    let loaded = db.load_book_chapters(&case.id, &case.path).unwrap();
    assert_eq!(html(&loaded), html(&original));
    assert_eq!(db.get_status(&case.id).unwrap().unwrap().location, location);
    assert_eq!(db.get_bookmarks(&case.id).unwrap()[0].location, bookmark);
    let after = db.get_highlights_by_chapter(&case.id).unwrap();
    let h = &after[&0][0];
    assert_eq!(
        (
            h.start_block,
            h.start_char,
            h.end_block,
            h.end_char,
            h.color.clone()
        ),
        span
    );
    assert_eq!(
        fs::read(case.path.parent().unwrap().join(SNAPSHOT_FILE)).unwrap(),
        case.original
    );
}

#[test]
fn forged_corrupt_deleted_and_oversized_checkpoints_rebuild_from_frozen_sources() {
    let case = Case::new();
    let original = warm(&case);
    let good = fs::read(cache_path(&case)).unwrap();
    let mut forged: serde_json::Value = serde_json::from_slice(&good).unwrap();
    forged["snapshot"]["pages"][0] = "<p>untrusted content</p>".into();
    for data in [serde_json::to_vec(&forged).unwrap(), b"broken".to_vec()] {
        fs::write(cache_path(&case), data).unwrap();
        assert!(!hit(&case));
        assert_eq!(html(&open(&case)), html(&original));
    }
    fs::remove_file(cache_path(&case)).unwrap();
    assert!(!hit(&case));
    assert_eq!(html(&open(&case)), html(&original));
    fs::OpenOptions::new()
        .write(true)
        .open(cache_path(&case))
        .unwrap()
        .set_len(16 * 1024 * 1024 + 1)
        .unwrap();
    assert!(!hit(&case));
    assert_eq!(html(&open(&case)), html(&original));
    assert_no_temporary_files(case.path.parent().unwrap());
}

#[test]
fn an_unwritable_cache_never_blocks_reading_or_leaves_temporary_files() {
    let case = Case::new();
    let original = warm(&case);
    fs::remove_file(cache_path(&case)).unwrap();
    fs::create_dir(cache_path(&case)).unwrap();
    assert_eq!(html(&open(&case)), html(&original));
    assert_no_temporary_files(case.path.parent().unwrap());
}

#[test]
fn same_length_source_edit_with_restored_mtime_is_not_a_checkpoint_hit() {
    let case = Case::new();
    warm(&case);
    let mtime = fs::metadata(&case.path).unwrap().modified().unwrap();
    let mut bytes = fs::read(&case.path).unwrap();
    bytes[18] ^= 1;
    std::thread::sleep(std::time::Duration::from_millis(2));
    fs::write(&case.path, bytes).unwrap();
    fs::File::options()
        .write(true)
        .open(&case.path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(mtime))
        .unwrap();
    assert!(!hit(&case));
    assert!(case.db.load_book_chapters(&case.id, &case.path).is_err());
}

#[test]
fn changed_projection_missing_original_and_changed_db_marker_still_fail_closed() {
    for change in 0..3 {
        let case = Case::new();
        warm(&case);
        match change {
            0 => {
                let path=case.path.parent().unwrap().join(FILE);
                let mut projection:Projection=serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                projection.snapshot.pages[0]=projection.snapshot.pages[0].replace("Started","Changed");
                fs::write(path,serde_json::to_vec(&projection).unwrap()).unwrap();
            }
            1 => {fs::remove_file(case.path.parent().unwrap().join(SNAPSHOT_FILE)).unwrap();}
            _ => case.sql("UPDATE book_status SET metadata=json_set(metadata,'$.pdf_reflow_projection_v8','wrong-token');"),
        }
        assert!(!hit(&case));
        assert!(case.db.load_book_chapters(&case.id, &case.path).is_err());
        if change == 1 {
            assert!(!case.path.parent().unwrap().join(SNAPSHOT_FILE).exists());
        }
    }
}

#[test]
fn a_missing_historical_projection_is_recovered_not_hidden_by_the_checkpoint() {
    let case = Case::new();
    let original = warm(&case);
    let path = case.path.parent().unwrap().join(FILE);
    let bytes = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    assert!(!hit(&case));
    assert_eq!(html(&open(&case)), html(&original));
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn checkpoint_assets_are_relative_and_missing_derivatives_can_be_recovered() {
    let case = native(super::super::super::content::tests::image_fixture());
    assert!(hit(&case));
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(cache_path(&case)).unwrap()).unwrap();
    assert!(json["snapshot"]["pages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p.as_str().unwrap().contains("src=\"pdf-assets-v3/")));
    let loaded = open(&case);
    let assets = case.path.parent().unwrap().join("pdf-assets-v3");
    fs::remove_dir_all(&assets).unwrap();
    assert!(
        hit(&case),
        "derived assets are recoverable without changing source coordinates"
    );
    assert!(assets.is_dir());
    assert_eq!(html(&open(&case)), html(&loaded));
    assert_eq!(
        fs::read(case.path.parent().unwrap().join(SNAPSHOT_FILE)).unwrap(),
        case.original
    );
}

#[test]
fn native_v8_originals_are_pinned_independently_of_the_disposable_checkpoint() {
    for change in 0..3 {
        let case = native(super::super::super::tests::fixture());
        case.db
            .add_highlight(&case.id, 0, (0, 0, 0, 1), "cyan")
            .unwrap();
        fs::remove_file(cache_path(&case)).unwrap();
        let path = case.path.parent().unwrap().join(SNAPSHOT_FILE);
        if change == 0 {
            fs::remove_file(&path).unwrap();
        } else {
            let mut snapshot: serde_json::Value = serde_json::from_slice(&case.original).unwrap();
            if change == 1 {
                let text = snapshot["pages"][0].as_str().unwrap().to_owned();
                snapshot["pages"][0] = format!("{text}<p>Injected</p>").into();
            } else {
                snapshot["version"] = 999.into();
            }
            fs::write(&path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
        }
        assert!(case.db.load_book_chapters(&case.id, &case.path).is_err());
        if change == 0 {
            assert!(
                !path.exists(),
                "do not reconstruct native coordinates either"
            );
        }
        let notes = case.db.get_highlights_by_chapter(&case.id).unwrap();
        assert_eq!((notes[&0][0].start_char, notes[&0][0].end_char), (0, 1));
    }
}

#[test]
fn a_checkpoint_and_proof_copied_to_another_book_are_not_trusted() {
    let a = native(super::super::super::tests::fixture());
    let b = native(super::super::super::tests::fixture());
    let bytes = fs::read(cache_path(&a)).unwrap();
    fs::write(cache_path(&b), &bytes).unwrap();
    b.sql(&format!(
        "UPDATE book_status SET metadata=json_set(metadata,'$.pdf_reader_checkpoint_v1','{}');",
        fingerprint(&bytes)
    ));
    assert!(!hit(&b));
    assert_eq!(open(&b).chapters.len(), 2);
}
