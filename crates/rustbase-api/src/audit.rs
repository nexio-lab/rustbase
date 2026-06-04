//! Audit log read endpoints, one per scope.
//!
//! - `GET /api/system/audit`              master scope (master admins only)
//! - `GET /api/workspaces/:workspace/audit`       workspace scope
//! - `GET /api/workspaces/:workspace/apps/:app/audit`  app scope
//!
//! All three accept the same `?page=&per_page=&action=&actor=` query
//! string and return the same `ListedAuditResponse` shape so a single
//! `AuditView` component in the dashboard can drive every scope. The
//! audit log is append-only; there is no PUT/DELETE on these routes.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use rustbase_core::{AppId, CoreError, WorkspaceId};
use rustbase_db::{
    apps::find_app,
    audit::{AuditQuery, list_paginated},
    workspaces::find_workspace,
};
use serde::{Deserialize, Serialize};

use crate::auth::AdminAuth;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct AuditListQuery {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub action: Option<String>,
    pub actor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuditEntryDto {
    pub id: i64,
    pub ts: String,
    pub actor: Option<String>,
    pub action: String,
    pub target: Option<String>,
    /// Already-parsed JSON when the stored details were valid JSON;
    /// `null` when the column is empty or unparseable.
    pub details: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct ListedAuditResponse {
    pub items: Vec<AuditEntryDto>,
    pub page: u32,
    pub per_page: u32,
    pub total_items: u64,
    pub total_pages: u64,
}

fn build_query(q: AuditListQuery) -> AuditQuery {
    AuditQuery {
        page: q.page.unwrap_or(1),
        per_page: q.per_page.unwrap_or(30),
        action: q.action.filter(|s| !s.is_empty()),
        actor: q.actor.filter(|s| !s.is_empty()),
    }
}

fn into_response(listed: rustbase_db::audit::ListedAudit) -> ListedAuditResponse {
    let per = listed.per_page.max(1) as u64;
    let total_pages = listed.total_items.div_ceil(per);
    let items = listed
        .items
        .into_iter()
        .map(|e| AuditEntryDto {
            id: e.id,
            ts: e.ts.to_rfc3339(),
            actor: e.actor,
            action: e.action,
            target: e.target,
            details: e
                .details_json
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(serde_json::Value::Null),
        })
        .collect();
    ListedAuditResponse {
        items,
        page: listed.page,
        per_page: listed.per_page,
        total_items: listed.total_items,
        total_pages,
    }
}

pub async fn system_list(
    auth: AdminAuth,
    State(state): State<AppState>,
    Query(q): Query<AuditListQuery>,
) -> Result<Json<ListedAuditResponse>, ApiError> {
    auth.require_master()?;
    let listed = list_paginated(state.system.pool(), build_query(q)).await?;
    Ok(Json(into_response(listed)))
}

pub async fn workspace_list(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(workspace): Path<String>,
    Query(q): Query<AuditListQuery>,
) -> Result<Json<ListedAuditResponse>, ApiError> {
    auth.require_workspace_access(&workspace)?;
    find_workspace(state.system.pool(), &workspace)
        .await?
        .ok_or(ApiError::Core(CoreError::WorkspaceNotFound(
            workspace.clone(),
        )))?;
    let pool = state
        .workspaces
        .pool_for(&WorkspaceId::from(workspace))
        .await?;
    let listed = list_paginated(&pool, build_query(q)).await?;
    Ok(Json(into_response(listed)))
}

pub async fn app_list(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((workspace, app)): Path<(String, String)>,
    Query(q): Query<AuditListQuery>,
) -> Result<Json<ListedAuditResponse>, ApiError> {
    auth.require_app_access(&workspace, &app)?;
    find_workspace(state.system.pool(), &workspace)
        .await?
        .ok_or(ApiError::Core(CoreError::WorkspaceNotFound(
            workspace.clone(),
        )))?;
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
    let app_pool = state
        .apps
        .pool_for(&WorkspaceId::from(workspace), &AppId::from(app))
        .await?;
    let listed = list_paginated(&app_pool, build_query(q)).await?;
    Ok(Json(into_response(listed)))
}
