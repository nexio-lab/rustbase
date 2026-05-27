//! Admin endpoints for end-user management.
//!
//! Five routes under `/api/realms/{realm}/users`, all gated by
//! `AdminAuth::require_realm_access` (master OR realm-admin of the
//! target realm):
//!
//! - `GET    /`               paginated list with optional `?q=<email_substring>`
//! - `GET    /:id`            user detail + TOTP status + linked OAuth providers
//! - `PATCH  /:id/verify`     force-flip `verified = 1` (for support recovery)
//! - `DELETE /:id/totp`       remove the TOTP row, unlocking a user who lost their device
//! - `DELETE /:id`            cascade-delete the user
//!
//! The self-service flows under `/auth/users/*` are untouched — this
//! module is purely the admin surface.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use rustbase_core::{CoreError, RealmId};
use rustbase_db::{
    oauth_links::{self, OAuthLink},
    realms::find_realm,
    user_totp::{self, UserTotp},
    users::{self, User, find_user_by_id},
};
use serde::{Deserialize, Serialize};

use crate::auth::AdminAuth;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_per_page")]
    pub per_page: u32,
    /// Substring match on email. Empty / absent → list every user.
    #[serde(default)]
    pub q: Option<String>,
}

fn default_page() -> u32 {
    1
}
fn default_per_page() -> u32 {
    30
}

#[derive(Debug, Serialize)]
pub struct UserPublic {
    pub id: String,
    pub email: String,
    pub verified: bool,
    pub has_password: bool,
    pub last_login: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<User> for UserPublic {
    fn from(u: User) -> Self {
        let has_password = u.password_hash.is_some();
        Self {
            id: u.id,
            email: u.email,
            verified: u.verified,
            has_password,
            last_login: u.last_login,
            created_at: u.created_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct UserListResponse {
    pub items: Vec<UserPublic>,
    pub page: u32,
    pub per_page: u32,
    pub total_items: i64,
    pub total_pages: i64,
}

#[derive(Debug, Serialize)]
pub struct UserDetailResponse {
    #[serde(flatten)]
    pub user: UserPublic,
    pub totp: Option<TotpStatus>,
    pub oauth_links: Vec<OAuthLinkPublic>,
}

#[derive(Debug, Serialize)]
pub struct TotpStatus {
    pub enabled: bool,
    pub enrolled_at: chrono::DateTime<chrono::Utc>,
    pub confirmed_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<UserTotp> for TotpStatus {
    fn from(t: UserTotp) -> Self {
        Self {
            enabled: t.enabled,
            enrolled_at: t.enrolled_at,
            confirmed_at: t.confirmed_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct OAuthLinkPublic {
    pub provider: String,
    pub provider_user_id: String,
}

impl From<OAuthLink> for OAuthLinkPublic {
    fn from(l: OAuthLink) -> Self {
        Self {
            provider: l.provider,
            provider_user_id: l.provider_user_id,
        }
    }
}

// ---- handlers ----

/// `GET /api/realms/:realm/users`.
pub async fn list(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(realm): Path<String>,
    Query(q): Query<ListQuery>,
) -> Result<Json<UserListResponse>, ApiError> {
    auth.require_realm_access(&realm)?;
    let pool = realm_pool(&state, &realm).await?;

    let needle = q.q.as_deref().unwrap_or("").trim().to_string();
    let per = q.per_page.clamp(1, 200);
    let page = q.page.max(1);
    let offset = ((page - 1) as i64) * per as i64;

    let rows = users::list_users(&pool, &needle, per as i64, offset).await?;
    let total = users::count_users(&pool, &needle).await?;
    // div_ceil is unstable on i64. total >= 0 + per > 0 by construction
    // (clamp above), so the manual ceil division is safe.
    let total_pages = if per == 0 {
        0
    } else {
        (total + per as i64 - 1) / per as i64
    };

    Ok(Json(UserListResponse {
        items: rows.into_iter().map(UserPublic::from).collect(),
        page,
        per_page: per,
        total_items: total,
        total_pages,
    }))
}

/// `GET /api/realms/:realm/users/:id`.
pub async fn get(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, id)): Path<(String, String)>,
) -> Result<Json<UserDetailResponse>, ApiError> {
    auth.require_realm_access(&realm)?;
    let pool = realm_pool(&state, &realm).await?;

    let user = find_user_by_id(&pool, &id)
        .await?
        .ok_or(ApiError::Core(CoreError::NotFound {
            collection: "user".into(),
            id: id.clone(),
        }))?;
    let totp = user_totp::find(&pool, &id).await?.map(TotpStatus::from);
    let links = oauth_links::list_for_user(&pool, &id)
        .await?
        .into_iter()
        .map(OAuthLinkPublic::from)
        .collect();

    Ok(Json(UserDetailResponse {
        user: user.into(),
        totp,
        oauth_links: links,
    }))
}

/// `PATCH /api/realms/:realm/users/:id/verify`. Force the verified
/// flag on. Idempotent: re-verifying an already-verified user is a
/// no-op write.
pub async fn verify(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    auth.require_realm_access(&realm)?;
    let pool = realm_pool(&state, &realm).await?;

    if find_user_by_id(&pool, &id).await?.is_none() {
        return Err(ApiError::Core(CoreError::NotFound {
            collection: "user".into(),
            id,
        }));
    }
    rustbase_db::users::mark_verified(&pool, &id).await?;
    tracing::info!(realm = %realm, user_id = %id, "admin force-verified user");
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/realms/:realm/users/:id/totp`. Used to unlock a user
/// who lost access to their authenticator app. The user's password
/// stays untouched; their next login skips the TOTP step until they
/// re-enroll.
pub async fn reset_totp(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    auth.require_realm_access(&realm)?;
    let pool = realm_pool(&state, &realm).await?;

    if find_user_by_id(&pool, &id).await?.is_none() {
        return Err(ApiError::Core(CoreError::NotFound {
            collection: "user".into(),
            id,
        }));
    }
    user_totp::disable(&pool, &id).await?;
    tracing::info!(realm = %realm, user_id = %id, "admin reset TOTP for user");
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/realms/:realm/users/:id`. Cascade-deletes the user
/// and every auth-side row referencing them (verifications, resets,
/// otps, totp, mfa challenges, oauth links).
pub async fn delete(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    auth.require_realm_access(&realm)?;
    let pool = realm_pool(&state, &realm).await?;

    let n = users::delete_user(&pool, &id).await?;
    if n == 0 {
        return Err(ApiError::Core(CoreError::NotFound {
            collection: "user".into(),
            id,
        }));
    }
    tracing::info!(realm = %realm, user_id = %id, "admin deleted user");
    Ok(StatusCode::NO_CONTENT)
}

async fn realm_pool(state: &AppState, realm: &str) -> Result<sqlx::SqlitePool, ApiError> {
    find_realm(state.system.pool(), realm)
        .await?
        .ok_or(ApiError::Core(CoreError::RealmNotFound(realm.to_string())))?;
    Ok(state
        .realms
        .pool_for(&RealmId::from(realm.to_string()))
        .await?)
}
