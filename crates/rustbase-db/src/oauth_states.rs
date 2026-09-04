//! CSRF state nonces for the OAuth2 authorization code flow.
//!
//! Issued by `/authorize`, consumed at `/callback`. Bound to the
//! provider name so a state minted for `google` can't redeem a code
//! returned via the `github` callback. Short TTL (caller passes; the
//! API layer uses 5 min) — long enough for the user to finish the
//! upstream consent screen, short enough that a leaked state nonce
//! is mostly useless by the time anyone notices.

use crate::error::{DbError, Result};
use crate::secret_at_rest::StoredSecret;
use crate::tokens::hash_token;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct OAuthState {
    pub state_hash: String,
    pub provider: String,
    pub redirect_uri: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
    /// PKCE `code_verifier` (RFC 7636) persisted at `/authorize` and
    /// replayed to the token endpoint at `/callback`. Optional only
    /// for legacy rows that predate the 0.2 PKCE pack — new flows
    /// always populate it.
    pub code_verifier: StoredSecret,
}

pub async fn issue(
    pool: &SqlitePool,
    state: &str,
    provider: &str,
    redirect_uri: &str,
    code_verifier: &StoredSecret,
    ttl: Duration,
) -> Result<OAuthState> {
    let issued_at = Utc::now();
    let expires_at = issued_at + ttl;
    let (clear_cv, enc_cv) = code_verifier.columns();
    sqlx::query(
        "INSERT INTO _oauth_states \
            (state_hash, provider, redirect_uri, issued_at, expires_at, consumed_at, \
             code_verifier, code_verifier_enc) \
         VALUES (?, ?, ?, ?, ?, NULL, ?, ?)",
    )
    .bind(hash_token(state))
    .bind(provider)
    .bind(redirect_uri)
    .bind(issued_at)
    .bind(expires_at)
    .bind(clear_cv)
    .bind(enc_cv)
    .execute(pool)
    .await?;
    Ok(OAuthState {
        state_hash: hash_token(state),
        provider: provider.into(),
        redirect_uri: redirect_uri.into(),
        issued_at,
        expires_at,
        consumed_at: None,
        code_verifier: code_verifier.clone(),
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConsumeOutcome {
    /// Successfully consumed. Caller proceeds to the token exchange
    /// using `redirect_uri` and replays `code_verifier` to the
    /// provider's `/token` endpoint when present.
    Ok {
        redirect_uri: String,
        code_verifier: StoredSecret,
    },
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

type StateRow = (
    String,
    String,
    String,
    DateTime<Utc>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<String>,
    Option<Vec<u8>>,
);

pub async fn consume(pool: &SqlitePool, state: &str, provider: &str) -> Result<ConsumeOutcome> {
    let found: Option<StateRow> = sqlx::query_as(
        "SELECT state_hash, provider, redirect_uri, issued_at, expires_at, consumed_at, \
                code_verifier, code_verifier_enc \
         FROM _oauth_states WHERE state_hash = ?",
    )
    .bind(hash_token(state))
    .fetch_optional(pool)
    .await?;
    let Some((state_hash, prov, redirect_uri, issued_at, expires_at, consumed_at, cv, cv_enc)) =
        found
    else {
        return Ok(ConsumeOutcome::Unknown);
    };
    let code_verifier = StoredSecret::from_columns(cv, cv_enc).ok_or_else(|| {
        DbError::Invariant(format!(
            "_oauth_states row {state_hash} holds neither exactly one verifier nor the other"
        ))
    })?;
    let row = OAuthState {
        state_hash,
        provider: prov,
        redirect_uri,
        issued_at,
        expires_at,
        consumed_at,
        code_verifier,
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
         WHERE state_hash = ? AND consumed_at IS NULL",
    )
    .bind(now)
    .bind(hash_token(state))
    .execute(pool)
    .await?;
    if updated.rows_affected() == 0 {
        return Ok(ConsumeOutcome::AlreadyConsumed);
    }
    Ok(ConsumeOutcome::Ok {
        redirect_uri: row.redirect_uri,
        code_verifier: row.code_verifier,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::{WORKSPACE_MIGRATIONS, apply_migrations};
    use crate::pool::open_memory_pool;

    async fn fresh() -> SqlitePool {
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(pool.clone(), WORKSPACE_MIGRATIONS)
            .await
            .unwrap();
        pool
    }

    /// The verifier is the last secret that has to survive a round
    /// trip rather than be compared, so it is encrypted rather than
    /// hashed. Encrypted, the clear column must stay empty — a copy
    /// left behind would defeat the whole point.
    #[tokio::test]
    async fn an_encrypted_verifier_leaves_no_clear_copy_behind() {
        let pool = fresh().await;
        issue(
            &pool,
            "st",
            "google",
            "https://app/cb",
            &StoredSecret::Encrypted(vec![9, 9, 9]),
            Duration::minutes(10),
        )
        .await
        .unwrap();

        let (clear, enc): (Option<String>, Option<Vec<u8>>) =
            sqlx::query_as("SELECT code_verifier, code_verifier_enc FROM _oauth_states")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            clear, None,
            "a clear verifier was kept next to the ciphertext"
        );
        assert_eq!(enc, Some(vec![9, 9, 9]));

        match consume(&pool, "st", "google").await.unwrap() {
            ConsumeOutcome::Ok { code_verifier, .. } => {
                assert_eq!(code_verifier, StoredSecret::Encrypted(vec![9, 9, 9]));
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn state_nonce_never_lands_on_disk_in_clear() {
        let pool = fresh().await;
        issue(
            &pool,
            "st_clear",
            "google",
            "https://app/cb",
            &StoredSecret::Clear("verifier".into()),
            Duration::minutes(10),
        )
        .await
        .unwrap();
        let stored: String = sqlx::query_scalar("SELECT state_hash FROM _oauth_states")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_ne!(stored, "st_clear");
        assert_eq!(stored, crate::tokens::hash_token("st_clear"));
        assert!(matches!(
            consume(&pool, "st_clear", "google").await.unwrap(),
            ConsumeOutcome::Ok { .. }
        ));
    }

    #[tokio::test]
    async fn issue_then_consume_returns_redirect_uri() {
        let pool = fresh().await;
        issue(
            &pool,
            "s-1",
            "google",
            "https://app/cb",
            &StoredSecret::Clear("verifier-1".into()),
            Duration::minutes(5),
        )
        .await
        .unwrap();
        match consume(&pool, "s-1", "google").await.unwrap() {
            ConsumeOutcome::Ok {
                redirect_uri,
                code_verifier,
            } => {
                assert_eq!(redirect_uri, "https://app/cb");
                assert_eq!(code_verifier, StoredSecret::Clear("verifier-1".into()));
            }
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
            &StoredSecret::Clear("v".into()),
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
            &StoredSecret::Clear("v".into()),
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
            &StoredSecret::Clear("v".into()),
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
