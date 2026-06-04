//! CORS layer over `tower-http::cors::CorsLayer`.
//!
//! Default is "no cross-origin allowed" — the dashboard is same-origin,
//! and you have to explicitly opt-in any third-party origin in
//! `rustbase.toml` under `[cors]`. Empty allowlist returns an explicitly
//! restrictive layer rather than `CorsLayer::very_permissive()` so a
//! missing config never leaks credentials.

use axum::http::{HeaderName, HeaderValue, Method};
use std::time::Duration;
use tower_http::cors::{AllowOrigin, CorsLayer};

#[derive(Debug, Clone)]
pub struct CorsConfig {
    pub allow_origins: Vec<String>,
    pub allow_credentials: bool,
    pub max_age: Duration,
}

/// Methods we expose on the API surface. Headers we accept are the
/// minimal set the JS SDK + dashboard need: `Authorization`,
/// `Content-Type`, plus the `X-Request-Id` we propagate.
fn allowed_methods() -> [Method; 6] {
    [
        Method::GET,
        Method::POST,
        Method::PATCH,
        Method::PUT,
        Method::DELETE,
        Method::OPTIONS,
    ]
}

fn allowed_headers() -> Vec<HeaderName> {
    vec![
        axum::http::header::AUTHORIZATION,
        axum::http::header::CONTENT_TYPE,
        HeaderName::from_static("x-request-id"),
    ]
}

pub fn layer(cfg: &CorsConfig) -> CorsLayer {
    let mut layer = CorsLayer::new()
        .allow_methods(allowed_methods())
        .allow_headers(allowed_headers())
        .max_age(cfg.max_age);

    if cfg.allow_origins.is_empty() {
        // No `allow_origin` configured → the layer denies every preflight
        // and strips CORS headers on simple requests, which is what we
        // want for a same-origin-only deployment.
        return layer;
    }

    let origins: Vec<HeaderValue> = cfg
        .allow_origins
        .iter()
        .filter_map(|s| HeaderValue::from_str(s).ok())
        .collect();
    layer = layer.allow_origin(AllowOrigin::list(origins));

    if cfg.allow_credentials {
        layer = layer.allow_credentials(true);
    }
    layer
}
