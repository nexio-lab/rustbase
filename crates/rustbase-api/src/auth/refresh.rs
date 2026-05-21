use axum::{Json, extract::State};
use rustbase_auth::{TokenRole, build_claims, encode_token};
use rustbase_core::CoreError;
use rustbase_db::tokens::{
    SubjectKind, find_active_refresh_token, insert_refresh_token, revoke_refresh_token,
};
use serde::{Deserialize, Serialize};

use super::{default_access_ttl, default_refresh_ttl, new_refresh_token};
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Serialize)]
pub struct RefreshResponse {
    pub access_token: String,
    pub refresh_token: String,
}

/// Rotate-on-use: each refresh revokes the presented token and issues a
/// fresh one. Reusing a refresh that's already been redeemed fails.
pub async fn master_admin_refresh(
    State(state): State<AppState>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<RefreshResponse>, ApiError> {
    let existing =
        find_active_refresh_token(state.system.pool(), &req.refresh_token, SubjectKind::MasterAdmin)
            .await?
            .ok_or(ApiError::Core(CoreError::Unauthorized))?;

    revoke_refresh_token(state.system.pool(), &existing.token).await?;

    let new_refresh = insert_refresh_token(
        state.system.pool(),
        &new_refresh_token(),
        SubjectKind::MasterAdmin,
        &existing.subject_id,
        default_refresh_ttl(),
    )
    .await?;

    let claims = build_claims(
        existing.subject_id,
        TokenRole::MasterAdmin,
        None,
        None,
        default_access_ttl(),
    );
    let access_token = encode_token(&claims, &state.master_key)?;

    Ok(Json(RefreshResponse {
        access_token,
        refresh_token: new_refresh.token,
    }))
}
