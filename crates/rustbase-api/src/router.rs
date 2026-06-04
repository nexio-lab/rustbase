use axum::{
    Router,
    routing::{any, get, post},
};
use tower_http::trace::TraceLayer;

use crate::access_rules;
use crate::apps;
use crate::audit;
use crate::auth::{
    master_admin_login, master_admin_refresh, realm_admin_login, realm_admin_refresh, user_login,
    user_refresh, user_register,
};
use crate::collections;
use crate::custom_routes;
use crate::files;
use crate::health::healthz;
use crate::hooks;
use crate::middleware::setup_gate;
use crate::policies;
use crate::realm_admins;
use crate::realms;
use crate::realtime;
use crate::records;
use crate::setup::setup;
use crate::state::AppState;

/// Build the full RustBase HTTP router. Layered with a setup gate (blocks
/// non-bootstrap routes while uninitialized) and a tracing middleware so
/// every request shows up in the access log.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/_/setup", post(setup))
        .route("/_/auth/admin/login", post(master_admin_login))
        .route("/_/auth/refresh", post(master_admin_refresh))
        .route("/api/realms", get(realms::list).post(realms::create))
        .route(
            "/api/realms/{id}",
            get(realms::get)
                .patch(realms::update)
                .delete(realms::delete),
        )
        .route("/api/realms/{realm}/admins", post(realm_admins::create))
        .route(
            "/api/realms/{realm}/auth/admin/login",
            post(realm_admin_login),
        )
        .route(
            "/api/realms/{realm}/auth/refresh",
            post(realm_admin_refresh),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/auth/users/register",
            post(user_register),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/auth/users/login",
            post(user_login),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/auth/users/refresh",
            post(user_refresh),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/auth/verify-email/request",
            post(crate::auth::verify_email::request),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/auth/verify-email/confirm",
            post(crate::auth::verify_email::confirm),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/auth/password-reset/request",
            post(crate::auth::password_reset::request),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/auth/password-reset/confirm",
            post(crate::auth::password_reset::confirm),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/auth/otp/request",
            post(crate::auth::email_otp::request),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/auth/otp/login",
            post(crate::auth::email_otp::login),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/auth/totp/enroll",
            post(crate::auth::totp::enroll),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/auth/totp/confirm",
            post(crate::auth::totp::confirm),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/auth/totp/disable",
            post(crate::auth::totp::disable),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/auth/users/login/totp",
            post(crate::auth::totp::login_totp),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/auth/oauth/{provider}/authorize",
            get(crate::auth::oauth::authorize),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/auth/oauth/{provider}/callback",
            post(crate::auth::oauth::callback),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/auth/oauth/providers",
            get(crate::auth::oauth_admin::list),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/auth/oauth/providers/{provider}",
            get(crate::auth::oauth_admin::get)
                .put(crate::auth::oauth_admin::put)
                .delete(crate::auth::oauth_admin::delete),
        )
        // Admin end-user management. Gated by AdminAuth::require_app_access
        // inside each handler; self-service flows under /auth/users stay
        // separate and untouched.
        .route(
            "/api/realms/{realm}/apps/{app}/users",
            get(crate::users::list),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/users/{id}",
            get(crate::users::get).delete(crate::users::delete),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/users/{id}/verify",
            axum::routing::patch(crate::users::verify),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/users/{id}/totp",
            axum::routing::delete(crate::users::reset_totp),
        )
        .route(
            "/api/realms/{realm}/apps",
            get(apps::list).post(apps::create),
        )
        .route(
            "/api/realms/{realm}/apps/{app}",
            get(apps::get).patch(apps::update).delete(apps::delete),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/collections",
            get(collections::list).post(collections::create),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/collections/{name}",
            get(collections::get)
                .patch(collections::patch)
                .delete(collections::delete),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/collections/{coll}/records",
            get(records::list).post(records::create),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/collections/{coll}/records/{id}",
            get(records::get)
                .patch(records::update)
                .delete(records::delete),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/collections/{coll}/access_rules",
            get(access_rules::list),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/collections/{coll}/access_rules/{action}",
            axum::routing::put(access_rules::put).delete(access_rules::delete),
        )
        // file endpoints
        .route(
            "/api/realms/{realm}/apps/{app}/files",
            get(files::list).post(files::upload),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/files/{id}",
            get(files::download).delete(files::delete),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/files/{id}/meta",
            get(files::meta),
        )
        // realtime SSE
        .route(
            "/api/realms/{realm}/apps/{app}/collections/{coll}/events",
            get(realtime::record_events),
        )
        // Custom JS-defined endpoints. The wildcard catches anything
        // under `/custom/`; the handler delegates to the JS shim's
        // `$app.routerAdd` table and returns 404 if no JS handler
        // is registered for (method, path).
        .route(
            "/api/realms/{realm}/apps/{app}/custom/{*path}",
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
        .route("/api/realms/{realm}/policies", get(policies::realm_list))
        .route(
            "/api/realms/{realm}/policies/{field}",
            get(policies::realm_get)
                .put(policies::realm_put)
                .delete(policies::realm_delete),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/policies",
            get(policies::app_list),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/policies/{field}",
            get(policies::app_get)
                .put(policies::app_put)
                .delete(policies::app_delete),
        )
        // audit log — read-only per scope
        .route("/api/system/audit", get(audit::system_list))
        .route("/api/realms/{realm}/audit", get(audit::realm_list))
        .route("/api/realms/{realm}/apps/{app}/audit", get(audit::app_list))
        // hook source files — read/write/reload
        .route("/api/realms/{realm}/apps/{app}/hooks", get(hooks::list))
        .route(
            "/api/realms/{realm}/apps/{app}/hooks/reload",
            post(hooks::reload),
        )
        .route(
            "/api/realms/{realm}/apps/{app}/hooks/{filename}",
            get(hooks::get).put(hooks::put).delete(hooks::delete),
        )
        // /api/realms/<realm>/apps/<app>/... will mount under here once
        // collections / records handlers land.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            setup_gate,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use rustbase_auth::{RevocationSet, SigningKey};
    use rustbase_db::{
        AppPoolManager, RealmPoolManager, SYSTEM_MIGRATIONS, SystemPool,
        admins::ensure_seed_master_admin, apply_migrations, realms::ensure_master_realm,
    };
    use rustbase_realtime::RealtimeBroker;
    use rustbase_runtime::HookEngine;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use tempfile::tempdir;
    use tower::ServiceExt;

    async fn fresh_state() -> (AppState, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let system = SystemPool::open(dir.path()).await.unwrap();
        apply_migrations(system.pool().clone(), SYSTEM_MIGRATIONS)
            .await
            .unwrap();
        ensure_master_realm(system.pool()).await.unwrap();
        // Seed the auto-created master admin row. Production bootstrap
        // does this in `rustbase-server`; tests must mirror it so
        // `/_/setup` and login/refresh paths line up with reality.
        ensure_seed_master_admin(system.pool()).await.unwrap();
        let data_dir = dir.path().to_path_buf();
        let storage = rustbase_storage::Storage::local(&data_dir).await.unwrap();
        let state = AppState {
            system: Arc::new(system),
            realms: Arc::new(RealmPoolManager::new(data_dir.clone(), 4)),
            apps: Arc::new(AppPoolManager::new(data_dir.clone(), 4)),
            revocations: RevocationSet::default(),
            master_key: Arc::new(SigningKey::generate()),
            broker: RealtimeBroker::default(),
            hooks: HookEngine::new(),
            data_dir: Arc::new(data_dir),
            initialized: Arc::new(AtomicBool::new(false)),
            mailer: Arc::new(crate::mailer::LogMailer::new()),
            oauth_kek: Arc::new(rustbase_auth::fresh_kek()),
            storage,
            login_attempts: crate::security::LoginAttempts::new(),
            lockout_policy: crate::security::LockoutPolicy::default(),
        };
        (state, dir)
    }

    async fn json_body(resp: axum::response::Response) -> serde_json::Value {
        let body = to_bytes(resp.into_body(), 4096).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn healthz_returns_uninitialized_on_fresh_install() {
        let (state, _dir) = fresh_state().await;
        let app = build_router(state);
        let req = Request::builder()
            .uri("/healthz")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert_eq!(j["initialized"], false);
    }

    #[tokio::test]
    async fn unknown_route_is_blocked_with_503_while_uninitialized() {
        let (state, _dir) = fresh_state().await;
        let app = build_router(state);
        let req = Request::builder()
            .uri("/some/path")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let j = json_body(resp).await;
        assert_eq!(j["code"], "uninitialized");
    }

    #[tokio::test]
    async fn setup_creates_master_admin_and_unlocks_the_server() {
        let (state, _dir) = fresh_state().await;
        let app = build_router(state.clone());

        // Setup only carries a password — the seed admin's username
        // ("admin") is fixed at boot.
        let body = serde_json::json!({ "password": "supersecret" });
        let req = Request::builder()
            .uri("/_/setup")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let j = json_body(resp).await;
        assert_eq!(j["username"], "admin");
        assert!(state.is_initialized());

        // Healthz now reports initialized=true.
        let app2 = build_router(state.clone());
        let req = Request::builder()
            .uri("/healthz")
            .body(Body::empty())
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        let j = json_body(resp).await;
        assert_eq!(j["initialized"], true);
    }

    #[tokio::test]
    async fn setup_rejects_short_password() {
        let (state, _dir) = fresh_state().await;
        let app = build_router(state);
        let body = serde_json::json!({ "password": "short" });
        let req = Request::builder()
            .uri("/_/setup")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn second_setup_returns_409_conflict() {
        let (state, _dir) = fresh_state().await;
        // First call completes the wizard.
        let app = build_router(state.clone());
        let body = serde_json::json!({ "password": "supersecret" });
        let req = Request::builder()
            .uri("/_/setup")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        // The setup gate now closes setup behind the setup_gate
        // middleware (only /healthz and /_/setup were open before),
        // and a second call is rejected with 409 by the handler.
        let app = build_router(state);
        let body = serde_json::json!({ "password": "anothersecret" });
        let req = Request::builder()
            .uri("/_/setup")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
        let j = json_body(resp).await;
        assert_eq!(j["code"], "conflict");
    }

    // ------------- auth flow tests -------------

    async fn initialized_state_with_admin(password: &str) -> (AppState, tempfile::TempDir, String) {
        let (state, dir) = fresh_state().await;
        // `fresh_state` already seeded the canonical `admin` row.
        // Set its password directly so tests log in as the same
        // principal production uses.
        let admin =
            rustbase_db::admins::find_master_admin_by_username(state.system.pool(), "admin")
                .await
                .unwrap()
                .expect("seed admin missing");
        let hash = rustbase_auth::hash_password(password).unwrap();
        rustbase_db::admins::set_master_admin_password(state.system.pool(), &admin.id, &hash)
            .await
            .unwrap();
        state.mark_initialized();
        (state, dir, admin.id)
    }

    async fn post_json(
        app: Router,
        uri: &str,
        body: &serde_json::Value,
    ) -> axum::response::Response {
        let req = Request::builder()
            .uri(uri)
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(body).unwrap()))
            .unwrap();
        app.oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn login_with_valid_credentials_returns_tokens() {
        let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
        let app = build_router(state);
        let body = serde_json::json!({"username":"admin","password":"hunter22"});
        let resp = post_json(app, "/_/auth/admin/login", &body).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert!(j["access_token"].as_str().unwrap().starts_with("ey"));
        assert!(j["refresh_token"].as_str().unwrap().starts_with("rfsh_"));
        assert_eq!(j["admin"]["id"], admin_id);
        assert_eq!(j["admin"]["username"], "admin");
        assert!(j["admin"].get("password_hash").is_none());
    }

    #[tokio::test]
    async fn login_with_wrong_password_returns_401() {
        let (state, _dir, _) = initialized_state_with_admin("hunter22").await;
        let app = build_router(state);
        let body = serde_json::json!({"username":"admin","password":"wrong"});
        let resp = post_json(app, "/_/auth/admin/login", &body).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn login_with_unknown_username_returns_401() {
        let (state, _dir, _) = initialized_state_with_admin("hunter22").await;
        let app = build_router(state);
        let body = serde_json::json!({"username":"nobody","password":"hunter22"});
        let resp = post_json(app, "/_/auth/admin/login", &body).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn refresh_rotates_token() {
        let (state, _dir, _) = initialized_state_with_admin("hunter22").await;

        // log in to get a refresh token
        let app = build_router(state.clone());
        let resp = post_json(
            app,
            "/_/auth/admin/login",
            &serde_json::json!({"username":"admin","password":"hunter22"}),
        )
        .await;
        let j = json_body(resp).await;
        let first_refresh = j["refresh_token"].as_str().unwrap().to_string();

        // exchange it
        let app = build_router(state.clone());
        let resp = post_json(
            app,
            "/_/auth/refresh",
            &serde_json::json!({"refresh_token": first_refresh}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        let second_refresh = j["refresh_token"].as_str().unwrap().to_string();
        assert_ne!(first_refresh, second_refresh);
        assert!(j["access_token"].as_str().unwrap().starts_with("ey"));

        // re-using the original refresh now fails
        let app = build_router(state);
        let resp = post_json(
            app,
            "/_/auth/refresh",
            &serde_json::json!({"refresh_token": first_refresh}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn refresh_with_unknown_token_returns_401() {
        let (state, _dir, _) = initialized_state_with_admin("hunter22").await;
        let app = build_router(state);
        let resp = post_json(
            app,
            "/_/auth/refresh",
            &serde_json::json!({"refresh_token":"rfsh_does_not_exist"}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn repeated_bad_password_eventually_locks_with_429() {
        let (mut state, _dir, _) = initialized_state_with_admin("hunter22").await;
        // Tight policy for a fast test: 3 failures, 60-second window,
        // 60-second lockout.
        state.lockout_policy = crate::security::LockoutPolicy::from_secs(true, 3, 60, 60);

        let app = build_router(state.clone());
        let body = serde_json::json!({"username":"admin","password":"wrong"});
        for _ in 0..2 {
            let resp = post_json(app.clone(), "/_/auth/admin/login", &body).await;
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        }
        // Third miss trips the lock — response is 429 with Retry-After.
        let resp = post_json(app.clone(), "/_/auth/admin/login", &body).await;
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        let retry = resp
            .headers()
            .get(axum::http::header::RETRY_AFTER)
            .expect("Retry-After header missing");
        assert!(retry.to_str().unwrap().parse::<u64>().unwrap() > 0);

        // Even the correct password now bounces with 429 while locked.
        let app2 = build_router(state.clone());
        let correct = serde_json::json!({"username":"admin","password":"hunter22"});
        let resp = post_json(app2, "/_/auth/admin/login", &correct).await;
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

        // The audit log has at least one `login_locked` row for this subject.
        let rows = rustbase_db::audit::list_recent(state.system.pool(), 50)
            .await
            .unwrap();
        assert!(
            rows.iter()
                .any(|e| e.action == "login_locked" && e.actor.as_deref() == Some("master:admin"))
        );
        assert!(
            rows.iter()
                .any(|e| e.action == "login_failed" && e.actor.as_deref() == Some("master:admin"))
        );
    }

    #[tokio::test]
    async fn good_password_after_failures_clears_lockout_state() {
        let (mut state, _dir, _) = initialized_state_with_admin("hunter22").await;
        state.lockout_policy = crate::security::LockoutPolicy::from_secs(true, 3, 60, 60);
        let app = build_router(state.clone());

        // Two failures, then a success.
        for _ in 0..2 {
            let resp = post_json(
                app.clone(),
                "/_/auth/admin/login",
                &serde_json::json!({"username":"admin","password":"wrong"}),
            )
            .await;
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        }
        let resp = post_json(
            app.clone(),
            "/_/auth/admin/login",
            &serde_json::json!({"username":"admin","password":"hunter22"}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);

        // After the success, two more failures should NOT trip the
        // lockout — counters reset.
        for _ in 0..2 {
            let resp = post_json(
                app.clone(),
                "/_/auth/admin/login",
                &serde_json::json!({"username":"admin","password":"wrong"}),
            )
            .await;
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn login_success_emits_audit_row() {
        let (state, _dir, _) = initialized_state_with_admin("hunter22").await;
        let app = build_router(state.clone());
        let resp = post_json(
            app,
            "/_/auth/admin/login",
            &serde_json::json!({"username":"admin","password":"hunter22"}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let rows = rustbase_db::audit::list_recent(state.system.pool(), 5)
            .await
            .unwrap();
        assert!(
            rows.iter()
                .any(|e| e.action == "login_success" && e.actor.as_deref() == Some("master:admin"))
        );
    }

    // ------------- realm CRUD tests -------------

    fn master_token(state: &AppState, admin_id: &str) -> String {
        let claims = rustbase_auth::build_claims(
            admin_id,
            rustbase_auth::TokenRole::MasterAdmin,
            None,
            None,
            chrono::Duration::minutes(15),
        );
        rustbase_auth::encode_token(&claims, &state.master_key).unwrap()
    }

    fn req_with_auth(
        method: &str,
        uri: &str,
        token: Option<&str>,
        body: Option<&serde_json::Value>,
    ) -> Request<Body> {
        let mut b = Request::builder().uri(uri).method(method);
        if let Some(tok) = token {
            b = b.header("authorization", format!("Bearer {tok}"));
        }
        if body.is_some() {
            b = b.header("content-type", "application/json");
        }
        let body = body
            .map(|j| Body::from(serde_json::to_vec(j).unwrap()))
            .unwrap_or_else(Body::empty);
        b.body(body).unwrap()
    }

    #[tokio::test]
    async fn list_realms_without_auth_returns_401() {
        let (state, _dir, _) = initialized_state_with_admin("hunter22").await;
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth("GET", "/api/realms", None, None))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn list_realms_with_master_token_returns_master_realm() {
        let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
        let token = master_token(&state, &admin_id);
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth("GET", "/api/realms", Some(&token), None))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        let arr = j.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], "master");
        assert_eq!(arr[0]["is_master"], true);
    }

    #[tokio::test]
    async fn create_realm_initializes_realm_db_and_lists_two() {
        let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
        let token = master_token(&state, &admin_id);
        let body = serde_json::json!({"id":"acme","name":"Acme Inc."});
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms",
                Some(&token),
                Some(&body),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let j = json_body(resp).await;
        assert_eq!(j["id"], "acme");
        assert_eq!(j["is_master"], false);

        // The realm.db should now exist and respond to a query against a
        // table from the realm schema. End-users no longer live in
        // realm.db — check the `apps` table instead.
        let realm_pool = state
            .realms
            .pool_for(&rustbase_core::RealmId::from("acme"))
            .await
            .unwrap();
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM apps")
            .fetch_one(&realm_pool)
            .await
            .unwrap();
        assert_eq!(n, 0);

        // listing now returns both
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth("GET", "/api/realms", Some(&token), None))
            .await
            .unwrap();
        let j = json_body(resp).await;
        assert_eq!(j.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn create_realm_rejects_reserved_master_id() {
        let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
        let token = master_token(&state, &admin_id);
        let body = serde_json::json!({"id":"master","name":"impersonator"});
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms",
                Some(&token),
                Some(&body),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn create_realm_rejects_uppercase_in_id() {
        let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
        let token = master_token(&state, &admin_id);
        let body = serde_json::json!({"id":"Acme","name":"x"});
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms",
                Some(&token),
                Some(&body),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_realm_twice_returns_409() {
        let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
        let token = master_token(&state, &admin_id);
        let body = serde_json::json!({"id":"acme","name":"Acme"});

        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms",
                Some(&token),
                Some(&body),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms",
                Some(&token),
                Some(&body),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn get_unknown_realm_returns_404() {
        let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
        let token = master_token(&state, &admin_id);
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth("GET", "/api/realms/nope", Some(&token), None))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn rename_realm_updates_name() {
        let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
        let token = master_token(&state, &admin_id);

        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms",
            Some(&token),
            Some(&serde_json::json!({"id":"acme","name":"Acme"})),
        ))
        .await
        .unwrap();

        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "PATCH",
                "/api/realms/acme",
                Some(&token),
                Some(&serde_json::json!({"name":"Acme Renamed"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert_eq!(j["name"], "Acme Renamed");
    }

    #[tokio::test]
    async fn delete_master_realm_is_forbidden() {
        let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
        let token = master_token(&state, &admin_id);
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "DELETE",
                "/api/realms/master",
                Some(&token),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn delete_realm_removes_row_and_folder() {
        let (state, dir, admin_id) = initialized_state_with_admin("hunter22").await;
        let token = master_token(&state, &admin_id);

        // create
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms",
            Some(&token),
            Some(&serde_json::json!({"id":"acme","name":"Acme"})),
        ))
        .await
        .unwrap();
        let realm_folder = dir.path().join("realms/acme");
        assert!(realm_folder.exists());

        // delete
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "DELETE",
                "/api/realms/acme",
                Some(&token),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // row gone and folder removed
        let still = rustbase_db::realms::find_realm(state.system.pool(), "acme")
            .await
            .unwrap();
        assert!(still.is_none());
        assert!(!realm_folder.exists());
    }

    #[tokio::test]
    async fn token_with_user_role_is_forbidden_on_master_endpoint() {
        let (state, _dir, _admin_id) = initialized_state_with_admin("hunter22").await;
        // Issue a token with the wrong role; signed with the correct key.
        let claims = rustbase_auth::build_claims(
            "u1",
            rustbase_auth::TokenRole::User,
            Some("acme".into()),
            None,
            chrono::Duration::minutes(15),
        );
        let token = rustbase_auth::encode_token(&claims, &state.master_key).unwrap();
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth("GET", "/api/realms", Some(&token), None))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // ------------- realm-admin auth + app CRUD tests -------------

    /// Bootstrap state through to "realm 'acme' exists, has one realm
    /// admin (ops@acme/secretpw)". Returns the realm-admin's id.
    async fn state_with_realm_and_admin() -> (AppState, tempfile::TempDir, String, String) {
        let (state, dir, master_id) = initialized_state_with_admin("hunter22").await;
        let master_tok = master_token(&state, &master_id);
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms",
                Some(&master_tok),
                Some(&serde_json::json!({"id":"acme","name":"Acme"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/admins",
                Some(&master_tok),
                Some(&serde_json::json!({
                    "email":"ops@acme.com","password":"secretpw","name":"Ops"
                })),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let admin = json_body(resp).await;
        let admin_id = admin["id"].as_str().unwrap().to_string();

        // Provision the canonical `mobile` app so every downstream test
        // that hits `/apps/mobile/...` has a target. This is the home
        // for end-user / OAuth state after the users-per-app refactor.
        let _ = ensure_mobile_app(&state, &admin_id).await;

        (state, dir, master_id, admin_id)
    }

    fn realm_token(state: &AppState, realm: &str, admin_id: &str) -> String {
        let claims = rustbase_auth::build_claims(
            admin_id,
            rustbase_auth::TokenRole::RealmAdmin,
            Some(realm.into()),
            None,
            chrono::Duration::minutes(15),
        );
        rustbase_auth::encode_token(&claims, &state.master_key).unwrap()
    }

    #[tokio::test]
    async fn realm_admin_creation_requires_master() {
        let (state, _dir, _, realm_admin_id) = state_with_realm_and_admin().await;
        let realm_tok = realm_token(&state, "acme", &realm_admin_id);
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/admins",
                Some(&realm_tok),
                Some(&serde_json::json!({"email":"x@y.z","password":"longenough"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn realm_admin_login_returns_realm_scoped_token() {
        let (state, _dir, _, _) = state_with_realm_and_admin().await;
        let app = build_router(state);
        let resp = post_json(
            app,
            "/api/realms/acme/auth/admin/login",
            &serde_json::json!({"email":"ops@acme.com","password":"secretpw"}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert!(j["access_token"].as_str().unwrap().starts_with("ey"));
        assert!(j["refresh_token"].as_str().unwrap().starts_with("rfsh_"));
        assert_eq!(j["admin"]["email"], "ops@acme.com");
    }

    #[tokio::test]
    async fn realm_admin_login_wrong_password_is_401() {
        let (state, _dir, _, _) = state_with_realm_and_admin().await;
        let app = build_router(state);
        let resp = post_json(
            app,
            "/api/realms/acme/auth/admin/login",
            &serde_json::json!({"email":"ops@acme.com","password":"wrong"}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn realm_admin_refresh_rotates() {
        let (state, _dir, _, _) = state_with_realm_and_admin().await;
        let app = build_router(state.clone());
        let resp = post_json(
            app,
            "/api/realms/acme/auth/admin/login",
            &serde_json::json!({"email":"ops@acme.com","password":"secretpw"}),
        )
        .await;
        let j = json_body(resp).await;
        let first = j["refresh_token"].as_str().unwrap().to_string();

        let app = build_router(state.clone());
        let resp = post_json(
            app,
            "/api/realms/acme/auth/refresh",
            &serde_json::json!({"refresh_token": first}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        let second = j["refresh_token"].as_str().unwrap().to_string();
        assert_ne!(first, second);

        let app = build_router(state);
        let resp = post_json(
            app,
            "/api/realms/acme/auth/refresh",
            &serde_json::json!({"refresh_token": first}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn realm_admin_creates_and_lists_apps_in_own_realm() {
        let (state, _dir, _, realm_admin_id) = state_with_realm_and_admin().await;
        let realm_tok = realm_token(&state, "acme", &realm_admin_id);

        // `state_with_realm_and_admin` already provisioned a `mobile`
        // app; use a different id for the create-step here.
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps",
                Some(&realm_tok),
                Some(&serde_json::json!({"id":"crm","name":"CRM"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let j = json_body(resp).await;
        assert_eq!(j["id"], "crm");

        // data.db is initialized — listing collections (the meta table)
        // should be empty without erroring.
        let app_pool = state
            .apps
            .pool_for(
                &rustbase_core::RealmId::from("acme"),
                &rustbase_core::AppId::from("crm"),
            )
            .await
            .unwrap();
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _collections")
            .fetch_one(&app_pool)
            .await
            .unwrap();
        assert_eq!(n, 0);

        // list returns both apps (the bootstrap `mobile` + this `crm`).
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps",
                Some(&realm_tok),
                None,
            ))
            .await
            .unwrap();
        let j = json_body(resp).await;
        assert_eq!(j.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn realm_admin_cannot_act_on_other_realm() {
        // Create realm 'acme' with admin 'ops@acme', and a second realm
        // 'widgetco'. The acme admin must not be able to list widgetco's
        // apps.
        let (state, _dir, master_id, realm_admin_id) = state_with_realm_and_admin().await;
        let master_tok = master_token(&state, &master_id);

        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms",
            Some(&master_tok),
            Some(&serde_json::json!({"id":"widgetco","name":"WidgetCo"})),
        ))
        .await
        .unwrap();

        let acme_tok = realm_token(&state, "acme", &realm_admin_id);
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/widgetco/apps",
                Some(&acme_tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn create_app_with_uppercase_id_is_400() {
        let (state, _dir, master_id, _) = state_with_realm_and_admin().await;
        let master_tok = master_token(&state, &master_id);
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps",
                Some(&master_tok),
                Some(&serde_json::json!({"id":"Mobile","name":"x"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_duplicate_app_returns_409() {
        let (state, _dir, master_id, _) = state_with_realm_and_admin().await;
        let master_tok = master_token(&state, &master_id);
        let body = serde_json::json!({"id":"mobile","name":"M"});

        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps",
            Some(&master_tok),
            Some(&body),
        ))
        .await
        .unwrap();

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps",
                Some(&master_tok),
                Some(&body),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn create_app_in_unknown_realm_is_404() {
        let (state, _dir, master_id, _) = state_with_realm_and_admin().await;
        let master_tok = master_token(&state, &master_id);
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/no-such-realm/apps",
                Some(&master_tok),
                Some(&serde_json::json!({"id":"mobile","name":"Mobile"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_app_removes_row_and_folder() {
        let (state, dir, master_id, _) = state_with_realm_and_admin().await;
        let master_tok = master_token(&state, &master_id);

        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps",
            Some(&master_tok),
            Some(&serde_json::json!({"id":"mobile","name":"M"})),
        ))
        .await
        .unwrap();
        let app_folder = dir.path().join("realms/acme/apps/mobile");
        assert!(app_folder.exists());

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "DELETE",
                "/api/realms/acme/apps/mobile",
                Some(&master_tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        assert!(!app_folder.exists());
    }

    #[tokio::test]
    async fn rename_app_updates_name() {
        let (state, _dir, _, realm_admin_id) = state_with_realm_and_admin().await;
        let tok = realm_token(&state, "acme", &realm_admin_id);

        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps",
            Some(&tok),
            Some(&serde_json::json!({"id":"mobile","name":"Original"})),
        ))
        .await
        .unwrap();

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "PATCH",
                "/api/realms/acme/apps/mobile",
                Some(&tok),
                Some(&serde_json::json!({"name":"Renamed"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert_eq!(j["name"], "Renamed");
    }

    // ------------- collections + records end-to-end -------------

    /// Idempotent: PUTs the `mobile` app inside `acme` using a freshly
    /// minted realm-admin token if it isn't there yet. Returns the
    /// realm-admin token so callers can keep using it.
    async fn ensure_mobile_app(state: &AppState, realm_admin_id: &str) -> String {
        let tok = realm_token(state, "acme", realm_admin_id);
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps",
                Some(&tok),
                Some(&serde_json::json!({"id":"mobile","name":"M"})),
            ))
            .await
            .unwrap();
        // 201 on first call, 409 on later calls — both are fine.
        assert!(
            resp.status() == StatusCode::CREATED || resp.status() == StatusCode::CONFLICT,
            "unexpected status creating mobile app: {}",
            resp.status()
        );
        tok
    }

    /// Bootstrap to: realm 'acme', realm-admin token, app 'mobile',
    /// collection 'notes' with fields {title:text, pinned:bool,
    /// metadata:json}. Returns (state, dir, realm_token).
    async fn state_with_app_and_collection() -> (AppState, tempfile::TempDir, String) {
        let (state, dir, _, realm_admin_id) = state_with_realm_and_admin().await;
        let tok = ensure_mobile_app(&state, &realm_admin_id).await;

        // create collection
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/collections",
                Some(&tok),
                Some(&serde_json::json!({
                    "schema": {
                        "id": "notes",
                        "kind": "base",
                        "fields": [
                            {"name": "title", "kind": "text", "required": true},
                            {"name": "pinned", "kind": "bool"},
                            {"name": "metadata", "kind": "json"}
                        ]
                    }
                })),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        (state, dir, tok)
    }

    #[tokio::test]
    async fn collection_reserved_id_is_rejected() {
        let (state, _dir, tok) = state_with_app_and_collection().await;
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/collections",
                Some(&tok),
                Some(&serde_json::json!({
                    "schema": {"id": "policies", "kind": "base", "fields": []}
                })),
            ))
            .await
            .unwrap();
        // collections::create_collection returns InvalidIdentifier →
        // CoreError::Validation → 400
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn record_full_lifecycle() {
        let (state, _dir, tok) = state_with_app_and_collection().await;

        // CREATE
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/collections/notes/records",
                Some(&tok),
                Some(&serde_json::json!({
                    "title": "Hello",
                    "pinned": true,
                    "metadata": {"tags": ["greeting"], "version": 1}
                })),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let created = json_body(resp).await;
        let id = created["id"].as_str().unwrap().to_string();
        assert_eq!(created["fields"]["title"], "Hello");
        assert_eq!(created["fields"]["pinned"], true);
        assert_eq!(created["fields"]["metadata"]["version"], 1);

        // GET
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                &format!("/api/realms/acme/apps/mobile/collections/notes/records/{id}"),
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let got = json_body(resp).await;
        assert_eq!(got["fields"]["title"], "Hello");

        // PATCH — only "title" supplied; "pinned" stays true
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "PATCH",
                &format!("/api/realms/acme/apps/mobile/collections/notes/records/{id}"),
                Some(&tok),
                Some(&serde_json::json!({"title": "Goodbye"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let patched = json_body(resp).await;
        assert_eq!(patched["fields"]["title"], "Goodbye");
        assert_eq!(patched["fields"]["pinned"], true);

        // LIST (pagination response shape)
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/collections/notes/records?per_page=10",
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        let listed = json_body(resp).await;
        assert_eq!(listed["total_items"], 1);
        assert_eq!(listed["page"], 1);
        assert_eq!(listed["per_page"], 10);
        assert_eq!(listed["items"].as_array().unwrap().len(), 1);

        // DELETE
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "DELETE",
                &format!("/api/realms/acme/apps/mobile/collections/notes/records/{id}"),
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // GET-after-delete → 404
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                &format!("/api/realms/acme/apps/mobile/collections/notes/records/{id}"),
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_collection_drops_table() {
        let (state, _dir, tok) = state_with_app_and_collection().await;
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "DELETE",
                "/api/realms/acme/apps/mobile/collections/notes",
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // creating a record now fails because the collection (and table) are gone
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/collections/notes/records",
                Some(&tok),
                Some(&serde_json::json!({"title": "x"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_records_with_filter_returns_only_matching() {
        let (state, _dir, tok) = state_with_app_and_collection().await;

        // Add 3 notes — only 2 have pinned=true.
        for (title, pinned) in [("a", true), ("b", false), ("c", true)] {
            let app = build_router(state.clone());
            app.oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/collections/notes/records",
                Some(&tok),
                Some(&serde_json::json!({"title": title, "pinned": pinned})),
            ))
            .await
            .unwrap();
        }

        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/collections/notes/records?filter=pinned%20%3D%20true",
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert_eq!(j["total_items"], 2);
        assert_eq!(j["items"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn list_records_with_unknown_filter_column_is_400() {
        let (state, _dir, tok) = state_with_app_and_collection().await;
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/collections/notes/records?filter=nope%20%3D%20%22x%22",
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let j = json_body(resp).await;
        let msg = j["message"].as_str().unwrap();
        assert!(msg.contains("nope"), "got message: {msg}");
    }

    // ------------- file storage -------------

    #[tokio::test]
    async fn file_upload_then_download_round_trip() {
        let (state, _dir, _, realm_admin_id) = state_with_realm_and_admin().await;
        let tok = realm_token(&state, "acme", &realm_admin_id);

        // create app
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps",
            Some(&tok),
            Some(&serde_json::json!({"id":"mobile","name":"M"})),
        ))
        .await
        .unwrap();

        // upload
        let app = build_router(state.clone());
        let req = Request::builder()
            .uri("/api/realms/acme/apps/mobile/files")
            .method("POST")
            .header("authorization", format!("Bearer {tok}"))
            .header("content-type", "image/png")
            .header("x-filename", "kitten.png")
            .body(Body::from(b"\x89PNG\x0d\x0a\x1a\x0afakebytes".to_vec()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let j = json_body(resp).await;
        let id = j["id"].as_str().unwrap().to_string();
        assert_eq!(j["filename"], "kitten.png");
        assert_eq!(j["mime"], "image/png");
        assert_eq!(j["size"], 17);

        // download
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                &format!("/api/realms/acme/apps/mobile/files/{id}"),
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get("content-type")
            .map(|v| v.to_str().unwrap().to_string());
        assert_eq!(ct.as_deref(), Some("image/png"));
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(&bytes[..], b"\x89PNG\x0d\x0a\x1a\x0afakebytes");

        // list
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/files",
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        let j = json_body(resp).await;
        assert_eq!(j.as_array().unwrap().len(), 1);

        // delete
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "DELETE",
                &format!("/api/realms/acme/apps/mobile/files/{id}"),
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn file_upload_without_filename_header_is_400() {
        let (state, _dir, _, realm_admin_id) = state_with_realm_and_admin().await;
        let tok = realm_token(&state, "acme", &realm_admin_id);
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps",
            Some(&tok),
            Some(&serde_json::json!({"id":"mobile","name":"M"})),
        ))
        .await
        .unwrap();

        let app = build_router(state);
        let req = Request::builder()
            .uri("/api/realms/acme/apps/mobile/files")
            .method("POST")
            .header("authorization", format!("Bearer {tok}"))
            .body(Body::from(b"x".to_vec()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // ------------- end-user auth + access rules -------------

    /// Bootstrap to: realm 'acme' + open app 'mobile' + collection
    /// 'notes' + one registered user 'u@acme'. Returns (state, dir,
    /// master_token, user_access_token).
    async fn state_with_collection_and_user() -> (AppState, tempfile::TempDir, String, String) {
        let (state, dir, tok) = state_with_app_and_collection().await;
        let row: (String,) = sqlx::query_as("SELECT id FROM master_admins LIMIT 1")
            .fetch_one(state.system.pool())
            .await
            .unwrap();
        let master_tok = master_token(&state, &row.0);

        // register a user
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/register",
                None,
                Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        // login -> token
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/login",
                None,
                Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        let user_tok = j["access_token"].as_str().unwrap().to_string();

        // keep the seed realm-admin token for the (admin) bootstrap caller
        let _ = tok;
        (state, dir, master_tok, user_tok)
    }

    #[tokio::test]
    async fn user_register_duplicate_email_returns_409() {
        let (state, _dir, _, _) = state_with_collection_and_user().await;
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/register",
                None,
                Some(&serde_json::json!({"email":"u@acme.com","password":"otherpass"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn user_login_wrong_password_returns_401() {
        let (state, _dir, _, _) = state_with_collection_and_user().await;
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/login",
                None,
                Some(&serde_json::json!({"email":"u@acme.com","password":"wrong"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ------------- email verification flow -------------

    /// Pull the token straight from the per-app DB. Avoids needing to
    /// downcast `Arc<dyn Mailer>` from AppState to read the body of
    /// the captured LogMailer message — the row that backs the email
    /// is the same string.
    async fn read_pending_verification_token(
        state: &AppState,
        realm: &str,
        app: &str,
        user_email: &str,
    ) -> String {
        let pool = state
            .apps
            .pool_for(
                &rustbase_core::RealmId::from(realm.to_string()),
                &rustbase_core::AppId::from(app.to_string()),
            )
            .await
            .unwrap();
        let row: (String,) = sqlx::query_as(
            "SELECT ev.token FROM _email_verifications ev \
             JOIN users u ON u.id = ev.user_id \
             WHERE u.email = ? AND ev.consumed_at IS NULL \
             ORDER BY ev.issued_at DESC LIMIT 1",
        )
        .bind(user_email)
        .fetch_one(&pool)
        .await
        .unwrap();
        row.0
    }

    #[tokio::test]
    async fn verify_email_request_then_confirm_marks_user_verified() {
        let (state, _dir, _, user_tok) = state_with_collection_and_user().await;

        // Step 1: user asks for a verification email.
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/verify-email/request",
                Some(&user_tok),
                Some(&serde_json::json!({})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        // Step 2: pull the token, confirm it.
        let token = read_pending_verification_token(&state, "acme", "mobile", "u@acme.com").await;
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/verify-email/confirm",
                None,
                Some(&serde_json::json!({"token": token})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert_eq!(j["verified"], true);

        // Step 3: user is now flagged verified — read from the app DB.
        let pool = state
            .apps
            .pool_for(
                &rustbase_core::RealmId::from("acme".to_string()),
                &rustbase_core::AppId::from("mobile".to_string()),
            )
            .await
            .unwrap();
        let user = rustbase_db::users::find_user_by_email(&pool, "u@acme.com")
            .await
            .unwrap()
            .unwrap();
        assert!(user.verified);
    }

    #[tokio::test]
    async fn verify_email_confirm_with_unknown_token_returns_404() {
        let (state, _dir, _, _) = state_with_collection_and_user().await;
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/verify-email/confirm",
                None,
                Some(&serde_json::json!({"token": "deadbeef".repeat(8)})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn verify_email_confirm_twice_second_call_409() {
        let (state, _dir, _, user_tok) = state_with_collection_and_user().await;

        // Issue and consume.
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps/mobile/auth/verify-email/request",
            Some(&user_tok),
            Some(&serde_json::json!({})),
        ))
        .await
        .unwrap();
        let token = read_pending_verification_token(&state, "acme", "mobile", "u@acme.com").await;
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps/mobile/auth/verify-email/confirm",
            None,
            Some(&serde_json::json!({"token": &token})),
        ))
        .await
        .unwrap();

        // Replay must fail with 409 Conflict.
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/verify-email/confirm",
                None,
                Some(&serde_json::json!({"token": &token})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    // ------------- password reset flow -------------

    async fn read_pending_reset_token(
        state: &AppState,
        realm: &str,
        app: &str,
        user_email: &str,
    ) -> String {
        let pool = state
            .apps
            .pool_for(
                &rustbase_core::RealmId::from(realm.to_string()),
                &rustbase_core::AppId::from(app.to_string()),
            )
            .await
            .unwrap();
        let row: (String,) = sqlx::query_as(
            "SELECT pr.token FROM _password_resets pr \
             JOIN users u ON u.id = pr.user_id \
             WHERE u.email = ? AND pr.consumed_at IS NULL \
             ORDER BY pr.issued_at DESC LIMIT 1",
        )
        .bind(user_email)
        .fetch_one(&pool)
        .await
        .unwrap();
        row.0
    }

    #[tokio::test]
    async fn password_reset_request_then_confirm_changes_password() {
        let (state, _dir, _, _) = state_with_collection_and_user().await;
        // Original password from state_with_collection_and_user is "userpass1".

        // 1. Request reset.
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/password-reset/request",
                None,
                Some(&serde_json::json!({"email":"u@acme.com"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        // 2. Pull token + confirm with new password.
        let token = read_pending_reset_token(&state, "acme", "mobile", "u@acme.com").await;
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/password-reset/confirm",
                None,
                Some(&serde_json::json!({"token": token, "new_password": "totallyNew!42"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert_eq!(j["reset"], true);

        // 3. Login with new password succeeds.
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/login",
                None,
                Some(&serde_json::json!({"email":"u@acme.com","password":"totallyNew!42"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // 4. Old password no longer works.
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/login",
                None,
                Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn password_reset_request_for_unknown_email_still_returns_202() {
        // Enumeration-resistance: same response regardless of whether
        // the address belongs to a user. The DB should be untouched.
        let (state, _dir, _, _) = state_with_collection_and_user().await;
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/password-reset/request",
                None,
                Some(&serde_json::json!({"email":"ghost@nowhere.com"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        let pool = state
            .apps
            .pool_for(
                &rustbase_core::RealmId::from("acme".to_string()),
                &rustbase_core::AppId::from("mobile".to_string()),
            )
            .await
            .unwrap();
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _password_resets")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 0);
    }

    #[tokio::test]
    async fn password_reset_confirm_invalidates_siblings() {
        // Issue two tokens for the same user; consuming one must
        // make the other return 409 instead of 200.
        let (state, _dir, _, _) = state_with_collection_and_user().await;
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps/mobile/auth/password-reset/request",
            None,
            Some(&serde_json::json!({"email":"u@acme.com"})),
        ))
        .await
        .unwrap();
        let first = read_pending_reset_token(&state, "acme", "mobile", "u@acme.com").await;

        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps/mobile/auth/password-reset/request",
            None,
            Some(&serde_json::json!({"email":"u@acme.com"})),
        ))
        .await
        .unwrap();
        let second = read_pending_reset_token(&state, "acme", "mobile", "u@acme.com").await;
        assert_ne!(first, second);

        // Consume the second; the first must then be dead.
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/password-reset/confirm",
                None,
                Some(&serde_json::json!({"token": &second, "new_password": "brandnew!9"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/password-reset/confirm",
                None,
                Some(&serde_json::json!({"token": &first, "new_password": "another!7"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn password_reset_confirm_rejects_weak_password() {
        let (state, _dir, _, _) = state_with_collection_and_user().await;
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps/mobile/auth/password-reset/request",
            None,
            Some(&serde_json::json!({"email":"u@acme.com"})),
        ))
        .await
        .unwrap();
        let token = read_pending_reset_token(&state, "acme", "mobile", "u@acme.com").await;
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/password-reset/confirm",
                None,
                Some(&serde_json::json!({"token": token, "new_password": "short"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // ------------- email OTP flow -------------

    async fn read_pending_otp_code(
        state: &AppState,
        realm: &str,
        app: &str,
        email: &str,
    ) -> String {
        let pool = state
            .apps
            .pool_for(
                &rustbase_core::RealmId::from(realm.to_string()),
                &rustbase_core::AppId::from(app.to_string()),
            )
            .await
            .unwrap();
        let row: (String,) = sqlx::query_as(
            "SELECT code FROM _email_otps \
             WHERE email = ? AND consumed_at IS NULL \
             ORDER BY issued_at DESC LIMIT 1",
        )
        .bind(email)
        .fetch_one(&pool)
        .await
        .unwrap();
        row.0
    }

    /// Bootstrap up to "realm 'acme' exists" without registering any
    /// user — OTP can sign people up so we don't want the test fixture
    /// pre-creating one.
    async fn state_with_empty_realm() -> (AppState, tempfile::TempDir) {
        let (state, dir, _master_id, _admin_id) = state_with_realm_and_admin().await;
        (state, dir)
    }

    #[tokio::test]
    async fn otp_request_then_login_signs_up_brand_new_user() {
        let (state, _dir) = state_with_empty_realm().await;

        // 1. New email asks for a code.
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/otp/request",
                None,
                Some(&serde_json::json!({"email":"new@acme.com"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        // 2. Pull the code from the DB, redeem it.
        let code = read_pending_otp_code(&state, "acme", "mobile", "new@acme.com").await;
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/otp/login",
                None,
                Some(&serde_json::json!({"email":"new@acme.com","code":code})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert!(j["access_token"].as_str().unwrap().starts_with("ey"));
        assert_eq!(j["user"]["email"], "new@acme.com");
        assert_eq!(j["user"]["verified"], true);

        // 3. The auto-created user exists with no password and is verified.
        let pool = state
            .apps
            .pool_for(
                &rustbase_core::RealmId::from("acme".to_string()),
                &rustbase_core::AppId::from("mobile".to_string()),
            )
            .await
            .unwrap();
        let user = rustbase_db::users::find_user_by_email(&pool, "new@acme.com")
            .await
            .unwrap()
            .unwrap();
        assert!(user.verified);
        assert!(user.password_hash.is_none(), "passwordless signup");
    }

    #[tokio::test]
    async fn otp_login_with_wrong_code_returns_400_with_attempts_left() {
        let (state, _dir) = state_with_empty_realm().await;
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps/mobile/auth/otp/request",
            None,
            Some(&serde_json::json!({"email":"a@acme.com"})),
        ))
        .await
        .unwrap();

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/otp/login",
                None,
                Some(&serde_json::json!({"email":"a@acme.com","code":"000000"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let j = json_body(resp).await;
        let msg = j["message"].as_str().unwrap();
        assert!(msg.contains("wrong code"), "got: {msg}");
        assert!(msg.contains("attempts left"), "got: {msg}");
    }

    #[tokio::test]
    async fn otp_request_invalidates_prior_pending_code() {
        let (state, _dir) = state_with_empty_realm().await;
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps/mobile/auth/otp/request",
            None,
            Some(&serde_json::json!({"email":"a@acme.com"})),
        ))
        .await
        .unwrap();
        let first = read_pending_otp_code(&state, "acme", "mobile", "a@acme.com").await;

        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps/mobile/auth/otp/request",
            None,
            Some(&serde_json::json!({"email":"a@acme.com"})),
        ))
        .await
        .unwrap();
        let second = read_pending_otp_code(&state, "acme", "mobile", "a@acme.com").await;

        assert_ne!(first, second, "second request must mint a fresh code");

        // Old code is dead.
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/otp/login",
                None,
                Some(&serde_json::json!({"email":"a@acme.com","code":&first})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        // New code logs in.
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/otp/login",
                None,
                Some(&serde_json::json!({"email":"a@acme.com","code":&second})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn otp_login_unknown_email_returns_409_no_enumeration() {
        // No prior /request → no pending row, but we still don't leak
        // "this email isn't registered" — Conflict + same message
        // shape as "code expired".
        let (state, _dir) = state_with_empty_realm().await;
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/otp/login",
                None,
                Some(&serde_json::json!({"email":"ghost@acme.com","code":"123456"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn otp_login_signs_in_existing_password_user_too() {
        // A user who registered with a password can ALSO use OTP — the
        // OTP path doesn't require password_hash to be NULL.
        let (state, _dir, _, _) = state_with_collection_and_user().await;
        // state_with_collection_and_user registered u@acme.com with a
        // password. Request an OTP for the SAME email:
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps/mobile/auth/otp/request",
            None,
            Some(&serde_json::json!({"email":"u@acme.com"})),
        ))
        .await
        .unwrap();
        let code = read_pending_otp_code(&state, "acme", "mobile", "u@acme.com").await;
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/otp/login",
                None,
                Some(&serde_json::json!({"email":"u@acme.com","code":code})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Password row should be untouched after OTP login.
        let pool = state
            .apps
            .pool_for(
                &rustbase_core::RealmId::from("acme".to_string()),
                &rustbase_core::AppId::from("mobile".to_string()),
            )
            .await
            .unwrap();
        let user = rustbase_db::users::find_user_by_email(&pool, "u@acme.com")
            .await
            .unwrap()
            .unwrap();
        assert!(user.password_hash.is_some(), "OTP must not clear password");
    }

    // ------------- OAuth2 sign-in -------------

    /// Spin up a localhost axum server that pretends to be an
    /// OAuth2 provider's `/token` and `/userinfo` endpoints. Returns
    /// the bound `http://127.0.0.1:PORT` URL prefix and a shutdown
    /// handle. The body the stub returns is parameterised so a single
    /// test can drive multiple provider responses.
    async fn fake_oauth_provider(
        userinfo_body: serde_json::Value,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use axum::{Json, Router, routing::post};
        // Token endpoint — accepts any form body, returns a fixed token.
        async fn token() -> Json<serde_json::Value> {
            Json(serde_json::json!({
                "access_token": "fake-access-token",
                "token_type": "Bearer",
                "expires_in": 3600,
            }))
        }
        let body_for_handler = userinfo_body;
        let userinfo_handler = move || {
            let body = body_for_handler.clone();
            async move { Json(body) }
        };
        let app: Router = Router::new()
            .route("/token", post(token))
            .route("/userinfo", axum::routing::get(userinfo_handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        // Give axum a tick to actually start accepting.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        (format!("http://{addr}"), handle)
    }

    /// Seed a provider config in the per-app DB pointing at the stub.
    /// The client_secret is encrypted under the AppState's KEK so the
    /// callback path can decrypt it on use, matching production wiring.
    async fn seed_provider(
        state: &AppState,
        realm: &str,
        app: &str,
        provider: &str,
        base_url: &str,
    ) {
        let pool = state
            .apps
            .pool_for(
                &rustbase_core::RealmId::from(realm.to_string()),
                &rustbase_core::AppId::from(app.to_string()),
            )
            .await
            .unwrap();
        let secret_enc = rustbase_auth::encrypt(b"test-secret", state.oauth_kek.as_ref()).unwrap();
        rustbase_db::oauth_providers::upsert_provider(
            &pool,
            &rustbase_db::oauth_providers::OAuthProvider {
                provider: provider.into(),
                client_id: "test-client".into(),
                secret_enc,
                config: rustbase_db::oauth_providers::OAuthProviderConfig {
                    auth_url: format!("{base_url}/authorize"),
                    token_url: format!("{base_url}/token"),
                    userinfo_url: format!("{base_url}/userinfo"),
                    scopes: vec!["openid".into(), "email".into()],
                    userinfo_id_field: "/sub".into(),
                    userinfo_email_field: "/email".into(),
                },
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn oauth_authorize_returns_url_with_state_and_scopes() {
        let (base_url, _h) = fake_oauth_provider(serde_json::json!({})).await;
        let (state, _dir, _, _) = state_with_realm_and_admin().await;
        seed_provider(&state, "acme", "mobile", "google", &base_url).await;

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/auth/oauth/google/authorize?redirect_uri=https%3A%2F%2Fapp%2Fcb",
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        let url = j["authorize_url"].as_str().unwrap();
        // The stub's authorize URL gets baked in; we don't follow it,
        // but every required query param has to be present.
        assert!(url.contains("client_id=test-client"), "got: {url}");
        assert!(url.contains("response_type=code"), "got: {url}");
        assert!(url.contains("state="), "got: {url}");
        assert!(url.contains("scope=openid+email"), "got: {url}");
        assert!(j["state"].as_str().unwrap().len() == 64);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn oauth_authorize_unknown_provider_returns_404() {
        let (state, _dir, _, _) = state_with_realm_and_admin().await;
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/auth/oauth/ghost/authorize?redirect_uri=https%3A%2F%2Fapp%2Fcb",
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn oauth_callback_round_trips_signup_via_stubbed_provider() {
        let (base_url, _h) = fake_oauth_provider(serde_json::json!({
            "sub": "google-sub-42",
            "email": "ada@google.test",
        }))
        .await;
        let (state, _dir, _, _) = state_with_realm_and_admin().await;
        seed_provider(&state, "acme", "mobile", "google", &base_url).await;

        // 1. /authorize → get a real state nonce.
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/auth/oauth/google/authorize?redirect_uri=https%3A%2F%2Fapp%2Fcb",
                None,
                None,
            ))
            .await
            .unwrap();
        let nonce = json_body(resp).await["state"].as_str().unwrap().to_string();

        // 2. /callback — the stub returns access_token + userinfo.
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/oauth/google/callback",
                None,
                Some(&serde_json::json!({"code":"unused", "state": nonce})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert!(j["access_token"].as_str().unwrap().starts_with("ey"));
        assert_eq!(j["user"]["email"], "ada@google.test");
        assert_eq!(j["user"]["verified"], true);

        // 3. The link row exists, and the user was created passwordless.
        let pool = state
            .apps
            .pool_for(
                &rustbase_core::RealmId::from("acme".to_string()),
                &rustbase_core::AppId::from("mobile".to_string()),
            )
            .await
            .unwrap();
        let link =
            rustbase_db::oauth_links::find_by_provider_user(&pool, "google", "google-sub-42")
                .await
                .unwrap()
                .unwrap();
        let user = rustbase_db::users::find_user_by_id(&pool, &link.user_id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            user.password_hash.is_none(),
            "new OAuth signup is passwordless"
        );
        assert!(user.verified);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn oauth_callback_links_to_existing_password_user_by_email() {
        // A user already exists with the same email (registered with
        // a password). OAuth callback should link the provider account
        // to the existing user, NOT create a duplicate.
        let (base_url, _h) = fake_oauth_provider(serde_json::json!({
            "sub": "google-sub-99",
            "email": "u@acme.com",  // matches state_with_collection_and_user
        }))
        .await;
        let (state, _dir, _, _) = state_with_collection_and_user().await;
        seed_provider(&state, "acme", "mobile", "google", &base_url).await;

        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/auth/oauth/google/authorize?redirect_uri=https%3A%2F%2Fapp%2Fcb",
                None,
                None,
            ))
            .await
            .unwrap();
        let nonce = json_body(resp).await["state"].as_str().unwrap().to_string();

        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/oauth/google/callback",
                None,
                Some(&serde_json::json!({"code":"unused", "state": nonce})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let pool = state
            .apps
            .pool_for(
                &rustbase_core::RealmId::from("acme".to_string()),
                &rustbase_core::AppId::from("mobile".to_string()),
            )
            .await
            .unwrap();
        // Still exactly one user with that email.
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users WHERE email = ?")
            .bind("u@acme.com")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 1);
        // Password preserved.
        let user = rustbase_db::users::find_user_by_email(&pool, "u@acme.com")
            .await
            .unwrap()
            .unwrap();
        assert!(user.password_hash.is_some());
        // Link row exists pointing at the same user_id.
        let link =
            rustbase_db::oauth_links::find_by_provider_user(&pool, "google", "google-sub-99")
                .await
                .unwrap()
                .unwrap();
        assert_eq!(link.user_id, user.id);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn oauth_callback_replayed_state_returns_409() {
        let (base_url, _h) = fake_oauth_provider(serde_json::json!({
            "sub": "google-sub-1", "email": "a@x"
        }))
        .await;
        let (state, _dir, _, _) = state_with_realm_and_admin().await;
        seed_provider(&state, "acme", "mobile", "google", &base_url).await;

        let app = build_router(state.clone());
        let nonce = json_body(
            app.oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/auth/oauth/google/authorize?redirect_uri=https%3A%2F%2Fapp%2Fcb",
                None,
                None,
            ))
            .await
            .unwrap(),
        )
        .await["state"]
            .as_str()
            .unwrap()
            .to_string();

        // First consume succeeds.
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/oauth/google/callback",
                None,
                Some(&serde_json::json!({"code":"unused", "state": &nonce})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Replay must be rejected.
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/oauth/google/callback",
                None,
                Some(&serde_json::json!({"code":"unused", "state": &nonce})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn oauth_callback_unknown_state_returns_401() {
        let (base_url, _h) = fake_oauth_provider(serde_json::json!({})).await;
        let (state, _dir, _, _) = state_with_realm_and_admin().await;
        seed_provider(&state, "acme", "mobile", "google", &base_url).await;

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/oauth/google/callback",
                None,
                Some(&serde_json::json!({"code":"unused", "state":"forged-or-stale"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn verify_email_request_rejects_admin_tokens() {
        // A realm-admin token isn't tied to an end user, so /request
        // must reject it rather than try to mail a non-existent user.
        let (state, _dir, _, realm_admin_id) = state_with_realm_and_admin().await;
        let admin_tok = realm_token(&state, "acme", &realm_admin_id);
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/verify-email/request",
                Some(&admin_tok),
                Some(&serde_json::json!({})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn user_blocked_from_records_without_a_rule() {
        let (state, _dir, _, user_tok) = state_with_collection_and_user().await;
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/collections/notes/records",
                Some(&user_tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn open_list_rule_lets_user_read() {
        let (state, _dir, master_tok, user_tok) = state_with_collection_and_user().await;
        // master opens 'list' rule
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "PUT",
                "/api/realms/acme/apps/mobile/collections/notes/access_rules/list",
                Some(&master_tok),
                Some(&serde_json::json!({"filter": ""})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // user can now read
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/collections/notes/records",
                Some(&user_tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn user_in_one_realm_cannot_read_another_realms_records() {
        let (state, _dir, master_tok, user_tok) = state_with_collection_and_user().await;
        // master creates widgetco with an OPEN notes collection
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms",
            Some(&master_tok),
            Some(&serde_json::json!({"id":"widgetco","name":"W"})),
        ))
        .await
        .unwrap();
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/widgetco/apps",
            Some(&master_tok),
            Some(&serde_json::json!({"id":"web","name":"W"})),
        ))
        .await
        .unwrap();
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/widgetco/apps/web/collections",
            Some(&master_tok),
            Some(&serde_json::json!({
                "schema":{"id":"items","kind":"base",
                          "fields":[{"name":"name","kind":"text","required":true}]}
            })),
        ))
        .await
        .unwrap();
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "PUT",
            "/api/realms/widgetco/apps/web/collections/items/access_rules/list",
            Some(&master_tok),
            Some(&serde_json::json!({"filter": ""})),
        ))
        .await
        .unwrap();

        // acme's user tries widgetco — must be 403 even with an open rule
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/widgetco/apps/web/collections/items/records",
                Some(&user_tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn template_rule_scopes_user_to_own_rows() {
        let (state, _dir, master_tok, user_tok) = state_with_collection_and_user().await;

        // Add an `owner` text field to 'notes' so the rule has something to bind to.
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "DELETE",
                "/api/realms/acme/apps/mobile/collections/notes",
                Some(&master_tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/collections",
                Some(&master_tok),
                Some(&serde_json::json!({
                    "schema": {
                        "id": "notes",
                        "kind": "base",
                        "fields": [
                            {"name":"title","kind":"text","required":true},
                            {"name":"owner","kind":"text","required":true}
                        ]
                    }
                })),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        // Seed: one record owned by our user, one owned by someone else.
        let user_id: String = {
            let row: (String,) = sqlx::query_as("SELECT id FROM users WHERE email = ?")
                .bind("u@acme.com")
                .fetch_one(
                    &state
                        .apps
                        .pool_for(
                            &rustbase_core::RealmId::from("acme"),
                            &rustbase_core::AppId::from("mobile"),
                        )
                        .await
                        .unwrap(),
                )
                .await
                .unwrap();
            row.0
        };
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps/mobile/collections/notes/records",
            Some(&master_tok),
            Some(&serde_json::json!({"title": "mine", "owner": user_id})),
        ))
        .await
        .unwrap();
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps/mobile/collections/notes/records",
            Some(&master_tok),
            Some(&serde_json::json!({"title": "theirs", "owner": "other-user-id"})),
        ))
        .await
        .unwrap();

        // Template rule: each user sees only their own rows.
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "PUT",
                "/api/realms/acme/apps/mobile/collections/notes/access_rules/list",
                Some(&master_tok),
                Some(&serde_json::json!({"filter": "owner = {{request.auth.id}}"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // The user should now see ONE row (their own).
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/collections/notes/records",
                Some(&user_tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert_eq!(j["total_items"], 1);
        assert_eq!(j["items"][0]["fields"]["title"], "mine");
    }

    #[tokio::test]
    async fn template_rule_scoped_get_returns_404_for_unowned() {
        let (state, _dir, master_tok, user_tok) = state_with_collection_and_user().await;

        // Replace notes with an owner field.
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "DELETE",
            "/api/realms/acme/apps/mobile/collections/notes",
            Some(&master_tok),
            None,
        ))
        .await
        .unwrap();
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps/mobile/collections",
            Some(&master_tok),
            Some(&serde_json::json!({
                "schema": {
                    "id":"notes","kind":"base","fields":[
                        {"name":"title","kind":"text","required":true},
                        {"name":"owner","kind":"text","required":true}
                    ]
                }
            })),
        ))
        .await
        .unwrap();

        // Make a record owned by someone else.
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/collections/notes/records",
                Some(&master_tok),
                Some(&serde_json::json!({"title":"x","owner":"other"})),
            ))
            .await
            .unwrap();
        let id = json_body(resp).await["id"].as_str().unwrap().to_string();

        // Open both view + list with the same per-row rule.
        for action in ["view", "list"] {
            let app = build_router(state.clone());
            app.oneshot(req_with_auth(
                "PUT",
                &format!("/api/realms/acme/apps/mobile/collections/notes/access_rules/{action}"),
                Some(&master_tok),
                Some(&serde_json::json!({"filter":"owner = {{request.auth.id}}"})),
            ))
            .await
            .unwrap();
        }

        // GET that record as the user → 404 (not their row).
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                &format!("/api/realms/acme/apps/mobile/collections/notes/records/{id}"),
                Some(&user_tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn template_with_unknown_placeholder_is_400() {
        let (state, _dir, master_tok, user_tok) = state_with_collection_and_user().await;

        // Open list with a bogus placeholder.
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "PUT",
            "/api/realms/acme/apps/mobile/collections/notes/access_rules/list",
            Some(&master_tok),
            Some(&serde_json::json!({"filter": "title = {{request.unknown}}"})),
        ))
        .await
        .unwrap();

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/collections/notes/records",
                Some(&user_tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn user_refresh_rotates_token() {
        let (state, _dir, _, _) = state_with_collection_and_user().await;
        // login to capture refresh
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/login",
                None,
                Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
            ))
            .await
            .unwrap();
        let j = json_body(resp).await;
        let first = j["refresh_token"].as_str().unwrap().to_string();

        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/refresh",
                None,
                Some(&serde_json::json!({"refresh_token": first})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        let second = j["refresh_token"].as_str().unwrap().to_string();
        assert_ne!(first, second);

        // reuse fails
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/refresh",
                None,
                Some(&serde_json::json!({"refresh_token": first})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ------------- policy engine tests -------------

    #[tokio::test]
    async fn master_can_set_policy_and_realm_clamps_below_master_bound() {
        let (state, _dir, master_id, realm_admin_id) = state_with_realm_and_admin().await;
        let m_tok = master_token(&state, &master_id);
        let r_tok = realm_token(&state, "acme", &realm_admin_id);

        // master sets range [4, 64]
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "PUT",
                "/api/system/policies/password.length",
                Some(&m_tok),
                Some(&serde_json::json!({"kind":"range","min":4,"max":64})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // realm sets [8, 32] — inside master, OK
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "PUT",
                "/api/realms/acme/policies/password.length",
                Some(&r_tok),
                Some(&serde_json::json!({"kind":"range","min":8,"max":32})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // realm tries [2, 100] — violates master → 409
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "PUT",
                "/api/realms/acme/policies/password.length",
                Some(&r_tok),
                Some(&serde_json::json!({"kind":"range","min":2,"max":100})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn master_tighten_cascades_into_realm_value() {
        let (state, _dir, master_id, realm_admin_id) = state_with_realm_and_admin().await;
        let m_tok = master_token(&state, &master_id);
        let r_tok = realm_token(&state, "acme", &realm_admin_id);

        // master [4, 64], realm [8, 32]
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "PUT",
            "/api/system/policies/password.length",
            Some(&m_tok),
            Some(&serde_json::json!({"kind":"range","min":4,"max":64})),
        ))
        .await
        .unwrap();
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "PUT",
            "/api/realms/acme/policies/password.length",
            Some(&r_tok),
            Some(&serde_json::json!({"kind":"range","min":8,"max":32})),
        ))
        .await
        .unwrap();

        // master tightens to [10, 20]; cascade flag should report it
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "PUT",
                "/api/system/policies/password.length",
                Some(&m_tok),
                Some(&serde_json::json!({"kind":"range","min":10,"max":20})),
            ))
            .await
            .unwrap();
        let j = json_body(resp).await;
        assert_eq!(j["cascaded"].as_array().unwrap().len(), 1);
        let outcome = &j["cascaded"][0];
        assert_eq!(outcome["realm"], "acme");
        assert_eq!(outcome["after"]["min"], 10);
        assert_eq!(outcome["after"]["max"], 20);

        // realm value reflects the clamp
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/policies/password.length",
                Some(&r_tok),
                None,
            ))
            .await
            .unwrap();
        let j = json_body(resp).await;
        assert_eq!(j["min"], 10);
        assert_eq!(j["max"], 20);
    }

    #[tokio::test]
    async fn realm_admin_cannot_set_master_policy() {
        let (state, _dir, _, realm_admin_id) = state_with_realm_and_admin().await;
        let r_tok = realm_token(&state, "acme", &realm_admin_id);
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "PUT",
                "/api/system/policies/password.length",
                Some(&r_tok),
                Some(&serde_json::json!({"kind":"range","min":4,"max":64})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn list_records_with_malformed_filter_is_400() {
        let (state, _dir, tok) = state_with_app_and_collection().await;
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/collections/notes/records?filter=this+is+not+valid",
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn cross_realm_admin_cannot_read_records() {
        let (state, _dir, _) = state_with_app_and_collection().await;
        // create realm 'widgetco' + its own admin, and try to list acme/mobile/notes/records
        let master_admin_id = rustbase_db::admins::count_master_admins(state.system.pool())
            .await
            .map(|_| {
                // we can't get the id back from count; just decode the master admin from email
                // — simpler: pull one row
            });
        let _ = master_admin_id;

        // simpler: derive the master id from the inserted row
        let row: (String,) = sqlx::query_as("SELECT id FROM master_admins LIMIT 1")
            .fetch_one(state.system.pool())
            .await
            .unwrap();
        let master_tok = master_token(&state, &row.0);

        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms",
            Some(&master_tok),
            Some(&serde_json::json!({"id":"widgetco","name":"WidgetCo"})),
        ))
        .await
        .unwrap();
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/widgetco/admins",
            Some(&master_tok),
            Some(&serde_json::json!({"email":"w@w","password":"longenough"})),
        ))
        .await
        .unwrap();

        // widgetco admin token (we know the id from the create response? we didn't capture
        // it — just use master_token + role swap)
        let claims = rustbase_auth::build_claims(
            "fake-admin",
            rustbase_auth::TokenRole::RealmAdmin,
            Some("widgetco".into()),
            None,
            chrono::Duration::minutes(15),
        );
        let w_tok = rustbase_auth::encode_token(&claims, &state.master_key).unwrap();

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/collections/notes/records",
                Some(&w_tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // ------------- user-lifecycle hooks -------------

    /// Plant a hook source file under the AppState's data_dir at the
    /// path apps::create would normally read from, then load it via
    /// the HookEngine using the same bridge + mailer wiring as
    /// production. Returns once the hook is live.
    async fn plant_hook_in_app(state: &AppState, realm: &str, app: &str, src: &str) {
        let dir = state.data_dir.join("hooks").join(realm).join(app);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("user_lifecycle.js"), src).unwrap();
        let bridge = crate::hook_bridge::ApiBridge::new(
            rustbase_core::RealmId::from(realm.to_string()),
            rustbase_core::AppId::from(app.to_string()),
            state.apps.clone(),
        )
        .into_sync();
        let quoted = std::sync::Arc::new(crate::mailer::QuotedMailer::new(
            state.mailer.clone(),
            rustbase_core::RealmId::from(realm.to_string()),
            rustbase_core::AppId::from(app.to_string()),
            state.apps.clone(),
        )) as std::sync::Arc<dyn rustbase_core::Mailer>;
        state
            .hooks
            .load_app(realm, app, &dir, Some(bridge), Some(quoted))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn user_after_register_hook_fires_on_password_signup() {
        let (state, _dir, tok) = state_with_app_and_collection().await;
        plant_hook_in_app(
            &state,
            "acme",
            "mobile",
            r#"$app.onUserAfterRegister((u) => $app.log("welcome " + u.email));"#,
        )
        .await;

        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/register",
                None,
                Some(&serde_json::json!({"email":"ada@acme.com","password":"hunter22"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let logs = state
            .hooks
            .get("acme", "mobile")
            .unwrap()
            .drain_logs()
            .await
            .unwrap();
        assert_eq!(logs, vec!["welcome ada@acme.com".to_string()]);
        let _ = tok; // keep realm-admin token alive through the test
    }

    #[tokio::test]
    async fn user_after_login_hook_fires_on_password_login() {
        let (state, _dir, _, _) = state_with_collection_and_user().await;
        plant_hook_in_app(
            &state,
            "acme",
            "mobile",
            r#"$app.onUserAfterLogin((u) => $app.log("login:" + u.email));"#,
        )
        .await;

        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/login",
                None,
                Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let logs = state
            .hooks
            .get("acme", "mobile")
            .unwrap()
            .drain_logs()
            .await
            .unwrap();
        assert_eq!(logs, vec!["login:u@acme.com".to_string()]);
    }

    #[tokio::test]
    async fn user_before_login_hook_can_veto_password_login() {
        let (state, _dir, _, _) = state_with_collection_and_user().await;
        plant_hook_in_app(
            &state,
            "acme",
            "mobile",
            r#"$app.onUserBeforeLogin((u) => { throw new Error("banned:" + u.email); });"#,
        )
        .await;

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/login",
                None,
                // Credentials are CORRECT — the hook is what blocks.
                Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn user_after_login_hook_does_not_see_password_hash() {
        // Defensive: the user object handed to hooks must only carry
        // public fields. Even though the underlying User struct has
        // password_hash, the JSON we pass to the JS runtime must not.
        let (state, _dir, _, _) = state_with_collection_and_user().await;
        plant_hook_in_app(
            &state,
            "acme",
            "mobile",
            r#"$app.onUserAfterLogin((u) => $app.log("keys:" + Object.keys(u).sort().join(",")));"#,
        )
        .await;

        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps/mobile/auth/users/login",
            None,
            Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
        ))
        .await
        .unwrap();

        let logs = state
            .hooks
            .get("acme", "mobile")
            .unwrap()
            .drain_logs()
            .await
            .unwrap();
        assert_eq!(logs, vec!["keys:email,id,verified".to_string()]);
    }

    #[tokio::test]
    async fn user_after_register_and_login_both_fire_on_otp_signup() {
        // OTP signup creates a brand-new user — register + login
        // events should both fire from the same /otp/login call.
        let (state, _dir, _tok) = state_with_app_and_collection().await;
        plant_hook_in_app(
            &state,
            "acme",
            "mobile",
            r#"
            $app.onUserAfterRegister((u) => $app.log("reg:" + u.email));
            $app.onUserAfterLogin((u)    => $app.log("log:" + u.email));
            "#,
        )
        .await;

        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps/mobile/auth/otp/request",
            None,
            Some(&serde_json::json!({"email":"fresh@acme.com"})),
        ))
        .await
        .unwrap();
        let code = {
            let pool = state
                .apps
                .pool_for(
                    &rustbase_core::RealmId::from("acme".to_string()),
                    &rustbase_core::AppId::from("mobile".to_string()),
                )
                .await
                .unwrap();
            let row: (String,) = sqlx::query_as(
                "SELECT code FROM _email_otps WHERE email = ? AND consumed_at IS NULL \
                 ORDER BY issued_at DESC LIMIT 1",
            )
            .bind("fresh@acme.com")
            .fetch_one(&pool)
            .await
            .unwrap();
            row.0
        };

        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/otp/login",
                None,
                Some(&serde_json::json!({"email":"fresh@acme.com","code":code})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let mut logs = state
            .hooks
            .get("acme", "mobile")
            .unwrap()
            .drain_logs()
            .await
            .unwrap();
        logs.sort();
        assert_eq!(
            logs,
            vec![
                "log:fresh@acme.com".to_string(),
                "reg:fresh@acme.com".to_string(),
            ]
        );
    }

    // ------------- $app.routerAdd integration -------------

    #[tokio::test]
    async fn router_add_get_returns_handler_json() {
        let (state, _dir, _) = state_with_app_and_collection().await;
        plant_hook_in_app(
            &state,
            "acme",
            "mobile",
            r#"
            $app.routerAdd("GET", "/hello", (ctx) => ({
                status: 200,
                body: { method: ctx.method, who: ctx.query.who || "stranger" },
            }));
            "#,
        )
        .await;

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/custom/hello?who=ada",
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert_eq!(j["method"], "GET");
        assert_eq!(j["who"], "ada");
    }

    #[tokio::test]
    async fn router_add_unknown_path_returns_404() {
        let (state, _dir, _) = state_with_app_and_collection().await;
        // No routerAdd at all → catch-all should answer 404.
        plant_hook_in_app(&state, "acme", "mobile", "/* nothing */").await;

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/custom/missing",
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn router_add_post_sees_json_body_and_headers() {
        let (state, _dir, _) = state_with_app_and_collection().await;
        plant_hook_in_app(
            &state,
            "acme",
            "mobile",
            r#"
            $app.routerAdd("POST", "/echo", (ctx) => ({
                body: {
                    got_body: ctx.body,
                    saw_content_type: ctx.headers["content-type"],
                },
            }));
            "#,
        )
        .await;

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/custom/echo",
                None,
                Some(&serde_json::json!({"hello":"world","n":42})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert_eq!(j["got_body"]["hello"], "world");
        assert_eq!(j["got_body"]["n"], 42);
        assert_eq!(j["saw_content_type"], "application/json");
    }

    #[tokio::test]
    async fn router_add_handler_throw_returns_500() {
        let (state, _dir, _) = state_with_app_and_collection().await;
        plant_hook_in_app(
            &state,
            "acme",
            "mobile",
            r#"$app.routerAdd("GET", "/boom", () => { throw new Error("kapow"); });"#,
        )
        .await;

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/custom/boom",
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let j = json_body(resp).await;
        assert_eq!(j["error"], "kapow");
    }

    #[tokio::test]
    async fn router_add_method_mismatch_returns_404() {
        let (state, _dir, _) = state_with_app_and_collection().await;
        plant_hook_in_app(
            &state,
            "acme",
            "mobile",
            r#"$app.routerAdd("GET", "/only-get", () => ({body:"ok"}));"#,
        )
        .await;

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/custom/only-get",
                None,
                Some(&serde_json::json!({})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn router_add_routes_are_scoped_to_the_owning_app() {
        // Same path registered in app A; app B has nothing — the
        // request to B's namespace must miss even though A would
        // have answered.
        let (state, _dir, master_tok, realm_admin_id) = {
            let (state, dir, master_id, realm_admin_id) = state_with_realm_and_admin().await;
            let master_tok = master_token(&state, &master_id);
            (state, dir, master_tok, realm_admin_id)
        };
        let realm_tok = realm_token(&state, "acme", &realm_admin_id);

        // Create two sibling apps under acme.
        for app_id in ["alpha", "beta"] {
            let app = build_router(state.clone());
            let resp = app
                .oneshot(req_with_auth(
                    "POST",
                    "/api/realms/acme/apps",
                    Some(&realm_tok),
                    Some(&serde_json::json!({"id":app_id,"name":app_id})),
                ))
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::CREATED);
        }

        // Plant a hook only in alpha.
        plant_hook_in_app(
            &state,
            "acme",
            "alpha",
            r#"$app.routerAdd("GET", "/hi", () => ({body:"from-alpha"}));"#,
        )
        .await;

        // alpha answers.
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/alpha/custom/hi",
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // beta does NOT — same path, different app namespace.
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/beta/custom/hi",
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let _ = master_tok;
    }

    // ------------- TOTP 2FA -------------

    /// Compute the current valid TOTP code for a base32 secret using
    /// the same parameters the server uses. Mirrors
    /// crate::auth::totp::build_totp.
    fn current_totp_code(secret_b32: &str) -> String {
        let bytes = totp_rs::Secret::Encoded(secret_b32.to_string())
            .to_bytes()
            .unwrap();
        let totp = totp_rs::TOTP::new_unchecked(
            totp_rs::Algorithm::SHA1,
            6,
            1,
            30,
            bytes,
            None,
            String::new(),
        );
        totp.generate_current().unwrap()
    }

    #[tokio::test]
    async fn totp_enroll_returns_secret_and_otpauth_url() {
        let (state, _dir, _, user_tok) = state_with_collection_and_user().await;
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/totp/enroll",
                Some(&user_tok),
                Some(&serde_json::json!({})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        let secret = j["secret_b32"].as_str().unwrap();
        assert!(!secret.is_empty());
        let url = j["otpauth_url"].as_str().unwrap();
        assert!(url.starts_with("otpauth://totp/"), "got: {url}");
        assert!(url.contains("RustBase"), "issuer should appear: {url}");
        assert!(url.contains("u%40acme.com") || url.contains("u@acme.com"));
    }

    #[tokio::test]
    async fn totp_confirm_with_valid_code_enables_2fa() {
        let (state, _dir, _, user_tok) = state_with_collection_and_user().await;
        // Enroll first.
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/totp/enroll",
                Some(&user_tok),
                Some(&serde_json::json!({})),
            ))
            .await
            .unwrap();
        let secret = json_body(resp).await["secret_b32"]
            .as_str()
            .unwrap()
            .to_string();

        // Confirm with the right code.
        let code = current_totp_code(&secret);
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/totp/confirm",
                Some(&user_tok),
                Some(&serde_json::json!({"code": code})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(json_body(resp).await["status"], "enabled");

        // Row should now be enabled.
        let pool = state
            .apps
            .pool_for(
                &rustbase_core::RealmId::from("acme".to_string()),
                &rustbase_core::AppId::from("mobile".to_string()),
            )
            .await
            .unwrap();
        let user = rustbase_db::users::find_user_by_email(&pool, "u@acme.com")
            .await
            .unwrap()
            .unwrap();
        let row = rustbase_db::user_totp::find(&pool, &user.id)
            .await
            .unwrap()
            .unwrap();
        assert!(row.enabled);
    }

    #[tokio::test]
    async fn totp_confirm_with_wrong_code_returns_401_and_keeps_pending() {
        let (state, _dir, _, user_tok) = state_with_collection_and_user().await;
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps/mobile/auth/totp/enroll",
            Some(&user_tok),
            Some(&serde_json::json!({})),
        ))
        .await
        .unwrap();

        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/totp/confirm",
                Some(&user_tok),
                Some(&serde_json::json!({"code": "000000"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let pool = state
            .apps
            .pool_for(
                &rustbase_core::RealmId::from("acme".to_string()),
                &rustbase_core::AppId::from("mobile".to_string()),
            )
            .await
            .unwrap();
        let user = rustbase_db::users::find_user_by_email(&pool, "u@acme.com")
            .await
            .unwrap()
            .unwrap();
        let row = rustbase_db::user_totp::find(&pool, &user.id)
            .await
            .unwrap()
            .unwrap();
        assert!(!row.enabled, "wrong code must NOT enable");
    }

    /// Drive enrol + confirm so the user comes out the other side with
    /// TOTP=enabled. Returns (state, dir, secret_b32) so a follow-up
    /// test can drive the two-step login.
    async fn state_with_totp_enabled_user() -> (AppState, tempfile::TempDir, String) {
        let (state, dir, _, user_tok) = state_with_collection_and_user().await;

        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/totp/enroll",
                Some(&user_tok),
                Some(&serde_json::json!({})),
            ))
            .await
            .unwrap();
        let secret = json_body(resp).await["secret_b32"]
            .as_str()
            .unwrap()
            .to_string();

        let code = current_totp_code(&secret);
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps/mobile/auth/totp/confirm",
            Some(&user_tok),
            Some(&serde_json::json!({"code": code})),
        ))
        .await
        .unwrap();
        (state, dir, secret)
    }

    #[tokio::test]
    async fn login_with_totp_enabled_returns_mfa_challenge_not_tokens() {
        let (state, _dir, _secret) = state_with_totp_enabled_user().await;
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/login",
                None,
                Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert_eq!(j["mfa_required"], true);
        assert!(j["mfa_token"].as_str().unwrap().len() == 64);
        assert!(j.get("access_token").is_none(), "no tokens yet");
    }

    #[tokio::test]
    async fn login_totp_second_step_returns_full_tokens() {
        let (state, _dir, secret) = state_with_totp_enabled_user().await;

        // Step 1: password login → mfa challenge.
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/login",
                None,
                Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
            ))
            .await
            .unwrap();
        let mfa_token = json_body(resp).await["mfa_token"]
            .as_str()
            .unwrap()
            .to_string();

        // Step 2: redeem.
        let code = current_totp_code(&secret);
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/login/totp",
                None,
                Some(&serde_json::json!({"mfa_token": mfa_token, "code": code})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert!(j["access_token"].as_str().unwrap().starts_with("ey"));
        assert!(j["refresh_token"].as_str().unwrap().starts_with("rfsh_"));
        assert_eq!(j["user"]["email"], "u@acme.com");
    }

    #[tokio::test]
    async fn login_totp_replayed_challenge_returns_401() {
        let (state, _dir, secret) = state_with_totp_enabled_user().await;

        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/login",
                None,
                Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
            ))
            .await
            .unwrap();
        let mfa_token = json_body(resp).await["mfa_token"]
            .as_str()
            .unwrap()
            .to_string();

        let code = current_totp_code(&secret);
        // First redemption: ok.
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/login/totp",
                None,
                Some(&serde_json::json!({"mfa_token": &mfa_token, "code": &code})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Replay with the same mfa_token: 401.
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/login/totp",
                None,
                Some(&serde_json::json!({"mfa_token": &mfa_token, "code": &code})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn totp_disable_with_valid_code_clears_the_row() {
        let (state, _dir, secret) = state_with_totp_enabled_user().await;
        // Need a fresh user token (TOTP-enabled users can't login with
        // password alone any more).
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/login",
                None,
                Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
            ))
            .await
            .unwrap();
        let mfa_token = json_body(resp).await["mfa_token"]
            .as_str()
            .unwrap()
            .to_string();
        let code1 = current_totp_code(&secret);
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/login/totp",
                None,
                Some(&serde_json::json!({"mfa_token": mfa_token, "code": code1})),
            ))
            .await
            .unwrap();
        let user_tok = json_body(resp).await["access_token"]
            .as_str()
            .unwrap()
            .to_string();

        // Now disable.
        let code2 = current_totp_code(&secret);
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/totp/disable",
                Some(&user_tok),
                Some(&serde_json::json!({"code": code2})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Row gone → next password login returns tokens directly.
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/login",
                None,
                Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
            ))
            .await
            .unwrap();
        let j = json_body(resp).await;
        assert!(j["access_token"].as_str().unwrap().starts_with("ey"));
        assert!(j.get("mfa_required").is_none());
    }

    // ------------- OAuth provider admin CRUD -------------

    fn provider_body() -> serde_json::Value {
        serde_json::json!({
            "client_id": "google-client-1",
            "client_secret": "shh-very-secret",
            "config": {
                "auth_url":     "https://accounts.google.com/o/oauth2/v2/auth",
                "token_url":    "https://oauth2.googleapis.com/token",
                "userinfo_url": "https://openidconnect.googleapis.com/v1/userinfo",
                "scopes":       ["openid", "email"],
                "userinfo_id_field":    "/sub",
                "userinfo_email_field": "/email",
            },
        })
    }

    #[tokio::test]
    async fn oauth_admin_put_then_get_returns_provider_without_secret() {
        let (state, _dir, master_id, _realm_admin_id) = state_with_realm_and_admin().await;
        let master_tok = master_token(&state, &master_id);

        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "PUT",
                "/api/realms/acme/apps/mobile/auth/oauth/providers/google",
                Some(&master_tok),
                Some(&provider_body()),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert_eq!(j["provider"], "google");
        assert_eq!(j["client_id"], "google-client-1");
        assert!(
            j.get("client_secret").is_none(),
            "PUT must not echo the secret"
        );

        // GET reads it back; still no secret in the response.
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/auth/oauth/providers/google",
                Some(&master_tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert_eq!(j["client_id"], "google-client-1");
        assert!(j.get("client_secret").is_none());
        assert_eq!(
            j["config"]["token_url"],
            "https://oauth2.googleapis.com/token"
        );
    }

    #[tokio::test]
    async fn oauth_admin_stored_secret_is_encrypted_at_rest() {
        let (state, _dir, master_id, _) = state_with_realm_and_admin().await;
        let master_tok = master_token(&state, &master_id);
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "PUT",
            "/api/realms/acme/apps/mobile/auth/oauth/providers/google",
            Some(&master_tok),
            Some(&provider_body()),
        ))
        .await
        .unwrap();

        // Read the raw row: client_secret_enc must NOT contain the
        // plaintext anywhere. AES-GCM ciphertext is opaque bytes.
        let pool = state
            .apps
            .pool_for(
                &rustbase_core::RealmId::from("acme".to_string()),
                &rustbase_core::AppId::from("mobile".to_string()),
            )
            .await
            .unwrap();
        let (ct,): (Vec<u8>,) =
            sqlx::query_as("SELECT client_secret_enc FROM oauth_providers WHERE provider = ?")
                .bind("google")
                .fetch_one(&pool)
                .await
                .unwrap();
        let as_str = String::from_utf8_lossy(&ct);
        assert!(
            !as_str.contains("shh-very-secret"),
            "raw row leaks plaintext: {as_str:?}"
        );
        // Sanity: KEK-aware decrypt round-trips.
        let pt = rustbase_auth::decrypt(&ct, state.oauth_kek.as_ref()).unwrap();
        assert_eq!(pt, b"shh-very-secret");
    }

    #[tokio::test]
    async fn oauth_admin_list_returns_summaries_in_provider_order() {
        let (state, _dir, master_id, _) = state_with_realm_and_admin().await;
        let master_tok = master_token(&state, &master_id);

        for name in ["google", "github"] {
            let mut body = provider_body();
            body["client_id"] = serde_json::Value::String(format!("{name}-id"));
            let app = build_router(state.clone());
            app.oneshot(req_with_auth(
                "PUT",
                &format!("/api/realms/acme/apps/mobile/auth/oauth/providers/{name}"),
                Some(&master_tok),
                Some(&body),
            ))
            .await
            .unwrap();
        }

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/auth/oauth/providers",
                Some(&master_tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        let arr = j.as_array().unwrap();
        let names: Vec<_> = arr
            .iter()
            .map(|p| p["provider"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["github", "google"]);
        for entry in arr {
            assert!(entry.get("client_secret").is_none());
        }
    }

    #[tokio::test]
    async fn oauth_admin_delete_then_get_returns_404() {
        let (state, _dir, master_id, _) = state_with_realm_and_admin().await;
        let master_tok = master_token(&state, &master_id);
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "PUT",
            "/api/realms/acme/apps/mobile/auth/oauth/providers/google",
            Some(&master_tok),
            Some(&provider_body()),
        ))
        .await
        .unwrap();

        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "DELETE",
                "/api/realms/acme/apps/mobile/auth/oauth/providers/google",
                Some(&master_tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // Double-delete is 404 (no row).
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "DELETE",
                "/api/realms/acme/apps/mobile/auth/oauth/providers/google",
                Some(&master_tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        // GET also misses.
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/auth/oauth/providers/google",
                Some(&master_tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn oauth_admin_realm_admin_can_manage_own_realm() {
        let (state, _dir, _, realm_admin_id) = state_with_realm_and_admin().await;
        let realm_tok = realm_token(&state, "acme", &realm_admin_id);

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "PUT",
                "/api/realms/acme/apps/mobile/auth/oauth/providers/google",
                Some(&realm_tok),
                Some(&provider_body()),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn oauth_admin_requires_admin_token() {
        let (state, _dir, _, _) = state_with_collection_and_user().await;
        // user_tok is on the same realm but is not an admin.
        let app = build_router(state.clone());
        let user_resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/login",
                None,
                Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
            ))
            .await
            .unwrap();
        let user_tok = json_body(user_resp).await["access_token"]
            .as_str()
            .unwrap()
            .to_string();

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "PUT",
                "/api/realms/acme/apps/mobile/auth/oauth/providers/google",
                Some(&user_tok),
                Some(&provider_body()),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn oauth_admin_end_to_end_callback_works_after_put() {
        // PUT a provider via the admin endpoint, then drive the full
        // OAuth callback against the stub — proves encrypt → store →
        // find → decrypt → token exchange round-trips.
        let (base_url, _h) = fake_oauth_provider(serde_json::json!({
            "sub": "google-sub-77",
            "email": "via-admin@acme.test",
        }))
        .await;
        let (state, _dir, master_id, _) = state_with_realm_and_admin().await;
        let master_tok = master_token(&state, &master_id);

        let mut body = provider_body();
        body["client_secret"] = serde_json::Value::String("admin-put-secret".into());
        body["config"]["auth_url"] = serde_json::Value::String(format!("{base_url}/authorize"));
        body["config"]["token_url"] = serde_json::Value::String(format!("{base_url}/token"));
        body["config"]["userinfo_url"] = serde_json::Value::String(format!("{base_url}/userinfo"));
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "PUT",
            "/api/realms/acme/apps/mobile/auth/oauth/providers/google",
            Some(&master_tok),
            Some(&body),
        ))
        .await
        .unwrap();

        // /authorize → state
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/auth/oauth/google/authorize?redirect_uri=https%3A%2F%2Fapp%2Fcb",
                None,
                None,
            ))
            .await
            .unwrap();
        let nonce = json_body(resp).await["state"].as_str().unwrap().to_string();

        // /callback
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/oauth/google/callback",
                None,
                Some(&serde_json::json!({"code":"unused","state":nonce})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert!(j["access_token"].as_str().unwrap().starts_with("ey"));
        assert_eq!(j["user"]["email"], "via-admin@acme.test");
    }

    #[tokio::test]
    async fn oauth_admin_put_without_secret_preserves_existing_ciphertext() {
        // Create with a real secret, then PUT again with only client_id
        // and config — the stored ciphertext should still decrypt to
        // the original secret. This is what the edit form relies on.
        let (state, _dir, master_id, _) = state_with_realm_and_admin().await;
        let master_tok = master_token(&state, &master_id);
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "PUT",
            "/api/realms/acme/apps/mobile/auth/oauth/providers/google",
            Some(&master_tok),
            Some(&provider_body()),
        ))
        .await
        .unwrap();

        // Edit without sending client_secret.
        let mut body = provider_body();
        body.as_object_mut().unwrap().remove("client_secret");
        body["client_id"] = serde_json::Value::String("rotated-id".into());
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "PUT",
                "/api/realms/acme/apps/mobile/auth/oauth/providers/google",
                Some(&master_tok),
                Some(&body),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Pull the raw ciphertext and decrypt with the server's KEK —
        // it should still match the ORIGINAL secret.
        let pool = state
            .apps
            .pool_for(
                &rustbase_core::RealmId::from("acme".to_string()),
                &rustbase_core::AppId::from("mobile".to_string()),
            )
            .await
            .unwrap();
        let (ct,): (Vec<u8>,) =
            sqlx::query_as("SELECT client_secret_enc FROM oauth_providers WHERE provider = ?")
                .bind("google")
                .fetch_one(&pool)
                .await
                .unwrap();
        let pt = rustbase_auth::decrypt(&ct, state.oauth_kek.as_ref()).unwrap();
        assert_eq!(pt, b"shh-very-secret");
        // And the new client_id stuck.
        let app = build_router(state);
        let detail = json_body(
            app.oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/auth/oauth/providers/google",
                Some(&master_tok),
                None,
            ))
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(detail["client_id"], "rotated-id");
    }

    #[tokio::test]
    async fn oauth_admin_put_without_secret_on_create_returns_400() {
        let (state, _dir, master_id, _) = state_with_realm_and_admin().await;
        let master_tok = master_token(&state, &master_id);
        let mut body = provider_body();
        body.as_object_mut().unwrap().remove("client_secret");
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "PUT",
                "/api/realms/acme/apps/mobile/auth/oauth/providers/google",
                Some(&master_tok),
                Some(&body),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // ------------- admin user management -------------

    /// Bootstrap: realm with two pre-registered end users + a
    /// realm-admin token. Helper to keep the user-admin tests short.
    async fn state_with_two_users() -> (AppState, tempfile::TempDir, String) {
        let (state, dir, _, realm_admin_id) = state_with_realm_and_admin().await;
        let tok = realm_token(&state, "acme", &realm_admin_id);
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps/mobile/auth/users/register",
            None,
            Some(&serde_json::json!({"email":"alice@acme.com","password":"alicepass1"})),
        ))
        .await
        .unwrap();
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps/mobile/auth/users/register",
            None,
            Some(&serde_json::json!({"email":"bob@acme.com","password":"bobpass1"})),
        ))
        .await
        .unwrap();
        (state, dir, tok)
    }

    #[tokio::test]
    async fn admin_list_users_paginates_and_filters() {
        let (state, _dir, tok) = state_with_two_users().await;
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/users?per_page=10",
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert_eq!(j["total_items"], 2);
        assert_eq!(j["items"].as_array().unwrap().len(), 2);
        // Substring filter on email.
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/users?q=alice",
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        let j = json_body(resp).await;
        assert_eq!(j["total_items"], 1);
        assert_eq!(j["items"][0]["email"], "alice@acme.com");
    }

    #[tokio::test]
    async fn admin_get_user_returns_totp_status_and_oauth_links() {
        let (state, _dir, tok) = state_with_two_users().await;
        // Look up Alice's id via the list endpoint.
        let app = build_router(state.clone());
        let list = json_body(
            app.oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/users?q=alice",
                Some(&tok),
                None,
            ))
            .await
            .unwrap(),
        )
        .await;
        let alice_id = list["items"][0]["id"].as_str().unwrap().to_string();

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                &format!("/api/realms/acme/apps/mobile/users/{alice_id}"),
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert_eq!(j["email"], "alice@acme.com");
        assert_eq!(j["verified"], false);
        // No TOTP enrolled, no OAuth links.
        assert!(j["totp"].is_null());
        assert!(j["oauth_links"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn admin_force_verify_flips_the_flag() {
        let (state, _dir, tok) = state_with_two_users().await;
        let app = build_router(state.clone());
        let list = json_body(
            app.oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/users?q=alice",
                Some(&tok),
                None,
            ))
            .await
            .unwrap(),
        )
        .await;
        let alice_id = list["items"][0]["id"].as_str().unwrap().to_string();

        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "PATCH",
                &format!("/api/realms/acme/apps/mobile/users/{alice_id}/verify"),
                Some(&tok),
                Some(&serde_json::json!({})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let app = build_router(state);
        let detail = json_body(
            app.oneshot(req_with_auth(
                "GET",
                &format!("/api/realms/acme/apps/mobile/users/{alice_id}"),
                Some(&tok),
                None,
            ))
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(detail["verified"], true);
    }

    #[tokio::test]
    async fn admin_delete_user_cascades_and_returns_404_on_replay() {
        let (state, _dir, tok) = state_with_two_users().await;
        let app = build_router(state.clone());
        let alice_id = json_body(
            app.oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/users?q=alice",
                Some(&tok),
                None,
            ))
            .await
            .unwrap(),
        )
        .await["items"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();

        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "DELETE",
                &format!("/api/realms/acme/apps/mobile/users/{alice_id}"),
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "DELETE",
                &format!("/api/realms/acme/apps/mobile/users/{alice_id}"),
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        // Bob still there.
        let app = build_router(state);
        let list = json_body(
            app.oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/users",
                Some(&tok),
                None,
            ))
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(list["total_items"], 1);
    }

    #[tokio::test]
    async fn admin_reset_totp_clears_pending_enrolment() {
        let (state, _dir, tok) = state_with_two_users().await;
        let pool = state
            .apps
            .pool_for(
                &rustbase_core::RealmId::from("acme".to_string()),
                &rustbase_core::AppId::from("mobile".to_string()),
            )
            .await
            .unwrap();
        let alice = rustbase_db::users::find_user_by_email(&pool, "alice@acme.com")
            .await
            .unwrap()
            .unwrap();
        // Plant a pending TOTP enrolment so we have something to clear.
        rustbase_db::user_totp::enroll(&pool, &alice.id, "ABCDEF234567")
            .await
            .unwrap();
        assert!(
            rustbase_db::user_totp::find(&pool, &alice.id)
                .await
                .unwrap()
                .is_some()
        );

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "DELETE",
                &format!("/api/realms/acme/apps/mobile/users/{}/totp", alice.id),
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        assert!(
            rustbase_db::user_totp::find(&pool, &alice.id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn admin_users_require_admin_token() {
        let (state, _dir, _, _) = state_with_collection_and_user().await;
        // user_tok belongs to an end-user, not an admin.
        let user_tok = json_body(
            build_router(state.clone())
                .oneshot(req_with_auth(
                    "POST",
                    "/api/realms/acme/apps/mobile/auth/users/login",
                    None,
                    Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
                ))
                .await
                .unwrap(),
        )
        .await["access_token"]
            .as_str()
            .unwrap()
            .to_string();

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/users",
                Some(&user_tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // ------------- hook source file CRUD -------------

    /// Bootstrap to: realm 'acme' + app 'mobile' (no hooks yet). Returns
    /// (state, dir, realm_admin_token).
    async fn state_with_app() -> (AppState, tempfile::TempDir, String) {
        let (state, dir, _, realm_admin_id) = state_with_realm_and_admin().await;
        let tok = realm_token(&state, "acme", &realm_admin_id);
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps",
            Some(&tok),
            Some(&serde_json::json!({"id":"mobile","name":"M"})),
        ))
        .await
        .unwrap();
        (state, dir, tok)
    }

    #[tokio::test]
    async fn hooks_list_is_empty_for_fresh_app() {
        let (state, _dir, tok) = state_with_app().await;
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/hooks",
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert!(j.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn hooks_put_writes_file_and_returns_reload_outcome() {
        let (state, dir, tok) = state_with_app().await;
        // Use the actual hook surface — `$app.onRecordAfterCreate` is
        // defined in the sandbox, while `console` is not.
        let src = "$app.onRecordAfterCreate(\"notes\", function(rec){});";
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "PUT",
                "/api/realms/acme/apps/mobile/hooks/log.js",
                Some(&tok),
                Some(&serde_json::json!({"source": src})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert_eq!(j["file"]["filename"], "log.js");
        assert_eq!(j["file"]["source"], src);
        // Reload reports zero errors for valid JS.
        assert_eq!(j["reload"]["errors"].as_array().unwrap().len(), 0);
        assert_eq!(j["reload"]["loaded"], 1);

        // File landed in the expected path.
        let path = dir.path().join("hooks/acme/mobile/log.js");
        assert!(path.exists());
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert_eq!(on_disk, src);

        // GET round-trips the body.
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/hooks/log.js",
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert_eq!(j["source"], src);
    }

    #[tokio::test]
    async fn hooks_put_with_invalid_filename_is_400() {
        let (state, _dir, tok) = state_with_app().await;
        // axum normalizes "../" and "subdir/" before reaching the
        // handler, so those route to 404. The cases below all reach
        // the handler and exercise its own validation.
        for bad in ["no-ext", ".hidden.js", "weird name.js"] {
            let encoded = bad.replace(' ', "%20");
            let app = build_router(state.clone());
            let resp = app
                .oneshot(req_with_auth(
                    "PUT",
                    &format!("/api/realms/acme/apps/mobile/hooks/{encoded}"),
                    Some(&tok),
                    Some(&serde_json::json!({"source": ""})),
                ))
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "for filename {bad}");
        }
    }

    #[tokio::test]
    async fn hooks_put_then_list_returns_metadata() {
        let (state, _dir, tok) = state_with_app().await;

        // create two files
        for (name, src) in [("a.js", "var x = 1;"), ("b.ts", "const y: number = 2;")] {
            let app = build_router(state.clone());
            let resp = app
                .oneshot(req_with_auth(
                    "PUT",
                    &format!("/api/realms/acme/apps/mobile/hooks/{name}"),
                    Some(&tok),
                    Some(&serde_json::json!({"source": src})),
                ))
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/hooks",
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        let j = json_body(resp).await;
        let arr = j.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["filename"], "a.js");
        assert_eq!(arr[1]["filename"], "b.ts");
        assert!(arr[0]["size"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn hooks_delete_removes_file_and_reloads() {
        let (state, dir, tok) = state_with_app().await;

        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "PUT",
            "/api/realms/acme/apps/mobile/hooks/keep.js",
            Some(&tok),
            Some(&serde_json::json!({"source": "// noop"})),
        ))
        .await
        .unwrap();

        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "DELETE",
                "/api/realms/acme/apps/mobile/hooks/keep.js",
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert_eq!(j["loaded"], 0);

        let path = dir.path().join("hooks/acme/mobile/keep.js");
        assert!(!path.exists());

        // GET-after-delete → 404
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/hooks/keep.js",
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn hooks_put_surfaces_compile_errors_in_reload_outcome() {
        let (state, _dir, tok) = state_with_app().await;
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "PUT",
                "/api/realms/acme/apps/mobile/hooks/broken.js",
                Some(&tok),
                // Unterminated string — script eval will throw.
                Some(&serde_json::json!({"source": "console.log('"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        // File is on disk and reload reports the error so the editor
        // can pin it next to the source.
        assert!(!j["reload"]["errors"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn hooks_require_admin_token() {
        let (state, _dir, _) = state_with_app().await;

        // register + login an end-user
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/realms/acme/apps/mobile/auth/users/register",
            None,
            Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
        ))
        .await
        .unwrap();
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps/mobile/auth/users/login",
                None,
                Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
            ))
            .await
            .unwrap();
        let user_tok = json_body(resp).await["access_token"]
            .as_str()
            .unwrap()
            .to_string();

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/apps/mobile/hooks",
                Some(&user_tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // ------------- audit log read-back -------------

    #[tokio::test]
    async fn system_audit_lists_master_scope_entries() {
        // policy_set on master writes a row into system.audit_log via
        // policy_engine::set_master_policy. Driving it through the
        // public PUT endpoint is the simplest seed.
        let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
        let tok = master_token(&state, &admin_id);

        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "PUT",
                "/api/system/policies/password.length",
                Some(&tok),
                Some(&serde_json::json!({"kind":"range","min":4,"max":64})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/system/audit?per_page=10",
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert!(j["total_items"].as_u64().unwrap() >= 1);
        let actions: Vec<String> = j["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["action"].as_str().unwrap().to_string())
            .collect();
        assert!(actions.iter().any(|a| a == "policy_set"));
    }

    #[tokio::test]
    async fn system_audit_filters_by_action_substring() {
        let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
        let tok = master_token(&state, &admin_id);

        // seed two distinct actions
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "PUT",
            "/api/system/policies/password.length",
            Some(&tok),
            Some(&serde_json::json!({"kind":"range","min":4,"max":64})),
        ))
        .await
        .unwrap();
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "DELETE",
            "/api/system/policies/password.length",
            Some(&tok),
            None,
        ))
        .await
        .unwrap();

        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/system/audit?action=delete",
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        let j = json_body(resp).await;
        let items = j["items"].as_array().unwrap();
        assert!(!items.is_empty());
        for e in items {
            assert!(e["action"].as_str().unwrap().contains("delete"), "{e:?}");
        }
    }

    #[tokio::test]
    async fn system_audit_requires_master() {
        let (state, _dir, _, realm_admin_id) = state_with_realm_and_admin().await;
        let realm_tok = realm_token(&state, "acme", &realm_admin_id);
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/system/audit",
                Some(&realm_tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn realm_audit_lists_entries_after_clamp() {
        // Tightening a master bound auto-clamps any realm value that
        // falls outside it AND writes "policy_clamped" into the realm's
        // audit log. That's the cascade we want to surface in the UI.
        let (state, _dir, master_id, realm_admin_id) = state_with_realm_and_admin().await;
        let master_tok = master_token(&state, &master_id);
        let realm_tok = realm_token(&state, "acme", &realm_admin_id);

        // 1. master opens the bound wide
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "PUT",
            "/api/system/policies/password.length",
            Some(&master_tok),
            Some(&serde_json::json!({"kind":"range","min":4,"max":64})),
        ))
        .await
        .unwrap();
        // 2. realm picks a tighter range inside the master bound
        // (realm bound must fit inside the master bound)
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "PUT",
            "/api/realms/acme/policies/password.length",
            Some(&realm_tok),
            Some(&serde_json::json!({"kind":"range","min":6,"max":12})),
        ))
        .await
        .unwrap();
        // 3. master tightens the upper bound below the realm value
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "PUT",
                "/api/system/policies/password.length",
                Some(&master_tok),
                Some(&serde_json::json!({"kind":"range","min":4,"max":10})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // realm-scoped audit should now mention policy_clamped
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/acme/audit",
                Some(&realm_tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        let actions: Vec<String> = j["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["action"].as_str().unwrap().to_string())
            .collect();
        assert!(actions.iter().any(|a| a == "policy_clamped"), "{actions:?}");
    }

    #[tokio::test]
    async fn audit_on_unknown_realm_is_404() {
        let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
        let tok = master_token(&state, &admin_id);
        let app = build_router(state);
        let resp = app
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/nope/audit",
                Some(&tok),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
