use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rustbase_auth::hash_password;
use rustbase_core::{CoreError, WorkspaceId};
use rustbase_db::users::{find_user_by_email, insert_user};
use serde::{Deserialize, Serialize};
use validator::Validate;

use super::require_workspace_exists;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize, Validate)]
pub struct RegisterRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 8, max = 256))]
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub id: String,
    pub email: String,
}

/// `POST /api/workspaces/:workspace/auth/users/register` — self-service
/// end-user signup. The created user must still call `…/login` to receive
/// tokens. End-users are workspace-scoped: a single `(email, workspace)`
/// pair is valid across every app in the workspace.
pub async fn user_register(
    State(state): State<AppState>,
    Path(workspace): Path<String>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<RegisterResponse>), ApiError> {
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;

    require_workspace_exists(&state, &workspace).await?;

    let workspace_id = WorkspaceId::from(workspace.clone());
    let pool = state.workspaces.pool_for(&workspace_id).await?;

    if find_user_by_email(&pool, &req.email).await?.is_some() {
        return Err(ApiError::Core(CoreError::Conflict(format!(
            "email '{}' already registered in workspace '{}'",
            req.email, workspace
        ))));
    }

    let hash = hash_password(&req.password)?;
    let user = insert_user(&pool, &req.email, &hash).await?;

    tracing::info!(
        workspace = %workspace,
        user_id = %user.id,
        email = %user.email,
        "user registered"
    );

    // User-lifecycle hooks load against `(workspace, app)` pairs. With
    // workspace-shared identity there is no specific app; fire across
    // every app in the workspace so per-app `onUserAfterRegister`
    // handlers still run. Failures are logged and swallowed.
    let public = serde_json::json!({
        "id": user.id,
        "email": user.email,
        "verified": user.verified,
    });
    let apps = rustbase_db::apps::list_apps(&pool).await?;
    for app in &apps {
        let hook_req = rustbase_runtime::HookRequest::system(&workspace, &app.id, "_user");
        if let Err(e) = state
            .hooks
            .dispatch_user_after_register(&workspace, &app.id, &hook_req, &public)
            .await
        {
            tracing::warn!(error = %e, workspace = %workspace, app = %app.id, "user_after_register hook errored");
        }
    }

    Ok((
        StatusCode::CREATED,
        Json(RegisterResponse {
            id: user.id,
            email: user.email,
        }),
    ))
}
