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
#[allow(unused_imports)]
use super::workspace_crud::*;
use crate::router::*;
use axum::http::StatusCode;
use tower::ServiceExt;

/// Bootstrap: workspace with two pre-registered end users + a
/// workspace-admin token. Helper to keep the user-admin tests short.
pub(super) async fn state_with_two_users() -> (AppState, tempfile::TempDir, String) {
    let (state, dir, _, workspace_admin_id) = state_with_workspace_and_admin().await;
    let tok = workspace_token(&state, "acme", &workspace_admin_id);
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/auth/users/register",
        None,
        Some(&serde_json::json!({"email":"alice@acme.com","password":"alicepass1"})),
    ))
    .await
    .unwrap();
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/auth/users/register",
        None,
        Some(&serde_json::json!({"email":"bob@acme.com","password":"bobpass1"})),
    ))
    .await
    .unwrap();
    (state, dir, tok)
}

#[tokio::test]
pub(super) async fn admin_list_users_paginates_and_filters() {
    let (state, _dir, tok) = state_with_two_users().await;
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/users?per_page=10",
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    assert_eq!(j["total_items"], 2);
    assert_eq!(j["items"].as_array().unwrap().len(), 2);
    // Substring filter on email.
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/users?q=alice",
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    let j = json_body(resp).await;
    assert_eq!(j["total_items"], 1);
    assert_eq!(j["items"][0]["email"], "alice@acme.com");
}

#[tokio::test]
pub(super) async fn admin_get_user_returns_totp_status_and_oauth_links() {
    let (state, _dir, tok) = state_with_two_users().await;
    // Look up Alice's id via the list endpoint.
    let app = build_router(state.clone());
    let list = json_body(
        app.oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/users?q=alice",
            Some(&tok),
            None,
        ))
        .await
        .unwrap(),
    )
    .await;
    let alice_id = list["items"][0]["id"].as_str().unwrap().to_string();

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            &format!("/api/workspaces/acme/users/{alice_id}"),
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    assert_eq!(j["email"], "alice@acme.com");
    assert_eq!(j["verified"], false);
    // No TOTP enrolled, no OAuth links.
    assert!(j["totp"].is_null());
    assert!(j["oauth_links"].as_array().unwrap().is_empty());
}

#[tokio::test]
pub(super) async fn admin_force_verify_flips_the_flag() {
    let (state, _dir, tok) = state_with_two_users().await;
    let app = build_router(state.clone());
    let list = json_body(
        app.oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/users?q=alice",
            Some(&tok),
            None,
        ))
        .await
        .unwrap(),
    )
    .await;
    let alice_id = list["items"][0]["id"].as_str().unwrap().to_string();

    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "PATCH",
            &format!("/api/workspaces/acme/users/{alice_id}/verify"),
            Some(&tok),
            Some(&serde_json::json!({})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let app = build_router(state);
    let detail = json_body(
        app.oneshot(req_with_auth(
            "GET",
            &format!("/api/workspaces/acme/users/{alice_id}"),
            Some(&tok),
            None,
        ))
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(detail["verified"], true);
}

#[tokio::test]
pub(super) async fn admin_delete_user_cascades_and_returns_404_on_replay() {
    let (state, _dir, tok) = state_with_two_users().await;
    let app = build_router(state.clone());
    let alice_id = json_body(
        app.oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/users?q=alice",
            Some(&tok),
            None,
        ))
        .await
        .unwrap(),
    )
    .await["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "DELETE",
            &format!("/api/workspaces/acme/users/{alice_id}"),
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "DELETE",
            &format!("/api/workspaces/acme/users/{alice_id}"),
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Bob still there.
    let app = build_router(state);
    let list = json_body(
        app.oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/users",
            Some(&tok),
            None,
        ))
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(list["total_items"], 1);
}

#[tokio::test]
pub(super) async fn admin_reset_totp_clears_pending_enrolment() {
    let (state, _dir, tok) = state_with_two_users().await;
    let pool = state
        .workspaces
        .pool_for(&rustbase_core::WorkspaceId::from("acme".to_string()))
        .await
        .unwrap();
    let alice = rustbase_db::users::find_user_by_email(&pool, "alice@acme.com")
        .await
        .unwrap()
        .unwrap();
    // Plant a pending TOTP enrolment so we have something to clear.
    rustbase_db::user_totp::enroll(&pool, &alice.id, "ABCDEF234567")
        .await
        .unwrap();
    assert!(
        rustbase_db::user_totp::find(&pool, &alice.id)
            .await
            .unwrap()
            .is_some()
    );

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "DELETE",
            &format!("/api/workspaces/acme/users/{}/totp", alice.id),
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert!(
        rustbase_db::user_totp::find(&pool, &alice.id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
pub(super) async fn admin_users_require_admin_token() {
    let (state, _dir, _, _) = state_with_collection_and_user().await;
    // user_tok belongs to an end-user, not an admin.
    let user_tok = json_body(
        build_router(state.clone())
            .oneshot(req_with_auth(
                "POST",
                "/api/workspaces/acme/auth/users/login",
                None,
                Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
            ))
            .await
            .unwrap(),
    )
    .await["access_token"]
        .as_str()
        .unwrap()
        .to_string();

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/users",
            Some(&user_tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
