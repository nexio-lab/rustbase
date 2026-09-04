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
    ///
    /// Redirects are NOT followed. The allowlist is enforced once,
    /// before the request leaves; a 302 from an authorised host would
    /// otherwise carry the hook to an address nobody vetted, which is
    /// the whole allowlist defeated by one `Location` header. The
    /// redirect is handed back to the JS caller instead, which can
    /// re-issue it through `$app.fetch` and get it checked properly.
    pub fn new(allowed_hosts: Vec<String>) -> Arc<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, response::IntoResponse, routing::get};

    /// Boot a throw-away HTTP server on a random loopback port and
    /// hand back its port plus a shutdown-on-drop handle.
    async fn serve(router: Router) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        (port, handle)
    }

    /// The allowlist is checked once, before the request goes out. If
    /// an allowed host answers with a redirect to a host that is NOT
    /// on the list, following it would take the hook somewhere the
    /// operator never authorised — cloud metadata endpoints being the
    /// obvious prize. The bridge must hand the redirect back instead.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_redirect_off_the_allowlist_is_not_followed() {
        let (secret_port, _secret) = serve(Router::new().route(
            "/secret",
            get(|| async { "OFF-ALLOWLIST-BODY".into_response() }),
        ))
        .await;

        let target = format!("http://localhost:{secret_port}/secret");
        let (hop_port, _hop) = serve(Router::new().route(
            "/hop",
            get(move || {
                let target = target.clone();
                async move { axum::response::Redirect::temporary(&target).into_response() }
            }),
        ))
        .await;

        // Only the hop host is authorised. `localhost` is a different
        // host string and is deliberately absent from the list.
        let bridge = ApiFetchBridge::new(vec!["127.0.0.1".to_string()]);
        let resp = bridge
            .request(FetchRequest {
                method: "GET".into(),
                url: format!("http://127.0.0.1:{hop_port}/hop"),
                headers: BTreeMap::new(),
                body: Vec::new(),
            })
            .unwrap();

        assert!(
            !resp.body.contains("OFF-ALLOWLIST-BODY"),
            "bridge followed a redirect off the allowlist; body: {}",
            resp.body
        );
        assert_eq!(
            resp.status, 307,
            "the redirect itself should be handed back to the hook"
        );
    }
}
