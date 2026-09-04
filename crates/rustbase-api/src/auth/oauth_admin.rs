//! Admin CRUD for per-workspace OAuth provider configuration.
//!
//! Four endpoints under `/api/workspaces/:workspace/auth/oauth/providers`:
//!
//! - `GET    /`             list providers (summary; no secrets)
//! - `GET    /:provider`    fetch one provider (summary; no secret)
//! - `PUT    /:provider`    upsert provider + client_secret
//! - `DELETE /:provider`    remove the row
//!
//! Auth: requires master OR workspace-admin of the target workspace —
//! `AdminAuth::require_workspace_access` enforces the matrix.
//! Workspace-shared identity means a provider configured here applies
//! to every app in the workspace.
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
use rustbase_core::{CoreError, WorkspaceId};
use rustbase_db::oauth_providers::{
    self, OAuthProvider, OAuthProviderConfig, OAuthProviderSummary,
};
use serde::{Deserialize, Serialize};

use crate::auth::{AdminAuth, require_workspace_exists};
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct PutProviderBody {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
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

/// `GET /api/workspaces/:workspace/auth/oauth/providers`.
pub async fn list(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(workspace): Path<String>,
) -> Result<Json<Vec<ProviderResponse>>, ApiError> {
    auth.require_workspace_access(&workspace)?;
    let pool = workspace_pool(&state, &workspace).await?;
    let rows = oauth_providers::list_providers(&pool).await?;
    Ok(Json(rows.into_iter().map(ProviderResponse::from).collect()))
}

/// `GET /api/workspaces/:workspace/auth/oauth/providers/:provider`.
pub async fn get(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((workspace, provider)): Path<(String, String)>,
) -> Result<Json<ProviderResponse>, ApiError> {
    auth.require_workspace_access(&workspace)?;
    let pool = workspace_pool(&state, &workspace).await?;
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

/// `PUT /api/workspaces/:workspace/auth/oauth/providers/:provider`.
pub async fn put(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((workspace, provider)): Path<(String, String)>,
    Json(body): Json<PutProviderBody>,
) -> Result<Json<ProviderResponse>, ApiError> {
    auth.require_workspace_access(&workspace)?;
    let pool = workspace_pool(&state, &workspace).await?;

    let secret_enc = match body.client_secret.as_deref().filter(|s| !s.is_empty()) {
        Some(plain) => {
            let kek = state.oauth_kek.as_ref().as_ref().ok_or_else(|| {
                ApiError::Core(CoreError::Unavailable(
                    "no key-encryption key: set RUSTBASE_KEK (32 hex bytes, \
                     e.g. `openssl rand -hex 32`) before storing an OAuth \
                     client secret"
                        .into(),
                ))
            })?;
            rustbase_auth::encrypt(plain.as_bytes(), kek).map_err(|e| {
                ApiError::Core(CoreError::Internal(format!("encrypt client_secret: {e}")))
            })?
        }
        None => {
            let existing = oauth_providers::find_provider(&pool, &provider)
                .await?
                .ok_or(ApiError::Core(CoreError::Validation(
                    "client_secret is required when creating a provider".into(),
                )))?;
            existing.secret_enc
        }
    };

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

    tracing::info!(workspace = %workspace, provider = %provider, "OAuth provider upserted");
    Ok(Json(ProviderResponse {
        provider,
        client_id: body.client_id,
        config: body.config,
    }))
}

/// `DELETE /api/workspaces/:workspace/auth/oauth/providers/:provider`.
pub async fn delete(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((workspace, provider)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    auth.require_workspace_access(&workspace)?;
    let pool = workspace_pool(&state, &workspace).await?;
    let n = oauth_providers::delete_provider(&pool, &provider).await?;
    if n == 0 {
        return Err(ApiError::Core(CoreError::NotFound {
            collection: "oauth_provider".into(),
            id: provider,
        }));
    }
    tracing::info!(workspace = %workspace, provider = %provider, "OAuth provider deleted");
    Ok(StatusCode::NO_CONTENT)
}

async fn workspace_pool(state: &AppState, workspace: &str) -> Result<sqlx::SqlitePool, ApiError> {
    require_workspace_exists(state, workspace).await?;
    Ok(state
        .workspaces
        .pool_for(&WorkspaceId::from(workspace.to_string()))
        .await?)
}
