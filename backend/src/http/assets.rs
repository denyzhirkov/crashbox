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

pub fn serve_root() -> Response {
    serve_inner("index.html")
}

pub fn serve_path(Path(path): Path<String>) -> Response {
    // Anything with an extension is asset-like; serve the file or 404.
    // Anything else — SPA route — falls back to index.html.
    if path.contains('.') {
        let resp = serve_inner(&path);
        if resp.status() == StatusCode::NOT_FOUND {
            return spa_index();
        }
        return resp;
    }
    spa_index()
}

// axum's `Handler` trait is only implemented for async fns, so this stays `async` even though the
// asset lookup underneath is synchronous (rust-embed).
#[allow(clippy::unused_async)]
pub async fn fallback(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.is_empty() {
        return spa_index();
    }
    if path.contains('.') {
        let resp = serve_inner(path);
        if resp.status() == StatusCode::NOT_FOUND {
            return spa_index();
        }
        return resp;
    }
    spa_index()
}

fn spa_index() -> Response {
    serve_inner("index.html")
}

fn serve_inner(path: &str) -> Response {
    let Some(content) = Asset::get(path) else {
        // No build artifacts present yet — return a tiny placeholder when looking up index.
        if path == "index.html" {
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                PLACEHOLDER_HTML,
            )
                .into_response();
        }
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
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

const PLACEHOLDER_HTML: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>crashbox</title>
<style>body{background:#15141a;color:#e9e6df;font-family:ui-monospace,monospace;
display:flex;height:100vh;align-items:center;justify-content:center;margin:0}
h1{font-family:Georgia,serif;font-weight:500;font-size:36px;margin:0 0 4px}
p{color:#8c8a82;font-size:13px}</style></head>
<body><div><h1>crashbox</h1>
<p>// frontend not built. run <code>pnpm --dir frontend build</code></p></div>
</body></html>"#;
