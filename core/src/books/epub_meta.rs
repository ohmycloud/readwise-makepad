use epub::doc::EpubDoc;
use std::path::Path;

/// 从 EPUB 解析出的元数据（M2 只取书架需要的）。
pub struct EpubMeta {
    pub title: String,
    pub author: String,
    pub language: String,
    /// 封面图字节 + MIME（无封面为 None）。
    pub cover: Option<(Vec<u8>, String)>,
}

/// 用纯 Rust 的 epub crate 解析元数据（替代老项目的 foliate-js）。
pub fn parse_meta<P: AsRef<Path>>(path: P) -> Result<EpubMeta, String> {
    let mut doc = EpubDoc::new(path).map_err(|e| format!("打开 EPUB 失败: {}", e))?;

    let title = doc
        .mdata("title")
        .map(|item| item.value.clone())
        .unwrap_or_else(|| "未知书名".to_string());
    let author = doc
        .mdata("creator")
        .or_else(|| doc.mdata("author"))
        .map(|item| item.value.clone())
        .unwrap_or_else(|| "未知作者".to_string());
    let language = doc
        .mdata("language")
        .map(|item| item.value.clone())
        .unwrap_or_else(|| "unknown".to_string());

    let cover = doc.get_cover();

    Ok(EpubMeta {
        title,
        author,
        language,
        cover,
    })
}
