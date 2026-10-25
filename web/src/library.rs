//! Bounded blocking bridge to the existing Db facade (which owns a Tokio runtime).
//! Never call its block_on methods on Topcoat's asynchronous request threads.
use crate::{
    content::{self, BookContent},
    settings::Preferences,
};
use readwise_core::{
    Db,
    books::models::BookShelfItem,
    settings::EnglishLevel,
    vocab::{lemma::lemmatize, models::VocabBookAddInput},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::sync::Semaphore;

pub const MAX_UPLOAD: usize = 200 * 1024 * 1024;
const CACHE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone)]
pub struct State {
    inner: Arc<Mutex<Library>>,
    jobs: Arc<Semaphore>,
    pub data_dir: PathBuf,
    pub allowed_hosts: Vec<String>,
    pub storage: sqlx::SqlitePool,
    pub ai_jobs: Arc<Semaphore>,
    _directory_lock: Arc<std::fs::File>,
}

pub struct Library {
    pub db: BlockingDb,
    cache: VecDeque<(String, Arc<BookContent>)>,
    math_cache: VecDeque<(String, usize, Arc<crate::math::MathPage>)>,
}

/// A native runtime must also be destroyed outside an async runtime context.
/// This matters on graceful server shutdown and in request-level tests.
pub struct BlockingDb(Option<Db>);

impl Deref for BlockingDb {
    type Target = Db;
    fn deref(&self) -> &Db {
        self.0.as_ref().unwrap()
    }
}

impl Drop for BlockingDb {
    fn drop(&mut self) {
        if let Some(db) = self.0.take() {
            if tokio::runtime::Handle::try_current().is_ok() {
                let _ = std::thread::spawn(move || drop(db)).join();
            } else {
                drop(db);
            }
        }
    }
}

pub struct ReaderPage {
    pub book: BookShelfItem,
    pub title: String,
    pub chapter: usize,
    pub total: usize,
    pub blocks: Vec<String>,
    pub toc: Vec<(String, usize, usize, Option<String>)>,
    pub preferences: Preferences,
    pub resume: Location,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub chapter: usize,
    pub block: usize,
    pub offset: usize,
}

impl Location {
    pub fn encode(self) -> String {
        // Web DOM text coordinates are intentionally not native TextFlow v3.
        format!("web:v1:{}:{}:{}", self.chapter, self.block, self.offset)
    }

    pub fn decode(value: &str) -> Self {
        let Some(value) = value.strip_prefix("web:v1:") else {
            return Self::default();
        };
        let fields: Vec<_> = value.split(':').collect();
        if fields.len() != 3 {
            return Self::default();
        }
        match (fields[0].parse(), fields[1].parse(), fields[2].parse()) {
            (Ok(chapter), Ok(block), Ok(offset)) => Self {
                chapter,
                block,
                offset,
            },
            _ => Self::default(),
        }
    }
}

impl State {
    pub async fn open(data_dir: PathBuf, allowed_hosts: Vec<String>) -> topcoat::Result<Self> {
        let path = data_dir.clone();
        let (db, directory_lock) = tokio::task::spawn_blocking(move || -> Result<_, String> {
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            let lock = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(path.join("web-server.lock"))
                .map_err(|e| e.to_string())?;
            lock.try_lock()
                .map_err(|_| "该 Web 数据目录已有服务在使用".to_owned())?;
            let uploads = path.join("uploads");
            std::fs::create_dir_all(&uploads).map_err(|e| e.to_string())?;
            // With the directory lock held, these cannot belong to a live
            // receiver. Recover only our own abandoned temporary uploads.
            for entry in std::fs::read_dir(&uploads).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                if !entry.file_name().to_string_lossy().starts_with("readwise-") {
                    continue;
                }
                let kind = entry.file_type().map_err(|e| e.to_string())?;
                if kind.is_dir() {
                    std::fs::remove_dir_all(entry.path()).map_err(|e| e.to_string())?;
                } else if kind.is_file()
                    && readwise_core::books::supported_format(&entry.path()).is_some()
                {
                    std::fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
                }
            }
            Ok((Db::open_in(path)?, Arc::new(lock)))
        })
        .await?
        .map_err(topcoat::Error::msg)?;
        let storage = crate::reading::open_storage(&data_dir).await?;
        Ok(Self {
            storage,
            ai_jobs: Arc::new(Semaphore::new(2)),
            inner: Arc::new(Mutex::new(Library {
                db: BlockingDb(Some(db)),
                cache: VecDeque::new(),
                math_cache: VecDeque::new(),
            })),
            jobs: Arc::new(Semaphore::new(4)),
            data_dir,
            allowed_hosts,
            _directory_lock: directory_lock,
        })
    }

    pub async fn call<T, F>(&self, work: F) -> topcoat::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Library) -> Result<T, String> + Send + 'static,
    {
        let permit = self.jobs.clone().acquire_owned().await?;
        let inner = self.inner.clone();
        let directory_lock = self._directory_lock.clone();
        tokio::task::spawn_blocking(move || {
            let _directory_lock = directory_lock;
            let _permit = permit;
            let mut library = inner
                .lock()
                .map_err(|_| "数据库工作线程不可用".to_owned())?;
            work(&mut library)
        })
        .await?
        .map_err(topcoat::Error::msg)
    }
}

impl Library {
    pub fn book(&self, id: &str) -> Result<Option<BookShelfItem>, String> {
        Ok(self
            .db
            .get_shelf_items()?
            .into_iter()
            .find(|book| book.id == id))
    }

    pub fn delete_book(&mut self, id: &str) -> Result<(), String> {
        self.db.delete_book(id)?;
        self.cache.retain(|(book_id, _)| book_id != id);
        self.math_cache.retain(|(book_id, _, _)| book_id != id);
        Ok(())
    }

    pub fn content(&mut self, book: &BookShelfItem) -> Result<Arc<BookContent>, String> {
        if let Some(index) = self.cache.iter().position(|(id, _)| id == &book.id) {
            let entry = self.cache.remove(index).unwrap();
            let content = entry.1.clone();
            self.cache.push_back(entry);
            return Ok(content);
        }
        let path = Path::new(&book.file_abs_path);
        let loaded = self.db.load_book_chapters(&book.id, path)?;
        let content = Arc::new(content::prepare(
            loaded,
            &book.id,
            path.parent().ok_or("书籍路径无效")?,
        ));
        if content.bytes <= CACHE_BYTES {
            while self.cache.len() >= 3
                || self.cache.iter().map(|(_, c)| c.bytes).sum::<usize>() + content.bytes
                    > CACHE_BYTES
            {
                self.cache.pop_front();
            }
            self.cache.push_back((book.id.clone(), content.clone()));
        }
        Ok(content)
    }

    #[cfg(test)]
    pub fn reader(&mut self, id: &str, chapter: usize) -> Result<Option<ReaderPage>, String> {
        let preferences = Preferences::load(&self.db)?;
        self.reader_with_preferences(id, chapter, preferences)
    }

    pub fn reader_with_preferences(
        &mut self,
        id: &str,
        chapter: usize,
        preferences: Preferences,
    ) -> Result<Option<ReaderPage>, String> {
        let Some(book) = self.book(id)? else {
            return Ok(None);
        };
        let content = self.content(&book)?;
        let Some(selected) = content.chapters.get(chapter) else {
            return Ok(None);
        };
        let math = if let Some(index) = self
            .math_cache
            .iter()
            .position(|(id, page, _)| id == &book.id && *page == chapter)
        {
            let entry = self.math_cache.remove(index).unwrap();
            let math = entry.2.clone();
            self.math_cache.push_back(entry);
            math
        } else {
            let math = Arc::new(crate::math::MathPage::pdf(
                Path::new(&book.file_abs_path),
                chapter,
                &selected.blocks,
                &selected.text,
            ));
            while self.math_cache.len() >= 6
                || self
                    .math_cache
                    .iter()
                    .map(|(_, _, m)| m.bytes)
                    .sum::<usize>()
                    + math.bytes
                    > 8 * 1024 * 1024
            {
                if self.math_cache.pop_front().is_none() {
                    break;
                }
            }
            if math.bytes <= 8 * 1024 * 1024 {
                self.math_cache
                    .push_back((book.id.clone(), chapter, math.clone()));
            }
            math
        };
        let (lexicon, known) = self.db.gloss_context(Some(id))?;
        let glosses = lexicon
            .select(EnglishLevel::from_str(&preferences.english_level))
            .into_iter()
            .map(|(lemma, gloss)| (lemma, content::escape(&gloss)))
            .collect();
        let blocks = selected
            .blocks
            .iter()
            .enumerate()
            .map(|(index, block)| {
                math.annotate(index, block, |html| {
                    crate::content::annotate(html, &glosses, &known)
                })
            })
            .collect();
        let mut resume = self
            .db
            .get_status(id)?
            .map(|status| Location::decode(&status.location))
            .unwrap_or_default();
        if resume.chapter != chapter || resume.block >= selected.blocks.len() {
            resume = Location {
                chapter,
                ..Default::default()
            };
        }
        Ok(Some(ReaderPage {
            book,
            title: selected.title.clone(),
            chapter,
            total: content.chapters.len(),
            blocks,
            toc: content
                .toc
                .iter()
                .map(|entry| {
                    (
                        entry.title.clone(),
                        entry.level,
                        entry.chapter,
                        entry.fragment.clone(),
                    )
                })
                .collect(),
            preferences,
            resume,
        }))
    }

    pub fn save_excerpt(
        &mut self,
        id: &str,
        chapter: usize,
        text: &str,
        note: &str,
    ) -> Result<String, String> {
        let book = self.book(id)?.ok_or("书籍不存在")?;
        let content = self.content(&book)?;
        let chapter = content.chapters.get(chapter).ok_or("章节不存在")?;
        let text = if note.trim().is_empty() {
            format!("{}\n\n—— {}", text.trim(), chapter.title)
        } else {
            format!("{}\n\n{}\n\n—— {}", text.trim(), note.trim(), chapter.title)
        };
        Ok(self
            .db
            .create_note(&book.id, &book.title, &book.author, &text)?
            .id)
    }

    pub fn progress(&mut self, id: &str, location: Location) -> Result<(), String> {
        let book = self.book(id)?.ok_or("书籍不存在")?;
        let content = self.content(&book)?;
        let chapter = content.chapters.get(location.chapter).ok_or("章节不存在")?;
        if location.block >= chapter.blocks.len() || location.offset > 10_000_000 {
            return Err("阅读锚点无效".into());
        }
        self.db.update_progress(
            id,
            location.chapter as i64 + 1,
            content.chapters.len() as i64,
            &location.encode(),
        )
    }

    pub fn add_vocabulary(&self, input: &VocabBookAddInput) -> Result<bool, String> {
        validate_word(&input.lemma)?;
        if input.word.len() > 200 || input.gloss.as_ref().is_some_and(|g| g.len() > 2000) {
            return Err("词汇内容过长".into());
        }
        if let Some(id) = &input.book_id
            && self.book(id)?.is_none()
        {
            return Err("书籍不存在".into());
        }
        self.db.vocab_book_add(input)
    }
}

pub fn validate_word(word: &str) -> Result<String, String> {
    if word.len() < 2
        || word.len() > 80
        || !word.starts_with(|ch: char| ch.is_ascii_alphabetic())
        || word.chars().filter(|ch| ch.is_ascii_alphabetic()).count() < 2
        || !word
            .chars()
            .all(|c| c.is_ascii_alphabetic() || matches!(c, '\'' | '’' | '-'))
    {
        return Err("词形无效".into());
    }
    Ok(lemmatize(word))
}

pub fn validate_epub(path: &Path) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| "不是有效的 EPUB 文件".to_owned())?;
    if archive.len() > 20_000 {
        return Err("EPUB 内部文件过多".into());
    }
    let mut total = 0u64;
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| e.to_string())?;
        total = total.checked_add(entry.size()).ok_or("EPUB 解压大小超限")?;
        if entry.size() > 64 * 1024 * 1024 || total > 512 * 1024 * 1024 {
            return Err("EPUB 解压大小超限（单项 64 MB，总计 512 MB）".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations_are_source_anchors_not_scroll_pixels_or_native_coordinates() {
        let value = Location {
            chapter: 2,
            block: 15,
            offset: 231,
        };
        assert_eq!(Location::decode(&value.encode()), value);
        for value in [
            "v3:2:15:231",
            "web:v1:0:0:NaN",
            "web:v1:0:0:1:2",
            "web:v1:-1:0:0",
        ] {
            assert_eq!(Location::decode(value), Location::default());
        }
    }

    #[test]
    fn mutation_words_are_bounded_and_normalized() {
        assert_eq!(validate_word("forests").unwrap(), "forest");
        for word in [
            "<script>",
            "a",
            "two words",
            "../../etc/passwd",
            "---",
            "'s",
        ] {
            assert!(validate_word(word).is_err());
        }
    }
}
