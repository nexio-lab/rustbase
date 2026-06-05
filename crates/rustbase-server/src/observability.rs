//! Prometheus metrics + HTTP instrumentation.
//!
//! When `[observability]` enables it, boot installs a global
//! `metrics_exporter_prometheus` recorder and the server mounts
//! `GET /metrics` (bearer-token gated). A small `from_fn` middleware
//! counts every request and records its latency by `(method, route,
//! status)`. The route label uses axum's `MatchedPath` — i.e. the
//! template (`/api/workspaces/{workspace}/...`), not the literal URI
//! — so the cardinality stays bounded.

use axum::{
    body::Body,
    extract::{MatchedPath, State},
    http::{HeaderMap, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::time::Instant;

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ObservabilityConfig {
    /// Master toggle. Defaults to `false`: no metrics recorder is
    /// installed, no `/metrics` route is mounted, no token check
    /// runs.
    #[serde(default)]
    pub metrics_enabled: bool,
    /// Bearer token required on `GET /metrics`. Empty / unset means
    /// the endpoint refuses every request with 404 — the operator
    /// MUST set a token before scrape config can succeed. This keeps
    /// the metrics surface from accidentally leaking to the internet
    /// on a misconfigured deployment.
    #[serde(default)]
    pub metrics_token: Option<String>,
}

/// Boot-time setup. When `metrics_enabled = false`, returns `None`
/// and the server skips the middleware + the `/metrics` route. When
/// enabled, installs the global recorder once and returns the
/// handle so the HTTP handler can render snapshots.
pub fn init(cfg: &ObservabilityConfig) -> anyhow::Result<Option<PrometheusHandle>> {
    if !cfg.metrics_enabled {
        return Ok(None);
    }
    if cfg
        .metrics_token
        .as_deref()
        .map(str::is_empty)
        .unwrap_or(true)
    {
        anyhow::bail!("[observability] metrics_enabled = true requires a non-empty metrics_token");
    }
    // Histogram buckets: 1ms → 10s, the meaningful range for an
    // SQLite-backed HTTP backend. Anything outside this almost
    // always means the request is queued behind a busy DB pool.
    let builder = PrometheusBuilder::new().set_buckets_for_metric(
        metrics_exporter_prometheus::Matcher::Full(
            "rustbase_http_request_duration_seconds".to_string(),
        ),
        &[
            0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
        ],
    )?;
    let handle = builder.install_recorder()?;

    // Emit a build_info gauge once — Prometheus convention for
    // shipping the running version alongside the runtime metrics.
    metrics::gauge!(
        "rustbase_build_info",
        "version" => env!("CARGO_PKG_VERSION"),
    )
    .set(1.0);

    Ok(Some(handle))
}

/// `axum::middleware::from_fn` body. Records latency + status for
/// every request that passes through. Skipped routes (those without
/// a `MatchedPath`, e.g. static dashboard 404s) are labelled
/// `route="<unmatched>"`.
pub async fn track_http_metrics(req: Request<Body>, next: Next) -> Response {
    let method = req.method().clone();
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_owned())
        .unwrap_or_else(|| "<unmatched>".to_owned());

    let start = Instant::now();
    let res = next.run(req).await;
    let elapsed = start.elapsed().as_secs_f64();
    let status = res.status().as_u16().to_string();

    metrics::counter!(
        "rustbase_http_requests_total",
        "method" => method.to_string(),
        "route" => route.clone(),
        "status" => status.clone(),
    )
    .increment(1);
    metrics::histogram!(
        "rustbase_http_request_duration_seconds",
        "method" => method.to_string(),
        "route" => route,
        "status" => status,
    )
    .record(elapsed);

    res
}

/// `GET /metrics` handler. Bearer-token gated against the configured
/// `metrics_token`. Returns the Prometheus text-format snapshot.
///
/// 404 (not 401) is intentional: scrapers without the token should
/// not learn that the endpoint exists.
pub async fn metrics_endpoint(State(state): State<MetricsState>, headers: HeaderMap) -> Response {
    let expected = match state.token.as_deref() {
        Some(t) if !t.is_empty() => t,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let provided = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if provided != Some(expected) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let body = state.handle.render();
    ([("Content-Type", "text/plain; version=0.0.4")], body).into_response()
}

/// Shared state injected into the `/metrics` handler via
/// `axum::extract::State`. Clone-cheap.
#[derive(Clone)]
pub struct MetricsState {
    pub handle: PrometheusHandle,
    pub token: Option<String>,
}
