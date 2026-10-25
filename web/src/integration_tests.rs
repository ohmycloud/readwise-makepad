//! Request-level regression tests: no external server, CLI or user database.
use super::*;
use http_body_util::BodyExt;
use std::io::Write;
use topcoat::asset::{Manifest, ManifestEntry};

fn test_assets(path: &std::path::Path) -> AssetBundle {
    // HTTP tests render real Topcoat views but do not run a browser. A small
    // owned catalog avoids depending on a pre-built target/debug/assets bundle.
    let assets = [
        (pages::STYLE, "style.css", "text/css"),
        (pages::SCRIPT, "browser.js", "text/javascript"),
        (pages::APPEARANCE_SCRIPT, "appearance.js", "text/javascript"),
        (pages::I18N_SCRIPT, "i18n.js", "text/javascript"),
        (
            pages::COLOURS_SCRIPT,
            "custom-colours.js",
            "text/javascript",
        ),
        (pages::PAGER_SCRIPT, "pager.js", "text/javascript"),
        (pages::SELECTION_SCRIPT, "selection.js", "text/javascript"),
        (pages::HIGHLIGHT_SCRIPT, "highlight.mjs", "text/javascript"),
        (shelf::SCRIPT, "shelf.js", "text/javascript"),
        (topcoat::runtime::SCRIPT, "topcoat.js", "text/javascript"),
    ];
    let entries = assets
        .into_iter()
        .map(|(asset, name, content_type)| {
            std::fs::write(path.join(name), "/* request-test asset */").unwrap();
            ManifestEntry {
                id: asset.id(),
                file: name.into(),
                hash: "test".into(),
                content_type: content_type.into(),
            }
        })
        .collect();
    Manifest {
        version: 1,
        assets: entries,
    }
    .save(path.join("manifest.toml"))
    .unwrap();
    AssetBundle::load_dir(path).unwrap()
}

fn epub() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let files = [
        ("mimetype", "application/epub+zip".to_owned()),
        ("META-INF/container.xml", r#"<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="book.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#.into()),
        ("book.opf", r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">test</dc:identifier><dc:title>中文 EPUB</dc:title><dc:creator>QA</dc:creator><dc:language>en</dc:language></metadata><manifest><item id="one" href="one.xhtml" media-type="application/xhtml+xml"/><item id="two" href="two.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="one"/><itemref idref="two"/></spine></package>"#.into()),
        ("one.xhtml", format!("<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><h1>Forest</h1>{}</body></html>", "<p>solitude forest murmur 中文🌿</p>".repeat(20))),
        ("two.xhtml", "<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><h1>Return</h1><p>solitude forest</p></body></html>".into()),
    ];
    for (name, content) in files {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(content.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn multipart(file: &[u8], close: bool) -> Vec<u8> {
    multipart_file(file, close, "中文书.epub")
}

fn multipart_file(file: &[u8], close: bool, filename: &str) -> Vec<u8> {
    let mut body = format!("--test-boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n").into_bytes();
    body.extend(file);
    if close {
        body.extend(b"\r\n--test-boundary--\r\n")
    }
    body
}

async fn request(
    router: &Router,
    method: &str,
    path: &str,
    content_type: &str,
    body: Vec<u8>,
    host: &str,
    marked: bool,
) -> (http::StatusCode, http::HeaderMap, String) {
    let mut request = http::Request::builder()
        .method(method)
        .uri(path)
        .header("host", host)
        .header("content-type", content_type);
    if marked {
        request = request.header("x-readwise-request", "1");
    }
    let response = router.handle(request.body(Body::from(body)).unwrap()).await;
    let status = response.status();
    let headers = response.headers().clone();
    let text = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .into_owned();
    (status, headers, text)
}

async fn get(router: &Router, path: &str) -> (http::StatusCode, http::HeaderMap, String) {
    request(
        router,
        "GET",
        path,
        "application/json",
        vec![],
        "localhost:3000",
        false,
    )
    .await
}

async fn post(
    router: &Router,
    path: &str,
    body: serde_json::Value,
) -> (http::StatusCode, http::HeaderMap, String) {
    request(
        router,
        "POST",
        path,
        "application/json",
        serde_json::to_vec(&body).unwrap(),
        "localhost:3000",
        true,
    )
    .await
}

#[tokio::test]
async fn custom_colours_and_favorites_validate_merge_and_save_atomically() {
    let root = tempfile::tempdir().unwrap();
    let assets = tempfile::tempdir().unwrap();
    let state = State::open(root.path().to_owned(), vec!["localhost:3000".into()])
        .await
        .unwrap();
    let router = router(state.clone(), test_assets(assets.path()));
    let custom = serde_json::json!(["#A1B2C3", "#F6D6CC", "#3F6F98", "#CFE8D5"]);
    let favorites = serde_json::json!([
        {"colors":["#A1B2C3", "#F6D6CC", "#3F6F98", "#CFE8D5"]},
        {"colors":["#7FB2A6", "transparent", "inherit", "transparent"]}
    ]);
    let (status, _, body) = post(
        &router,
        "/api/preferences",
        serde_json::json!({"custom_colors":custom, "favorites":favorites}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let saved: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(saved["custom_colors"], custom);
    assert_eq!(saved["favorites"], favorites);
    let native = state.call(|l| l.db.load_reader_settings()).await.unwrap();
    assert!(native.gloss_color_slot < 4 && native.word_background_slot < 4);
    assert!(
        get(&router, "/settings")
            .await
            .2
            .contains("--gloss-color:#A1B2C3")
    );
    let (status, _, body) = post(
        &router,
        "/api/preferences",
        serde_json::json!({"gloss_background":3}),
    )
    .await;
    assert_eq!(status, 200);
    let saved: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(saved["custom_colors"], custom);
    assert_eq!(saved["favorites"], favorites);
    for bad in [
        serde_json::json!({"custom_colors":["red",null,null,null]}),
        serde_json::json!({"custom_colors":["#abc;url(x)",null,null,null]}),
        serde_json::json!({"custom_colors":["transparent",null,null,null]}),
        serde_json::json!({"custom_colors":["#123456"]}),
        serde_json::json!({"favorites":[{"colors":["inherit","transparent","inherit","transparent"]}]}),
        serde_json::json!({"favorites":[{"colors":["#123456","inherit","inherit","transparent"]}]}),
        serde_json::json!({"favorites":[{"colors":["#abcdef","transparent","inherit","transparent"]},{"colors":["#ABCDEF","transparent","inherit","transparent"]}]}),
        serde_json::json!({"favorites": (0..33).map(|i| serde_json::json!({"colors":[format!("#{i:06X}"),"transparent","inherit","transparent"]})).collect::<Vec<_>>()}),
    ] {
        assert_eq!(post(&router, "/api/preferences", bad).await.0, 400);
    }
    // Extension failure rolls back native values in the SAME transaction.
    let before = get(&router, "/api/preferences").await.2;
    sqlx::query("CREATE TRIGGER reject_colours BEFORE UPDATE ON app_settings WHEN NEW.key = 'web.annotation_colours.v1' BEGIN SELECT RAISE(FAIL, 'simulated colour failure'); END").execute(&state.storage).await.unwrap();
    assert_eq!(
        post(
            &router,
            "/api/preferences",
            serde_json::json!({"font_size":32,"custom_colors":[null,null,null,null],"favorites":[]})
        )
        .await
        .0,
        400
    );
    assert_eq!(get(&router, "/api/preferences").await.2, before);
    assert_eq!(
        state
            .call(|l| Ok(l.db.load_reader_settings()?.font_size))
            .await
            .unwrap(),
        native.font_size
    );
    sqlx::query("DROP TRIGGER reject_colours")
        .execute(&state.storage)
        .await
        .unwrap();
    assert_eq!(
        post(
            &router,
            "/api/preferences",
            serde_json::json!({"custom_colors":[null,null,null,null]})
        )
        .await
        .0,
        200
    );
    let reset: serde_json::Value =
        serde_json::from_str(&get(&router, "/api/preferences").await.2).unwrap();
    assert_eq!(reset["favorites"], favorites);
    assert_eq!(reset["gloss_background"], 3);
    // A new Library projection reads durable extensions, not process-local state.
    assert_eq!(
        state
            .call(|l| Ok(settings::Preferences::load(&l.db)?.favorites.len()))
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn shelf_management_reuses_native_state_and_cascades_only_owned_data() {
    let root = tempfile::tempdir().unwrap();
    let assets = tempfile::tempdir().unwrap();
    let state = State::open(root.path().to_owned(), vec!["localhost:3000".into()])
        .await
        .unwrap();
    let router = router(state.clone(), test_assets(assets.path()));
    let (_, _, imported) = request(
        &router,
        "POST",
        "/api/import",
        "multipart/form-data; boundary=test-boundary",
        multipart(&epub(), true),
        "localhost:3000",
        true,
    )
    .await;
    let imported: serde_json::Value = serde_json::from_str(&imported).unwrap();
    let id = imported["id"].as_str().unwrap();
    let path = format!("/api/books/{id}");
    let reader_path = format!("/read/{id}/0");
    assert_eq!(get(&router, &reader_path).await.0, 200); // Populate content cache.
    let anchor =
        serde_json::json!({"book_id": id, "location": {"chapter": 0, "block": 1, "offset": 8}});
    assert_eq!(post(&router, "/api/progress", anchor).await.0, 200);
    let before = state
        .call({
            let id = id.to_owned();
            move |l| l.db.get_status(&id)
        })
        .await
        .unwrap()
        .unwrap();
    let html = get(&router, "/").await.2;
    assert!(html.contains("在读 · 50%"));
    assert!(html.contains("book-more") && html.contains("book-delete-form"));
    for bad in [
        serde_json::json!({"action":"rename", "title":"  "}),
        serde_json::json!({"action":"rename", "title":"x".repeat(301)}),
        serde_json::json!({"action":"tags", "tags":["x".repeat(25)]}),
        serde_json::json!({"action":"unknown"}),
    ] {
        assert_eq!(post(&router, &path, bad).await.0, 400);
    }
    let renamed = post(
        &router,
        &path,
        serde_json::json!({"action":"rename", "title":"  中文 <新书名>  "}),
    )
    .await;
    assert_eq!(renamed.0, 200);
    assert!(!renamed.2.contains("file_abs_path"));
    assert_eq!(
        post(
            &router,
            &path,
            serde_json::json!({"action":"want_to_read", "enabled":true})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        post(
            &router,
            &path,
            serde_json::json!({"action":"tags", "tags":[" Rust ","rust","技术"]})
        )
        .await
        .0,
        200
    );
    let after = state
        .call({
            let id = id.to_owned();
            move |l| {
                let b = l.book(&id)?.unwrap();
                assert_eq!(b.title, "中文 <新书名>");
                assert!(b.want_to_read);
                assert_eq!(b.tags, ["Rust", "技术"]);
                l.db.get_status(&id)
            }
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.location, after.location);
    assert_eq!(before.progress_current, after.progress_current);
    assert!(get(&router, "/").await.2.contains("&lt;新书名&gt;"));
    assert_eq!(
        post(&router, &path, serde_json::json!({"action":"complete"}))
            .await
            .0,
        200
    );
    assert!(get(&router, "/").await.2.contains("读完 · 100%"));
    let after = state
        .call({
            let id = id.to_owned();
            move |l| l.db.get_status(&id)
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.location, after.location);
    assert_eq!(
        post(
            &router,
            &path,
            serde_json::json!({"action":"tags", "tags":[]})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        request(
            &router,
            "POST",
            &path,
            "application/json",
            br#"{"action":"delete"}"#.to_vec(),
            "localhost:3000",
            false
        )
        .await
        .0,
        403
    );
    assert_eq!(
        post(
            &router,
            "/api/books/not-a-uuid",
            serde_json::json!({"action":"delete"})
        )
        .await
        .0,
        404
    );
    // Real Web highlight FK must cascade even though this table is not native.
    assert_eq!(post(&router, "/api/highlights", serde_json::json!({"book_id":id,"chapter":0,"start":{"block":1,"offset":0},"end":{"block":1,"offset":8},"color":0})).await.0, 200);
    assert_eq!(post(&router, "/api/excerpts", serde_json::json!({"book_id": id, "chapter": 0, "text": "A saved excerpt", "note": "Keep this independent note"})).await.0, 200);
    let external = root.path().join("user-original.epub");
    std::fs::write(&external, epub()).unwrap();
    assert_eq!(
        post(&router, &path, serde_json::json!({"action":"delete"}))
            .await
            .0,
        200
    );
    assert!(external.exists());
    assert!(!root.path().join("books").join(id).exists());
    assert_eq!(get(&router, &reader_path).await.0, 404);
    assert!(get(&router, "/").await.2.contains("从一本书开始"));
    let highlights: i64 = sqlx::query_scalar("SELECT count(*) FROM web_highlights")
        .fetch_one(&state.storage)
        .await
        .unwrap();
    assert_eq!(highlights, 0);
    let detached_notes: i64 =
        sqlx::query_scalar("SELECT count(*) FROM notes WHERE book_id IS NULL")
            .fetch_one(&state.storage)
            .await
            .unwrap();
    assert_eq!(detached_notes, 1);
    assert_eq!(
        post(
            &router,
            &path,
            serde_json::json!({"action":"rename", "title":"stale"})
        )
        .await
        .0,
        400
    );
    assert_eq!(
        post(&router, &path, serde_json::json!({"action":"delete"}))
            .await
            .0,
        400
    );
}

#[tokio::test]
async fn web_highlights_validate_utf8_ranges_and_do_not_touch_native_coordinates() {
    let root = tempfile::tempdir().unwrap();
    let assets = tempfile::tempdir().unwrap();
    let state = State::open(root.path().to_owned(), vec!["localhost:3000".into()])
        .await
        .unwrap();
    let router = router(state.clone(), test_assets(assets.path()));
    let (_, _, imported) = request(
        &router,
        "POST",
        "/api/import",
        "multipart/form-data; boundary=test-boundary",
        multipart(&epub(), true),
        "localhost:3000",
        true,
    )
    .await;
    let book: serde_json::Value = serde_json::from_str(&imported).unwrap();
    let id = book["id"].as_str().unwrap();
    let mut payload = serde_json::json!({"book_id": id, "chapter": 0, "start": {"block": 1,"offset": 0}, "end": {"block": 1,"offset": 8}, "color": 2});
    let (status, _, added) = post(&router, "/api/highlights", payload.clone()).await;
    assert_eq!(status, 200);
    let added: serde_json::Value = serde_json::from_str(&added).unwrap();
    let path = format!("/api/highlights/{id}/0");
    let list: serde_json::Value = serde_json::from_str(&get(&router, &path).await.2).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["color"], 2);
    for offset in [24, 10000] {
        payload["end"]["offset"] = offset.into();
        assert_eq!(
            post(&router, "/api/highlights", payload.clone()).await.0,
            400
        );
    }
    payload["end"]["offset"] = 8.into();
    payload["color"] = 4.into();
    assert_eq!(
        post(&router, "/api/highlights", payload.clone()).await.0,
        400
    );
    payload["color"] = 1.into();
    payload["chapter"] = 100.into();
    assert_eq!(
        post(&router, "/api/highlights", payload.clone()).await.0,
        400
    );
    let native: i64 = sqlx::query_scalar("SELECT count(*) FROM book_notes WHERE type='annotation'")
        .fetch_one(&state.storage)
        .await
        .unwrap();
    assert_eq!(native, 0);
    let (_, _, removed) = post(
        &router,
        "/api/highlights/remove",
        serde_json::json!({"book_id": id, "chapter": 1, "ids": [added["id"]]}),
    )
    .await;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&removed).unwrap()["removed"],
        0
    );
    assert_eq!(
        post(
            &router,
            "/api/highlights/remove",
            serde_json::json!({"book_id": id,"chapter": 0,"ids": [added["id"]]})
        )
        .await
        .0,
        200
    );
    assert_eq!(get(&router, &path).await.2, "[]");
    assert_eq!(
        post(&router, "/api/marker", serde_json::json!({"slot": 7}))
            .await
            .0,
        200
    );
    assert_eq!(
        post(&router, "/api/marker", serde_json::json!({"slot": 8}))
            .await
            .0,
        400
    );
}

#[tokio::test]
async fn colour_swatches_apply_atomically_preserve_outside_ranges_and_are_retry_safe() {
    let root = tempfile::tempdir().unwrap();
    let assets = tempfile::tempdir().unwrap();
    let state = State::open(root.path().to_owned(), vec!["localhost:3000".into()])
        .await
        .unwrap();
    let router = router(state.clone(), test_assets(assets.path()));
    let (_, _, imported) = request(
        &router,
        "POST",
        "/api/import",
        "multipart/form-data; boundary=test-boundary",
        multipart(&epub(), true),
        "localhost:3000",
        true,
    )
    .await;
    let book: serde_json::Value = serde_json::from_str(&imported).unwrap();
    let id = book["id"].as_str().unwrap();
    let path = format!("/api/highlights/{id}/0");
    let mut payload = serde_json::json!({"book_id":id,"chapter":0,"start":{"block":1,"offset":0},"end":{"block":3,"offset":33},"color":3});
    assert_eq!(
        post(&router, "/api/highlights/apply", payload.clone())
            .await
            .0,
        200
    );
    let foreign = serde_json::json!({"book_id":id,"chapter":1,"start":{"block":1,"offset":0},"end":{"block":1,"offset":8},"color":2});
    assert_eq!(post(&router, "/api/highlights", foreign).await.0, 200);
    let other_path = format!("/api/highlights/{id}/1");
    let other_before = get(&router, &other_path).await.2;
    payload["start"]["offset"] = 9.into();
    payload["end"]["offset"] = 8.into();
    // Reapplying a colour is idempotent in range/colour, never the pen toggle.
    for color in [0, 0, 1] {
        payload["color"] = color.into();
        let (status, _, body) = post(&router, "/api/highlights/apply", payload.clone()).await;
        assert_eq!(status, 200);
        let rows: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 3);
        for (index, start, end, colour) in [
            (
                0,
                serde_json::json!({"block":1,"offset":0}),
                payload["start"].clone(),
                3,
            ),
            (1, payload["start"].clone(), payload["end"].clone(), color),
            (
                2,
                payload["end"].clone(),
                serde_json::json!({"block":3,"offset":33}),
                3,
            ),
        ] {
            assert_eq!(rows[index]["start"], start);
            assert_eq!(rows[index]["end"], end);
            assert_eq!(rows[index]["color"], colour);
        }
    }
    let nested = serde_json::json!({"book_id":id,"chapter":0,"start":{"block":2,"offset":0},"end":{"block":2,"offset":8},"color":2});
    assert_eq!(post(&router, "/api/highlights", nested).await.0, 200);
    payload["color"] = 0.into();
    let (_, _, body) = post(&router, "/api/highlights/apply", payload.clone()).await;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let before = get(&router, &path).await.2;
    for invalid in [
        serde_json::json!({"color":4}),
        serde_json::json!({"chapter":999}),
        serde_json::json!({"book_id":"not-a-uuid"}),
        serde_json::json!({"start":{"block":1,"offset":24}}),
        serde_json::json!({"end":payload["start"]}),
    ] {
        let mut bad = payload.clone();
        for (key, value) in invalid.as_object().unwrap() {
            bad[key] = value.clone();
        }
        assert_eq!(post(&router, "/api/highlights/apply", bad).await.0, 400);
        assert_eq!(get(&router, &path).await.2, before);
    }
    assert_eq!(
        request(
            &router,
            "POST",
            "/api/highlights/apply",
            "application/json",
            serde_json::to_vec(&payload).unwrap(),
            "localhost:3000",
            false
        )
        .await
        .0,
        403
    );
    sqlx::query("CREATE TRIGGER fail_colour_apply BEFORE INSERT ON web_highlights WHEN NEW.color=1 BEGIN SELECT RAISE(ABORT,'owned rollback fixture'); END").execute(&state.storage).await.unwrap();
    let subset = serde_json::json!({"book_id":id,"chapter":0,"start":{"block":2,"offset":1},"end":{"block":2,"offset":7},"color":1});
    assert!(
        post(&router, "/api/highlights/apply", subset.clone())
            .await
            .0
            .is_server_error()
    );
    assert_eq!(
        get(&router, &path).await.2,
        before,
        "failed insert must roll back overlap deletion"
    );
    sqlx::query("DROP TRIGGER fail_colour_apply")
        .execute(&state.storage)
        .await
        .unwrap();
    for _ in 0..2 {
        let (status, _, body) = post(&router, "/api/highlights/apply", subset.clone()).await;
        assert_eq!(status, 200);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&body)
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            5
        );
    }
    assert_eq!(get(&router, &other_path).await.2, other_before);
    let native: i64 = sqlx::query_scalar("SELECT count(*) FROM book_notes WHERE type='annotation'")
        .fetch_one(&state.storage)
        .await
        .unwrap();
    assert_eq!(native, 0);
}

#[tokio::test]
async fn ai_configuration_redacts_secrets_and_rejects_unsafe_addresses() {
    let root = tempfile::tempdir().unwrap();
    let assets = tempfile::tempdir().unwrap();
    let state = State::open(root.path().to_owned(), vec!["localhost:3000".into()])
        .await
        .unwrap();
    let router = router(state.clone(), test_assets(assets.path()));
    for endpoint in [
        "file:///etc/passwd",
        "https://user:password@example.com/v1",
        "https://example.com/v1?key=secret",
        "https://example.com/v1#fragment",
    ] {
        assert_eq!(
            post(
                &router,
                "/api/ai/settings",
                serde_json::json!({"endpoint": endpoint,"model": "model","api_key": null})
            )
            .await
            .0,
            400
        );
    }
    let mut value = serde_json::json!({"endpoint": "http://127.0.0.1:8080", "model": "qa", "api_key": "private-test-secret"});
    assert_eq!(
        post(&router, "/api/ai/settings", value.clone()).await.0,
        200
    );
    let get_config = get(&router, "/api/ai/settings").await.2;
    assert!(!get_config.contains("private-test-secret"));
    assert!(!get_config.contains("\"api_key\""));
    assert!(get_config.contains("has_api_key\":true"));
    value["api_key"] = serde_json::Value::Null;
    assert_eq!(
        post(&router, "/api/ai/settings", value.clone()).await.0,
        200
    );
    assert_eq!(
        state
            .call(|l| Ok(l.db.load_ai_settings()?.api_key))
            .await
            .unwrap(),
        "private-test-secret"
    );
    value["clear_key"] = true.into();
    assert_eq!(post(&router, "/api/ai/settings", value).await.0, 200);
    assert!(
        state
            .call(|l| Ok(l.db.load_ai_settings()?.api_key))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        !get(&router, "/settings")
            .await
            .2
            .contains("private-test-secret")
    );
}

#[tokio::test]
async fn excerpts_validate_input_and_persist_as_real_book_notes() {
    let root = tempfile::tempdir().unwrap();
    let assets = tempfile::tempdir().unwrap();
    let state = State::open(root.path().to_owned(), vec!["localhost:3000".into()])
        .await
        .unwrap();
    let router = router(state.clone(), test_assets(assets.path()));
    let (status, _, imported) = request(
        &router,
        "POST",
        "/api/import",
        "multipart/form-data; boundary=test-boundary",
        multipart(&epub(), true),
        "localhost:3000",
        true,
    )
    .await;
    assert_eq!(status, 200);
    let book: serde_json::Value = serde_json::from_str(&imported).unwrap();
    let id = book["id"].as_str().unwrap();
    let path = format!("/excerpts/{id}");
    assert!(get(&router, &path).await.2.contains("选中正文后"));
    assert_eq!(get(&router, "/excerpts/not-a-book").await.0, 404);
    assert_eq!(
        get(&router, "/excerpts/00000000-0000-0000-0000-000000000000")
            .await
            .0,
        404
    );
    for text in ["".to_owned(), " ".to_owned(), "🌿".repeat(6001)] {
        assert_eq!(
            post(
                &router,
                "/api/excerpts",
                serde_json::json!({"book_id": id, "chapter": 0, "text": text})
            )
            .await
            .0,
            400
        );
    }
    assert_eq!(
        post(
            &router,
            "/api/excerpts",
            serde_json::json!({"book_id": id, "chapter": 100, "text": "Missing chapter"})
        )
        .await
        .0,
        400
    );
    assert_eq!(
        request(
            &router,
            "POST",
            "/api/excerpts",
            "application/json",
            serde_json::to_vec(
                &serde_json::json!({"book_id": id, "chapter": 0, "text": "Forbidden"})
            )
            .unwrap(),
            "localhost:3000",
            false
        )
        .await
        .0,
        403
    );
    let text = "solitude 中文 🌿\n\n<script>alert(1)</script>";
    let (status, _, saved) = post(
        &router,
        "/api/excerpts",
        serde_json::json!({"book_id": id, "chapter": 0, "text": text}),
    )
    .await;
    assert_eq!(status, 200);
    assert!(
        serde_json::from_str::<serde_json::Value>(&saved).unwrap()["saved"]
            .as_bool()
            .unwrap()
    );
    let id_for_db = id.to_owned();
    let notes = state
        .call(move |library| library.db.get_notes_by_book(&id_for_db))
        .await
        .unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].content, format!("{text}\n\n—— Forest"));
    let html = get(&router, &path).await.2;
    assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(!html.contains("<script>alert(1)</script>"));
    let unknown = "00000000-0000-0000-0000-000000000000";
    assert_eq!(
        post(
            &router,
            "/api/excerpts",
            serde_json::json!({"book_id": unknown, "chapter": 0, "text": "Missing book"})
        )
        .await
        .0,
        400
    );
}

#[tokio::test]
async fn topcoat_library_reader_mutations_and_persistence_round_trip() {
    let root = tempfile::tempdir().unwrap();
    let assets = tempfile::tempdir().unwrap();
    let state = State::open(root.path().to_owned(), vec!["localhost:3000".into()])
        .await
        .unwrap();
    let router = router(state.clone(), test_assets(assets.path()));
    let (status, headers, html) = get(&router, "/").await;
    assert_eq!(status, 200);
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert!(html.contains("从一本书开始"));
    assert_eq!(get(&router, "/read/not-a-uuid/0").await.0, 404);
    assert_eq!(
        get(&router, "/read/00000000-0000-0000-0000-000000000000/0")
            .await
            .0,
        404
    );

    let data = multipart(&epub(), true);
    let (status, _, json) = request(
        &router,
        "POST",
        "/api/import",
        "multipart/form-data; boundary=test-boundary",
        data,
        "localhost:3000",
        true,
    )
    .await;
    assert_eq!(status, 200, "{json}");
    let result: serde_json::Value = serde_json::from_str(&json).unwrap();
    let id = result["id"].as_str().unwrap();
    assert_eq!(result["title"], "中文 EPUB");
    assert!(
        root.path()
            .join("uploads")
            .read_dir()
            .unwrap()
            .next()
            .is_none()
    );
    let (status, _, html) = get(&router, &format!("/read/{id}/0")).await;
    assert_eq!(status, 200, "{html}");
    assert!(html.contains("data-lemma=\"solitude\""));
    assert!(html.contains("data-known=\"false\""));
    assert!(!html.contains(&root.path().display().to_string()));
    assert_eq!(get(&router, &format!("/read/{id}/9999")).await.0, 404);

    assert_eq!(
        post(
            &router,
            "/api/known",
            serde_json::json!({"lemma":"solitude","known":true})
        )
        .await
        .0,
        200
    );
    let known = get(&router, "/api/known").await.2;
    assert!(known.contains("solitude"));
    let marked = get(&router, &format!("/read/{id}/0")).await.2;
    // A refresh/revisit must preserve all the same ruby slots, not just hide a
    // currently mounted client component while rebuilding different HTML.
    assert_eq!(
        html.matches("<ruby ").count(),
        marked.matches("<ruby ").count()
    );
    assert!(marked.contains("data-lemma=\"solitude\" data-known=\"true\""));
    assert_eq!(
        post(
            &router,
            "/api/known",
            serde_json::json!({"lemma":"<script>","known":true})
        )
        .await
        .0,
        400
    );

    let entry = serde_json::json!({"lemma":"solitude","word":"solitude","gloss":"独处 <img onerror=alert(1)>","level":null,"contextSentence":null,"bookId":id});
    assert_eq!(post(&router, "/api/vocabulary", entry).await.0, 200);
    let vocabulary = get(&router, "/vocabulary").await.2;
    assert!(vocabulary.contains("独处 &lt;img"));
    assert!(!vocabulary.contains("<img onerror"));

    let prefs = serde_json::json!({"gloss_color":0,"gloss_background":1,"word_color":0,"word_background":2,"gloss_enabled":true,"font_size":24,"gloss_size":11.2,"english_level":"B1"});
    assert_eq!(
        post(&router, "/api/preferences", prefs.clone()).await.0,
        200
    );
    let mut invalid = prefs;
    invalid["word_background"] = serde_json::json!(99);
    assert_eq!(post(&router, "/api/preferences", invalid).await.0, 400);
    assert!(get(&router, "/settings").await.2.contains("#FFE9A8"));

    assert_eq!(
        post(
            &router,
            "/api/progress",
            serde_json::json!({"book_id":id,"location":{"chapter":1,"block":1,"offset":3}})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        post(
            &router,
            "/api/progress",
            serde_json::json!({"book_id":id,"location":{"chapter":100,"block":0,"offset":0}})
        )
        .await
        .0,
        400
    );
    let (status, headers, _) = get(&router, &format!("/read/{id}")).await;
    assert_eq!(status, 303);
    assert_eq!(headers["location"], format!("/read/{id}/1"));
    let html = get(&router, &format!("/read/{id}/1")).await.2;
    assert!(html.contains("data-resume-block=\"1\" data-resume-offset=\"3\""));
    assert_eq!(get(&router, &format!("/media/{id}/1000")).await.0, 404);

    drop(router);
    drop(state);
    let reopened = State::open(root.path().to_owned(), vec![]).await.unwrap();
    let (known, prefs, saved) = reopened
        .call(move |library| {
            Ok((
                library.db.get_known_words()?,
                library.db.load_reader_settings()?,
                library.db.get_shelf_items()?,
            ))
        })
        .await
        .unwrap();
    assert!(known.contains(&"solitude".into()));
    assert_eq!(prefs.word_background_slot, 2);
    assert_eq!(saved.len(), 1);
    drop(reopened);
}

#[tokio::test]
async fn requests_enforce_origin_host_limits_and_upload_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let assets = tempfile::tempdir().unwrap();
    let state = State::open(root.path().to_owned(), vec!["localhost:3000".into()])
        .await
        .unwrap();
    let router = router(state.clone(), test_assets(assets.path()));
    assert_eq!(
        request(
            &router,
            "GET",
            "/api/health",
            "application/json",
            vec![],
            "evil.invalid:3000",
            false
        )
        .await
        .0,
        403
    );
    assert_eq!(
        request(
            &router,
            "POST",
            "/api/known",
            "application/json",
            b"{}".to_vec(),
            "localhost:3000",
            false
        )
        .await
        .0,
        403
    );
    let foreign = http::Request::builder()
        .method("POST")
        .uri("/api/known")
        .header("host", "localhost:3000")
        .header("origin", "https://evil.invalid")
        .header("x-readwise-request", "1")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    assert_eq!(router.handle(foreign).await.status(), 403);
    let oversized = " ".repeat(70 * 1024);
    assert_eq!(
        request(
            &router,
            "POST",
            "/api/known",
            "application/json",
            oversized.into_bytes(),
            "localhost:3000",
            true
        )
        .await
        .0,
        413
    );
    for data in [
        multipart(b"not an epub", true),
        multipart(b"interrupted upload", false),
    ] {
        let (status, _, body) = request(
            &router,
            "POST",
            "/api/import",
            "multipart/form-data; boundary=test-boundary",
            data,
            "localhost:3000",
            true,
        )
        .await;
        assert!(status.is_client_error(), "{status}: {body}");
        assert!(
            root.path()
                .join("uploads")
                .read_dir()
                .unwrap()
                .next()
                .is_none()
        );
    }
    assert!(
        state
            .call(|library| library.db.get_shelf_items())
            .await
            .unwrap()
            .is_empty()
    );
    drop(router);
    drop(state);
}

#[tokio::test]
async fn partial_preferences_preserve_unrelated_native_and_web_fields() {
    let root = tempfile::tempdir().unwrap();
    let assets = tempfile::tempdir().unwrap();
    let state = State::open(root.path().to_owned(), vec!["localhost:3000".into()])
        .await
        .unwrap();
    let router = router(state.clone(), test_assets(assets.path()));
    for body in [
        r#"{"theme":"light","font":"seravek","spacing":2.1,"font_size":26,"columns":"two","background":"sage"}"#,
        r#"{"gloss_background":3}"#,
    ] {
        let (status, _, result) = request(
            &router,
            "POST",
            "/api/preferences",
            "application/json",
            body.as_bytes().to_vec(),
            "localhost:3000",
            true,
        )
        .await;
        assert_eq!(status, 200, "{result}");
    }
    let (_, _, json) = get(&router, "/api/preferences").await;
    let p: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(p["theme"], "light");
    assert_eq!(p["font"], "seravek");
    assert_eq!(p["font_size"], 26.0);
    assert_eq!(p["columns"], "two");
    assert_eq!(p["background"], "sage");
    assert_eq!(p["gloss_background"], 3);
    for body in [
        "null",
        "[]",
        r#"{"unknown_field":3}"#,
        r#"{"font_size":999}"#,
        r#"{"gloss_background":-1}"#,
    ] {
        assert_eq!(
            request(
                &router,
                "POST",
                "/api/preferences",
                "application/json",
                body.as_bytes().to_vec(),
                "localhost:3000",
                true
            )
            .await
            .0,
            400
        );
    }
    assert_eq!(
        get(&router, "/api/preferences").await.2,
        json,
        "invalid partial updates must not save anything"
    );
}

#[tokio::test]
async fn data_directory_lock_and_abandoned_upload_recovery_are_scoped() {
    let root = tempfile::tempdir().unwrap();
    let uploads = root.path().join("uploads");
    std::fs::create_dir(&uploads).unwrap();
    std::fs::write(uploads.join("readwise-abandoned.epub"), b"partial").unwrap();
    std::fs::create_dir(uploads.join("readwise-abandoned-directory")).unwrap();
    std::fs::write(
        uploads.join("readwise-abandoned-directory/partial.pdf"),
        b"partial",
    )
    .unwrap();
    std::fs::write(uploads.join("user-original.epub"), b"do not delete").unwrap();
    let state = State::open(root.path().to_owned(), vec![]).await.unwrap();
    assert!(!uploads.join("readwise-abandoned.epub").exists());
    assert!(!uploads.join("readwise-abandoned-directory").exists());
    assert!(uploads.join("user-original.epub").exists());
    assert!(State::open(root.path().to_owned(), vec![]).await.is_err());
    drop(state);
    drop(State::open(root.path().to_owned(), vec![]).await.unwrap());
}

#[tokio::test]
async fn pdf_displaced_sum_keeps_remote_highlights_anchors_and_snapshot_on_restart() {
    let root = tempfile::tempdir().unwrap();
    let assets = tempfile::tempdir().unwrap();
    let state = State::open(root.path().to_owned(), vec!["localhost:3000".into()])
        .await
        .unwrap();
    let router = router(state.clone(), test_assets(assets.path()));
    let bytes = include_bytes!("../tests/fixtures/math-scattered.pdf");
    let (status, _, body) = request(
        &router,
        "POST",
        "/api/import",
        "multipart/form-data; boundary=test-boundary",
        multipart_file(bytes, true, "scattered.pdf"),
        "localhost:3000",
        true,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let imported: serde_json::Value = serde_json::from_str(&body).unwrap();
    let id = imported["id"].as_str().unwrap().to_owned();
    let (block, offset) = state
        .call({
            let id = id.clone();
            move |l| {
                let book = l.book(&id)?.unwrap();
                let content = l.content(&book)?;
                let (block, text) = content.chapters[0]
                    .text
                    .iter()
                    .enumerate()
                    .find(|(_, text)| text.contains('P'))
                    .unwrap();
                Ok((block, text.find('P').unwrap()))
            }
        })
        .await
        .unwrap();
    let directory = root.path().join("books").join(&id);
    let snapshots = || {
        directory
            .read_dir()
            .unwrap()
            .map(|e| e.unwrap())
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .map(|e| (e.path(), std::fs::read(e.path()).unwrap()))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let before = snapshots();
    assert_eq!(
        post(
            &router,
            "/api/progress",
            serde_json::json!({"book_id":id,"location":{"chapter":0,"block":block,"offset":offset}})
        )
        .await
        .0,
        200
    );
    assert_eq!(post(&router,"/api/highlights",serde_json::json!({"book_id":id,"chapter":0,"start":{"block":block,"offset":offset},"end":{"block":block,"offset":offset+1},"color":2})).await.0,200);
    let (_, _, html) = get(&router, &format!("/read/{id}/0")).await;
    assert!(html.contains("data-math-symbols=\"∑\""));
    assert!(html.contains("math-remote-source"));
    assert_eq!(snapshots(), before);
    drop(router);
    drop(state);
    let reopened = State::open(root.path().to_owned(), vec!["localhost:3000".into()])
        .await
        .unwrap();
    let projection = reopened
        .call({
            let id = id.clone();
            move |l| Ok(l.reader(&id, 0)?.unwrap())
        })
        .await
        .unwrap();
    assert_eq!(projection.resume.block, block);
    assert_eq!(projection.resume.offset, offset);
    let router = super::router(reopened, test_assets(assets.path()));
    let marks: serde_json::Value =
        serde_json::from_str(&get(&router, &format!("/api/highlights/{id}/0")).await.2).unwrap();
    assert_eq!(marks.as_array().unwrap().len(), 1);
    assert_eq!(marks[0]["start"]["offset"], offset);
    assert_eq!(marks[0]["end"]["offset"], offset + 1);
    assert_eq!(snapshots(), before);
    assert_eq!(std::fs::read(directory.join("book.pdf")).unwrap(), bytes);
}

#[tokio::test]
async fn pdf_typst_keeps_snapshot_utf8_anchors_and_existing_highlights() {
    let root = tempfile::tempdir().unwrap();
    let assets = tempfile::tempdir().unwrap();
    let state = State::open(root.path().to_owned(), vec!["localhost:3000".into()])
        .await
        .unwrap();
    let router = router(state.clone(), test_assets(assets.path()));
    let source = include_bytes!("../tests/fixtures/equations.pdf");
    let (status, _, body) = request(
        &router,
        "POST",
        "/api/import",
        "multipart/form-data; boundary=test-boundary",
        multipart_file(source, true, "equations.pdf"),
        "localhost:3000",
        true,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let imported: serde_json::Value = serde_json::from_str(&body).unwrap();
    let id = imported["id"].as_str().unwrap().to_owned();
    let (canonical, math_block) = state
        .call({
            let id = id.clone();
            move |l| {
                let book = l.book(&id)?.unwrap();
                let content = l.content(&book)?;
                let text = content.chapters[0].text.clone();
                let block = text.iter().position(|t| t.contains("f(x)")).unwrap();
                Ok((text, block))
            }
        })
        .await
        .unwrap();
    let directory = root.path().join("books").join(&id);
    let snapshots = || {
        directory
            .read_dir()
            .unwrap()
            .map(|entry| entry.unwrap())
            .filter(|f| f.path().extension().is_some_and(|e| e == "json"))
            .map(|f| (f.path(), std::fs::read(f.path()).unwrap()))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let before = snapshots();
    let location = serde_json::json!({"chapter":0,"block":math_block,"offset":2});
    assert_eq!(
        post(
            &router,
            "/api/progress",
            serde_json::json!({"book_id":id,"location":location})
        )
        .await
        .0,
        200
    );
    assert_eq!(post(&router, "/api/highlights", serde_json::json!({"book_id":id,"chapter":0,"start":{"block":math_block,"offset":0},"end":{"block":math_block,"offset":2},"color":1})).await.0, 200);
    let path = format!("/read/{id}/0");
    let (status, _, html) = get(&router, &path).await;
    assert_eq!(status, 200);
    assert!(html.contains("data-math-renderer=\"typst\"") && html.contains("<svg"));
    assert!(html.contains("data-lemma=\"solitude\""));
    assert!(!html.contains("class=\"rw-math-"));
    let projection = state
        .call({
            let id = id.clone();
            move |l| Ok(l.reader(&id, 0)?.unwrap())
        })
        .await
        .unwrap();
    assert_eq!(projection.resume.block, math_block);
    assert_eq!(projection.resume.offset, 2);
    let text: Vec<_> = projection
        .blocks
        .iter()
        .map(|html| {
            let cleaned = ammonia::Builder::default()
                .tags(Default::default())
                .clean_content_tags(["rt", "rp", "svg", "script", "style"].into_iter().collect())
                .clean(html)
                .to_string();
            content::plain_text(&cleaned)
        })
        .collect();
    assert_eq!(text, canonical);
    assert!(
        projection
            .blocks
            .iter()
            .map(|b| b.matches("data-math-renderer=\"typst\"").count())
            .sum::<usize>()
            >= 3
    );
    assert_eq!(
        post(
            &router,
            "/api/known",
            serde_json::json!({"lemma":"solitude","known":true})
        )
        .await
        .0,
        200
    );
    let (_, _, known) = get(&router, &path).await;
    assert!(known.contains("data-known=\"true\""));
    assert_eq!(
        known.matches("data-math-renderer=\"typst\"").count(),
        html.matches("data-math-renderer=\"typst\"").count()
    );
    let marks: serde_json::Value =
        serde_json::from_str(&get(&router, &format!("/api/highlights/{id}/0")).await.2).unwrap();
    assert_eq!(marks.as_array().unwrap().len(), 1);
    assert_eq!(marks[0]["end"]["offset"], 2);
    assert_eq!(snapshots(), before);
    assert_eq!(std::fs::read(directory.join("book.pdf")).unwrap(), source);
}

#[tokio::test]
async fn pdf_import_uses_the_existing_snapshot_and_ruby_pipeline() {
    let root = tempfile::tempdir().unwrap();
    let assets = tempfile::tempdir().unwrap();
    let state = State::open(root.path().to_owned(), vec!["localhost:3000".into()])
        .await
        .unwrap();
    let router = router(state.clone(), test_assets(assets.path()));
    let source = include_bytes!("../../core/tests/fixtures/reader-text.pdf");
    let (status, _, body) = request(
        &router,
        "POST",
        "/api/import",
        "multipart/form-data; boundary=test-boundary",
        multipart_file(source, true, "文字阅读.PDF"),
        "localhost:3000",
        true,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let result: serde_json::Value = serde_json::from_str(&body).unwrap();
    let id = result["id"].as_str().unwrap();
    let (status, _, html) = get(&router, &format!("/read/{id}/0")).await;
    assert_eq!(status, 200);
    assert!(html.contains("data-lemma=\"solitude\""));
    assert!(html.contains("reader"));
    assert!(
        root.path()
            .join("uploads")
            .read_dir()
            .unwrap()
            .next()
            .is_none()
    );
    assert_eq!(
        std::fs::read(root.path().join("books").join(id).join("book.pdf")).unwrap(),
        source
    );
    assert!(
        root.path()
            .join("books")
            .join(id)
            .join("pdf-text-v1.json")
            .exists()
    );
    // Remove only the metadata key, preserving byte lengths/xref positions.
    // A title-less PDF must fall back to the user's basename, not a random
    // receiving path. The original checked-in source is never modified.
    let mut without_title = source.to_vec();
    let start = without_title
        .windows(6)
        .position(|value| value == b"/Title")
        .unwrap();
    without_title[start..start + 6].copy_from_slice(b"/Other");
    let (status, _, json) = request(
        &router,
        "POST",
        "/api/import",
        "multipart/form-data; boundary=test-boundary",
        multipart_file(&without_title, true, "无标题中文书.PDF"),
        "localhost:3000",
        true,
    )
    .await;
    assert_eq!(status, 200, "{json}");
    let imported: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(imported["title"], "无标题中文书");
    assert!(
        root.path()
            .join("uploads")
            .read_dir()
            .unwrap()
            .next()
            .is_none()
    );
    drop(router);
    drop(state);
}

#[tokio::test]
async fn reader_uses_one_request_snapshot_even_if_settings_change_before_render() {
    let root = tempfile::tempdir().unwrap();
    let assets = tempfile::tempdir().unwrap();
    let state = State::open(root.path().to_owned(), vec!["localhost:3000".into()])
        .await
        .unwrap();
    let router = router(state.clone(), test_assets(assets.path()));
    let (status, _, result) = request(
        &router,
        "POST",
        "/api/import",
        "multipart/form-data; boundary=test-boundary",
        multipart(&epub(), true),
        "localhost:3000",
        true,
    )
    .await;
    assert_eq!(status, 200, "{result}");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    let id = result["id"].as_str().unwrap().to_owned();
    state
        .call(move |library| {
            let mut snapshot = settings::Preferences::load(&library.db)?;
            snapshot.language = "en".into();
            snapshot.english_level = "A1".into();
            snapshot.font_size = 26.5;
            let mut newer = snapshot.clone();
            newer.language = "zh-CN".into();
            newer.english_level = "C3".into();
            newer.font_size = 32.0;
            newer.save(&library.db)?;
            let reader = library
                .reader_with_preferences(&id, 0, snapshot.clone())?
                .unwrap();
            assert_eq!(reader.preferences, snapshot);
            assert!(
                reader.blocks.iter().any(|block| block.contains("<ruby")),
                "annotation must use the request's A1 snapshot, not the newer database C3 setting"
            );
            assert_eq!(settings::Preferences::load(&library.db)?, newer);
            Ok(())
        })
        .await
        .unwrap();
}
