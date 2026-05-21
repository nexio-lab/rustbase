use rustbase_auth::RevocationSet;
use rustbase_db::{AppPoolManager, RealmPoolManager, SystemPool};
use std::sync::Arc;

/// Shared state threaded through every axum handler via `with_state`.
#[derive(Clone)]
pub struct AppState {
    pub system: Arc<SystemPool>,
    pub realms: Arc<RealmPoolManager>,
    pub apps: Arc<AppPoolManager>,
    pub revocations: RevocationSet,
}
