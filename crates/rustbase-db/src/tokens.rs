//! Refresh-token storage.
//!
//! Refresh tokens live in `_refresh_tokens`. Both `system.db` and every
//! `workspace.db` have a table with the same schema; this module is generic
//! over the pool so the same code serves master and workspace scopes —
//! callers decide which pool to pass.

use crate::error::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubjectKind {
    MasterAdmin,
    WorkspaceAdmin,
    AppAdmin,
    User,
}

impl SubjectKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SubjectKind::MasterAdmin => "master_admin",
            SubjectKind::WorkspaceAdmin => "workspace_admin",
            SubjectKind::AppAdmin => "app_admin",
            SubjectKind::User => "user",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct RefreshToken {
    pub token: String,
    pub subject_kind: String,
    pub subject_id: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked: bool,
}

/// Insert a new refresh token. Callers supply the token string (opaque
/// random bytes from a secure RNG); this module does no entropy work.
pub async fn insert_refresh_token(
    pool: &SqlitePool,
    token: &str,
    subject_kind: SubjectKind,
    subject_id: &str,
    ttl: Duration,
) -> Result<RefreshToken> {
    let issued_at = Utc::now();
    let expires_at = issued_at + ttl;
    sqlx::query(
        "INSERT INTO _refresh_tokens (token, subject_kind, subject_id, issued_at, expires_at, revoked) \
         VALUES (?, ?, ?, ?, ?, 0)",
    )
    .bind(token)
    .bind(subject_kind.as_str())
    .bind(subject_id)
    .bind(issued_at)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(RefreshToken {
        token: token.to_string(),
        subject_kind: subject_kind.as_str().to_string(),
        subject_id: subject_id.to_string(),
        issued_at,
        expires_at,
        revoked: false,
    })
}

/// Look up an active (not revoked, not expired) refresh token of the
/// expected kind.
pub async fn find_active_refresh_token(
    pool: &SqlitePool,
    token: &str,
    expected_kind: SubjectKind,
) -> Result<Option<RefreshToken>> {
    let row: Option<RefreshToken> = sqlx::query_as(
        "SELECT token, subject_kind, subject_id, issued_at, expires_at, \
                CAST(revoked AS BOOLEAN) AS revoked \
         FROM _refresh_tokens \
         WHERE token = ? AND subject_kind = ? AND revoked = 0 AND expires_at > ?",
    )
    .bind(token)
    .bind(expected_kind.as_str())
    .bind(Utc::now())
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn revoke_refresh_token(pool: &SqlitePool, token: &str) -> Result<()> {
    sqlx::query("UPDATE _refresh_tokens SET revoked = 1 WHERE token = ?")
        .bind(token)
        .execute(pool)
        .await?;
    Ok(())
}

/// Atomic end-user login: bump `users.last_login` AND insert the
/// fresh refresh-token row in a single transaction. Cuts the
/// happy-path commit count from two to one — every successful user
/// login goes through this. Returns the newly issued row.
pub async fn commit_user_login(
    pool: &SqlitePool,
    user_id: &str,
    refresh_token: &str,
    ttl: Duration,
) -> Result<RefreshToken> {
    let now = Utc::now();
    let expires_at = now + ttl;
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE users SET last_login = ? WHERE id = ?")
        .bind(now)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO _refresh_tokens (token, subject_kind, subject_id, issued_at, expires_at, revoked) \
         VALUES (?, ?, ?, ?, ?, 0)",
    )
    .bind(refresh_token)
    .bind(SubjectKind::User.as_str())
    .bind(user_id)
    .bind(now)
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(RefreshToken {
        token: refresh_token.to_string(),
        subject_kind: SubjectKind::User.as_str().to_string(),
        subject_id: user_id.to_string(),
        issued_at: now,
        expires_at,
        revoked: false,
    })
}

/// Atomic rotate: revoke `old_token` and insert `new_token` in a single
/// transaction. Cuts the refresh path's commit count from two to one
/// (one fsync vs two under WAL + `synchronous=NORMAL`). Returns the
/// newly issued row.
pub async fn rotate_refresh_token(
    pool: &SqlitePool,
    old_token: &str,
    new_token: &str,
    subject_kind: SubjectKind,
    subject_id: &str,
    ttl: Duration,
) -> Result<RefreshToken> {
    let issued_at = Utc::now();
    let expires_at = issued_at + ttl;
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE _refresh_tokens SET revoked = 1 WHERE token = ?")
        .bind(old_token)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO _refresh_tokens (token, subject_kind, subject_id, issued_at, expires_at, revoked) \
         VALUES (?, ?, ?, ?, ?, 0)",
    )
    .bind(new_token)
    .bind(subject_kind.as_str())
    .bind(subject_id)
    .bind(issued_at)
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(RefreshToken {
        token: new_token.to_string(),
        subject_kind: subject_kind.as_str().to_string(),
        subject_id: subject_id.to_string(),
        issued_at,
        expires_at,
        revoked: false,
    })
}

/// Revoke every active refresh token for a subject. Useful for "log out
/// everywhere" and for cleaning up after a password reset.
pub async fn revoke_all_for_subject(
    pool: &SqlitePool,
    subject_kind: SubjectKind,
    subject_id: &str,
) -> Result<u64> {
    let res = sqlx::query(
        "UPDATE _refresh_tokens SET revoked = 1 \
         WHERE subject_kind = ? AND subject_id = ? AND revoked = 0",
    )
    .bind(subject_kind.as_str())
    .bind(subject_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
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
    async fn insert_then_find_active() {
        let pool = fresh_pool().await;
        insert_refresh_token(
            &pool,
            "rfsh_abc",
            SubjectKind::MasterAdmin,
            "admin-1",
            Duration::days(30),
        )
        .await
        .unwrap();
        let found = find_active_refresh_token(&pool, "rfsh_abc", SubjectKind::MasterAdmin)
            .await
            .unwrap();
        assert!(found.is_some());
    }

    #[tokio::test]
    async fn revoke_makes_it_inactive() {
        let pool = fresh_pool().await;
        insert_refresh_token(
            &pool,
            "rfsh_abc",
            SubjectKind::MasterAdmin,
            "admin-1",
            Duration::days(30),
        )
        .await
        .unwrap();
        revoke_refresh_token(&pool, "rfsh_abc").await.unwrap();
        let found = find_active_refresh_token(&pool, "rfsh_abc", SubjectKind::MasterAdmin)
            .await
            .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn kind_mismatch_returns_none() {
        let pool = fresh_pool().await;
        insert_refresh_token(
            &pool,
            "rfsh_abc",
            SubjectKind::MasterAdmin,
            "admin-1",
            Duration::days(30),
        )
        .await
        .unwrap();
        // looking for it as a user → no match
        let found = find_active_refresh_token(&pool, "rfsh_abc", SubjectKind::User)
            .await
            .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn expired_token_is_not_returned() {
        let pool = fresh_pool().await;
        insert_refresh_token(
            &pool,
            "rfsh_expired",
            SubjectKind::MasterAdmin,
            "admin-1",
            Duration::seconds(-60),
        )
        .await
        .unwrap();
        let found = find_active_refresh_token(&pool, "rfsh_expired", SubjectKind::MasterAdmin)
            .await
            .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn rotate_revokes_old_and_inserts_new_atomically() {
        let pool = fresh_pool().await;
        insert_refresh_token(
            &pool,
            "rfsh_old",
            SubjectKind::MasterAdmin,
            "admin-1",
            Duration::days(30),
        )
        .await
        .unwrap();
        let rotated = rotate_refresh_token(
            &pool,
            "rfsh_old",
            "rfsh_new",
            SubjectKind::MasterAdmin,
            "admin-1",
            Duration::days(30),
        )
        .await
        .unwrap();
        assert_eq!(rotated.token, "rfsh_new");
        // old token no longer active
        let old = find_active_refresh_token(&pool, "rfsh_old", SubjectKind::MasterAdmin)
            .await
            .unwrap();
        assert!(old.is_none());
        // new one is active
        let new = find_active_refresh_token(&pool, "rfsh_new", SubjectKind::MasterAdmin)
            .await
            .unwrap();
        assert!(new.is_some());
    }

    #[tokio::test]
    async fn commit_user_login_updates_last_login_and_inserts_refresh() {
        use crate::migrations::WORKSPACE_MIGRATIONS;
        use crate::users::insert_user;
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(pool.clone(), WORKSPACE_MIGRATIONS)
            .await
            .unwrap();
        let user = insert_user(&pool, "u@acme.com", "hash").await.unwrap();
        let before = user.last_login;

        let row = commit_user_login(&pool, &user.id, "rfsh_new", Duration::days(30))
            .await
            .unwrap();
        assert_eq!(row.token, "rfsh_new");
        assert_eq!(row.subject_kind, "user");
        assert_eq!(row.subject_id, user.id);

        // last_login is populated.
        let after = crate::users::find_user_by_id(&pool, &user.id)
            .await
            .unwrap()
            .unwrap();
        assert!(after.last_login.is_some());
        assert_ne!(after.last_login, before);

        // refresh token row is active.
        let found = find_active_refresh_token(&pool, "rfsh_new", SubjectKind::User)
            .await
            .unwrap();
        assert!(found.is_some());
    }

    #[tokio::test]
    async fn revoke_all_for_subject_counts_rows() {
        let pool = fresh_pool().await;
        for i in 0..3 {
            insert_refresh_token(
                &pool,
                &format!("rfsh_{i}"),
                SubjectKind::MasterAdmin,
                "admin-1",
                Duration::days(30),
            )
            .await
            .unwrap();
        }
        let revoked = revoke_all_for_subject(&pool, SubjectKind::MasterAdmin, "admin-1")
            .await
            .unwrap();
        assert_eq!(revoked, 3);
        // a second sweep finds nothing to revoke
        let revoked2 = revoke_all_for_subject(&pool, SubjectKind::MasterAdmin, "admin-1")
            .await
            .unwrap();
        assert_eq!(revoked2, 0);
    }
}
