//! Auth-event audit helpers.
//!
//! Every login attempt produces exactly one row in the appropriate
//! `audit_log`:
//!
//! - `login_success` — credential + (when applicable) MFA accepted, tokens issued.
//! - `login_failed` — credentials rejected. Subject left unlocked.
//! - `login_locked` — N-th rejection inside the policy window; subject
//!   is now locked for `lockout_secs`.
//!
//! Master-scope events go to `system.db`; realm-admin and end-user
//! events go to the realm's `realm.db`. App-scoped auth events stay
//! under the realm log so an app admin can see the auth attempts
//! against users that span its app.
//!
//! All emissions are best-effort: the audit append happens after the
//! credential path's primary return value is already settled, and a
//! failure to write the audit row is logged but does not change the
//! HTTP response.

use rustbase_core::{CoreError, RealmId};
use rustbase_db::audit::append;
use serde_json::Value;
use sqlx::SqlitePool;

use crate::state::AppState;

#[derive(Debug, Clone, Copy)]
pub enum AuthOutcome {
    Success,
    Failed,
    Locked,
}

impl AuthOutcome {
    fn action(self) -> &'static str {
        match self {
            AuthOutcome::Success => "login_success",
            AuthOutcome::Failed => "login_failed",
            AuthOutcome::Locked => "login_locked",
        }
    }
}

#[derive(Debug, Clone)]
pub enum Scope<'a> {
    Master,
    Realm(&'a str),
}

#[derive(Debug, Clone)]
pub struct AuthEvent<'a> {
    pub outcome: AuthOutcome,
    pub scope: Scope<'a>,
    /// Stable subject key — same shape used for the lockout map.
    pub subject: &'a str,
    /// Display target (`username`, `email`, etc.) — what shows up in
    /// the dashboard's audit list.
    pub target: Option<&'a str>,
    /// Optional extras (e.g. `{"flow":"totp"}`) — never include
    /// passwords or hash material.
    pub details: Value,
}

/// Best-effort: write an audit row in the scope appropriate for the
/// event. Errors are swallowed at the boundary (they should not break
/// the user-facing response), but they are logged.
pub async fn record(state: &AppState, ev: AuthEvent<'_>) {
    let res = match ev.scope {
        Scope::Master => append_with_log(state.system.pool(), &ev).await,
        Scope::Realm(realm) => match state
            .realms
            .pool_for(&RealmId::from(realm.to_string()))
            .await
        {
            Ok(pool) => append_with_log(&pool, &ev).await,
            Err(e) => Err(CoreError::Internal(format!(
                "audit realm pool open failed: {e}"
            ))),
        },
    };
    if let Err(e) = res {
        tracing::warn!(error = %e, subject = ev.subject, action = ev.outcome.action(), "audit append failed");
    }
}

async fn append_with_log(pool: &SqlitePool, ev: &AuthEvent<'_>) -> rustbase_core::Result<()> {
    append(
        pool,
        Some(ev.subject),
        ev.outcome.action(),
        ev.target,
        &ev.details,
    )
    .await
    .map(|_| ())
    .map_err(CoreError::from)
}
