use rustbase_auth::RevocationSet;
use rustbase_db::{AppPoolManager, RealmPoolManager, SystemPool};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Shared state threaded through every axum handler via `with_state`.
#[derive(Clone)]
pub struct AppState {
    pub system: Arc<SystemPool>,
    pub realms: Arc<RealmPoolManager>,
    pub apps: Arc<AppPoolManager>,
    pub revocations: RevocationSet,
    /// Cached "has at least one master admin." Flipped from `false` to
    /// `true` exactly once when the setup wizard completes; on subsequent
    /// boots, populated from the DB count.
    pub initialized: Arc<AtomicBool>,
}

impl AppState {
    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire)
    }

    pub fn mark_initialized(&self) {
        self.initialized.store(true, Ordering::Release);
    }
}
