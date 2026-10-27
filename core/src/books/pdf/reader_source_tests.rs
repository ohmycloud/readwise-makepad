use super::*;
use std::sync::atomic::Ordering;

#[test]
fn full_migration_chain_validates_the_source_once_per_open_not_per_projection() {
    let case = Case::new();
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
        let source = ReaderSource::new(&case.path);
        let first = reflow_v8_migration::load_reader_pages(&pool, &case.id, &case.path, &source)
            .await
            .unwrap();
        assert_eq!(source.validations.load(Ordering::Relaxed), 1);
        // A fresh open must validate again; the committed projection chain must
        // still reuse just one source check and must not move saved coordinates.
        let next = ReaderSource::new(&case.path);
        let second = reflow_v8_migration::load_reader_pages(&pool, &case.id, &case.path, &next)
            .await
            .unwrap();
        assert_eq!(next.validations.load(Ordering::Relaxed), 1);
        assert_eq!(first.chapters[0].html, second.chapters[0].html);
        assert_eq!(
            std::fs::read(case.path.parent().unwrap().join(SNAPSHOT_FILE)).unwrap(),
            case.original
        );
        pool.close().await;
    });
}

#[test]
fn a_new_open_rejects_changed_sources_and_missing_original_snapshots() {
    let case = Case::new();
    case.db.load_book_chapters(&case.id, &case.path).unwrap();
    let original = std::fs::read(&case.path).unwrap();
    std::fs::write(&case.path, b"changed pdf").unwrap();
    assert!(case.db.load_book_chapters(&case.id, &case.path).is_err());
    std::fs::write(&case.path, original).unwrap();
    case.db.load_book_chapters(&case.id, &case.path).unwrap();
    let path = case.path.parent().unwrap().join(SNAPSHOT_FILE);
    std::fs::remove_file(&path).unwrap();
    assert!(case.db.load_book_chapters(&case.id, &case.path).is_err());
    assert!(
        !path.exists(),
        "must not rebuild a source under committed coordinates"
    );
}
