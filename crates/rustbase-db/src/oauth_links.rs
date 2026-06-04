//! Mapping from `(provider, provider_user_id)` to a local user.
//!
//! Backed by the `user_oauth_links` table that the workspace initial
//! migration provisioned. A user can link multiple providers (one
//! row per provider); a single provider account only links to one
//! user (enforced by the `(user_id, provider)` PK plus the
//! application logic — we look up by (provider, provider_user_id)
//! before inserting).

use crate::error::Result;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct OAuthLink {
    pub user_id: String,
    pub provider: String,
    pub provider_user_id: String,
}

/// Idempotent link creation. Re-linking the same (user, provider)
/// pair to a new provider_user_id replaces the prior value — useful
/// if a provider rotates account ids.
pub async fn upsert_link(
    pool: &SqlitePool,
    user_id: &str,
    provider: &str,
    provider_user_id: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO user_oauth_links (user_id, provider, provider_user_id) \
         VALUES (?, ?, ?) \
         ON CONFLICT(user_id, provider) DO UPDATE SET \
             provider_user_id = excluded.provider_user_id",
    )
    .bind(user_id)
    .bind(provider)
    .bind(provider_user_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Find the local user already linked to this provider account, if
/// any. The callback path uses this to decide between login (link
/// hit) and signup (miss).
pub async fn find_by_provider_user(
    pool: &SqlitePool,
    provider: &str,
    provider_user_id: &str,
) -> Result<Option<OAuthLink>> {
    let row: Option<OAuthLink> = sqlx::query_as(
        "SELECT user_id, provider, provider_user_id \
         FROM user_oauth_links \
         WHERE provider = ? AND provider_user_id = ?",
    )
    .bind(provider)
    .bind(provider_user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Every provider account linked to `user_id`. The admin user-detail
/// page lists them so an operator can see which OAuth identities a
/// given user signed in with. Ordered by provider name for a stable
/// row order.
pub async fn list_for_user(pool: &SqlitePool, user_id: &str) -> Result<Vec<OAuthLink>> {
    let rows: Vec<OAuthLink> = sqlx::query_as(
        "SELECT user_id, provider, provider_user_id \
         FROM user_oauth_links \
         WHERE user_id = ? \
         ORDER BY provider ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::{APP_MIGRATIONS, apply_migrations};
    use crate::pool::open_memory_pool;
    use crate::users::insert_passwordless_user;

    async fn fresh() -> (SqlitePool, String) {
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(pool.clone(), APP_MIGRATIONS)
            .await
            .unwrap();
        let user = insert_passwordless_user(&pool, "ada@x.com").await.unwrap();
        (pool, user.id)
    }

    #[tokio::test]
    async fn upsert_then_find_round_trips() {
        let (pool, user_id) = fresh().await;
        upsert_link(&pool, &user_id, "google", "google-sub-1")
            .await
            .unwrap();
        let link = find_by_provider_user(&pool, "google", "google-sub-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(link.user_id, user_id);
    }

    #[tokio::test]
    async fn relink_replaces_provider_user_id() {
        let (pool, user_id) = fresh().await;
        upsert_link(&pool, &user_id, "google", "old-sub")
            .await
            .unwrap();
        upsert_link(&pool, &user_id, "google", "new-sub")
            .await
            .unwrap();
        assert!(
            find_by_provider_user(&pool, "google", "old-sub")
                .await
                .unwrap()
                .is_none()
        );
        let link = find_by_provider_user(&pool, "google", "new-sub")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(link.user_id, user_id);
    }

    #[tokio::test]
    async fn miss_returns_none() {
        let (pool, _user_id) = fresh().await;
        assert!(
            find_by_provider_user(&pool, "google", "ghost")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn list_for_user_orders_by_provider() {
        let (pool, user_id) = fresh().await;
        upsert_link(&pool, &user_id, "google", "g-1").await.unwrap();
        upsert_link(&pool, &user_id, "github", "h-1").await.unwrap();
        let links = list_for_user(&pool, &user_id).await.unwrap();
        let providers: Vec<_> = links.iter().map(|l| l.provider.as_str()).collect();
        assert_eq!(providers, vec!["github", "google"]);
    }

    #[tokio::test]
    async fn list_for_user_returns_empty_when_unlinked() {
        let (pool, user_id) = fresh().await;
        assert!(list_for_user(&pool, &user_id).await.unwrap().is_empty());
    }
}
