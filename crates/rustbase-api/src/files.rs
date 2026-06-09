//! File upload/download endpoints.
//!
//! - `POST   /api/workspaces/:workspace/apps/:app/files`
//!   Raw-body upload. Headers `X-Filename` (required) and
//!   `Content-Type` (optional) are stored as metadata.
//! - `GET    /api/workspaces/:workspace/apps/:app/files`            list metadata
//! - `GET    /api/workspaces/:workspace/apps/:app/files/:id`        download bytes
//! - `GET    /api/workspaces/:workspace/apps/:app/files/:id/meta`   just the row
//! - `DELETE /api/workspaces/:workspace/apps/:app/files/:id`
//!
//! All five require app-level admin access. Wiring file uploads to
//! end-user tokens + per-app access rules is a follow-up.

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use rustbase_core::{AppId, CoreError, WorkspaceId};
use rustbase_db::{
    FileMeta,
    apps::find_app,
    files::{delete_file, find_file, insert_file, list_files},
    workspaces::find_workspace,
};

use crate::auth::AdminAuth;
use crate::error::ApiError;
use crate::state::AppState;

/// Cap accepted upload size to keep us safe until a policy field
/// surfaces this. 25 MiB is generous enough for typical avatar /
/// attachment workloads.
const MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024;

pub async fn upload(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((workspace, app)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<FileMeta>), ApiError> {
    auth.require_app_access(&workspace, &app)?;
    let app_pool = open_app_pool(&state, &workspace, &app).await?;

    if body.len() > MAX_UPLOAD_BYTES {
        return Err(ApiError::Core(CoreError::Validation(format!(
            "upload exceeds {MAX_UPLOAD_BYTES} bytes"
        ))));
    }

    let filename = headers
        .get("x-filename")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .ok_or(ApiError::Core(CoreError::Validation(
            "X-Filename header is required".into(),
        )))?
        .to_string();
    let mime = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let size = body.len();
    let meta = insert_file(&app_pool, &filename, mime.as_deref(), size as i64).await?;
    state
        .storage
        .put(&storage_key(&workspace, &app, &meta.id), body.to_vec())
        .await
        .map_err(|e| ApiError::Core(CoreError::Internal(format!("storage put: {e}"))))?;

    metrics::counter!("rustbase_file_uploads_total").increment(1);
    metrics::counter!("rustbase_file_upload_bytes_total").increment(size as u64);

    tracing::info!(
        workspace = %workspace, app = %app, file = %meta.id, size,
        "file uploaded"
    );
    Ok((StatusCode::CREATED, Json(meta)))
}

pub async fn list(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((workspace, app)): Path<(String, String)>,
) -> Result<Json<Vec<FileMeta>>, ApiError> {
    auth.require_app_access(&workspace, &app)?;
    let app_pool = open_app_pool(&state, &workspace, &app).await?;
    Ok(Json(list_files(&app_pool).await?))
}

pub async fn download(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((workspace, app, id)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    auth.require_app_access(&workspace, &app)?;
    let app_pool = open_app_pool(&state, &workspace, &app).await?;

    let meta = find_file(&app_pool, &id)
        .await?
        .ok_or(ApiError::Core(CoreError::NotFound {
            collection: "file".into(),
            id: id.clone(),
        }))?;
    let bytes = state
        .storage
        .get(&storage_key(&workspace, &app, &id))
        .await
        .map_err(|e| ApiError::Core(CoreError::Internal(format!("storage get: {e}"))))?;

    let mut resp = bytes.into_response();
    if let Some(mime) = &meta.mime {
        let v = mime
            .parse()
            .unwrap_or_else(|_| header::HeaderValue::from_static("application/octet-stream"));
        resp.headers_mut().insert(header::CONTENT_TYPE, v);
    }
    let filename = meta
        .filename
        .parse()
        .unwrap_or_else(|_| header::HeaderValue::from_static("file"));
    resp.headers_mut().insert("x-filename", filename);
    Ok(resp)
}

pub async fn meta(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((workspace, app, id)): Path<(String, String, String)>,
) -> Result<Json<FileMeta>, ApiError> {
    auth.require_app_access(&workspace, &app)?;
    let app_pool = open_app_pool(&state, &workspace, &app).await?;
    let meta = find_file(&app_pool, &id)
        .await?
        .ok_or(ApiError::Core(CoreError::NotFound {
            collection: "file".into(),
            id,
        }))?;
    Ok(Json(meta))
}

pub async fn delete(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((workspace, app, id)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    auth.require_app_access(&workspace, &app)?;
    let app_pool = open_app_pool(&state, &workspace, &app).await?;

    delete_file(&app_pool, &id).await.map_err(|e| match e {
        rustbase_db::DbError::Sqlx(sqlx::Error::RowNotFound) => {
            ApiError::Core(CoreError::NotFound {
                collection: "file".into(),
                id: id.clone(),
            })
        }
        other => ApiError::from(other),
    })?;
    // Best-effort delete on the object store. If the row was deleted
    // but the file is gone (or never existed), don't surface that as a
    // 500 — the row is the source of truth for "does this file exist".
    let _ = state
        .storage
        .delete(&storage_key(&workspace, &app, &id))
        .await;
    Ok(StatusCode::NO_CONTENT)
}

/// Compose the per-(workspace, app, file) key used by the global Storage
/// backend. With LocalStorage rooted at `data_dir` this resolves to
/// the same on-disk path as the previous per-app `Storage::local()`
/// layout (`data_dir/workspaces/<r>/apps/<a>/storage/<id>`), so existing
/// data carries over transparently. With S3 it becomes an in-bucket
/// key with the same prefix.
fn storage_key(workspace: &str, app: &str, file_id: &str) -> String {
    format!("workspaces/{workspace}/apps/{app}/storage/{file_id}")
}

async fn open_app_pool(
    state: &AppState,
    workspace: &str,
    app: &str,
) -> Result<sqlx::SqlitePool, ApiError> {
    find_workspace(state.system.pool(), workspace)
        .await?
        .ok_or(ApiError::Core(CoreError::WorkspaceNotFound(
            workspace.to_string(),
        )))?;
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
