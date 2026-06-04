//! Master-admin endpoints for managing workspace admins.
//!
//! Workspace admins live in their workspace's `workspace.db`. They can be created
//! by a master admin (typically right after creating a workspace), and they
//! authenticate at `POST /api/workspaces/:workspace/auth/admin/login`.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rustbase_auth::hash_password;
use rustbase_core::{CoreError, WorkspaceId};
use rustbase_db::{
    admins::{find_workspace_admin_by_email, insert_realm_admin},
    workspaces::find_workspace,
};
use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::auth::AdminAuth;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize, Validate)]
pub struct CreateRealmAdminRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 8, max = 256))]
    pub password: String,
    #[validate(length(max = 100))]
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceAdminResponse {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// `POST /api/workspaces/:workspace/admins` — master only.
pub async fn create(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(workspace): Path<String>,
    Json(req): Json<CreateRealmAdminRequest>,
) -> Result<(StatusCode, Json<WorkspaceAdminResponse>), ApiError> {
    auth.require_master()?;
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;

    // Verify the workspace exists in system.db before touching its DB.
    find_workspace(state.system.pool(), &workspace)
        .await?
        .ok_or(ApiError::Core(CoreError::WorkspaceNotFound(
            workspace.clone(),
        )))?;

    let workspace_id = WorkspaceId::from(workspace.clone());
    let pool = state.workspaces.pool_for(&workspace_id).await?;

    if find_workspace_admin_by_email(&pool, &req.email)
        .await?
        .is_some()
    {
        return Err(ApiError::Core(CoreError::Conflict(format!(
            "workspace admin '{}' already exists in workspace '{}'",
            req.email, workspace
        ))));
    }

    let hash = hash_password(&req.password)?;
    let admin = insert_realm_admin(&pool, &req.email, &hash, req.name.as_deref()).await?;

    tracing::info!(workspace = %workspace, admin_id = %admin.id, email = %admin.email, "workspace admin created");

    Ok((
        StatusCode::CREATED,
        Json(WorkspaceAdminResponse {
            id: admin.id,
            email: admin.email,
            name: admin.name,
            created_at: admin.created_at,
        }),
    ))
}
