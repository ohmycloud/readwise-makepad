use crate::{
    content::{media_type, private_media},
    library::{Location, MAX_UPLOAD, State, validate_epub, validate_word},
    settings::Preferences,
};
use readwise_core::vocab::models::VocabBookAddInput;
use serde::Deserialize;
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{
        content::{Json, multipart::Multipart},
        error::{bad_request, not_found},
        header, path_param,
        response::IntoResponse,
        route,
    },
};

path_param!(pub book_id);
path_param!(pub chapter_no: usize, error = not_found);
path_param!(media_no: usize, error = not_found);

pub fn state(cx: &Cx) -> State {
    app_context::<State>(cx).clone()
}

pub fn book_id(cx: &Cx) -> Result<String> {
    let id = path_param::<BookId>(cx);
    uuid::Uuid::parse_str(id).map_err(|_| not_found())?;
    Ok(id.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_upload_basename_is_bounded_and_cannot_escape_the_private_directory() {
        assert_eq!(upload_name("../../中文书.PDF", "pdf"), "中文书.pdf");
        assert_eq!(
            upload_name(r"C:\fakepath\中文书.epub", "epub"),
            "中文书.epub"
        );
        assert_eq!(upload_name("..", "pdf"), "book.pdf");
        let long = upload_name(&format!("{}.pdf", "🌿".repeat(300)), "pdf");
        assert!(long.len() < 255);
        assert!(!upload_name("a:*?.epub", "epub").contains([':', '*', '?']));
    }
}

#[route(GET "/api/health")]
async fn health() -> Result<Json<serde_json::Value>> {
    Ok(Json(
        serde_json::json!({"status": "ok", "service": "readwise-web"}),
    ))
}

fn upload_name(filename: &str, extension: &str) -> String {
    let basename = filename.rsplit(['/', '\\']).next().unwrap_or("book");
    let stem = Path::new(basename)
        .file_stem()
        .and_then(|v| v.to_str())
        .unwrap_or("book");
    let stem: String = stem
        .chars()
        .take(60)
        .map(|ch| {
            if ch.is_control() || "<>:\"|?*".contains(ch) {
                '_'
            } else {
                ch
            }
        })
        .collect();
    let stem = stem.trim_matches(['.', ' ']);
    format!(
        "{}.{extension}",
        if stem.is_empty() { "book" } else { stem }
    )
}

#[route(POST "/api/import")]
async fn import(cx: &Cx, mut multipart: Multipart) -> Result<Json<serde_json::Value>> {
    let state = state(cx);
    let mut field = multipart
        .next_field()
        .await?
        .ok_or_else(|| bad_request("请选择书籍"))?;
    if field.name() != Some("file") {
        return Err(bad_request("上传字段必须为 file").into());
    }
    let filename = field.file_name().unwrap_or("").to_owned();
    let extension = Path::new(&filename)
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "epub" | "pdf") {
        return Err(bad_request("仅支持 EPUB 和 PDF").into());
    }
    let temporary = tempfile::Builder::new()
        .prefix("readwise-")
        .tempdir_in(state.data_dir.join("uploads"))?;
    // Retain a safe original basename for PDF's missing-title fallback, while
    // the random private directory prevents collisions and traversal.
    let source = temporary.path().join(upload_name(&filename, &extension));
    let mut file = tokio::fs::File::create(&source).await?;
    let mut size = 0usize;
    while let Some(chunk) = field.chunk().await? {
        size = size
            .checked_add(chunk.len())
            .ok_or_else(|| bad_request("文件过大"))?;
        if size > MAX_UPLOAD {
            return Err(bad_request("每本书最大 200 MB").into());
        }
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    drop(file);
    drop(field);
    if size == 0 {
        return Err(bad_request("文件为空").into());
    }
    if multipart.next_field().await?.is_some() {
        return Err(bad_request("每次请求只上传一本；批量导入会逐本提交").into());
    }
    let imported = state
        .call(move |library| {
            let _temporary = temporary;
            if extension == "epub" {
                validate_epub(&source)?;
            }
            // Parsing, copying, PDF snapshots and SQL all run on the bounded worker.
            // TempDir cleans only our private upload on success/failure/drop.
            library.db.import_book(&source)
        })
        .await
        .map_err(|error| bad_request(error.to_string()))?;
    Ok(Json(
        serde_json::json!({"id": imported.id, "title": imported.title, "filename": filename}),
    ))
}

#[route(GET "/api/preferences")]
async fn get_preferences(cx: &Cx) -> Result<Json<Preferences>> {
    Ok(Json(
        state(cx)
            .call(|library| Preferences::load(&library.db))
            .await?,
    ))
}

#[route(POST "/api/preferences")]
async fn save_preferences(
    cx: &Cx,
    Json(patch): Json<serde_json::Value>,
) -> Result<Json<Preferences>> {
    let patch = patch
        .as_object()
        .ok_or_else(|| bad_request("设置必须是对象"))?
        .clone();
    let saved = state(cx)
        .call(move |library| {
            let mut merged =
                serde_json::to_value(Preferences::load(&library.db)?).map_err(|e| e.to_string())?;
            for (key, value) in patch {
                if !merged.as_object().unwrap().contains_key(&key) {
                    return Err("未知设置字段".into());
                }
                merged.as_object_mut().unwrap().insert(key, value);
            }
            let preferences: Preferences =
                serde_json::from_value(merged).map_err(|_| "设置格式无效")?;
            preferences.save(&library.db)?;
            Preferences::load(&library.db)
        })
        .await
        .map_err(|error| bad_request(error.to_string()))?;
    Ok(Json(saved))
}

#[route(GET "/api/known")]
async fn known(cx: &Cx) -> Result<Json<Vec<String>>> {
    Ok(Json(
        state(cx)
            .call(|library| library.db.get_known_words())
            .await?,
    ))
}

#[derive(Deserialize)]
struct KnownInput {
    lemma: String,
    known: bool,
}

#[route(POST "/api/known")]
async fn save_known(cx: &Cx, Json(input): Json<KnownInput>) -> Result<Json<serde_json::Value>> {
    let lemma = validate_word(&input.lemma).map_err(bad_request)?;
    state(cx)
        .call(move |library| {
            if input.known {
                library.db.add_known_word(&lemma, "web")
            } else {
                library.db.remove_known_word(&lemma)
            }
        })
        .await?;
    Ok(Json(serde_json::json!({"saved": true})))
}

#[route(POST "/api/vocabulary")]
async fn add_vocabulary(
    cx: &Cx,
    Json(mut input): Json<VocabBookAddInput>,
) -> Result<Json<serde_json::Value>> {
    input.lemma = validate_word(&input.lemma).map_err(bad_request)?;
    let added = state(cx)
        .call(move |library| library.add_vocabulary(&input))
        .await?;
    Ok(Json(serde_json::json!({"added": added})))
}

#[derive(Deserialize)]
struct RemoveVocabulary {
    lemma: String,
}

#[route(POST "/api/vocabulary/remove")]
async fn remove_vocabulary(
    cx: &Cx,
    Json(input): Json<RemoveVocabulary>,
) -> Result<Json<serde_json::Value>> {
    let lemma = validate_word(&input.lemma).map_err(bad_request)?;
    state(cx)
        .call(move |library| library.db.vocab_book_remove(&lemma))
        .await?;
    Ok(Json(serde_json::json!({"saved": true})))
}

#[derive(Deserialize)]
struct ExcerptInput {
    book_id: String,
    chapter: usize,
    text: String,
    #[serde(default)]
    note: String,
}

#[route(POST "/api/excerpts")]
async fn save_excerpt(cx: &Cx, Json(input): Json<ExcerptInput>) -> Result<Json<serde_json::Value>> {
    uuid::Uuid::parse_str(&input.book_id).map_err(|_| bad_request("书籍 ID 无效"))?;
    if input.text.trim().is_empty() || input.text.len() > 24_000 || input.note.len() > 12_000 {
        return Err(bad_request("摘录不能为空，且不能超过 24000 字节").into());
    }
    let id = state(cx)
        .call(move |library| {
            library.save_excerpt(&input.book_id, input.chapter, &input.text, &input.note)
        })
        .await
        .map_err(|error| bad_request(error.to_string()))?;
    Ok(Json(serde_json::json!({"saved": true, "id": id})))
}

#[derive(Deserialize)]
struct ProgressInput {
    book_id: String,
    location: Location,
}

#[route(POST "/api/progress")]
async fn save_progress(
    cx: &Cx,
    Json(input): Json<ProgressInput>,
) -> Result<Json<serde_json::Value>> {
    uuid::Uuid::parse_str(&input.book_id).map_err(|_| bad_request("书籍 ID 无效"))?;
    state(cx)
        .call(move |library| library.progress(&input.book_id, input.location))
        .await
        .map_err(|error| bad_request(error.to_string()))?;
    Ok(Json(serde_json::json!({"saved": true})))
}

async fn image(path: std::path::PathBuf) -> Result<impl IntoResponse> {
    let mime = media_type(&path).ok_or_else(not_found)?;
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| not_found())?;
    if file.metadata().await?.len() > 32 * 1024 * 1024 {
        return Err(not_found().into());
    }
    let mut bytes = Vec::new();
    file.take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(not_found().into());
    }
    Ok(([(header::CONTENT_TYPE, mime)], bytes))
}

#[route(GET "/cover/{book_id}")]
async fn cover(cx: &Cx) -> Result<impl IntoResponse> {
    let id = book_id(cx)?;
    let path = state(cx)
        .call(move |library| {
            let Some(book) = library.book(&id)? else {
                return Ok(None);
            };
            let Some(cover) = book.cover_abs_path else {
                return Ok(None);
            };
            Ok(private_media(
                Path::new(&cover),
                Path::new(&book.file_abs_path)
                    .parent()
                    .ok_or("书籍路径无效")?,
            ))
        })
        .await?
        .ok_or_else(not_found)?;
    image(path).await
}

#[route(GET "/media/{book_id}/{media_no}")]
async fn media(cx: &Cx) -> Result<impl IntoResponse> {
    let id = book_id(cx)?;
    let index = *path_param::<MediaNo>(cx)?;
    let path = state(cx)
        .call(move |library| {
            let Some(book) = library.book(&id)? else {
                return Ok(None);
            };
            let content = library.content(&book)?;
            Ok(content
                .media
                .get(index)
                .and_then(|path| private_media(path, Path::new(&book.file_abs_path).parent()?)))
        })
        .await?
        .ok_or_else(not_found)?;
    image(path).await
}
