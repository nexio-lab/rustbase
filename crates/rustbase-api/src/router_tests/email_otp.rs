#[allow(unused_imports)]
use super::admin_users::*;
#[allow(unused_imports)]
use super::auth_flow::*;
#[allow(unused_imports)]
use super::collections_records::*;
#[allow(unused_imports)]
use super::common::*;
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
#[allow(unused_imports)]
use super::workspace_crud::*;
use crate::router::*;
use axum::http::StatusCode;
use tower::ServiceExt;

/// Bootstrap up to "workspace 'acme' exists" without registering any
/// user — OTP can sign people up so we don't want the test fixture
/// pre-creating one.
pub(super) async fn state_with_empty_realm() -> (AppState, tempfile::TempDir) {
    let (state, dir, _master_id, _admin_id) = state_with_workspace_and_admin().await;
    (state, dir)
}

#[tokio::test]
pub(super) async fn otp_request_then_login_signs_up_brand_new_user() {
    let (mut state, _dir) = state_with_empty_realm().await;
    let mail = install_capturing_mailer(&mut state);

    // 1. New email asks for a code.
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/otp/request",
            None,
            Some(&serde_json::json!({"email":"new@acme.com"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    // 2. Pull the code from the DB, redeem it.
    let code = mail.last_code();
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/otp/login",
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
        .workspaces
        .pool_for(&rustbase_core::WorkspaceId::from("acme".to_string()))
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
pub(super) async fn otp_login_with_wrong_code_returns_400_with_attempts_left() {
    let (state, _dir) = state_with_empty_realm().await;
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/auth/otp/request",
        None,
        Some(&serde_json::json!({"email":"a@acme.com"})),
    ))
    .await
    .unwrap();

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/otp/login",
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
pub(super) async fn otp_request_invalidates_prior_pending_code() {
    let (mut state, _dir) = state_with_empty_realm().await;
    let mail = install_capturing_mailer(&mut state);
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/auth/otp/request",
        None,
        Some(&serde_json::json!({"email":"a@acme.com"})),
    ))
    .await
    .unwrap();
    let first = mail.last_code();

    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/auth/otp/request",
        None,
        Some(&serde_json::json!({"email":"a@acme.com"})),
    ))
    .await
    .unwrap();
    let second = mail.last_code();

    assert_ne!(first, second, "second request must mint a fresh code");

    // Old code is dead.
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/otp/login",
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
            "/api/workspaces/acme/auth/otp/login",
            None,
            Some(&serde_json::json!({"email":"a@acme.com","code":&second})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
pub(super) async fn otp_login_unknown_email_returns_409_no_enumeration() {
    // No prior /request → no pending row, but we still don't leak
    // "this email isn't registered" — Conflict + same message
    // shape as "code expired".
    let (state, _dir) = state_with_empty_realm().await;
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/otp/login",
            None,
            Some(&serde_json::json!({"email":"ghost@acme.com","code":"123456"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
pub(super) async fn otp_login_signs_in_existing_password_user_too() {
    // A user who registered with a password can ALSO use OTP — the
    // OTP path doesn't require password_hash to be NULL.
    let (mut state, _dir, _, _) = state_with_collection_and_user().await;
    let mail = install_capturing_mailer(&mut state);
    // state_with_collection_and_user registered u@acme.com with a
    // password. Request an OTP for the SAME email:
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/auth/otp/request",
        None,
        Some(&serde_json::json!({"email":"u@acme.com"})),
    ))
    .await
    .unwrap();
    let code = mail.last_code();
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/otp/login",
            None,
            Some(&serde_json::json!({"email":"u@acme.com","code":code})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Password row should be untouched after OTP login.
    let pool = state
        .workspaces
        .pool_for(&rustbase_core::WorkspaceId::from("acme".to_string()))
        .await
        .unwrap();
    let user = rustbase_db::users::find_user_by_email(&pool, "u@acme.com")
        .await
        .unwrap()
        .unwrap();
    assert!(user.password_hash.is_some(), "OTP must not clear password");
}
