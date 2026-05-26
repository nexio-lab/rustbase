use anyhow::Result;
use axum::{Router, routing::get};
use rustbase_api::{AppState, build_router};
use rustbase_auth::{RevocationSet, SigningKey};
use rustbase_db::{
    AppPoolManager, RealmPoolManager, SYSTEM_MIGRATIONS, SystemPool,
    admins::count_master_admins,
    apply_migrations,
    realms::ensure_master_realm,
    secrets::{MASTER_SIGNING_KEY, get_or_init_secret},
};
use rustbase_realtime::RealtimeBroker;
use rustbase_runtime::HookEngine;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tracing_subscriber::EnvFilter;

mod config;
mod dashboard;
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
        tracing::warn!(
            "no master admin found — only /healthz and POST /_/setup are reachable until setup completes"
        );
    }

    let fresh = SigningKey::generate();
    let key_bytes = get_or_init_secret(system.pool(), MASTER_SIGNING_KEY, fresh.as_bytes()).await?;
    let master_key = Arc::new(SigningKey::from_secret(&key_bytes));

    // OAuth client_secret encryption key. Generated once at first boot
    // and persisted; loaded as-is on subsequent boots so existing
    // ciphertexts in oauth_providers stay decryptable.
    let fresh_kek = rustbase_auth::fresh_kek();
    let kek_bytes = get_or_init_secret(system.pool(), "oauth_kek", &fresh_kek).await?;
    let oauth_kek: [u8; 32] = kek_bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("oauth_kek persisted at wrong length"))?;
    let oauth_kek = Arc::new(oauth_kek);

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

    // Pick a mailer: SMTP if `[mail.smtp]` is configured, otherwise a
    // capturing LogMailer that only writes to the tracing log. A bad
    // SMTP config aborts boot — silently downgrading would hide the
    // misconfiguration until the first verify-email request.
    let mailer: Arc<dyn rustbase_core::Mailer> = match &cfg.mail.smtp {
        Some(smtp_cfg) => {
            let smtp = rustbase_api::mailer::SmtpMailer::new(smtp_cfg)?;
            tracing::info!(
                host = %smtp_cfg.host,
                port = smtp_cfg.port,
                tls = ?smtp_cfg.tls,
                "mail: SMTP transport ready"
            );
            Arc::new(smtp)
        }
        None => {
            tracing::info!(
                "mail: no [mail.smtp] configured; using LogMailer \
                 (messages captured + logged, not delivered)"
            );
            Arc::new(rustbase_api::mailer::LogMailer::new())
        }
    };

    // File storage backend. S3 if `[storage.s3]` is configured, else
    // a local directory rooted at `data_dir`. Boot aborts on a bad
    // S3 config — silent fallback to local would hide misconfig until
    // the first upload.
    let storage = match &cfg.storage.s3 {
        Some(s3_cfg) => {
            tracing::info!(
                bucket = %s3_cfg.bucket,
                region = %s3_cfg.region,
                endpoint = ?s3_cfg.endpoint,
                "storage: S3 backend"
            );
            rustbase_storage::Storage::s3(s3_cfg)?
        }
        None => {
            tracing::info!(
                root = %cfg.data_dir.display(),
                "storage: local backend"
            );
            rustbase_storage::Storage::local(&cfg.data_dir).await?
        }
    };

    let state = AppState {
        system: Arc::new(system),
        realms: Arc::new(RealmPoolManager::new(
            cfg.data_dir.clone(),
            cfg.realm_pool_cap,
        )),
        apps: Arc::new(AppPoolManager::new(cfg.data_dir.clone(), cfg.app_pool_cap)),
        revocations: RevocationSet::default(),
        master_key,
        broker: RealtimeBroker::default(),
        hooks: HookEngine::new(),
        data_dir: Arc::new(cfg.data_dir.clone()),
        initialized: Arc::new(AtomicBool::new(already_initialized)),
        mailer,
        oauth_kek,
        storage,
    };

    // Load JS hooks for every (realm, app) that exists on disk.
    if let Err(e) = load_all_hooks(&state).await {
        tracing::error!(error = %e, "loading hooks at boot failed; continuing without them");
    }

    let dashboard_routes: Router<()> = Router::new()
        .route("/_/", get(dashboard::index))
        .route("/_/{*path}", get(dashboard::asset));
    let app = build_router(state).merge(dashboard_routes);
    let listener = tokio::net::TcpListener::bind(&cfg.listen).await?;
    tracing::info!(listen = %cfg.listen, "rustbase: ready");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Walk every realm + app that exists in `system.db`, and load JS
/// hooks from `data/hooks/<realm>/<app>/` if that directory exists.
/// Apps without a hooks directory are simply skipped.
async fn load_all_hooks(state: &rustbase_api::AppState) -> Result<()> {
    use rustbase_core::{AppId, RealmId};
    use rustbase_db::{apps::list_apps as db_list_apps, paths, realms::list_realms};

    let realms = list_realms(state.system.pool()).await?;
    for realm in realms {
        let realm_id = RealmId::from(realm.id.clone());
        // A realm row exists in system.db before its realm.db has been
        // initialized (master is created at boot, before any app).
        // Skip rather than try to read a not-yet-migrated DB.
        if !paths::realm_db(state.data_dir.as_ref(), &realm_id).exists() {
            continue;
        }
        let realm_pool = state.realms.pool_for(&realm_id).await?;
        let apps_in_realm = db_list_apps(&realm_pool).await?;
        for app in apps_in_realm {
            let app_id = AppId::from(app.id.clone());
            let dir = state
                .data_dir
                .join("hooks")
                .join(realm_id.as_str())
                .join(app_id.as_str());
            let bridge = rustbase_api::hook_bridge::ApiBridge::new(
                realm_id.clone(),
                app_id.clone(),
                state.apps.clone(),
            )
            .into_sync();
            let quoted = Arc::new(rustbase_api::mailer::QuotedMailer::new(
                state.mailer.clone(),
                realm_id.clone(),
                app_id.clone(),
                state.apps.clone(),
            )) as Arc<dyn rustbase_core::Mailer>;
            match state
                .hooks
                .load_app(&realm.id, &app.id, &dir, Some(bridge), Some(quoted))
                .await
            {
                Ok(n) if n > 0 => {
                    tracing::info!(
                        realm = %realm.id,
                        app = %app.id,
                        files = n,
                        "loaded JS hooks"
                    );
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(
                    realm = %realm.id,
                    app = %app.id,
                    error = %e,
                    "failed to load JS hooks"
                ),
            }
        }
    }
    Ok(())
}
