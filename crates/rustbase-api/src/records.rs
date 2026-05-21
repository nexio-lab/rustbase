//! Record CRUD over the per-collection tables.
//!
//! - `POST   /api/realms/:realm/apps/:app/collections/:coll/records`         create
//! - `GET    /api/realms/:realm/apps/:app/collections/:coll/records`         paginated list
//! - `GET    /api/realms/:realm/apps/:app/collections/:coll/records/:id`     fetch one
//! - `PATCH  /api/realms/:realm/apps/:app/collections/:coll/records/:id`     partial update
//! - `DELETE /api/realms/:realm/apps/:app/collections/:coll/records/:id`     delete
//!
//! All five require app-level access (master, realm admin for :realm,
//! or app admin for :realm/:app).
//!
//! Per-collection access rules and filter queries on `list` are
//! coming on later branches; the SQL translator they'll use is already
//! in `rustbase-db::filter_sql`.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use rustbase_core::{AppId, CoreError, FilterNode, RealmId, Record, Schema, parse_filter};
use rustbase_db::{
    DbError, ListPage, ListedRecords,
    apps::find_app,
    collections::find_collection,
    realms::find_realm,
    records::{create_record, delete_record, find_record, list_records, update_record},
};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json_;
use std::collections::BTreeMap;

use crate::auth::AdminAuth;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_per_page")]
    pub per_page: u32,
    #[serde(default)]
    pub filter: Option<String>,
}

fn default_page() -> u32 {
    1
}
fn default_per_page() -> u32 {
    30
}

#[derive(Debug, Serialize)]
pub struct ListResponse {
    pub items: Vec<Record>,
    pub page: u32,
    pub per_page: u32,
    pub total_items: u64,
    pub total_pages: u64,
}

impl From<ListedRecords> for ListResponse {
    fn from(l: ListedRecords) -> Self {
        let per = l.per_page.max(1) as u64;
        let total_pages = (l.total_items + per - 1) / per;
        Self {
            items: l.items,
            page: l.page,
            per_page: l.per_page,
            total_items: l.total_items,
            total_pages,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateRecordRequest {
    #[serde(default, flatten)]
    pub fields: BTreeMap<String, Json_>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRecordRequest {
    #[serde(default, flatten)]
    pub fields: BTreeMap<String, Json_>,
}

pub async fn list(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, app, coll)): Path<(String, String, String)>,
    Query(q): Query<ListQuery>,
) -> Result<Json<ListResponse>, ApiError> {
    auth.require_app_access(&realm, &app)?;
    let (app_pool, schema) = open_app_and_schema(&state, &realm, &app, &coll).await?;

    let filter = match &q.filter {
        Some(s) if !s.trim().is_empty() => {
            let node = parse_filter(s).map_err(ApiError::from)?;
            validate_filter_columns(&node, &schema)?;
            Some(node)
        }
        _ => None,
    };

    let listed = list_records(
        &app_pool,
        &schema,
        ListPage {
            page: q.page,
            per_page: q.per_page,
        },
        filter.as_ref(),
    )
    .await?;
    Ok(Json(listed.into()))
}

/// Walk the filter AST, asserting every referenced column is either a
/// declared schema field or one of the implicit `id`/`created_at`/
/// `updated_at`. Catches typos up front so callers see a precise 400
/// instead of a SQLite "no such column" surfaced as a 500.
fn validate_filter_columns(node: &FilterNode, schema: &Schema) -> Result<(), ApiError> {
    let mut known: std::collections::HashSet<&str> =
        ["id", "created_at", "updated_at"].into_iter().collect();
    for f in &schema.fields {
        known.insert(&f.name);
    }
    walk(node, &known)
}

fn walk(node: &FilterNode, known: &std::collections::HashSet<&str>) -> Result<(), ApiError> {
    match node {
        FilterNode::And(l, r) | FilterNode::Or(l, r) => {
            walk(l, known)?;
            walk(r, known)?;
        }
        FilterNode::Not(inner) => walk(inner, known)?,
        FilterNode::Eq(f, _)
        | FilterNode::Ne(f, _)
        | FilterNode::Gt(f, _)
        | FilterNode::Gte(f, _)
        | FilterNode::Lt(f, _)
        | FilterNode::Lte(f, _)
        | FilterNode::Like(f, _) => check_field(f, known)?,
        FilterNode::In(f, _) => check_field(f, known)?,
    }
    Ok(())
}

fn check_field(name: &str, known: &std::collections::HashSet<&str>) -> Result<(), ApiError> {
    if known.contains(name) {
        Ok(())
    } else {
        Err(ApiError::Core(CoreError::Validation(format!(
            "unknown field in filter: {name}"
        ))))
    }
}

pub async fn create(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, app, coll)): Path<(String, String, String)>,
    Json(req): Json<CreateRecordRequest>,
) -> Result<(StatusCode, Json<Record>), ApiError> {
    auth.require_app_access(&realm, &app)?;
    let (app_pool, schema) = open_app_and_schema(&state, &realm, &app, &coll).await?;
    let rec = create_record(&app_pool, &schema, req.fields).await?;
    Ok((StatusCode::CREATED, Json(rec)))
}

pub async fn get(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, app, coll, id)): Path<(String, String, String, String)>,
) -> Result<Json<Record>, ApiError> {
    auth.require_app_access(&realm, &app)?;
    let (app_pool, schema) = open_app_and_schema(&state, &realm, &app, &coll).await?;
    let rec = find_record(&app_pool, &schema, &id)
        .await?
        .ok_or(ApiError::Core(CoreError::NotFound {
            collection: coll,
            id,
        }))?;
    Ok(Json(rec))
}

pub async fn update(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, app, coll, id)): Path<(String, String, String, String)>,
    Json(req): Json<UpdateRecordRequest>,
) -> Result<Json<Record>, ApiError> {
    auth.require_app_access(&realm, &app)?;
    let (app_pool, schema) = open_app_and_schema(&state, &realm, &app, &coll).await?;
    let rec = update_record(&app_pool, &schema, &id, req.fields)
        .await
        .map_err(|e| match e {
            DbError::Sqlx(sqlx::Error::RowNotFound) => ApiError::Core(CoreError::NotFound {
                collection: coll.clone(),
                id: id.clone(),
            }),
            other => ApiError::from(other),
        })?;
    Ok(Json(rec))
}

pub async fn delete(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, app, coll, id)): Path<(String, String, String, String)>,
) -> Result<StatusCode, ApiError> {
    auth.require_app_access(&realm, &app)?;
    let (app_pool, schema) = open_app_and_schema(&state, &realm, &app, &coll).await?;
    delete_record(&app_pool, &schema, &id)
        .await
        .map_err(|e| match e {
            DbError::Sqlx(sqlx::Error::RowNotFound) => ApiError::Core(CoreError::NotFound {
                collection: coll.clone(),
                id: id.clone(),
            }),
            other => ApiError::from(other),
        })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn open_app_and_schema(
    state: &AppState,
    realm: &str,
    app: &str,
    coll: &str,
) -> Result<(sqlx::SqlitePool, rustbase_core::Schema), ApiError> {
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
    let collection = find_collection(&app_pool, coll).await?.ok_or_else(|| {
        ApiError::Core(CoreError::NotFound {
            collection: coll.to_string(),
            id: String::new(),
        })
    })?;
    Ok((app_pool, collection.schema))
}
