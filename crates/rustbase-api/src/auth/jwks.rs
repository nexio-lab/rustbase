//! JWKS handler — publishes the public half of the master RS256
//! keypair so external systems can verify RustBase-issued JWTs
//! without sharing the private signing key.
//!
//! Mounted at both:
//!   - `/_/auth/jwks.json` — discoverable from the dashboard scope.
//!   - `/.well-known/jwks.json` — the OIDC / OAuth convention so
//!     standard libraries can consume it without any custom config.
//!
//! The endpoint is anonymous: the body is public material by
//! definition. Caching headers tell well-behaved clients to refresh
//! at most once per hour — short enough that a key rotation propagates
//! quickly, long enough to keep the per-request cost off the hot path.

use axum::{Json, extract::State, http::header, response::IntoResponse};

use crate::state::AppState;

pub async fn jwks(State(state): State<AppState>) -> impl IntoResponse {
    let body = state.jwt.jwks();
    (
        [
            (header::CONTENT_TYPE, "application/jwk-set+json"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        Json(body),
    )
}
