//! Bounded CPU-only reader preparations. Never retain widgets, layout, Areas or
//! page measurements here. Source/pin identity and Db's content epoch gate reuse.
use super::*;
use std::collections::VecDeque;
#[cfg(test)]
#[path = "reader_prepare_tests.rs"]
mod tests;

const MAX_BOOKS: usize = 2;
const MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Key {
    source: String,
    epoch: u64,
    level: EnglishLevel,
    gloss: bool,
}
impl Key {
    pub(super) fn current_data(&self) -> bool {
        DB.get().is_some_and(|db| db.reader_epoch() == self.epoch)
            && *ENGLISH_LEVEL.read().unwrap() == self.level
            && *GLOSS_ENABLED.read().unwrap() == self.gloss
    }
    pub(super) fn capture(item: &ShelfItem) -> Option<Self> {
        let db = DB.get()?;
        let epoch = db.reader_epoch();
        let source = db.reader_identity(&item.id, std::path::Path::new(&item.file_path))?;
        (epoch == db.reader_epoch()).then(|| Self {
            source,
            epoch,
            level: *ENGLISH_LEVEL.read().unwrap(),
            gloss: *GLOSS_ENABLED.read().unwrap(),
        })
    }
}

pub(super) struct Prepared {
    pub book: OpenBook,
    pub resume: ReaderLocation,
    pub bookmarks: Vec<readwise_core::books::Bookmark>,
    pub parse_ms: u128,
    pub prepare_ms: u128,
    pub initial_gloss_ms: u128,
}
impl Prepared {
    pub(super) fn into_action(self, request_id: u64) -> ReaderAction {
        let gloss_count = self.book.glosses.len();
        ReaderAction::Opened {
            request_id,
            book: Mutex::new(Some(self.book)),
            resume: self.resume,
            bookmarks: self.bookmarks,
            gloss_count,
            parse_ms: self.parse_ms,
            prepare_ms: self.prepare_ms,
            initial_gloss_ms: self.initial_gloss_ms,
        }
    }
    fn bytes(&self) -> usize {
        fn map_bytes(m: &HashMap<String, String>) -> usize {
            m.capacity() * std::mem::size_of::<(String, String)>()
                + m.iter()
                    .map(|(a, b)| a.capacity() + b.capacity())
                    .sum::<usize>()
        }
        let b = &self.book;
        let strings = |s: &[String]| {
            s.iter().map(String::capacity).sum::<usize>() + s.len() * std::mem::size_of::<String>()
        };
        strings(&b.chapters)
            + strings(&b.chapter_paths)
            + b.glossed_chapter_chunks
                .iter()
                .flatten()
                .map(|s| strings(s))
                .sum::<usize>()
            + map_bytes(&b.glosses)
            + map_bytes(&b.lexicon.standard)
            + map_bytes(&b.lexicon.custom)
            + b.known_words
                .iter()
                .map(|s| s.capacity() + std::mem::size_of::<String>())
                .sum::<usize>()
            + b.figure_targets.iter().map(map_bytes).sum::<usize>()
            // Registered formula SVGs may be cloned into the visual registry.
            + b.math.prepared_bytes() * 2
            + b.toc.iter().map(|t| t.title.capacity() + t.fragment.as_ref().map_or(0,String::capacity) + std::mem::size_of_val(t)).sum::<usize>()
            + b.highlights.values().flatten().map(|h|h.id.capacity()+h.color.capacity()+std::mem::size_of_val(h)).sum::<usize>()
            + self.bookmarks.iter().map(|b|b.id.capacity()+b.label.capacity()+b.location.capacity()+std::mem::size_of_val(b)).sum::<usize>()
    }
}

#[derive(Default)]
struct Cache {
    ready: VecDeque<(Prepared, usize)>,
    bytes: usize,
    jobs: HashMap<String, std::sync::Weak<Job>>,
}
impl Cache {
    fn join(&self, id: &str, key: Option<&Key>, request_id: u64) -> bool {
        let Some(job) = self.jobs.get(id).and_then(std::sync::Weak::upgrade) else {
            return false;
        };
        if key.is_none() || job.key.as_ref() != key {
            return false;
        }
        let active = job.request.load(Ordering::Acquire);
        // Only a request-free prewarm can be adopted. An older foreground
        // generation may already be cancelling; joining it can strand A→B→A.
        if request_id != 0 && active != 0 && active != request_id {
            return false;
        }
        if request_id != 0 {
            job.request.store(request_id, Ordering::Release);
        }
        true
    }
    fn put(&mut self, prepared: Prepared) -> Vec<Prepared> {
        let mut retired = Vec::new();
        if prepared.book.source_key.is_none() {
            retired.push(prepared);
            return retired;
        }
        let size = prepared.bytes();
        if size > MAX_BYTES {
            retired.push(prepared);
            return retired;
        }
        if let Some(i) = self
            .ready
            .iter()
            .position(|(p, _)| p.book.id == prepared.book.id)
        {
            let (p, n) = self.ready.remove(i).unwrap();
            self.bytes -= n;
            retired.push(p);
        }
        while self.ready.len() >= MAX_BOOKS || self.bytes + size > MAX_BYTES {
            let Some((p, n)) = self.ready.pop_front() else {
                break;
            };
            self.bytes -= n;
            retired.push(p);
        }
        self.bytes += size;
        self.ready.push_back((prepared, size));
        retired
    }
    fn take(&mut self, id: &str, key: &Key) -> Option<Prepared> {
        let i = self
            .ready
            .iter()
            .position(|(p, _)| p.book.id == id && p.book.source_key.as_ref() == Some(key))?;
        let (p, n) = self.ready.remove(i)?;
        self.bytes -= n;
        Some(p)
    }
}
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(Cache::default()));
struct Job {
    key: Option<Key>,
    request: AtomicU64,
}

pub(super) fn take(item: &ShelfItem, key: Option<&Key>) -> Option<Prepared> {
    let mut p = CACHE.lock().unwrap().take(&item.id, key?)?;
    p.book.title = item.title.clone();
    Some(p)
}
pub(super) fn remember(
    mut book: OpenBook,
    resume: ReaderLocation,
    bookmarks: Vec<readwise_core::books::Bookmark>,
) {
    let document = book.math.suspend();
    let retired = CACHE.lock().unwrap().put(Prepared {
        book,
        resume,
        bookmarks,
        parse_ms: 0,
        prepare_ms: 0,
        initial_gloss_ms: 0,
    });
    // Drop large, evicted dictionaries/presentations off the event thread.
    if !retired.is_empty() || document.is_some() {
        std::thread::spawn(move || drop((retired, document)));
    }
}

pub(super) fn warm_recent() {
    if CURRENT_ITEM.read().unwrap().is_some() {
        return;
    }
    let item = App::reading_home_book();
    if let Some(item) = item.filter(|item| item.last_read_at.is_some()) {
        request(item, 0);
    }
}

pub(super) fn request(item: ShelfItem, request_id: u64) {
    let key = Key::capture(&item);
    let job = {
        let mut cache = CACHE.lock().unwrap();
        if request_id == 0
            && cache
                .ready
                .iter()
                .any(|(p, _)| p.book.id == item.id && p.book.source_key == key)
        {
            return;
        }
        if cache.join(&item.id, key.as_ref(), request_id) {
            return;
        }
        let job = Arc::new(Job {
            key,
            request: AtomicU64::new(request_id),
        });
        cache.jobs.retain(|_, job| job.strong_count() > 0);
        cache.jobs.insert(item.id.clone(), Arc::downgrade(&job));
        job
    };
    std::thread::spawn(move || {
        let wanted = job.request.load(Ordering::Acquire);
        if wanted != 0 && wanted != OPEN_BOOK_REQUEST_ID.load(Ordering::Acquire) {
            return;
        }
        // A prewarm has no UI identity and cannot navigate/save progress. A tap
        // can adopt it, rather than starting a duplicate PDF/gloss preparation.
        let epoch = DB.get().map(readwise_core::Db::reader_epoch);
        let mut result = prepare_open_book(item.clone(), wanted);
        if epoch != DB.get().map(readwise_core::Db::reader_epoch) {
            result = prepare_open_book(item.clone(), wanted);
        }
        {
            let mut cache = CACHE.lock().unwrap();
            if cache
                .jobs
                .get(&item.id)
                .and_then(std::sync::Weak::upgrade)
                .is_some_and(|current| Arc::ptr_eq(&current, &job))
            {
                cache.jobs.remove(&item.id);
            }
        }
        let request = job.request.load(Ordering::Acquire);
        match result {
            Ok(prepared)
                if request != 0 && request == OPEN_BOOK_REQUEST_ID.load(Ordering::Acquire) =>
            {
                Cx::post_action(prepared.into_action(request));
            }
            Ok(mut prepared) => {
                if prepared.book.source_key.is_some()
                    && prepared.book.source_key.as_ref() == Key::capture(&item).as_ref()
                {
                    drop(prepared.book.math.suspend());
                    ::log::info!(
                        "Reader prepared in background: {} ({}ms)",
                        item.title,
                        prepared.prepare_ms
                    );
                    let retired = CACHE.lock().unwrap().put(prepared);
                    drop(retired);
                }
            }
            Err(error)
                if request != 0 && request == OPEN_BOOK_REQUEST_ID.load(Ordering::Acquire) =>
            {
                Cx::post_action(ReaderAction::OpenError {
                    request_id: request,
                    error,
                });
            }
            Err(_) => {}
        }
    });
}
