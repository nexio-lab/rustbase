//! In-process pub/sub broker for realtime subscriptions.
//!
//! Channels are keyed by `(realm_id, app_id, collection, optional record_id)`.
//! SSE and WebSocket handlers in `rustbase-api` subscribe; DB lifecycle hooks
//! and JS/TS hooks publish.
