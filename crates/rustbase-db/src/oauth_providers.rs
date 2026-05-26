//! Per-realm OAuth2 provider configuration.
//!
//! The realm DB already ships an `oauth_providers` table from the
//! initial migration; this module is the typed CRUD layer over it.
//! Each row stores a provider's client identity (id + secret) and a
//! JSON blob of "everything else" — auth URL, token URL, userinfo
//! URL, scopes, and which JSON paths to read for the user id + email
//! out of the userinfo response.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// Full provider config — what the API layer needs to drive an
/// authorization-code flow against the upstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthProvider {
    /// Slug used in URLs and tokens (e.g. "google", "github").
    pub provider: String,
    pub client_id: String,
    /// Stored as-is for the moment; real deployments should swap the
    /// column to an encrypted variant before exposing the admin UI.
    pub client_secret: String,
    pub config: OAuthProviderConfig,
}

/// JSON-serialised companion of `OAuthProvider`. Kept in a separate
/// struct so the row layout (id/secret as plain columns) and the
/// extensible config blob stay decoupled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthProviderConfig {
    /// Where to send the user agent for authorization.
    pub auth_url: String,
    /// Where the server POSTs the auth code to exchange for tokens.
    pub token_url: String,
    /// Where to GET the user profile with the access token in
    /// `Authorization: Bearer …`.
    pub userinfo_url: String,
    /// Scopes requested at authorize time (e.g. `["openid","email"]`).
    #[serde(default)]
    pub scopes: Vec<String>,
    /// JSON pointer (RFC 6901, leading slash, dots optional) into the
    /// userinfo body for the stable per-user id. Most providers
    /// use `/sub` (OIDC) or `/id` (GitHub). Defaults to `/sub`.
    #[serde(default = "default_id_field")]
    pub userinfo_id_field: String,
    /// JSON pointer into the userinfo body for the verified email
    /// address. Most providers use `/email`.
    #[serde(default = "default_email_field")]
    pub userinfo_email_field: String,
}

fn default_id_field() -> String {
    "/sub".into()
}
fn default_email_field() -> String {
    "/email".into()
}

pub async fn upsert_provider(pool: &SqlitePool, p: &OAuthProvider) -> Result<()> {
    let config_json = serde_json::to_string(&p.config)
        .map_err(|e| crate::error::DbError::InvalidIdentifier(format!("config: {e}")))?;
    sqlx::query(
        "INSERT INTO oauth_providers (provider, client_id, client_secret_enc, config_json) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT(provider) DO UPDATE SET \
             client_id = excluded.client_id, \
             client_secret_enc = excluded.client_secret_enc, \
             config_json = excluded.config_json",
    )
    .bind(&p.provider)
    .bind(&p.client_id)
    .bind(&p.client_secret)
    .bind(&config_json)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn find_provider(pool: &SqlitePool, provider: &str) -> Result<Option<OAuthProvider>> {
    let row: Option<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT provider, client_id, client_secret_enc, config_json \
         FROM oauth_providers WHERE provider = ?",
    )
    .bind(provider)
    .fetch_optional(pool)
    .await?;
    let Some((provider, client_id, client_secret, config_json)) = row else {
        return Ok(None);
    };
    let config: OAuthProviderConfig = match config_json {
        Some(s) => serde_json::from_str(&s)
            .map_err(|e| crate::error::DbError::InvalidIdentifier(format!("config: {e}")))?,
        // Defensive: a row with NULL config_json shouldn't happen,
        // but if a partial admin write left one, fall back to the
        // bare minimum that won't NPE downstream.
        None => OAuthProviderConfig {
            auth_url: String::new(),
            token_url: String::new(),
            userinfo_url: String::new(),
            scopes: Vec::new(),
            userinfo_id_field: default_id_field(),
            userinfo_email_field: default_email_field(),
        },
    };
    Ok(Some(OAuthProvider {
        provider,
        client_id,
        client_secret,
        config,
    }))
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

    fn sample() -> OAuthProvider {
        OAuthProvider {
            provider: "google".into(),
            client_id: "abc.apps.googleusercontent.com".into(),
            client_secret: "shh".into(),
            config: OAuthProviderConfig {
                auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
                token_url: "https://oauth2.googleapis.com/token".into(),
                userinfo_url: "https://openidconnect.googleapis.com/v1/userinfo".into(),
                scopes: vec!["openid".into(), "email".into()],
                userinfo_id_field: "/sub".into(),
                userinfo_email_field: "/email".into(),
            },
        }
    }

    #[tokio::test]
    async fn upsert_then_find_round_trips() {
        let pool = fresh().await;
        let p = sample();
        upsert_provider(&pool, &p).await.unwrap();
        let f = find_provider(&pool, "google").await.unwrap().unwrap();
        assert_eq!(f, p);
    }

    #[tokio::test]
    async fn upsert_replaces_existing_row() {
        let pool = fresh().await;
        upsert_provider(&pool, &sample()).await.unwrap();
        let mut updated = sample();
        updated.client_id = "new-id".into();
        upsert_provider(&pool, &updated).await.unwrap();
        let f = find_provider(&pool, "google").await.unwrap().unwrap();
        assert_eq!(f.client_id, "new-id");
    }

    #[tokio::test]
    async fn find_unknown_provider_returns_none() {
        let pool = fresh().await;
        assert!(find_provider(&pool, "github").await.unwrap().is_none());
    }
}
