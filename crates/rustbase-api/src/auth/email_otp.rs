//! Passwordless email-OTP login.
//!
//! `POST /api/workspaces/:workspace/auth/otp/request` body `{ email }`
//! is anonymous and always returns 202 with the same generic message
//! regardless of whether the email maps to an existing user — same
//! enumeration-resistance posture as `password-reset/request`. On a
//! syntactically valid email, a fresh 6-digit code is issued
//! (invalidating any prior pending one) and mailed.
//!
//! `POST /api/workspaces/:workspace/auth/otp/login` body `{ email, code }`
//! is also anonymous and atomically consumes the code:
//!
//! * Right code → find-or-create user, mark verified=true (the OTP
//!   delivery proved control of the address), issue access + refresh
//!   tokens.
//! * Wrong code → 400 with `attempts_left` in the message so a client
//!   can show "3 tries remaining".
//! * Expired / locked / unknown email → 409.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rand_core::{OsRng, RngCore};
use rustbase_auth::{TokenRole, build_claims};
use rustbase_core::{CoreError, EmailMessage, WorkspaceId};
use rustbase_db::{
    email_otps::{self, ConsumeOutcome},
    tokens::commit_user_login,
    users::{User, find_user_by_email, insert_passwordless_user, mark_verified},
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use validator::Validate;

use crate::auth::login::UserPublic;
use crate::auth::{
    default_access_ttl, default_refresh_ttl, new_refresh_token, require_workspace_exists,
};
use crate::error::ApiError;
use crate::state::AppState;

const OTP_TTL_MINUTES: i64 = 10;
const SYSTEM_FROM_ADDRESS: &str = "no-reply@rustbase.local";

#[derive(Debug, Deserialize, Validate)]
pub struct OtpRequest {
    #[validate(email)]
    pub email: String,
}

#[derive(Debug, Serialize)]
pub struct OtpRequestResponse {
    pub message: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct OtpLoginRequest {
    #[validate(email)]
    pub email: String,
    /// The 6-digit code from the email. Stored + matched as a string
    /// so leading zeros are preserved (the spec output is `000123`,
    /// not `123`).
    #[validate(length(min = 4, max = 12))]
    pub code: String,
}

#[derive(Debug, Serialize)]
pub struct OtpLoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub user: UserPublic,
}

/// Anonymous code-request endpoint. Always 202 — no enumeration signal.
pub async fn request(
    State(state): State<AppState>,
    Path(workspace): Path<String>,
    Json(req): Json<OtpRequest>,
) -> Result<(StatusCode, Json<OtpRequestResponse>), ApiError> {
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;
    require_workspace_exists(&state, &workspace).await?;

    let workspace_id = WorkspaceId::from(workspace.clone());
    let pool = state.workspaces.pool_for(&workspace_id).await?;

    let code = fresh_otp_code();
    email_otps::issue(
        &pool,
        &code,
        &req.email,
        chrono::Duration::minutes(OTP_TTL_MINUTES),
    )
    .await?;

    let body = format!(
        "Hello,\n\nYour one-time login code is:\n\n  {code}\n\n\
         This code is valid for {OTP_TTL_MINUTES} minutes. If you didn't \
         request it, you can safely ignore this email.\n",
    );
    let msg = EmailMessage::new(
        SYSTEM_FROM_ADDRESS,
        &req.email,
        format!("Your login code for {workspace}"),
        body,
    );
    let send_result = state.mailer.send(msg).await;
    metrics::counter!(
        "rustbase_mailer_dispatches_total",
        "kind"    => "otp_login",
        "outcome" => if send_result.is_ok() { "success" } else { "failed" },
    )
    .increment(1);
    if let Err(e) = send_result {
        tracing::error!(
            error = %e, workspace = %workspace, email = %req.email,
            "mailer dropped OTP"
        );
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(OtpRequestResponse {
            message: "if that email is valid, a login code has been sent".into(),
        }),
    ))
}

/// Code-redemption endpoint. Issues login tokens on success.
pub async fn login(
    State(state): State<AppState>,
    Path(workspace): Path<String>,
    Json(req): Json<OtpLoginRequest>,
) -> Result<Json<OtpLoginResponse>, ApiError> {
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;
    require_workspace_exists(&state, &workspace).await?;
    let workspace_id = WorkspaceId::from(workspace.clone());
    let pool = state.workspaces.pool_for(&workspace_id).await?;

    // Share the lockout subject with password and TOTP, so a bad-OTP
    // burst against `alice@x` counts toward the same budget as
    // password attempts. Subject is now workspace-scoped to match the
    // shared identity pool.
    let subject = format!("workspace:{workspace}:user:{}", req.email);
    state
        .login_attempts
        .check(&subject, &state.lockout_policy)?;

    let outcome = email_otps::consume(&pool, &req.email, &req.code).await?;
    match outcome {
        ConsumeOutcome::Ok { email } => {
            state.login_attempts.note_success(&subject);
            let res = issue_tokens_for(&state, &pool, &workspace, &email).await;
            if res.is_ok() {
                crate::auth::audit_events::record(
                    &state,
                    crate::auth::audit_events::AuthEvent {
                        outcome: crate::auth::audit_events::AuthOutcome::Success,
                        scope: crate::auth::audit_events::Scope::Workspace(&workspace),
                        subject: &subject,
                        target: Some(&email),
                        details: serde_json::json!({ "flow": "email_otp" }),
                    },
                )
                .await;
            }
            res
        }
        ConsumeOutcome::WrongCode { attempts_left } => {
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
                            target: Some(&req.email),
                            details: serde_json::json!({
                                "flow": "email_otp",
                                "attempts_left": attempts_left,
                            }),
                        },
                    )
                    .await;
                    ApiError::Core(CoreError::Validation(format!(
                        "wrong code ({attempts_left} attempts left)"
                    )))
                }
                Err(CoreError::TooManyRequests { retry_after_secs }) => {
                    crate::auth::audit_events::record(
                        &state,
                        crate::auth::audit_events::AuthEvent {
                            outcome: crate::auth::audit_events::AuthOutcome::Locked,
                            scope: crate::auth::audit_events::Scope::Workspace(&workspace),
                            subject: &subject,
                            target: Some(&req.email),
                            details: serde_json::json!({
                                "flow": "email_otp",
                                "retry_after_secs": retry_after_secs,
                            }),
                        },
                    )
                    .await;
                    ApiError::Core(CoreError::TooManyRequests { retry_after_secs })
                }
                Err(other) => ApiError::Core(other),
            };
            Err(err)
        }
        ConsumeOutcome::Unknown => Err(ApiError::Core(CoreError::Conflict(
            "no pending code for this email — request a new one".into(),
        ))),
        ConsumeOutcome::Expired => Err(ApiError::Core(CoreError::Conflict(
            "code expired — request a new one".into(),
        ))),
        ConsumeOutcome::Locked => Err(ApiError::Core(CoreError::Conflict(
            "too many wrong attempts — request a new code".into(),
        ))),
    }
}

/// Successful-consume path: find-or-create the user, mark verified
/// (the OTP proves email ownership), record the login, mint tokens.
async fn issue_tokens_for(
    state: &AppState,
    pool: &SqlitePool,
    workspace: &str,
    email: &str,
) -> Result<Json<OtpLoginResponse>, ApiError> {
    let mut just_signed_up = false;
    let user: User = match find_user_by_email(pool, email).await? {
        Some(u) => {
            if !u.verified {
                mark_verified(pool, &u.id).await?;
            }
            u
        }
        None => {
            let fresh = insert_passwordless_user(pool, email).await?;
            mark_verified(pool, &fresh.id).await?;
            just_signed_up = true;
            tracing::info!(
                workspace = %workspace,
                user_id = %fresh.id,
                email = %email,
                "user signed up via email OTP"
            );
            fresh
        }
    };

    let public = serde_json::json!({
        "id": &user.id,
        "email": &user.email,
        "verified": true,
    });
    // Fan-out hooks across every app in the workspace until
    // workspace-scoped hook loading lands.
    let apps = rustbase_db::apps::list_apps(pool).await?;
    for app in &apps {
        let hook_req = rustbase_runtime::HookRequest::system(workspace, &app.id, "_user");
        if just_signed_up
            && let Err(e) = state
                .hooks
                .dispatch_user_after_register(workspace, &app.id, &hook_req, &public)
                .await
        {
            tracing::warn!(error = %e, %workspace, app = %app.id, "user_after_register hook errored");
        }

        state
            .hooks
            .dispatch_user_before_login(workspace, &app.id, &hook_req, &public)
            .await
            .map_err(|e| match e {
                rustbase_runtime::RuntimeError::Veto(msg) => {
                    tracing::info!(
                        %workspace,
                        app = %app.id,
                        user_id = %user.id,
                        %msg,
                        "login vetoed by hook"
                    );
                    ApiError::Core(CoreError::Forbidden)
                }
                other => ApiError::Core(CoreError::Internal(other.to_string())),
            })?;
    }

    let claims = build_claims(
        user.id.clone(),
        TokenRole::User,
        Some(workspace.to_string()),
        // Workspace-shared identity → no `app` claim.
        None,
        default_access_ttl(),
    );
    let access_token = state.jwt.issue(&claims)?;
    // last_login + refresh insert in one txn — one fsync.
    let refresh =
        commit_user_login(pool, &user.id, &new_refresh_token(), default_refresh_ttl()).await?;

    tracing::info!(workspace = %workspace, user_id = %user.id, "user login via email OTP");

    for app in &apps {
        let hook_req = rustbase_runtime::HookRequest::system(workspace, &app.id, "_user");
        if let Err(e) = state
            .hooks
            .dispatch_user_after_login(workspace, &app.id, &hook_req, &public)
            .await
        {
            tracing::warn!(error = %e, %workspace, app = %app.id, "user_after_login hook errored");
        }
    }

    Ok(Json(OtpLoginResponse {
        access_token,
        refresh_token: refresh.token,
        user: UserPublic {
            id: user.id,
            email: user.email,
            verified: true,
        },
    }))
}

/// Six random digits. OsRng → u32 → string with leading zeros so
/// `000123` is a valid code (and stored as such).
fn fresh_otp_code() -> String {
    let n = OsRng.next_u32() % 1_000_000;
    format!("{n:06}")
}
