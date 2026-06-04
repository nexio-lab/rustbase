//! Cross-cutting HTTP middleware.
//!
//! `setup_gate` short-circuits the API while the server is uninitialized
//! (mounted as an axum from_fn middleware so it has access to
//! `AppState`). `security_headers`, `cors`, and `rate_limit` build
//! tower layers consumed by `rustbase-server` at boot.

pub mod cors;
pub mod rate_limit;
pub mod security_headers;
pub mod setup_gate;

pub use setup_gate::setup_gate;
