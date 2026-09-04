//! IP-keyed token-bucket rate limiter at the HTTP entry layer.
//!
//! Backed by `tower_governor`, which wraps the `governor` crate. The
//! per-second / burst values come from `[rate_limit]` in `rustbase.toml`.
//! On rejection we return a JSON envelope shaped like every other
//! `ApiError` response (`code = "too_many_requests"`, plus
//! `Retry-After` when the limiter knows when the next slot opens).

use axum::Json;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use governor::clock::QuantaInstant;
use governor::middleware::NoOpMiddleware;
use serde_json::json;
use tower_governor::GovernorLayer;
use tower_governor::errors::GovernorError;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::{PeerIpKeyExtractor, SmartIpKeyExtractor};

#[derive(Debug, Clone, Copy)]
pub struct RateLimitConfig {
    pub per_second: u32,
    pub burst: u32,
    /// Key on the client address carried by `X-Forwarded-For` /
    /// `X-Real-IP` / `Forwarded` instead of the peer address.
    ///
    /// Off by default, and it must stay that way unless a trusted
    /// proxy sits in front: those headers are caller-supplied, so on a
    /// directly exposed server anyone can mint a fresh identity per
    /// request and escape the limiter completely. Behind a proxy the
    /// opposite is true — every request shows the proxy's own address,
    /// so peer-IP keying puts the whole internet in one bucket.
    pub trust_proxy_headers: bool,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            per_second: 10,
            burst: 20,
            trust_proxy_headers: false,
        }
    }
}

/// Build a tower-governor layer keyed by the connection's source IP.
///
/// Returns `None` when the builder rejects the parameters (e.g. zero
/// rate). Callers should treat `None` as "rate limit disabled" so a
/// misconfig never blocks the entire surface.
/// `axum::body::Body` here matches the response body type used by the
/// rest of the router so the layer composes without a `Either`-wrapped
/// response.
pub type RateLimitLayer =
    GovernorLayer<PeerIpKeyExtractor, NoOpMiddleware<QuantaInstant>, axum::body::Body>;

pub type ForwardedRateLimitLayer =
    GovernorLayer<SmartIpKeyExtractor, NoOpMiddleware<QuantaInstant>, axum::body::Body>;

/// Which key the limiter buckets on. The two extractors are distinct
/// types, so the choice has to travel to the call site rather than
/// being hidden behind one alias.
pub enum RateLimitLayers {
    PeerIp(RateLimitLayer),
    ForwardedIp(ForwardedRateLimitLayer),
}

pub fn layer(cfg: RateLimitConfig) -> Option<RateLimitLayers> {
    let per_second = cfg.per_second.max(1);
    let per_ms = u64::from((1000 / per_second).max(1));
    let burst = cfg.burst.max(per_second);

    let governor_cfg = GovernorConfigBuilder::default()
        .per_millisecond(per_ms)
        .burst_size(burst)
        .finish()?;

    if cfg.trust_proxy_headers {
        let governor_cfg = GovernorConfigBuilder::default()
            .per_millisecond(per_ms)
            .burst_size(burst)
            .key_extractor(SmartIpKeyExtractor)
            .finish()?;
        return Some(RateLimitLayers::ForwardedIp(
            GovernorLayer::new(governor_cfg).error_handler(governor_error_to_response),
        ));
    }
    Some(RateLimitLayers::PeerIp(
        GovernorLayer::new(governor_cfg).error_handler(governor_error_to_response),
    ))
}

fn governor_error_to_response(err: GovernorError) -> Response {
    let (status, retry_after_secs, code, message) = match err {
        GovernorError::TooManyRequests { wait_time, .. } => (
            StatusCode::TOO_MANY_REQUESTS,
            Some(wait_time),
            "too_many_requests",
            format!("rate limit exceeded; retry after {wait_time}s"),
        ),
        GovernorError::UnableToExtractKey => (
            StatusCode::INTERNAL_SERVER_ERROR,
            None,
            "internal",
            "could not extract rate-limit key".to_string(),
        ),
        GovernorError::Other { code, msg, .. } => (
            code,
            None,
            "internal",
            msg.unwrap_or_else(|| "rate-limit middleware error".to_string()),
        ),
    };
    let body = json!({ "code": code, "message": message });
    let mut resp = (status, Json(body)).into_response();
    if let Some(secs) = retry_after_secs
        && let Ok(value) = axum::http::HeaderValue::from_str(&secs.to_string())
    {
        resp.headers_mut().insert(header::RETRY_AFTER, value);
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, http::Request, routing::get};
    use tower::ServiceExt;

    fn router_with(cfg: RateLimitConfig) -> Router {
        let app = Router::new().route("/", get(|| async { "ok" }));
        match layer(cfg).expect("layer must build") {
            RateLimitLayers::ForwardedIp(l) => app.layer(l),
            RateLimitLayers::PeerIp(l) => app.layer(l),
        }
    }

    async fn get_as(app: &Router, forwarded_for: &str) -> StatusCode {
        let req = Request::builder()
            .uri("/")
            .header("x-forwarded-for", forwarded_for)
            .body(Body::empty())
            .unwrap();
        app.clone().oneshot(req).await.unwrap().status()
    }

    /// Behind a reverse proxy every request arrives from the proxy's
    /// own address. Keying on the peer IP therefore puts the entire
    /// internet in one bucket: one client can exhaust it for everyone,
    /// and per-client brute-force limiting stops existing. With
    /// `trust_proxy_headers`, the forwarded address is the key.
    #[tokio::test]
    async fn two_clients_behind_one_proxy_get_separate_buckets() {
        let app = router_with(RateLimitConfig {
            per_second: 1,
            burst: 1,
            trust_proxy_headers: true,
        });

        assert_eq!(get_as(&app, "203.0.113.1").await, StatusCode::OK);
        assert_eq!(
            get_as(&app, "203.0.113.2").await,
            StatusCode::OK,
            "a second client was charged to the first one's bucket"
        );
        assert_eq!(
            get_as(&app, "203.0.113.1").await,
            StatusCode::TOO_MANY_REQUESTS,
            "the first client should have spent its budget"
        );
    }

    /// The flag must stay off by default: trusting `X-Forwarded-For`
    /// on a directly exposed server lets any caller forge a fresh
    /// identity per request and escape the limiter entirely.
    #[test]
    fn proxy_headers_are_not_trusted_unless_asked() {
        assert!(!RateLimitConfig::default().trust_proxy_headers);
    }
}
