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
use tower_governor::key_extractor::PeerIpKeyExtractor;

#[derive(Debug, Clone, Copy)]
pub struct RateLimitConfig {
    pub per_second: u32,
    pub burst: u32,
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

pub fn layer(cfg: RateLimitConfig) -> Option<RateLimitLayer> {
    let per_second = cfg.per_second.max(1);
    let per_ms = u64::from((1000 / per_second).max(1));
    let burst = cfg.burst.max(per_second);

    let governor_cfg = GovernorConfigBuilder::default()
        .per_millisecond(per_ms)
        .burst_size(burst)
        .finish()?;

    Some(GovernorLayer::new(governor_cfg).error_handler(governor_error_to_response))
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
