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

#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    pub page: u32,
    pub per_page: u32,
    /// Substring match on `action` (case-insensitive via `LIKE`).
    pub action: Option<String>,
    /// Exact match on `actor`.
    pub actor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ListedAudit {
    pub items: Vec<AuditEntry>,
    pub page: u32,
    pub per_page: u32,
    pub total_items: u64,
}

/// Page through the audit log, newest first. `action` is matched as a
/// case-insensitive substring; `actor` is matched verbatim. Bounds are
/// clamped to keep a buggy caller from asking for a million rows.
pub async fn list_paginated(pool: &SqlitePool, q: AuditQuery) -> Result<ListedAudit> {
    let per_page = q.per_page.clamp(1, 200);
    let page = q.page.max(1);
    let offset = ((page - 1) as i64) * (per_page as i64);

    // Building the WHERE clause by hand keeps the bind parameters in a
    // straight line. We never interpolate user input — only `LIKE` patterns
    // built around bound parameters.
    let mut where_sql = String::new();
    if q.action.is_some() {
        where_sql.push_str(" WHERE action LIKE ?");
    }
    if q.actor.is_some() {
        where_sql.push_str(if where_sql.is_empty() {
            " WHERE actor = ?"
        } else {
            " AND actor = ?"
        });
    }

    let count_sql = format!("SELECT COUNT(*) FROM audit_log{where_sql}");
    let list_sql = format!(
        "SELECT id, ts, actor, action, target, details_json \
         FROM audit_log{where_sql} ORDER BY id DESC LIMIT ? OFFSET ?"
    );

    let action_like = q.action.as_ref().map(|s| format!("%{s}%"));

    // total
    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql);
    if let Some(like) = &action_like {
        count_q = count_q.bind(like);
    }
    if let Some(actor) = &q.actor {
        count_q = count_q.bind(actor);
    }
    let total: i64 = count_q.fetch_one(pool).await?;

    // page
    let mut list_q = sqlx::query_as::<_, AuditEntry>(&list_sql);
    if let Some(like) = &action_like {
        list_q = list_q.bind(like);
    }
    if let Some(actor) = &q.actor {
        list_q = list_q.bind(actor);
    }
    let items = list_q
        .bind(per_page as i64)
        .bind(offset)
        .fetch_all(pool)
        .await?;

    Ok(ListedAudit {
        items,
        page,
        per_page,
        total_items: total as u64,
    })
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

    async fn seed(pool: &SqlitePool) {
        for i in 0..7 {
            append(
                pool,
                Some(if i % 2 == 0 { "alice" } else { "bob" }),
                if i < 4 {
                    "policy_set"
                } else {
                    "policy_clamped"
                },
                Some("field"),
                &json!({"i": i}),
            )
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn paginated_lists_newest_first() {
        let pool = fresh_pool().await;
        seed(&pool).await;

        let page1 = list_paginated(
            &pool,
            AuditQuery {
                page: 1,
                per_page: 3,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(page1.total_items, 7);
        assert_eq!(page1.items.len(), 3);
        // Newest first → last seeded `i = 6` is on top.
        let first_details = page1.items[0].details_json.as_deref().unwrap();
        assert!(first_details.contains("\"i\":6"));

        let page3 = list_paginated(
            &pool,
            AuditQuery {
                page: 3,
                per_page: 3,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(page3.items.len(), 1);
    }

    #[tokio::test]
    async fn paginated_filters_by_action_and_actor() {
        let pool = fresh_pool().await;
        seed(&pool).await;

        // action substring
        let clamps = list_paginated(
            &pool,
            AuditQuery {
                page: 1,
                per_page: 30,
                action: Some("clamp".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(clamps.total_items, 3);
        assert!(clamps.items.iter().all(|e| e.action == "policy_clamped"));

        // actor exact + action substring intersection
        let bob_clamps = list_paginated(
            &pool,
            AuditQuery {
                page: 1,
                per_page: 30,
                action: Some("clamp".into()),
                actor: Some("bob".into()),
            },
        )
        .await
        .unwrap();
        assert!(
            bob_clamps
                .items
                .iter()
                .all(|e| e.actor.as_deref() == Some("bob"))
        );
        assert!(
            bob_clamps
                .items
                .iter()
                .all(|e| e.action == "policy_clamped")
        );
    }
}
