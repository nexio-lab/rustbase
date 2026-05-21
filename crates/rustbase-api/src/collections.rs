//! Endpoints for managing collections inside an app.
//!
//! - `GET    /api/realms/:realm/apps/:app/collections`         list collections
//! - `POST   /api/realms/:realm/apps/:app/collections`         create a collection
//! - `GET    /api/realms/:realm/apps/:app/collections/:name`   fetch one
//! - `DELETE /api/realms/:realm/apps/:app/collections/:name`   drop the table + meta row
//!
//! All four accept a master, a realm-admin-of-:realm, or an
//! app-admin-of-:realm/:app.
//!
//! Schema evolution (renaming, adding/removing fields, changing types)
//! is deferred to a later feature.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rustbase_core::{AppId, CoreError, RealmId, Schema};
use rustbase_db::{
    Collection,
    apps::find_app,
    collections::{create_collection, delete_collection, find_collection, list_collections},
    realms::find_realm,
};
use serde::Deserialize;

use crate::auth::AdminAuth;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateCollectionRequest {
    pub schema: Schema,
}

pub async fn list(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, app)): Path<(String, String)>,
) -> Result<Json<Vec<Collection>>, ApiError> {
    auth.require_app_access(&realm, &app)?;
    let app_pool = open_app_pool(&state, &realm, &app).await?;
    Ok(Json(list_collections(&app_pool).await?))
}

pub async fn create(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, app)): Path<(String, String)>,
    Json(req): Json<CreateCollectionRequest>,
) -> Result<(StatusCode, Json<Collection>), ApiError> {
    auth.require_app_access(&realm, &app)?;
    let app_pool = open_app_pool(&state, &realm, &app).await?;

    if find_collection(&app_pool, req.schema.id.as_str())
        .await?
        .is_some()
    {
        return Err(ApiError::Core(CoreError::Conflict(format!(
            "collection '{}' already exists",
            req.schema.id
        ))));
    }
    let coll = create_collection(&app_pool, &req.schema).await?;
    tracing::info!(realm = %realm, app = %app, collection = %coll.id, "collection created");
    Ok((StatusCode::CREATED, Json(coll)))
}

pub async fn get(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, app, name)): Path<(String, String, String)>,
) -> Result<Json<Collection>, ApiError> {
    auth.require_app_access(&realm, &app)?;
    let app_pool = open_app_pool(&state, &realm, &app).await?;
    let coll = find_collection(&app_pool, &name)
        .await?
        .ok_or(ApiError::Core(CoreError::NotFound {
            collection: name,
            id: String::new(),
        }))?;
    Ok(Json(coll))
}

pub async fn delete(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, app, name)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    auth.require_app_access(&realm, &app)?;
    let app_pool = open_app_pool(&state, &realm, &app).await?;

    delete_collection(&app_pool, &name)
        .await
        .map_err(|e| match e {
            rustbase_db::DbError::Sqlx(sqlx::Error::RowNotFound) => {
                ApiError::Core(CoreError::NotFound {
                    collection: name.clone(),
                    id: String::new(),
                })
            }
            other => ApiError::from(other),
        })?;

    tracing::info!(realm = %realm, app = %app, collection = %name, "collection deleted");
    Ok(StatusCode::NO_CONTENT)
}

/// Verify the realm + app exist in their respective DBs, then return
/// the (cached) app pool.
async fn open_app_pool(
    state: &AppState,
    realm: &str,
    app: &str,
) -> Result<sqlx::SqlitePool, ApiError> {
    find_realm(state.system.pool(), realm)
        .await?
        .ok_or(ApiError::Core(CoreError::RealmNotFound(realm.to_string())))?;

    let realm_id = RealmId::from(realm.to_string());
    let realm_pool = state.realms.pool_for(&realm_id).await?;
    find_app(&realm_pool, app).await?.ok_or_else(|| {
        ApiError::Core(CoreError::AppNotFound {
            realm: realm.to_string(),
            app: app.to_string(),
        })
    })?;

    let app_id = AppId::from(app.to_string());
    Ok(state.apps.pool_for(&realm_id, &app_id).await?)
}
