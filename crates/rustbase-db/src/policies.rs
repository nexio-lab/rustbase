//! Per-scope policy storage.
//!
//! Each scope (system, realm, app) has its own `policies` table —
//! same shape, just in a different DB. This module is generic over
//! the pool; callers pick `system.db`, the realm's `realm.db`, or an
//! app's `data.db`.
//!
//! `PolicySpec` is serialized as JSON in the `policy_json` column.
//! Validation against parent bounds and the auto-clamp cascade live
//! in `policy_engine`.

use crate::error::{DbError, Result};
use chrono::{DateTime, Utc};
use rustbase_core::PolicySpec;
use sqlx::SqlitePool;

#[derive(Debug, Clone, PartialEq)]
pub struct PolicyRow {
    pub field: String,
    pub spec: PolicySpec,
    pub updated_at: DateTime<Utc>,
}

pub async fn get_policy(pool: &SqlitePool, field: &str) -> Result<Option<PolicySpec>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT policy_json FROM policies WHERE field = ?")
            .bind(field)
            .fetch_optional(pool)
            .await?;
    row.map(|(json,)| {
        serde_json::from_str(&json).map_err(|e| {
            DbError::InvalidIdentifier(format!("policy_json for '{field}': {e}"))
        })
    })
    .transpose()
}

pub async fn upsert_policy(
    pool: &SqlitePool,
    field: &str,
    spec: &PolicySpec,
) -> Result<DateTime<Utc>> {
    let json = serde_json::to_string(spec)
        .map_err(|e| DbError::InvalidIdentifier(format!("policy_json: {e}")))?;
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO policies (field, policy_json, updated_at) VALUES (?, ?, ?) \
         ON CONFLICT(field) DO UPDATE SET policy_json = excluded.policy_json, updated_at = excluded.updated_at",
    )
    .bind(field)
    .bind(&json)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(now)
}

pub async fn delete_policy(pool: &SqlitePool, field: &str) -> Result<()> {
    let res = sqlx::query("DELETE FROM policies WHERE field = ?")
        .bind(field)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::Sqlx(sqlx::Error::RowNotFound));
    }
    Ok(())
}

pub async fn list_policies(pool: &SqlitePool) -> Result<Vec<PolicyRow>> {
    let rows: Vec<(String, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT field, policy_json, updated_at FROM policies ORDER BY field",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(field, json, updated_at)| {
            let spec: PolicySpec = serde_json::from_str(&json).map_err(|e| {
                DbError::InvalidIdentifier(format!("policy_json for '{field}': {e}"))
            })?;
            Ok(PolicyRow {
                field,
                spec,
                updated_at,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::{SYSTEM_MIGRATIONS, apply_migrations};
    use crate::pool::open_memory_pool;
    use rustbase_core::{RangePolicy, PolicySpec};

    async fn fresh_pool() -> SqlitePool {
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(pool.clone(), SYSTEM_MIGRATIONS).await.unwrap();
        pool
    }

    fn range(min: i64, max: i64) -> PolicySpec {
        PolicySpec::Range(RangePolicy::new(min, max).unwrap())
    }

    #[tokio::test]
    async fn upsert_get_round_trip() {
        let pool = fresh_pool().await;
        upsert_policy(&pool, "password.length", &range(4, 64))
            .await
            .unwrap();
        let got = get_policy(&pool, "password.length").await.unwrap().unwrap();
        assert_eq!(got, range(4, 64));
    }

    #[tokio::test]
    async fn upsert_replaces_existing() {
        let pool = fresh_pool().await;
        upsert_policy(&pool, "password.length", &range(4, 64))
            .await
            .unwrap();
        upsert_policy(&pool, "password.length", &range(8, 32))
            .await
            .unwrap();
        assert_eq!(
            get_policy(&pool, "password.length").await.unwrap().unwrap(),
            range(8, 32)
        );
    }

    #[tokio::test]
    async fn delete_unknown_returns_row_not_found() {
        let pool = fresh_pool().await;
        let err = delete_policy(&pool, "absent").await.unwrap_err();
        assert!(matches!(err, DbError::Sqlx(sqlx::Error::RowNotFound)));
    }

    #[tokio::test]
    async fn list_returns_all_sorted_by_field() {
        let pool = fresh_pool().await;
        upsert_policy(&pool, "z", &range(0, 1)).await.unwrap();
        upsert_policy(&pool, "a", &range(0, 1)).await.unwrap();
        let rows = list_policies(&pool).await.unwrap();
        let fields: Vec<_> = rows.iter().map(|r| r.field.clone()).collect();
        assert_eq!(fields, vec!["a", "z"]);
    }
}
