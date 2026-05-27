//! `POST /_/setup` — set the password on the auto-seeded `admin` master admin.
//!
//! On first boot, `rustbase-server` calls `ensure_seed_master_admin` to
//! insert a row with `username = "admin"` and `password_hash = NULL`.
//! While that hash is still NULL, the [`crate::middleware`] setup gate
//! blocks every route except `/healthz` and `/_/setup`. This handler
//! consumes a single password, hashes it onto the seeded row, and flips
//! the server into the "initialized" state. Subsequent calls return 409.

use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use rustbase_auth::hash_password;
use rustbase_core::CoreError;
use rustbase_db::admins::{
    find_master_admin_by_username, master_admin_is_initialized, set_master_admin_password,
};
use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize, Validate)]
pub struct SetupRequest {
    #[validate(length(min = 8, max = 256))]
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct SetupResponse {
    pub id: String,
    pub username: String,
    pub created_at: DateTime<Utc>,
}

pub async fn setup(
    State(state): State<AppState>,
    Json(req): Json<SetupRequest>,
) -> Result<(StatusCode, Json<SetupResponse>), ApiError> {
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;

    // Idempotency guard — once the seed admin has a password, refuse.
    if master_admin_is_initialized(state.system.pool()).await? {
        return Err(ApiError::Core(CoreError::Conflict(
            "server already initialized".into(),
        )));
    }

    // The seed row was inserted on boot. Look it up by username so we
    // can target it precisely; a stale DB without the seed gets a
    // graceful 500 rather than a silent UPDATE-touched-nothing.
    let admin = find_master_admin_by_username(state.system.pool(), "admin")
        .await?
        .ok_or_else(|| {
            ApiError::Core(CoreError::Internal(
                "seed admin row missing; restart the server".into(),
            ))
        })?;

    let hash = hash_password(&req.password)?;
    set_master_admin_password(state.system.pool(), &admin.id, &hash).await?;

    state.mark_initialized();
    tracing::info!(admin_id = %admin.id, "master admin password set via setup wizard");

    Ok((
        StatusCode::CREATED,
        Json(SetupResponse {
            id: admin.id,
            username: admin.username,
            created_at: admin.created_at,
        }),
    ))
}
