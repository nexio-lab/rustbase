//! HTTP API for RustBase.
//!
//! Exposes REST endpoints under `/api/realms/<realm>/apps/<app>/...`, plus
//! SSE and WebSocket endpoints for realtime subscriptions. Errors map to
//! HTTP status codes via an `IntoResponse` implementation for `ApiError`.

pub mod access_rules;
pub mod apps;
pub mod auth;
pub mod collections;
pub mod error;
pub mod health;
pub mod middleware;
pub mod policies;
pub mod realm_admins;
pub mod realms;
pub mod records;
pub mod router;
pub mod setup;
pub mod state;

pub use auth::AdminAuth;
pub use error::ApiError;
pub use router::build_router;
pub use state::AppState;
