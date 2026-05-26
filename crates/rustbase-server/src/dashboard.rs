//! Embedded dashboard server.
//!
//! - In release builds, `include_dir!` walks `./dashboard/` at compile
//!   time and bundles every file into the binary. In dev mode, setting
//!   the `RUSTBASE_DASHBOARD_PATH` env var redirects reads to a
//!   directory on disk so a SvelteKit dev server can iterate without
//!   rebuilding the Rust binary.
//! - Mounted at `GET /_/...` so `POST /_/setup` etc. still reach the
//!   API. Unknown paths fall back to `index.html` (SPA convention).

use axum::{
    extract::Path,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use include_dir::{Dir, include_dir};
use std::path::{Path as StdPath, PathBuf};

static EMBEDDED: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/dashboard");

fn dev_root() -> Option<PathBuf> {
    std::env::var("RUSTBASE_DASHBOARD_PATH")
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/// Resolve a request path under the dashboard mount and return the file
/// bytes + best-guess MIME. Missing files fall back to `index.html` so
/// SPA client-side routing works.
fn resolve(path: &str) -> Option<(Vec<u8>, String)> {
    let clean = path.trim_start_matches('/');
    let candidates: [&str; 2] = [
        if clean.is_empty() {
            "index.html"
        } else {
            clean
        },
        "index.html",
    ];

    // dev override
    if let Some(root) = dev_root() {
        for c in candidates {
            let p = root.join(c);
            if p.is_file() {
                if let Ok(bytes) = std::fs::read(&p) {
                    let mime = mime_for(StdPath::new(c));
                    return Some((bytes, mime));
                }
            }
        }
    }

    // embedded
    for c in candidates {
        if let Some(file) = EMBEDDED.get_file(c) {
            let mime = mime_for(StdPath::new(c));
            return Some((file.contents().to_vec(), mime));
        }
    }
    None
}

fn mime_for(p: &StdPath) -> String {
    mime_guess::from_path(p)
        .first_or_octet_stream()
        .essence_str()
        .to_string()
}

/// `GET /_/` — serve index.html.
pub async fn index() -> Response {
    serve("index.html")
}

/// `GET /_/{*path}` — serve a static asset under the dashboard mount.
pub async fn asset(Path(path): Path<String>) -> Response {
    serve(&path)
}

fn serve(path: &str) -> Response {
    let Some((bytes, mime)) = resolve(path) else {
        return (StatusCode::NOT_FOUND, "dashboard asset not found").into_response();
    };
    let mut resp = bytes.into_response();
    if let Ok(v) = HeaderValue::from_str(&mime) {
        resp.headers_mut().insert(header::CONTENT_TYPE, v);
    }
    resp
}
