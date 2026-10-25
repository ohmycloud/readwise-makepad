mod assistant;
mod content;
mod controls;
mod i18n;
#[cfg(test)]
mod integration_tests;
mod library;
mod math;
mod pages;
mod reading;
mod routes;
mod selection;
mod settings;
mod shelf;
mod syntax;

use library::{MAX_UPLOAD, State};
use std::{net::SocketAddr, path::PathBuf};
use topcoat::{
    asset::{AssetBundle, RouterBuilderAssetExt},
    context::{Cx, app_context},
    router::{
        Body, BodyLimit, LayerFn, Next, Router, RouterBuilderDiscoverExt,
        error::{bad_request, forbidden},
        module_router, request,
        response::Response,
    },
    runtime::RouterBuilderRuntimeExt,
};

fn router(state: State, assets: AssetBundle) -> Router {
    module_router!()
        .discover()
        .app_context(state)
        .layer(BodyLimit::max(64 * 1024))
        .layer(BodyLimit::max(MAX_UPLOAD + 1024 * 1024).at("/api/import"))
        .assets(assets)
        .layer(LayerFn::new(None::<&str>, |cx, body, next| {
            Box::pin(interface_context(cx, body, next))
        }))
        .runtime()
        .layer(LayerFn::new(None::<&str>, |cx, body, next| {
            Box::pin(security(cx, body, next))
        }))
        .build()
}

/// One immutable settings snapshot supplies locale, shell, controls and reader.
/// Assets and mutation endpoints do not need a settings query or a stale snapshot.
async fn interface_context(cx: &Cx, body: Body, next: Next<'_>) -> topcoat::Result<Response> {
    let path = request::uri(cx).path();
    if matches!(path, "/" | "/settings" | "/vocabulary" | "/statistics")
        || path.starts_with("/read/")
        || path.starts_with("/excerpts/")
    {
        let preferences = app_context::<State>(cx)
            .call(|library| settings::Preferences::load(&library.db))
            .await?;
        let locale = if preferences.language == "en" {
            topcoat::l10n::locale!("en")
        } else {
            topcoat::l10n::locale!("zh-CN")
        };
        next.run(&cx.with(locale).with(preferences), body).await
    } else {
        next.run(cx, body).await
    }
}

async fn security(cx: &Cx, body: Body, next: Next<'_>) -> topcoat::Result<Response> {
    let headers = request::headers(cx);
    let state = app_context::<State>(cx);
    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !state.allowed_hosts.iter().any(|allowed| allowed == host) {
        return Err(forbidden().into());
    }
    // A hostile website cannot send this non-simple header without a CORS
    // preflight. No CORS opt-in is provided. OriginPolicy adds its own checks.
    if request::uri(cx).path().starts_with("/api/")
        && !matches!(
            *request::method(cx),
            topcoat::router::Method::GET | topcoat::router::Method::HEAD
        )
        && headers
            .get("x-readwise-request")
            .and_then(|v| v.to_str().ok())
            != Some("1")
    {
        return Err(forbidden().into());
    }
    let mut response = next.run(cx, body).await?;
    for (name, value) in [
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "no-referrer"),
        ("cache-control", "no-store"),
        (
            "content-security-policy",
            "object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
        ),
    ] {
        response.headers_mut().insert(
            topcoat::router::HeaderName::from_static(name),
            topcoat::router::HeaderValue::from_static(value),
        );
    }
    Ok(response)
}

fn bind_address(value: &str) -> topcoat::Result<SocketAddr> {
    let address: SocketAddr = value
        .parse()
        .map_err(|_| bad_request("READWISE_WEB_BIND 必须是 IP:端口"))?;
    if !address.ip().is_loopback() {
        return Err(
            bad_request("当前 Web 版本仅支持回环地址；公网部署须先实现认证与访问隔离").into(),
        );
    }
    Ok(address)
}

#[tokio::main]
async fn main() -> topcoat::Result<()> {
    let address = bind_address(
        &std::env::var("READWISE_WEB_BIND").unwrap_or_else(|_| "127.0.0.1:3000".into()),
    )?;
    // Load assets before opening the native Db runtime so a missing bundle
    // produces a useful error without affecting the database.
    let assets = AssetBundle::load().map_err(|e| {
        topcoat::Error::msg(format!(
            "加载 Web 资源失败：{e}。请先执行 topcoat asset bundle -p readwise-web"
        ))
    })?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    let address = listener.local_addr()?;
    let data_dir = std::env::var_os("READWISE_WEB_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| readwise_core::paths::app_data_dir().join("web"));
    tokio::fs::create_dir_all(&data_dir).await?;
    let data_dir = tokio::fs::canonicalize(data_dir).await?;
    let hosts = vec![address.to_string(), format!("localhost:{}", address.port())];
    let state = State::open(data_dir.clone(), hosts).await?;
    eprintln!(
        "ReadWise Web: http://{address}\n数据目录：{}\n仅本机单用户；按 Ctrl+C 停止。",
        data_dir.display()
    );
    topcoat::serve_until(listener, router(state, assets), async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unauthenticated_preview_can_only_bind_loopback() {
        assert!(bind_address("127.0.0.1:3000").is_ok());
        assert!(bind_address("[::1]:3000").is_ok());
        for address in ["0.0.0.0:3000", "192.168.1.1:3000", "localhost:3000"] {
            assert!(bind_address(address).is_err());
        }
    }
}
