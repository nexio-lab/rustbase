//! Bridge that gives JS hooks read + create access to records.
//!
//! `rustbase-runtime` defines `AsyncRecordsBridge` so it doesn't have
//! to depend on this crate; we implement it here in terms of the
//! existing sqlx + filter machinery and wrap it in `SyncBridge` for
//! the JS runtime.

use async_trait::async_trait;
use rustbase_core::{AppId, FilterNode, WorkspaceId, parse_filter};
use rustbase_db::{
    AppPoolManager, DbError, ListPage, audit,
    collections::find_collection,
    records::{create_record, delete_record, find_record, list_records, update_record},
};
use rustbase_runtime::{
    AsyncRecordsBridge, AuditBridge, FetchBridge, FetchRequest, FetchResponse, Result as RtResult,
    RuntimeError, SyncBridge,
};
use serde_json::Value as Json;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

/// Per-(workspace, app) bridge. Cheaply cloneable: the pool manager is
/// already `Arc`'d.
pub struct ApiBridge {
    workspace: WorkspaceId,
    app: AppId,
    apps: Arc<AppPoolManager>,
}

impl ApiBridge {
    pub fn new(workspace: WorkspaceId, app: AppId, apps: Arc<AppPoolManager>) -> Self {
        Self {
            workspace,
            app,
            apps,
        }
    }

    /// Wrap `self` in a `SyncBridge` so the rquickjs callbacks can
    /// call it synchronously via `block_in_place + block_on`.
    pub fn into_sync(self) -> Arc<SyncBridge<Self>> {
        Arc::new(SyncBridge(Arc::new(self)))
    }

    async fn pool(&self) -> RtResult<sqlx::SqlitePool> {
        self.apps
            .pool_for(&self.workspace, &self.app)
            .await
            .map_err(|e| RuntimeError::Js(format!("pool: {e}")))
    }
}

#[async_trait]
impl AsyncRecordsBridge for ApiBridge {
    async fn find_one(&self, collection: &str, id: &str) -> RtResult<Option<Json>> {
        let pool = self.pool().await?;
        let Some(coll) = find_collection(&pool, collection)
            .await
            .map_err(|e| RuntimeError::Js(format!("find_collection: {e}")))?
        else {
            return Err(RuntimeError::Js(format!(
                "unknown collection: {collection}"
            )));
        };
        let rec = find_record(&pool, &coll.schema, id)
            .await
            .map_err(|e| RuntimeError::Js(format!("find_record: {e}")))?;
        Ok(rec.and_then(|r| serde_json::to_value(r).ok()))
    }

    async fn find_by_filter(
        &self,
        collection: &str,
        filter: &str,
        per_page: u32,
    ) -> RtResult<Vec<Json>> {
        let pool = self.pool().await?;
        let Some(coll) = find_collection(&pool, collection)
            .await
            .map_err(|e| RuntimeError::Js(format!("find_collection: {e}")))?
        else {
            return Err(RuntimeError::Js(format!(
                "unknown collection: {collection}"
            )));
        };
        let node: Option<FilterNode> = if filter.trim().is_empty() {
            None
        } else {
            Some(parse_filter(filter).map_err(|e| RuntimeError::Js(format!("filter: {e}")))?)
        };
        let listed = list_records(
            &pool,
            &coll.schema,
            ListPage {
                page: 1,
                per_page: per_page.clamp(1, 200),
            },
            node.as_ref(),
        )
        .await
        .map_err(|e| RuntimeError::Js(format!("list_records: {e}")))?;
        Ok(listed
            .items
            .into_iter()
            .filter_map(|r| serde_json::to_value(r).ok())
            .collect())
    }

    async fn create(&self, collection: &str, fields: BTreeMap<String, Json>) -> RtResult<Json> {
        let pool = self.pool().await?;
        let Some(coll) = find_collection(&pool, collection)
            .await
            .map_err(|e| RuntimeError::Js(format!("find_collection: {e}")))?
        else {
            return Err(RuntimeError::Js(format!(
                "unknown collection: {collection}"
            )));
        };
        let rec = create_record(&pool, &coll.schema, fields)
            .await
            .map_err(|e| RuntimeError::Js(format!("create_record: {e}")))?;
        serde_json::to_value(rec).map_err(|e| RuntimeError::Js(format!("serialise: {e}")))
    }

    async fn update(
        &self,
        collection: &str,
        id: &str,
        patch: BTreeMap<String, Json>,
    ) -> RtResult<Json> {
        let pool = self.pool().await?;
        let Some(coll) = find_collection(&pool, collection)
            .await
            .map_err(|e| RuntimeError::Js(format!("find_collection: {e}")))?
        else {
            return Err(RuntimeError::Js(format!(
                "unknown collection: {collection}"
            )));
        };
        let rec = update_record(&pool, &coll.schema, id, patch)
            .await
            .map_err(|e| match e {
                DbError::Sqlx(sqlx::Error::RowNotFound) => {
                    RuntimeError::Js(format!("not found: {collection}/{id}"))
                }
                other => RuntimeError::Js(format!("update_record: {other}")),
            })?;
        serde_json::to_value(rec).map_err(|e| RuntimeError::Js(format!("serialise: {e}")))
    }

    async fn delete(&self, collection: &str, id: &str) -> RtResult<()> {
        let pool = self.pool().await?;
        let Some(coll) = find_collection(&pool, collection)
            .await
            .map_err(|e| RuntimeError::Js(format!("find_collection: {e}")))?
        else {
            return Err(RuntimeError::Js(format!(
                "unknown collection: {collection}"
            )));
        };
        delete_record(&pool, &coll.schema, id)
            .await
            .map_err(|e| match e {
                DbError::Sqlx(sqlx::Error::RowNotFound) => {
                    RuntimeError::Js(format!("not found: {collection}/{id}"))
                }
                other => RuntimeError::Js(format!("delete_record: {other}")),
            })
    }
}

/// Audit-log bridge for `$app.audit.write(...)`. Writes hook-side
/// audit entries into the same per-app `audit_log` table the API
/// handlers append to, with `actor = "hook"` so the dashboard can
/// distinguish operator events from user-initiated ones.
pub struct ApiAuditBridge {
    workspace: WorkspaceId,
    app: AppId,
    apps: Arc<AppPoolManager>,
}

impl ApiAuditBridge {
    pub fn new(workspace: WorkspaceId, app: AppId, apps: Arc<AppPoolManager>) -> Arc<Self> {
        Arc::new(Self {
            workspace,
            app,
            apps,
        })
    }
}

impl AuditBridge for ApiAuditBridge {
    fn write(&self, action: &str, target: Option<&str>, details_json: &str) -> RtResult<()> {
        let workspace = self.workspace.clone();
        let app = self.app.clone();
        let apps = self.apps.clone();
        let action = action.to_string();
        let target = target.map(str::to_string);
        let details: Json = serde_json::from_str(details_json)
            .map_err(|e| RuntimeError::Internal(format!("audit details JSON: {e}")))?;
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let pool = apps
                    .pool_for(&workspace, &app)
                    .await
                    .map_err(|e| RuntimeError::Internal(format!("audit pool: {e}")))?;
                audit::append(&pool, Some("hook"), &action, target.as_deref(), &details)
                    .await
                    .map_err(|e| RuntimeError::Internal(format!("audit append: {e}")))?;
                Ok(())
            })
        })
    }
}

/// HTTP-fetch bridge for `$app.fetch(url, init)`.
///
/// Holds a shared `reqwest::Client` so connection-pooling persists
/// across hook invocations, and an allowlist of host strings. A
/// request whose URL's host isn't on the allowlist is rejected with
/// `RuntimeError::Forbidden` before any network IO happens. Empty
/// allowlist = `$app.fetch` is effectively disabled.
pub struct ApiFetchBridge {
    client: reqwest::Client,
    allowed_hosts: Vec<String>,
}

impl ApiFetchBridge {
    /// Build a fetcher with the supplied host allowlist. The
    /// `reqwest::Client` is built with a 30 s default timeout so a
    /// stuck upstream doesn't hold the JS interpreter hostage past
    /// the hook's CPU budget.
    pub fn new(allowed_hosts: Vec<String>) -> Arc<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Arc::new(Self {
            client,
            allowed_hosts,
        })
    }

    fn host_allowed(&self, url: &str) -> Option<String> {
        let after_scheme = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
        let host = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
        let bare = host.split_once(':').map(|(h, _)| h).unwrap_or(host);
        if self.allowed_hosts.iter().any(|h| h == bare) {
            Some(bare.to_string())
        } else {
            None
        }
    }
}

impl FetchBridge for ApiFetchBridge {
    fn request(&self, req: FetchRequest) -> RtResult<FetchResponse> {
        if self.host_allowed(&req.url).is_none() {
            return Err(RuntimeError::Forbidden(format!(
                "host for {:?} not on workspace fetch allowlist",
                req.url
            )));
        }
        let method = reqwest::Method::from_bytes(req.method.as_bytes())
            .map_err(|e| RuntimeError::Internal(format!("bad method: {e}")))?;
        let mut builder = self.client.request(method, &req.url);
        for (k, v) in &req.headers {
            builder = builder.header(k, v);
        }
        if !req.body.is_empty() {
            builder = builder.body(req.body);
        }
        let client = self.client.clone();
        let request = builder
            .build()
            .map_err(|e| RuntimeError::Internal(format!("build request: {e}")))?;
        let resp = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move { client.execute(request).await })
        })
        .map_err(|e| RuntimeError::Internal(format!("fetch: {e}")))?;

        let status = resp.status().as_u16();
        let mut headers = BTreeMap::new();
        for (k, v) in resp.headers().iter() {
            headers.insert(
                k.as_str().to_lowercase(),
                v.to_str().unwrap_or("").to_string(),
            );
        }
        let body_bytes = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move { resp.bytes().await })
        })
        .map_err(|e| RuntimeError::Internal(format!("read body: {e}")))?;
        let body = String::from_utf8_lossy(&body_bytes).to_string();
        Ok(FetchResponse {
            status,
            headers,
            body,
        })
    }
}
