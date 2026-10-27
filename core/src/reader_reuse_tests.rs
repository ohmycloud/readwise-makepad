//! Only copied/temporary sources are used; never touch a formal library.
use super::*;
struct Temp(PathBuf);
impl Temp {
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn fixture() -> (Temp, Db, SimpleBook, PathBuf) {
    let root = Temp(std::env::temp_dir().join(format!("readwise-reuse-{}", uuid::Uuid::new_v4())));
    let db = Db::open_in(root.path().into()).unwrap();
    let input = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reader-text.pdf");
    let book = db.import_book(&input).unwrap();
    let path = root.path().join(&book.file_path);
    db.load_book_chapters(&book.id, &path).unwrap();
    (root, db, book, path)
}
#[test]
fn progress_and_flags_do_not_invalidate_reader_but_definitions_and_annotations_do() {
    let (_r, db, b, path) = fixture();
    let identity = db.reader_identity(&b.id, &path).unwrap();
    let epoch = db.reader_epoch();
    db.update_progress(&b.id, 1, 1, "v2:0:0:0").unwrap();
    db.set_book_want_to_read(&b.id, true).unwrap();
    assert_eq!(epoch, db.reader_epoch());
    assert_eq!(db.reader_identity(&b.id, &path), Some(identity));
    db.add_known_word("reader", "reader-test").unwrap();
    assert!(db.reader_epoch() > epoch);
    let epoch = db.reader_epoch();
    db.remove_known_word("reader").unwrap();
    assert!(db.reader_epoch() > epoch);
    let epoch = db.reader_epoch();
    db.add_bookmark(&b.id, "v2:0:0:0", "saved").unwrap();
    assert!(db.reader_epoch() > epoch);
    let epoch = db.reader_epoch();
    let _ = db.get_bookmarks(&b.id).unwrap();
    assert_eq!(epoch, db.reader_epoch());
    assert!(db.add_bookmark("missing-book", "v2:0:0:0", "bad").is_err());
    assert!(db.reader_epoch() > epoch);
}
#[test]
fn changed_or_malformed_coordinate_pins_never_reuse_the_native_context() {
    let (_r, db, b, path) = fixture();
    let identity = db.reader_identity(&b.id, &path).unwrap();
    let raw: String = db
        .rt
        .block_on(
            sqlx::query_scalar("SELECT metadata FROM book_status WHERE book_id=?")
                .bind(&b.id)
                .fetch_one(&db.pool),
        )
        .unwrap();
    let mut metadata: serde_json::Value = serde_json::from_str(&raw).unwrap();
    metadata["pdf_original_snapshot_v1"] = serde_json::json!("forged");
    db.rt
        .block_on(
            sqlx::query("UPDATE book_status SET metadata=? WHERE book_id=?")
                .bind(metadata.to_string())
                .bind(&b.id)
                .execute(&db.pool),
        )
        .unwrap();
    assert_ne!(db.reader_identity(&b.id, &path), Some(identity));
    assert!(db
        .load_book_chapters(&b.id, &path)
        .err()
        .unwrap()
        .contains("已被替换"));
    db.rt
        .block_on(
            sqlx::query("UPDATE book_status SET metadata='not-json' WHERE book_id=?")
                .bind(&b.id)
                .execute(&db.pool),
        )
        .unwrap();
    assert_eq!(db.reader_identity(&b.id, &path), None);
    assert!(db
        .load_book_chapters(&b.id, &path)
        .err()
        .unwrap()
        .contains("元数据损坏"));
}
#[test]
fn deleted_or_overwritten_derived_images_cannot_bypass_asset_recovery() {
    let (_r, db, b, path) = fixture();
    let dir = path.parent().unwrap().join("pdf-assets-v3");
    fs::create_dir_all(&dir).unwrap();
    let png = dir.join("extra.png");
    fs::write(&png, b"1234").unwrap();
    let first = db.reader_identity(&b.id, &path).unwrap();
    let time = fs::metadata(&png).unwrap().modified().unwrap();
    fs::write(&png, b"5678").unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&png)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(time))
        .unwrap();
    assert_ne!(db.reader_identity(&b.id, &path), Some(first));
    let first = db.reader_identity(&b.id, &path).unwrap();
    fs::remove_file(&png).unwrap();
    assert_ne!(db.reader_identity(&b.id, &path), Some(first));
}
#[test]
fn same_length_source_edits_restoring_mtime_and_missing_snapshots_invalidate_reuse() {
    let (_r, db, b, path) = fixture();
    let identity = db.reader_identity(&b.id, &path).unwrap();
    let time = fs::metadata(&path).unwrap().modified().unwrap();
    let original = fs::read(&path).unwrap();
    let mut changed = original.clone();
    changed[0] ^= 1;
    fs::write(&path, &changed).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(time))
        .unwrap();
    assert_ne!(db.reader_identity(&b.id, &path), Some(identity));
    fs::write(&path, &original).unwrap();
    let snapshot = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.file_name().unwrap() == "pdf-text-v1.json")
        .unwrap();
    let identity = db.reader_identity(&b.id, &path).unwrap();
    fs::remove_file(snapshot).unwrap();
    assert_ne!(db.reader_identity(&b.id, &path), Some(identity));
    assert!(db
        .load_book_chapters(&b.id, &path)
        .err()
        .unwrap()
        .contains("原始文本快照缺失"));
}
