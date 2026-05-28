//! Embed the built frontend (`frontend/dist`) into the backend binary and serve it as a SPA.
//!
//! At build time, the frontend must be built (`pnpm build`). The `frontend/dist/.gitkeep` ensures
//! the directory exists even when the build has not yet run, so `cargo build` succeeds on a fresh
//! checkout — in that case the embedded set is empty and `/` returns a small placeholder page.

use axum::body::Body;
use axum::extract::Path;
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../frontend/dist/"]
struct Asset;

pub async fn serve_root() -> Response {
    serve_inner("index.html").await
}

pub async fn serve_path(Path(path): Path<String>) -> Response {
    // Anything with an extension is asset-like; serve the file or 404.
    // Anything else — SPA route — falls back to index.html.
    if path.contains('.') {
        let resp = serve_inner(&path).await;
        if resp.status() == StatusCode::NOT_FOUND {
            return spa_index().await;
        }
        return resp
    }
    spa_index().await
}

pub async fn fallback(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.is_empty() {
        return spa_index().await;
    }
    if path.contains('.') {
        let resp = serve_inner(path).await;
        if resp.status() == StatusCode::NOT_FOUND {
            return spa_index().await;
        }
        return resp;
    }
    spa_index().await
}

async fn spa_index() -> Response {
    serve_inner("index.html").await
}

async fn serve_inner(path: &str) -> Response {
    match Asset::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let mut resp = Response::builder()
                .status(StatusCode::OK)
                .body(Body::from(content.data.into_owned()))
                .unwrap_or_else(|_| Response::new(Body::empty()));
            if let Ok(v) = HeaderValue::from_str(mime.as_ref()) {
                resp.headers_mut().insert(header::CONTENT_TYPE, v);
            }
            // Cache hashed assets aggressively; index.html is always fresh.
            let cache = if path == "index.html" {
                "no-cache"
            } else {
                "public, max-age=31536000, immutable"
            };
            if let Ok(v) = HeaderValue::from_str(cache) {
                resp.headers_mut().insert(header::CACHE_CONTROL, v);
            }
            resp
        }
        None => {
            // No build artifacts present yet — return a tiny placeholder when looking up index.
            if path == "index.html" {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    PLACEHOLDER_HTML,
                )
                    .into_response();
            }
            (StatusCode::NOT_FOUND, "not found").into_response()
        }
    }
}

const PLACEHOLDER_HTML: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>crashbox</title>
<style>body{background:#15141a;color:#e9e6df;font-family:ui-monospace,monospace;
display:flex;height:100vh;align-items:center;justify-content:center;margin:0}
h1{font-family:Georgia,serif;font-weight:500;font-size:36px;margin:0 0 4px}
p{color:#8c8a82;font-size:13px}</style></head>
<body><div><h1>crashbox</h1>
<p>// frontend not built. run <code>pnpm --dir frontend build</code></p></div>
</body></html>"#;
