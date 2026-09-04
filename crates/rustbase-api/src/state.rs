use crate::security::{LockoutPolicy, LoginAttempts};
use rustbase_auth::{JwtIssuer, RevocationSet, SigningKey};
use rustbase_core::Mailer;
use rustbase_db::{AppPoolManager, SystemPool, WorkspacePoolManager};
use rustbase_realtime::RealtimeBroker;
use rustbase_runtime::HookEngine;
use rustbase_storage::Storage;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Shared state threaded through every axum handler via `with_state`.
#[derive(Clone)]
pub struct AppState {
    pub system: Arc<SystemPool>,
    pub workspaces: Arc<WorkspacePoolManager>,
    pub apps: Arc<AppPoolManager>,
    pub revocations: RevocationSet,
    /// Legacy HS256 signing key. Kept around until the post-0.1.x
    /// transition is over so already-issued symmetric tokens stay
    /// valid until they naturally expire. New tokens are always RS256
    /// — issue them via [`AppState::jwt`].
    pub master_key: Arc<SigningKey>,
    /// Active JWT issuer/verifier. Issues RS256, accepts RS256 + the
    /// legacy HS256 transition key for inbound tokens. Backed by the
    /// PKCS#8 DER persisted under `system.db._secrets` so the public
    /// key (and JWKS `kid`) is stable across restarts.
    pub jwt: Arc<JwtIssuer>,
    /// In-process pub/sub broker for record CRUD events.
    pub broker: RealtimeBroker,
    /// Embedded JS hook runtime, keyed per (workspace, app).
    pub hooks: HookEngine,
    /// Root of the on-disk data tree. Used by handlers that need to
    /// resolve a workspace or app folder for delete / storage operations.
    pub data_dir: Arc<PathBuf>,
    /// Cached "has at least one master admin." Flipped from `false` to
    /// `true` exactly once when the setup wizard completes; on subsequent
    /// boots, populated from the DB count.
    pub initialized: Arc<AtomicBool>,
    /// Outbound mail. In tests this is a `LogMailer` that captures
    /// messages in memory; in production it's an SMTP-backed impl.
    pub mailer: Arc<dyn Mailer>,
    /// 32-byte KEK that encrypts at-rest secrets — currently the
    /// OAuth provider `client_secret`. Read from `RUSTBASE_KEK`, or
    /// from `system.db._secrets.oauth_kek` on installs that predate
    /// the variable.
    ///
    /// `None` when neither exists. The server still boots — most
    /// deployments never configure OAuth — but storing a secret is
    /// then refused: minting a key into the data directory would put
    /// it in the same file as the ciphertext, which protects nothing
    /// against whoever can read those files.
    pub oauth_kek: Arc<Option<[u8; 32]>>,
    /// File storage backend. Either a local directory rooted at
    /// `data_dir` or an S3-compatible bucket — picked by config at
    /// boot. Handlers use it with scoped keys of the form
    /// `workspaces/<workspace>/apps/<app>/storage/<file_id>`.
    pub storage: Storage,
    /// Per-subject failed-login counters that drive the auth lockout.
    /// Cloned cheaply (DashMap behind an `Arc`).
    pub login_attempts: LoginAttempts,
    /// Lockout thresholds applied by `login_attempts`. Loaded from
    /// `[lockout]` in `rustbase.toml` at boot.
    pub lockout_policy: LockoutPolicy,
    /// Whether the dashboard session cookies (`rb_at`, `rb_rt`)
    /// should be emitted with the `Secure` attribute. Defaults to
    /// `true` for production; flip to `false` for local-dev HTTP.
    pub cookie_secure: bool,
    /// Host allowlist for `$app.fetch` in JS hooks. Empty (default)
    /// = `$app.fetch` is disabled; the bridge throws `Forbidden`
    /// before any network IO. Loaded from `[hooks.fetch]` in
    /// `rustbase.toml` at boot.
    pub hook_fetch_allowed_hosts: Vec<String>,
}

impl AppState {
    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire)
    }

    pub fn mark_initialized(&self) {
        self.initialized.store(true, Ordering::Release);
    }
}
