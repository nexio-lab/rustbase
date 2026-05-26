//! OAuth2 authorization-code sign-in.
//!
//! Two endpoints, both anonymous:
//!
//! - `GET  /api/realms/:realm/auth/oauth/:provider/authorize?redirect_uri=...`
//!   Mints a fresh CSRF state nonce, persists it bound to the
//!   provider + caller-supplied `redirect_uri`, and returns the
//!   provider's authorize URL with `client_id`, `redirect_uri`,
//!   `scope`, `response_type=code`, and `state` query parameters
//!   baked in. The client redirects the user's browser there.
//!
//! - `POST /api/realms/:realm/auth/oauth/:provider/callback`
//!   Body: `{ code, state }`. The endpoint atomically consumes the
//!   state nonce (rejecting mismatched provider, replay, or
//!   expiry), exchanges the code for an access token at the
//!   provider's `token_url`, fetches the userinfo blob, extracts
//!   the stable provider-side id + email, then:
//!     * if a `user_oauth_links` row matches → log that user in,
//!     * if a user already exists for the userinfo email → link it
//!       and log in,
//!     * otherwise create a passwordless user, mark it verified
//!       (OAuth proved email ownership), link it, and log it in.
//!
//! Provider configuration (auth URL, token URL, userinfo URL, scopes,
//! id/email JSON pointers) is per-realm and currently seeded by tests
//! or operators via direct DB writes; an admin-facing CRUD endpoint
//! is deferred to a follow-up.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::Duration;
use rand_core::{OsRng, RngCore};
use rustbase_auth::{TokenRole, build_claims, encode_token};
use rustbase_core::{CoreError, RealmId};
use rustbase_db::{
    oauth_links,
    oauth_providers::{self, OAuthProvider},
    oauth_states::{self, ConsumeOutcome},
    realms::find_realm,
    tokens::{SubjectKind, insert_refresh_token},
    users::{User, find_user_by_email, insert_passwordless_user, mark_verified, record_last_login},
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::auth::login::UserPublic;
use crate::auth::{default_access_ttl, default_refresh_ttl, new_refresh_token};
use crate::error::ApiError;
use crate::state::AppState;

const STATE_TTL_MINUTES: i64 = 5;

#[derive(Debug, Deserialize)]
pub struct AuthorizeQuery {
    /// Where the provider should send the user back after consent —
    /// must match the value registered with the provider. Passed
    /// straight through to the upstream URL.
    pub redirect_uri: String,
}

#[derive(Debug, Serialize)]
pub struct AuthorizeResponse {
    pub authorize_url: String,
    pub state: String,
}

#[derive(Debug, Deserialize)]
pub struct CallbackBody {
    pub code: String,
    pub state: String,
}

#[derive(Debug, Serialize)]
pub struct CallbackResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub user: UserPublic,
}

/// `GET /api/realms/:realm/auth/oauth/:provider/authorize`.
pub async fn authorize(
    State(state): State<AppState>,
    Path((realm, provider)): Path<(String, String)>,
    Query(q): Query<AuthorizeQuery>,
) -> Result<Json<AuthorizeResponse>, ApiError> {
    find_realm(state.system.pool(), &realm)
        .await?
        .ok_or(ApiError::Core(CoreError::RealmNotFound(realm.clone())))?;
    let pool = state.realms.pool_for(&RealmId::from(realm.clone())).await?;

    let cfg = oauth_providers::find_provider(&pool, &provider)
        .await?
        .ok_or(ApiError::Core(CoreError::NotFound {
            collection: "oauth_provider".into(),
            id: provider.clone(),
        }))?;

    let nonce = fresh_state_nonce();
    oauth_states::issue(
        &pool,
        &nonce,
        &provider,
        &q.redirect_uri,
        Duration::minutes(STATE_TTL_MINUTES),
    )
    .await?;

    let authorize_url = build_authorize_url(&cfg, &q.redirect_uri, &nonce)?;
    Ok(Json(AuthorizeResponse {
        authorize_url,
        state: nonce,
    }))
}

/// `POST /api/realms/:realm/auth/oauth/:provider/callback`.
pub async fn callback(
    State(app_state): State<AppState>,
    Path((realm, provider)): Path<(String, String)>,
    Json(body): Json<CallbackBody>,
) -> Result<Json<CallbackResponse>, ApiError> {
    find_realm(app_state.system.pool(), &realm)
        .await?
        .ok_or(ApiError::Core(CoreError::RealmNotFound(realm.clone())))?;
    let pool = app_state
        .realms
        .pool_for(&RealmId::from(realm.clone()))
        .await?;

    let redirect_uri = match oauth_states::consume(&pool, &body.state, &provider).await? {
        ConsumeOutcome::Ok { redirect_uri } => redirect_uri,
        ConsumeOutcome::Unknown | ConsumeOutcome::ProviderMismatch => {
            return Err(ApiError::Core(CoreError::Unauthorized));
        }
        ConsumeOutcome::AlreadyConsumed => {
            return Err(ApiError::Core(CoreError::Conflict(
                "oauth state already used".into(),
            )));
        }
        ConsumeOutcome::Expired => {
            return Err(ApiError::Core(CoreError::Conflict(
                "oauth state expired — restart sign-in".into(),
            )));
        }
    };

    let cfg = oauth_providers::find_provider(&pool, &provider)
        .await?
        .ok_or(ApiError::Core(CoreError::NotFound {
            collection: "oauth_provider".into(),
            id: provider.clone(),
        }))?;

    // Exchange the auth code for an access token at the provider.
    let token = exchange_code(&cfg, &body.code, &redirect_uri).await?;
    // Fetch the userinfo blob with the access token. Extract the
    // stable provider-side id and the verified email using the JSON
    // pointers stored in the provider config.
    let userinfo = fetch_userinfo(&cfg, &token).await?;
    let provider_user_id = pluck_string(&userinfo, &cfg.config.userinfo_id_field).ok_or(
        ApiError::Core(CoreError::Validation(format!(
            "userinfo missing {} (provider {})",
            cfg.config.userinfo_id_field, provider
        ))),
    )?;
    let email = pluck_string(&userinfo, &cfg.config.userinfo_email_field).ok_or(ApiError::Core(
        CoreError::Validation(format!(
            "userinfo missing {} (provider {})",
            cfg.config.userinfo_email_field, provider
        )),
    ))?;

    let user = resolve_user(&pool, &provider, &provider_user_id, &email).await?;
    record_last_login(&pool, &user.id).await?;

    let claims = build_claims(
        user.id.clone(),
        TokenRole::User,
        Some(realm.clone()),
        None,
        default_access_ttl(),
    );
    let access_token = encode_token(&claims, &app_state.master_key)?;
    let refresh = insert_refresh_token(
        &pool,
        &new_refresh_token(),
        SubjectKind::User,
        &user.id,
        default_refresh_ttl(),
    )
    .await?;

    tracing::info!(
        realm = %realm,
        user_id = %user.id,
        provider = %provider,
        "user signed in via OAuth"
    );

    Ok(Json(CallbackResponse {
        access_token,
        refresh_token: refresh.token,
        user: UserPublic {
            id: user.id,
            email: user.email,
            verified: true,
        },
    }))
}

/// Three-way resolve: existing link → existing user (link) →
/// brand-new passwordless signup. Marks verified=true in every
/// branch since the provider proved email ownership.
async fn resolve_user(
    pool: &SqlitePool,
    provider: &str,
    provider_user_id: &str,
    email: &str,
) -> Result<User, ApiError> {
    if let Some(link) = oauth_links::find_by_provider_user(pool, provider, provider_user_id).await?
    {
        let user = rustbase_db::users::find_user_by_id(pool, &link.user_id)
            .await?
            .ok_or(ApiError::Core(CoreError::Internal(
                "linked user disappeared".into(),
            )))?;
        if !user.verified {
            mark_verified(pool, &user.id).await?;
        }
        return Ok(user);
    }
    if let Some(user) = find_user_by_email(pool, email).await? {
        oauth_links::upsert_link(pool, &user.id, provider, provider_user_id).await?;
        if !user.verified {
            mark_verified(pool, &user.id).await?;
        }
        return Ok(user);
    }
    let fresh = insert_passwordless_user(pool, email).await?;
    mark_verified(pool, &fresh.id).await?;
    oauth_links::upsert_link(pool, &fresh.id, provider, provider_user_id).await?;
    tracing::info!(user_id = %fresh.id, %provider, "user signed up via OAuth");
    Ok(fresh)
}

/// Build the provider's authorize URL with our query params attached.
/// We hand-build this rather than reach into the `oauth2` crate
/// purely so the URL we hand the client is stable + auditable; the
/// `oauth2` crate makes opinionated choices about response types and
/// scope encoding that we don't actually need to inherit here.
fn build_authorize_url(
    cfg: &OAuthProvider,
    redirect_uri: &str,
    state: &str,
) -> Result<String, ApiError> {
    let scope = cfg.config.scopes.join(" ");
    let mut url = reqwest::Url::parse(&cfg.config.auth_url).map_err(|e| {
        ApiError::Core(CoreError::Internal(format!(
            "provider {} has invalid auth_url: {e}",
            cfg.provider
        )))
    })?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("client_id", &cfg.client_id);
        q.append_pair("redirect_uri", redirect_uri);
        q.append_pair("response_type", "code");
        q.append_pair("scope", &scope);
        q.append_pair("state", state);
    }
    Ok(url.into())
}

/// POST to the provider's token endpoint. We hand-roll the form body
/// (RFC 6749 §4.1.3) for the same reason as `build_authorize_url`:
/// total transparency over what's on the wire.
async fn exchange_code(
    cfg: &OAuthProvider,
    code: &str,
    redirect_uri: &str,
) -> Result<String, ApiError> {
    let client = reqwest::Client::new();
    let resp = client
        .post(&cfg.config.token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", cfg.client_id.as_str()),
            ("client_secret", cfg.client_secret.as_str()),
        ])
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|e| ApiError::Core(CoreError::Internal(format!("token POST: {e}"))))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(ApiError::Core(CoreError::Internal(format!(
            "token endpoint returned {status}: {body}"
        ))));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ApiError::Core(CoreError::Internal(format!("token json: {e}"))))?;
    body.get("access_token")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or(ApiError::Core(CoreError::Internal(
            "token response missing access_token".into(),
        )))
}

async fn fetch_userinfo(
    cfg: &OAuthProvider,
    access_token: &str,
) -> Result<serde_json::Value, ApiError> {
    let client = reqwest::Client::new();
    let resp = client
        .get(&cfg.config.userinfo_url)
        .bearer_auth(access_token)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|e| ApiError::Core(CoreError::Internal(format!("userinfo GET: {e}"))))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(ApiError::Core(CoreError::Internal(format!(
            "userinfo endpoint returned {status}: {body}"
        ))));
    }
    resp.json()
        .await
        .map_err(|e| ApiError::Core(CoreError::Internal(format!("userinfo json: {e}"))))
}

/// Resolve a JSON Pointer (RFC 6901) into the userinfo blob and
/// extract a string. Returns `None` if the pointer doesn't resolve or
/// the target isn't a string.
fn pluck_string(v: &serde_json::Value, pointer: &str) -> Option<String> {
    v.pointer(pointer)
        .and_then(|p| p.as_str())
        .map(str::to_string)
}

/// 32 random bytes hex-encoded — same shape as the verify-email /
/// password-reset tokens. Provides ~256 bits of entropy, far more
/// than enough for a 5-minute-TTL CSRF nonce.
fn fresh_state_nonce() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
