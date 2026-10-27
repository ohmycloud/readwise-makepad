//! v8 continues from the committed v7 coordinate system, including books
//! natively imported as v7. Never regenerate v7 with the new algorithm.
use super::super::{reflow_v8 as reflow, EXTENDED_REFLOW_EXTRACTOR as REFLOW_EXTRACTOR};
use super::*;
pub(super) const REFLOW_FILE: &str = "pdf-reflow-v8.json";
pub(super) const REFLOW_MARKER: &str = "pdf_reflow_projection_v8";

pub(crate) async fn load_reader_pages(
    pool: &SqlitePool,
    book_id: &str,
    path: &Path,
    source: &ReaderSource<'_>,
) -> Result<LoadedChapters, String> {
    let directory = path.parent().ok_or("PDF 目录无效")?;
    let metadata: Option<String> =
        sqlx::query_scalar("SELECT metadata FROM book_status WHERE book_id = ?")
            .bind(book_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?
            .flatten();
    let markers = metadata
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
    let committed = markers
        .as_ref()
        .is_some_and(|m| m.get(REFLOW_MARKER).is_some());
    if committed && !directory.join(SNAPSHOT_FILE).is_file() {
        return Err("PDF 原始文本快照缺失，未重建，以避免已有标注错位".into());
    }
    let loaded = source.loaded()?;
    let mut base = source.original()?.1.clone();
    if base.extractor == REFLOW_EXTRACTOR {
        if markers.as_ref().is_some_and(|m| {
            [
                MARKER,
                CONTENT_MARKER,
                STRUCTURE_MARKER,
                LIST_MARKER,
                TOC_MARKER,
                super::reflow_migration::REFLOW_MARKER,
                REFLOW_MARKER,
            ]
            .iter()
            .any(|k| m.get(*k).is_some())
        }) {
            return Err("PDF 原始文本快照已被替换，原标注保持不变".into());
        }
        return Ok(loaded);
    }
    super::reflow_migration::load_reader_pages(pool, book_id, path, source).await?;
    if base.extractor != super::super::REFLOW_EXTRACTOR {
        let data = std::fs::read(directory.join(super::reflow_migration::REFLOW_FILE))
            .map_err(|e| e.to_string())?;
        let projection: Projection = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
        base = projection.snapshot;
    }
    let source_snapshot = fingerprint(&serde_json::to_vec(&base).map_err(|e| e.to_string())?);
    let cache = directory.join(REFLOW_FILE);
    let projection = match std::fs::read(&cache) {
        Ok(data) => {
            let projection: Projection = serde_json::from_slice(&data)
                .map_err(|e| format!("PDF 代码重排快照损坏，原标注保持不变: {e}"))?;
            if projection.source_snapshot != source_snapshot
                || projection.snapshot.fingerprint != base.fingerprint
                || projection.snapshot.extractor != REFLOW_EXTRACTOR
            {
                return Err("PDF 代码重排快照与原快照不一致，原标注保持不变".into());
            }
            projection.validate(&base)?;
            projection
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let doc = open(read_pdf(path)?)?;
            let mut snapshot = base.clone();
            let mut maps = Vec::new();
            let mut source_texts = Vec::new();
            for (page, html) in snapshot.pages.iter_mut().enumerate() {
                if let Some(repair) = reflow::repair(&doc, page, html)? {
                    *html = repair.html;
                    maps.push(repair.maps);
                    source_texts.push(Some(repair.source));
                } else {
                    let blocks = super::super::super::epub_meta::split_blocks(html);
                    maps.push(
                        blocks
                            .iter()
                            .enumerate()
                            .map(|(block, b)| BlockTarget {
                                block,
                                prefix: 0,
                                old_len: crate::vocab::annotate::rendered_text(b).len(),
                                offsets: None,
                                layout_changed: false,
                            })
                            .collect(),
                    );
                    source_texts.push(None);
                }
            }
            snapshot.extractor = REFLOW_EXTRACTOR.into();
            let projection = Projection {
                source_snapshot,
                snapshot,
                maps,
                source_texts: Some(source_texts),
            };
            projection.validate(&base)?;
            let tmp = directory.join(format!(".pdf-reflow-v8-{}.tmp", uuid::Uuid::new_v4()));
            let data = serde_json::to_vec(&projection).map_err(|e| e.to_string())?;
            if let Err(e) = std::fs::write(&tmp, data).and_then(|_| std::fs::rename(&tmp, &cache)) {
                let _ = std::fs::remove_file(&tmp);
                return Err(format!("保存 PDF 代码重排快照失败: {e}"));
            }
            projection
        }
        Err(e) => return Err(format!("读取 PDF 代码重排快照失败: {e}")),
    };
    let loaded = apply_projection(pool, book_id, projection, REFLOW_MARKER).await?;
    Ok(super::super::content::resolve_assets(loaded, path))
}
