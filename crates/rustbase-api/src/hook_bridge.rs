//! Bridge that gives JS hooks read + create access to records.
//!
//! `rustbase-runtime` defines `AsyncRecordsBridge` so it doesn't have
//! to depend on this crate; we implement it here in terms of the
//! existing sqlx + filter machinery and wrap it in `SyncBridge` for
//! the JS runtime.

use async_trait::async_trait;
use rustbase_core::{AppId, FilterNode, WorkspaceId, parse_filter};
use rustbase_db::{
    AppPoolManager, DbError, ListPage,
    collections::find_collection,
    records::{create_record, delete_record, find_record, list_records, update_record},
};
use rustbase_runtime::{AsyncRecordsBridge, Result as RtResult, RuntimeError, SyncBridge};
use serde_json::Value as Json;
use std::collections::BTreeMap;
use std::sync::Arc;

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
