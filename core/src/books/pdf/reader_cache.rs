//! A disposable, DB-authenticated checkpoint of already validated coordinates.
//! Like a presentation cache, it is never a new source of book text. Any changed
//! dependency falls back to the complete frozen migration/validation pipeline.
use super::super::ORIGINAL_MARKER;
use super::*;
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::PathBuf,
};

pub(super) const FILE: &str = "pdf-reader-v1.json";
const PROOF: &str = "pdf_reader_checkpoint_v1";
// Bump this and update dependencies/markers whenever the reader pipeline changes.
const VERSION: u32 = 1;
const MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct FileStamp {
    length: u64,
    modified: (i64, i64),
    changed: (i64, i64),
    device: u64,
    inode: u64,
}

fn stamp(path: &Path) -> Option<FileStamp> {
    // mtime+length alone misses same-length edits with restored mtime. Unix
    // ctime and file identity cover those edits/replacements, including Android.
    // On platforms without that guarantee, retain the complete validation path.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = fs::metadata(path).ok()?;
        m.is_file().then(|| FileStamp {
            length: m.len(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
            device: m.dev(),
            inode: m.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Dependencies(Vec<Option<FileStamp>>);
impl Dependencies {
    pub(super) fn capture(path: &Path) -> Option<Self> {
        let root = path.parent()?;
        let source = stamp(path)?;
        let mut files = vec![Some(source)];
        for name in [
            super::SNAPSHOT_FILE,
            super::FILE,
            CONTENT_FILE,
            STRUCTURE_FILE,
            LIST_FILE,
            TOC_FILE,
            reflow_migration::REFLOW_FILE,
            reflow_v8_migration::REFLOW_FILE,
        ] {
            let p = root.join(name);
            // Missing files are part of the identity too: adding a previously
            // absent legacy projection must invalidate the checkpoint.
            files.push(if p.try_exists().ok()? {
                Some(stamp(&p)?)
            } else {
                None
            });
        }
        Some(Self(files))
    }
}

/// A lease for already-validated *in-memory* text. It does not deserialize or
/// trust the disposable checkpoint. Recheck source/projection identities and
/// committed coordinate pins before reusing a native prepared reader.
fn asset_identity(directory: &Path) -> Option<Option<Vec<(PathBuf, FileStamp)>>> {
    if !directory.try_exists().ok()? {
        return Some(None);
    }
    let started = std::time::Instant::now();
    let mut files = Vec::new();
    for entry in fs::read_dir(directory).ok()? {
        // Do not turn a fast UI lease check into an unbounded directory walk.
        // Large/busy asset sets simply retain the normal background reader path.
        if files.len() >= 256 || started.elapsed() > std::time::Duration::from_millis(2) {
            return None;
        }
        let entry = entry.ok()?;
        files.push((PathBuf::from(entry.file_name()), stamp(&entry.path())?));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Some(Some(files))
}

pub(crate) async fn reader_identity(pool: &SqlitePool, id: &str, path: &Path) -> Option<String> {
    let canonical = path.canonicalize().ok()?;
    // Bound parsing on the native event thread; large unrelated metadata only
    // disables reuse, never rejects the normal full-validation reader path.
    let raw: Option<Option<String>> = sqlx::query_scalar(
        "SELECT metadata FROM book_status WHERE book_id = ? AND (metadata IS NULL OR length(metadata) <= 65536)")
        .bind(id).fetch_optional(pool).await.ok()?;
    let raw = raw?;
    let metadata = match raw.as_deref().filter(|s| !s.trim().is_empty()) {
        None => serde_json::json!({}),
        Some(s) if s.len() <= 65536 => match serde_json::from_str::<serde_json::Value>(s).ok()? {
            serde_json::Value::Null => serde_json::json!({}),
            v if v.is_object() => v,
            _ => return None,
        },
        _ => return None,
    };
    let files = if path
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("pdf"))
    {
        serde_json::to_value(Dependencies::capture(path)?).ok()?
    } else {
        serde_json::to_value(stamp(path)?).ok()?
    };
    let root = path.parent()?;
    let assets = asset_identity(
        &root.join(
            if path
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("pdf"))
            {
                "pdf-assets-v3"
            } else {
                "res"
            },
        ),
    )?;
    serde_json::to_string(&(id, canonical, files, assets, markers(&metadata))).ok()
}

#[derive(Serialize, Deserialize)]
struct Record {
    version: u32,
    book: String,
    path: PathBuf,
    dependencies: Dependencies,
    markers: BTreeMap<String, serde_json::Value>,
    snapshot: Snapshot,
}

async fn metadata_value(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<(Option<String>, serde_json::Value)>, String> {
    let row: Option<Option<String>> =
        sqlx::query_scalar("SELECT metadata FROM book_status WHERE book_id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;
    let Some(raw) = row else {
        return Ok(None);
    };
    let invalid = || "PDF 阅读坐标元数据损坏，原标注保持不变".to_string();
    let value = match raw.as_deref().filter(|s| !s.trim().is_empty()) {
        None => serde_json::json!({}),
        Some(s) => match serde_json::from_str(s).map_err(|_| invalid())? {
            serde_json::Value::Null => serde_json::json!({}),
            v if v.is_object() => v,
            _ => return Err(invalid()),
        },
    };
    Ok(Some((raw, value)))
}

// Import prepares the original snapshot before committing book_status. Never
// regenerate it for a committed book, including a native v8 import without any
// historical projection markers. A separate DB pin survives cache deletion.
pub(super) async fn validate_original(
    pool: &SqlitePool,
    id: &str,
    path: &Path,
    source: &ReaderSource<'_>,
) -> Result<(), String> {
    let Some((_, meta)) = metadata_value(pool, id).await? else {
        return Ok(());
    };
    if !path
        .parent()
        .ok_or("PDF 目录无效")?
        .join(SNAPSHOT_FILE)
        .is_file()
    {
        return Err("PDF 原始文本快照缺失，未重建，以避免已有标注错位".into());
    }
    if let Some(expected) = meta.get(ORIGINAL_MARKER) {
        let original = &source.original()?.1;
        let token = fingerprint(&serde_json::to_vec(original).map_err(|e| e.to_string())?);
        if expected.as_str() != Some(&token) {
            return Err("PDF 原始文本快照已被替换，原标注保持不变".into());
        }
    }
    Ok(())
}

fn markers(value: &serde_json::Value) -> BTreeMap<String, serde_json::Value> {
    [
        ORIGINAL_MARKER,
        MARKER,
        CONTENT_MARKER,
        STRUCTURE_MARKER,
        LIST_MARKER,
        TOC_MARKER,
        reflow_migration::REFLOW_MARKER,
        reflow_v8_migration::REFLOW_MARKER,
    ]
    .into_iter()
    .filter_map(|key| value.get(key).map(|v| (key.into(), v.clone())))
    .collect()
}

pub(super) async fn load(
    pool: &SqlitePool,
    id: &str,
    path: &Path,
    dependencies: &Dependencies,
) -> Option<LoadedChapters> {
    let (_, metadata) = metadata_value(pool, id).await.ok()??;
    metadata.get(ORIGINAL_MARKER)?.as_str()?;
    let proof = metadata.get(PROOF)?.as_str()?;
    let file = fs::File::open(path.parent()?.join(FILE)).ok()?;
    if file.metadata().ok()?.len() > MAX_BYTES as u64 {
        return None;
    }
    let mut data = Vec::new();
    file.take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut data)
        .ok()?;
    if data.len() > MAX_BYTES || fingerprint(&data) != proof {
        return None;
    }
    let record: Record = serde_json::from_slice(&data).ok()?;
    if record.version != VERSION
        || record.book != id
        || record.path != path.canonicalize().ok()?
        || &record.dependencies != dependencies
        || record.markers != markers(&metadata)
        || record.snapshot.version != super::super::SNAPSHOT_VERSION
        || record.snapshot.extractor != super::super::EXTENDED_REFLOW_EXTRACTOR
        || record.snapshot.pages.is_empty()
        || record
            .snapshot
            .toc
            .iter()
            .any(|t| t.page >= record.snapshot.pages.len())
        || Dependencies::capture(path).as_ref() != Some(dependencies)
    {
        return None;
    }
    let loaded = record.snapshot.into_chapters();
    super::super::content::ensure_assets(path, &loaded).ok()?;
    // Asset recovery can take longer than an ordinary hit. Recheck both file
    // identities and committed coordinates before publishing its result.
    let (_, current) = metadata_value(pool, id).await.ok()??;
    if Dependencies::capture(path).as_ref() != Some(dependencies)
        || current.get(PROOF).and_then(|v| v.as_str()) != Some(proof)
        || markers(&current) != record.markers
    {
        return None;
    }
    Some(super::super::content::resolve_assets(loaded, path))
}

struct Bounded(Vec<u8>);
impl Write for Bounded {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if data.len() > MAX_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other("PDF reader checkpoint too large"));
        }
        self.0.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(super) async fn save(
    pool: &SqlitePool,
    id: &str,
    path: &Path,
    before: &Dependencies,
    original: &Snapshot,
    loaded: &LoadedChapters,
) -> Result<(), String> {
    if Dependencies::capture(path).as_ref() != Some(before) {
        // The migration created a dependency, or another writer changed one.
        // A subsequent stable validation can publish a checkpoint; never guess.
        return Ok(());
    }
    if loaded.chapters.iter().map(|c| c.html.len()).sum::<usize>() > MAX_BYTES
        || loaded.chapters.iter().enumerate().any(|(i, c)| {
            !c.figure_targets.is_empty()
                || c.source_path != format!("pdf-page-{i}")
                || c.title != format!("PDF {}", i + 1)
        })
        || loaded.toc.iter().any(|t| t.fragment.is_some())
    {
        return Ok(());
    }
    let (raw, mut meta) = metadata_value(pool, id)
        .await?
        .ok_or("PDF checkpoint metadata unavailable")?;
    let original_token = fingerprint(&serde_json::to_vec(original).map_err(|e| e.to_string())?);
    if meta
        .get(ORIGINAL_MARKER)
        .is_some_and(|v| v.as_str() != Some(&original_token))
    {
        return Err("PDF 原始文本快照已被替换，原标注保持不变".into());
    }
    meta[ORIGINAL_MARKER] = original_token.into();
    let root = path.parent().ok_or("PDF directory unavailable")?;
    let assets = super::super::content::escape(&root.join("pdf-assets-v3").to_string_lossy());
    let record = Record {
        version: VERSION,
        book: id.into(),
        path: path.canonicalize().map_err(|e| e.to_string())?,
        dependencies: before.clone(),
        markers: markers(&meta),
        snapshot: Snapshot {
            version: original.version,
            fingerprint: original.fingerprint.clone(),
            extractor: super::super::EXTENDED_REFLOW_EXTRACTOR.into(),
            pages: loaded
                .chapters
                .iter()
                .map(|c| {
                    c.html
                        .replace(&format!("src=\"{assets}/"), "src=\"pdf-assets-v3/")
                })
                .collect(),
            toc: loaded
                .toc
                .iter()
                .map(|t| super::super::Navigation {
                    title: t.title.clone(),
                    level: t.level,
                    page: t.chapter,
                })
                .collect(),
        },
    };
    let mut data = Bounded(Vec::new());
    serde_json::to_writer(&mut data, &record).map_err(|e| e.to_string())?;
    let token = fingerprint(&data.0);
    if Dependencies::capture(path).as_ref() != Some(before) {
        return Ok(());
    }
    let temporary = root.join(format!(".pdf-reader-{}.tmp", uuid::Uuid::new_v4()));
    let write = fs::write(&temporary, data.0).and_then(|_| fs::rename(&temporary, root.join(FILE)));
    if write.is_err() {
        let _ = fs::remove_file(&temporary);
    } else {
        meta[PROOF] = token.into();
    }
    // Authenticate the disposable payload in the same DB that owns coordinates.
    // CAS preserves concurrent want_to_read/settings changes, progress and notes.
    sqlx::query("UPDATE book_status SET metadata = ? WHERE book_id = ? AND metadata IS ?")
        .bind(meta.to_string())
        .bind(id)
        .bind(raw)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    write.map_err(|e| e.to_string())
}
