//! End-user storage. One row per user inside a realm's `realm.db`.

use crate::error::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct User {
    pub id: String,
    pub email: String,
    pub password_hash: Option<String>,
    pub verified: bool,
    pub last_login: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

pub async fn insert_user(pool: &SqlitePool, email: &str, password_hash: &str) -> Result<User> {
    let id = Uuid::now_v7().to_string();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, verified, last_login, created_at) \
         VALUES (?, ?, ?, 0, NULL, ?)",
    )
    .bind(&id)
    .bind(email)
    .bind(password_hash)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(User {
        id,
        email: email.to_string(),
        password_hash: Some(password_hash.to_string()),
        verified: false,
        last_login: None,
        created_at: now,
    })
}

pub async fn find_user_by_email(pool: &SqlitePool, email: &str) -> Result<Option<User>> {
    let row: Option<User> = sqlx::query_as(
        "SELECT id, email, password_hash, CAST(verified AS BOOLEAN) AS verified, \
                last_login, created_at \
         FROM users WHERE email = ?",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn find_user_by_id(pool: &SqlitePool, id: &str) -> Result<Option<User>> {
    let row: Option<User> = sqlx::query_as(
        "SELECT id, email, password_hash, CAST(verified AS BOOLEAN) AS verified, \
                last_login, created_at \
         FROM users WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn record_last_login(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("UPDATE users SET last_login = ? WHERE id = ?")
        .bind(Utc::now())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Flip `verified` to true. Idempotent: re-running on an already-
/// verified user is a no-op write.
pub async fn mark_verified(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("UPDATE users SET verified = 1 WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Replace the user's password hash. Callers supply the already-hashed
/// PHC string; this module never touches plaintext.
pub async fn set_password_hash(pool: &SqlitePool, id: &str, hash: &str) -> Result<()> {
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(hash)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::{REALM_MIGRATIONS, apply_migrations};
    use crate::pool::open_memory_pool;

    async fn fresh_pool() -> SqlitePool {
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(pool.clone(), REALM_MIGRATIONS)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn insert_then_find_by_email() {
        let pool = fresh_pool().await;
        let u = insert_user(&pool, "ada@x.com", "$argon2id$hash")
            .await
            .unwrap();
        let f = find_user_by_email(&pool, "ada@x.com")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(u.id, f.id);
        assert!(!f.verified);
        assert!(f.last_login.is_none());
    }

    #[tokio::test]
    async fn duplicate_email_is_unique_violation() {
        let pool = fresh_pool().await;
        insert_user(&pool, "a@x.com", "h").await.unwrap();
        let err = insert_user(&pool, "a@x.com", "h2").await.unwrap_err();
        assert!(matches!(err, crate::DbError::Sqlx(_)));
    }

    #[tokio::test]
    async fn record_last_login_updates_column() {
        let pool = fresh_pool().await;
        let u = insert_user(&pool, "ada@x.com", "h").await.unwrap();
        record_last_login(&pool, &u.id).await.unwrap();
        let again = find_user_by_id(&pool, &u.id).await.unwrap().unwrap();
        assert!(again.last_login.is_some());
    }
}
