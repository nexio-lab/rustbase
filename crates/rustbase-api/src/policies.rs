//! Hierarchical policy endpoints.
//!
//! Three scopes, same shape:
//!
//! - `GET    /api/system/policies`                              master scope
//! - `GET    /api/system/policies/:field`
//! - `PUT    /api/system/policies/:field`                       master only;
//!     triggers an auto-clamp cascade down to every realm + every app
//!     whose stored value would violate the new bound.
//! - `DELETE /api/system/policies/:field`
//!
//! - `GET    /api/realms/:realm/policies`                       realm scope;
//!     master OR realm-admin
//! - `PUT    /api/realms/:realm/policies/:field`                validated
//!     against the master bound (if any), then cascades to apps.
//! - `DELETE /api/realms/:realm/policies/:field`
//!
//! - `GET    /api/realms/:realm/apps/:app/policies`             app scope
//! - `PUT    /api/realms/:realm/apps/:app/policies/:field`      validated
//!     against the realm bound (if any).
//! - `DELETE /api/realms/:realm/apps/:app/policies/:field`

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rustbase_core::{AppId, CoreError, PolicySpec, RealmId};
use rustbase_db::{
    apps::find_app,
    audit, policies, policy_engine, realms::find_realm,
};
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
        state.realms.clone(),
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
// realm scope
// ============================================================

pub async fn realm_list(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(realm): Path<String>,
) -> Result<Json<Vec<PolicyResponse>>, ApiError> {
    auth.require_realm_access(&realm)?;
    require_realm_exists(&state, &realm).await?;
    let realm_pool = state.realms.pool_for(&RealmId::from(realm)).await?;
    let rows = policies::list_policies(&realm_pool).await?;
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

pub async fn realm_get(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, field)): Path<(String, String)>,
) -> Result<Json<PolicySpec>, ApiError> {
    auth.require_realm_access(&realm)?;
    require_realm_exists(&state, &realm).await?;
    let realm_pool = state.realms.pool_for(&RealmId::from(realm)).await?;
    let spec = policies::get_policy(&realm_pool, &field)
        .await?
        .ok_or(ApiError::Core(CoreError::NotFound {
            collection: "policy".into(),
            id: field,
        }))?;
    Ok(Json(spec))
}

pub async fn realm_put(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, field)): Path<(String, String)>,
    Json(spec): Json<PolicySpec>,
) -> Result<Json<PutPolicyResponse>, ApiError> {
    auth.require_realm_access(&realm)?;
    require_realm_exists(&state, &realm).await?;

    // Validate against master bound, if any.
    if let Some(master_spec) = policies::get_policy(state.system.pool(), &field).await? {
        master_spec
            .validate(&field, &spec)
            .map_err(ApiError::Core)?;
    }

    let realm_pool = state.realms.pool_for(&RealmId::from(realm.clone())).await?;
    policies::upsert_policy(&realm_pool, &field, &spec).await?;
    audit::append(
        &realm_pool,
        Some(&auth.admin_id),
        "policy_set",
        Some(&field),
        &json!({"scope":"realm","spec":spec}),
    )
    .await?;

    let cascaded = policy_engine::cascade_realm_to_apps(
        &realm_pool,
        state.apps.clone(),
        &realm,
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

pub async fn realm_delete(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, field)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    auth.require_realm_access(&realm)?;
    require_realm_exists(&state, &realm).await?;
    let realm_pool = state.realms.pool_for(&RealmId::from(realm.clone())).await?;
    policies::delete_policy(&realm_pool, &field)
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
        &realm_pool,
        Some(&auth.admin_id),
        "policy_deleted",
        Some(&field),
        &json!({"scope":"realm"}),
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
    Path((realm, app)): Path<(String, String)>,
) -> Result<Json<Vec<PolicyResponse>>, ApiError> {
    auth.require_app_access(&realm, &app)?;
    let app_pool = open_app_pool(&state, &realm, &app).await?;
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
    Path((realm, app, field)): Path<(String, String, String)>,
) -> Result<Json<PolicySpec>, ApiError> {
    auth.require_app_access(&realm, &app)?;
    let app_pool = open_app_pool(&state, &realm, &app).await?;
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
    Path((realm, app, field)): Path<(String, String, String)>,
    Json(spec): Json<PolicySpec>,
) -> Result<Json<PutPolicyResponse>, ApiError> {
    auth.require_app_access(&realm, &app)?;
    let realm_pool = state.realms.pool_for(&RealmId::from(realm.clone())).await?;
    find_app(&realm_pool, &app)
        .await?
        .ok_or(ApiError::Core(CoreError::AppNotFound {
            realm: realm.clone(),
            app: app.clone(),
        }))?;

    // Validate against realm bound first (if set); the realm bound is
    // already inside master's, so a single check suffices.
    if let Some(realm_spec) = policies::get_policy(&realm_pool, &field).await? {
        realm_spec.validate(&field, &spec).map_err(ApiError::Core)?;
    } else if let Some(master_spec) = policies::get_policy(state.system.pool(), &field).await? {
        master_spec.validate(&field, &spec).map_err(ApiError::Core)?;
    }

    let app_pool = state
        .apps
        .pool_for(&RealmId::from(realm.clone()), &AppId::from(app.clone()))
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
    Path((realm, app, field)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    auth.require_app_access(&realm, &app)?;
    let app_pool = open_app_pool(&state, &realm, &app).await?;
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

async fn require_realm_exists(state: &AppState, realm: &str) -> Result<(), ApiError> {
    find_realm(state.system.pool(), realm)
        .await?
        .ok_or(ApiError::Core(CoreError::RealmNotFound(realm.to_string())))?;
    Ok(())
}

async fn open_app_pool(
    state: &AppState,
    realm: &str,
    app: &str,
) -> Result<sqlx::SqlitePool, ApiError> {
    require_realm_exists(state, realm).await?;
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
