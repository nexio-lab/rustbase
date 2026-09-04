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

#[tokio::test]
pub(super) async fn password_reset_request_then_confirm_changes_password() {
    let (mut state, _dir, _, _) = state_with_collection_and_user().await;
    let mail = install_capturing_mailer(&mut state);
    // Original password from state_with_collection_and_user is "userpass1".

    // 1. Request reset.
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/password-reset/request",
            None,
            Some(&serde_json::json!({"email":"u@acme.com"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    // 2. Pull token + confirm with new password.
    let token = mail.last_token();
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/password-reset/confirm",
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
            "/api/workspaces/acme/auth/users/login",
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
            "/api/workspaces/acme/auth/users/login",
            None,
            Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
pub(super) async fn password_reset_request_for_unknown_email_still_returns_202() {
    // Enumeration-resistance: same response regardless of whether
    // the address belongs to a user. The DB should be untouched.
    let (state, _dir, _, _) = state_with_collection_and_user().await;
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/password-reset/request",
            None,
            Some(&serde_json::json!({"email":"ghost@nowhere.com"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    let pool = state
        .workspaces
        .pool_for(&rustbase_core::WorkspaceId::from("acme".to_string()))
        .await
        .unwrap();
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _password_resets")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count.0, 0);
}

#[tokio::test]
pub(super) async fn password_reset_confirm_invalidates_siblings() {
    // Issue two tokens for the same user; consuming one must
    // make the other return 409 instead of 200.
    let (mut state, _dir, _, _) = state_with_collection_and_user().await;
    let mail = install_capturing_mailer(&mut state);
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/auth/password-reset/request",
        None,
        Some(&serde_json::json!({"email":"u@acme.com"})),
    ))
    .await
    .unwrap();
    let first = mail.last_token();

    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/auth/password-reset/request",
        None,
        Some(&serde_json::json!({"email":"u@acme.com"})),
    ))
    .await
    .unwrap();
    let second = mail.last_token();
    assert_ne!(first, second);

    // Consume the second; the first must then be dead.
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/password-reset/confirm",
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
            "/api/workspaces/acme/auth/password-reset/confirm",
            None,
            Some(&serde_json::json!({"token": &first, "new_password": "another!7"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
pub(super) async fn password_reset_confirm_rejects_weak_password() {
    let (mut state, _dir, _, _) = state_with_collection_and_user().await;
    let mail = install_capturing_mailer(&mut state);
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/auth/password-reset/request",
        None,
        Some(&serde_json::json!({"email":"u@acme.com"})),
    ))
    .await
    .unwrap();
    let token = mail.last_token();
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/password-reset/confirm",
            None,
            Some(&serde_json::json!({"token": token, "new_password": "short"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
