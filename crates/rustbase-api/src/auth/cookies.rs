//! Dashboard session cookies — `HttpOnly`, `SameSite=Strict`,
//! optionally `Secure`. Two cookies are emitted on every successful
//! dashboard login or refresh:
//!
//! `rb_at` — the access token. Path `/`, Max-Age aligned with the
//! access-token TTL (15 min by default). Sent on every same-origin
//! request so the API surface authenticates the dashboard without
//! the SPA ever touching the raw token.
//!
//! `rb_rt` — the refresh token. Path `/_/auth` so it's only sent
//! to the dashboard auth endpoints (`/login`, `/refresh`, `/logout`).
//! Max-Age aligned with the refresh-token TTL (30 days).
//!
//! Both cookies are `HttpOnly` so client-side JS cannot read them —
//! that blocks the XSS-token-theft attack `localStorage` would
//! expose. `SameSite=Strict` blocks CSRF without a separate token.
//! `Secure` is gated on `AppState::cookie_secure` so local-dev HTTP
//! works.

use axum::http::HeaderValue;

/// Cookie name for the access token. Path `/`, so the dashboard SPA
/// authenticates against the REST surface implicitly.
pub const ACCESS_COOKIE: &str = "rb_at";
/// Cookie name for the refresh token. Path scoped to `/_/auth` so it
/// only travels to the dashboard auth endpoints.
pub const REFRESH_COOKIE: &str = "rb_rt";

#[derive(Debug, Clone, Copy)]
pub struct CookieFlags {
    pub secure: bool,
}

pub fn build_access_cookie(token: &str, ttl_secs: i64, flags: CookieFlags) -> HeaderValue {
    build_cookie(ACCESS_COOKIE, token, "/", ttl_secs, flags)
}

pub fn build_refresh_cookie(token: &str, ttl_secs: i64, flags: CookieFlags) -> HeaderValue {
    build_cookie(REFRESH_COOKIE, token, "/_/auth", ttl_secs, flags)
}

/// Set a cookie whose `Max-Age=0` instructs the browser to drop the
/// stored value immediately. Used by `/_/auth/logout`.
pub fn clear_access_cookie(flags: CookieFlags) -> HeaderValue {
    build_cookie(ACCESS_COOKIE, "", "/", 0, flags)
}

pub fn clear_refresh_cookie(flags: CookieFlags) -> HeaderValue {
    build_cookie(REFRESH_COOKIE, "", "/_/auth", 0, flags)
}

fn build_cookie(
    name: &str,
    value: &str,
    path: &str,
    max_age_secs: i64,
    flags: CookieFlags,
) -> HeaderValue {
    let mut out =
        format!("{name}={value}; Path={path}; Max-Age={max_age_secs}; HttpOnly; SameSite=Strict");
    if flags.secure {
        out.push_str("; Secure");
    }
    // A well-formed token never contains a CR/LF, so `from_str` cannot
    // fail; the unreachable path falls back to an empty-value cookie
    // to keep this function infallible.
    HeaderValue::from_str(&out)
        .unwrap_or_else(|_| HeaderValue::from_static("rb_invalid=; Max-Age=0; HttpOnly"))
}

/// Read a cookie value out of an `axum::http::HeaderMap`, scanning the
/// raw `Cookie:` header for `name=value` pairs. Returns the first
/// match (browsers should never send duplicate names; if they do, the
/// first wins).
pub fn read_cookie(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    let header = headers.get(axum::http::header::COOKIE)?;
    let raw = header.to_str().ok()?;
    for pair in raw.split(';') {
        let trimmed = pair.trim();
        if let Some(value) = trimmed.strip_prefix(&format!("{name}=")) {
            return Some(value.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_cookie_has_required_flags() {
        let v = build_access_cookie("abc", 900, CookieFlags { secure: true });
        let s = v.to_str().unwrap();
        assert!(s.starts_with("rb_at=abc"));
        assert!(s.contains("Path=/"));
        assert!(s.contains("HttpOnly"));
        assert!(s.contains("SameSite=Strict"));
        assert!(s.contains("Secure"));
        assert!(s.contains("Max-Age=900"));
    }

    #[test]
    fn refresh_cookie_scopes_path_to_auth_only() {
        let v = build_refresh_cookie("rfsh_xyz", 2_592_000, CookieFlags { secure: false });
        let s = v.to_str().unwrap();
        assert!(s.starts_with("rb_rt=rfsh_xyz"));
        assert!(s.contains("Path=/_/auth"));
        assert!(!s.contains("Secure"));
    }

    #[test]
    fn clear_cookie_sets_max_age_zero() {
        let v = clear_access_cookie(CookieFlags { secure: true });
        let s = v.to_str().unwrap();
        assert!(s.contains("Max-Age=0"));
        assert!(s.starts_with("rb_at=;"));
    }

    #[test]
    fn read_cookie_extracts_named_pair() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            axum::http::HeaderValue::from_static("foo=bar; rb_at=tok-123; baz=qux"),
        );
        assert_eq!(read_cookie(&headers, "rb_at").as_deref(), Some("tok-123"));
        assert_eq!(read_cookie(&headers, "missing"), None);
    }
}
