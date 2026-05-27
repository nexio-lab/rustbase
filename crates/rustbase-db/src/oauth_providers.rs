//! Per-realm OAuth2 provider configuration.
//!
//! The realm DB already ships an `oauth_providers` table from the
//! initial migration; this module is the typed CRUD layer over it.
//! Each row stores a provider's client identity (id + secret) and a
//! JSON blob of "everything else" — auth URL, token URL, userinfo
//! URL, scopes, and which JSON paths to read for the user id + email
//! out of the userinfo response.
//!
//! **The client secret is opaque to this layer**: `secret_enc` is a
//! `Vec<u8>` ciphertext. Encryption / decryption happens at the API
//! boundary, where the KEK persisted in
//! `system.db._secrets.oauth_kek` is in scope. Keeps the DB layer
//! crypto-free and the trust boundary obvious.

use crate::error::{DbError, Result};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// Full provider record as the API layer hands it in/out — secret
/// here is the *ciphertext*, not the plaintext. Decryption is the
/// caller's responsibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthProvider {
    /// Slug used in URLs and tokens (e.g. "google", "github").
    pub provider: String,
    pub client_id: String,
    /// Encrypted client_secret bytes. Opaque to this module.
    pub secret_enc: Vec<u8>,
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
        .map_err(|e| DbError::InvalidIdentifier(format!("config: {e}")))?;
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
    .bind(&p.secret_enc)
    .bind(&config_json)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn find_provider(pool: &SqlitePool, provider: &str) -> Result<Option<OAuthProvider>> {
    let row: Option<(String, String, Vec<u8>, Option<String>)> = sqlx::query_as(
        "SELECT provider, client_id, client_secret_enc, config_json \
         FROM oauth_providers WHERE provider = ?",
    )
    .bind(provider)
    .fetch_optional(pool)
    .await?;
    let Some((provider, client_id, secret_enc, config_json)) = row else {
        return Ok(None);
    };
    let config = decode_config(config_json)?;
    Ok(Some(OAuthProvider {
        provider,
        client_id,
        secret_enc,
        config,
    }))
}

/// Public-facing provider summary — never carries the secret bytes.
/// The admin-list endpoint returns these; the callback path uses
/// `find_provider` instead because it needs the ciphertext to decrypt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthProviderSummary {
    pub provider: String,
    pub client_id: String,
    pub config: OAuthProviderConfig,
}

pub async fn list_providers(pool: &SqlitePool) -> Result<Vec<OAuthProviderSummary>> {
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT provider, client_id, config_json FROM oauth_providers ORDER BY provider ASC",
    )
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (provider, client_id, config_json) in rows {
        out.push(OAuthProviderSummary {
            provider,
            client_id,
            config: decode_config(config_json)?,
        });
    }
    Ok(out)
}

pub async fn delete_provider(pool: &SqlitePool, provider: &str) -> Result<u64> {
    let res = sqlx::query("DELETE FROM oauth_providers WHERE provider = ?")
        .bind(provider)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

fn decode_config(config_json: Option<String>) -> Result<OAuthProviderConfig> {
    Ok(match config_json {
        Some(s) => serde_json::from_str(&s)
            .map_err(|e| DbError::InvalidIdentifier(format!("config: {e}")))?,
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

    fn sample() -> OAuthProvider {
        OAuthProvider {
            provider: "google".into(),
            client_id: "abc.apps.googleusercontent.com".into(),
            // Tests use placeholder ciphertext — real callers would
            // produce this via rustbase_auth::encrypt.
            secret_enc: b"ciphertext-bytes".to_vec(),
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

    #[tokio::test]
    async fn list_returns_summaries_in_provider_order() {
        let pool = fresh().await;
        let mut a = sample();
        a.provider = "github".into();
        let mut b = sample();
        b.provider = "google".into();
        upsert_provider(&pool, &b).await.unwrap();
        upsert_provider(&pool, &a).await.unwrap();
        let list = list_providers(&pool).await.unwrap();
        let names: Vec<_> = list.iter().map(|s| s.provider.as_str()).collect();
        assert_eq!(names, vec!["github", "google"]);
    }

    #[tokio::test]
    async fn delete_removes_the_row() {
        let pool = fresh().await;
        upsert_provider(&pool, &sample()).await.unwrap();
        assert_eq!(delete_provider(&pool, "google").await.unwrap(), 1);
        assert!(find_provider(&pool, "google").await.unwrap().is_none());
        assert_eq!(delete_provider(&pool, "google").await.unwrap(), 0);
    }
}
