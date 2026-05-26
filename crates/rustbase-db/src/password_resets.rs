//! One-shot password-reset tokens.
//!
//! Mirror of `email_verifications` for the "I forgot my password"
//! flow. Lives in the realm DB. Token is issued when the user asks
//! to reset, mailed to the address on file, and presented back on
//! the confirm endpoint along with the new password. The atomic
//! consume() machinery prevents replay.

use crate::error::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct PasswordReset {
    pub token: String,
    pub user_id: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
}

/// Issue a new reset token. The token string is supplied by the
/// caller (opaque random bytes from a secure RNG).
pub async fn issue(
    pool: &SqlitePool,
    token: &str,
    user_id: &str,
    ttl: Duration,
) -> Result<PasswordReset> {
    let issued_at = Utc::now();
    let expires_at = issued_at + ttl;
    sqlx::query(
        "INSERT INTO _password_resets (token, user_id, issued_at, expires_at, consumed_at) \
         VALUES (?, ?, ?, ?, NULL)",
    )
    .bind(token)
    .bind(user_id)
    .bind(issued_at)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(PasswordReset {
        token: token.to_string(),
        user_id: user_id.to_string(),
        issued_at,
        expires_at,
        consumed_at: None,
    })
}

pub async fn find(pool: &SqlitePool, token: &str) -> Result<Option<PasswordReset>> {
    let row: Option<PasswordReset> = sqlx::query_as(
        "SELECT token, user_id, issued_at, expires_at, consumed_at \
         FROM _password_resets WHERE token = ?",
    )
    .bind(token)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConsumeOutcome {
    /// Successfully consumed — caller should set the new password.
    Ok {
        user_id: String,
    },
    Unknown,
    AlreadyConsumed,
    Expired,
}

/// Atomically consume a token. Same race-resilient semantics as the
/// email-verification consume.
pub async fn consume(pool: &SqlitePool, token: &str) -> Result<ConsumeOutcome> {
    let Some(row) = find(pool, token).await? else {
        return Ok(ConsumeOutcome::Unknown);
    };
    if row.consumed_at.is_some() {
        return Ok(ConsumeOutcome::AlreadyConsumed);
    }
    let now = Utc::now();
    if row.expires_at <= now {
        return Ok(ConsumeOutcome::Expired);
    }
    let result = sqlx::query(
        "UPDATE _password_resets SET consumed_at = ? \
         WHERE token = ? AND consumed_at IS NULL",
    )
    .bind(now)
    .bind(token)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Ok(ConsumeOutcome::AlreadyConsumed);
    }
    Ok(ConsumeOutcome::Ok {
        user_id: row.user_id,
    })
}

/// Drop every unconsumed token for `user_id`. Called from the confirm
/// path after a successful password change so that any other pending
/// reset requests for the same user are invalidated immediately —
/// reduces the window where a stolen alternate token could be used.
pub async fn invalidate_all_for_user(pool: &SqlitePool, user_id: &str) -> Result<u64> {
    let res = sqlx::query(
        "UPDATE _password_resets SET consumed_at = ? \
         WHERE user_id = ? AND consumed_at IS NULL",
    )
    .bind(Utc::now())
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::{REALM_MIGRATIONS, apply_migrations};
    use crate::pool::open_memory_pool;
    use crate::users::insert_user;

    async fn setup() -> (SqlitePool, String) {
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(pool.clone(), REALM_MIGRATIONS)
            .await
            .unwrap();
        let user = insert_user(&pool, "ada@x.com", "hash").await.unwrap();
        (pool, user.id)
    }

    #[tokio::test]
    async fn issue_then_consume_returns_user_id() {
        let (pool, user_id) = setup().await;
        issue(&pool, "tok-1", &user_id, Duration::hours(1))
            .await
            .unwrap();
        match consume(&pool, "tok-1").await.unwrap() {
            ConsumeOutcome::Ok { user_id: out } => assert_eq!(out, user_id),
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn second_consume_returns_already_consumed() {
        let (pool, user_id) = setup().await;
        issue(&pool, "tok-2", &user_id, Duration::hours(1))
            .await
            .unwrap();
        consume(&pool, "tok-2").await.unwrap();
        assert_eq!(
            consume(&pool, "tok-2").await.unwrap(),
            ConsumeOutcome::AlreadyConsumed
        );
    }

    #[tokio::test]
    async fn unknown_token_returns_unknown() {
        let (pool, _user_id) = setup().await;
        assert_eq!(
            consume(&pool, "nope").await.unwrap(),
            ConsumeOutcome::Unknown
        );
    }

    #[tokio::test]
    async fn expired_token_returns_expired() {
        let (pool, user_id) = setup().await;
        issue(&pool, "tok-exp", &user_id, Duration::seconds(-1))
            .await
            .unwrap();
        assert_eq!(
            consume(&pool, "tok-exp").await.unwrap(),
            ConsumeOutcome::Expired
        );
    }

    #[tokio::test]
    async fn invalidate_all_marks_pending_consumed() {
        let (pool, user_id) = setup().await;
        issue(&pool, "tok-a", &user_id, Duration::hours(1))
            .await
            .unwrap();
        issue(&pool, "tok-b", &user_id, Duration::hours(1))
            .await
            .unwrap();
        let n = invalidate_all_for_user(&pool, &user_id).await.unwrap();
        assert_eq!(n, 2);
        // Both tokens now refuse to consume.
        assert_eq!(
            consume(&pool, "tok-a").await.unwrap(),
            ConsumeOutcome::AlreadyConsumed
        );
        assert_eq!(
            consume(&pool, "tok-b").await.unwrap(),
            ConsumeOutcome::AlreadyConsumed
        );
    }

    #[tokio::test]
    async fn invalidate_all_skips_already_consumed_rows() {
        let (pool, user_id) = setup().await;
        issue(&pool, "tok-used", &user_id, Duration::hours(1))
            .await
            .unwrap();
        consume(&pool, "tok-used").await.unwrap();
        // Nothing pending, so 0 rows updated.
        let n = invalidate_all_for_user(&pool, &user_id).await.unwrap();
        assert_eq!(n, 0);
    }
}
