//! `POST /_/setup` — one-shot creation of the first master admin.
//!
//! While `AppState::is_initialized()` is `false`, the [`crate::middleware`]
//! gate blocks every route except `/healthz` and `/_/setup`. The first
//! successful call here creates the master admin, flips the flag, and
//! every other endpoint becomes reachable. Subsequent calls return 409.

use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use rustbase_auth::hash_password;
use rustbase_core::CoreError;
use rustbase_db::admins::{count_master_admins, insert_master_admin};
use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize, Validate)]
pub struct SetupRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 8, max = 256))]
    pub password: String,
    #[validate(length(max = 100))]
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SetupResponse {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub async fn setup(
    State(state): State<AppState>,
    Json(req): Json<SetupRequest>,
) -> Result<(StatusCode, Json<SetupResponse>), ApiError> {
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;

    // Idempotency guard — once a master admin exists, refuse politely.
    // We re-check the DB rather than just the atomic so a racing setup
    // call against a freshly-restored DB still loses cleanly.
    if count_master_admins(state.system.pool()).await? > 0 {
        return Err(ApiError::Core(CoreError::Conflict(
            "server already initialized".into(),
        )));
    }

    let hash = hash_password(&req.password)?;
    let admin =
        insert_master_admin(state.system.pool(), &req.email, &hash, req.name.as_deref()).await?;

    state.mark_initialized();
    tracing::info!(admin_id = %admin.id, email = %admin.email, "master admin created via setup wizard");

    Ok((
        StatusCode::CREATED,
        Json(SetupResponse {
            id: admin.id,
            email: admin.email,
            name: admin.name,
            created_at: admin.created_at,
        }),
    ))
}
