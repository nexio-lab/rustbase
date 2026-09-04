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

pub(super) fn provider_body() -> serde_json::Value {
    serde_json::json!({
        "client_id": "google-client-1",
        "client_secret": "shh-very-secret",
        "config": {
            "auth_url":     "https://accounts.google.com/o/oauth2/v2/auth",
            "token_url":    "https://oauth2.googleapis.com/token",
            "userinfo_url": "https://openidconnect.googleapis.com/v1/userinfo",
            "scopes":       ["openid", "email"],
            "userinfo_id_field":    "/sub",
            "userinfo_email_field": "/email",
        },
    })
}

/// Without a key-encryption key there is nowhere safe to put a
/// client secret. Generating one into the data directory is the very
/// defect `RUSTBASE_KEK` exists to remove, so the write is refused
/// and the operator is told which variable to set.
#[tokio::test]
pub(super) async fn storing_a_client_secret_without_a_kek_is_refused() {
    let (mut state, _dir, _, workspace_admin_id) = state_with_workspace_and_admin().await;
    state.oauth_kek = std::sync::Arc::new(None);
    let tok = workspace_token(&state, "acme", &workspace_admin_id);

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "PUT",
            "/api/workspaces/acme/auth/oauth/providers/google",
            Some(&tok),
            Some(&provider_body()),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = json_body(resp).await.to_string();
    assert!(
        body.contains("RUSTBASE_KEK"),
        "the operator is not told what to set: {body}"
    );
}

#[tokio::test]
pub(super) async fn oauth_admin_put_then_get_returns_provider_without_secret() {
    let (state, _dir, master_id, _realm_admin_id) = state_with_workspace_and_admin().await;
    let master_tok = master_token(&state, &master_id);

    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "PUT",
            "/api/workspaces/acme/auth/oauth/providers/google",
            Some(&master_tok),
            Some(&provider_body()),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    assert_eq!(j["provider"], "google");
    assert_eq!(j["client_id"], "google-client-1");
    assert!(
        j.get("client_secret").is_none(),
        "PUT must not echo the secret"
    );

    // GET reads it back; still no secret in the response.
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/auth/oauth/providers/google",
            Some(&master_tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    assert_eq!(j["client_id"], "google-client-1");
    assert!(j.get("client_secret").is_none());
    assert_eq!(
        j["config"]["token_url"],
        "https://oauth2.googleapis.com/token"
    );
}

#[tokio::test]
pub(super) async fn oauth_admin_stored_secret_is_encrypted_at_rest() {
    let (state, _dir, master_id, _) = state_with_workspace_and_admin().await;
    let master_tok = master_token(&state, &master_id);
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "PUT",
        "/api/workspaces/acme/auth/oauth/providers/google",
        Some(&master_tok),
        Some(&provider_body()),
    ))
    .await
    .unwrap();

    // Read the raw row: client_secret_enc must NOT contain the
    // plaintext anywhere. AES-GCM ciphertext is opaque bytes.
    let pool = state
        .workspaces
        .pool_for(&rustbase_core::WorkspaceId::from("acme".to_string()))
        .await
        .unwrap();
    let (ct,): (Vec<u8>,) =
        sqlx::query_as("SELECT client_secret_enc FROM oauth_providers WHERE provider = ?")
            .bind("google")
            .fetch_one(&pool)
            .await
            .unwrap();
    let as_str = String::from_utf8_lossy(&ct);
    assert!(
        !as_str.contains("shh-very-secret"),
        "raw row leaks plaintext: {as_str:?}"
    );
    // Sanity: KEK-aware decrypt round-trips.
    let pt = rustbase_auth::decrypt(&ct, state.oauth_kek.as_ref().as_ref().unwrap()).unwrap();
    assert_eq!(pt, b"shh-very-secret");
}

#[tokio::test]
pub(super) async fn oauth_admin_list_returns_summaries_in_provider_order() {
    let (state, _dir, master_id, _) = state_with_workspace_and_admin().await;
    let master_tok = master_token(&state, &master_id);

    for name in ["google", "github"] {
        let mut body = provider_body();
        body["client_id"] = serde_json::Value::String(format!("{name}-id"));
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "PUT",
            &format!("/api/workspaces/acme/auth/oauth/providers/{name}"),
            Some(&master_tok),
            Some(&body),
        ))
        .await
        .unwrap();
    }

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/auth/oauth/providers",
            Some(&master_tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    let arr = j.as_array().unwrap();
    let names: Vec<_> = arr
        .iter()
        .map(|p| p["provider"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["github", "google"]);
    for entry in arr {
        assert!(entry.get("client_secret").is_none());
    }
}

#[tokio::test]
pub(super) async fn oauth_admin_delete_then_get_returns_404() {
    let (state, _dir, master_id, _) = state_with_workspace_and_admin().await;
    let master_tok = master_token(&state, &master_id);
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "PUT",
        "/api/workspaces/acme/auth/oauth/providers/google",
        Some(&master_tok),
        Some(&provider_body()),
    ))
    .await
    .unwrap();

    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "DELETE",
            "/api/workspaces/acme/auth/oauth/providers/google",
            Some(&master_tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Double-delete is 404 (no row).
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "DELETE",
            "/api/workspaces/acme/auth/oauth/providers/google",
            Some(&master_tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // GET also misses.
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/auth/oauth/providers/google",
            Some(&master_tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
pub(super) async fn oauth_admin_realm_admin_can_manage_own_realm() {
    let (state, _dir, _, workspace_admin_id) = state_with_workspace_and_admin().await;
    let workspace_tok = workspace_token(&state, "acme", &workspace_admin_id);

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "PUT",
            "/api/workspaces/acme/auth/oauth/providers/google",
            Some(&workspace_tok),
            Some(&provider_body()),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
pub(super) async fn oauth_admin_requires_admin_token() {
    let (state, _dir, _, _) = state_with_collection_and_user().await;
    // user_tok is on the same workspace but is not an admin.
    let app = build_router(state.clone());
    let user_resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/users/login",
            None,
            Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
        ))
        .await
        .unwrap();
    let user_tok = json_body(user_resp).await["access_token"]
        .as_str()
        .unwrap()
        .to_string();

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "PUT",
            "/api/workspaces/acme/auth/oauth/providers/google",
            Some(&user_tok),
            Some(&provider_body()),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
pub(super) async fn oauth_admin_end_to_end_callback_works_after_put() {
    // PUT a provider via the admin endpoint, then drive the full
    // OAuth callback against the stub — proves encrypt → store →
    // find → decrypt → token exchange round-trips.
    let (base_url, _h) = fake_oauth_provider(serde_json::json!({
        "sub": "google-sub-77",
        "email": "via-admin@acme.test",
    }))
    .await;
    let (state, _dir, master_id, _) = state_with_workspace_and_admin().await;
    let master_tok = master_token(&state, &master_id);

    let mut body = provider_body();
    body["client_secret"] = serde_json::Value::String("admin-put-secret".into());
    body["config"]["auth_url"] = serde_json::Value::String(format!("{base_url}/authorize"));
    body["config"]["token_url"] = serde_json::Value::String(format!("{base_url}/token"));
    body["config"]["userinfo_url"] = serde_json::Value::String(format!("{base_url}/userinfo"));
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "PUT",
        "/api/workspaces/acme/auth/oauth/providers/google",
        Some(&master_tok),
        Some(&body),
    ))
    .await
    .unwrap();

    // /authorize → state
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/auth/oauth/google/authorize?redirect_uri=https%3A%2F%2Fapp%2Fcb",
            None,
            None,
        ))
        .await
        .unwrap();
    let nonce = json_body(resp).await["state"].as_str().unwrap().to_string();

    // /callback
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/oauth/google/callback",
            None,
            Some(&serde_json::json!({"code":"unused","state":nonce})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    assert!(j["access_token"].as_str().unwrap().starts_with("ey"));
    assert_eq!(j["user"]["email"], "via-admin@acme.test");
}

#[tokio::test]
pub(super) async fn oauth_admin_put_without_secret_preserves_existing_ciphertext() {
    // Create with a real secret, then PUT again with only client_id
    // and config — the stored ciphertext should still decrypt to
    // the original secret. This is what the edit form relies on.
    let (state, _dir, master_id, _) = state_with_workspace_and_admin().await;
    let master_tok = master_token(&state, &master_id);
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "PUT",
        "/api/workspaces/acme/auth/oauth/providers/google",
        Some(&master_tok),
        Some(&provider_body()),
    ))
    .await
    .unwrap();

    // Edit without sending client_secret.
    let mut body = provider_body();
    body.as_object_mut().unwrap().remove("client_secret");
    body["client_id"] = serde_json::Value::String("rotated-id".into());
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "PUT",
            "/api/workspaces/acme/auth/oauth/providers/google",
            Some(&master_tok),
            Some(&body),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Pull the raw ciphertext and decrypt with the server's KEK —
    // it should still match the ORIGINAL secret.
    let pool = state
        .workspaces
        .pool_for(&rustbase_core::WorkspaceId::from("acme".to_string()))
        .await
        .unwrap();
    let (ct,): (Vec<u8>,) =
        sqlx::query_as("SELECT client_secret_enc FROM oauth_providers WHERE provider = ?")
            .bind("google")
            .fetch_one(&pool)
            .await
            .unwrap();
    let pt = rustbase_auth::decrypt(&ct, state.oauth_kek.as_ref().as_ref().unwrap()).unwrap();
    assert_eq!(pt, b"shh-very-secret");
    // And the new client_id stuck.
    let app = build_router(state);
    let detail = json_body(
        app.oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/auth/oauth/providers/google",
            Some(&master_tok),
            None,
        ))
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(detail["client_id"], "rotated-id");
}

#[tokio::test]
pub(super) async fn oauth_admin_put_without_secret_on_create_returns_400() {
    let (state, _dir, master_id, _) = state_with_workspace_and_admin().await;
    let master_tok = master_token(&state, &master_id);
    let mut body = provider_body();
    body.as_object_mut().unwrap().remove("client_secret");
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "PUT",
            "/api/workspaces/acme/auth/oauth/providers/google",
            Some(&master_tok),
            Some(&body),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
