//! Realm rows in `system.db`.

use crate::error::Result;
use chrono::{DateTime, Utc};
use rustbase_core::MASTER_REALM_ID;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Realm {
    pub id: String,
    pub name: String,
    pub is_master: bool,
    pub created_at: DateTime<Utc>,
}

/// Ensure the master realm row exists. Idempotent — calling repeatedly
/// never overwrites a renamed master.
pub async fn ensure_master_realm(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO realms (id, name, is_master, created_at) \
         VALUES (?, ?, 1, ?)",
    )
    .bind(MASTER_REALM_ID)
    .bind("Master")
    .bind(Utc::now())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn find_realm(pool: &SqlitePool, id: &str) -> Result<Option<Realm>> {
    let row: Option<Realm> = sqlx::query_as(
        "SELECT id, name, CAST(is_master AS BOOLEAN) AS is_master, created_at \
         FROM realms WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn list_realms(pool: &SqlitePool) -> Result<Vec<Realm>> {
    let rows: Vec<Realm> = sqlx::query_as(
        "SELECT id, name, CAST(is_master AS BOOLEAN) AS is_master, created_at \
         FROM realms ORDER BY is_master DESC, created_at ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::{SYSTEM_MIGRATIONS, apply_migrations};
    use crate::pool::open_memory_pool;

    async fn fresh_pool() -> SqlitePool {
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(&pool, SYSTEM_MIGRATIONS).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn ensure_master_is_idempotent() {
        let pool = fresh_pool().await;
        ensure_master_realm(&pool).await.unwrap();
        ensure_master_realm(&pool).await.unwrap();
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM realms WHERE is_master = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn find_master_realm_after_ensure() {
        let pool = fresh_pool().await;
        ensure_master_realm(&pool).await.unwrap();
        let m = find_realm(&pool, MASTER_REALM_ID).await.unwrap().unwrap();
        assert_eq!(m.id, MASTER_REALM_ID);
        assert!(m.is_master);
    }

    #[tokio::test]
    async fn list_returns_master_first() {
        let pool = fresh_pool().await;
        ensure_master_realm(&pool).await.unwrap();
        sqlx::query("INSERT INTO realms (id, name, is_master, created_at) VALUES (?, ?, 0, ?)")
            .bind("acme")
            .bind("Acme")
            .bind(Utc::now())
            .execute(&pool)
            .await
            .unwrap();
        let realms = list_realms(&pool).await.unwrap();
        assert_eq!(realms.len(), 2);
        assert_eq!(realms[0].id, MASTER_REALM_ID);
        assert!(realms[0].is_master);
        assert!(!realms[1].is_master);
    }
}
