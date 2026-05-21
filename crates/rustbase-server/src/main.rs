use anyhow::Result;
use rustbase_api::{AppState, build_router};
use rustbase_auth::{RevocationSet, SigningKey};
use rustbase_db::{
    AppPoolManager, RealmPoolManager, SYSTEM_MIGRATIONS, SystemPool, admins::count_master_admins,
    apply_migrations, realms::ensure_master_realm,
    secrets::{MASTER_SIGNING_KEY, get_or_init_secret},
};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tracing_subscriber::EnvFilter;

mod config;
mod litestream;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cfg = config::load()?;
    tracing::info!(?cfg, "rustbase: starting");

    tokio::fs::create_dir_all(&cfg.data_dir).await?;

    let system = SystemPool::open(&cfg.data_dir).await?;
    let applied = apply_migrations(system.pool().clone(), SYSTEM_MIGRATIONS).await?;
    if applied > 0 {
        tracing::info!(applied, "system migrations applied");
    }
    ensure_master_realm(system.pool()).await?;
    let already_initialized = count_master_admins(system.pool()).await? > 0;
    if !already_initialized {
        tracing::warn!("no master admin found — only /healthz and POST /_/setup are reachable until setup completes");
    }

    let fresh = SigningKey::generate();
    let key_bytes =
        get_or_init_secret(system.pool(), MASTER_SIGNING_KEY, fresh.as_bytes()).await?;
    let master_key = Arc::new(SigningKey::from_secret(&key_bytes));

    // Optional: generate litestream.yml at boot when replication is enabled.
    if cfg.litestream.enabled {
        match litestream::write_yaml(&cfg.data_dir, &cfg.litestream).await {
            Ok(path) => {
                tracing::info!(
                    path = %path.display(),
                    "litestream.yml generated — run `litestream replicate -config {}` to start replication",
                    path.display()
                );
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to generate litestream.yml");
            }
        }
    }

    let state = AppState {
        system: Arc::new(system),
        realms: Arc::new(RealmPoolManager::new(cfg.data_dir.clone(), cfg.realm_pool_cap)),
        apps: Arc::new(AppPoolManager::new(cfg.data_dir.clone(), cfg.app_pool_cap)),
        revocations: RevocationSet::default(),
        master_key,
        data_dir: Arc::new(cfg.data_dir.clone()),
        initialized: Arc::new(AtomicBool::new(already_initialized)),
    };

    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(&cfg.listen).await?;
    tracing::info!(listen = %cfg.listen, "rustbase: ready");
    axum::serve(listener, app).await?;
    Ok(())
}
