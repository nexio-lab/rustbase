//! Hierarchical policy endpoints.
//!
//! Three scopes, same shape:
//!
//! - `GET    /api/system/policies`                              master scope
//! - `GET    /api/system/policies/:field`
//! - `PUT    /api/system/policies/:field`                       master only;
//!   triggers an auto-clamp cascade down to every workspace + every app
//!   whose stored value would violate the new bound.
//! - `DELETE /api/system/policies/:field`
//!
//! - `GET    /api/workspaces/:workspace/policies`                       workspace scope;
//!   master OR workspace-admin
//! - `PUT    /api/workspaces/:workspace/policies/:field`                validated
//!   against the master bound (if any), then cascades to apps.
//! - `DELETE /api/workspaces/:workspace/policies/:field`
//!
//! - `GET    /api/workspaces/:workspace/apps/:app/policies`             app scope
//! - `PUT    /api/workspaces/:workspace/apps/:app/policies/:field`      validated
//!   against the workspace bound (if any).
//! - `DELETE /api/workspaces/:workspace/apps/:app/policies/:field`

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rustbase_core::{AppId, CoreError, PolicyLevel, PolicySpec, WorkspaceId, validate_chain};
use rustbase_db::{apps::find_app, audit, policies, policy_engine, workspaces::find_workspace};
use serde::Serialize;
use serde_json::json;

use crate::auth::AdminAuth;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct PolicyResponse {
    pub field: String,
    pub spec: PolicySpec,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub struct PutPolicyResponse {
    pub field: String,
    pub spec: PolicySpec,
    /// Auto-clamp outcomes when a parent change rippled into children.
    /// Empty when the change loosens or when no child stored a value.
    pub cascaded: Vec<policy_engine::ClampOutcome>,
}

// ============================================================
// system / master scope
// ============================================================

pub async fn system_list(
    auth: AdminAuth,
    State(state): State<AppState>,
) -> Result<Json<Vec<PolicyResponse>>, ApiError> {
    auth.require_master()?;
    let rows = policies::list_policies(state.system.pool()).await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| PolicyResponse {
                field: r.field,
                spec: r.spec,
                updated_at: r.updated_at,
            })
            .collect(),
    ))
}

pub async fn system_get(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(field): Path<String>,
) -> Result<Json<PolicySpec>, ApiError> {
    auth.require_master()?;
    let spec = policies::get_policy(state.system.pool(), &field)
        .await?
        .ok_or(ApiError::Core(CoreError::NotFound {
            collection: "policy".into(),
            id: field,
        }))?;
    Ok(Json(spec))
}

pub async fn system_put(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(field): Path<String>,
    Json(spec): Json<PolicySpec>,
) -> Result<Json<PutPolicyResponse>, ApiError> {
    auth.require_master()?;
    policies::upsert_policy(state.system.pool(), &field, &spec).await?;
    audit::append(
        state.system.pool(),
        Some(&auth.admin_id),
        "policy_set",
        Some(&field),
        &json!({"scope":"system","spec":spec}),
    )
    .await?;

    let cascaded = policy_engine::cascade_master_to_realms_and_apps(
        state.system.pool(),
        state.workspaces.clone(),
        state.apps.clone(),
        &field,
        &spec,
        Some(&auth.admin_id),
    )
    .await?;

    tracing::info!(
        field = %field,
        cascaded = cascaded.len(),
        "master policy updated"
    );

    Ok(Json(PutPolicyResponse {
        field,
        spec,
        cascaded,
    }))
}

pub async fn system_delete(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(field): Path<String>,
) -> Result<StatusCode, ApiError> {
    auth.require_master()?;
    policies::delete_policy(state.system.pool(), &field)
        .await
        .map_err(|e| match e {
            rustbase_db::DbError::Sqlx(sqlx::Error::RowNotFound) => {
                ApiError::Core(CoreError::NotFound {
                    collection: "policy".into(),
                    id: field.clone(),
                })
            }
            other => ApiError::from(other),
        })?;
    audit::append(
        state.system.pool(),
        Some(&auth.admin_id),
        "policy_deleted",
        Some(&field),
        &json!({"scope":"system"}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ============================================================
// workspace scope
// ============================================================

pub async fn workspace_list(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(workspace): Path<String>,
) -> Result<Json<Vec<PolicyResponse>>, ApiError> {
    auth.require_workspace_access(&workspace)?;
    require_realm_exists(&state, &workspace).await?;
    let workspace_pool = state
        .workspaces
        .pool_for(&WorkspaceId::from(workspace))
        .await?;
    let rows = policies::list_policies(&workspace_pool).await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| PolicyResponse {
                field: r.field,
                spec: r.spec,
                updated_at: r.updated_at,
            })
            .collect(),
    ))
}

pub async fn workspace_get(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((workspace, field)): Path<(String, String)>,
) -> Result<Json<PolicySpec>, ApiError> {
    auth.require_workspace_access(&workspace)?;
    require_realm_exists(&state, &workspace).await?;
    let workspace_pool = state
        .workspaces
        .pool_for(&WorkspaceId::from(workspace))
        .await?;
    let spec = policies::get_policy(&workspace_pool, &field)
        .await?
        .ok_or(ApiError::Core(CoreError::NotFound {
            collection: "policy".into(),
            id: field,
        }))?;
    Ok(Json(spec))
}

pub async fn workspace_put(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((workspace, field)): Path<(String, String)>,
    Json(spec): Json<PolicySpec>,
) -> Result<Json<PutPolicyResponse>, ApiError> {
    auth.require_workspace_access(&workspace)?;
    require_realm_exists(&state, &workspace).await?;

    // Walk master → workspace as a chain so the violation, if any, names
    // the offending tier ("password.length (workspace)") instead of just
    // the field.
    let mut chain = Vec::new();
    if let Some(master_spec) = policies::get_policy(state.system.pool(), &field).await? {
        chain.push(PolicyLevel::new("master", master_spec));
    }
    chain.push(PolicyLevel::new("workspace", spec.clone()));
    validate_chain(&field, &chain).map_err(ApiError::Core)?;

    let workspace_pool = state
        .workspaces
        .pool_for(&WorkspaceId::from(workspace.clone()))
        .await?;
    policies::upsert_policy(&workspace_pool, &field, &spec).await?;
    audit::append(
        &workspace_pool,
        Some(&auth.admin_id),
        "policy_set",
        Some(&field),
        &json!({"scope":"workspace","spec":spec}),
    )
    .await?;

    let cascaded = policy_engine::cascade_realm_to_apps(
        &workspace_pool,
        state.apps.clone(),
        &workspace,
        &field,
        &spec,
        Some(&auth.admin_id),
    )
    .await?;

    Ok(Json(PutPolicyResponse {
        field,
        spec,
        cascaded,
    }))
}

pub async fn workspace_delete(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((workspace, field)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    auth.require_workspace_access(&workspace)?;
    require_realm_exists(&state, &workspace).await?;
    let workspace_pool = state
        .workspaces
        .pool_for(&WorkspaceId::from(workspace.clone()))
        .await?;
    policies::delete_policy(&workspace_pool, &field)
        .await
        .map_err(|e| match e {
            rustbase_db::DbError::Sqlx(sqlx::Error::RowNotFound) => {
                ApiError::Core(CoreError::NotFound {
                    collection: "policy".into(),
                    id: field.clone(),
                })
            }
            other => ApiError::from(other),
        })?;
    audit::append(
        &workspace_pool,
        Some(&auth.admin_id),
        "policy_deleted",
        Some(&field),
        &json!({"scope":"workspace"}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ============================================================
// app scope
// ============================================================

pub async fn app_list(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((workspace, app)): Path<(String, String)>,
) -> Result<Json<Vec<PolicyResponse>>, ApiError> {
    auth.require_app_access(&workspace, &app)?;
    let app_pool = open_app_pool(&state, &workspace, &app).await?;
    let rows = policies::list_policies(&app_pool).await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| PolicyResponse {
                field: r.field,
                spec: r.spec,
                updated_at: r.updated_at,
            })
            .collect(),
    ))
}

pub async fn app_get(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((workspace, app, field)): Path<(String, String, String)>,
) -> Result<Json<PolicySpec>, ApiError> {
    auth.require_app_access(&workspace, &app)?;
    let app_pool = open_app_pool(&state, &workspace, &app).await?;
    let spec = policies::get_policy(&app_pool, &field)
        .await?
        .ok_or(ApiError::Core(CoreError::NotFound {
            collection: "policy".into(),
            id: field,
        }))?;
    Ok(Json(spec))
}

pub async fn app_put(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((workspace, app, field)): Path<(String, String, String)>,
    Json(spec): Json<PolicySpec>,
) -> Result<Json<PutPolicyResponse>, ApiError> {
    auth.require_app_access(&workspace, &app)?;
    let workspace_pool = state
        .workspaces
        .pool_for(&WorkspaceId::from(workspace.clone()))
        .await?;
    find_app(&workspace_pool, &app)
        .await?
        .ok_or(ApiError::Core(CoreError::AppNotFound {
            workspace: workspace.clone(),
            app: app.clone(),
        }))?;

    // Build the full master → workspace → app chain. A direct workspace vs
    // app check would normally suffice (the workspace bound is itself
    // inside master's), but walking everything is cheap and catches
    // the rare case where the master/workspace chain went stale.
    let mut chain = Vec::new();
    if let Some(s) = policies::get_policy(state.system.pool(), &field).await? {
        chain.push(PolicyLevel::new("master", s));
    }
    if let Some(s) = policies::get_policy(&workspace_pool, &field).await? {
        chain.push(PolicyLevel::new("workspace", s));
    }
    chain.push(PolicyLevel::new("app", spec.clone()));
    validate_chain(&field, &chain).map_err(ApiError::Core)?;

    let app_pool = state
        .apps
        .pool_for(
            &WorkspaceId::from(workspace.clone()),
            &AppId::from(app.clone()),
        )
        .await?;
    policies::upsert_policy(&app_pool, &field, &spec).await?;
    audit::append(
        &app_pool,
        Some(&auth.admin_id),
        "policy_set",
        Some(&field),
        &json!({"scope":"app","spec":spec}),
    )
    .await?;

    Ok(Json(PutPolicyResponse {
        field,
        spec,
        cascaded: vec![],
    }))
}

pub async fn app_delete(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((workspace, app, field)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    auth.require_app_access(&workspace, &app)?;
    let app_pool = open_app_pool(&state, &workspace, &app).await?;
    policies::delete_policy(&app_pool, &field)
        .await
        .map_err(|e| match e {
            rustbase_db::DbError::Sqlx(sqlx::Error::RowNotFound) => {
                ApiError::Core(CoreError::NotFound {
                    collection: "policy".into(),
                    id: field.clone(),
                })
            }
            other => ApiError::from(other),
        })?;
    audit::append(
        &app_pool,
        Some(&auth.admin_id),
        "policy_deleted",
        Some(&field),
        &json!({"scope":"app"}),
    )
    .await?;
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

async fn open_app_pool(
    state: &AppState,
    workspace: &str,
    app: &str,
) -> Result<sqlx::SqlitePool, ApiError> {
    require_realm_exists(state, workspace).await?;
    let workspace_id = WorkspaceId::from(workspace.to_string());
    let workspace_pool = state.workspaces.pool_for(&workspace_id).await?;
    find_app(&workspace_pool, app).await?.ok_or_else(|| {
        ApiError::Core(CoreError::AppNotFound {
            workspace: workspace.to_string(),
            app: app.to_string(),
        })
    })?;
    let app_id = AppId::from(app.to_string());
    Ok(state.apps.pool_for(&workspace_id, &app_id).await?)
}
