//! HTTP API for RustBase.
//!
//! Exposes REST endpoints under `/api/realms/<realm>/apps/<app>/...`, plus
//! SSE and WebSocket endpoints for realtime subscriptions. Errors map to
//! HTTP status codes via an `IntoResponse` implementation for `ApiError`.
