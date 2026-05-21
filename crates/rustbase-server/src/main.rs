use anyhow::Result;
use rustbase_api::{AppState, build_router};
use rustbase_auth::RevocationSet;
use rustbase_db::{
    AppPoolManager, RealmPoolManager, SYSTEM_MIGRATIONS, SystemPool, apply_migrations,
};
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Debug)]
struct ServerConfig {
    listen: String,
    data_dir: PathBuf,
    realm_pool_cap: usize,
    app_pool_cap: usize,
}

impl ServerConfig {
    fn from_env() -> Self {
        Self {
            listen: std::env::var("RUSTBASE_LISTEN").unwrap_or_else(|_| "0.0.0.0:8080".into()),
            data_dir: std::env::var("RUSTBASE_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("./data")),
            realm_pool_cap: parse_env_usize("RUSTBASE_REALM_POOL_CAP", 32),
            app_pool_cap: parse_env_usize("RUSTBASE_APP_POOL_CAP", 64),
        }
    }
}

fn parse_env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cfg = ServerConfig::from_env();
    tracing::info!(?cfg, "rustbase: starting");

    tokio::fs::create_dir_all(&cfg.data_dir).await?;

    let system = SystemPool::open(&cfg.data_dir).await?;
    let applied = apply_migrations(system.pool(), SYSTEM_MIGRATIONS).await?;
    if applied > 0 {
        tracing::info!(applied, "system migrations applied");
    }

    let state = AppState {
        system: Arc::new(system),
        realms: Arc::new(RealmPoolManager::new(cfg.data_dir.clone(), cfg.realm_pool_cap)),
        apps: Arc::new(AppPoolManager::new(cfg.data_dir.clone(), cfg.app_pool_cap)),
        revocations: RevocationSet::default(),
    };

    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(&cfg.listen).await?;
    tracing::info!(listen = %cfg.listen, "rustbase: ready");
    axum::serve(listener, app).await?;
    Ok(())
}
