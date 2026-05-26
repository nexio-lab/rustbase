//! Endpoints for managing apps under a realm.
//!
//! - `GET    /api/realms/:realm/apps`         list apps in a realm
//! - `POST   /api/realms/:realm/apps`         create an app + init its data.db
//! - `GET    /api/realms/:realm/apps/:app`    fetch one
//! - `PATCH  /api/realms/:realm/apps/:app`    rename
//! - `DELETE /api/realms/:realm/apps/:app`    cascade-delete the app
//!
//! All five accept either a master admin or a realm admin for the
//! target realm. App admins (single-app scope) are deliberately
//! excluded — they manage their own app's data, not the app's identity.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rustbase_core::{AppId, CoreError, RealmId};
use rustbase_db::{
    APP_MIGRATIONS, App, apply_migrations,
    apps::{create_app, delete_app, find_app, list_apps, rename_app},
    paths,
    realms::find_realm,
};
use serde::Deserialize;
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
    Path(realm): Path<String>,
) -> Result<Json<Vec<App>>, ApiError> {
    auth.require_realm_access(&realm)?;
    require_realm_exists(&state, &realm).await?;
    let pool = state.realms.pool_for(&RealmId::from(realm)).await?;
    Ok(Json(list_apps(&pool).await?))
}

pub async fn create(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(realm): Path<String>,
    Json(req): Json<CreateAppRequest>,
) -> Result<(StatusCode, Json<App>), ApiError> {
    auth.require_realm_access(&realm)?;
    validate_app_id(&req.id)?;
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;

    require_realm_exists(&state, &realm).await?;
    let realm_id = RealmId::from(realm.clone());
    let realm_pool = state.realms.pool_for(&realm_id).await?;

    if find_app(&realm_pool, &req.id).await?.is_some() {
        return Err(ApiError::Core(CoreError::Conflict(format!(
            "app '{}' already exists in realm '{}'",
            req.id, realm
        ))));
    }

    let app = create_app(&realm_pool, &req.id, &req.name).await?;

    // Initialize the app's data.db.
    let app_id = AppId::from(req.id.clone());
    let app_pool = state.apps.pool_for(&realm_id, &app_id).await?;
    apply_migrations(app_pool, APP_MIGRATIONS).await?;

    // Pick up any JS hooks dropped on disk before the app was created.
    let hooks_dir = state
        .data_dir
        .join("hooks")
        .join(&realm)
        .join(&req.id);
    if let Err(e) = state.hooks.load_app(&realm, &req.id, &hooks_dir).await {
        tracing::warn!(realm = %realm, app = %req.id, error = %e, "loading hooks failed");
    }

    tracing::info!(realm = %realm, app = %req.id, "app created");
    Ok((StatusCode::CREATED, Json(app)))
}

pub async fn get(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, app)): Path<(String, String)>,
) -> Result<Json<App>, ApiError> {
    auth.require_realm_access(&realm)?;
    require_realm_exists(&state, &realm).await?;
    let pool = state.realms.pool_for(&RealmId::from(realm.clone())).await?;
    let row = find_app(&pool, &app)
        .await?
        .ok_or(ApiError::Core(CoreError::AppNotFound { realm, app }))?;
    Ok(Json(row))
}

pub async fn update(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, app)): Path<(String, String)>,
    Json(req): Json<UpdateAppRequest>,
) -> Result<Json<App>, ApiError> {
    auth.require_realm_access(&realm)?;
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;
    require_realm_exists(&state, &realm).await?;

    let pool = state.realms.pool_for(&RealmId::from(realm.clone())).await?;
    rename_app(&pool, &app, &req.name).await.map_err(|e| match e {
        rustbase_db::DbError::Sqlx(sqlx::Error::RowNotFound) => {
            ApiError::Core(CoreError::AppNotFound {
                realm: realm.clone(),
                app: app.clone(),
            })
        }
        other => ApiError::from(other),
    })?;

    let row = find_app(&pool, &app)
        .await?
        .ok_or(ApiError::Core(CoreError::AppNotFound { realm, app }))?;
    Ok(Json(row))
}

pub async fn delete(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, app)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    auth.require_realm_access(&realm)?;
    require_realm_exists(&state, &realm).await?;

    let realm_id = RealmId::from(realm.clone());
    let app_id = AppId::from(app.clone());
    let realm_pool = state.realms.pool_for(&realm_id).await?;

    find_app(&realm_pool, &app)
        .await?
        .ok_or_else(|| {
            ApiError::Core(CoreError::AppNotFound {
                realm: realm.clone(),
                app: app.clone(),
            })
        })?;

    state.apps.evict(&realm_id, &app_id);
    delete_app(&realm_pool, &app).await?;

    let dir = paths::app_dir(state.data_dir.as_ref(), &realm_id, &app_id);
    if dir.exists() {
        tokio::fs::remove_dir_all(&dir).await.map_err(|e| {
            ApiError::Core(CoreError::Internal(format!(
                "failed to remove app folder: {e}"
            )))
        })?;
    }

    tracing::info!(realm = %realm, app = %app, "app deleted");
    Ok(StatusCode::NO_CONTENT)
}

async fn require_realm_exists(state: &AppState, realm: &str) -> Result<(), ApiError> {
    find_realm(state.system.pool(), realm)
        .await?
        .ok_or(ApiError::Core(CoreError::RealmNotFound(realm.to_string())))?;
    Ok(())
}

/// App ids share the realm-id slug rules.
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
