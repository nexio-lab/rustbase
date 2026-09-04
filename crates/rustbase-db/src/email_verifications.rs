//! One-shot email-verification tokens.
//!
//! Lives in the workspace DB. A token is issued when the user asks to
//! verify their email, mailed to them, and presented back on the
//! confirm endpoint. Tokens carry an explicit TTL and a
//! `consumed_at` column — using one flips `consumed_at`, and a
//! later attempt with the same token is rejected.

use crate::error::Result;
use crate::tokens::hash_token;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct EmailVerification {
    pub token_hash: String,
    pub user_id: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
}

/// Issue a new verification token. The token string itself is supplied
/// by the caller (opaque random bytes from a secure RNG) — this module
/// does no entropy work.
pub async fn issue(
    pool: &SqlitePool,
    token: &str,
    user_id: &str,
    ttl: Duration,
) -> Result<EmailVerification> {
    let issued_at = Utc::now();
    let expires_at = issued_at + ttl;
    sqlx::query(
        "INSERT INTO _email_verifications (token_hash, user_id, issued_at, expires_at, consumed_at) \
         VALUES (?, ?, ?, ?, NULL)",
    )
    .bind(hash_token(token))
    .bind(user_id)
    .bind(issued_at)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(EmailVerification {
        token_hash: hash_token(token),
        user_id: user_id.to_string(),
        issued_at,
        expires_at,
        consumed_at: None,
    })
}

/// Read a token without mutating it. Used by tests + by the confirm
/// path before deciding to consume.
pub async fn find(pool: &SqlitePool, token: &str) -> Result<Option<EmailVerification>> {
    let row: Option<EmailVerification> = sqlx::query_as(
        "SELECT token_hash, user_id, issued_at, expires_at, consumed_at \
         FROM _email_verifications WHERE token_hash = ?",
    )
    .bind(hash_token(token))
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Outcome of attempting to redeem a token.
#[derive(Debug, PartialEq, Eq)]
pub enum ConsumeOutcome {
    /// Successfully consumed — caller should mark the user verified.
    Ok { user_id: String },
    /// No row with that token.
    Unknown,
    /// Token exists but was already used.
    AlreadyConsumed,
    /// Token exists, never consumed, but past expiry.
    Expired,
}

/// Atomically consume a token: succeeds iff the row exists, is
/// unconsumed, and not expired. On success, sets `consumed_at = now`
/// and returns the associated `user_id`.
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
        "UPDATE _email_verifications SET consumed_at = ? \
         WHERE token_hash = ? AND consumed_at IS NULL",
    )
    .bind(now)
    .bind(hash_token(token))
    .execute(pool)
    .await?;
    // If a racing call also tried to consume the same token, our
    // UPDATE matched zero rows — the other call won.
    if result.rows_affected() == 0 {
        return Ok(ConsumeOutcome::AlreadyConsumed);
    }
    Ok(ConsumeOutcome::Ok {
        user_id: row.user_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::{WORKSPACE_MIGRATIONS, apply_migrations};
    use crate::pool::open_memory_pool;
    use crate::users::insert_user;

    async fn setup() -> (SqlitePool, String) {
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(pool.clone(), WORKSPACE_MIGRATIONS)
            .await
            .unwrap();
        let user = insert_user(&pool, "ada@x.com", "hash").await.unwrap();
        (pool, user.id)
    }

    #[tokio::test]
    async fn verification_token_never_lands_on_disk_in_clear() {
        let (pool, user_id) = setup().await;
        issue(&pool, "vrfy_clear", &user_id, Duration::hours(1))
            .await
            .unwrap();
        let stored: String = sqlx::query_scalar("SELECT token_hash FROM _email_verifications")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_ne!(stored, "vrfy_clear");
        assert_eq!(stored, crate::tokens::hash_token("vrfy_clear"));
        assert!(find(&pool, "vrfy_clear").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn issue_then_consume_marks_consumed_and_returns_user() {
        let (pool, user_id) = setup().await;
        issue(&pool, "tok-1", &user_id, Duration::hours(1))
            .await
            .unwrap();
        match consume(&pool, "tok-1").await.unwrap() {
            ConsumeOutcome::Ok { user_id: out } => assert_eq!(out, user_id),
            other => panic!("expected Ok, got {other:?}"),
        }
        let row = find(&pool, "tok-1").await.unwrap().unwrap();
        assert!(row.consumed_at.is_some());
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
    async fn expired_token_returns_expired_and_is_not_marked_consumed() {
        let (pool, user_id) = setup().await;
        // Issue with a NEGATIVE TTL so the row is already expired.
        issue(&pool, "tok-exp", &user_id, Duration::seconds(-1))
            .await
            .unwrap();
        assert_eq!(
            consume(&pool, "tok-exp").await.unwrap(),
            ConsumeOutcome::Expired
        );
        let row = find(&pool, "tok-exp").await.unwrap().unwrap();
        assert!(row.consumed_at.is_none());
    }
}
