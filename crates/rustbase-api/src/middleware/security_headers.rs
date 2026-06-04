//! Default-on bundle of conservative HTTP response headers.
//!
//! We layer this in front of the router via `tower-http`'s
//! `SetResponseHeaderLayer`. Caddy/nginx users who already inject the
//! same headers can disable the bundle in `rustbase.toml`.
//!
//! Header choices follow the OWASP Secure Headers baseline:
//!   - `Strict-Transport-Security` (TLS pinning; opt-in to a long
//!     `max-age` and `includeSubDomains` via config).
//!   - `X-Content-Type-Options: nosniff` (no MIME guessing).
//!   - `Referrer-Policy: strict-origin-when-cross-origin` (minimal
//!     referrer leakage).
//!   - `X-Frame-Options: DENY` (clickjacking defense; CSP `frame-ancestors`
//!     is the modern replacement but XFO still works on legacy browsers).
//!   - `Permissions-Policy` — turn off the high-risk APIs by default.
//!   - `Content-Security-Policy` — restrictive baseline tuned for the
//!     embedded dashboard (self-only scripts/styles/images plus
//!     `data:` images for inlined SVGs).

use axum::http::{HeaderName, HeaderValue, header};
use tower::ServiceBuilder;
use tower::layer::util::{Identity, Stack};
use tower_http::set_header::SetResponseHeaderLayer;

#[derive(Debug, Clone, Copy)]
pub struct SecurityHeadersConfig {
    pub hsts_max_age_secs: u64,
    pub hsts_include_subdomains: bool,
}

fn hsts_value(cfg: SecurityHeadersConfig) -> Option<HeaderValue> {
    if cfg.hsts_max_age_secs == 0 {
        return None;
    }
    let suffix = if cfg.hsts_include_subdomains {
        "; includeSubDomains"
    } else {
        ""
    };
    HeaderValue::from_str(&format!("max-age={}{}", cfg.hsts_max_age_secs, suffix)).ok()
}

const CSP_BASELINE: &str = "default-src 'self'; \
    img-src 'self' data:; \
    style-src 'self' 'unsafe-inline'; \
    script-src 'self'; \
    connect-src 'self'; \
    frame-ancestors 'none'; \
    base-uri 'self'; \
    form-action 'self'";

const PERMISSIONS_POLICY: &str =
    "geolocation=(), microphone=(), camera=(), payment=(), usb=(), interest-cohort=()";

type HeaderStack = Stack<
    SetResponseHeaderLayer<HeaderValue>,
    Stack<
        SetResponseHeaderLayer<HeaderValue>,
        Stack<
            SetResponseHeaderLayer<HeaderValue>,
            Stack<
                SetResponseHeaderLayer<HeaderValue>,
                Stack<
                    SetResponseHeaderLayer<HeaderValue>,
                    Stack<SetResponseHeaderLayer<HeaderValue>, Identity>,
                >,
            >,
        >,
    >,
>;

/// Build a `ServiceBuilder` stack that overrides each header
/// unconditionally (replaces any upstream value so a misbehaving handler
/// can't downgrade them).
pub fn layer(cfg: SecurityHeadersConfig) -> ServiceBuilder<HeaderStack> {
    let hsts = hsts_value(cfg).unwrap_or_else(|| HeaderValue::from_static("max-age=0"));
    let nosniff = HeaderValue::from_static("nosniff");
    let referrer = HeaderValue::from_static("strict-origin-when-cross-origin");
    let frame_options = HeaderValue::from_static("DENY");
    let permissions = HeaderValue::from_static(PERMISSIONS_POLICY);
    let csp = HeaderValue::from_static(CSP_BASELINE);

    ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
            header::STRICT_TRANSPORT_SECURITY,
            hsts,
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            nosniff,
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            referrer,
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-frame-options"),
            frame_options,
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("permissions-policy"),
            permissions,
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            csp,
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsts_zero_omits() {
        assert!(
            hsts_value(SecurityHeadersConfig {
                hsts_max_age_secs: 0,
                hsts_include_subdomains: true,
            })
            .is_none()
        );
    }

    #[test]
    fn hsts_two_years_with_subdomains() {
        let v = hsts_value(SecurityHeadersConfig {
            hsts_max_age_secs: 63_072_000,
            hsts_include_subdomains: true,
        })
        .unwrap();
        assert_eq!(v.to_str().unwrap(), "max-age=63072000; includeSubDomains");
    }

    #[test]
    fn hsts_without_subdomains() {
        let v = hsts_value(SecurityHeadersConfig {
            hsts_max_age_secs: 3600,
            hsts_include_subdomains: false,
        })
        .unwrap();
        assert_eq!(v.to_str().unwrap(), "max-age=3600");
    }
}
