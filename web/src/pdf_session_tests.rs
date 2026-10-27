//! Session lifetime is independent from the small rendered-page LRU.
use super::*;

#[test]
fn parser_session_survives_page_eviction_and_is_bounded_and_deleted_with_book() {
    let root = tempfile::tempdir().unwrap();
    let mut library = Library {
        db: BlockingDb(Some(Db::open_in(root.path().to_owned()).unwrap())),
        cache: VecDeque::new(),
        math_cache: VecDeque::new(),
        pdf_sessions: VecDeque::new(),
    };
    let session_ids = |library: &Library| {
        library
            .pdf_sessions
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>()
    };
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vector-figure.pdf");
    let mut ids = Vec::new();
    for _ in 0..3 {
        ids.push(library.db.import_book(&path).unwrap().id);
    }
    // Returning after eviction must reproduce identical geometry/source HTML.
    let initial = library.reader(&ids[0], 0).unwrap().unwrap().blocks;
    for page in 1..6 {
        library.reader(&ids[0], page).unwrap().unwrap();
    }
    library.math_cache.clear();
    assert_eq!(session_ids(&library), [ids[0].clone()]);
    assert_eq!(library.reader(&ids[0], 0).unwrap().unwrap().blocks, initial);
    library.reader(&ids[1], 1).unwrap().unwrap();
    // Touching the first book updates parser-session LRU even on page misses.
    library.math_cache.clear();
    library.reader(&ids[0], 1).unwrap().unwrap();
    library.reader(&ids[2], 1).unwrap().unwrap();
    assert_eq!(session_ids(&library), [ids[0].clone(), ids[2].clone()]);
    library.delete_book(&ids[0]).unwrap();
    assert_eq!(session_ids(&library), [ids[2].clone()]);
    assert!(library.reader(&ids[0], 1).unwrap().is_none());
}
