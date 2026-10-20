pub mod epub_meta;
pub mod models;
pub mod pdf;

/// Shared by local import, drop targets and LAN uploads.
pub fn supported_format(path: &std::path::Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "epub" => Some("epub"),
        "pdf" => Some("pdf"),
        _ => None,
    }
}

pub fn parse_meta(path: &std::path::Path) -> Result<epub_meta::EpubMeta, String> {
    match supported_format(path) {
        Some("epub") => epub_meta::parse_meta(path),
        Some("pdf") => pdf::parse_meta(path),
        _ => Err(format!("仅支持 EPUB 和 PDF：{}", path.display())),
    }
}

pub fn load_chapters_with_toc(path: &std::path::Path) -> Result<epub_meta::LoadedChapters, String> {
    match supported_format(path) {
        Some("epub") => epub_meta::load_chapters_with_toc(path),
        Some("pdf") => pdf::load_pages(path),
        _ => Err(format!("仅支持 EPUB 和 PDF：{}", path.display())),
    }
}
/// Low-level source-snapshot loader for app-owned files. For persisted reader
/// books use `Db::load_book_chapters`, which also migrates PDF coordinates.
/// Never write snapshots alongside arbitrary originals selected by the user.
pub fn load_imported_chapters_with_toc(
    path: &std::path::Path,
) -> Result<epub_meta::LoadedChapters, String> {
    if supported_format(path) == Some("pdf") {
        pdf::load_imported_pages(path)
    } else {
        load_chapters_with_toc(path)
    }
}

mod queries;

pub use queries::*;
