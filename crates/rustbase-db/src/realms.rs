//! Realm rows in `system.db`.

use crate::error::{DbError, Result};
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

/// Insert a non-master realm row. Master rows are inserted only by
/// `ensure_master_realm` at boot.
pub async fn create_realm(pool: &SqlitePool, id: &str, name: &str) -> Result<Realm> {
    let now = Utc::now();
    sqlx::query("INSERT INTO realms (id, name, is_master, created_at) VALUES (?, ?, 0, ?)")
        .bind(id)
        .bind(name)
        .bind(now)
        .execute(pool)
        .await?;
    Ok(Realm {
        id: id.to_string(),
        name: name.to_string(),
        is_master: false,
        created_at: now,
    })
}

/// Rename any realm (including master). Returns RowNotFound if the
/// realm doesn't exist.
pub async fn rename_realm(pool: &SqlitePool, id: &str, new_name: &str) -> Result<()> {
    let res = sqlx::query("UPDATE realms SET name = ? WHERE id = ?")
        .bind(new_name)
        .bind(id)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::Sqlx(sqlx::Error::RowNotFound));
    }
    Ok(())
}

/// Delete a non-master realm. The `is_master = 0` predicate is defense
/// in depth — handlers should refuse master deletion first.
pub async fn delete_realm(pool: &SqlitePool, id: &str) -> Result<()> {
    let res = sqlx::query("DELETE FROM realms WHERE id = ? AND is_master = 0")
        .bind(id)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::Sqlx(sqlx::Error::RowNotFound));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::{SYSTEM_MIGRATIONS, apply_migrations};
    use crate::pool::open_memory_pool;

    async fn fresh_pool() -> SqlitePool {
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(pool.clone(), SYSTEM_MIGRATIONS)
            .await
            .unwrap();
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
