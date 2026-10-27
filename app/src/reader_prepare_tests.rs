use super::*;
fn key() -> Key {
    Key {
        source: "validated-source-and-pins".into(),
        epoch: 1,
        level: EnglishLevel::A1,
        gloss: true,
    }
}
fn prepared(id: &str, body: usize) -> Prepared {
    let a = super::super::reader_open_tests::opened(1, id);
    let mut book = a.take_opened_book(1, Some(id)).unwrap();
    book.source_key = Some(key());
    book.chapters = vec!["x".repeat(body)];
    book.glossed_chapter_chunks =
        vec![Some(vec!["<p><ruby>reader<rt>读者</rt></ruby></p>".into()])];
    Prepared {
        book,
        resume: ReaderLocation {
            chapter: 0,
            block: 7,
            scroll: -31.,
            source: Some(123),
        },
        bookmarks: vec![],
        parse_ms: 0,
        prepare_ms: 0,
        initial_gloss_ms: 0,
    }
}
#[test]
fn preparation_is_one_shot_and_preserves_exact_resume_and_ruby_slots() {
    let mut c = Cache::default();
    assert!(c.put(prepared("a", 10)).is_empty());
    assert!(c.take("b", &key()).is_none());
    let p = c.take("a", &key()).unwrap();
    assert_eq!(
        p.resume,
        ReaderLocation {
            chapter: 0,
            block: 7,
            scroll: -31.,
            source: Some(123)
        }
    );
    assert_eq!(
        p.book.glossed_chapter_chunks[0].as_ref().unwrap()[0],
        "<p><ruby>reader<rt>读者</rt></ruby></p>"
    );
    assert!(c.take("a", &key()).is_none());
    assert_eq!(c.bytes, 0);
    let action = p.into_action(9);
    assert!(action.take_opened_book(8, Some("a")).is_none());
    assert!(action.take_opened_book(9, Some("a")).is_some());
    assert!(action.take_opened_book(9, Some("a")).is_none());
}
#[test]
fn source_coordinate_pins_words_level_and_gloss_changes_are_misses() {
    let mut c = Cache::default();
    c.put(prepared("a", 10));
    let mut changes = vec![key(); 4];
    changes[0].source.push_str("changed");
    changes[1].epoch += 1;
    changes[2].level = EnglishLevel::C3;
    changes[3].gloss = false;
    for changed in changes {
        assert!(c.take("a", &changed).is_none());
    }
    assert!(c.take("a", &key()).is_some());
}
#[test]
fn book_count_payload_budget_and_uncacheable_readers_are_bounded() {
    let mut c = Cache::default();
    for id in ["a", "b", "c"] {
        c.put(prepared(id, 100));
    }
    assert_eq!(c.ready.len(), 2);
    assert!(c.take("a", &key()).is_none());
    let huge = prepared("huge", MAX_BYTES + 1);
    assert_eq!(c.put(huge).len(), 1);
    assert!(c.bytes <= MAX_BYTES);
    let mut no_guard = prepared("unsafe", 10);
    no_guard.book.source_key = None;
    assert_eq!(c.put(no_guard).len(), 1);
    assert_eq!(c.ready.len(), 2);
    let mut c = Cache::default();
    for id in ["a", "b", "c"] {
        c.put(prepared(id, MAX_BYTES / 2));
    }
    assert!(c.bytes <= MAX_BYTES);
    assert_eq!(c.ready.len(), 1);
}
#[test]
fn a_tap_adopts_matching_prewarm_but_changed_or_unverifiable_sources_do_not_join() {
    let mut c = Cache::default();
    let job = Arc::new(Job {
        key: Some(key()),
        request: AtomicU64::new(0),
    });
    c.jobs.insert("a".into(), Arc::downgrade(&job));
    assert!(c.join("a", Some(&key()), 4));
    assert_eq!(job.request.load(Ordering::Acquire), 4);
    assert!(!c.join("a", Some(&key()), 7));
    assert_eq!(job.request.load(Ordering::Acquire), 4);
    assert!(c.join("a", Some(&key()), 4));
    assert!(c.join("a", Some(&key()), 0));
    assert_eq!(job.request.load(Ordering::Acquire), 4);
    let mut other = key();
    other.epoch += 1;
    assert!(!c.join("a", Some(&other), 9));
    assert!(!c.join("b", Some(&key()), 9));
    assert!(!c.join("a", None, 9));
}
#[test]
fn returning_to_current_reader_bypasses_destructive_reset_and_prewarm_has_no_ui_side_effects() {
    let s = include_str!("app.rs")
        .split("    fn open_book_for_reading(")
        .nth(1)
        .unwrap()
        .split("    fn open_library_rename_editor(")
        .next()
        .unwrap();
    let fast = s
        .split("if same_ready {")
        .nth(1)
        .unwrap()
        .split("let prepared =")
        .next()
        .unwrap();
    assert!(fast.contains("return;"));
    for forbidden in [
        "reset_for_book_open",
        "begin_open_book_request",
        "set_chapter_at",
        "replace_paragraphs",
    ] {
        assert!(!fast.contains(forbidden));
    }
    let notice = include_str!("app.rs")
        .split("    fn handle_reader_open_notice(")
        .nth(1)
        .unwrap()
        .split("    fn open_book_for_reading(")
        .next()
        .unwrap();
    assert!(
        notice.find("!key.current_data()").unwrap() < notice.find("*CURRENT_BOOK.write()").unwrap()
    );
    let prepare = include_str!("app.rs")
        .split("fn prepare_open_book(")
        .nth(1)
        .unwrap()
        .split("fn build_chapter_chunks(")
        .next()
        .unwrap();
    assert!(prepare.contains("auxiliary_ready &&"));
    assert!(
        prepare.find("Open request superseded").unwrap()
            < prepare.find("math.prepare_initial").unwrap()
    );
    let source = include_str!("reader_prepare.rs");
    for forbidden in [
        "CURRENT_BOOK.write",
        "CURRENT_ITEM.write",
        "start_reading_timer",
        "activate_page",
        "WidgetRef",
        "Area::",
    ] {
        assert!(!source.contains(forbidden));
    }
    let list = include_str!("reader_list.rs")
        .split("pub fn stop_scrolling(")
        .nth(1)
        .unwrap()
        .split("    pub fn ")
        .next()
        .unwrap();
    assert!(list.contains("ScrollState::Stopped"));
    assert!(!list.contains("first_scroll ="));
    assert!(!list.contains("height_tree.clear"));
}
