//! Endpoints for managing apps under a workspace.
//!
//! - `GET    /api/workspaces/:workspace/apps`         list apps in a workspace
//! - `POST   /api/workspaces/:workspace/apps`         create an app + init its data.db
//! - `GET    /api/workspaces/:workspace/apps/:app`    fetch one
//! - `PATCH  /api/workspaces/:workspace/apps/:app`    rename
//! - `DELETE /api/workspaces/:workspace/apps/:app`    cascade-delete the app
//!
//! All five accept either a master admin or a workspace admin for the
//! target workspace. App admins (single-app scope) are deliberately
//! excluded — they manage their own app's data, not the app's identity.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rustbase_core::{AppId, CoreError, WorkspaceId};
use rustbase_db::{
    APP_MIGRATIONS, App, apply_migrations,
    apps::{create_app, delete_app, find_app, list_apps, rename_app},
    paths,
    workspaces::find_workspace,
};
use serde::Deserialize;
use std::sync::Arc;
use validator::Validate;

use crate::auth::AdminAuth;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize, Validate)]
pub struct CreateAppRequest {
    pub id: String,
    #[validate(length(min = 1, max = 100))]
    pub name: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct UpdateAppRequest {
    #[validate(length(min = 1, max = 100))]
    pub name: String,
}

pub async fn list(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(workspace): Path<String>,
) -> Result<Json<Vec<App>>, ApiError> {
    auth.require_workspace_access(&workspace)?;
    require_realm_exists(&state, &workspace).await?;
    let pool = state
        .workspaces
        .pool_for(&WorkspaceId::from(workspace))
        .await?;
    Ok(Json(list_apps(&pool).await?))
}

pub async fn create(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(workspace): Path<String>,
    Json(req): Json<CreateAppRequest>,
) -> Result<(StatusCode, Json<App>), ApiError> {
    auth.require_workspace_access(&workspace)?;
    validate_app_id(&req.id)?;
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;

    require_realm_exists(&state, &workspace).await?;
    let workspace_id = WorkspaceId::from(workspace.clone());
    let workspace_pool = state.workspaces.pool_for(&workspace_id).await?;

    if find_app(&workspace_pool, &req.id).await?.is_some() {
        return Err(ApiError::Core(CoreError::Conflict(format!(
            "app '{}' already exists in workspace '{}'",
            req.id, workspace
        ))));
    }

    let app = create_app(&workspace_pool, &req.id, &req.name).await?;

    // Initialize the app's data.db.
    let app_id = AppId::from(req.id.clone());
    let app_pool = state.apps.pool_for(&workspace_id, &app_id).await?;
    apply_migrations(app_pool, APP_MIGRATIONS).await?;

    // Pick up any JS hooks dropped on disk before the app was created.
    let hooks_dir = state.data_dir.join("hooks").join(&workspace).join(&req.id);
    let bridge = crate::hook_bridge::ApiBridge::new(
        WorkspaceId::from(workspace.clone()),
        AppId::from(req.id.clone()),
        state.apps.clone(),
    )
    .into_sync();
    // Wrap the server-wide mailer in a per-(workspace, app) quota gate so a
    // runaway $app.mailer.send loop can't flood the relay. System-issued
    // mail (verify-email + password-reset endpoints) keeps using the
    // bare state.mailer and is intentionally not quota'd.
    let quoted = Arc::new(crate::mailer::QuotedMailer::new(
        state.mailer.clone(),
        WorkspaceId::from(workspace.clone()),
        AppId::from(req.id.clone()),
        state.apps.clone(),
    )) as Arc<dyn rustbase_core::Mailer>;
    if let Err(e) = state
        .hooks
        .load_app(&workspace, &req.id, &hooks_dir, Some(bridge), Some(quoted))
        .await
    {
        tracing::warn!(workspace = %workspace, app = %req.id, error = %e, "loading hooks failed");
    }

    tracing::info!(workspace = %workspace, app = %req.id, "app created");
    Ok((StatusCode::CREATED, Json(app)))
}

pub async fn get(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((workspace, app)): Path<(String, String)>,
) -> Result<Json<App>, ApiError> {
    auth.require_workspace_access(&workspace)?;
    require_realm_exists(&state, &workspace).await?;
    let pool = state
        .workspaces
        .pool_for(&WorkspaceId::from(workspace.clone()))
        .await?;
    let row = find_app(&pool, &app)
        .await?
        .ok_or(ApiError::Core(CoreError::AppNotFound { workspace, app }))?;
    Ok(Json(row))
}

pub async fn update(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((workspace, app)): Path<(String, String)>,
    Json(req): Json<UpdateAppRequest>,
) -> Result<Json<App>, ApiError> {
    auth.require_workspace_access(&workspace)?;
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;
    require_realm_exists(&state, &workspace).await?;

    let pool = state
        .workspaces
        .pool_for(&WorkspaceId::from(workspace.clone()))
        .await?;
    rename_app(&pool, &app, &req.name)
        .await
        .map_err(|e| match e {
            rustbase_db::DbError::Sqlx(sqlx::Error::RowNotFound) => {
                ApiError::Core(CoreError::AppNotFound {
                    workspace: workspace.clone(),
                    app: app.clone(),
                })
            }
            other => ApiError::from(other),
        })?;

    let row = find_app(&pool, &app)
        .await?
        .ok_or(ApiError::Core(CoreError::AppNotFound { workspace, app }))?;
    Ok(Json(row))
}

pub async fn delete(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((workspace, app)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    auth.require_workspace_access(&workspace)?;
    require_realm_exists(&state, &workspace).await?;

    let workspace_id = WorkspaceId::from(workspace.clone());
    let app_id = AppId::from(app.clone());
    let workspace_pool = state.workspaces.pool_for(&workspace_id).await?;

    find_app(&workspace_pool, &app).await?.ok_or_else(|| {
        ApiError::Core(CoreError::AppNotFound {
            workspace: workspace.clone(),
            app: app.clone(),
        })
    })?;

    state.apps.evict(&workspace_id, &app_id);
    delete_app(&workspace_pool, &app).await?;

    let dir = paths::app_dir(state.data_dir.as_ref(), &workspace_id, &app_id);
    if dir.exists() {
        tokio::fs::remove_dir_all(&dir).await.map_err(|e| {
            ApiError::Core(CoreError::Internal(format!(
                "failed to remove app folder: {e}"
            )))
        })?;
    }

    tracing::info!(workspace = %workspace, app = %app, "app deleted");
    Ok(StatusCode::NO_CONTENT)
}

async fn require_realm_exists(state: &AppState, workspace: &str) -> Result<(), ApiError> {
    find_workspace(state.system.pool(), workspace)
        .await?
        .ok_or(ApiError::Core(CoreError::WorkspaceNotFound(
            workspace.to_string(),
        )))?;
    Ok(())
}

/// App ids share the workspace-id slug rules.
fn validate_app_id(id: &str) -> Result<(), ApiError> {
    let len = id.len();
    if !(2..=50).contains(&len) {
        return Err(ApiError::Core(CoreError::Validation(
            "app id must be 2-50 characters".into(),
        )));
    }
    if id.starts_with('-') || id.ends_with('-') {
        return Err(ApiError::Core(CoreError::Validation(
            "app id must not start or end with '-'".into(),
        )));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(ApiError::Core(CoreError::Validation(
            "app id may only contain lowercase letters, digits, and '-'".into(),
        )));
    }
    Ok(())
}
