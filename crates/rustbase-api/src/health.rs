use axum::{Json, extract::State, http::StatusCode};
use rustbase_db::admins::master_admin_is_initialized;
use serde::Serialize;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    /// Whether the setup wizard has completed (the auto-seeded master
    /// admin has a real password hash). When `false`, only the wizard
    /// at `/_/setup` should be accessible.
    pub initialized: bool,
}

/// `GET /healthz` — cheap liveness + initialization probe.
pub async fn healthz(
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<HealthResponse>), ApiError> {
    let initialized = master_admin_is_initialized(state.system.pool()).await?;
    Ok((
        StatusCode::OK,
        Json(HealthResponse {
            status: "ok",
            initialized,
        }),
    ))
}
