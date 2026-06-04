//! `POST /_/auth/logout` — server-side session destruction.
//!
//! Two side effects:
//!
//! 1. If a refresh token is presented (cookie or body), revoke it in
//!    the master `_refresh_tokens` table so the same refresh can't be
//!    reused even if the cookie is somehow recovered.
//! 2. Issue `Set-Cookie` headers with `Max-Age=0` for both `rb_at` and
//!    `rb_rt`, instructing the browser to drop the cached values.
//!
//! Anonymous endpoint — a token isn't required (a stale session that
//! lost its access token should still be able to log out cleanly).
//! Bearer header isn't read at all here; the dashboard hits the
//! endpoint with the cookies the browser already attaches.
//!
//! Workspace + end-user logouts are intentionally out of scope for this
//! endpoint: the cookie is path-scoped to `/_/auth`, so a browser
//! that's only logged into the dashboard never sends an `rb_rt`
//! cookie to `/api/...`. Workspace/app SDK clients hold their refresh
//! token in memory and just discard it client-side.

use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use rustbase_db::tokens::revoke_refresh_token;
use serde::Deserialize;
use serde_json::json;

use crate::error::ApiError;
use crate::state::AppState;

use super::cookies::{
    CookieFlags, REFRESH_COOKIE, clear_access_cookie, clear_refresh_cookie, read_cookie,
};

#[derive(Debug, Deserialize, Default)]
pub struct LogoutRequest {
    #[serde(default)]
    pub refresh_token: Option<String>,
}

pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<LogoutRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let presented = match body {
        Ok(Json(b)) => b.refresh_token,
        Err(_) => None,
    }
    .filter(|s| !s.is_empty())
    .or_else(|| read_cookie(&headers, REFRESH_COOKIE));

    if let Some(tok) = presented.as_deref() {
        // Best-effort revoke under the master scope (the dashboard's
        // refresh cookie is master-scoped). Failures are swallowed so
        // a missing/expired token still returns 204 — the user should
        // be able to log out idempotently.
        if let Err(e) = revoke_under_master(state.system.pool(), tok).await {
            tracing::debug!(error = %e, "logout: master revoke best-effort failure");
        }
    }

    let flags = CookieFlags {
        secure: state.cookie_secure,
    };
    let mut resp = Json(json!({ "ok": true })).into_response();
    let h = resp.headers_mut();
    h.append(axum::http::header::SET_COOKIE, clear_access_cookie(flags));
    h.append(axum::http::header::SET_COOKIE, clear_refresh_cookie(flags));
    Ok(resp)
}

async fn revoke_under_master(
    pool: &sqlx::SqlitePool,
    token: &str,
) -> Result<(), rustbase_db::DbError> {
    // The dashboard refresh cookie is master-scoped; revoke by token
    // unconditionally. A missing row is a no-op — the cookies still
    // get cleared, which is the shape the user wants from logout.
    revoke_refresh_token(pool, token).await
}
