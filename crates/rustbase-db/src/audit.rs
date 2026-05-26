//! Audit log helper.
//!
//! Every scope (system, realm, app) has an `audit_log` table with the
//! same shape. This module is generic over the pool — callers pass the
//! one matching the event's scope.

use crate::error::Result;
use chrono::Utc;
use serde_json::Value;
use sqlx::SqlitePool;

pub async fn append(
    pool: &SqlitePool,
    actor: Option<&str>,
    action: &str,
    target: Option<&str>,
    details: &Value,
) -> Result<i64> {
    let res = sqlx::query(
        "INSERT INTO audit_log (ts, actor, action, target, details_json) \
         VALUES (?, ?, ?, ?, ?) \
         RETURNING id",
    )
    .bind(Utc::now())
    .bind(actor)
    .bind(action)
    .bind(target)
    .bind(details.to_string())
    .fetch_one(pool)
    .await?;
    let id: i64 = sqlx::Row::try_get(&res, 0)?;
    Ok(id)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AuditEntry {
    pub id: i64,
    pub ts: chrono::DateTime<chrono::Utc>,
    pub actor: Option<String>,
    pub action: String,
    pub target: Option<String>,
    pub details_json: Option<String>,
}

pub async fn list_recent(pool: &SqlitePool, limit: i64) -> Result<Vec<AuditEntry>> {
    let rows: Vec<AuditEntry> = sqlx::query_as(
        "SELECT id, ts, actor, action, target, details_json \
         FROM audit_log ORDER BY id DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::{SYSTEM_MIGRATIONS, apply_migrations};
    use crate::pool::open_memory_pool;
    use serde_json::json;

    async fn fresh_pool() -> SqlitePool {
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(pool.clone(), SYSTEM_MIGRATIONS)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn append_returns_id_and_is_listable() {
        let pool = fresh_pool().await;
        let id = append(
            &pool,
            Some("admin-1"),
            "policy_set",
            Some("password.length"),
            &json!({"before": null, "after": {"min": 4, "max": 64}}),
        )
        .await
        .unwrap();
        assert!(id > 0);

        let entries = list_recent(&pool, 10).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "policy_set");
        assert_eq!(entries[0].target.as_deref(), Some("password.length"));
        assert_eq!(entries[0].actor.as_deref(), Some("admin-1"));
    }
}
