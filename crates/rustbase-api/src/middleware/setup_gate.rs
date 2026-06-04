//! `setup_gate` blocks every route except `/healthz` and `/_/setup` while
//! the server has no master admin. The dashboard reads `/healthz` to
//! decide when to redirect users to the setup wizard.

use axum::{
    Json,
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::json;

use crate::state::AppState;

/// Routes that remain available before setup completes.
fn is_allowed_before_setup(method: &axum::http::Method, path: &str) -> bool {
    if path == "/healthz" || path == "/_/setup" {
        return true;
    }
    // JWKS is anonymous public material. Exposing it pre-setup costs
    // nothing and lets external systems (CI smoke tests, OIDC
    // discovery probes) reach it from the very first boot.
    if method == axum::http::Method::GET
        && (path == "/.well-known/jwks.json" || path == "/_/auth/jwks.json")
    {
        return true;
    }
    // Dashboard reads are always safe — the setup wizard itself lives
    // inside the dashboard.
    method == axum::http::Method::GET && (path == "/_/" || path.starts_with("/_/"))
}

pub async fn setup_gate(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if state.is_initialized() || is_allowed_before_setup(req.method(), req.uri().path()) {
        return next.run(req).await;
    }
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "code": "uninitialized",
            "message": "complete setup at POST /_/setup",
        })),
    )
        .into_response()
}
