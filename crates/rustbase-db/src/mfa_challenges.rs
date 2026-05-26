//! Short-lived MFA challenges issued by the password-login endpoint.
//!
//! When a user has TOTP enabled, `POST /auth/users/login` doesn't
//! return access/refresh tokens directly. Instead it stores a fresh
//! random token here bound to the user, and returns it to the client
//! as `mfa_token`. The client then POSTs `(mfa_token, totp_code)` to
//! `/auth/users/login/totp`, which consumes the challenge and issues
//! the real tokens.
//!
//! Single-use, TTL ~5 minutes. Same race-resilient `consume`
//! semantics as the email-verification / password-reset tables.

use crate::error::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct MfaChallenge {
    pub token: String,
    pub user_id: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
}

pub async fn issue(
    pool: &SqlitePool,
    token: &str,
    user_id: &str,
    ttl: Duration,
) -> Result<MfaChallenge> {
    let issued_at = Utc::now();
    let expires_at = issued_at + ttl;
    sqlx::query(
        "INSERT INTO _mfa_challenges (token, user_id, issued_at, expires_at, consumed_at) \
         VALUES (?, ?, ?, ?, NULL)",
    )
    .bind(token)
    .bind(user_id)
    .bind(issued_at)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(MfaChallenge {
        token: token.into(),
        user_id: user_id.into(),
        issued_at,
        expires_at,
        consumed_at: None,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConsumeOutcome {
    Ok { user_id: String },
    Unknown,
    AlreadyConsumed,
    Expired,
}

pub async fn consume(pool: &SqlitePool, token: &str) -> Result<ConsumeOutcome> {
    let row: Option<MfaChallenge> = sqlx::query_as(
        "SELECT token, user_id, issued_at, expires_at, consumed_at \
         FROM _mfa_challenges WHERE token = ?",
    )
    .bind(token)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(ConsumeOutcome::Unknown);
    };
    if row.consumed_at.is_some() {
        return Ok(ConsumeOutcome::AlreadyConsumed);
    }
    let now = Utc::now();
    if row.expires_at <= now {
        return Ok(ConsumeOutcome::Expired);
    }
    let updated = sqlx::query(
        "UPDATE _mfa_challenges SET consumed_at = ? \
         WHERE token = ? AND consumed_at IS NULL",
    )
    .bind(now)
    .bind(token)
    .execute(pool)
    .await?;
    if updated.rows_affected() == 0 {
        return Ok(ConsumeOutcome::AlreadyConsumed);
    }
    Ok(ConsumeOutcome::Ok {
        user_id: row.user_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::{REALM_MIGRATIONS, apply_migrations};
    use crate::pool::open_memory_pool;
    use crate::users::insert_user;

    async fn fresh() -> (SqlitePool, String) {
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(pool.clone(), REALM_MIGRATIONS)
            .await
            .unwrap();
        let u = insert_user(&pool, "ada@x.com", "hash").await.unwrap();
        (pool, u.id)
    }

    #[tokio::test]
    async fn issue_then_consume_returns_user_id() {
        let (pool, uid) = fresh().await;
        issue(&pool, "tok-1", &uid, Duration::minutes(5))
            .await
            .unwrap();
        match consume(&pool, "tok-1").await.unwrap() {
            ConsumeOutcome::Ok { user_id } => assert_eq!(user_id, uid),
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn replay_returns_already_consumed() {
        let (pool, uid) = fresh().await;
        issue(&pool, "tok-2", &uid, Duration::minutes(5))
            .await
            .unwrap();
        consume(&pool, "tok-2").await.unwrap();
        assert_eq!(
            consume(&pool, "tok-2").await.unwrap(),
            ConsumeOutcome::AlreadyConsumed
        );
    }

    #[tokio::test]
    async fn expired_returns_expired() {
        let (pool, uid) = fresh().await;
        issue(&pool, "tok-3", &uid, Duration::seconds(-1))
            .await
            .unwrap();
        assert_eq!(
            consume(&pool, "tok-3").await.unwrap(),
            ConsumeOutcome::Expired
        );
    }

    #[tokio::test]
    async fn unknown_returns_unknown() {
        let (pool, _uid) = fresh().await;
        assert_eq!(
            consume(&pool, "nope").await.unwrap(),
            ConsumeOutcome::Unknown
        );
    }
}
