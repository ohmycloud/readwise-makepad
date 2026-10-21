//! Extract PDF text into the same semantic HTML used by EPUB reading.
//! All entry points perform file I/O / extraction and must run off the UI thread.
use super::epub_meta::{sanitize_for_reader, ChapterInfo, EpubMeta, LoadedChapters, TocEntry};
use pdf_oxide::{converters::ConversionOptions, object::Object, PdfDocument};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, path::Path};

const SNAPSHOT_FILE: &str = "pdf-text-v1.json";
const SNAPSHOT_VERSION: u32 = 1;
mod content;
pub(crate) mod cover;
mod headings;
mod migration;
mod structure;
pub(crate) use migration::load_reader_pages;

const LEGACY_EXTRACTOR: &str = "pdf_oxide-0.3.78/semantic-v1";
const EXTRACTOR: &str = "pdf_oxide-0.3.78/semantic-v2";
const CONTENT_EXTRACTOR: &str = "pdf_oxide-0.3.78/semantic-v3";
const STRUCTURE_EXTRACTOR: &str = "pdf_oxide-0.3.78/semantic-v4";
const NO_TEXT: &str = "PDF 没有可提取的文本层（可能是扫描件），请先进行 OCR 后重新导入";

fn open(bytes: Vec<u8>) -> Result<PdfDocument, String> {
    let doc = PdfDocument::from_bytes(bytes).map_err(|e| format!("打开 PDF 失败: {e}"))?;
    if doc.is_encrypted()
        && !doc
            .authenticate(b"")
            .map_err(|e| format!("PDF 解密失败: {e}"))?
    {
        return Err("此 PDF 需要密码，请先解密后重新导入".into());
    }
    if doc
        .page_count()
        .map_err(|e| format!("读取 PDF 页数失败: {e}"))?
        == 0
    {
        return Err("PDF 没有可阅读的页面".into());
    }
    Ok(doc)
}

fn read_pdf(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("读取 PDF 失败: {e}"))
}

fn dictionary_text(doc: &PdfDocument, object: &Object, key: &str) -> Option<String> {
    let object = doc.resolve_object(object).ok()?;
    let value = doc.resolve_object(object.as_dict()?.get(key)?).ok()?;
    let text = pdf_oxide::optional_content::decode_pdf_text_string(value.as_string()?);
    let text = text.trim().trim_start_matches('\u{feff}').trim();
    (!text.is_empty()).then(|| text.to_owned())
}

pub fn parse_meta(path: &Path) -> Result<EpubMeta, String> {
    let doc = open(read_pdf(path)?)?;
    let info = doc.trailer().as_dict().and_then(|dict| dict.get("Info"));
    let title = info
        .and_then(|info| dictionary_text(&doc, info, "Title"))
        .unwrap_or_else(|| {
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        });
    Ok(EpubMeta {
        title,
        author: info
            .and_then(|info| dictionary_text(&doc, info, "Author"))
            .unwrap_or_default(),
        language: doc
            .catalog()
            .ok()
            .and_then(|catalog| dictionary_text(&doc, &catalog, "Lang"))
            .unwrap_or_default(),
        // Import generates the thumbnail separately, after text validation.
        cover: None,
    })
}

#[derive(Clone, Serialize, Deserialize)]
struct Navigation {
    title: String,
    level: usize,
    page: usize,
}

/// Immutable, UNANNOTATED source projection. Highlights refer to the block and
/// UTF-8 offsets in this snapshot, not to a fresh extraction on each book open.
/// Extractor upgrades must not silently replace an existing projection.
#[derive(Clone, Serialize, Deserialize)]
struct Snapshot {
    version: u32,
    fingerprint: String,
    extractor: String,
    pages: Vec<String>,
    toc: Vec<Navigation>,
}

impl Snapshot {
    fn into_chapters(self) -> LoadedChapters {
        let chapters = self
            .pages
            .into_iter()
            .enumerate()
            .map(|(page, html)| ChapterInfo {
                title: format!("PDF {}", page + 1),
                html,
                // Keep one chapter per ORIGINAL page, including empty pages. This
                // preserves old image-reader locations and bookmark chapter indices.
                source_path: format!("pdf-page-{page}"),
                figure_targets: HashMap::new(),
            })
            .collect();
        let toc = self
            .toc
            .into_iter()
            .map(|entry| TocEntry {
                title: entry.title,
                level: entry.level,
                chapter: entry.page,
                fragment: None,
            })
            .collect();
        LoadedChapters { chapters, toc }
    }
}

fn fingerprint(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

// pdf_oxide's current HTML converter does not apply expand_ligatures to every
// font path. Normalize text nodes here so `ofﬁce` can match the `office` gloss;
// attributes (including link targets) must remain byte-for-byte untouched.
fn expand_text_ligatures(html: &str) -> String {
    let mut output = String::with_capacity(html.len());
    let mut tag = false;
    for c in html.chars() {
        if c == '<' {
            tag = true;
        }
        if !tag {
            match c {
                'ﬀ' => output.push_str("ff"),
                'ﬁ' => output.push_str("fi"),
                'ﬂ' => output.push_str("fl"),
                'ﬃ' => output.push_str("ffi"),
                'ﬄ' => output.push_str("ffl"),
                'ﬅ' | 'ﬆ' => output.push_str("st"),
                _ => output.push(c),
            }
        } else {
            output.push(c);
        }
        if c == '>' {
            tag = false;
        }
    }
    output
}

fn extract(bytes: Vec<u8>, fingerprint: String) -> Result<Snapshot, String> {
    extract_with_assets(bytes, fingerprint, None)
}

fn extract_with_assets(
    bytes: Vec<u8>,
    fingerprint: String,
    directory: Option<&Path>,
) -> Result<Snapshot, String> {
    let doc = open(bytes)?;
    let count = doc
        .page_count()
        .map_err(|e| format!("读取 PDF 页数失败: {e}"))?;
    let options = ConversionOptions {
        preserve_layout: false,
        include_images: false,
        include_form_fields: false,
        expand_ligatures: true,
        ..Default::default()
    };
    let mut pages = Vec::with_capacity(count);
    let mut has_text = false;
    for page in 0..count {
        let html = doc
            .to_html(page, &options)
            .map_err(|e| format!("提取 PDF 第 {} 页失败: {e}", page + 1))?;
        let html = expand_text_ligatures(&sanitize_for_reader(&html.replace('\0', "")));
        let spans = doc
            .extract_spans(page)
            .map_err(|e| format!("读取 PDF 第 {} 页行坐标失败: {e}", page + 1))?;
        let (html, _) = headings::repair(&html, &spans);
        let (html, _) = content::repair(&doc, page, &html, directory)?;
        let (html, _) = structure::repair(&doc, page, &html)?;
        let text = crate::vocab::annotate::rendered_text(&html);
        if text.chars().any(|c| !c.is_whitespace()) {
            has_text = true;
            pages.push(html);
        } else {
            // Do not invent prose that would become searchable / annotatable.
            // Empty chapters retain their original page index.
            pages.push(if html.contains("<img ") {
                html
            } else {
                "<p></p>".into()
            });
        }
    }
    if !has_text {
        return Err(NO_TEXT.into());
    }
    let mut toc = Vec::new();
    match doc.get_outline() {
        Ok(Some(outline)) => flatten_outline(&doc, &outline, count, 0, &mut toc),
        Err(error) => log::warn!("PDF 目录解析失败，使用原始页目录: {error}"),
        _ => {}
    }
    if toc.is_empty() {
        toc = (0..count)
            .map(|page| Navigation {
                title: format!("PDF {}", page + 1),
                level: 0,
                page,
            })
            .collect();
    }
    Ok(Snapshot {
        version: SNAPSHOT_VERSION,
        fingerprint,
        extractor: STRUCTURE_EXTRACTOR.into(),
        pages,
        toc,
    })
}

fn flatten_outline(
    doc: &PdfDocument,
    items: &[pdf_oxide::OutlineItem],
    count: usize,
    level: usize,
    out: &mut Vec<Navigation>,
) {
    for item in items {
        let page = match &item.dest {
            Some(pdf_oxide::Destination::PageIndex(page)) => Some(*page),
            Some(pdf_oxide::Destination::Named(name)) => {
                doc.resolve_named_destination(name).ok().flatten()
            }
            None => None,
        };
        if let Some(page) = page.filter(|page| *page < count) {
            out.push(Navigation {
                title: item.title.clone(),
                level: level.min(2),
                page,
            });
        }
        flatten_outline(doc, &item.children, count, level + 1, out);
    }
}

/// Extract an arbitrary source without creating files beside the user's PDF.
pub fn load_pages(path: &Path) -> Result<LoadedChapters, String> {
    let bytes = read_pdf(path)?;
    let hash = fingerprint(&bytes);
    extract(bytes, hash).map(Snapshot::into_chapters)
}

/// Source snapshot for the app-owned copy in books/<id>/. Also extracts PDFs
/// imported by the former image reader. Persisted reading must go through
/// `Db::load_book_chapters` to apply heading repairs and coordinate migration.
pub fn load_imported_pages(path: &Path) -> Result<LoadedChapters, String> {
    let bytes = read_pdf(path)?;
    let hash = fingerprint(&bytes);
    let directory = path.parent().ok_or("PDF 书籍目录无效")?;
    let cache = directory.join(SNAPSHOT_FILE);
    match std::fs::read(&cache) {
        Ok(data) => {
            let snapshot: Snapshot = serde_json::from_slice(&data)
                .map_err(|e| format!("PDF 文本快照损坏，请重新导入书籍以避免标注错位: {e}"))?;
            if snapshot.version != SNAPSHOT_VERSION
                || snapshot.fingerprint != hash
                || snapshot.pages.is_empty()
                || snapshot
                    .toc
                    .iter()
                    .any(|entry| entry.page >= snapshot.pages.len())
            {
                return Err("PDF 原文件或文本快照已变化，请重新导入书籍以避免标注错位".into());
            }
            let loaded = snapshot.into_chapters();
            content::ensure_assets(path, &loaded)?;
            return Ok(content::resolve_assets(loaded, path));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("读取 PDF 文本快照失败: {error}")),
    }
    let snapshot = extract_with_assets(bytes, hash, Some(directory))?;
    let data = serde_json::to_vec(&snapshot).map_err(|e| format!("编码 PDF 文本快照失败: {e}"))?;
    let temporary = directory.join(format!(".pdf-text-{}.tmp", uuid::Uuid::new_v4()));
    if let Err(error) =
        std::fs::write(&temporary, data).and_then(|_| std::fs::rename(&temporary, &cache))
    {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("保存 PDF 文本快照失败: {error}"));
    }
    Ok(content::resolve_assets(snapshot.into_chapters(), path))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn fixture() -> Vec<u8> {
        pdf_with_pages(&[
            "BT /F1 14 Tf 20 260 Td (The solitude of a resilient reader & <forest>.) Tj 0 -30 Td (A gentle murmur follows.) Tj ET",
            "BT /F1 14 Tf 20 260 Td (The reader returns to the forest.) Tj ET",
        ])
    }

    fn pdf_with_pages(contents: &[&str]) -> Vec<u8> {
        pdf_with_outline(contents, false)
    }

    fn pdf_with_outline(contents: &[&str], outline: bool) -> Vec<u8> {
        pdf_with_font(contents, outline, "Helvetica")
    }

    pub(crate) fn pdf_with_font(contents: &[&str], outline: bool, font_name: &str) -> Vec<u8> {
        let count = contents.len();
        let font = 3 + 2 * count;
        let kids = (0..count)
            .map(|i| format!("{} 0 R", i + 3))
            .collect::<Vec<_>>()
            .join(" ");
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R /Lang (en) >>".to_owned(),
            format!("<< /Type /Pages /Kids [{kids}] /Count {count} >>"),
        ];
        for i in 0..count {
            objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Resources << /Font << /F1 {font} 0 R /F2 {} 0 R >> >> /Contents {} 0 R >>", font + 3, 3 + count + i));
        }
        for content in contents {
            objects.push(format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            ));
        }
        objects.push(format!(
            "<< /Type /Font /Subtype /Type1 /BaseFont /{font_name} /ToUnicode {} 0 R >>",
            font + 2
        ));
        objects.push("<< /Title (Test PDF) /Author (ReadWise) >>".into());
        let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /Test def /CMapType 2 def 1 begincodespacerange <00> <FF> endcodespacerange 1 beginbfrange <20> <7E> <0020> endbfrange 3 beginbfchar <01> <4E2D> <02> <6587> <03> <FB01> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end";
        objects.push(format!(
            "<< /Length {} >>\nstream\n{cmap}\nendstream",
            cmap.len()
        ));
        objects.push(format!(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode {} 0 R >>",
            font + 2
        ));
        if outline {
            objects[0] = format!(
                "<< /Type /Catalog /Pages 2 0 R /Outlines {} 0 R >>",
                font + 4
            );
            objects.push(format!(
                "<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count 2 >>",
                font + 5,
                font + 5
            ));
            objects.push(format!("<< /Title (Introduction) /Parent {} 0 R /Dest [3 0 R /Fit] /First {} 0 R /Last {} 0 R /Count 1 >>", font + 4, font + 6, font + 6));
            objects.push(format!(
                "<< /Title (Next page) /Parent {} 0 R /Dest [4 0 R /Fit] >>",
                font + 5
            ));
        }
        let mut bytes = b"%PDF-1.4\n".to_vec();
        let mut offsets = vec![0];
        for (i, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
        }
        let start = bytes.len();
        bytes.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes(),
        );
        for offset in &offsets[1..] {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R /Info {} 0 R >>\nstartxref\n{start}\n%%EOF\n",
                offsets.len(),
                font + 1
            )
            .as_bytes(),
        );
        bytes
    }

    struct Temporary(std::path::PathBuf);
    impl Temporary {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("readwise-pdf-oxide-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn write(&self, name: &str, bytes: &[u8]) -> std::path::PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }
    }
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn oxide_extracts_selectable_html_not_page_bitmaps() {
        let temp = Temporary::new();
        let source = temp.write("中文 & \"test\".PDF", &fixture());
        let meta = parse_meta(&source).unwrap();
        assert_eq!(meta.title, "Test PDF");
        assert_eq!(meta.author, "ReadWise");
        assert_eq!(meta.language, "en");
        assert!(meta.cover.is_none());
        let pages = load_pages(&source).unwrap();
        assert_eq!(pages.chapters.len(), 2);
        assert_eq!(pages.toc[1].chapter, 1);
        let html = &pages.chapters[0].html;
        assert!(html.contains("solitude"), "{html}");
        assert!(html.contains("&amp;"), "{html}");
        assert!(html.contains("&lt;forest&gt;"), "{html}");
        assert!(!html.contains("<img"));
        assert!(!html.contains("position:"));
        let text = crate::vocab::annotate::rendered_text(html);
        assert!(text.contains("& <forest>"), "{text}");
        let glosses = HashMap::from([("solitude".to_owned(), "独处".to_owned())]);
        let annotated = crate::vocab::annotate::annotate_html(html, &glosses, &Default::default());
        assert!(annotated.contains("<ruby"), "{annotated}");
        assert_eq!(crate::vocab::annotate::rendered_text(&annotated), text);
        assert!(
            !temp.0.join(SNAPSHOT_FILE).exists(),
            "never write beside a user source"
        );
    }

    pub(super) fn heading_fixture() -> Vec<u8> {
        let content = format!("BT /F1 24 Tf 20 260 Td (Chapter) Tj /F2 24 Tf ( ) Tj /F1 24 Tf (1) Tj ET BT /F1 28 Tf 20 200 Td (Getting) Tj /F2 28 Tf ( ) Tj /F1 28 Tf (Started) Tj ET BT /F1 10 Tf 20 140 Td ({}) Tj ET", "Readable body text with a reader and murmur. ".repeat(8));
        pdf_with_font(&[&content], false, "Helvetica-Bold")
    }

    #[test]
    fn split_pdf_heading_spans_are_rejoined_only_on_their_original_line() {
        let bytes = heading_fixture();
        let doc = open(bytes.clone()).unwrap();
        let raw = doc.to_html(0, &Default::default()).unwrap();
        let spans = doc.extract_spans(0).unwrap();
        let (fixed, mapping) = headings::repair(&raw, &spans);
        let blocks = super::super::epub_meta::split_blocks(&fixed);
        assert_eq!(
            crate::vocab::annotate::rendered_text(&blocks[0]),
            "Chapter 1",
            "raw={raw}\nfixed={fixed}"
        );
        assert!(blocks[0].starts_with("<h1>"));
        assert_eq!(blocks[1], "<h1>Getting Started</h1>");
        assert!(mapping[1].prefix > 0);
        let snapshot = extract(bytes, "test".into()).unwrap();
        assert!(snapshot.pages[0].contains("<h1>Getting Started</h1>"));
    }

    #[test]
    fn native_pdf_outline_preserves_hierarchy_and_original_page_targets() {
        let temp = Temporary::new();
        let source = temp.write(
            "outline.pdf",
            &pdf_with_outline(
                &[
                    "BT /F1 14 Tf 20 200 Td (First page.) Tj ET",
                    "BT /F1 14 Tf 20 200 Td (Second page.) Tj ET",
                ],
                true,
            ),
        );
        let pages = load_pages(&source).unwrap();
        assert_eq!(
            pages
                .toc
                .iter()
                .map(|entry| (entry.title.as_str(), entry.level, entry.chapter))
                .collect::<Vec<_>>(),
            vec![("Introduction", 0, 0), ("Next page", 1, 1)]
        );
    }

    #[test]
    fn unicode_text_metadata_and_ligatures_survive_semantic_extraction() {
        let temp = Temporary::new();
        let source = temp.write(
            "unicode.pdf",
            &pdf_with_pages(&[
                "BT /F1 14 Tf 20 200 Td <0102> Tj 0 -30 Td (of) Tj <03> Tj (ce) Tj ET",
            ]),
        );
        let pages = load_pages(&source).unwrap();
        let text = crate::vocab::annotate::rendered_text(&pages.chapters[0].html);
        assert!(text.contains("中文"), "{text}");
        assert!(text.contains("office"), "{text}");
        let doc = open(fixture()).unwrap();
        let info = Object::Dictionary(HashMap::from([(
            "Title".into(),
            Object::String(vec![0xfe, 0xff, 0x4e, 0x2d, 0x65, 0x87]),
        )]));
        assert_eq!(dictionary_text(&doc, &info, "Title").unwrap(), "中文");
        assert_eq!(
            expand_text_ligatures("<a href=\"https://example.org/ﬁ\">ofﬁce</a>"),
            "<a href=\"https://example.org/ﬁ\">office</a>"
        );
    }

    #[test]
    fn blank_pages_keep_original_indices_but_scans_are_rejected() {
        let temp = Temporary::new();
        let mixed = temp.write(
            "mixed.pdf",
            &pdf_with_pages(&["", "BT /F1 14 Tf 20 200 Td (Readable words.) Tj ET", ""]),
        );
        let pages = load_pages(&mixed).unwrap();
        assert_eq!(pages.chapters.len(), 3);
        assert_eq!(pages.chapters[1].source_path, "pdf-page-1");
        assert!(
            crate::vocab::annotate::rendered_text(&pages.chapters[0].html)
                .trim()
                .is_empty()
        );
        let scan = temp.write("scan.pdf", &pdf_with_pages(&["", ""]));
        assert!(load_pages(&scan).err().unwrap().contains("OCR"));
        let bad = temp.write("invalid.pdf", b"not a PDF");
        assert!(load_pages(&bad).is_err());
    }

    #[test]
    fn text_snapshots_are_stable_and_do_not_silently_rebuild_invalid_offsets() {
        let temp = Temporary::new();
        let source = temp.write("book.pdf", &fixture());
        let first = load_imported_pages(&source).unwrap();
        let cache = temp.0.join(SNAPSHOT_FILE);
        let bytes = std::fs::read(&cache).unwrap();
        let second = load_imported_pages(&source).unwrap();
        assert_eq!(first.chapters[0].html, second.chapters[0].html);
        assert_eq!(std::fs::read(&cache).unwrap(), bytes);
        // The projection survives extractor upgrades; only an explicit schema
        // migration may change block IDs and invalidate saved annotations.
        let mut snapshot: Snapshot = serde_json::from_slice(&bytes).unwrap();
        snapshot.extractor = "previous-extractor".into();
        std::fs::write(&cache, serde_json::to_vec(&snapshot).unwrap()).unwrap();
        assert_eq!(
            load_imported_pages(&source).unwrap().chapters[0].html,
            first.chapters[0].html
        );
        std::fs::write(&cache, b"broken").unwrap();
        assert!(load_imported_pages(&source)
            .err()
            .unwrap()
            .contains("快照损坏"));
        std::fs::write(&cache, bytes).unwrap();
        std::fs::write(
            &source,
            pdf_with_pages(&["BT /F1 14 Tf 20 200 Td (Changed.) Tj ET"]),
        )
        .unwrap();
        assert!(load_imported_pages(&source)
            .err()
            .unwrap()
            .contains("已变化"));
    }

    #[test]
    fn pdf_import_reopen_highlights_bookmarks_and_delete_keep_the_source() {
        let root = Temporary::new();
        let source = root.write("中文 & test.PDF", &fixture());
        let data = root.0.join("data");
        let db = crate::Db::open_in(data.clone()).unwrap();
        let book = db.import_book(&source).unwrap();
        assert_eq!(book.format, "pdf");
        assert_eq!(book.title, "Test PDF");
        assert!(book.file_path.ends_with("book.pdf"));
        let shelf = db.get_shelf_items().unwrap();
        let stored = Path::new(&shelf[0].file_abs_path);
        assert!(stored.parent().unwrap().join(SNAPSHOT_FILE).exists());
        let pages = super::super::load_imported_chapters_with_toc(stored).unwrap();
        let blocks = super::super::epub_meta::split_blocks(&pages.chapters[0].html);
        assert!(!blocks.is_empty());
        let text = crate::vocab::annotate::rendered_text(&blocks[0]);
        let start = text.find("solitude").unwrap();
        let id = db
            .add_highlight(
                &book.id,
                0,
                (0, start, 0, start + "solitude".len()),
                "yellow",
            )
            .unwrap();
        let location = "v3:1:0:0";
        db.update_progress(&book.id, 2, 2, location).unwrap();
        db.add_bookmark(&book.id, location, "PDF page 2").unwrap();
        let scan = root.write("scan.pdf", &pdf_with_pages(&[""]));
        assert!(db.import_book(&scan).err().unwrap().contains("OCR"));
        assert_eq!(db.get_shelf_items().unwrap().len(), 1);
        assert_eq!(
            std::fs::read_dir(root.0.join("data/books"))
                .unwrap()
                .count(),
            1
        );
        assert!(scan.is_file());
        drop(db);
        let db = crate::Db::open_in(data).unwrap();
        assert_eq!(db.get_status(&book.id).unwrap().unwrap().location, location);
        assert_eq!(db.get_bookmarks(&book.id).unwrap()[0].label, "PDF page 2");
        let highlights = db.get_highlights_by_chapter(&book.id).unwrap();
        assert_eq!(highlights[&0][0].id, id);
        assert_eq!(highlights[&0][0].start_char, start);
        db.delete_book(&book.id).unwrap();
        assert!(db.get_shelf_items().unwrap().is_empty());
        assert_eq!(std::fs::read(&source).unwrap(), fixture());
    }
}
