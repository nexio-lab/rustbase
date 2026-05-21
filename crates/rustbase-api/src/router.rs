use axum::{Router, routing::get};
use tower_http::trace::TraceLayer;

use crate::health::healthz;
use crate::state::AppState;

/// Build the full RustBase HTTP router. Layered with a tracing middleware
/// so every request shows up in the access log.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        // /api/realms/<realm>/apps/<app>/... will mount under here once
        // collections / records / auth handlers land.
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
        AppPoolManager, RealmPoolManager, SystemPool, SYSTEM_MIGRATIONS, apply_migrations,
    };
    use std::sync::Arc;
    use tempfile::tempdir;
    use tower::ServiceExt;

    async fn test_state() -> (AppState, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let system = SystemPool::open(dir.path()).await.unwrap();
        apply_migrations(system.pool(), SYSTEM_MIGRATIONS).await.unwrap();
        let state = AppState {
            system: Arc::new(system),
            realms: Arc::new(RealmPoolManager::new(dir.path().to_path_buf(), 4)),
            apps: Arc::new(AppPoolManager::new(dir.path().to_path_buf(), 4)),
            revocations: RevocationSet::default(),
        };
        (state, dir)
    }

    #[tokio::test]
    async fn healthz_returns_uninitialized_on_fresh_install() {
        let (state, _dir) = test_state().await;
        let app = build_router(state);
        let req = Request::builder().uri("/healthz").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["initialized"], false);
    }

    #[tokio::test]
    async fn healthz_flips_initialized_when_master_admin_exists() {
        let (state, _dir) = test_state().await;
        rustbase_db::admins::insert_master_admin(
            state.system.pool(),
            "ada@example.com",
            "$argon2id$hash",
            None,
        )
        .await
        .unwrap();
        let app = build_router(state);
        let req = Request::builder().uri("/healthz").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body = to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["initialized"], true);
    }

    #[tokio::test]
    async fn unknown_route_returns_404() {
        let (state, _dir) = test_state().await;
        let app = build_router(state);
        let req = Request::builder()
            .uri("/does-not-exist")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
