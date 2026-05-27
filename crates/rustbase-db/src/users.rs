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

/// Create a user with no password hash (NULL column) — e.g. for
/// passwordless OTP signup. The user can only log in via OTP / OAuth
/// until they explicitly set a password (which the password-reset
/// flow already covers via `set_password_hash`).
pub async fn insert_passwordless_user(pool: &SqlitePool, email: &str) -> Result<User> {
    let id = Uuid::now_v7().to_string();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, verified, last_login, created_at) \
         VALUES (?, ?, NULL, 0, NULL, ?)",
    )
    .bind(&id)
    .bind(email)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(User {
        id,
        email: email.to_string(),
        password_hash: None,
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

/// Admin-facing search. `email_like` is wrapped in `%…%` if non-empty;
/// an empty string lists every user in the realm. Ordered by
/// `created_at DESC` so the most recent signups land on page 1.
pub async fn list_users(
    pool: &SqlitePool,
    email_like: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<User>> {
    let pattern = if email_like.is_empty() {
        "%".to_string()
    } else {
        format!("%{email_like}%")
    };
    let rows: Vec<User> = sqlx::query_as(
        "SELECT id, email, password_hash, CAST(verified AS BOOLEAN) AS verified, \
                last_login, created_at \
         FROM users \
         WHERE email LIKE ? \
         ORDER BY created_at DESC \
         LIMIT ? OFFSET ?",
    )
    .bind(&pattern)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Total count for the same filter as `list_users`. Cheap COUNT(*)
/// against the same LIKE pattern so the UI can show "page X of Y".
pub async fn count_users(pool: &SqlitePool, email_like: &str) -> Result<i64> {
    let pattern = if email_like.is_empty() {
        "%".to_string()
    } else {
        format!("%{email_like}%")
    };
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users WHERE email LIKE ?")
        .bind(&pattern)
        .fetch_one(pool)
        .await?;
    Ok(n)
}

/// Hard delete. Cascades to `_email_verifications`, `_password_resets`,
/// `_email_otps`, `_user_totp`, `_mfa_challenges`, `user_oauth_links`
/// via their `ON DELETE CASCADE` foreign keys. Returns the row count
/// affected — 0 means the user didn't exist.
pub async fn delete_user(pool: &SqlitePool, id: &str) -> Result<u64> {
    let r = sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(r.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::{APP_MIGRATIONS, apply_migrations};
    use crate::pool::open_memory_pool;

    async fn fresh_pool() -> SqlitePool {
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(pool.clone(), APP_MIGRATIONS)
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

    #[tokio::test]
    async fn list_paginates_and_orders_by_recency() {
        let pool = fresh_pool().await;
        for i in 0..5 {
            insert_user(&pool, &format!("u{i}@x.com"), "h")
                .await
                .unwrap();
        }
        let page1 = list_users(&pool, "", 2, 0).await.unwrap();
        let page2 = list_users(&pool, "", 2, 2).await.unwrap();
        assert_eq!(page1.len(), 2);
        assert_eq!(page2.len(), 2);
        // Most recent first; uuid_v7 is monotonic so u4 → u3 → u2 → u1 → u0.
        assert_eq!(page1[0].email, "u4@x.com");
        assert_eq!(page1[1].email, "u3@x.com");
        assert_eq!(page2[0].email, "u2@x.com");
        assert_eq!(count_users(&pool, "").await.unwrap(), 5);
    }

    #[tokio::test]
    async fn list_filters_by_email_substring() {
        let pool = fresh_pool().await;
        insert_user(&pool, "ada@acme.com", "h").await.unwrap();
        insert_user(&pool, "ben@acme.com", "h").await.unwrap();
        insert_user(&pool, "charlie@widgets.com", "h")
            .await
            .unwrap();
        let acme = list_users(&pool, "acme", 10, 0).await.unwrap();
        assert_eq!(acme.len(), 2);
        assert_eq!(count_users(&pool, "acme").await.unwrap(), 2);
        // Substring works mid-word too.
        let ada = list_users(&pool, "ada@", 10, 0).await.unwrap();
        assert_eq!(ada.len(), 1);
        assert_eq!(ada[0].email, "ada@acme.com");
    }

    #[tokio::test]
    async fn delete_removes_the_row() {
        let pool = fresh_pool().await;
        let u = insert_user(&pool, "ada@x.com", "h").await.unwrap();
        assert_eq!(delete_user(&pool, &u.id).await.unwrap(), 1);
        assert!(find_user_by_id(&pool, &u.id).await.unwrap().is_none());
        // Idempotent — deleting twice doesn't blow up.
        assert_eq!(delete_user(&pool, &u.id).await.unwrap(), 0);
    }
}
