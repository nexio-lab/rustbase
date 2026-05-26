//! Catch-all axum handler for `$app.routerAdd` JS endpoints.
//!
//! User code registers routes from inside a hook file:
//!
//! ```js
//! $app.routerAdd("GET", "/hello", (ctx) => ({
//!     body: { msg: "hi " + ctx.query.name },
//! }));
//! ```
//!
//! That handler is reachable at:
//!
//! ```text
//! GET /api/realms/<realm>/apps/<app>/custom/hello?name=...
//! ```
//!
//! The axum side here strips the `/custom` prefix from the URL,
//! builds a [`CustomRouteContext`] from the incoming HTTP request
//! (method, query, headers, JSON body), hands it to the runtime,
//! and translates the JSON response shape into an `axum::Response`.

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
};
use rustbase_core::CoreError;
use rustbase_runtime::CustomRouteContext;
use serde_json::Value as Json;
use std::collections::BTreeMap;

use crate::error::ApiError;
use crate::state::AppState;

/// Catch-all handler mounted at
/// `/api/realms/{realm}/apps/{app}/custom/{*path}` in the router.
///
/// `path` is whatever followed `/custom/`. We prepend a `/` so the
/// JS shim sees the same path it registered (`routerAdd("…", "/hello", …)`).
pub async fn handle(
    State(state): State<AppState>,
    Path((realm, app, rest)): Path<(String, String, String)>,
    method: Method,
    headers: HeaderMap,
    Query(query): Query<BTreeMap<String, String>>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let path = format!("/{rest}");

    let mut hdrs = BTreeMap::new();
    for (k, v) in headers.iter() {
        if let Ok(s) = v.to_str() {
            hdrs.insert(k.as_str().to_lowercase(), s.to_string());
        }
    }

    // Parse the body as JSON when Content-Type says JSON. Anything
    // else — including malformed JSON — surfaces as a null body; the
    // handler can still inspect Content-Type via ctx.headers if it
    // cares about non-JSON payloads.
    let is_json = hdrs
        .get("content-type")
        .map(|v| v.starts_with("application/json"))
        .unwrap_or(false);
    let parsed_body: Json = if is_json && !body.is_empty() {
        serde_json::from_slice(&body).unwrap_or(Json::Null)
    } else {
        Json::Null
    };

    let ctx = CustomRouteContext {
        method: method.to_string(),
        path: path.clone(),
        query,
        headers: hdrs,
        body: parsed_body,
    };

    let resp = state
        .hooks
        .invoke_custom_route(&realm, &app, method.as_str(), &path, &ctx)
        .await
        .map_err(|e| ApiError::Core(CoreError::Internal(format!("custom route: {e}"))))?;

    let Some(resp) = resp else {
        return Err(ApiError::Core(CoreError::NotFound {
            collection: "custom_route".into(),
            id: format!("{} {}", method.as_str(), path),
        }));
    };

    let status = StatusCode::from_u16(resp.status)
        .map_err(|e| ApiError::Core(CoreError::Internal(format!("bad status: {e}"))))?;
    let body_bytes = serde_json::to_vec(&resp.body)
        .map_err(|e| ApiError::Core(CoreError::Internal(format!("encode body: {e}"))))?;

    let mut out = (status, body_bytes).into_response();
    // Default content type for JSON. The handler can override.
    out.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    for (k, v) in &resp.headers {
        let Ok(name) = HeaderName::from_bytes(k.as_bytes()) else {
            continue;
        };
        let Ok(val) = HeaderValue::from_str(v) else {
            continue;
        };
        out.headers_mut().insert(name, val);
    }
    Ok(out)
}
