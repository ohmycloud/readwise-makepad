use super::*;
use readwise_core::{books::epub_meta::split_blocks, vocab::annotate::rendered_text};

fn input(path: &Path, page: usize) -> (Vec<String>, Vec<String>) {
    let loaded = readwise_core::books::pdf::load_pages(path).unwrap();
    let blocks = split_blocks(&loaded.chapters[page].html);
    let text = blocks.iter().map(|b| rendered_text(b)).collect();
    (blocks, text)
}
fn source(root: &Path, figure: bool) -> PathBuf {
    let path = root.join("book.pdf");
    fs::write(
        &path,
        if figure {
            include_bytes!("../../web/tests/fixtures/vector-figure.pdf").as_slice()
        } else {
            include_bytes!("../../web/tests/fixtures/math-scattered.pdf").as_slice()
        },
    )
    .unwrap();
    path
}
fn render(page: &MathPage, blocks: &[String]) -> Vec<String> {
    blocks
        .iter()
        .enumerate()
        .map(|(i, b)| page.annotate(i, b, str::to_owned))
        .collect()
}
#[test]
fn fresh_sessions_restore_identical_formulas_figures_and_source_without_opening_pdf() {
    for figure in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let path = source(temp.path(), figure);
        let dir = temp.path().join("visual");
        let (blocks, text) = input(&path, 0);
        let cold = PdfSession::with_cache(&path, &dir);
        let a = cold.page(0, &blocks, &text);
        assert!(a.bytes > 0);
        assert!(cold.document.get().unwrap().is_some());
        let warm = cold.without_document();
        let b = warm
            .cached_page(0, &blocks, &text)
            .expect("cache hit before any parsing/Typst");
        assert_eq!(render(&a, &blocks), render(&b, &blocks));
        assert_eq!(a.bytes, b.bytes);
        assert!(warm.document.get().is_none());
        assert_eq!(
            render(&warm.page(0, &blocks, &text), &blocks),
            render(&a, &blocks)
        );
        assert!(warm.document.get().is_none());
        for (html, raw) in render(&b, &blocks).iter().zip(&text) {
            assert_eq!(rendered_text(html), *raw);
        }
        for (&id, parts) in &a.fragments {
            for (x, y) in parts.iter().zip(&b.fragments[&id]) {
                if let (Projection::Formula(x), Projection::Formula(y)) =
                    (&x.projection, &y.projection)
                {
                    assert_eq!(
                        (
                            x.width_em.to_bits(),
                            x.height_em.to_bits(),
                            x.align_em.to_bits()
                        ),
                        (
                            y.width_em.to_bits(),
                            y.height_em.to_bits(),
                            y.align_em.to_bits()
                        )
                    );
                }
            }
        }
    }
}
#[test]
fn ordinary_empty_presentations_are_cached_but_failures_and_standalone_calls_are_not() {
    let temp = tempfile::tempdir().unwrap();
    let path = source(temp.path(), true);
    let dir = temp.path().join("visual");
    let (blocks, text) = input(&path, 1);
    assert_eq!(
        PdfSession::with_cache(&path, &dir)
            .page(1, &blocks, &text)
            .bytes,
        0
    );
    let session = PdfSession::with_cache(&path, &dir);
    assert!(session.cached_page(1, &blocks, &text).is_some());
    assert!(session.document.get().is_none());
    let missing = PdfSession::with_cache(
        temp.path().join("missing.pdf"),
        temp.path().join("failures"),
    );
    missing.page(0, &[], &[]);
    assert!(!temp.path().join("failures").exists());
    let plain = temp.path().join("plain.pdf");
    fs::copy(&path, &plain).unwrap();
    PdfSession::new(&plain).page(1, &blocks, &text);
    assert!(!temp.path().join("pdf-visual-cache").exists());
}
#[test]
fn cache_keys_include_source_identity_page_blocks_and_text_not_just_file_length() {
    let temp = tempfile::tempdir().unwrap();
    let path = source(temp.path(), false);
    let dir = temp.path().join("visual");
    let (blocks, text) = input(&path, 0);
    let session = PdfSession::with_cache(&path, &dir);
    session.page(0, &blocks, &text);
    assert!(session.cached_page(1, &blocks, &text).is_none());
    let mut b = blocks.clone();
    b[0] = format!("<strong>{}</strong>", b[0]);
    assert!(session.cached_page(0, &b, &text).is_none());
    let mut t = text.clone();
    t[0].push('x');
    assert!(session.cached_page(0, &blocks, &t).is_none());
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let mut bytes = fs::read(&path).unwrap();
    bytes[18] ^= 1;
    std::thread::sleep(std::time::Duration::from_millis(2));
    fs::write(&path, &bytes).unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(mtime))
        .unwrap();
    assert!(session.cached_page(0, &blocks, &text).is_none());
    assert!(
        PdfSession::with_cache(&path, &dir)
            .cached_page(0, &blocks, &text)
            .is_none()
    );
}
#[test]
fn corrupted_oversized_wrong_version_and_invalid_utf8_ranges_are_misses() {
    let temp = tempfile::tempdir().unwrap();
    let path = source(temp.path(), false);
    let (blocks, text) = input(&path, 0);
    let cache = PageCache::new(&path, temp.path().join("visual")).unwrap();
    let key = cache.key(0, &blocks, &text).unwrap();
    let file = cache.path(&key);
    let good = PdfSession::new(&path).page(0, &blocks, &text);
    cache.write(0, &blocks, &text, &good).unwrap();
    let original = fs::read(&file).unwrap();
    for data in [
        b"bad JSON".to_vec(),
        original
            .iter()
            .copied()
            .chain(std::iter::once(b'x'))
            .collect(),
    ] {
        fs::write(&file, data).unwrap();
        assert!(cache.read(0, &blocks, &text).is_none());
    }
    let mut record: Record = serde_json::from_slice(&original).unwrap();
    record.version += 1;
    fs::write(&file, serde_json::to_vec(&record).unwrap()).unwrap();
    assert!(cache.read(0, &blocks, &text).is_none());
    let mut record: Record = serde_json::from_slice(&original).unwrap();
    let mut page: MathPage = serde_json::from_str(&record.payload).unwrap();
    page.fragments.values_mut().next().unwrap()[0].end = usize::MAX;
    record.payload = serde_json::to_string(&page).unwrap();
    record.checksum = digest(record.payload.as_bytes());
    fs::write(&file, serde_json::to_vec(&record).unwrap()).unwrap();
    assert!(cache.read(0, &blocks, &text).is_none());
    fs::File::create(&file)
        .unwrap()
        .set_len(MAX_RECORD as u64 + 1)
        .unwrap();
    assert!(cache.read(0, &blocks, &text).is_none());
    cache.write(0, &blocks, &text, &good).unwrap();
    assert!(cache.read(0, &blocks, &text).is_some());
}
#[test]
fn unwritable_cache_is_optional_and_does_not_leave_temporary_files() {
    let temp = tempfile::tempdir().unwrap();
    let path = source(temp.path(), false);
    let (blocks, text) = input(&path, 0);
    let dir = temp.path().join("not-a-directory");
    fs::write(&dir, b"keep").unwrap();
    let page = PdfSession::with_cache(&path, &dir).page(0, &blocks, &text);
    assert!(page.bytes > 0);
    assert_eq!(fs::read(&dir).unwrap(), b"keep");
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
}
#[test]
fn disk_count_and_byte_budgets_only_prune_owned_cache_files() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("keep.txt"), b"user data").unwrap();
    for i in 0..5 {
        fs::write(temp.path().join(format!("page-v1-{i}.json")), b"123456").unwrap();
    }
    prune(temp.path(), 2, 10);
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
    assert!(temp.path().join("keep.txt").exists());
}
#[test]
fn record_writer_rejects_expansion_beyond_its_budget() {
    let mut writer = Limited(Vec::new());
    let bytes = vec![0; MAX_RECORD + 1];
    assert!(writer.write(&bytes).is_err());
    assert!(writer.0.is_empty());
}
