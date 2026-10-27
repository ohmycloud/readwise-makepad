use super::*;

pub(super) fn opened(request_id: u64, id: &str) -> ReaderAction {
    ReaderAction::Opened {
        request_id,
        book: Mutex::new(Some(OpenBook {
            id: id.into(),
            source_key: None,
            title: format!("Book {id}"),
            chapters: vec![format!("<p>Content belonging to {id}</p>")],
            chapter_paths: vec!["chapter.xhtml".into()],
            math: Default::default(),
            figure_targets: vec![HashMap::new()],
            glossed_chapter_chunks: vec![None],
            glosses: HashMap::new(),
            lexicon: Default::default(),
            english_level: EnglishLevel::A1,
            known_words: HashSet::new(),
            toc: Vec::new(),
            highlights: HashMap::new(),
        })),
        resume: ReaderLocation {
            chapter: 0,
            block: 0,
            scroll: 0.,
            source: Some(12),
        },
        bookmarks: Vec::new(),
        gloss_count: 0,
        parse_ms: 0,
        prepare_ms: 0,
        initial_gloss_ms: 0,
    }
}

#[test]
fn opened_book_is_published_once_with_its_own_document_and_resume() {
    let action = opened(7, "new");
    let mut book = action.take_opened_book(7, Some("new")).unwrap();
    assert_eq!(book.id, "new");
    assert_eq!(
        book.chapter_chunks(0, false).unwrap(),
        ["<p>Content belonging to new</p>"]
    );
    let ReaderAction::Opened { resume, .. } = &action else {
        unreachable!()
    };
    assert_eq!(resume.source, Some(12));
    assert!(action.take_opened_book(7, Some("new")).is_none());
}

#[test]
fn rapid_a_b_a_switch_rejects_the_first_a_even_when_book_ids_match() {
    let first_a = opened(1, "a");
    let b = opened(2, "b");
    let latest_a = opened(3, "a");
    for stale in [&first_a, &b] {
        assert!(stale.take_opened_book(3, Some("a")).is_none());
    }
    assert_eq!(latest_a.take_opened_book(3, Some("a")).unwrap().id, "a");
    // Rejected payloads were not published or consumed as the new document.
    let ReaderAction::Opened { book, .. } = &first_a else {
        unreachable!()
    };
    assert_eq!(book.lock().unwrap().as_ref().unwrap().id, "a");
}

#[test]
fn wrong_book_missing_request_and_failed_open_cannot_publish_a_document() {
    let action = opened(4, "a");
    assert!(action.take_opened_book(4, Some("b")).is_none());
    assert!(action.take_opened_book(4, None).is_none());
    assert!(action.take_opened_book(5, Some("a")).is_none());
    let failed = ReaderAction::OpenError {
        request_id: 4,
        error: "missing file".into(),
    };
    assert!(failed.take_opened_book(4, Some("a")).is_none());
}

#[test]
fn book_open_saves_old_location_before_reset_and_only_ui_publishes_results() {
    let source = include_str!("app.rs");
    let open = source
        .split("    fn open_book_for_reading(")
        .nth(1)
        .unwrap()
        .split("    fn open_library_rename_editor(")
        .next()
        .unwrap();
    assert!(
        open.find("self.stop_reading_timer(cx, false)").unwrap()
            < open.find("begin_open_book_request(book.clone())").unwrap()
    );
    assert!(open.contains("reader.reset_for_book_open(cx)"));
    assert!(open.contains("self.sync_reader_open_state(cx)"));
    let worker = source
        .split("fn spawn_open_book(")
        .nth(1)
        .unwrap()
        .split("fn build_chapter_chunks(")
        .next()
        .unwrap();
    assert!(!worker.contains("CURRENT_BOOK.write()"));
    assert!(!worker.contains("replace_paragraphs("));
    assert!(worker.contains("reader_prepare::Prepared"));
    let preparation = include_str!("reader_prepare.rs");
    assert!(preparation.contains("book: Mutex::new(Some(self.book))"));
    assert!(!preparation.contains("CURRENT_BOOK.write()"));
    assert!(!preparation.contains("replace_paragraphs("));
    let reset = source
        .split("fn begin_open_book_request(")
        .nth(1)
        .unwrap()
        .split("fn spawn_open_book(")
        .next()
        .unwrap();
    assert!(reset.contains("*CURRENT_BOOK.write().unwrap() = None"));
    assert!(reset.contains("replace_paragraphs(Vec::new())"));
    assert!(reset.contains("CURRENT_READER_SOURCE.store(usize::MAX"));
    // Never route mark-known, layout refresh or page-count completion through
    // this destructive document reset. They have separate continuity contracts.
    assert_eq!(
        source
            .matches("begin_open_book_request(book.clone())")
            .count(),
        1
    );
    assert_eq!(source.matches("reader.reset_for_book_open(cx)").count(), 1);
}

#[test]
fn loading_and_failure_surfaces_survive_both_reload_paths_without_old_pages() {
    let source = include_str!("app.rs");
    let sync = source
        .split("    fn sync_reader_open_state(")
        .nth(1)
        .unwrap()
        .split("    fn open_book_for_reading(")
        .next()
        .unwrap();
    assert!(sync.contains("requested_book_is_ready()"));
    assert!(sync.contains("READER_OPEN_ERROR.read()"));
    assert!(sync
        .split_whitespace()
        .collect::<String>()
        .contains("ids!(chapter_list)).set_visible(cx,ready)"));
    assert!(sync.contains("ids!(reader_open_state)"));
    // All desktop/mobile title controls show the book name only. Loading or
    // failure belongs to the one body placeholder, not a second title message.
    assert!(sync.contains("self.set_reader_title_text(cx, &title)"));
    assert!(!sync.contains("Loading"));
    assert!(!sync.contains("加载中"));
    assert!(sync.contains("message.as_deref().unwrap_or_default()"));
    assert!(!sync.contains("self.set_reader_title_text(cx, &message)"));
    let reload = source
        .rsplit_once("    fn restore_after_script_reload(")
        .unwrap()
        .1
        .split("    fn ")
        .next()
        .unwrap();
    assert!(reload.contains("self.sync_reader_open_state(cx)"));
    let reset = source
        .split("    fn reset_for_book_open(")
        .nth(1)
        .unwrap()
        .split("    fn redraw_visible_code_views(")
        .next()
        .unwrap();
    for required in [
        "self.cancel_reader_tap(cx)",
        "self.page_turn.cancel()",
        "self.pager = ReaderPagedState::default()",
        "self.book_pages = BookPages::default()",
        "self.hit_runs.clear()",
        "self.text_maps.clear()",
    ] {
        assert!(
            reset.contains(required),
            "book switch must retire {required}"
        );
    }
}
