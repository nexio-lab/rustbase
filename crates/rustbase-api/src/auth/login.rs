use axum::{
    Json,
    extract::{Path, State},
};
use rustbase_auth::{TokenRole, build_claims, encode_token, verify_password};
use rustbase_core::{AppId, CoreError, RealmId};
use rustbase_db::{
    admins::{find_master_admin_by_username, find_realm_admin_by_email},
    apps::find_app,
    realms::find_realm,
    tokens::{SubjectKind, insert_refresh_token},
    users::{find_user_by_email, record_last_login},
};
use serde::{Deserialize, Serialize};
use validator::Validate;

use super::{default_access_ttl, default_refresh_ttl, new_refresh_token};
use crate::error::ApiError;
use crate::state::AppState;

/// Master admin login takes a username (default `admin`); email is optional.
#[derive(Debug, Deserialize, Validate)]
pub struct MasterLoginRequest {
    #[validate(length(min = 1, max = 100))]
    pub username: String,
    #[validate(length(min = 1))]
    pub password: String,
}

/// Realm-admin and end-user logins still use email.
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
) -> Result<Json<MasterLoginResponse>, ApiError> {
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;

    let admin = find_master_admin_by_username(state.system.pool(), &req.username)
        .await?
        .ok_or(ApiError::Core(CoreError::Unauthorized))?;

    // The setup wizard hasn't run yet — surface this as Unauthorized so
    // we don't leak the existence of the seed row. The setup gate
    // already kept this handler unreachable in the uninitialized state,
    // but defense-in-depth keeps the message uniform.
    let hash = admin
        .password_hash
        .as_deref()
        .ok_or(ApiError::Core(CoreError::Unauthorized))?;

    if !verify_password(&req.password, hash)? {
        return Err(ApiError::Core(CoreError::Unauthorized));
    }

    let claims = build_claims(
        admin.id.clone(),
        TokenRole::MasterAdmin,
        None,
        None,
        default_access_ttl(),
    );
    let access_token = encode_token(&claims, &state.master_key)?;

    let refresh = insert_refresh_token(
        state.system.pool(),
        &new_refresh_token(),
        SubjectKind::MasterAdmin,
        &admin.id,
        default_refresh_ttl(),
    )
    .await?;

    tracing::info!(admin_id = %admin.id, username = %admin.username, "master admin login");

    Ok(Json(MasterLoginResponse {
        access_token,
        refresh_token: refresh.token,
        admin: MasterAdminPublic {
            id: admin.id,
            username: admin.username,
            email: admin.email,
            name: admin.name,
        },
    }))
}

pub async fn realm_admin_login(
    State(state): State<AppState>,
    Path(realm): Path<String>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;

    // Look up the realm's pool — if the realm doesn't exist, the row
    // will be missing in system.db. We don't surface that here; just
    // return Unauthorized so an attacker can't enumerate realms.
    let realm_id = RealmId::from(realm.clone());
    let pool = state.realms.pool_for(&realm_id).await?;

    let admin = find_realm_admin_by_email(&pool, &req.email)
        .await?
        .ok_or(ApiError::Core(CoreError::Unauthorized))?;

    if !verify_password(&req.password, &admin.password_hash)? {
        return Err(ApiError::Core(CoreError::Unauthorized));
    }

    let claims = build_claims(
        admin.id.clone(),
        TokenRole::RealmAdmin,
        Some(realm.clone()),
        None,
        default_access_ttl(),
    );
    let access_token = encode_token(&claims, &state.master_key)?;

    let refresh = insert_refresh_token(
        &pool,
        &new_refresh_token(),
        SubjectKind::RealmAdmin,
        &admin.id,
        default_refresh_ttl(),
    )
    .await?;

    tracing::info!(realm = %realm, admin_id = %admin.id, "realm admin login");

    Ok(Json(LoginResponse {
        access_token,
        refresh_token: refresh.token,
        admin: AdminPublic {
            id: admin.id,
            email: admin.email,
            name: admin.name,
        },
    }))
}

pub async fn user_login(
    State(state): State<AppState>,
    Path(realm): Path<String>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<UserLoginResponse>, ApiError> {
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;

    let realm_id = RealmId::from(realm.clone());
    let pool = state.realms.pool_for(&realm_id).await?;

    let user = find_user_by_email(&pool, &req.email)
        .await?
        .ok_or(ApiError::Core(CoreError::Unauthorized))?;

    let hash = user
        .password_hash
        .as_deref()
        .ok_or(ApiError::Core(CoreError::Unauthorized))?;
    if !verify_password(&req.password, hash)? {
        return Err(ApiError::Core(CoreError::Unauthorized));
    }

    // Credential check passed. Fire onUserBeforeLogin — a hook may
    // throw to abort issuance (e.g. ban-list, geo-fence). Public
    // user shape only; never expose password_hash to hooks.
    let public = serde_json::json!({
        "id": &user.id,
        "email": &user.email,
        "verified": user.verified,
    });
    let hook_req = rustbase_runtime::HookRequest::system(&realm, "", "_user");
    state
        .hooks
        .dispatch_user_before_login(&realm, &hook_req, &public)
        .await
        .map_err(|e| match e {
            rustbase_runtime::RuntimeError::Veto(msg) => {
                tracing::info!(realm = %realm, user_id = %user.id, %msg, "login vetoed by hook");
                ApiError::Core(CoreError::Forbidden)
            }
            other => ApiError::Core(CoreError::Internal(other.to_string())),
        })?;

    // TOTP gate: if the user has 2FA enabled, don't issue tokens
    // here. Mint a one-shot mfa_token and let the client complete the
    // login via /auth/users/login/totp. The user-lifecycle
    // `after_login` event waits for the second step too — semantically
    // login isn't complete until both factors are accepted.
    if crate::auth::totp::is_user_totp_enabled(&pool, &user.id).await? {
        let mfa_token = crate::auth::totp::issue_mfa_challenge(&pool, &user.id).await?;
        tracing::info!(
            realm = %realm,
            user_id = %user.id,
            "password ok; awaiting TOTP second step"
        );
        return Ok(Json(UserLoginResponse::MfaRequired {
            mfa_required: true,
            mfa_token,
        }));
    }

    record_last_login(&pool, &user.id).await?;

    let claims = build_claims(
        user.id.clone(),
        TokenRole::User,
        Some(realm.clone()),
        None,
        default_access_ttl(),
    );
    let access_token = encode_token(&claims, &state.master_key)?;

    let refresh = insert_refresh_token(
        &pool,
        &new_refresh_token(),
        SubjectKind::User,
        &user.id,
        default_refresh_ttl(),
    )
    .await?;

    tracing::info!(realm = %realm, user_id = %user.id, "user login");

    // Best-effort observer fire; failures are logged but don't roll
    // back the successful login.
    if let Err(e) = state
        .hooks
        .dispatch_user_after_login(&realm, &hook_req, &public)
        .await
    {
        tracing::warn!(error = %e, realm = %realm, "user_after_login hook errored");
    }

    Ok(Json(UserLoginResponse::Tokens {
        access_token,
        refresh_token: refresh.token,
        user: UserPublic {
            id: user.id,
            email: user.email,
            verified: user.verified,
        },
    }))
}
