//! Auth handlers and extractors.
//!
//! - `login.rs` — `POST /_/auth/admin/login`
//! - `refresh.rs` — `POST /_/auth/refresh`
//! - `extract.rs` — `MasterAdminAuth` axum extractor
//!
//! Workspace-admin and end-user flows come later, on their own feature branches.

pub mod audit_events;
pub mod cookies;
pub mod email_otp;
pub mod extract;
pub mod jwks;
pub mod login;
pub mod logout;
pub mod oauth;
pub mod oauth_admin;
pub mod password_reset;
pub mod refresh;
pub mod register;
pub mod totp;
pub mod verify_email;

pub use extract::{AdminAuth, PrincipalAuth};
pub use login::{master_admin_login, user_login, workspace_admin_login};
pub use refresh::{master_admin_refresh, user_refresh, workspace_admin_refresh};
pub use register::user_register;

use rand_core::{OsRng, RngCore};
use rustbase_core::{CoreError, WorkspaceId};
use rustbase_db::{apps::find_app, workspaces::find_workspace};

use crate::error::ApiError;
use crate::state::AppState;

/// Verify that `(workspace, app)` exists. Used by every record / file
/// / collection / per-app handler: paths come from the URL and must
/// be validated before we touch the per-app pool.
pub async fn require_app_exists(
    state: &AppState,
    workspace: &str,
    app: &str,
) -> Result<(), ApiError> {
    require_workspace_exists(state, workspace).await?;
    let workspace_pool = state
        .workspaces
        .pool_for(&WorkspaceId::from(workspace.to_string()))
        .await?;
    find_app(&workspace_pool, app)
        .await?
        .ok_or(ApiError::Core(CoreError::AppNotFound {
            workspace: workspace.to_string(),
            app: app.to_string(),
        }))?;
    Ok(())
}

/// Verify that `workspace` exists. Used by every workspace-scoped
/// end-user / OAuth handler before touching the workspace pool.
pub async fn require_workspace_exists(state: &AppState, workspace: &str) -> Result<(), ApiError> {
    find_workspace(state.system.pool(), workspace)
        .await?
        .ok_or(ApiError::Core(CoreError::WorkspaceNotFound(
            workspace.to_string(),
        )))?;
    Ok(())
}

/// Generate an opaque refresh token (64 hex chars from 32 random bytes,
/// prefixed with `rfsh_` for greppability in logs).
pub fn new_refresh_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let mut out = String::with_capacity(5 + 64);
    out.push_str("rfsh_");
    for b in &bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Default access-token TTL until the policy engine surfaces a
/// configurable value (15 min per the design spec).
pub fn default_access_ttl() -> chrono::Duration {
    chrono::Duration::minutes(15)
}

/// Default refresh-token TTL (30 days).
pub fn default_refresh_ttl() -> chrono::Duration {
    chrono::Duration::days(30)
}
