use axum::{
    Json,
    extract::{Path, State},
    response::{IntoResponse, Response},
};
use rustbase_auth::{TokenRole, build_claims, verify_password};

use super::cookies::{CookieFlags, build_access_cookie, build_refresh_cookie};
use rustbase_core::{CoreError, WorkspaceId};
use rustbase_db::{
    admins::{find_master_admin_by_username, find_workspace_admin_by_email},
    tokens::{SubjectKind, commit_user_login, insert_refresh_token},
    users::find_user_by_email,
};
use serde::{Deserialize, Serialize};
use validator::Validate;

use super::audit_events::{AuthEvent, AuthOutcome, Scope, record as record_audit};
use super::{default_access_ttl, default_refresh_ttl, new_refresh_token, require_workspace_exists};
use crate::error::ApiError;
use crate::state::AppState;

/// Attach `Set-Cookie: rb_at=...` and `Set-Cookie: rb_rt=...` to the
/// response carrying the JSON `body`. Used by every dashboard-facing
/// login path so a browser session no longer needs to keep tokens in
/// `localStorage`.
fn with_session_cookies<T: serde::Serialize>(
    state: &AppState,
    body: T,
    access_token: &str,
    refresh_token: &str,
) -> Response {
    let flags = CookieFlags {
        secure: state.cookie_secure,
    };
    let access_cookie = build_access_cookie(
        access_token,
        super::default_access_ttl().num_seconds(),
        flags,
    );
    let refresh_cookie = build_refresh_cookie(
        refresh_token,
        super::default_refresh_ttl().num_seconds(),
        flags,
    );
    let mut resp = Json(body).into_response();
    let headers = resp.headers_mut();
    headers.append(axum::http::header::SET_COOKIE, access_cookie);
    headers.append(axum::http::header::SET_COOKIE, refresh_cookie);
    resp
}

/// Subject keys used by both the lockout map and the audit log.
fn master_subject(username: &str) -> String {
    format!("master:{username}")
}
fn workspace_admin_subject(workspace: &str, email: &str) -> String {
    format!("workspace:{workspace}:admin:{email}")
}
fn user_subject(workspace: &str, email: &str) -> String {
    format!("workspace:{workspace}:user:{email}")
}

/// Translate a `LoginAttempts::note_failure` outcome into the audit
/// record + the response error. The boolean we return tells the caller
/// whether the failure also tripped the lockout (so the response should
/// be 429 instead of 401).
async fn record_failure_and_pick_error(
    state: &AppState,
    scope: Scope<'_>,
    subject: &str,
    target: Option<&str>,
) -> ApiError {
    match state
        .login_attempts
        .note_failure(subject, &state.lockout_policy)
    {
        Ok(()) => {
            record_audit(
                state,
                AuthEvent {
                    outcome: AuthOutcome::Failed,
                    scope,
                    subject,
                    target,
                    details: serde_json::json!({}),
                },
            )
            .await;
            ApiError::Core(CoreError::Unauthorized)
        }
        Err(CoreError::TooManyRequests { retry_after_secs }) => {
            record_audit(
                state,
                AuthEvent {
                    outcome: AuthOutcome::Locked,
                    scope,
                    subject,
                    target,
                    details: serde_json::json!({ "retry_after_secs": retry_after_secs }),
                },
            )
            .await;
            ApiError::Core(CoreError::TooManyRequests { retry_after_secs })
        }
        Err(other) => ApiError::Core(other),
    }
}

/// Master admin login takes a username (default `admin`); email is optional.
#[derive(Debug, Deserialize, Validate)]
pub struct MasterLoginRequest {
    #[validate(length(min = 1, max = 100))]
    pub username: String,
    #[validate(length(min = 1))]
    pub password: String,
}

/// Workspace-admin and end-user logins still use email.
#[derive(Debug, Deserialize, Validate)]
pub struct LoginRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 1))]
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct MasterAdminPublic {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AdminPublic {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MasterLoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub admin: MasterAdminPublic,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub admin: AdminPublic,
}

#[derive(Debug, Serialize)]
pub struct UserPublic {
    pub id: String,
    pub email: String,
    pub verified: bool,
}

/// Two-shape response: full tokens for non-2FA users, or an
/// `mfa_token` challenge when the user has TOTP enabled and must
/// complete the second step. Clients disambiguate by presence of
/// `mfa_required: true`.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum UserLoginResponse {
    Tokens {
        access_token: String,
        refresh_token: String,
        user: UserPublic,
    },
    MfaRequired {
        mfa_required: bool,
        mfa_token: String,
    },
}

pub async fn master_admin_login(
    State(state): State<AppState>,
    Json(req): Json<MasterLoginRequest>,
) -> Result<Response, ApiError> {
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;

    let subject = master_subject(&req.username);
    state
        .login_attempts
        .check(&subject, &state.lockout_policy)?;

    let admin = match find_master_admin_by_username(state.system.pool(), &req.username).await? {
        Some(a) => a,
        None => {
            return Err(record_failure_and_pick_error(
                &state,
                Scope::Master,
                &subject,
                Some(&req.username),
            )
            .await);
        }
    };

    // The setup wizard hasn't run yet — surface this as Unauthorized so
    // we don't leak the existence of the seed row. The setup gate
    // already kept this handler unreachable in the uninitialized state,
    // but defense-in-depth keeps the message uniform.
    let hash = match admin.password_hash.as_deref() {
        Some(h) => h,
        None => {
            return Err(record_failure_and_pick_error(
                &state,
                Scope::Master,
                &subject,
                Some(&req.username),
            )
            .await);
        }
    };

    if !verify_password(&req.password, hash)? {
        return Err(record_failure_and_pick_error(
            &state,
            Scope::Master,
            &subject,
            Some(&req.username),
        )
        .await);
    }
    state.login_attempts.note_success(&subject);

    let claims = build_claims(
        admin.id.clone(),
        TokenRole::MasterAdmin,
        None,
        None,
        default_access_ttl(),
    );
    let access_token = state.jwt.issue(&claims)?;

    let issued = new_refresh_token();
    insert_refresh_token(
        state.system.pool(),
        &issued,
        SubjectKind::MasterAdmin,
        &admin.id,
        default_refresh_ttl(),
    )
    .await?;

    tracing::info!(admin_id = %admin.id, username = %admin.username, "master admin login");
    record_audit(
        &state,
        AuthEvent {
            outcome: AuthOutcome::Success,
            scope: Scope::Master,
            subject: &subject,
            target: Some(&admin.username),
            details: serde_json::json!({ "admin_id": &admin.id }),
        },
    )
    .await;

    let body = MasterLoginResponse {
        access_token: access_token.clone(),
        refresh_token: issued.clone(),
        admin: MasterAdminPublic {
            id: admin.id,
            username: admin.username,
            email: admin.email,
            name: admin.name,
        },
    };
    Ok(with_session_cookies(&state, body, &access_token, &issued))
}

pub async fn workspace_admin_login(
    State(state): State<AppState>,
    Path(workspace): Path<String>,
    Json(req): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;

    let subject = workspace_admin_subject(&workspace, &req.email);
    state
        .login_attempts
        .check(&subject, &state.lockout_policy)?;

    // Look up the workspace's pool — if the workspace doesn't exist, the row
    // will be missing in system.db. We don't surface that here; just
    // return Unauthorized so an attacker can't enumerate workspaces.
    let workspace_id = WorkspaceId::from(workspace.clone());
    let pool = state.workspaces.pool_for(&workspace_id).await?;

    let admin = match find_workspace_admin_by_email(&pool, &req.email).await? {
        Some(a) => a,
        None => {
            return Err(record_failure_and_pick_error(
                &state,
                Scope::Workspace(&workspace),
                &subject,
                Some(&req.email),
            )
            .await);
        }
    };

    if !verify_password(&req.password, &admin.password_hash)? {
        return Err(record_failure_and_pick_error(
            &state,
            Scope::Workspace(&workspace),
            &subject,
            Some(&req.email),
        )
        .await);
    }
    state.login_attempts.note_success(&subject);

    let claims = build_claims(
        admin.id.clone(),
        TokenRole::WorkspaceAdmin,
        Some(workspace.clone()),
        None,
        default_access_ttl(),
    );
    let access_token = state.jwt.issue(&claims)?;

    let issued = new_refresh_token();
    insert_refresh_token(
        &pool,
        &issued,
        SubjectKind::WorkspaceAdmin,
        &admin.id,
        default_refresh_ttl(),
    )
    .await?;

    tracing::info!(workspace = %workspace, admin_id = %admin.id, "workspace admin login");
    record_audit(
        &state,
        AuthEvent {
            outcome: AuthOutcome::Success,
            scope: Scope::Workspace(&workspace),
            subject: &subject,
            target: Some(&admin.email),
            details: serde_json::json!({ "admin_id": &admin.id }),
        },
    )
    .await;

    let body = LoginResponse {
        access_token: access_token.clone(),
        refresh_token: issued.clone(),
        admin: AdminPublic {
            id: admin.id,
            email: admin.email,
            name: admin.name,
        },
    };
    Ok(with_session_cookies(&state, body, &access_token, &issued))
}

pub async fn user_login(
    State(state): State<AppState>,
    Path(workspace): Path<String>,
    Json(req): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;

    let subject = user_subject(&workspace, &req.email);
    state
        .login_attempts
        .check(&subject, &state.lockout_policy)?;

    require_workspace_exists(&state, &workspace).await?;

    let workspace_id = WorkspaceId::from(workspace.clone());
    let pool = state.workspaces.pool_for(&workspace_id).await?;

    let user = match find_user_by_email(&pool, &req.email).await? {
        Some(u) => u,
        None => {
            return Err(record_failure_and_pick_error(
                &state,
                Scope::Workspace(&workspace),
                &subject,
                Some(&req.email),
            )
            .await);
        }
    };

    let hash = match user.password_hash.as_deref() {
        Some(h) => h,
        None => {
            return Err(record_failure_and_pick_error(
                &state,
                Scope::Workspace(&workspace),
                &subject,
                Some(&req.email),
            )
            .await);
        }
    };
    if !verify_password(&req.password, hash)? {
        return Err(record_failure_and_pick_error(
            &state,
            Scope::Workspace(&workspace),
            &subject,
            Some(&req.email),
        )
        .await);
    }
    // Password ok — but a TOTP-enabled user still has to complete the
    // second factor before the lockout is considered cleared.
    let totp_enabled = crate::auth::totp::is_user_totp_enabled(&pool, &user.id).await?;
    if !totp_enabled {
        state.login_attempts.note_success(&subject);
    }

    // Credential check passed. Fire onUserBeforeLogin across every
    // app in the workspace — any app's hook can veto the login.
    // Workspace-shared identity means no specific app owns the user;
    // until workspace-scoped hooks land, per-app handlers each get
    // a turn so existing setups keep working.
    let public = serde_json::json!({
        "id": &user.id,
        "email": &user.email,
        "verified": user.verified,
    });
    let apps = rustbase_db::apps::list_apps(&pool).await?;
    for app in &apps {
        let hook_req = rustbase_runtime::HookRequest::system(&workspace, &app.id, "_user");
        state
            .hooks
            .dispatch_user_before_login(&workspace, &app.id, &hook_req, &public)
            .await
            .map_err(|e| match e {
                rustbase_runtime::RuntimeError::Veto(msg) => {
                    tracing::info!(
                        workspace = %workspace,
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

    // TOTP gate: if the user has 2FA enabled, don't issue tokens
    // here. Mint a one-shot mfa_token and let the client complete the
    // login via /auth/users/login/totp. The user-lifecycle
    // `after_login` event waits for the second step too — semantically
    // login isn't complete until both factors are accepted.
    if totp_enabled {
        let mfa_token = crate::auth::totp::issue_mfa_challenge(&pool, &user.id).await?;
        tracing::info!(
            workspace = %workspace,
            user_id = %user.id,
            "password ok; awaiting TOTP second step"
        );
        // MFA challenge: no session cookies yet — the access token
        // is only minted after the second step.
        return Ok(Json(UserLoginResponse::MfaRequired {
            mfa_required: true,
            mfa_token,
        })
        .into_response());
    }

    let claims = build_claims(
        user.id.clone(),
        TokenRole::User,
        Some(workspace.clone()),
        // Workspace-shared identity → no `app` claim on user tokens.
        None,
        default_access_ttl(),
    );
    let access_token = state.jwt.issue(&claims)?;

    // Bump users.last_login + insert the refresh row in one txn so the
    // happy path costs one fsync instead of two.
    let issued = new_refresh_token();
    commit_user_login(&pool, &user.id, &issued, default_refresh_ttl()).await?;

    tracing::info!(workspace = %workspace, user_id = %user.id, "user login");
    record_audit(
        &state,
        AuthEvent {
            outcome: AuthOutcome::Success,
            scope: Scope::Workspace(&workspace),
            subject: &subject,
            target: Some(&user.email),
            details: serde_json::json!({
                "user_id": &user.id,
            }),
        },
    )
    .await;

    // Best-effort observer fire for every app in the workspace.
    for app in &apps {
        let hook_req = rustbase_runtime::HookRequest::system(&workspace, &app.id, "_user");
        if let Err(e) = state
            .hooks
            .dispatch_user_after_login(&workspace, &app.id, &hook_req, &public)
            .await
        {
            tracing::warn!(
                error = %e,
                workspace = %workspace,
                app = %app.id,
                "user_after_login hook errored"
            );
        }
    }

    let body = UserLoginResponse::Tokens {
        access_token: access_token.clone(),
        refresh_token: issued.clone(),
        user: UserPublic {
            id: user.id,
            email: user.email,
            verified: user.verified,
        },
    };
    Ok(with_session_cookies(&state, body, &access_token, &issued))
}
