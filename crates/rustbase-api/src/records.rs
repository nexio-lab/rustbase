//! Record CRUD over the per-collection tables.
//!
//! - `POST   /api/workspaces/:workspace/apps/:app/collections/:coll/records`         create
//! - `GET    /api/workspaces/:workspace/apps/:app/collections/:coll/records`         paginated list
//! - `GET    /api/workspaces/:workspace/apps/:app/collections/:coll/records/:id`     fetch one
//! - `PATCH  /api/workspaces/:workspace/apps/:app/collections/:coll/records/:id`     partial update
//! - `DELETE /api/workspaces/:workspace/apps/:app/collections/:coll/records/:id`     delete
//!
//! Authorization is per verb:
//!   - Admin tokens (master / workspace-admin / app-admin matching the
//!     path) always pass.
//!   - End-user tokens are scoped to one workspace. They pass only when
//!     the collection's access rule for the verb is set to an "open"
//!     filter (empty string or `true`). Other rule strings are
//!     reserved for the substitution-aware evaluator landing on a
//!     later branch; until then they deny by default.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use rustbase_core::{
    AppId, CoreError, FilterNode, Record, RuleContext, Schema, WorkspaceId, parse_filter,
    rule_template,
};
use rustbase_db::{
    DbError, ListPage, ListedRecords,
    access_rules::{AccessAction, RuleDecision, classify_rule, get_rule},
    apps::find_app,
    collections::find_collection,
    records::{create_record, delete_record, list_records, update_record},
    workspaces::find_workspace,
};
use rustbase_realtime::{RealtimeEvent, SubscriptionKey};
use rustbase_runtime::{HookAuth, HookEvent, HookRequest};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json_;
use std::collections::BTreeMap;

use crate::auth::PrincipalAuth;
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
        let total_pages = l.total_items.div_ceil(per);
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
    auth: PrincipalAuth,
    State(state): State<AppState>,
    Path((workspace, app, coll)): Path<(String, String, String)>,
    Query(q): Query<ListQuery>,
) -> Result<Json<ListResponse>, ApiError> {
    let (app_pool, schema) = open_app_and_schema(&state, &workspace, &app, &coll).await?;
    let rule_filter = authorize_record_action(
        &auth,
        &app_pool,
        &workspace,
        &app,
        &coll,
        AccessAction::List,
        &schema,
    )
    .await?;

    let user_filter = match &q.filter {
        Some(s) if !s.trim().is_empty() => {
            let node = parse_filter(s).map_err(ApiError::from)?;
            validate_filter_columns(&node, &schema)?;
            Some(node)
        }
        _ => None,
    };

    // AND the rule (if any) into the user-supplied filter so SQL does the
    // row-level scoping.
    let combined = match (rule_filter, user_filter) {
        (Some(r), Some(u)) => Some(FilterNode::and(r, u)),
        (Some(r), None) => Some(r),
        (None, uf) => uf,
    };

    let listed = list_records(
        &app_pool,
        &schema,
        ListPage {
            page: q.page,
            per_page: q.per_page,
        },
        combined.as_ref(),
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
    auth: PrincipalAuth,
    State(state): State<AppState>,
    Path((workspace, app, coll)): Path<(String, String, String)>,
    Json(req): Json<CreateRecordRequest>,
) -> Result<(StatusCode, Json<Record>), ApiError> {
    let (app_pool, schema) = open_app_and_schema(&state, &workspace, &app, &coll).await?;
    let rule_filter = authorize_record_action(
        &auth,
        &app_pool,
        &workspace,
        &app,
        &coll,
        AccessAction::Create,
        &schema,
    )
    .await?;
    // Template rules don't apply to creation: the record doesn't exist
    // yet, so there's nothing to evaluate against. Refuse rather than
    // silently treating the rule as open.
    if rule_filter.is_some() {
        return Err(ApiError::Core(CoreError::Forbidden));
    }

    // before-hook: may mutate fields, throw to veto.
    let request = hook_request(&auth, &workspace, &app, &coll);
    let fields = state
        .hooks
        .dispatch_before_create(&workspace, &app, &coll, &request, req.fields)
        .await?;

    let rec = create_record(&app_pool, &schema, fields).await?;
    state.broker.publish(
        &SubscriptionKey::new(&workspace, &app, &coll),
        RealtimeEvent::RecordCreated {
            record: rec.clone(),
        },
    );
    if let Err(e) = state
        .hooks
        .dispatch(
            &workspace,
            &app,
            &coll,
            HookEvent::AfterCreate,
            &request,
            &rec,
        )
        .await
    {
        tracing::error!(error = %e, "hook dispatch (after_create) failed");
    }
    Ok((StatusCode::CREATED, Json(rec)))
}

pub async fn get(
    auth: PrincipalAuth,
    State(state): State<AppState>,
    Path((workspace, app, coll, id)): Path<(String, String, String, String)>,
) -> Result<Json<Record>, ApiError> {
    let (app_pool, schema) = open_app_and_schema(&state, &workspace, &app, &coll).await?;
    let rule_filter = authorize_record_action(
        &auth,
        &app_pool,
        &workspace,
        &app,
        &coll,
        AccessAction::View,
        &schema,
    )
    .await?;

    // Combine `id = :id` with the rule filter (if any) and look up via
    // list_records so the rule is enforced at the SQL layer.
    let id_filter = FilterNode::Eq("id".into(), serde_json::Value::String(id.clone()));
    let combined = match rule_filter {
        Some(r) => FilterNode::and(id_filter, r),
        None => id_filter,
    };
    let listed = list_records(
        &app_pool,
        &schema,
        ListPage {
            page: 1,
            per_page: 1,
        },
        Some(&combined),
    )
    .await?;
    listed
        .items
        .into_iter()
        .next()
        .map(Json)
        .ok_or(ApiError::Core(CoreError::NotFound {
            collection: coll,
            id,
        }))
}

pub async fn update(
    auth: PrincipalAuth,
    State(state): State<AppState>,
    Path((workspace, app, coll, id)): Path<(String, String, String, String)>,
    Json(req): Json<UpdateRecordRequest>,
) -> Result<Json<Record>, ApiError> {
    let (app_pool, schema) = open_app_and_schema(&state, &workspace, &app, &coll).await?;
    let rule_filter = authorize_record_action(
        &auth,
        &app_pool,
        &workspace,
        &app,
        &coll,
        AccessAction::Update,
        &schema,
    )
    .await?;

    // Fetch the existing row, applying the access rule if any. Empty
    // result = either no such row OR the rule rejects it; report 404
    // either way to avoid leaking existence.
    let id_filter = FilterNode::Eq("id".into(), serde_json::Value::String(id.clone()));
    let lookup_filter = match rule_filter {
        Some(r) => FilterNode::and(id_filter, r),
        None => id_filter,
    };
    let listed = list_records(
        &app_pool,
        &schema,
        ListPage {
            page: 1,
            per_page: 1,
        },
        Some(&lookup_filter),
    )
    .await?;
    let Some(existing) = listed.items.into_iter().next() else {
        return Err(ApiError::Core(CoreError::NotFound {
            collection: coll,
            id,
        }));
    };

    // before-hook: may mutate the patch or throw to veto.
    let request = hook_request(&auth, &workspace, &app, &coll);
    let patch = state
        .hooks
        .dispatch_before_update(&workspace, &app, &coll, &request, &existing, req.fields)
        .await?;

    let rec = update_record(&app_pool, &schema, &id, patch)
        .await
        .map_err(|e| match e {
            DbError::Sqlx(sqlx::Error::RowNotFound) => ApiError::Core(CoreError::NotFound {
                collection: coll.clone(),
                id: id.clone(),
            }),
            other => ApiError::from(other),
        })?;
    state.broker.publish(
        &SubscriptionKey::new(&workspace, &app, &coll),
        RealtimeEvent::RecordUpdated {
            record: rec.clone(),
        },
    );
    if let Err(e) = state
        .hooks
        .dispatch(
            &workspace,
            &app,
            &coll,
            HookEvent::AfterUpdate,
            &request,
            &rec,
        )
        .await
    {
        tracing::error!(error = %e, "hook dispatch (after_update) failed");
    }
    Ok(Json(rec))
}

pub async fn delete(
    auth: PrincipalAuth,
    State(state): State<AppState>,
    Path((workspace, app, coll, id)): Path<(String, String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let (app_pool, schema) = open_app_and_schema(&state, &workspace, &app, &coll).await?;
    let rule_filter = authorize_record_action(
        &auth,
        &app_pool,
        &workspace,
        &app,
        &coll,
        AccessAction::Delete,
        &schema,
    )
    .await?;

    // Same fetch-then-act shape as update so before-hooks get the row.
    let id_filter = FilterNode::Eq("id".into(), serde_json::Value::String(id.clone()));
    let lookup_filter = match rule_filter {
        Some(r) => FilterNode::and(id_filter, r),
        None => id_filter,
    };
    let listed = list_records(
        &app_pool,
        &schema,
        ListPage {
            page: 1,
            per_page: 1,
        },
        Some(&lookup_filter),
    )
    .await?;
    let Some(existing) = listed.items.into_iter().next() else {
        return Err(ApiError::Core(CoreError::NotFound {
            collection: coll,
            id,
        }));
    };

    // before-hook: may throw to veto.
    let request = hook_request(&auth, &workspace, &app, &coll);
    state
        .hooks
        .dispatch_before_delete(&workspace, &app, &coll, &request, &existing)
        .await?;

    delete_record(&app_pool, &schema, &id)
        .await
        .map_err(|e| match e {
            DbError::Sqlx(sqlx::Error::RowNotFound) => ApiError::Core(CoreError::NotFound {
                collection: coll.clone(),
                id: id.clone(),
            }),
            other => ApiError::from(other),
        })?;
    state.broker.publish(
        &SubscriptionKey::new(&workspace, &app, &coll),
        RealtimeEvent::RecordDeleted { id: id.clone() },
    );
    if let Err(e) = state
        .hooks
        .dispatch(
            &workspace,
            &app,
            &coll,
            HookEvent::AfterDelete,
            &request,
            &serde_json::json!({ "id": id }),
        )
        .await
    {
        tracing::error!(error = %e, "hook dispatch (after_delete) failed");
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Result of authorising a request against an access rule.
///
/// - `Ok(None)` — the principal is unconditionally allowed (admin or
///   open rule). Records query needs no additional WHERE.
/// - `Ok(Some(node))` — the principal is allowed *if* a template rule
///   evaluates true; the caller must AND `node` into its records
///   query so the SQL layer filters rows down to those the rule
///   matches. Returned only when the rule is a template AND the
///   principal is a user-of-workspace.
/// - `Err(Forbidden)` — denied outright.
async fn authorize_record_action(
    auth: &PrincipalAuth,
    app_pool: &sqlx::SqlitePool,
    workspace: &str,
    app: &str,
    coll: &str,
    action: AccessAction,
    schema: &Schema,
) -> Result<Option<FilterNode>, ApiError> {
    if auth.is_admin_for_app(workspace, app) {
        return Ok(None);
    }
    if auth.user_workspace() != Some(workspace) {
        return Err(ApiError::Core(CoreError::Forbidden));
    }

    let rule = get_rule(app_pool, coll, action).await?;
    match classify_rule(&rule) {
        RuleDecision::Deny => Err(ApiError::Core(CoreError::Forbidden)),
        RuleDecision::Allow => Ok(None),
        RuleDecision::Evaluate(template) => {
            let ctx = RuleContext {
                user_id: Some(auth.subject_id.clone()),
                user_email: None, // populated in a later branch when the user record is loaded
                user_workspace: auth.user_workspace().map(str::to_string),
            };
            let resolved = rule_template::substitute(&template, &ctx).map_err(ApiError::Core)?;
            let node = parse_filter(&resolved).map_err(ApiError::Core)?;
            validate_filter_columns(&node, schema)?;
            Ok(Some(node))
        }
    }
}

async fn open_app_and_schema(
    state: &AppState,
    workspace: &str,
    app: &str,
    coll: &str,
) -> Result<(sqlx::SqlitePool, rustbase_core::Schema), ApiError> {
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
    let app_pool = state.apps.pool_for(&workspace_id, &app_id).await?;
    let collection = find_collection(&app_pool, coll).await?.ok_or_else(|| {
        ApiError::Core(CoreError::NotFound {
            collection: coll.to_string(),
            id: String::new(),
        })
    })?;
    Ok((app_pool, collection.schema))
}

/// Build a HookRequest from the authenticated principal + path scope.
/// `$app.request` inside JS hooks reflects this.
fn hook_request(auth: &PrincipalAuth, workspace: &str, app: &str, coll: &str) -> HookRequest {
    let role = match auth.claims.role {
        rustbase_auth::TokenRole::MasterAdmin => "master_admin",
        rustbase_auth::TokenRole::WorkspaceAdmin => "workspace_admin",
        rustbase_auth::TokenRole::AppAdmin => "app_admin",
        rustbase_auth::TokenRole::User => "user",
    }
    .to_string();
    HookRequest {
        auth: Some(HookAuth {
            id: auth.subject_id.clone(),
            role,
            workspace: auth.claims.workspace.clone(),
        }),
        workspace: workspace.to_string(),
        app: app.to_string(),
        collection: coll.to_string(),
    }
}
