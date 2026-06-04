//! HTTP API for RustBase.
//!
//! Exposes REST endpoints under `/api/workspaces/<workspace>/apps/<app>/...`, plus
//! SSE and WebSocket endpoints for realtime subscriptions. Errors map to
//! HTTP status codes via an `IntoResponse` implementation for `ApiError`.

pub mod access_rules;
pub mod apps;
pub mod audit;
pub mod auth;
pub mod collections;
pub mod custom_routes;
pub mod error;
pub mod files;
pub mod health;
pub mod hook_bridge;
pub mod hooks;
pub mod mailer;
pub mod middleware;
pub mod policies;
pub mod realtime;
pub mod records;
pub mod router;
pub mod security;
pub mod setup;
pub mod state;
pub mod users;
pub mod workspace_admins;
pub mod workspaces;

pub use auth::AdminAuth;
pub use error::ApiError;
pub use router::build_router;
pub use state::AppState;
