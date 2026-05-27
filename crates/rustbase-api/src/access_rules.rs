//! HTTP CRUD for per-collection access rules.
//!
//! - `GET /api/realms/:realm/apps/:app/collections/:coll/access_rules`
//! - `PUT /api/realms/:realm/apps/:app/collections/:coll/access_rules/:action`
//! - `DELETE …/access_rules/:action`
//!
//! All three require app-level admin access (master, realm-admin for
//! :realm, or app-admin for :realm/:app).

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rustbase_core::{AppId, CoreError, RealmId};
use rustbase_db::{
    access_rules::{AccessAction, AccessRule, get_rule, list_rules, set_rule},
    apps::find_app,
    collections::find_collection,
    realms::find_realm,
};
use serde::Deserialize;

use crate::auth::AdminAuth;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct SetRuleRequest {
    /// `None` (or omitted) locks the action to admins only.
    /// Empty string / `"true"` opens the action to any authenticated
    /// user of the realm. Other filter strings are stored but treated
    /// as deny until the substitution-aware evaluator lands.
    pub filter: Option<String>,
}

pub async fn list(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, app, coll)): Path<(String, String, String)>,
) -> Result<Json<Vec<AccessRule>>, ApiError> {
    auth.require_app_access(&realm, &app)?;
    let app_pool = open_app_and_check(&state, &realm, &app, &coll).await?;
    Ok(Json(list_rules(&app_pool, &coll).await?))
}

pub async fn put(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, app, coll, action)): Path<(String, String, String, String)>,
    Json(req): Json<SetRuleRequest>,
) -> Result<Json<AccessRule>, ApiError> {
    auth.require_app_access(&realm, &app)?;
    let action = AccessAction::from_str(&action).ok_or(ApiError::Core(CoreError::Validation(
        format!("unknown access action: {action}"),
    )))?;
    let app_pool = open_app_and_check(&state, &realm, &app, &coll).await?;
    set_rule(&app_pool, &coll, action, req.filter.as_deref()).await?;
    let _ = get_rule(&app_pool, &coll, action).await?;
    tracing::info!(realm = %realm, app = %app, collection = %coll, action = action.as_str(), "access rule updated");
    Ok(Json(AccessRule {
        collection: coll,
        action,
        filter: req.filter,
    }))
}

pub async fn delete(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, app, coll, action)): Path<(String, String, String, String)>,
) -> Result<StatusCode, ApiError> {
    auth.require_app_access(&realm, &app)?;
    let action = AccessAction::from_str(&action).ok_or(ApiError::Core(CoreError::Validation(
        format!("unknown access action: {action}"),
    )))?;
    let app_pool = open_app_and_check(&state, &realm, &app, &coll).await?;
    let res = sqlx::query("DELETE FROM _access_rules WHERE collection_id = ? AND action = ?")
        .bind(&coll)
        .bind(action.as_str())
        .execute(&app_pool)
        .await
        .map_err(rustbase_db::DbError::Sqlx)?;
    if res.rows_affected() == 0 {
        return Err(ApiError::Core(CoreError::NotFound {
            collection: coll,
            id: action.as_str().to_string(),
        }));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn open_app_and_check(
    state: &AppState,
    realm: &str,
    app: &str,
    coll: &str,
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
    let app_pool = state.apps.pool_for(&realm_id, &app_id).await?;
    find_collection(&app_pool, coll).await?.ok_or_else(|| {
        ApiError::Core(CoreError::NotFound {
            collection: coll.to_string(),
            id: String::new(),
        })
    })?;
    Ok(app_pool)
}
