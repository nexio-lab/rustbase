//! CSRF state nonces for the OAuth2 authorization code flow.
//!
//! Issued by `/authorize`, consumed at `/callback`. Bound to the
//! provider name so a state minted for `google` can't redeem a code
//! returned via the `github` callback. Short TTL (caller passes; the
//! API layer uses 5 min) — long enough for the user to finish the
//! upstream consent screen, short enough that a leaked state nonce
//! is mostly useless by the time anyone notices.

use crate::error::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct OAuthState {
    pub state: String,
    pub provider: String,
    pub redirect_uri: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
}

pub async fn issue(
    pool: &SqlitePool,
    state: &str,
    provider: &str,
    redirect_uri: &str,
    ttl: Duration,
) -> Result<OAuthState> {
    let issued_at = Utc::now();
    let expires_at = issued_at + ttl;
    sqlx::query(
        "INSERT INTO _oauth_states (state, provider, redirect_uri, issued_at, expires_at, consumed_at) \
         VALUES (?, ?, ?, ?, ?, NULL)",
    )
    .bind(state)
    .bind(provider)
    .bind(redirect_uri)
    .bind(issued_at)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(OAuthState {
        state: state.into(),
        provider: provider.into(),
        redirect_uri: redirect_uri.into(),
        issued_at,
        expires_at,
        consumed_at: None,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConsumeOutcome {
    /// Successfully consumed. Caller proceeds to the token exchange
    /// using `redirect_uri`.
    Ok { redirect_uri: String },
    /// State unknown — either was never issued, or is from a forged
    /// callback. Reject the request.
    Unknown,
    /// State already consumed (replay).
    AlreadyConsumed,
    /// State past its TTL.
    Expired,
    /// State exists but was minted for a different provider.
    ProviderMismatch,
}

pub async fn consume(pool: &SqlitePool, state: &str, provider: &str) -> Result<ConsumeOutcome> {
    let row: Option<OAuthState> = sqlx::query_as(
        "SELECT state, provider, redirect_uri, issued_at, expires_at, consumed_at \
         FROM _oauth_states WHERE state = ?",
    )
    .bind(state)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(ConsumeOutcome::Unknown);
    };
    if row.provider != provider {
        return Ok(ConsumeOutcome::ProviderMismatch);
    }
    if row.consumed_at.is_some() {
        return Ok(ConsumeOutcome::AlreadyConsumed);
    }
    let now = Utc::now();
    if row.expires_at <= now {
        return Ok(ConsumeOutcome::Expired);
    }
    let updated = sqlx::query(
        "UPDATE _oauth_states SET consumed_at = ? \
         WHERE state = ? AND consumed_at IS NULL",
    )
    .bind(now)
    .bind(state)
    .execute(pool)
    .await?;
    if updated.rows_affected() == 0 {
        return Ok(ConsumeOutcome::AlreadyConsumed);
    }
    Ok(ConsumeOutcome::Ok {
        redirect_uri: row.redirect_uri,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::{REALM_MIGRATIONS, apply_migrations};
    use crate::pool::open_memory_pool;

    async fn fresh() -> SqlitePool {
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(pool.clone(), REALM_MIGRATIONS)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn issue_then_consume_returns_redirect_uri() {
        let pool = fresh().await;
        issue(
            &pool,
            "s-1",
            "google",
            "https://app/cb",
            Duration::minutes(5),
        )
        .await
        .unwrap();
        match consume(&pool, "s-1", "google").await.unwrap() {
            ConsumeOutcome::Ok { redirect_uri } => assert_eq!(redirect_uri, "https://app/cb"),
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn replay_returns_already_consumed() {
        let pool = fresh().await;
        issue(
            &pool,
            "s-2",
            "google",
            "https://app/cb",
            Duration::minutes(5),
        )
        .await
        .unwrap();
        consume(&pool, "s-2", "google").await.unwrap();
        assert_eq!(
            consume(&pool, "s-2", "google").await.unwrap(),
            ConsumeOutcome::AlreadyConsumed
        );
    }

    #[tokio::test]
    async fn provider_mismatch_does_not_consume() {
        let pool = fresh().await;
        issue(
            &pool,
            "s-3",
            "google",
            "https://app/cb",
            Duration::minutes(5),
        )
        .await
        .unwrap();
        assert_eq!(
            consume(&pool, "s-3", "github").await.unwrap(),
            ConsumeOutcome::ProviderMismatch
        );
        // Right provider still works (the failed attempt was a no-op).
        assert!(matches!(
            consume(&pool, "s-3", "google").await.unwrap(),
            ConsumeOutcome::Ok { .. }
        ));
    }

    #[tokio::test]
    async fn expired_returns_expired() {
        let pool = fresh().await;
        issue(
            &pool,
            "s-4",
            "google",
            "https://app/cb",
            Duration::seconds(-1),
        )
        .await
        .unwrap();
        assert_eq!(
            consume(&pool, "s-4", "google").await.unwrap(),
            ConsumeOutcome::Expired
        );
    }

    #[tokio::test]
    async fn unknown_returns_unknown() {
        let pool = fresh().await;
        assert_eq!(
            consume(&pool, "nope", "google").await.unwrap(),
            ConsumeOutcome::Unknown
        );
    }
}
