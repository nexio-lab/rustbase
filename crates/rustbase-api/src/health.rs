use axum::{Json, extract::State, http::StatusCode};
use rustbase_db::admins::count_master_admins;
use serde::Serialize;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    /// Whether the master admin exists. When `false`, only the setup
    /// wizard at `/_/setup` should be accessible.
    pub initialized: bool,
}

/// `GET /healthz` — cheap liveness + initialization probe.
pub async fn healthz(
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<HealthResponse>), ApiError> {
    let n = count_master_admins(state.system.pool()).await?;
    Ok((
        StatusCode::OK,
        Json(HealthResponse {
            status: "ok",
            initialized: n > 0,
        }),
    ))
}
