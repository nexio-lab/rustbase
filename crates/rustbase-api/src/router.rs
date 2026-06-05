use axum::{
    Router,
    routing::{any, get, post},
};
use tower_http::trace::TraceLayer;

use crate::access_rules;
use crate::apps;
use crate::audit;
use crate::auth::{
    master_admin_login, master_admin_refresh, user_login, user_refresh, user_register,
    workspace_admin_login, workspace_admin_refresh,
};
use crate::collections;
use crate::custom_routes;
use crate::files;
use crate::health::healthz;
use crate::hooks;
use crate::middleware::setup_gate;
use crate::policies;
use crate::realtime;
use crate::records;
use crate::setup::setup;
use crate::state::AppState;
use crate::workspace_admins;
use crate::workspaces;

/// Build the full RustBase HTTP router. Layered with a setup gate (blocks
/// non-bootstrap routes while uninitialized) and a tracing middleware so
/// every request shows up in the access log.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/_/setup", post(setup))
        .route("/_/auth/admin/login", post(master_admin_login))
        .route("/_/auth/refresh", post(master_admin_refresh))
        .route("/_/auth/logout", post(crate::auth::logout::logout))
        .route("/_/auth/jwks.json", get(crate::auth::jwks::jwks))
        .route("/.well-known/jwks.json", get(crate::auth::jwks::jwks))
        .route(
            "/api/workspaces",
            get(workspaces::list).post(workspaces::create),
        )
        .route(
            "/api/workspaces/{id}",
            get(workspaces::get)
                .patch(workspaces::update)
                .delete(workspaces::delete),
        )
        .route(
            "/api/workspaces/{workspace}/admins",
            post(workspace_admins::create),
        )
        .route(
            "/api/workspaces/{workspace}/auth/admin/login",
            post(workspace_admin_login),
        )
        .route(
            "/api/workspaces/{workspace}/auth/refresh",
            post(workspace_admin_refresh),
        )
        // Workspace-shared end-user identity. Users live at workspace
        // scope (one (email, workspace) pair across every app) and
        // every auth flow is workspace-scoped to match.
        .route(
            "/api/workspaces/{workspace}/auth/users/register",
            post(user_register),
        )
        .route(
            "/api/workspaces/{workspace}/auth/users/login",
            post(user_login),
        )
        .route(
            "/api/workspaces/{workspace}/auth/users/refresh",
            post(user_refresh),
        )
        .route(
            "/api/workspaces/{workspace}/auth/verify-email/request",
            post(crate::auth::verify_email::request),
        )
        .route(
            "/api/workspaces/{workspace}/auth/verify-email/confirm",
            post(crate::auth::verify_email::confirm),
        )
        .route(
            "/api/workspaces/{workspace}/auth/password-reset/request",
            post(crate::auth::password_reset::request),
        )
        .route(
            "/api/workspaces/{workspace}/auth/password-reset/confirm",
            post(crate::auth::password_reset::confirm),
        )
        .route(
            "/api/workspaces/{workspace}/auth/otp/request",
            post(crate::auth::email_otp::request),
        )
        .route(
            "/api/workspaces/{workspace}/auth/otp/login",
            post(crate::auth::email_otp::login),
        )
        .route(
            "/api/workspaces/{workspace}/auth/totp/enroll",
            post(crate::auth::totp::enroll),
        )
        .route(
            "/api/workspaces/{workspace}/auth/totp/confirm",
            post(crate::auth::totp::confirm),
        )
        .route(
            "/api/workspaces/{workspace}/auth/totp/disable",
            post(crate::auth::totp::disable),
        )
        .route(
            "/api/workspaces/{workspace}/auth/users/login/totp",
            post(crate::auth::totp::login_totp),
        )
        .route(
            "/api/workspaces/{workspace}/auth/oauth/{provider}/authorize",
            get(crate::auth::oauth::authorize),
        )
        .route(
            "/api/workspaces/{workspace}/auth/oauth/{provider}/callback",
            post(crate::auth::oauth::callback),
        )
        .route(
            "/api/workspaces/{workspace}/auth/oauth/providers",
            get(crate::auth::oauth_admin::list),
        )
        .route(
            "/api/workspaces/{workspace}/auth/oauth/providers/{provider}",
            get(crate::auth::oauth_admin::get)
                .put(crate::auth::oauth_admin::put)
                .delete(crate::auth::oauth_admin::delete),
        )
        // Admin end-user management. Gated by
        // AdminAuth::require_workspace_access inside each handler.
        .route("/api/workspaces/{workspace}/users", get(crate::users::list))
        .route(
            "/api/workspaces/{workspace}/users/{id}",
            get(crate::users::get).delete(crate::users::delete),
        )
        .route(
            "/api/workspaces/{workspace}/users/{id}/verify",
            axum::routing::patch(crate::users::verify),
        )
        .route(
            "/api/workspaces/{workspace}/users/{id}/totp",
            axum::routing::delete(crate::users::reset_totp),
        )
        .route(
            "/api/workspaces/{workspace}/apps",
            get(apps::list).post(apps::create),
        )
        .route(
            "/api/workspaces/{workspace}/apps/{app}",
            get(apps::get).patch(apps::update).delete(apps::delete),
        )
        .route(
            "/api/workspaces/{workspace}/apps/{app}/collections",
            get(collections::list).post(collections::create),
        )
        .route(
            "/api/workspaces/{workspace}/apps/{app}/collections/{name}",
            get(collections::get)
                .patch(collections::patch)
                .delete(collections::delete),
        )
        .route(
            "/api/workspaces/{workspace}/apps/{app}/collections/{coll}/records",
            get(records::list).post(records::create),
        )
        .route(
            "/api/workspaces/{workspace}/apps/{app}/collections/{coll}/records/{id}",
            get(records::get)
                .patch(records::update)
                .delete(records::delete),
        )
        .route(
            "/api/workspaces/{workspace}/apps/{app}/collections/{coll}/access_rules",
            get(access_rules::list),
        )
        .route(
            "/api/workspaces/{workspace}/apps/{app}/collections/{coll}/access_rules/{action}",
            axum::routing::put(access_rules::put).delete(access_rules::delete),
        )
        // file endpoints
        .route(
            "/api/workspaces/{workspace}/apps/{app}/files",
            get(files::list).post(files::upload),
        )
        .route(
            "/api/workspaces/{workspace}/apps/{app}/files/{id}",
            get(files::download).delete(files::delete),
        )
        .route(
            "/api/workspaces/{workspace}/apps/{app}/files/{id}/meta",
            get(files::meta),
        )
        // realtime SSE
        .route(
            "/api/workspaces/{workspace}/apps/{app}/collections/{coll}/events",
            get(realtime::record_events),
        )
        // Custom JS-defined endpoints. The wildcard catches anything
        // under `/custom/`; the handler delegates to the JS shim's
        // `$app.routerAdd` table and returns 404 if no JS handler
        // is registered for (method, path).
        .route(
            "/api/workspaces/{workspace}/apps/{app}/custom/{*path}",
            any(custom_routes::handle),
        )
        // policy endpoints
        .route("/api/system/policies", get(policies::system_list))
        .route(
            "/api/system/policies/{field}",
            get(policies::system_get)
                .put(policies::system_put)
                .delete(policies::system_delete),
        )
        .route(
            "/api/workspaces/{workspace}/policies",
            get(policies::workspace_list),
        )
        .route(
            "/api/workspaces/{workspace}/policies/{field}",
            get(policies::workspace_get)
                .put(policies::workspace_put)
                .delete(policies::workspace_delete),
        )
        .route(
            "/api/workspaces/{workspace}/apps/{app}/policies",
            get(policies::app_list),
        )
        .route(
            "/api/workspaces/{workspace}/apps/{app}/policies/{field}",
            get(policies::app_get)
                .put(policies::app_put)
                .delete(policies::app_delete),
        )
        // audit log — read-only per scope
        .route("/api/system/audit", get(audit::system_list))
        .route(
            "/api/workspaces/{workspace}/audit",
            get(audit::workspace_list),
        )
        .route(
            "/api/workspaces/{workspace}/apps/{app}/audit",
            get(audit::app_list),
        )
        // hook source files — read/write/reload
        .route(
            "/api/workspaces/{workspace}/apps/{app}/hooks",
            get(hooks::list),
        )
        .route(
            "/api/workspaces/{workspace}/apps/{app}/hooks/reload",
            post(hooks::reload),
        )
        .route(
            "/api/workspaces/{workspace}/apps/{app}/hooks/{filename}",
            get(hooks::get).put(hooks::put).delete(hooks::delete),
        )
        // /api/workspaces/<workspace>/apps/<app>/... will mount under here once
        // collections / records handlers land.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            setup_gate,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[cfg(test)]
#[path = "router_tests/mod.rs"]
mod tests;
