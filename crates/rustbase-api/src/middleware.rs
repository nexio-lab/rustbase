//! Cross-cutting middleware.
//!
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
fn is_allowed_before_setup(path: &str) -> bool {
    matches!(path, "/healthz" | "/_/setup")
}

pub async fn setup_gate(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    if state.is_initialized() || is_allowed_before_setup(req.uri().path()) {
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
