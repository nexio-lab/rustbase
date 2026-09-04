//! TOTP second-factor: enrolment, confirmation, disable.
//!
//! Three user-authenticated endpoints under
//! `/api/workspaces/:workspace/auth/totp/`. With workspace-shared
//! identity, enabling TOTP gates every app in the workspace at once.
//!
//! `POST /enroll` starts (or restarts) enrolment. A fresh secret is
//! stored in `_user_totp` in pending state. The response carries
//! both `secret_b32` (the base32 secret, for users typing it by hand)
//! and `otpauth_url` (the `otpauth://` URI an authenticator app reads
//! directly from a QR code rendered by the client).
//!
//! `POST /confirm` body `{code}` verifies the code against the pending
//! secret. On success, flips `enabled=1`. After this point the user is
//! in 2FA mode: their next password login returns an `mfa_token`
//! instead of access tokens.
//!
//! `POST /disable` body `{code}` requires a valid current code, then
//! removes the `_user_totp` row outright. A user who lost their device
//! can be rescued by an admin via direct DB update; that admin path
//! lives elsewhere.
//!
//! The login integration lives in `login.rs::user_login` (returns an
//! `mfa_token` when the user has TOTP enabled) and in `login_totp`
//! below (the second-step endpoint that exchanges the challenge +
//! code for real access/refresh tokens).

use axum::{
    Json,
    extract::{Path, State},
};
use chrono::Duration;
use rand_core::{OsRng, RngCore};
use rustbase_auth::{TokenRole, build_claims};
use rustbase_core::{CoreError, WorkspaceId};
use rustbase_db::{
    mfa_challenges::{self, ConsumeOutcome as MfaConsume},
    tokens::commit_user_login,
    user_totp,
    users::{find_user_by_email, find_user_by_id},
};
use serde::{Deserialize, Serialize};
use totp_rs::{Algorithm, Secret, TOTP};

use crate::auth::PrincipalAuth;
use crate::auth::login::UserPublic;
use crate::auth::{
    default_access_ttl, default_refresh_ttl, new_refresh_token, require_workspace_exists,
};
use crate::error::ApiError;
use crate::state::AppState;

const ISSUER: &str = "RustBase";
const STEP_SECONDS: u64 = 30;
const DIGITS: usize = 6;
/// Accept the code from one step before and one step after the
/// current 30-second window to tolerate small clock skew.
const SKEW: u8 = 1;
const MFA_CHALLENGE_TTL_MINUTES: i64 = 5;

#[derive(Debug, Serialize)]
pub struct EnrollResponse {
    pub secret_b32: String,
    pub otpauth_url: String,
}

#[derive(Debug, Deserialize)]
pub struct CodeBody {
    pub code: String,
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub status: &'static str,
}

/// `POST /api/workspaces/:workspace/auth/totp/enroll`.
///
/// Authenticated end-user starts enrolment. A fresh secret replaces
/// any prior row (whether pending or enabled).
pub async fn enroll(
    auth: PrincipalAuth,
    State(state): State<AppState>,
    Path(workspace): Path<String>,
) -> Result<Json<EnrollResponse>, ApiError> {
    auth.require_user_in_workspace(&workspace)?;
    require_workspace_exists(&state, &workspace).await?;
    let workspace_id = WorkspaceId::from(workspace.clone());
    let pool = state.workspaces.pool_for(&workspace_id).await?;

    let user = find_user_by_id(&pool, &auth.subject_id)
        .await?
        .ok_or(ApiError::Core(CoreError::NotFound {
            collection: "user".into(),
            id: auth.subject_id.clone(),
        }))?;

    // 20 random bytes ⇒ 32 base32 chars after padding stripping;
    // RFC 4226 §4 R6 recommends 160 bits.
    let secret = Secret::generate_secret();
    let secret_b32 = secret.to_encoded().to_string();

    let totp = build_totp(&secret_b32, &workspace, &user.email)?;
    let otpauth_url = totp.get_url();

    user_totp::enroll(&pool, &user.id, &secret_b32).await?;
    tracing::info!(workspace = %workspace, user_id = %user.id, "TOTP enrolment started");

    Ok(Json(EnrollResponse {
        secret_b32,
        otpauth_url,
    }))
}

/// `POST /api/workspaces/:workspace/auth/totp/confirm`.
pub async fn confirm(
    auth: PrincipalAuth,
    State(state): State<AppState>,
    Path(workspace): Path<String>,
    Json(body): Json<CodeBody>,
) -> Result<Json<StatusResponse>, ApiError> {
    auth.require_user_in_workspace(&workspace)?;
    require_workspace_exists(&state, &workspace).await?;
    let workspace_id = WorkspaceId::from(workspace.clone());
    let pool = state.workspaces.pool_for(&workspace_id).await?;

    let row = user_totp::find(&pool, &auth.subject_id)
        .await?
        .ok_or(ApiError::Core(CoreError::Conflict(
            "no pending enrolment — call /enroll first".into(),
        )))?;
    let user = find_user_by_id(&pool, &auth.subject_id)
        .await?
        .ok_or(ApiError::Core(CoreError::Unauthorized))?;

    let totp = build_totp(&row.secret_b32, &workspace, &user.email)?;
    if !check_code(&totp, &body.code)? {
        return Err(ApiError::Core(CoreError::Unauthorized));
    }
    let n = user_totp::confirm_enabled(&pool, &auth.subject_id).await?;
    if n == 0 {
        return Ok(Json(StatusResponse { status: "enabled" }));
    }
    tracing::info!(workspace = %workspace, user_id = %auth.subject_id, "TOTP confirmed");
    Ok(Json(StatusResponse { status: "enabled" }))
}

/// `POST /api/workspaces/:workspace/auth/totp/disable`.
pub async fn disable(
    auth: PrincipalAuth,
    State(state): State<AppState>,
    Path(workspace): Path<String>,
    Json(body): Json<CodeBody>,
) -> Result<Json<StatusResponse>, ApiError> {
    auth.require_user_in_workspace(&workspace)?;
    require_workspace_exists(&state, &workspace).await?;
    let workspace_id = WorkspaceId::from(workspace.clone());
    let pool = state.workspaces.pool_for(&workspace_id).await?;

    let row = user_totp::find(&pool, &auth.subject_id)
        .await?
        .ok_or(ApiError::Core(CoreError::NotFound {
            collection: "user_totp".into(),
            id: auth.subject_id.clone(),
        }))?;
    let user = find_user_by_id(&pool, &auth.subject_id)
        .await?
        .ok_or(ApiError::Core(CoreError::Unauthorized))?;

    let totp = build_totp(&row.secret_b32, &workspace, &user.email)?;
    if !check_code(&totp, &body.code)? {
        return Err(ApiError::Core(CoreError::Unauthorized));
    }
    user_totp::disable(&pool, &auth.subject_id).await?;
    tracing::info!(workspace = %workspace, user_id = %auth.subject_id, "TOTP disabled");
    Ok(Json(StatusResponse { status: "disabled" }))
}

// ---- second-step login ----

#[derive(Debug, Deserialize)]
pub struct LoginTotpBody {
    pub mfa_token: String,
    pub code: String,
}

#[derive(Debug, Serialize)]
pub struct LoginTotpResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub user: UserPublic,
}

/// `POST /api/workspaces/:workspace/auth/users/login/totp`.
///
/// Second step of the 2FA login. Consumes the `mfa_token` issued by
/// `user_login`, verifies the TOTP code against the user's secret,
/// and returns full access/refresh tokens on success.
pub async fn login_totp(
    State(state): State<AppState>,
    Path(workspace): Path<String>,
    Json(body): Json<LoginTotpBody>,
) -> Result<Json<LoginTotpResponse>, ApiError> {
    require_workspace_exists(&state, &workspace).await?;
    let workspace_id = WorkspaceId::from(workspace.clone());
    let pool = state.workspaces.pool_for(&workspace_id).await?;

    let user_id = match mfa_challenges::consume(&pool, &body.mfa_token).await? {
        MfaConsume::Ok { user_id } => user_id,
        MfaConsume::Unknown | MfaConsume::AlreadyConsumed => {
            return Err(ApiError::Core(CoreError::Unauthorized));
        }
        MfaConsume::Expired => {
            return Err(ApiError::Core(CoreError::Conflict(
                "mfa challenge expired — start login over".into(),
            )));
        }
    };

    let row = user_totp::find(&pool, &user_id)
        .await?
        .ok_or(ApiError::Core(CoreError::Unauthorized))?;
    if !row.enabled {
        return Err(ApiError::Core(CoreError::Conflict(
            "TOTP not enabled — restart login".into(),
        )));
    }
    let user = find_user_by_id(&pool, &user_id)
        .await?
        .ok_or(ApiError::Core(CoreError::Unauthorized))?;

    // Subject mirrors the password step (workspace-scoped) so the
    // password + TOTP failure budgets are shared.
    let subject = format!("workspace:{workspace}:user:{}", user.email);
    state
        .login_attempts
        .check(&subject, &state.lockout_policy)?;

    let totp = build_totp(&row.secret_b32, &workspace, &user.email)?;
    if !check_code(&totp, &body.code)? {
        let err = match state
            .login_attempts
            .note_failure(&subject, &state.lockout_policy)
        {
            Ok(()) => {
                crate::auth::audit_events::record(
                    &state,
                    crate::auth::audit_events::AuthEvent {
                        outcome: crate::auth::audit_events::AuthOutcome::Failed,
                        scope: crate::auth::audit_events::Scope::Workspace(&workspace),
                        subject: &subject,
                        target: Some(&user.email),
                        details: serde_json::json!({"flow":"totp"}),
                    },
                )
                .await;
                ApiError::Core(CoreError::Unauthorized)
            }
            Err(CoreError::TooManyRequests { retry_after_secs }) => {
                crate::auth::audit_events::record(
                    &state,
                    crate::auth::audit_events::AuthEvent {
                        outcome: crate::auth::audit_events::AuthOutcome::Locked,
                        scope: crate::auth::audit_events::Scope::Workspace(&workspace),
                        subject: &subject,
                        target: Some(&user.email),
                        details: serde_json::json!({
                            "flow": "totp",
                            "retry_after_secs": retry_after_secs,
                        }),
                    },
                )
                .await;
                ApiError::Core(CoreError::TooManyRequests { retry_after_secs })
            }
            Err(other) => ApiError::Core(other),
        };
        return Err(err);
    }
    state.login_attempts.note_success(&subject);

    let claims = build_claims(
        user.id.clone(),
        TokenRole::User,
        Some(workspace.clone()),
        // Workspace-shared identity → user tokens carry no `app`.
        None,
        default_access_ttl(),
    );
    let access_token = state.jwt.issue(&claims)?;
    // last_login + refresh insert in one txn.
    let issued = new_refresh_token();
    commit_user_login(&pool, &user.id, &issued, default_refresh_ttl()).await?;
    tracing::info!(workspace = %workspace, user_id = %user.id, "user login (TOTP second step)");
    crate::auth::audit_events::record(
        &state,
        crate::auth::audit_events::AuthEvent {
            outcome: crate::auth::audit_events::AuthOutcome::Success,
            scope: crate::auth::audit_events::Scope::Workspace(&workspace),
            subject: &subject,
            target: Some(&user.email),
            details: serde_json::json!({
                "flow": "totp",
                "user_id": &user.id,
            }),
        },
    )
    .await;

    Ok(Json(LoginTotpResponse {
        access_token,
        refresh_token: issued,
        user: UserPublic {
            id: user.id,
            email: user.email,
            verified: user.verified,
        },
    }))
}

// ---- helpers ----

/// Issue an MFA challenge for `user_id` and return the opaque token
/// the client should pass to `/login/totp`. Lives here so the login
/// endpoint can call it without re-importing the storage module.
pub async fn issue_mfa_challenge(
    pool: &sqlx::SqlitePool,
    user_id: &str,
) -> Result<String, ApiError> {
    let token = fresh_mfa_token();
    mfa_challenges::issue(
        pool,
        &token,
        user_id,
        Duration::minutes(MFA_CHALLENGE_TTL_MINUTES),
    )
    .await?;
    Ok(token)
}

/// True when `user_id` has TOTP enrolled and confirmed. Cheap query
/// the login path calls on every password login.
pub async fn is_user_totp_enabled(
    pool: &sqlx::SqlitePool,
    user_id: &str,
) -> Result<bool, ApiError> {
    Ok(user_totp::find(pool, user_id)
        .await?
        .map(|r| r.enabled)
        .unwrap_or(false))
}

/// Find a user by email and return their TOTP status. Convenience for
/// the login endpoint which already does the email lookup.
pub async fn user_id_for_email_with_totp(
    pool: &sqlx::SqlitePool,
    email: &str,
) -> Result<Option<(String, bool)>, ApiError> {
    let Some(u) = find_user_by_email(pool, email).await? else {
        return Ok(None);
    };
    let enabled = is_user_totp_enabled(pool, &u.id).await?;
    Ok(Some((u.id, enabled)))
}

fn build_totp(secret_b32: &str, workspace: &str, account: &str) -> Result<TOTP, ApiError> {
    let bytes = Secret::Encoded(secret_b32.to_string())
        .to_bytes()
        .map_err(|e| ApiError::Core(CoreError::Internal(format!("decode totp secret: {e:?}"))))?;
    TOTP::new(
        Algorithm::SHA1,
        DIGITS,
        SKEW,
        STEP_SECONDS,
        bytes,
        Some(format!("{ISSUER} ({workspace})")),
        account.to_string(),
    )
    .map_err(|e| ApiError::Core(CoreError::Internal(format!("build totp: {e:?}"))))
}

fn check_code(totp: &TOTP, code: &str) -> Result<bool, ApiError> {
    totp.check_current(code)
        .map_err(|e| ApiError::Core(CoreError::Internal(format!("check totp: {e:?}"))))
}

fn fresh_mfa_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
