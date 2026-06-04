//! Password-reset endpoints.
//!
//! - `POST /api/workspaces/:workspace/apps/:app/auth/password-reset/request`
//!   Anonymous. Body: `{ "email": "..." }`. Always answers
//!   `202 Accepted` with the same generic message so the response
//!   can't be used to enumerate which addresses are registered. A
//!   reset token is issued and mailed *only* when the email
//!   actually resolves to a user in this app; the no-match case
//!   is silent.
//!
//! - `POST /api/workspaces/:workspace/apps/:app/auth/password-reset/confirm`
//!   Anonymous. Body: `{ "token": "...", "new_password": "..." }`.
//!   Consumes the token atomically, rehashes the new password, and
//!   replaces the stored hash. On success, every other pending
//!   reset token for the same user is invalidated so a parallel
//!   in-flight request becomes a dead letter.
//!
//! Tokens are 32 random bytes hex-encoded, with a 1-hour TTL — the
//! reset window is deliberately tighter than email verification.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rand_core::{OsRng, RngCore};
use rustbase_auth::hash_password;
use rustbase_core::{AppId, CoreError, EmailMessage, WorkspaceId};
use rustbase_db::{
    password_resets::{self, ConsumeOutcome},
    users::{find_user_by_email, set_password_hash},
};
use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::auth::require_app_exists;
use crate::error::ApiError;
use crate::state::AppState;

const RESET_TTL_MINUTES: i64 = 60;
const SYSTEM_FROM_ADDRESS: &str = "no-reply@rustbase.local";

#[derive(Debug, Deserialize, Validate)]
pub struct ResetRequest {
    #[validate(email)]
    pub email: String,
}

#[derive(Debug, Serialize)]
pub struct ResetRequestResponse {
    pub message: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct ResetConfirm {
    pub token: String,
    #[validate(length(min = 8, max = 256))]
    pub new_password: String,
}

#[derive(Debug, Serialize)]
pub struct ResetConfirmResponse {
    pub user_id: String,
    pub reset: bool,
}

/// Always returns 202. If the email matches a user, a token is issued
/// and mailed in the background of this same request; if not, nothing
/// happens but the caller can't tell the difference.
pub async fn request(
    State(state): State<AppState>,
    Path((workspace, app)): Path<(String, String)>,
    Json(req): Json<ResetRequest>,
) -> Result<(StatusCode, Json<ResetRequestResponse>), ApiError> {
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;
    require_app_exists(&state, &workspace, &app).await?;

    let workspace_id = WorkspaceId::from(workspace.clone());
    let app_id = AppId::from(app.clone());
    let pool = state.apps.pool_for(&workspace_id, &app_id).await?;

    let generic_response = (
        StatusCode::ACCEPTED,
        Json(ResetRequestResponse {
            message: "if that email is registered, a reset link has been sent".into(),
        }),
    );

    let Some(user) = find_user_by_email(&pool, &req.email).await? else {
        // Don't leak the absence of the email — same response shape.
        tracing::info!(
            workspace = %workspace,
            app = %app,
            email = %req.email,
            "password reset requested for unknown email"
        );
        return Ok(generic_response);
    };

    let token = fresh_token();
    password_resets::issue(
        &pool,
        &token,
        &user.id,
        chrono::Duration::minutes(RESET_TTL_MINUTES),
    )
    .await?;

    let body = format!(
        "Hello,\n\nUse the following token to reset your password:\n\n\
         {token}\n\nThis token is valid for {RESET_TTL_MINUTES} minutes. \
         If you didn't request this, you can safely ignore this email.\n",
    );
    let msg = EmailMessage::new(
        SYSTEM_FROM_ADDRESS,
        &user.email,
        format!("Reset your password for {workspace}/{app}"),
        body,
    );
    if let Err(e) = state.mailer.send(msg).await {
        tracing::error!(
            error = %e, workspace = %workspace, app = %app, user_id = %user.id,
            "mailer dropped reset email"
        );
    } else {
        tracing::info!(
            workspace = %workspace, app = %app, user_id = %user.id,
            "password reset token issued + mailed"
        );
    }

    Ok(generic_response)
}

pub async fn confirm(
    State(state): State<AppState>,
    Path((workspace, app)): Path<(String, String)>,
    Json(req): Json<ResetConfirm>,
) -> Result<Json<ResetConfirmResponse>, ApiError> {
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;
    require_app_exists(&state, &workspace, &app).await?;
    let workspace_id = WorkspaceId::from(workspace.clone());
    let app_id = AppId::from(app.clone());
    let pool = state.apps.pool_for(&workspace_id, &app_id).await?;

    let user_id = match password_resets::consume(&pool, &req.token).await? {
        ConsumeOutcome::Ok { user_id } => user_id,
        ConsumeOutcome::Unknown => {
            return Err(ApiError::Core(CoreError::NotFound {
                collection: "password_reset".into(),
                id: req.token,
            }));
        }
        ConsumeOutcome::AlreadyConsumed => {
            return Err(ApiError::Core(CoreError::Conflict(
                "reset token already used".into(),
            )));
        }
        ConsumeOutcome::Expired => {
            return Err(ApiError::Core(CoreError::Conflict(
                "reset token expired".into(),
            )));
        }
    };

    let hash = hash_password(&req.new_password)?;
    set_password_hash(&pool, &user_id, &hash).await?;

    let invalidated = password_resets::invalidate_all_for_user(&pool, &user_id).await?;
    tracing::info!(
        workspace = %workspace,
        app = %app,
        user_id = %user_id,
        invalidated_siblings = invalidated,
        "password reset confirmed"
    );

    Ok(Json(ResetConfirmResponse {
        user_id,
        reset: true,
    }))
}

fn fresh_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
