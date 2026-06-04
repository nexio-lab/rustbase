use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use rustbase_auth::{TokenRole, build_claims};
use rustbase_core::{CoreError, WorkspaceId};
use rustbase_db::tokens::{
    SubjectKind, find_active_refresh_token, insert_refresh_token, revoke_refresh_token,
};
use serde::{Deserialize, Serialize};

use super::cookies::{
    CookieFlags, REFRESH_COOKIE, build_access_cookie, build_refresh_cookie, read_cookie,
};
use super::{default_access_ttl, default_refresh_ttl, new_refresh_token, require_workspace_exists};
use crate::error::ApiError;
use crate::state::AppState;

/// Pick the refresh token from the JSON body, falling back to the
/// `rb_rt` cookie. The cookie path scopes it to `/_/auth`, so we
/// don't have to disambiguate by call site.
fn pick_refresh_token(headers: &HeaderMap, body: &RefreshRequest) -> Option<String> {
    if let Some(tok) = body.refresh_token.as_ref()
        && !tok.is_empty()
    {
        return Some(tok.clone());
    }
    read_cookie(headers, REFRESH_COOKIE)
}

fn with_session_cookies(state: &AppState, body: RefreshResponse) -> Response {
    let flags = CookieFlags {
        secure: state.cookie_secure,
    };
    let access_cookie = build_access_cookie(
        &body.access_token,
        default_access_ttl().num_seconds(),
        flags,
    );
    let refresh_cookie = build_refresh_cookie(
        &body.refresh_token,
        default_refresh_ttl().num_seconds(),
        flags,
    );
    let mut resp = Json(body).into_response();
    let headers = resp.headers_mut();
    headers.append(axum::http::header::SET_COOKIE, access_cookie);
    headers.append(axum::http::header::SET_COOKIE, refresh_cookie);
    resp
}

#[derive(Debug, Deserialize, Default)]
pub struct RefreshRequest {
    /// Optional — when omitted (e.g. dashboard cookie session), the
    /// handler reads the `rb_rt` cookie instead. Validation in the
    /// handler returns `Unauthorized` if neither source carries a
    /// value.
    #[serde(default)]
    pub refresh_token: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RefreshResponse {
    pub access_token: String,
    pub refresh_token: String,
}

/// Rotate-on-use: each refresh revokes the presented token and issues a
/// fresh one. Reusing a refresh that's already been redeemed fails.
pub async fn master_admin_refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Option<Json<RefreshRequest>>,
) -> Result<Response, ApiError> {
    let req = body.map(|j| j.0).unwrap_or_default();
    let presented =
        pick_refresh_token(&headers, &req).ok_or(ApiError::Core(CoreError::Unauthorized))?;

    let existing =
        find_active_refresh_token(state.system.pool(), &presented, SubjectKind::MasterAdmin)
            .await?
            .ok_or(ApiError::Core(CoreError::Unauthorized))?;

    revoke_refresh_token(state.system.pool(), &existing.token).await?;

    let new_refresh = insert_refresh_token(
        state.system.pool(),
        &new_refresh_token(),
        SubjectKind::MasterAdmin,
        &existing.subject_id,
        default_refresh_ttl(),
    )
    .await?;

    let claims = build_claims(
        existing.subject_id,
        TokenRole::MasterAdmin,
        None,
        None,
        default_access_ttl(),
    );
    let access_token = state.jwt.issue(&claims)?;

    let body = RefreshResponse {
        access_token,
        refresh_token: new_refresh.token,
    };
    Ok(with_session_cookies(&state, body))
}

pub async fn user_refresh(
    State(state): State<AppState>,
    Path(workspace): Path<String>,
    headers: HeaderMap,
    body: Option<Json<RefreshRequest>>,
) -> Result<Response, ApiError> {
    require_workspace_exists(&state, &workspace).await?;
    let req = body.map(|j| j.0).unwrap_or_default();
    let presented =
        pick_refresh_token(&headers, &req).ok_or(ApiError::Core(CoreError::Unauthorized))?;

    let workspace_id = WorkspaceId::from(workspace.clone());
    let pool = state.workspaces.pool_for(&workspace_id).await?;

    let existing = find_active_refresh_token(&pool, &presented, SubjectKind::User)
        .await?
        .ok_or(ApiError::Core(CoreError::Unauthorized))?;

    revoke_refresh_token(&pool, &existing.token).await?;

    let new_refresh = insert_refresh_token(
        &pool,
        &new_refresh_token(),
        SubjectKind::User,
        &existing.subject_id,
        default_refresh_ttl(),
    )
    .await?;

    let claims = build_claims(
        existing.subject_id,
        TokenRole::User,
        Some(workspace),
        // Workspace-shared identity → user tokens carry no `app`.
        None,
        default_access_ttl(),
    );
    let access_token = state.jwt.issue(&claims)?;

    let body = RefreshResponse {
        access_token,
        refresh_token: new_refresh.token,
    };
    Ok(with_session_cookies(&state, body))
}

pub async fn workspace_admin_refresh(
    State(state): State<AppState>,
    Path(workspace): Path<String>,
    headers: HeaderMap,
    body: Option<Json<RefreshRequest>>,
) -> Result<Response, ApiError> {
    let req = body.map(|j| j.0).unwrap_or_default();
    let presented =
        pick_refresh_token(&headers, &req).ok_or(ApiError::Core(CoreError::Unauthorized))?;
    let workspace_id = WorkspaceId::from(workspace.clone());
    let pool = state.workspaces.pool_for(&workspace_id).await?;

    let existing = find_active_refresh_token(&pool, &presented, SubjectKind::WorkspaceAdmin)
        .await?
        .ok_or(ApiError::Core(CoreError::Unauthorized))?;

    revoke_refresh_token(&pool, &existing.token).await?;

    let new_refresh = insert_refresh_token(
        &pool,
        &new_refresh_token(),
        SubjectKind::WorkspaceAdmin,
        &existing.subject_id,
        default_refresh_ttl(),
    )
    .await?;

    let claims = build_claims(
        existing.subject_id,
        TokenRole::WorkspaceAdmin,
        Some(workspace),
        None,
        default_access_ttl(),
    );
    let access_token = state.jwt.issue(&claims)?;

    let body = RefreshResponse {
        access_token,
        refresh_token: new_refresh.token,
    };
    Ok(with_session_cookies(&state, body))
}
