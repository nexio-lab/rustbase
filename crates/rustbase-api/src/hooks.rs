//! Endpoints for managing JS/TS hook source files.
//!
//! - `GET    /api/realms/:realm/apps/:app/hooks`
//! - `GET    /api/realms/:realm/apps/:app/hooks/:filename`
//! - `PUT    /api/realms/:realm/apps/:app/hooks/:filename`     write + reload
//! - `DELETE /api/realms/:realm/apps/:app/hooks/:filename`     delete + reload
//! - `POST   /api/realms/:realm/apps/:app/hooks/reload`        reload without writing
//!
//! Files live on disk under `data/hooks/<realm>/<app>/`. Every mutating
//! call rebuilds the app's `AppHooks` via `HookEngine::load_app` and
//! returns any errors that piled up during script evaluation — the
//! dashboard surfaces them next to the editor so a syntax error doesn't
//! silently kill the runtime.

use axum::{
    Json,
    extract::{Path, State},
};
use rustbase_core::{AppId, CoreError, RealmId};
use rustbase_db::{apps::find_app, realms::find_realm};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use crate::auth::AdminAuth;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct HookFile {
    pub filename: String,
    pub size: u64,
    /// RFC3339 mtime of the underlying file.
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct HookFileBody {
    pub filename: String,
    pub source: String,
    pub size: u64,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct PutHookBody {
    pub source: String,
}

#[derive(Debug, Serialize)]
pub struct ReloadOutcome {
    /// Number of files successfully loaded by the runtime.
    pub loaded: usize,
    /// Per-file evaluation errors drained from the runtime right after
    /// the reload. Empty means everything compiled and registered cleanly.
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct PutHookResponse {
    pub file: HookFileBody,
    pub reload: ReloadOutcome,
}

pub async fn list(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, app)): Path<(String, String)>,
) -> Result<Json<Vec<HookFile>>, ApiError> {
    auth.require_app_access(&realm, &app)?;
    require_app_exists(&state, &realm, &app).await?;

    let dir = hooks_dir(&state, &realm, &app);
    let mut files = Vec::new();
    if dir.exists() {
        let mut rd = tokio::fs::read_dir(&dir).await.map_err(io_err)?;
        while let Some(entry) = rd.next_entry().await.map_err(io_err)? {
            let path = entry.path();
            if !is_hook_file(&path) {
                continue;
            }
            let meta = entry.metadata().await.map_err(io_err)?;
            let filename = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            files.push(HookFile {
                filename,
                size: meta.len(),
                updated_at: mtime_rfc3339(meta.modified().ok()),
            });
        }
    }
    files.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(Json(files))
}

pub async fn get(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, app, filename)): Path<(String, String, String)>,
) -> Result<Json<HookFileBody>, ApiError> {
    auth.require_app_access(&realm, &app)?;
    require_app_exists(&state, &realm, &app).await?;
    validate_filename(&filename)?;

    let path = hooks_dir(&state, &realm, &app).join(&filename);
    if !path.exists() {
        return Err(ApiError::Core(CoreError::NotFound {
            collection: "hook".into(),
            id: filename,
        }));
    }
    let bytes = tokio::fs::read(&path).await.map_err(io_err)?;
    let meta = tokio::fs::metadata(&path).await.map_err(io_err)?;
    let source = String::from_utf8(bytes)
        .map_err(|e| ApiError::Core(CoreError::Internal(format!("hook is not utf-8: {e}"))))?;
    Ok(Json(HookFileBody {
        filename,
        size: meta.len(),
        updated_at: mtime_rfc3339(meta.modified().ok()),
        source,
    }))
}

pub async fn put(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, app, filename)): Path<(String, String, String)>,
    Json(body): Json<PutHookBody>,
) -> Result<Json<PutHookResponse>, ApiError> {
    auth.require_app_access(&realm, &app)?;
    require_app_exists(&state, &realm, &app).await?;
    validate_filename(&filename)?;

    let dir = hooks_dir(&state, &realm, &app);
    tokio::fs::create_dir_all(&dir).await.map_err(io_err)?;
    let path = dir.join(&filename);
    tokio::fs::write(&path, body.source.as_bytes())
        .await
        .map_err(io_err)?;

    let reload = reload_app(&state, &realm, &app).await?;
    let meta = tokio::fs::metadata(&path).await.map_err(io_err)?;
    Ok(Json(PutHookResponse {
        file: HookFileBody {
            filename,
            size: meta.len(),
            updated_at: mtime_rfc3339(meta.modified().ok()),
            source: body.source,
        },
        reload,
    }))
}

pub async fn delete(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, app, filename)): Path<(String, String, String)>,
) -> Result<Json<ReloadOutcome>, ApiError> {
    auth.require_app_access(&realm, &app)?;
    require_app_exists(&state, &realm, &app).await?;
    validate_filename(&filename)?;

    let path = hooks_dir(&state, &realm, &app).join(&filename);
    if !path.exists() {
        return Err(ApiError::Core(CoreError::NotFound {
            collection: "hook".into(),
            id: filename,
        }));
    }
    tokio::fs::remove_file(&path).await.map_err(io_err)?;

    let reload = reload_app(&state, &realm, &app).await?;
    Ok(Json(reload))
}

pub async fn reload(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path((realm, app)): Path<(String, String)>,
) -> Result<Json<ReloadOutcome>, ApiError> {
    auth.require_app_access(&realm, &app)?;
    require_app_exists(&state, &realm, &app).await?;
    let outcome = reload_app(&state, &realm, &app).await?;
    Ok(Json(outcome))
}

// ----- helpers -----

fn hooks_dir(state: &AppState, realm: &str, app: &str) -> PathBuf {
    state.data_dir.join("hooks").join(realm).join(app)
}

/// Reject anything that could escape the hooks directory or fool the JS
/// loader. Names must look like `<id>.{js,ts}` and contain only safe
/// characters.
fn validate_filename(name: &str) -> Result<(), ApiError> {
    if name.is_empty() || name.len() > 255 {
        return Err(invalid("filename must be 1-255 characters"));
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(invalid("filename must not contain path separators or '..'"));
    }
    let lower = name.to_ascii_lowercase();
    if !(lower.ends_with(".js") || lower.ends_with(".ts")) {
        return Err(invalid("filename must end in .js or .ts"));
    }
    // First char must be a letter/digit/underscore so dotfiles can't be
    // written. The middle can include `-` and `.` (e.g. `cron.ts`).
    let bytes = name.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() && bytes[0] != b'_' {
        return Err(invalid("filename must start with a letter, digit, or '_'"));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err(invalid(
            "filename may only contain letters, digits, '.', '_' and '-'",
        ));
    }
    Ok(())
}

fn invalid(msg: &str) -> ApiError {
    ApiError::Core(CoreError::Validation(msg.into()))
}

fn io_err(e: std::io::Error) -> ApiError {
    ApiError::Core(CoreError::Internal(format!("hook fs error: {e}")))
}

fn is_hook_file(p: &std::path::Path) -> bool {
    match p.extension().and_then(|s| s.to_str()) {
        Some(ext) => {
            let l = ext.to_ascii_lowercase();
            l == "js" || l == "ts"
        }
        None => false,
    }
}

fn mtime_rfc3339(mt: Option<SystemTime>) -> String {
    match mt {
        Some(t) => chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339(),
        None => String::new(),
    }
}

async fn require_app_exists(state: &AppState, realm: &str, app: &str) -> Result<(), ApiError> {
    find_realm(state.system.pool(), realm)
        .await?
        .ok_or(ApiError::Core(CoreError::RealmNotFound(realm.to_string())))?;
    let pool = state
        .realms
        .pool_for(&RealmId::from(realm.to_string()))
        .await?;
    find_app(&pool, app)
        .await?
        .ok_or(ApiError::Core(rustbase_core::CoreError::AppNotFound {
            realm: realm.to_string(),
            app: app.to_string(),
        }))?;
    Ok(())
}

/// Rebuild the app's `AppHooks` from disk and return any per-script
/// errors the runtime captured. Bridge + mailer wiring mirrors what
/// `apps::create` does on first load — they must stay in sync.
async fn reload_app(state: &AppState, realm: &str, app: &str) -> Result<ReloadOutcome, ApiError> {
    let dir = hooks_dir(state, realm, app);
    let bridge = crate::hook_bridge::ApiBridge::new(
        RealmId::from(realm.to_string()),
        AppId::from(app.to_string()),
        state.apps.clone(),
    )
    .into_sync();
    let quoted = Arc::new(crate::mailer::QuotedMailer::new(
        state.mailer.clone(),
        RealmId::from(realm.to_string()),
        AppId::from(app.to_string()),
        state.apps.clone(),
    )) as Arc<dyn rustbase_core::Mailer>;

    let loaded = state
        .hooks
        .load_app(realm, app, &dir, Some(bridge), Some(quoted))
        .await
        .map_err(|e| ApiError::Core(CoreError::Internal(format!("hook reload failed: {e}"))))?;

    // Drain anything user scripts logged via `__rb_errors` during load.
    // `get(realm, app)` is `Some` immediately after `load_app`.
    let errors = match state.hooks.get(realm, app) {
        Some(h) => h.drain_errors().await.unwrap_or_default(),
        None => Vec::new(),
    };
    Ok(ReloadOutcome { loaded, errors })
}
