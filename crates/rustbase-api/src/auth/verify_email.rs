//! Email-verification endpoints.
//!
//! Two routes, both under an app scope:
//!
//! - `POST /api/workspaces/:workspace/apps/:app/auth/verify-email/request`
//!   Authenticated end-user asks to receive a verification email.
//!   A fresh token is issued in the app DB, mailed to the user's
//!   stored address, and the response indicates whether a mail was
//!   dispatched. Already-verified users receive `200 OK` with an
//!   `already verified` message instead — we don't leak whether a
//!   token row was created.
//!
//! - `POST /api/workspaces/:workspace/apps/:app/auth/verify-email/confirm`
//!   Anyone can call. Body carries `{ "token": "<opaque>" }`. The
//!   token is consumed atomically; on success the matching user is
//!   marked verified.
//!
//! Tokens are 32 random bytes encoded as 64 hex chars, with a 24-hour
//! TTL. The `consume` machinery in `rustbase_db::email_verifications`
//! enforces single-use semantics.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rand_core::{OsRng, RngCore};
use rustbase_core::{AppId, CoreError, EmailMessage, WorkspaceId};
use rustbase_db::{
    email_verifications::{self, ConsumeOutcome},
    users::{find_user_by_id, mark_verified},
};
use serde::{Deserialize, Serialize};

use crate::auth::{PrincipalAuth, require_app_exists};
use crate::error::ApiError;
use crate::state::AppState;

const VERIFICATION_TTL_HOURS: i64 = 24;
const SYSTEM_FROM_ADDRESS: &str = "no-reply@rustbase.local";

#[derive(Debug, Serialize)]
pub struct VerifyRequestResponse {
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct ConfirmRequest {
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct ConfirmResponse {
    pub verified: bool,
    pub user_id: String,
}

/// Issue + mail a fresh verification token to the calling user's
/// stored email address.
pub async fn request(
    auth: PrincipalAuth,
    State(state): State<AppState>,
    Path((workspace, app)): Path<(String, String)>,
) -> Result<(StatusCode, Json<VerifyRequestResponse>), ApiError> {
    // Only end users in this app may ask for verification of their own
    // email; admins and tokens from another app/workspace are rejected.
    auth.require_user_in_app(&workspace, &app)?;

    require_app_exists(&state, &workspace, &app).await?;
    let workspace_id = WorkspaceId::from(workspace.clone());
    let app_id = AppId::from(app.clone());
    let pool = state.apps.pool_for(&workspace_id, &app_id).await?;

    let user = find_user_by_id(&pool, &auth.subject_id)
        .await?
        .ok_or(ApiError::Core(CoreError::NotFound {
            collection: "user".into(),
            id: auth.subject_id.clone(),
        }))?;

    if user.verified {
        return Ok((
            StatusCode::OK,
            Json(VerifyRequestResponse {
                message: "already verified".into(),
            }),
        ));
    }

    let token = fresh_token();
    email_verifications::issue(
        &pool,
        &token,
        &user.id,
        chrono::Duration::hours(VERIFICATION_TTL_HOURS),
    )
    .await?;

    let body = format!(
        "Hello,\n\nUse the following token to verify your email address:\n\n\
         {token}\n\nThis token is valid for {VERIFICATION_TTL_HOURS} hours.\n",
    );
    let msg = EmailMessage::new(
        SYSTEM_FROM_ADDRESS,
        &user.email,
        format!("Verify your email for {workspace}/{app}"),
        body,
    );
    state
        .mailer
        .send(msg)
        .await
        .map_err(|e| ApiError::Core(CoreError::Internal(format!("mailer: {e}"))))?;

    tracing::info!(
        workspace = %workspace,
        app = %app,
        user_id = %user.id,
        "verification token issued + mailed"
    );

    Ok((
        StatusCode::ACCEPTED,
        Json(VerifyRequestResponse {
            message: "verification email sent".into(),
        }),
    ))
}

/// Confirm a verification token. Anyone may call: the token itself is
/// the proof of email ownership. Idempotent only in the sense that
/// re-using a consumed token returns 410 Gone.
pub async fn confirm(
    State(state): State<AppState>,
    Path((workspace, app)): Path<(String, String)>,
    Json(req): Json<ConfirmRequest>,
) -> Result<Json<ConfirmResponse>, ApiError> {
    require_app_exists(&state, &workspace, &app).await?;
    let workspace_id = WorkspaceId::from(workspace.clone());
    let app_id = AppId::from(app.clone());
    let pool = state.apps.pool_for(&workspace_id, &app_id).await?;

    match email_verifications::consume(&pool, &req.token).await? {
        ConsumeOutcome::Ok { user_id } => {
            mark_verified(&pool, &user_id).await?;
            tracing::info!(workspace = %workspace, app = %app, user_id = %user_id, "email verified");
            Ok(Json(ConfirmResponse {
                verified: true,
                user_id,
            }))
        }
        ConsumeOutcome::Unknown => Err(ApiError::Core(CoreError::NotFound {
            collection: "email_verification".into(),
            id: req.token,
        })),
        ConsumeOutcome::AlreadyConsumed => Err(ApiError::Core(CoreError::Conflict(
            "verification token already used".into(),
        ))),
        ConsumeOutcome::Expired => Err(ApiError::Core(CoreError::Conflict(
            "verification token expired".into(),
        ))),
    }
}

/// 32 random bytes from the OS RNG, hex-encoded (64 chars). No leading
/// "vrf_" prefix — the column is opaque to clients anyway and the
/// shorter shape is friendlier when a user hand-types or copies.
fn fresh_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
