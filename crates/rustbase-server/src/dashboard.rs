//! Embedded dashboard server.
//!
//! - In release builds, `include_dir!` walks `../../ui/build/` at
//!   compile time and bundles every file into the binary. The SvelteKit
//!   SPA's static artifact ships inside the single `rustbase`
//!   executable — `bun --cwd ui run build` produces `ui/build/`, then
//!   `cargo build --release` snapshots it.
//! - Set `RUSTBASE_DASHBOARD_PATH=ui/build` (or any built dashboard
//!   directory) to override the embed at runtime — useful when iterating
//!   on the dashboard without recompiling Rust. For interactive
//!   iteration, run `bun --cwd ui run dev` on :5173 instead — its
//!   vite proxy forwards `/api`, `/_/setup`, `/_/auth/*`, `/healthz`
//!   straight to the Rust server on :8080.
//! - Mounted at `GET /_/...` so `POST /_/setup` etc. still reach the
//!   API. Unknown paths fall back to `index.html` so the SPA can
//!   handle client-side routing.

use axum::{
    extract::Path,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use include_dir::{Dir, include_dir};
use std::path::{Path as StdPath, PathBuf};

static EMBEDDED: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../ui/build");

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
            if p.is_file()
                && let Ok(bytes) = std::fs::read(&p)
            {
                let mime = mime_for(StdPath::new(c));
                return Some((bytes, mime));
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
