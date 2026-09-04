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
use super::user_lifecycle_hooks::*;
#[allow(unused_imports)]
use super::workspace_admin_app_crud::*;
#[allow(unused_imports)]
use super::workspace_crud::*;
use crate::router::*;
use axum::http::StatusCode;
use tower::ServiceExt;

/// Compute the current valid TOTP code for a base32 secret using
/// the same parameters the server uses. Mirrors
/// crate::auth::totp::build_totp.
pub(super) fn current_totp_code(secret_b32: &str) -> String {
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

/// With a key configured, enrolment must leave no readable secret on
/// disk — and the enrolled user must still be able to log in with it,
/// which is what proves the round trip rather than just the absence.
#[tokio::test]
pub(super) async fn an_enrolled_secret_is_encrypted_at_rest_and_still_validates() {
    let (state, _dir, _, user_tok) = state_with_collection_and_user().await;
    assert!(
        state.oauth_kek.is_some(),
        "fixture must carry a KEK for this to mean anything"
    );

    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/totp/enroll",
            Some(&user_tok),
            Some(&serde_json::json!({})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let secret_b32 = json_body(resp).await["secret_b32"]
        .as_str()
        .unwrap()
        .to_string();

    let pool = state
        .workspaces
        .pool_for(&rustbase_core::WorkspaceId::from("acme".to_string()))
        .await
        .unwrap();
    let (clear, enc): (Option<String>, Option<Vec<u8>>) =
        sqlx::query_as("SELECT secret_b32, secret_enc FROM _user_totp")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(clear, None, "the secret was stored in clear despite a KEK");
    let ct = enc.expect("no ciphertext stored");
    assert!(
        !String::from_utf8_lossy(&ct).contains(&secret_b32),
        "the ciphertext still contains the secret verbatim"
    );

    // The confirm step decrypts and checks a live code: if the round
    // trip were broken, this would fail.
    let code = current_totp_code(&secret_b32);
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/totp/confirm",
            Some(&user_tok),
            Some(&serde_json::json!({ "code": code })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "decryption round trip broke");
}

#[tokio::test]
pub(super) async fn totp_enroll_returns_secret_and_otpauth_url() {
    let (state, _dir, _, user_tok) = state_with_collection_and_user().await;
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/totp/enroll",
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
pub(super) async fn totp_confirm_with_valid_code_enables_2fa() {
    let (state, _dir, _, user_tok) = state_with_collection_and_user().await;
    // Enroll first.
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/totp/enroll",
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
            "/api/workspaces/acme/auth/totp/confirm",
            Some(&user_tok),
            Some(&serde_json::json!({"code": code})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(json_body(resp).await["status"], "enabled");

    // Row should now be enabled.
    let pool = state
        .workspaces
        .pool_for(&rustbase_core::WorkspaceId::from("acme".to_string()))
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
pub(super) async fn totp_confirm_with_wrong_code_returns_401_and_keeps_pending() {
    let (state, _dir, _, user_tok) = state_with_collection_and_user().await;
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/auth/totp/enroll",
        Some(&user_tok),
        Some(&serde_json::json!({})),
    ))
    .await
    .unwrap();

    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/totp/confirm",
            Some(&user_tok),
            Some(&serde_json::json!({"code": "000000"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let pool = state
        .workspaces
        .pool_for(&rustbase_core::WorkspaceId::from("acme".to_string()))
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
pub(super) async fn state_with_totp_enabled_user() -> (AppState, tempfile::TempDir, String) {
    let (state, dir, _, user_tok) = state_with_collection_and_user().await;

    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/totp/enroll",
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
        "/api/workspaces/acme/auth/totp/confirm",
        Some(&user_tok),
        Some(&serde_json::json!({"code": code})),
    ))
    .await
    .unwrap();
    (state, dir, secret)
}

#[tokio::test]
pub(super) async fn login_with_totp_enabled_returns_mfa_challenge_not_tokens() {
    let (state, _dir, _secret) = state_with_totp_enabled_user().await;
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
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    assert_eq!(j["mfa_required"], true);
    assert!(j["mfa_token"].as_str().unwrap().len() == 64);
    assert!(j.get("access_token").is_none(), "no tokens yet");
}

#[tokio::test]
pub(super) async fn login_totp_second_step_returns_full_tokens() {
    let (state, _dir, secret) = state_with_totp_enabled_user().await;

    // Step 1: password login → mfa challenge.
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
            "/api/workspaces/acme/auth/users/login/totp",
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
pub(super) async fn login_totp_replayed_challenge_returns_401() {
    let (state, _dir, secret) = state_with_totp_enabled_user().await;

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
            "/api/workspaces/acme/auth/users/login/totp",
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
            "/api/workspaces/acme/auth/users/login/totp",
            None,
            Some(&serde_json::json!({"mfa_token": &mfa_token, "code": &code})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
pub(super) async fn totp_disable_with_valid_code_clears_the_row() {
    let (state, _dir, secret) = state_with_totp_enabled_user().await;
    // Need a fresh user token (TOTP-enabled users can't login with
    // password alone any more).
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
    let mfa_token = json_body(resp).await["mfa_token"]
        .as_str()
        .unwrap()
        .to_string();
    let code1 = current_totp_code(&secret);
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/users/login/totp",
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
            "/api/workspaces/acme/auth/totp/disable",
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
            "/api/workspaces/acme/auth/users/login",
            None,
            Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
        ))
        .await
        .unwrap();
    let j = json_body(resp).await;
    assert!(j["access_token"].as_str().unwrap().starts_with("ey"));
    assert!(j.get("mfa_required").is_none());
}
