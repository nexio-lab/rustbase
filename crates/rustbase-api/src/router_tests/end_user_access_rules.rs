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
#[allow(unused_imports)]
use super::workspace_crud::*;
use crate::router::*;
use axum::http::StatusCode;
use tower::ServiceExt;

/// Bootstrap to: workspace 'acme' + open app 'mobile' + collection
/// 'notes' + one registered user 'u@acme'. Returns (state, dir,
/// master_token, user_access_token).
pub(super) async fn state_with_collection_and_user() -> (AppState, tempfile::TempDir, String, String)
{
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
            "/api/workspaces/acme/auth/users/register",
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
            "/api/workspaces/acme/auth/users/login",
            None,
            Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    let user_tok = j["access_token"].as_str().unwrap().to_string();

    // keep the seed workspace-admin token for the (admin) bootstrap caller
    let _ = tok;
    (state, dir, master_tok, user_tok)
}

#[tokio::test]
pub(super) async fn user_register_duplicate_email_returns_409() {
    let (state, _dir, _, _) = state_with_collection_and_user().await;
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/users/register",
            None,
            Some(&serde_json::json!({"email":"u@acme.com","password":"otherpass"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
pub(super) async fn user_login_wrong_password_returns_401() {
    let (state, _dir, _, _) = state_with_collection_and_user().await;
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/users/login",
            None,
            Some(&serde_json::json!({"email":"u@acme.com","password":"wrong"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
