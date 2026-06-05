#[allow(unused_imports)]
use super::admin_users::*;
#[allow(unused_imports)]
use super::auth_flow::*;
#[allow(unused_imports)]
use super::collections_records::*;
#[allow(unused_imports)]
use super::common::*;
#[allow(unused_imports)]
use super::email_otp::*;
#[allow(unused_imports)]
use super::email_verification::*;
#[allow(unused_imports)]
use super::end_user_access_rules::*;
#[allow(unused_imports)]
use super::hooks_crud::*;
#[allow(unused_imports)]
use super::oauth_admin::*;
#[allow(unused_imports)]
use super::oauth_sign_in::*;
#[allow(unused_imports)]
use super::password_reset::*;
#[allow(unused_imports)]
use super::totp::*;
#[allow(unused_imports)]
use super::user_lifecycle_hooks::*;
#[allow(unused_imports)]
use super::workspace_admin_app_crud::*;
use crate::router::*;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;

pub(super) fn master_token(state: &AppState, admin_id: &str) -> String {
    let claims = rustbase_auth::build_claims(
        admin_id,
        rustbase_auth::TokenRole::MasterAdmin,
        None,
        None,
        chrono::Duration::minutes(15),
    );
    state.jwt.issue(&claims).unwrap()
}

pub(super) fn req_with_auth(
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
pub(super) async fn list_realms_without_auth_returns_401() {
    let (state, _dir, _) = initialized_state_with_admin("hunter22").await;
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth("GET", "/api/workspaces", None, None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
pub(super) async fn list_realms_with_master_token_returns_master_realm() {
    let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
    let token = master_token(&state, &admin_id);
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth("GET", "/api/workspaces", Some(&token), None))
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
pub(super) async fn create_realm_initializes_realm_db_and_lists_two() {
    let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
    let token = master_token(&state, &admin_id);
    let body = serde_json::json!({"id":"acme","name":"Acme Inc."});
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces",
            Some(&token),
            Some(&body),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let j = json_body(resp).await;
    assert_eq!(j["id"], "acme");
    assert_eq!(j["is_master"], false);

    // The workspace.db should now exist and respond to a query against a
    // table from the workspace schema. End-users no longer live in
    // workspace.db — check the `apps` table instead.
    let workspace_pool = state
        .workspaces
        .pool_for(&rustbase_core::WorkspaceId::from("acme"))
        .await
        .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM apps")
        .fetch_one(&workspace_pool)
        .await
        .unwrap();
    assert_eq!(n, 0);

    // listing now returns both
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth("GET", "/api/workspaces", Some(&token), None))
        .await
        .unwrap();
    let j = json_body(resp).await;
    assert_eq!(j.as_array().unwrap().len(), 2);
}

#[tokio::test]
pub(super) async fn create_realm_rejects_reserved_master_id() {
    let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
    let token = master_token(&state, &admin_id);
    let body = serde_json::json!({"id":"master","name":"impersonator"});
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces",
            Some(&token),
            Some(&body),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
pub(super) async fn create_realm_rejects_uppercase_in_id() {
    let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
    let token = master_token(&state, &admin_id);
    let body = serde_json::json!({"id":"Acme","name":"x"});
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces",
            Some(&token),
            Some(&body),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
pub(super) async fn create_realm_twice_returns_409() {
    let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
    let token = master_token(&state, &admin_id);
    let body = serde_json::json!({"id":"acme","name":"Acme"});

    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces",
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
            "/api/workspaces",
            Some(&token),
            Some(&body),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
pub(super) async fn get_unknown_realm_returns_404() {
    let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
    let token = master_token(&state, &admin_id);
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/nope",
            Some(&token),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
pub(super) async fn rename_realm_updates_name() {
    let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
    let token = master_token(&state, &admin_id);

    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces",
        Some(&token),
        Some(&serde_json::json!({"id":"acme","name":"Acme"})),
    ))
    .await
    .unwrap();

    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "PATCH",
            "/api/workspaces/acme",
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
pub(super) async fn delete_master_realm_is_forbidden() {
    let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
    let token = master_token(&state, &admin_id);
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "DELETE",
            "/api/workspaces/master",
            Some(&token),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
pub(super) async fn delete_realm_removes_row_and_folder() {
    let (state, dir, admin_id) = initialized_state_with_admin("hunter22").await;
    let token = master_token(&state, &admin_id);

    // create
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces",
        Some(&token),
        Some(&serde_json::json!({"id":"acme","name":"Acme"})),
    ))
    .await
    .unwrap();
    let workspace_folder = dir.path().join("workspaces/acme");
    assert!(workspace_folder.exists());

    // delete
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "DELETE",
            "/api/workspaces/acme",
            Some(&token),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // row gone and folder removed
    let still = rustbase_db::workspaces::find_workspace(state.system.pool(), "acme")
        .await
        .unwrap();
    assert!(still.is_none());
    assert!(!workspace_folder.exists());
}

#[tokio::test]
pub(super) async fn token_with_user_role_is_forbidden_on_master_endpoint() {
    let (state, _dir, _admin_id) = initialized_state_with_admin("hunter22").await;
    // Issue a token with the wrong role; signed with the correct key.
    let claims = rustbase_auth::build_claims(
        "u1",
        rustbase_auth::TokenRole::User,
        Some("acme".into()),
        None,
        chrono::Duration::minutes(15),
    );
    let token = state.jwt.issue(&claims).unwrap();
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth("GET", "/api/workspaces", Some(&token), None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
