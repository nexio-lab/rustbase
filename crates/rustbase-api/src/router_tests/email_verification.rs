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

#[tokio::test]
pub(super) async fn verify_email_request_then_confirm_marks_user_verified() {
    let (mut state, _dir, _, user_tok) = state_with_collection_and_user().await;
    let mail = install_capturing_mailer(&mut state);

    // Step 1: user asks for a verification email.
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/verify-email/request",
            Some(&user_tok),
            Some(&serde_json::json!({})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    // Step 2: pull the token, confirm it.
    let token = mail.last_token();
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/verify-email/confirm",
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
        .workspaces
        .pool_for(&rustbase_core::WorkspaceId::from("acme".to_string()))
        .await
        .unwrap();
    let user = rustbase_db::users::find_user_by_email(&pool, "u@acme.com")
        .await
        .unwrap()
        .unwrap();
    assert!(user.verified);
}

#[tokio::test]
pub(super) async fn verify_email_confirm_with_unknown_token_returns_404() {
    let (state, _dir, _, _) = state_with_collection_and_user().await;
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/verify-email/confirm",
            None,
            Some(&serde_json::json!({"token": "deadbeef".repeat(8)})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
pub(super) async fn verify_email_confirm_twice_second_call_409() {
    let (mut state, _dir, _, user_tok) = state_with_collection_and_user().await;
    let mail = install_capturing_mailer(&mut state);

    // Issue and consume.
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/auth/verify-email/request",
        Some(&user_tok),
        Some(&serde_json::json!({})),
    ))
    .await
    .unwrap();
    let token = mail.last_token();
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/auth/verify-email/confirm",
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
            "/api/workspaces/acme/auth/verify-email/confirm",
            None,
            Some(&serde_json::json!({"token": &token})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}
