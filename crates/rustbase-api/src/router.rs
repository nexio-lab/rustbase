use axum::{
    Router,
    routing::{get, post},
};
use tower_http::trace::TraceLayer;

use crate::health::healthz;
use crate::middleware::setup_gate;
use crate::setup::setup;
use crate::state::AppState;

/// Build the full RustBase HTTP router. Layered with a setup gate (blocks
/// non-bootstrap routes while uninitialized) and a tracing middleware so
/// every request shows up in the access log.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/_/setup", post(setup))
        // /api/realms/<realm>/apps/<app>/... will mount under here once
        // collections / records / auth handlers land.
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
    use rustbase_auth::RevocationSet;
    use rustbase_db::{
        AppPoolManager, RealmPoolManager, SYSTEM_MIGRATIONS, SystemPool, apply_migrations,
        realms::ensure_master_realm,
    };
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use tempfile::tempdir;
    use tower::ServiceExt;

    async fn fresh_state() -> (AppState, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let system = SystemPool::open(dir.path()).await.unwrap();
        apply_migrations(system.pool(), SYSTEM_MIGRATIONS).await.unwrap();
        ensure_master_realm(system.pool()).await.unwrap();
        let state = AppState {
            system: Arc::new(system),
            realms: Arc::new(RealmPoolManager::new(dir.path().to_path_buf(), 4)),
            apps: Arc::new(AppPoolManager::new(dir.path().to_path_buf(), 4)),
            revocations: RevocationSet::default(),
            initialized: Arc::new(AtomicBool::new(false)),
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
        let req = Request::builder().uri("/healthz").body(Body::empty()).unwrap();
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

        let body = serde_json::json!({
            "email": "ada@example.com",
            "password": "supersecret",
            "name": "Ada",
        });
        let req = Request::builder()
            .uri("/_/setup")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let j = json_body(resp).await;
        assert_eq!(j["email"], "ada@example.com");
        assert_eq!(j["name"], "Ada");
        assert!(state.is_initialized());

        // Healthz now reports initialized=true.
        let app2 = build_router(state.clone());
        let req = Request::builder().uri("/healthz").body(Body::empty()).unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        let j = json_body(resp).await;
        assert_eq!(j["initialized"], true);
    }

    #[tokio::test]
    async fn setup_rejects_short_password() {
        let (state, _dir) = fresh_state().await;
        let app = build_router(state);
        let body = serde_json::json!({"email": "a@b.c", "password": "short"});
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
    async fn setup_rejects_invalid_email() {
        let (state, _dir) = fresh_state().await;
        let app = build_router(state);
        let body = serde_json::json!({"email": "not-an-email", "password": "longenough"});
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
        // pre-create one master admin so the second call is a conflict
        rustbase_db::admins::insert_master_admin(
            state.system.pool(),
            "first@example.com",
            "$argon2id$hash",
            None,
        )
        .await
        .unwrap();
        state.mark_initialized();

        let app = build_router(state);
        let body = serde_json::json!({"email": "second@example.com", "password": "supersecret"});
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
}
