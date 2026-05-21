//! Key/value store for server-wide secrets that need to outlive a
//! restart (currently: the master JWT signing key).
//!
//! The table lives in `system.db`. Values are opaque bytes; this module
//! never generates them — the caller (`rustbase-server` at boot, or
//! `rustbase-auth`) does, then asks us to persist or fetch.

use crate::error::Result;
use chrono::Utc;
use sqlx::SqlitePool;

pub const MASTER_SIGNING_KEY: &str = "master_signing_key";

pub async fn get_secret(pool: &SqlitePool, name: &str) -> Result<Option<Vec<u8>>> {
    let row: Option<Vec<u8>> = sqlx::query_scalar("SELECT value FROM _secrets WHERE name = ?")
        .bind(name)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

pub async fn put_secret(pool: &SqlitePool, name: &str, value: &[u8]) -> Result<()> {
    sqlx::query(
        "INSERT OR REPLACE INTO _secrets (name, value, created_at) VALUES (?, ?, ?)",
    )
    .bind(name)
    .bind(value)
    .bind(Utc::now())
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch `name`; if missing, persist `default` and return it. Lets the
/// caller's RNG never leak into this module.
pub async fn get_or_init_secret(
    pool: &SqlitePool,
    name: &str,
    default: &[u8],
) -> Result<Vec<u8>> {
    if let Some(value) = get_secret(pool, name).await? {
        return Ok(value);
    }
    put_secret(pool, name, default).await?;
    Ok(default.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::{SYSTEM_MIGRATIONS, apply_migrations};
    use crate::pool::open_memory_pool;

    async fn fresh_pool() -> SqlitePool {
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(pool.clone(), SYSTEM_MIGRATIONS).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn put_then_get_round_trips() {
        let pool = fresh_pool().await;
        let bytes = vec![1, 2, 3, 4, 5];
        put_secret(&pool, "test", &bytes).await.unwrap();
        let got = get_secret(&pool, "test").await.unwrap().unwrap();
        assert_eq!(got, bytes);
    }

    #[tokio::test]
    async fn get_or_init_persists_default_once() {
        let pool = fresh_pool().await;
        let first = get_or_init_secret(&pool, "k", &[7, 7, 7]).await.unwrap();
        let second = get_or_init_secret(&pool, "k", &[9, 9, 9]).await.unwrap();
        assert_eq!(first, vec![7, 7, 7]);
        assert_eq!(second, vec![7, 7, 7]); // default ignored on second call
    }

    #[tokio::test]
    async fn missing_secret_returns_none() {
        let pool = fresh_pool().await;
        assert!(get_secret(&pool, "absent").await.unwrap().is_none());
    }
}
