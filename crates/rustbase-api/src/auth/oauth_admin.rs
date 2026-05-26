//! Admin CRUD for per-realm OAuth provider configuration.
//!
//! Four endpoints under `/api/realms/:realm/auth/oauth/providers`:
//!
//! - `GET    /`             list providers (summary; no secrets)
//! - `GET    /:provider`    fetch one provider (summary; no secret)
//! - `PUT    /:provider`    upsert provider + client_secret
//! - `DELETE /:provider`    remove the row
//!
//! Auth: requires master OR realm-admin of the target realm — same
//! gate as the policy endpoints, via `AdminAuth::require_realm_access`.
//!
//! Secrets handling:
//!
//! - Inbound PUT bodies carry the secret in plaintext. We encrypt
//!   with the server-wide KEK (`state.oauth_kek`) before persisting,
//!   matching the format the callback path decrypts.
//! - GET responses never echo the secret in any form — neither
//!   plaintext nor ciphertext nor a masked stub. Once stored, the
//!   secret is opaque to read paths.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rustbase_core::{CoreError, RealmId};
use rustbase_db::{
    oauth_providers::{self, OAuthProvider, OAuthProviderConfig, OAuthProviderSummary},
    realms::find_realm,
};
use serde::{Deserialize, Serialize};

use crate::auth::AdminAuth;
use crate::error::ApiError;
use crate::state::AppState;

/// PUT body. `client_secret` is plaintext — server encrypts.
#[derive(Debug, Deserialize)]
pub struct PutProviderBody {
    pub client_id: String,
    pub client_secret: String,
    pub config: OAuthProviderConfig,
}

#[derive(Debug, Serialize)]
pub struct ProviderResponse {
    pub provider: String,
    pub client_id: String,
    pub config: OAuthProviderConfig,
}

impl From<OAuthProviderSummary> for ProviderResponse {
    fn from(s: OAuthProviderSummary) -> Self {
        Self {
            provider: s.provider,
            client_id: s.client_id,
            config: s.config,
        }
    }
}

/// `GET /api/realms/:realm/auth/oauth/providers`.
pub async fn list(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(realm): Path<String>,
) -> Result<Json<Vec<ProviderResponse>>, ApiError> {
    auth.require_realm_access(&realm)?;
    let pool = realm_pool(&state, &realm).await?;
    let rows = oauth_providers::list_providers(&pool).await?;
    Ok(Json(rows.into_iter().map(ProviderResponse::from).collect()))
}

/// `GET /api/realms/:realm/auth/oauth/providers/:provider`.
pub async fn get(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, provider)): Path<(String, String)>,
) -> Result<Json<ProviderResponse>, ApiError> {
    auth.require_realm_access(&realm)?;
    let pool = realm_pool(&state, &realm).await?;
    let row = oauth_providers::find_provider(&pool, &provider)
        .await?
        .ok_or(ApiError::Core(CoreError::NotFound {
            collection: "oauth_provider".into(),
            id: provider.clone(),
        }))?;
    Ok(Json(ProviderResponse {
        provider: row.provider,
        client_id: row.client_id,
        config: row.config,
    }))
}

/// `PUT /api/realms/:realm/auth/oauth/providers/:provider`.
pub async fn put(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, provider)): Path<(String, String)>,
    Json(body): Json<PutProviderBody>,
) -> Result<Json<ProviderResponse>, ApiError> {
    auth.require_realm_access(&realm)?;
    let pool = realm_pool(&state, &realm).await?;

    let secret_enc =
        rustbase_auth::encrypt(body.client_secret.as_bytes(), state.oauth_kek.as_ref()).map_err(
            |e| ApiError::Core(CoreError::Internal(format!("encrypt client_secret: {e}"))),
        )?;

    oauth_providers::upsert_provider(
        &pool,
        &OAuthProvider {
            provider: provider.clone(),
            client_id: body.client_id.clone(),
            secret_enc,
            config: body.config.clone(),
        },
    )
    .await?;

    tracing::info!(realm = %realm, provider = %provider, "OAuth provider upserted");
    Ok(Json(ProviderResponse {
        provider,
        client_id: body.client_id,
        config: body.config,
    }))
}

/// `DELETE /api/realms/:realm/auth/oauth/providers/:provider`.
pub async fn delete(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, provider)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    auth.require_realm_access(&realm)?;
    let pool = realm_pool(&state, &realm).await?;
    let n = oauth_providers::delete_provider(&pool, &provider).await?;
    if n == 0 {
        return Err(ApiError::Core(CoreError::NotFound {
            collection: "oauth_provider".into(),
            id: provider,
        }));
    }
    tracing::info!(realm = %realm, provider = %provider, "OAuth provider deleted");
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
