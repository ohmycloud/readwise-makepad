//! One-time, transactional migration of legacy span-split heading coordinates.
//! Keep the original snapshot; publish a separate projection BEFORE committing
//! its DB marker. A crash/failure before commit leaves all old coordinates valid.
use super::{
    fingerprint,
    headings::{repair, BlockTarget},
    load_imported_pages, open, read_pdf, LoadedChapters, Snapshot, EXTRACTOR, LEGACY_EXTRACTOR,
    SNAPSHOT_FILE,
};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use std::path::Path;

const FILE: &str = "pdf-headings-v2.json";
const MARKER: &str = "pdf_heading_projection_v2";
const PACK: i64 = 1_000_000;

#[cfg(test)]
#[path = "migration_tests.rs"]
mod tests;

#[derive(Serialize, Deserialize)]
struct Projection {
    source_snapshot: String,
    snapshot: Snapshot,
    maps: Vec<Vec<BlockTarget>>,
}

impl Projection {
    fn validate(&self, original: &Snapshot) -> Result<(), String> {
        let invalid = || "PDF 标题快照的文本坐标不一致，原标注保持不变".to_string();
        if self.snapshot.version != original.version
            || self.snapshot.pages.len() != original.pages.len()
            || self.maps.len() != original.pages.len()
        {
            return Err(invalid());
        }
        for ((old, new), map) in original
            .pages
            .iter()
            .zip(&self.snapshot.pages)
            .zip(&self.maps)
        {
            let old = super::super::epub_meta::split_blocks(old);
            let new = super::super::epub_meta::split_blocks(new);
            if old.len() != map.len() {
                return Err(invalid());
            }
            for (block, target) in old.iter().zip(map) {
                let text = crate::vocab::annotate::rendered_text(block);
                let new_text = new
                    .get(target.block)
                    .map(|b| crate::vocab::annotate::rendered_text(b))
                    .ok_or_else(invalid)?;
                let end = target
                    .prefix
                    .checked_add(target.old_len)
                    .ok_or_else(invalid)?;
                if text.len() != target.old_len {
                    return Err(invalid());
                }
                if let Some(offsets) = &target.offsets {
                    if offsets.len() != text.len() + 1
                        || offsets.first().copied() != Some(target.prefix)
                        || offsets.windows(2).any(|w| w[0] >= w[1])
                        || offsets.last().is_none_or(|p| *p > new_text.len())
                    {
                        return Err(invalid());
                    }
                    for (i, byte) in text.bytes().enumerate() {
                        if new_text.as_bytes().get(offsets[i]) != Some(&byte) {
                            return Err(invalid());
                        }
                        if text.is_char_boundary(i) && !new_text.is_char_boundary(offsets[i]) {
                            return Err(invalid());
                        }
                    }
                    if offsets
                        .last()
                        .is_some_and(|p| !new_text.is_char_boundary(*p))
                    {
                        return Err(invalid());
                    }
                } else if new_text.get(target.prefix..end) != Some(text.as_str()) {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }

    fn point(&self, chapter: usize, block: usize, byte: usize) -> Result<(usize, usize), String> {
        let target = self
            .maps
            .get(chapter)
            .and_then(|m| m.get(block))
            .ok_or_else(|| {
                format!("PDF 标题修复无法迁移位置 {chapter}:{block}:{byte}，原数据保持不变")
            })?;
        let byte = byte.min(target.old_len);
        let offset = target
            .offsets
            .as_ref()
            .map_or(target.prefix + byte, |map| map[byte]);
        Ok((target.block, offset))
    }

    fn location(&self, location: &str) -> Result<String, String> {
        let parts: Vec<_> = location.split(':').collect();
        if parts.len() != 4 || !matches!(parts[0], "v2" | "v3") {
            return Ok(location.into());
        }
        let (Ok(chapter), Ok(block)) = (parts[1].parse::<usize>(), parts[2].parse::<usize>())
        else {
            return Ok(location.into());
        };
        if parts[0] == "v3" {
            let Ok(byte) = parts[3].parse::<usize>() else {
                return Ok(location.into());
            };
            let (block, byte) = self.point(chapter, block, byte)?;
            Ok(format!("v3:{chapter}:{block}:{byte}"))
        } else {
            let Ok(scroll) = parts[3].parse::<f64>() else {
                return Ok(location.into());
            };
            if !scroll.is_finite() {
                return Ok(location.into());
            }
            let (new_block, prefix) = self.point(chapter, block, 0)?;
            let new_blocks = super::super::epub_meta::split_blocks(&self.snapshot.pages[chapter]);
            let length = crate::vocab::annotate::rendered_text(&new_blocks[new_block]).len();
            let target = &self.maps[chapter][block];
            if prefix == 0
                && length == target.old_len
                && target.offsets.is_none()
                && !target.layout_changed
            {
                // An unchanged body paragraph only moved to a different block
                // index: its pixel offset is still meaningful.
                Ok(format!("v2:{chapter}:{new_block}:{}", parts[3]))
            } else {
                // A pixel offset inside an old heading is not meaningful in a
                // joined one. Preserve the beginning of that exact text run.
                Ok(format!("v3:{chapter}:{new_block}:{prefix}"))
            }
        }
    }
}

fn prepare(path: &Path, data: &[u8], mut snapshot: Snapshot) -> Result<Projection, String> {
    let directory = path.parent().ok_or("PDF 目录无效")?;
    let source_snapshot = fingerprint(data);
    let cache = directory.join(FILE);
    match std::fs::read(&cache) {
        Ok(data) => {
            let projection: Projection = serde_json::from_slice(&data)
                .map_err(|e| format!("PDF 标题快照损坏，原标注保持不变: {e}"))?;
            if projection.source_snapshot != source_snapshot
                || projection.snapshot.fingerprint != snapshot.fingerprint
            {
                return Err("PDF 标题快照与原快照不一致，原标注保持不变".into());
            }
            projection.validate(&snapshot)?;
            return Ok(projection);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("读取 PDF 标题快照失败: {e}")),
    }
    let doc = open(read_pdf(path)?)?;
    let mut maps = Vec::new();
    for (index, html) in snapshot.pages.iter_mut().enumerate() {
        let spans = if html.contains("<h") {
            doc.extract_spans(index)
                .map_err(|e| format!("读取 PDF 行坐标失败: {e}"))?
        } else {
            Vec::new()
        };
        let (repaired, mapping) = repair(html, &spans);
        *html = repaired;
        maps.push(mapping);
    }
    snapshot.extractor = EXTRACTOR.into();
    let projection = Projection {
        source_snapshot,
        snapshot,
        maps,
    };
    let data =
        serde_json::to_vec(&projection).map_err(|e| format!("编码 PDF 标题快照失败: {e}"))?;
    let tmp = directory.join(format!(".pdf-heading-{}.tmp", uuid::Uuid::new_v4()));
    if let Err(e) = std::fs::write(&tmp, data).and_then(|_| std::fs::rename(&tmp, &cache)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("保存 PDF 标题快照失败: {e}"));
    }
    Ok(projection)
}

async fn load_heading_pages(
    pool: &SqlitePool,
    book_id: &str,
    path: &Path,
) -> Result<LoadedChapters, String> {
    // A committed projection still depends on its ORIGINAL source coordinates.
    // Do not silently regenerate that source if a cache-cleaner removed it.
    let metadata: Option<String> =
        sqlx::query_scalar("SELECT metadata FROM book_status WHERE book_id = ?")
            .bind(book_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?
            .flatten();
    let committed = metadata
        .as_deref()
        .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
        .is_some_and(|m| m.get(MARKER).is_some());
    let source_path = path.parent().ok_or("PDF 目录无效")?.join(SNAPSHOT_FILE);
    if committed && !source_path.is_file() {
        return Err("PDF 原始文本快照缺失，未重建，以避免已有标注错位".into());
    }
    // Validate original-file identity and snapshot schema before any migration.
    let original = load_imported_pages(path)?;
    let data = std::fs::read(path.parent().ok_or("PDF 目录无效")?.join(SNAPSHOT_FILE))
        .map_err(|e| e.to_string())?;
    let snapshot: Snapshot = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
    if snapshot.extractor != LEGACY_EXTRACTOR {
        if committed {
            return Err("PDF 原始文本快照已被替换，原标注保持不变".into());
        }
        return Ok(original);
    }
    let projection = prepare(path, &data, snapshot)?;
    apply_projection(pool, book_id, projection, MARKER).await
}

async fn apply_projection(
    pool: &SqlitePool,
    book_id: &str,
    projection: Projection,
    marker: &str,
) -> Result<LoadedChapters, String> {
    let token = fingerprint(&serde_json::to_vec(&projection).map_err(|e| e.to_string())?);
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    // Acquire the SQLite write lock BEFORE reading the marker, so concurrent
    // opens cannot both rebase the same stored coordinates.
    let locked = sqlx::query("UPDATE book_status SET metadata = metadata WHERE book_id = ?")
        .bind(book_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    if locked.rows_affected() != 1 {
        return Err("PDF 阅读状态不存在，未迁移标注".into());
    }
    let row = sqlx::query("SELECT metadata, location FROM book_status WHERE book_id = ?")
        .bind(book_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let raw: Option<String> = row.get("metadata");
    let mut metadata: serde_json::Value = match raw.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(s) => {
            serde_json::from_str(s).map_err(|e| format!("阅读元数据损坏，未迁移标注: {e}"))?
        }
        None => serde_json::json!({}),
    };
    if metadata.is_null() {
        metadata = serde_json::json!({});
    }
    if !metadata.is_object() {
        return Err("阅读元数据格式无效，未迁移标注".into());
    }
    match metadata.get(marker) {
        Some(serde_json::Value::String(saved)) if saved == &token => {}
        Some(_) => return Err("PDF 标题快照已变化，原标注保持不变".into()),
        None => {
            let notes = sqlx::query("SELECT id, type, style, cfi, section_id, norm_start, norm_end FROM book_notes WHERE book_id = ?")
                .bind(book_id).fetch_all(&mut *tx).await.map_err(|e| e.to_string())?;
            for note in notes {
                let cfi: String = note.get("cfi");
                let cfi = projection.location(&cfi)?;
                let is_highlight = note.get::<String, _>("type") == "annotation"
                    && note.get::<Option<String>, _>("style").as_deref() == Some("highlight");
                let chapter = is_highlight
                    .then(|| note.get::<Option<String>, _>("section_id"))
                    .flatten()
                    .and_then(|s| s.parse::<usize>().ok());
                let rebase = |value: Option<i64>, end: bool| -> Result<Option<i64>, String> {
                    match (chapter, value) {
                        (Some(chapter), Some(value)) if value >= 0 => {
                            let old_block = (value / PACK) as usize;
                            let old_byte = (value % PACK) as usize;
                            let (block, mut byte) =
                                projection.point(chapter, old_block, old_byte)?;
                            // End affinity excludes whitespace inserted before the
                            // next source glyph. Start/location affinity includes it.
                            if end
                                && old_byte > 0
                                && projection.maps[chapter][old_block].old_len > 0
                            {
                                if let Some(offsets) =
                                    projection.maps[chapter][old_block].offsets.as_ref()
                                {
                                    let old_byte =
                                        old_byte.min(projection.maps[chapter][old_block].old_len);
                                    byte = offsets[old_byte - 1] + 1;
                                }
                            }
                            if byte >= PACK as usize {
                                return Err("PDF 标题超出高亮坐标范围，原标注保持不变".into());
                            }
                            Ok(Some(block as i64 * PACK + byte as i64))
                        }
                        _ => Ok(value),
                    }
                };
                let start = rebase(note.get("norm_start"), false)?;
                let end = rebase(note.get("norm_end"), true)?;
                sqlx::query("UPDATE book_notes SET cfi = ?, norm_start = ?, norm_end = ? WHERE id = ? AND book_id = ?")
                    .bind(cfi).bind(start).bind(end).bind(note.get::<String, _>("id")).bind(book_id)
                    .execute(&mut *tx).await.map_err(|e| e.to_string())?;
            }
            let location = projection.location(&row.get::<String, _>("location"))?;
            metadata[marker] = serde_json::Value::String(token);
            // Keep other metadata (including want_to_read) and all progress,
            // timestamps, quote text, note bodies, IDs and colours untouched.
            sqlx::query("UPDATE book_status SET location = ?, metadata = ? WHERE book_id = ?")
                .bind(location)
                .bind(metadata.to_string())
                .bind(book_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    tx.commit()
        .await
        .map_err(|e| format!("提交 PDF 标题标注迁移失败: {e}"))?;
    Ok(projection.snapshot.into_chapters())
}

const CONTENT_FILE: &str = "pdf-content-v3.json";
const CONTENT_MARKER: &str = "pdf_content_projection_v3";

async fn load_v3_pages(
    pool: &SqlitePool,
    book_id: &str,
    path: &Path,
) -> Result<LoadedChapters, String> {
    let directory = path.parent().ok_or("PDF 目录无效")?;
    let metadata: Option<String> =
        sqlx::query_scalar("SELECT metadata FROM book_status WHERE book_id = ?")
            .bind(book_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?
            .flatten();
    let committed = metadata
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .is_some_and(|m| m.get(CONTENT_MARKER).is_some());
    if committed && !directory.join(SNAPSHOT_FILE).is_file() {
        return Err("PDF 原始文本快照缺失，未重建，以避免已有标注错位".into());
    }
    let base = load_heading_pages(pool, book_id, path).await?;
    let source = std::fs::read(directory.join(SNAPSHOT_FILE)).map_err(|e| e.to_string())?;
    let mut snapshot: Snapshot = serde_json::from_slice(&source).map_err(|e| e.to_string())?;
    if snapshot.extractor == super::CONTENT_EXTRACTOR {
        if committed {
            return Err("PDF 原始文本快照已被替换，原标注保持不变".into());
        }
        return Ok(super::content::resolve_assets(base, path));
    }
    if !matches!(snapshot.extractor.as_str(), LEGACY_EXTRACTOR | EXTRACTOR) {
        return Err("不支持此 PDF 文本提取版本，原标注保持不变".into());
    }
    // Headings may already have their own committed projection. Rebase from
    // that active coordinate system, never from the obsolete original blocks.
    snapshot.pages = base.chapters.into_iter().map(|c| c.html).collect();
    let source_snapshot = fingerprint(&serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?);
    let cache = directory.join(CONTENT_FILE);
    let projection = match std::fs::read(&cache) {
        Ok(data) => {
            let projection: Projection = serde_json::from_slice(&data)
                .map_err(|e| format!("PDF 内容快照损坏，原标注保持不变: {e}"))?;
            if projection.source_snapshot != source_snapshot
                || projection.snapshot.fingerprint != snapshot.fingerprint
                || projection.snapshot.extractor != super::CONTENT_EXTRACTOR
            {
                return Err("PDF 内容快照与原快照不一致，原标注保持不变".into());
            }
            projection.validate(&snapshot)?;
            projection
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let doc = open(read_pdf(path)?)?;
            let mut repaired = snapshot.clone();
            let mut maps = Vec::new();
            for (page, html) in repaired.pages.iter_mut().enumerate() {
                let (new, mapping) = super::content::repair(&doc, page, html, Some(directory))?;
                *html = new;
                maps.push(mapping);
            }
            repaired.extractor = super::CONTENT_EXTRACTOR.into();
            let projection = Projection {
                source_snapshot,
                snapshot: repaired,
                maps,
            };
            projection.validate(&snapshot)?;
            let data = serde_json::to_vec(&projection).map_err(|e| e.to_string())?;
            let tmp = directory.join(format!(".pdf-content-{}.tmp", uuid::Uuid::new_v4()));
            if let Err(e) = std::fs::write(&tmp, data).and_then(|_| std::fs::rename(&tmp, &cache)) {
                let _ = std::fs::remove_file(tmp);
                return Err(format!("保存 PDF 内容快照失败: {e}"));
            }
            projection
        }
        Err(e) => return Err(format!("读取 PDF 内容快照失败: {e}")),
    };
    // Recover derived images before publishing the DB marker. This never
    // changes text/coordinates, even after assets were removed by a cleaner.
    super::content::ensure_assets(path, &projection.snapshot.clone().into_chapters())?;
    let loaded = apply_projection(pool, book_id, projection, CONTENT_MARKER).await?;
    Ok(super::content::resolve_assets(loaded, path))
}

const STRUCTURE_FILE: &str = "pdf-structure-v4.json";
const STRUCTURE_MARKER: &str = "pdf_structure_projection_v4";

pub(crate) async fn load_reader_pages(
    pool: &SqlitePool,
    book_id: &str,
    path: &Path,
) -> Result<LoadedChapters, String> {
    let directory = path.parent().ok_or("PDF 目录无效")?;
    let metadata: Option<String> =
        sqlx::query_scalar("SELECT metadata FROM book_status WHERE book_id = ?")
            .bind(book_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?
            .flatten();
    let metadata = metadata
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
    let committed = metadata.as_ref().is_some_and(|m| {
        [MARKER, CONTENT_MARKER, STRUCTURE_MARKER]
            .iter()
            .any(|key| m.get(*key).is_some())
    });
    if committed && !directory.join(SNAPSHOT_FILE).is_file() {
        return Err("PDF 原始文本快照缺失，未重建，以避免已有标注错位".into());
    }
    let loaded = load_imported_pages(path)?;
    let bytes = std::fs::read(directory.join(SNAPSHOT_FILE)).map_err(|e| e.to_string())?;
    let mut base: Snapshot = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if base.extractor == super::STRUCTURE_EXTRACTOR {
        if committed {
            return Err("PDF 原始文本快照已被替换，原标注保持不变".into());
        }
        return Ok(loaded);
    }
    // Replay older projections unchanged before rebasing from the CURRENT v3
    // coordinate system. Their serialized tokens must remain byte-for-byte stable.
    load_v3_pages(pool, book_id, path).await?;
    if base.extractor != super::CONTENT_EXTRACTOR {
        let data = std::fs::read(directory.join(CONTENT_FILE)).map_err(|e| e.to_string())?;
        let projection: Projection = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
        base = projection.snapshot;
    }
    let source_snapshot = fingerprint(&serde_json::to_vec(&base).map_err(|e| e.to_string())?);
    let cache = directory.join(STRUCTURE_FILE);
    let projection = match std::fs::read(&cache) {
        Ok(data) => {
            let projection: Projection = serde_json::from_slice(&data)
                .map_err(|e| format!("PDF 结构快照损坏，原标注保持不变: {e}"))?;
            if projection.source_snapshot != source_snapshot
                || projection.snapshot.fingerprint != base.fingerprint
                || projection.snapshot.extractor != super::STRUCTURE_EXTRACTOR
            {
                return Err("PDF 结构快照与原快照不一致，原标注保持不变".into());
            }
            projection.validate(&base)?;
            projection
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let doc = open(read_pdf(path)?)?;
            let mut snapshot = base.clone();
            let mut maps = Vec::new();
            for (page, html) in snapshot.pages.iter_mut().enumerate() {
                let (new, map) = super::structure::repair(&doc, page, html)?;
                *html = new;
                maps.push(map);
            }
            snapshot.extractor = super::STRUCTURE_EXTRACTOR.into();
            let projection = Projection {
                source_snapshot,
                snapshot,
                maps,
            };
            projection.validate(&base)?;
            let tmp = directory.join(format!(".pdf-structure-{}.tmp", uuid::Uuid::new_v4()));
            let data = serde_json::to_vec(&projection).map_err(|e| e.to_string())?;
            if let Err(e) = std::fs::write(&tmp, data).and_then(|_| std::fs::rename(&tmp, &cache)) {
                let _ = std::fs::remove_file(tmp);
                return Err(format!("保存 PDF 结构快照失败: {e}"));
            }
            projection
        }
        Err(e) => return Err(format!("读取 PDF 结构快照失败: {e}")),
    };
    super::content::ensure_assets(path, &projection.snapshot.clone().into_chapters())?;
    let loaded = apply_projection(pool, book_id, projection, STRUCTURE_MARKER).await?;
    Ok(super::content::resolve_assets(loaded, path))
}
