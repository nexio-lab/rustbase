//! Shared fixtures + helpers for the router test suite, plus a
//! handful of bootstrap-flow tests that live closer to the
//! fixtures they exercise than to any thematic section.

use crate::router::*;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use rustbase_auth::{RevocationSet, SigningKey};
use rustbase_db::{
    AppPoolManager, SYSTEM_MIGRATIONS, SystemPool, WorkspacePoolManager,
    admins::ensure_seed_master_admin, apply_migrations, workspaces::ensure_master_realm,
};
use rustbase_realtime::RealtimeBroker;
use rustbase_runtime::HookEngine;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tempfile::tempdir;
use tower::ServiceExt;

pub(super) async fn fresh_state() -> (AppState, tempfile::TempDir) {
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
    let hmac = SigningKey::generate();
    let (rsa_key, _pkcs8) = rustbase_auth::generate_rsa_with_pkcs8().unwrap();
    let jwt = Arc::new(rustbase_auth::JwtIssuer::new(rsa_key).with_legacy_hmac(hmac.clone()));
    let state = AppState {
        system: Arc::new(system),
        workspaces: Arc::new(WorkspacePoolManager::new(data_dir.clone(), 4)),
        apps: Arc::new(AppPoolManager::new(data_dir.clone(), 4)),
        revocations: RevocationSet::default(),
        master_key: Arc::new(hmac),
        jwt,
        broker: RealtimeBroker::default(),
        hooks: HookEngine::new(),
        data_dir: Arc::new(data_dir),
        initialized: Arc::new(AtomicBool::new(false)),
        mailer: Arc::new(crate::mailer::LogMailer::new()),
        oauth_kek: Arc::new(rustbase_auth::fresh_kek()),
        storage,
        login_attempts: crate::security::LoginAttempts::new(),
        lockout_policy: crate::security::LockoutPolicy::default(),
        // Tests use plain HTTP; emitting `Secure` cookies would
        // make assertions on the header brittle (and would make a
        // browser drop the cookie outright in a real run).
        cookie_secure: false,
        // Empty allowlist → `$app.fetch` is disabled in tests
        // unless the spec wires its own bridge directly.
        hook_fetch_allowed_hosts: Vec::new(),
    };
    (state, dir)
}

pub(super) async fn json_body(resp: axum::response::Response) -> serde_json::Value {
    let body = to_bytes(resp.into_body(), 4096).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
pub(super) async fn healthz_returns_uninitialized_on_fresh_install() {
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
pub(super) async fn unknown_route_is_blocked_with_503_while_uninitialized() {
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
pub(super) async fn setup_creates_master_admin_and_unlocks_the_server() {
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
pub(super) async fn setup_rejects_short_password() {
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
pub(super) async fn second_setup_returns_409_conflict() {
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
