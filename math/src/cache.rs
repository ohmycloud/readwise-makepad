//! Bounded, disposable per-page presentation, not another coordinate system.
//! Bump VERSION when glyph mapping, Typst output, or figure recovery changes.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

const VERSION: u32 = 1;
const MAX_RECORD: usize = 16 * 1024 * 1024;
const MAX_PAGE: usize = 8 * 1024 * 1024;
const MAX_FILES: usize = 32;
const MAX_DISK: u64 = 64 * 1024 * 1024;

fn digest(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn identity(path: &Path) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = fs::metadata(path).ok()?;
        if !m.is_file() {
            return None;
        }
        Some(format!(
            "{:?}:{}:{}:{}:{}:{}:{}:{}",
            path.canonicalize().ok()?,
            m.dev(),
            m.ino(),
            m.len(),
            m.mtime(),
            m.mtime_nsec(),
            m.ctime(),
            m.ctime_nsec()
        ))
    }
    // Do not silently weaken identity to mtime+size on other platforms.
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

#[derive(Clone)]
pub(super) struct PageCache {
    pdf: PathBuf,
    identity: String,
    directory: PathBuf,
}
#[derive(Serialize, Deserialize)]
struct Record {
    version: u32,
    key: String,
    checksum: String,
    // Keep exact serialized bytes for the checksum. HashMap iteration order
    // must not turn a successful deserialize into a false cache miss.
    payload: String,
}
impl PageCache {
    pub(super) fn new(pdf: &Path, directory: PathBuf) -> Option<Self> {
        pdf.extension()?.eq_ignore_ascii_case("pdf").then_some(())?;
        Some(Self {
            pdf: pdf.into(),
            identity: identity(pdf)?,
            directory,
        })
    }
    fn key(&self, page: usize, blocks: &[String], text: &[String]) -> Option<String> {
        if blocks.len() != text.len()
            || blocks.len() > 1024
            || text.iter().map(String::len).sum::<usize>() > 512 * 1024
            || blocks.iter().map(String::len).sum::<usize>() > 2 * 1024 * 1024
            || identity(&self.pdf).as_deref() != Some(self.identity.as_str())
        {
            return None;
        }
        // Includes wrappers/block boundaries, not just flattened words. No
        // annotation, theme, viewport or transient reader request enters here.
        Some(digest(
            &serde_json::to_vec(&(VERSION, &self.identity, page, blocks, text)).ok()?,
        ))
    }
    fn path(&self, key: &str) -> PathBuf {
        self.directory.join(format!("page-v{VERSION}-{key}.json"))
    }
    pub(super) fn read(&self, page: usize, blocks: &[String], text: &[String]) -> Option<MathPage> {
        let key = self.key(page, blocks, text)?;
        let mut file = fs::File::open(self.path(&key)).ok()?;
        if file.metadata().ok()?.len() > MAX_RECORD as u64 {
            return None;
        }
        let mut data = Vec::new();
        (&mut file)
            .take(MAX_RECORD as u64 + 1)
            .read_to_end(&mut data)
            .ok()?;
        if data.len() > MAX_RECORD {
            return None;
        }
        let record: Record = serde_json::from_slice(&data).ok()?;
        if record.version != VERSION
            || record.key != key
            || record.checksum != digest(record.payload.as_bytes())
        {
            return None;
        }
        let mut result: MathPage = serde_json::from_str(&record.payload).ok()?;
        if !valid(&mut result, blocks, text)
            || self.key(page, blocks, text).as_deref() != Some(&key)
        {
            return None;
        }
        let _ = file.set_modified(SystemTime::now()); // LRU, not an immutable book file.
        Some(result)
    }
    pub(super) fn write(
        &self,
        page: usize,
        blocks: &[String],
        text: &[String],
        result: &MathPage,
    ) -> std::io::Result<()> {
        if result.bytes > MAX_PAGE {
            return Ok(());
        }
        let Some(key) = self.key(page, blocks, text) else {
            return Ok(());
        };
        let mut payload = Limited(Vec::new());
        serde_json::to_writer(&mut payload, result)?;
        let payload = String::from_utf8(payload.0).map_err(std::io::Error::other)?;
        let mut data = Limited(Vec::new());
        serde_json::to_writer(
            &mut data,
            &Record {
                version: VERSION,
                key: key.clone(),
                checksum: digest(payload.as_bytes()),
                payload,
            },
        )?;
        if self.key(page, blocks, text).as_deref() != Some(&key) {
            return Ok(());
        }
        fs::create_dir_all(&self.directory)?;
        let mut file = tempfile::NamedTempFile::new_in(&self.directory)?;
        file.write_all(&data.0)?;
        file.persist(self.path(&key)).map_err(|e| e.error)?;
        prune(&self.directory, MAX_FILES, MAX_DISK);
        Ok(())
    }
}
struct Limited(Vec<u8>);
impl Write for Limited {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if data.len() > MAX_RECORD.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other(
                "PDF presentation exceeds cache budget",
            ));
        }
        self.0.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn valid(page: &mut MathPage, blocks: &[String], texts: &[String]) -> bool {
    if blocks
        .iter()
        .zip(texts)
        .any(|(b, t)| text_positions(b).0 != *t)
    {
        return false;
    }
    for (&block, fragments) in &page.fragments {
        let Some(text) = texts.get(block) else {
            return false;
        };
        let mut end = 0;
        for f in fragments {
            if f.start < end
                || f.end <= f.start
                || !text.is_char_boundary(f.start)
                || !text.is_char_boundary(f.end)
                || f.end > text.len()
                || content::plain_text(&f.html) != text[f.start..f.end]
            {
                return false;
            }
            match &f.projection {
                Projection::Formula(p) => {
                    if p.svg.len() > 1024 * 1024
                        || !p.svg.starts_with("<svg ")
                        || ![p.width_em, p.height_em, p.align_em]
                            .iter()
                            .all(|v| v.is_finite())
                        || !(0.0..=4096.).contains(&p.width_em)
                        || !(0.0..=1024.).contains(&p.height_em)
                        || p.align_em.abs() > 1024.
                    {
                        return false;
                    }
                }
                Projection::Figure(p) => {
                    if p.width == 0
                        || p.height == 0
                        || p.width > 1800
                        || p.height > 1800
                        || p.png.len() > 4 * 1024 * 1024
                        || !p.png.starts_with(b"\x89PNG\r\n\x1a\n")
                    {
                        return false;
                    }
                }
                Projection::Hidden { owner } => {
                    if owner.is_empty() || owner.len() > 256 {
                        return false;
                    }
                }
            }
            end = f.end;
        }
    }
    page.bytes = page.fragments.values().flatten().map(Fragment::bytes).sum();
    page.bytes <= MAX_PAGE
}
fn prune(root: &Path, max_files: usize, max_bytes: u64) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut files = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let name = name.to_str()?;
            if !name.starts_with("page-v") || !name.ends_with(".json") {
                return None;
            }
            let m = e.metadata().ok()?;
            if !m.is_file() {
                return None;
            }
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|f| f.0);
    let mut bytes = files.iter().fold(0u64, |sum, f| sum.saturating_add(f.1));
    let mut count = files.len();
    for (_, size, path) in files {
        if count <= max_files && bytes <= max_bytes {
            break;
        }
        if fs::remove_file(path).is_ok() {
            count -= 1;
            bytes = bytes.saturating_sub(size);
        }
    }
}
pub(super) mod png {
    use super::*;
    use base64::{Engine, engine::general_purpose::STANDARD};
    pub fn serialize<S: serde::Serializer>(
        data: &std::sync::Arc<Vec<u8>>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(data.as_slice()))
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> Result<std::sync::Arc<Vec<u8>>, D::Error> {
        let data = String::deserialize(d)?;
        STANDARD
            .decode(data)
            .map(std::sync::Arc::new)
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(all(test, unix))]
#[path = "cache_tests.rs"]
mod tests;
