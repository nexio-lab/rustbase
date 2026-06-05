#[allow(unused_imports)]
use super::admin_users::*;
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
#[allow(unused_imports)]
use super::workspace_crud::*;
use crate::router::*;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;

pub(super) async fn initialized_state_with_admin(
    password: &str,
) -> (AppState, tempfile::TempDir, String) {
    let (state, dir) = fresh_state().await;
    // `fresh_state` already seeded the canonical `admin` row.
    // Set its password directly so tests log in as the same
    // principal production uses.
    let admin = rustbase_db::admins::find_master_admin_by_username(state.system.pool(), "admin")
        .await
        .unwrap()
        .expect("seed admin missing");
    let hash = rustbase_auth::hash_password(password).unwrap();
    rustbase_db::admins::set_master_admin_password(state.system.pool(), &admin.id, &hash)
        .await
        .unwrap();
    state.mark_initialized();
    (state, dir, admin.id)
}

pub(super) async fn post_json(
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
pub(super) async fn login_with_valid_credentials_returns_tokens() {
    let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
    let app = build_router(state);
    let body = serde_json::json!({"username":"admin","password":"hunter22"});
    let resp = post_json(app, "/_/auth/admin/login", &body).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    assert!(j["access_token"].as_str().unwrap().starts_with("ey"));
    assert!(j["refresh_token"].as_str().unwrap().starts_with("rfsh_"));
    assert_eq!(j["admin"]["id"], admin_id);
    assert_eq!(j["admin"]["username"], "admin");
    assert!(j["admin"].get("password_hash").is_none());
}

#[tokio::test]
pub(super) async fn login_with_wrong_password_returns_401() {
    let (state, _dir, _) = initialized_state_with_admin("hunter22").await;
    let app = build_router(state);
    let body = serde_json::json!({"username":"admin","password":"wrong"});
    let resp = post_json(app, "/_/auth/admin/login", &body).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
pub(super) async fn login_with_unknown_username_returns_401() {
    let (state, _dir, _) = initialized_state_with_admin("hunter22").await;
    let app = build_router(state);
    let body = serde_json::json!({"username":"nobody","password":"hunter22"});
    let resp = post_json(app, "/_/auth/admin/login", &body).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
pub(super) async fn refresh_rotates_token() {
    let (state, _dir, _) = initialized_state_with_admin("hunter22").await;

    // log in to get a refresh token
    let app = build_router(state.clone());
    let resp = post_json(
        app,
        "/_/auth/admin/login",
        &serde_json::json!({"username":"admin","password":"hunter22"}),
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
pub(super) async fn refresh_with_unknown_token_returns_401() {
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

#[tokio::test]
pub(super) async fn issued_access_token_is_rs256_with_kid() {
    let (state, _dir, _) = initialized_state_with_admin("hunter22").await;
    let app = build_router(state.clone());
    let resp = post_json(
        app,
        "/_/auth/admin/login",
        &serde_json::json!({"username":"admin","password":"hunter22"}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    let token = j["access_token"].as_str().unwrap();
    let header = jsonwebtoken::decode_header(token).unwrap();
    assert_eq!(header.alg, jsonwebtoken::Algorithm::RS256);
    assert_eq!(header.kid.as_deref(), Some(state.jwt.active.kid.as_str()));
}

#[tokio::test]
pub(super) async fn jwks_returns_active_key_unauthenticated_pre_setup() {
    // No setup yet — JWKS must still be reachable.
    let (state, _dir) = fresh_state().await;
    let kid = state.jwt.active.kid.clone();
    let app = build_router(state);
    let req = Request::builder()
        .uri("/.well-known/jwks.json")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap(),
        "application/jwk-set+json"
    );
    let j = json_body(resp).await;
    let keys = j["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0]["kty"], "RSA");
    assert_eq!(keys[0]["alg"], "RS256");
    assert_eq!(keys[0]["use"], "sig");
    assert_eq!(keys[0]["kid"], kid);
    assert!(keys[0]["n"].as_str().unwrap().len() > 100);
    assert_eq!(keys[0]["e"], "AQAB");
}

#[tokio::test]
pub(super) async fn jwks_under_underscore_mount_returns_same_keyset() {
    let (state, _dir) = fresh_state().await;
    let app = build_router(state);
    let req = Request::builder()
        .uri("/_/auth/jwks.json")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
pub(super) async fn legacy_hs256_token_validates_against_jwt_issuer() {
    // An HS256 token issued before the RS256 swap (we emulate by
    // hand-signing with the legacy `master_key`) must still verify
    // on the principal extractor.
    let (state, _dir, _) = initialized_state_with_admin("hunter22").await;
    let claims = rustbase_auth::build_claims(
        "u1",
        rustbase_auth::TokenRole::MasterAdmin,
        None,
        None,
        chrono::Duration::minutes(15),
    );
    let legacy = rustbase_auth::encode_token(&claims, &state.master_key).unwrap();
    // Use the JwtIssuer directly to assert legacy HS256 validates.
    let decoded = state.jwt.verify(&legacy).unwrap();
    assert_eq!(decoded.sub, "u1");
}

#[tokio::test]
pub(super) async fn login_sets_httponly_session_cookies() {
    let (state, _dir, _) = initialized_state_with_admin("hunter22").await;
    let app = build_router(state);
    let resp = post_json(
        app,
        "/_/auth/admin/login",
        &serde_json::json!({"username":"admin","password":"hunter22"}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let cookies: Vec<_> = resp
        .headers()
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect();
    assert_eq!(
        cookies.len(),
        2,
        "expected access + refresh cookies, got {cookies:?}"
    );
    let access = cookies.iter().find(|c| c.starts_with("rb_at=")).unwrap();
    let refresh = cookies.iter().find(|c| c.starts_with("rb_rt=")).unwrap();
    for c in [access, refresh] {
        assert!(c.contains("HttpOnly"), "missing HttpOnly: {c}");
        assert!(
            c.contains("SameSite=Strict"),
            "missing SameSite=Strict: {c}"
        );
        // fresh_state uses cookie_secure = false (plain HTTP tests).
        assert!(!c.contains("Secure"), "should not be Secure in tests: {c}");
    }
    assert!(
        refresh.contains("Path=/_/auth"),
        "refresh cookie wrong Path: {refresh}"
    );
    assert!(
        access.contains("Path=/"),
        "access cookie wrong Path: {access}"
    );
}

#[tokio::test]
pub(super) async fn admin_auth_accepts_rb_at_cookie() {
    let (state, _dir, _) = initialized_state_with_admin("hunter22").await;
    let app = build_router(state);
    let login = post_json(
        app.clone(),
        "/_/auth/admin/login",
        &serde_json::json!({"username":"admin","password":"hunter22"}),
    )
    .await;
    let access_cookie = login
        .headers()
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .find(|c| c.starts_with("rb_at="))
        .unwrap();
    let just_pair = access_cookie.split(';').next().unwrap().to_string();

    // Issue a protected request using only the cookie — no Bearer.
    let req = Request::builder()
        .uri("/api/workspaces")
        .method("GET")
        .header(axum::http::header::COOKIE, just_pair)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
pub(super) async fn refresh_via_cookie_rotates_tokens_and_returns_new_cookies() {
    let (state, _dir, _) = initialized_state_with_admin("hunter22").await;
    let app = build_router(state);
    let login = post_json(
        app.clone(),
        "/_/auth/admin/login",
        &serde_json::json!({"username":"admin","password":"hunter22"}),
    )
    .await;
    let cookies: Vec<String> = login
        .headers()
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect();
    let refresh_pair = cookies
        .iter()
        .find(|c| c.starts_with("rb_rt="))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    // Empty body — server must read the cookie.
    let req = Request::builder()
        .uri("/_/auth/refresh")
        .method("POST")
        .header(axum::http::header::COOKIE, refresh_pair.clone())
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let cookies_after: Vec<_> = resp
        .headers()
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect();
    assert_eq!(cookies_after.len(), 2);
    assert!(cookies_after.iter().any(|c| c.starts_with("rb_at=")));
    assert!(cookies_after.iter().any(|c| c.starts_with("rb_rt=")));
    // Rotated — the new refresh `name=value` pair must differ
    // from the one we just used to redeem.
    let new_refresh_pair = cookies_after
        .iter()
        .find(|c| c.starts_with("rb_rt="))
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    assert_ne!(new_refresh_pair, refresh_pair);
}

#[tokio::test]
pub(super) async fn logout_clears_session_cookies_and_revokes_refresh() {
    let (state, _dir, _) = initialized_state_with_admin("hunter22").await;
    let app = build_router(state);
    let login = post_json(
        app.clone(),
        "/_/auth/admin/login",
        &serde_json::json!({"username":"admin","password":"hunter22"}),
    )
    .await;
    let refresh_pair = login
        .headers()
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .find(|c| c.starts_with("rb_rt="))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    // Logout with the refresh cookie attached.
    let req = Request::builder()
        .uri("/_/auth/logout")
        .method("POST")
        .header(axum::http::header::COOKIE, refresh_pair.clone())
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let cleared: Vec<String> = resp
        .headers()
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect();
    assert!(
        cleared
            .iter()
            .any(|c| c.contains("rb_at=") && c.contains("Max-Age=0"))
    );
    assert!(
        cleared
            .iter()
            .any(|c| c.contains("rb_rt=") && c.contains("Max-Age=0"))
    );

    // The refresh token must no longer be usable.
    let req = Request::builder()
        .uri("/_/auth/refresh")
        .method("POST")
        .header(axum::http::header::COOKIE, refresh_pair)
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
pub(super) async fn repeated_bad_password_eventually_locks_with_429() {
    let (mut state, _dir, _) = initialized_state_with_admin("hunter22").await;
    // Tight policy for a fast test: 3 failures, 60-second window,
    // 60-second lockout.
    state.lockout_policy = crate::security::LockoutPolicy::from_secs(true, 3, 60, 60);

    let app = build_router(state.clone());
    let body = serde_json::json!({"username":"admin","password":"wrong"});
    for _ in 0..2 {
        let resp = post_json(app.clone(), "/_/auth/admin/login", &body).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
    // Third miss trips the lock — response is 429 with Retry-After.
    let resp = post_json(app.clone(), "/_/auth/admin/login", &body).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry = resp
        .headers()
        .get(axum::http::header::RETRY_AFTER)
        .expect("Retry-After header missing");
    assert!(retry.to_str().unwrap().parse::<u64>().unwrap() > 0);

    // Even the correct password now bounces with 429 while locked.
    let app2 = build_router(state.clone());
    let correct = serde_json::json!({"username":"admin","password":"hunter22"});
    let resp = post_json(app2, "/_/auth/admin/login", &correct).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

    // The audit log has at least one `login_locked` row for this subject.
    let rows = rustbase_db::audit::list_recent(state.system.pool(), 50)
        .await
        .unwrap();
    assert!(
        rows.iter()
            .any(|e| e.action == "login_locked" && e.actor.as_deref() == Some("master:admin"))
    );
    assert!(
        rows.iter()
            .any(|e| e.action == "login_failed" && e.actor.as_deref() == Some("master:admin"))
    );
}

#[tokio::test]
pub(super) async fn good_password_after_failures_clears_lockout_state() {
    let (mut state, _dir, _) = initialized_state_with_admin("hunter22").await;
    state.lockout_policy = crate::security::LockoutPolicy::from_secs(true, 3, 60, 60);
    let app = build_router(state.clone());

    // Two failures, then a success.
    for _ in 0..2 {
        let resp = post_json(
            app.clone(),
            "/_/auth/admin/login",
            &serde_json::json!({"username":"admin","password":"wrong"}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
    let resp = post_json(
        app.clone(),
        "/_/auth/admin/login",
        &serde_json::json!({"username":"admin","password":"hunter22"}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    // After the success, two more failures should NOT trip the
    // lockout — counters reset.
    for _ in 0..2 {
        let resp = post_json(
            app.clone(),
            "/_/auth/admin/login",
            &serde_json::json!({"username":"admin","password":"wrong"}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
pub(super) async fn login_success_emits_audit_row() {
    let (state, _dir, _) = initialized_state_with_admin("hunter22").await;
    let app = build_router(state.clone());
    let resp = post_json(
        app,
        "/_/auth/admin/login",
        &serde_json::json!({"username":"admin","password":"hunter22"}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let rows = rustbase_db::audit::list_recent(state.system.pool(), 5)
        .await
        .unwrap();
    assert!(
        rows.iter()
            .any(|e| e.action == "login_success" && e.actor.as_deref() == Some("master:admin"))
    );
}
