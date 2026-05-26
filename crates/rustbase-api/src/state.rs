use rustbase_auth::{RevocationSet, SigningKey};
use rustbase_core::Mailer;
use rustbase_db::{AppPoolManager, RealmPoolManager, SystemPool};
use rustbase_realtime::RealtimeBroker;
use rustbase_runtime::HookEngine;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Shared state threaded through every axum handler via `with_state`.
#[derive(Clone)]
pub struct AppState {
    pub system: Arc<SystemPool>,
    pub realms: Arc<RealmPoolManager>,
    pub apps: Arc<AppPoolManager>,
    pub revocations: RevocationSet,
    /// HS256 signing key for master-admin tokens. Persisted in
    /// `system.db._secrets` so it survives restarts.
    pub master_key: Arc<SigningKey>,
    /// In-process pub/sub broker for record CRUD events.
    pub broker: RealtimeBroker,
    /// Embedded JS hook runtime, keyed per (realm, app).
    pub hooks: HookEngine,
    /// Root of the on-disk data tree. Used by handlers that need to
    /// resolve a realm or app folder for delete / storage operations.
    pub data_dir: Arc<PathBuf>,
    /// Cached "has at least one master admin." Flipped from `false` to
    /// `true` exactly once when the setup wizard completes; on subsequent
    /// boots, populated from the DB count.
    pub initialized: Arc<AtomicBool>,
    /// Outbound mail. In tests this is a `LogMailer` that captures
    /// messages in memory; in production it's an SMTP-backed impl.
    pub mailer: Arc<dyn Mailer>,
}

impl AppState {
    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire)
    }

    pub fn mark_initialized(&self) {
        self.initialized.store(true, Ordering::Release);
    }
}
