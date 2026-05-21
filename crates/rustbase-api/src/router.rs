use axum::{
    Router,
    routing::{get, post},
};
use tower_http::trace::TraceLayer;

use crate::apps;
use crate::auth::{
    master_admin_login, master_admin_refresh, realm_admin_login, realm_admin_refresh,
};
use crate::collections;
use crate::health::healthz;
use crate::middleware::setup_gate;
use crate::realm_admins;
use crate::realms;
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
            get(realms::get).patch(realms::update).delete(realms::delete),
        )
        .route("/api/realms/{realm}/admins", post(realm_admins::create))
        .route("/api/realms/{realm}/auth/admin/login", post(realm_admin_login))
        .route("/api/realms/{realm}/auth/refresh", post(realm_admin_refresh))
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
            get(collections::get).delete(collections::delete),
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
        apply_migrations(system.pool().clone(), SYSTEM_MIGRATIONS).await.unwrap();
        ensure_master_realm(system.pool()).await.unwrap();
        let data_dir = dir.path().to_path_buf();
        let state = AppState {
            system: Arc::new(system),
            realms: Arc::new(RealmPoolManager::new(data_dir.clone(), 4)),
            apps: Arc::new(AppPoolManager::new(data_dir.clone(), 4)),
            revocations: RevocationSet::default(),
            master_key: Arc::new(SigningKey::generate()),
            data_dir: Arc::new(data_dir),
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

    // ------------- auth flow tests -------------

    async fn initialized_state_with_admin(
        password: &str,
    ) -> (AppState, tempfile::TempDir, String) {
        let (state, dir) = fresh_state().await;
        let hash = rustbase_auth::hash_password(password).unwrap();
        let admin = rustbase_db::admins::insert_master_admin(
            state.system.pool(),
            "ada@example.com",
            &hash,
            Some("Ada"),
        )
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
        let body = serde_json::json!({"email":"ada@example.com","password":"hunter22"});
        let resp = post_json(app, "/_/auth/admin/login", &body).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let j = json_body(resp).await;
        assert!(j["access_token"].as_str().unwrap().starts_with("ey"));
        assert!(j["refresh_token"].as_str().unwrap().starts_with("rfsh_"));
        assert_eq!(j["admin"]["id"], admin_id);
        assert_eq!(j["admin"]["email"], "ada@example.com");
        assert_eq!(j["admin"]["name"], "Ada");
        assert!(j["admin"].get("password_hash").is_none());
    }

    #[tokio::test]
    async fn login_with_wrong_password_returns_401() {
        let (state, _dir, _) = initialized_state_with_admin("hunter22").await;
        let app = build_router(state);
        let body = serde_json::json!({"email":"ada@example.com","password":"wrong"});
        let resp = post_json(app, "/_/auth/admin/login", &body).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn login_with_unknown_email_returns_401() {
        let (state, _dir, _) = initialized_state_with_admin("hunter22").await;
        let app = build_router(state);
        let body = serde_json::json!({"email":"nobody@example.com","password":"hunter22"});
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
            &serde_json::json!({"email":"ada@example.com","password":"hunter22"}),
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
        // table from the realm schema.
        let realm_pool = state
            .realms
            .pool_for(&rustbase_core::RealmId::from("acme"))
            .await
            .unwrap();
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
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
            .oneshot(req_with_auth(
                "GET",
                "/api/realms/nope",
                Some(&token),
                None,
            ))
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

        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/realms/acme/apps",
                Some(&realm_tok),
                Some(&serde_json::json!({"id":"mobile","name":"Mobile"})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let j = json_body(resp).await;
        assert_eq!(j["id"], "mobile");

        // data.db is initialized — listing collections (the meta table)
        // should be empty without erroring.
        let app_pool = state
            .apps
            .pool_for(
                &rustbase_core::RealmId::from("acme"),
                &rustbase_core::AppId::from("mobile"),
            )
            .await
            .unwrap();
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _collections")
            .fetch_one(&app_pool)
            .await
            .unwrap();
        assert_eq!(n, 0);

        // list returns the app
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
        assert_eq!(j.as_array().unwrap().len(), 1);
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

    /// Bootstrap to: realm 'acme', realm-admin token, app 'mobile',
    /// collection 'notes' with fields {title:text, pinned:bool,
    /// metadata:json}. Returns (state, dir, realm_token).
    async fn state_with_app_and_collection() -> (AppState, tempfile::TempDir, String) {
        let (state, dir, _, realm_admin_id) = state_with_realm_and_admin().await;
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
        let row: (String,) =
            sqlx::query_as("SELECT id FROM master_admins LIMIT 1")
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
}
