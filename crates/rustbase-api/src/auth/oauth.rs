//! OAuth2 authorization-code sign-in with PKCE (RFC 7636).
//!
//! Two endpoints, both anonymous.
//!
//! `GET /api/workspaces/:workspace/apps/:app/auth/oauth/:provider/authorize?redirect_uri=...`
//! mints a fresh CSRF state nonce and a PKCE `code_verifier`,
//! persists both bound to the provider plus the caller-supplied
//! `redirect_uri`, and returns the provider's authorize URL with
//! `client_id`, `redirect_uri`, `scope`, `response_type=code`,
//! `state`, `code_challenge` (S256), and `code_challenge_method=S256`
//! query parameters baked in. The client redirects the user's
//! browser there.
//!
//! `POST /api/workspaces/:workspace/apps/:app/auth/oauth/:provider/callback`
//! takes `{ code, state }`. The endpoint atomically consumes the
//! state nonce (rejecting mismatched provider, replay, or expiry),
//! reads the stored `code_verifier`, exchanges the code plus the
//! verifier for an access token at the provider's `token_url`,
//! fetches the userinfo blob, and extracts the stable provider-side
//! id plus email. It then performs a three-way resolve — match an
//! existing `user_oauth_links` row, fall back to an existing user
//! with the same email (link them), or otherwise create a
//! passwordless user, mark verified (OAuth proved email ownership),
//! link, and log in.
//!
//! PKCE is mandatory — every flow generates a 32-byte verifier and
//! sends the S256 challenge. Providers that don't enforce it ignore
//! the field; providers that do (Google, GitHub, Microsoft, every
//! modern OIDC IdP) reject token exchanges whose verifier doesn't
//! match the original challenge, blocking authorization-code
//! interception attacks.
//!
//! Provider configuration (auth URL, token URL, userinfo URL, scopes,
//! id/email JSON pointers) is per-app — each app gets its own
//! `oauth_providers` table in its `data.db`.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Duration;
use rand_core::{OsRng, RngCore};
use rustbase_auth::{TokenRole, build_claims};
use rustbase_core::{CoreError, WorkspaceId};
use rustbase_db::{
    oauth_links,
    oauth_providers::{self, OAuthProvider},
    oauth_states::{self, ConsumeOutcome},
    tokens::commit_user_login,
    users::{User, find_user_by_email, insert_passwordless_user, mark_verified},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::auth::login::UserPublic;
use crate::auth::{
    default_access_ttl, default_refresh_ttl, new_refresh_token, require_workspace_exists,
};
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

/// `GET /api/workspaces/:workspace/auth/oauth/:provider/authorize`.
pub async fn authorize(
    State(state): State<AppState>,
    Path((workspace, provider)): Path<(String, String)>,
    Query(q): Query<AuthorizeQuery>,
) -> Result<Json<AuthorizeResponse>, ApiError> {
    require_workspace_exists(&state, &workspace).await?;
    let workspace_id = WorkspaceId::from(workspace.clone());
    let pool = state.workspaces.pool_for(&workspace_id).await?;

    let cfg = oauth_providers::find_provider(&pool, &provider)
        .await?
        .ok_or(ApiError::Core(CoreError::NotFound {
            collection: "oauth_provider".into(),
            id: provider.clone(),
        }))?;

    let nonce = fresh_state_nonce();
    let code_verifier = fresh_code_verifier();
    let code_challenge = derive_s256_challenge(&code_verifier);
    oauth_states::issue(
        &pool,
        &nonce,
        &provider,
        &q.redirect_uri,
        &verifier_for_storage(&state, &code_verifier)?,
        Duration::minutes(STATE_TTL_MINUTES),
    )
    .await?;

    let authorize_url = build_authorize_url(&cfg, &q.redirect_uri, &nonce, &code_challenge)?;
    Ok(Json(AuthorizeResponse {
        authorize_url,
        state: nonce,
    }))
}

/// `POST /api/workspaces/:workspace/auth/oauth/:provider/callback`.
pub async fn callback(
    State(app_state): State<AppState>,
    Path((workspace, provider)): Path<(String, String)>,
    Json(body): Json<CallbackBody>,
) -> Result<Json<CallbackResponse>, ApiError> {
    require_workspace_exists(&app_state, &workspace).await?;
    let workspace_id = WorkspaceId::from(workspace.clone());
    let pool = app_state.workspaces.pool_for(&workspace_id).await?;

    let (redirect_uri, code_verifier) =
        match oauth_states::consume(&pool, &body.state, &provider).await? {
            ConsumeOutcome::Ok {
                redirect_uri,
                code_verifier,
            } => (redirect_uri, code_verifier),
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

    // Decrypt the at-rest client_secret with the server-wide KEK.
    let kek = app_state.oauth_kek.as_ref().as_ref().ok_or_else(|| {
        ApiError::Core(CoreError::Unavailable(
            "no key-encryption key: set RUSTBASE_KEK to the value this \
             provider's secret was encrypted with"
                .into(),
        ))
    })?;
    let client_secret_bytes = rustbase_auth::decrypt(&cfg.secret_enc, kek)
        .map_err(|e| ApiError::Core(CoreError::Internal(format!("decrypt client_secret: {e}"))))?;
    let client_secret = String::from_utf8(client_secret_bytes)
        .map_err(|e| ApiError::Core(CoreError::Internal(format!("client_secret utf8: {e}"))))?;

    // Exchange the auth code for an access token at the provider.
    let token = exchange_code(
        &cfg,
        &client_secret,
        &body.code,
        &redirect_uri,
        Some(verifier_from_storage(&app_state, &code_verifier)?.as_str()),
    )
    .await?;
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

    let (user, just_signed_up) = resolve_user(&pool, &provider, &provider_user_id, &email).await?;

    let public = serde_json::json!({
        "id": &user.id,
        "email": &user.email,
        "verified": true,
    });
    let apps = rustbase_db::apps::list_apps(&pool).await?;
    for app in &apps {
        let hook_req = rustbase_runtime::HookRequest::system(&workspace, &app.id, "_user");
        if just_signed_up
            && let Err(e) = app_state
                .hooks
                .dispatch_user_after_register(&workspace, &app.id, &hook_req, &public)
                .await
        {
            tracing::warn!(error = %e, %workspace, app = %app.id, %provider, "user_after_register hook errored");
        }

        app_state
            .hooks
            .dispatch_user_before_login(&workspace, &app.id, &hook_req, &public)
            .await
            .map_err(|e| match e {
                rustbase_runtime::RuntimeError::Veto(msg) => {
                    tracing::info!(%workspace, app = %app.id, user_id = %user.id, %provider, %msg, "oauth login vetoed by hook");
                    ApiError::Core(CoreError::Forbidden)
                }
                other => ApiError::Core(CoreError::Internal(other.to_string())),
            })?;
    }

    let claims = build_claims(
        user.id.clone(),
        TokenRole::User,
        Some(workspace.clone()),
        // Workspace-shared identity → no `app` claim.
        None,
        default_access_ttl(),
    );
    let access_token = app_state.jwt.issue(&claims)?;
    // last_login + refresh insert in one txn.
    let issued = new_refresh_token();
    commit_user_login(&pool, &user.id, &issued, default_refresh_ttl()).await?;

    tracing::info!(
        workspace = %workspace,
        user_id = %user.id,
        provider = %provider,
        "user signed in via OAuth"
    );

    for app in &apps {
        let hook_req = rustbase_runtime::HookRequest::system(&workspace, &app.id, "_user");
        if let Err(e) = app_state
            .hooks
            .dispatch_user_after_login(&workspace, &app.id, &hook_req, &public)
            .await
        {
            tracing::warn!(error = %e, %workspace, app = %app.id, %provider, "user_after_login hook errored");
        }
    }

    Ok(Json(CallbackResponse {
        access_token,
        refresh_token: issued,
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
/// Returns the resolved user and `true` if this call inserted a
/// fresh row (so the caller knows to fire `onUserAfterRegister`).
async fn resolve_user(
    pool: &SqlitePool,
    provider: &str,
    provider_user_id: &str,
    email: &str,
) -> Result<(User, bool), ApiError> {
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
        return Ok((user, false));
    }
    if let Some(user) = find_user_by_email(pool, email).await? {
        oauth_links::upsert_link(pool, &user.id, provider, provider_user_id).await?;
        if !user.verified {
            mark_verified(pool, &user.id).await?;
        }
        return Ok((user, false));
    }
    let fresh = insert_passwordless_user(pool, email).await?;
    mark_verified(pool, &fresh.id).await?;
    oauth_links::upsert_link(pool, &fresh.id, provider, provider_user_id).await?;
    tracing::info!(user_id = %fresh.id, %provider, "user signed up via OAuth");
    Ok((fresh, true))
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
    code_challenge: &str,
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
        // PKCE (RFC 7636). S256 is the only method we offer; `plain`
        // is rejected by every modern provider and defeats the whole
        // purpose of the verifier.
        q.append_pair("code_challenge", code_challenge);
        q.append_pair("code_challenge_method", "S256");
    }
    Ok(url.into())
}

/// POST to the provider's token endpoint. We hand-roll the form body
/// (RFC 6749 §4.1.3) for the same reason as `build_authorize_url`:
/// total transparency over what's on the wire.
async fn exchange_code(
    cfg: &OAuthProvider,
    client_secret: &str,
    code: &str,
    redirect_uri: &str,
    code_verifier: Option<&str>,
) -> Result<String, ApiError> {
    let client = reqwest::Client::new();
    let mut form: Vec<(&str, &str)> = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", cfg.client_id.as_str()),
        ("client_secret", client_secret),
    ];
    if let Some(v) = code_verifier {
        form.push(("code_verifier", v));
    }
    let resp = client
        .post(&cfg.config.token_url)
        .form(&form)
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

/// PKCE `code_verifier` (RFC 7636 §4.1). 32 OS-random bytes encoded
/// as base64url-no-pad — yields a 43-character verifier (the upper
/// bound the spec recommends; allowed range is 43..=128).
/// PKCE verifiers get the same treatment as TOTP secrets: encrypted
/// when a KEK is configured, clear and marked otherwise. They cannot
/// be digests — `/callback` has to replay the verifier to the
/// provider — and refusing sign-in for want of a key would take OAuth
/// down entirely, which is a worse trade than a secret that lives for
/// the few minutes between the two legs.
fn verifier_for_storage(
    state: &AppState,
    verifier: &str,
) -> Result<rustbase_db::secret_at_rest::StoredSecret, ApiError> {
    use rustbase_db::secret_at_rest::StoredSecret;
    match state.oauth_kek.as_ref() {
        Some(kek) => {
            let ct = rustbase_auth::encrypt(verifier.as_bytes(), kek).map_err(|e| {
                ApiError::Core(CoreError::Internal(format!("encrypt code_verifier: {e}")))
            })?;
            Ok(StoredSecret::Encrypted(ct))
        }
        None => Ok(StoredSecret::Clear(verifier.to_string())),
    }
}

fn verifier_from_storage(
    state: &AppState,
    stored: &rustbase_db::secret_at_rest::StoredSecret,
) -> Result<String, ApiError> {
    use rustbase_db::secret_at_rest::StoredSecret;
    match stored {
        StoredSecret::Clear(s) => Ok(s.clone()),
        StoredSecret::Encrypted(ct) => {
            let kek = state.oauth_kek.as_ref().as_ref().ok_or_else(|| {
                ApiError::Core(CoreError::Unavailable(
                    "this OAuth flow's PKCE verifier is encrypted but RUSTBASE_KEK is \
                     not set; restore the key that started it"
                        .into(),
                ))
            })?;
            let plain = rustbase_auth::decrypt(ct, kek).map_err(|e| {
                ApiError::Core(CoreError::Internal(format!("decrypt code_verifier: {e}")))
            })?;
            String::from_utf8(plain).map_err(|e| {
                ApiError::Core(CoreError::Internal(format!("code_verifier not utf-8: {e}")))
            })
        }
    }
}

fn fresh_code_verifier() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// PKCE `code_challenge` for `code_challenge_method = "S256"`
/// (RFC 7636 §4.2): `base64url(sha256(code_verifier))`.
fn derive_s256_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_code_verifier_meets_rfc7636_size_bounds() {
        let v = fresh_code_verifier();
        // base64url(32 bytes) → 43 chars, comfortably inside 43..=128.
        assert!((43..=128).contains(&v.len()));
        assert!(
            v.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    #[test]
    fn derive_s256_challenge_matches_rfc7636_example() {
        // RFC 7636 Appendix B vector.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = derive_s256_challenge(verifier);
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn challenge_is_deterministic_for_same_verifier() {
        let v = "abcdefghijklmnopqrstuvwxyz0123456789-_";
        let c1 = derive_s256_challenge(v);
        let c2 = derive_s256_challenge(v);
        assert_eq!(c1, c2);
    }

    #[test]
    fn challenge_differs_for_different_verifiers() {
        let c1 = derive_s256_challenge("verifier-a");
        let c2 = derive_s256_challenge("verifier-b");
        assert_ne!(c1, c2);
    }
}
