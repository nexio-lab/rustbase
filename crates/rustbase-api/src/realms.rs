//! Master-admin endpoints for managing realms.
//!
//! - `GET    /api/realms`        — list every realm
//! - `POST   /api/realms`        — create a new realm (id + display name)
//! - `GET    /api/realms/:id`    — fetch one
//! - `PATCH  /api/realms/:id`    — rename
//! - `DELETE /api/realms/:id`    — cascade-delete (refuses master)
//!
//! All five require a master-admin token. Creation also initializes
//! the realm's `realm.db` by opening the pool and running
//! `REALM_MIGRATIONS`. Deletion evicts the realm + every app pool under
//! it, deletes the row, and removes the realm's folder.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rustbase_core::{CoreError, MASTER_REALM_ID, RealmId};
use rustbase_db::{
    REALM_MIGRATIONS, Realm, apply_migrations, paths,
    realms::{create_realm, delete_realm, find_realm, list_realms, rename_realm},
};
use serde::Deserialize;
use validator::Validate;

use crate::auth::AdminAuth;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize, Validate)]
pub struct CreateRealmRequest {
    pub id: String,
    #[validate(length(min = 1, max = 100))]
    pub name: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct UpdateRealmRequest {
    #[validate(length(min = 1, max = 100))]
    pub name: String,
}

pub async fn list(
    auth: AdminAuth,
    State(state): State<AppState>,
) -> Result<Json<Vec<Realm>>, ApiError> {
    auth.require_master()?;
    Ok(Json(list_realms(state.system.pool()).await?))
}

pub async fn create(
    auth: AdminAuth,
    State(state): State<AppState>,
    Json(req): Json<CreateRealmRequest>,
) -> Result<(StatusCode, Json<Realm>), ApiError> {
    auth.require_master()?;
    validate_realm_id(&req.id)?;
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;

    if req.id == MASTER_REALM_ID {
        return Err(ApiError::Core(CoreError::Conflict(
            "realm id 'master' is reserved".into(),
        )));
    }
    if find_realm(state.system.pool(), &req.id).await?.is_some() {
        return Err(ApiError::Core(CoreError::Conflict(format!(
            "realm '{}' already exists",
            req.id
        ))));
    }

    let realm = create_realm(state.system.pool(), &req.id, &req.name).await?;

    let realm_id = RealmId::from(req.id.clone());
    let realm_pool = state.realms.pool_for(&realm_id).await?;
    apply_migrations(realm_pool, REALM_MIGRATIONS).await?;

    tracing::info!(realm = %req.id, "created realm");
    Ok((StatusCode::CREATED, Json(realm)))
}

pub async fn get(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Realm>, ApiError> {
    auth.require_master()?;
    let realm = find_realm(state.system.pool(), &id)
        .await?
        .ok_or(ApiError::Core(CoreError::RealmNotFound(id)))?;
    Ok(Json(realm))
}

pub async fn update(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateRealmRequest>,
) -> Result<Json<Realm>, ApiError> {
    auth.require_master()?;
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;

    rename_realm(state.system.pool(), &id, &req.name)
        .await
        .map_err(|e| match e {
            rustbase_db::DbError::Sqlx(sqlx::Error::RowNotFound) => {
                ApiError::Core(CoreError::RealmNotFound(id.clone()))
            }
            other => ApiError::from(other),
        })?;

    let realm = find_realm(state.system.pool(), &id)
        .await?
        .ok_or(ApiError::Core(CoreError::RealmNotFound(id)))?;
    Ok(Json(realm))
}

pub async fn delete(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    auth.require_master()?;
    if id == MASTER_REALM_ID {
        return Err(ApiError::Core(CoreError::Forbidden));
    }

    let realm = find_realm(state.system.pool(), &id)
        .await?
        .ok_or(ApiError::Core(CoreError::RealmNotFound(id.clone())))?;
    if realm.is_master {
        return Err(ApiError::Core(CoreError::Forbidden));
    }

    let realm_id = RealmId::from(id.clone());
    state.realms.evict(&realm_id);
    state.apps.evict_realm(&realm_id);

    delete_realm(state.system.pool(), &id).await?;

    let dir = paths::realm_dir(state.data_dir.as_ref(), &realm_id);
    if dir.exists() {
        tokio::fs::remove_dir_all(&dir).await.map_err(|e| {
            ApiError::Core(CoreError::Internal(format!(
                "failed to remove realm folder: {e}"
            )))
        })?;
    }

    tracing::info!(realm = %id, "deleted realm");
    Ok(StatusCode::NO_CONTENT)
}

/// Realm ids are slugs: 2–50 chars, `[a-z0-9-]`, no leading/trailing dash.
fn validate_realm_id(id: &str) -> Result<(), ApiError> {
    let len = id.len();
    if !(2..=50).contains(&len) {
        return Err(ApiError::Core(CoreError::Validation(
            "realm id must be 2-50 characters".into(),
        )));
    }
    if id.starts_with('-') || id.ends_with('-') {
        return Err(ApiError::Core(CoreError::Validation(
            "realm id must not start or end with '-'".into(),
        )));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(ApiError::Core(CoreError::Validation(
            "realm id may only contain lowercase letters, digits, and '-'".into(),
        )));
    }
    Ok(())
}
