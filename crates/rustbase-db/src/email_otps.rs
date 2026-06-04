//! One-time email codes for passwordless / second-factor login.
//!
//! Differs from `email_verifications` / `password_resets` in three
//! ways that matter at the call-site:
//!
//! - Keyed by `email`, not `user_id`. OTP doubles as a signup channel
//!   so the user row may not exist yet when a code is issued.
//! - Numeric 6-digit codes, not opaque hex tokens. They're meant to
//!   be typed by hand from an email body.
//! - Bounded retries. Each wrong-code attempt against the *current*
//!   pending code increments `attempts`; past a fixed cap the code is
//!   marked consumed so a parallel guesser can't keep trying.
//!
//! Issuing a new code for an email atomically invalidates any prior
//! unconsumed codes for that address — single in-flight code per
//! email at any time.

use crate::error::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// Hard cap on wrong-code attempts before the row is force-consumed.
/// Five tries against a 6-digit code: 5 / 1_000_000 ≈ 5 ppm guess
/// budget, which is fine for the 10-minute TTL.
const MAX_ATTEMPTS: i64 = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct EmailOtp {
    pub id: i64,
    pub code: String,
    pub email: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
    pub attempts: i64,
}

/// Issue a new code for `email`. Invalidates any prior unconsumed
/// codes for the same address in the same transaction so only one
/// code is ever valid at a time. The caller supplies both the code
/// (so the generation policy can live above this module — 6 digits,
/// 8 digits, base32, whatever) and the TTL.
pub async fn issue(pool: &SqlitePool, code: &str, email: &str, ttl: Duration) -> Result<EmailOtp> {
    let issued_at = Utc::now();
    let expires_at = issued_at + ttl;

    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE _email_otps SET consumed_at = ? \
         WHERE email = ? AND consumed_at IS NULL",
    )
    .bind(issued_at)
    .bind(email)
    .execute(&mut *tx)
    .await?;

    let row: (i64,) = sqlx::query_as(
        "INSERT INTO _email_otps (code, email, issued_at, expires_at, consumed_at, attempts) \
         VALUES (?, ?, ?, ?, NULL, 0) RETURNING id",
    )
    .bind(code)
    .bind(email)
    .bind(issued_at)
    .bind(expires_at)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(EmailOtp {
        id: row.0,
        code: code.to_string(),
        email: email.to_string(),
        issued_at,
        expires_at,
        consumed_at: None,
        attempts: 0,
    })
}

/// The single currently-pending code for `email`, if any. Used by
/// tests and by `consume`.
pub async fn current(pool: &SqlitePool, email: &str) -> Result<Option<EmailOtp>> {
    let row: Option<EmailOtp> = sqlx::query_as(
        "SELECT id, code, email, issued_at, expires_at, consumed_at, attempts \
         FROM _email_otps \
         WHERE email = ? AND consumed_at IS NULL \
         ORDER BY issued_at DESC LIMIT 1",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConsumeOutcome {
    /// Successfully consumed. Caller should issue tokens for `email`
    /// (creating the user row if it doesn't exist yet).
    Ok { email: String },
    /// No pending code for this email.
    Unknown,
    /// Code value didn't match the pending row. `attempts_left` may
    /// hit `0`, in which case the next call will be `Locked`.
    WrongCode { attempts_left: i64 },
    /// Pending code exists but its expiry has passed.
    Expired,
    /// Pending code exists but has burned its attempt budget — row is
    /// now marked consumed_at, so retrying with the right code also
    /// fails. The caller must request a fresh code.
    Locked,
}

/// Atomically check the supplied `code` against the current pending
/// row for `email`. See `ConsumeOutcome` for the cases.
pub async fn consume(pool: &SqlitePool, email: &str, code: &str) -> Result<ConsumeOutcome> {
    let Some(row) = current(pool, email).await? else {
        return Ok(ConsumeOutcome::Unknown);
    };
    let now = Utc::now();
    if row.expires_at <= now {
        // Burn it so a delayed correct guess doesn't suddenly work.
        sqlx::query("UPDATE _email_otps SET consumed_at = ? WHERE id = ?")
            .bind(now)
            .bind(row.id)
            .execute(pool)
            .await?;
        return Ok(ConsumeOutcome::Expired);
    }
    if row.code == code {
        let updated = sqlx::query(
            "UPDATE _email_otps SET consumed_at = ? \
             WHERE id = ? AND consumed_at IS NULL",
        )
        .bind(now)
        .bind(row.id)
        .execute(pool)
        .await?;
        if updated.rows_affected() == 0 {
            // Lost the race against a parallel consumer.
            return Ok(ConsumeOutcome::Unknown);
        }
        return Ok(ConsumeOutcome::Ok { email: row.email });
    }
    // Wrong code path: bump attempts, lock when over budget.
    let new_attempts = row.attempts + 1;
    if new_attempts >= MAX_ATTEMPTS {
        sqlx::query(
            "UPDATE _email_otps SET attempts = ?, consumed_at = ? \
             WHERE id = ?",
        )
        .bind(new_attempts)
        .bind(now)
        .bind(row.id)
        .execute(pool)
        .await?;
        Ok(ConsumeOutcome::Locked)
    } else {
        sqlx::query("UPDATE _email_otps SET attempts = ? WHERE id = ?")
            .bind(new_attempts)
            .bind(row.id)
            .execute(pool)
            .await?;
        Ok(ConsumeOutcome::WrongCode {
            attempts_left: MAX_ATTEMPTS - new_attempts,
        })
    }
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

    #[tokio::test]
    async fn issue_then_consume_correct_code_returns_ok() {
        let pool = fresh().await;
        issue(&pool, "123456", "ada@x.com", Duration::minutes(10))
            .await
            .unwrap();
        let out = consume(&pool, "ada@x.com", "123456").await.unwrap();
        assert_eq!(
            out,
            ConsumeOutcome::Ok {
                email: "ada@x.com".into()
            }
        );
    }

    #[tokio::test]
    async fn wrong_code_decrements_attempts_then_locks() {
        let pool = fresh().await;
        issue(&pool, "111111", "ada@x.com", Duration::minutes(10))
            .await
            .unwrap();
        for expected_left in (0..MAX_ATTEMPTS).rev() {
            let out = consume(&pool, "ada@x.com", "999999").await.unwrap();
            match (expected_left, &out) {
                (0, ConsumeOutcome::Locked) => {}
                (n, ConsumeOutcome::WrongCode { attempts_left }) if *attempts_left == n => {}
                _ => panic!("unexpected at attempts_left={expected_left}: {out:?}"),
            }
        }
        // Even the correct code can't save it now.
        assert_eq!(
            consume(&pool, "ada@x.com", "111111").await.unwrap(),
            ConsumeOutcome::Unknown // current() finds no pending row
        );
    }

    #[tokio::test]
    async fn issuing_new_code_invalidates_prior_one() {
        let pool = fresh().await;
        issue(&pool, "111111", "ada@x.com", Duration::minutes(10))
            .await
            .unwrap();
        issue(&pool, "222222", "ada@x.com", Duration::minutes(10))
            .await
            .unwrap();
        // Old code no longer works.
        assert_eq!(
            consume(&pool, "ada@x.com", "111111").await.unwrap(),
            ConsumeOutcome::WrongCode {
                attempts_left: MAX_ATTEMPTS - 1
            }
        );
        // Fresh code does.
        assert_eq!(
            consume(&pool, "ada@x.com", "222222").await.unwrap(),
            ConsumeOutcome::Ok {
                email: "ada@x.com".into()
            }
        );
    }

    #[tokio::test]
    async fn expired_code_returns_expired_and_is_consumed() {
        let pool = fresh().await;
        issue(&pool, "555555", "ada@x.com", Duration::seconds(-1))
            .await
            .unwrap();
        assert_eq!(
            consume(&pool, "ada@x.com", "555555").await.unwrap(),
            ConsumeOutcome::Expired
        );
        // Burned: not retryable.
        assert_eq!(
            consume(&pool, "ada@x.com", "555555").await.unwrap(),
            ConsumeOutcome::Unknown
        );
    }

    #[tokio::test]
    async fn unknown_email_returns_unknown() {
        let pool = fresh().await;
        assert_eq!(
            consume(&pool, "ghost@nowhere", "123456").await.unwrap(),
            ConsumeOutcome::Unknown
        );
    }

    #[tokio::test]
    async fn second_consume_after_success_returns_unknown() {
        let pool = fresh().await;
        issue(&pool, "424242", "ada@x.com", Duration::minutes(10))
            .await
            .unwrap();
        consume(&pool, "ada@x.com", "424242").await.unwrap();
        assert_eq!(
            consume(&pool, "ada@x.com", "424242").await.unwrap(),
            ConsumeOutcome::Unknown
        );
    }
}
