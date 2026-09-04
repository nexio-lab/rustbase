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

/// Spin up a localhost axum server that pretends to be an
/// OAuth2 provider's `/token` and `/userinfo` endpoints. Returns
/// the bound `http://127.0.0.1:PORT` URL prefix and a shutdown
/// handle. The body the stub returns is parameterised so a single
/// test can drive multiple provider responses.
pub(super) async fn fake_oauth_provider(
    userinfo_body: serde_json::Value,
) -> (String, tokio::task::JoinHandle<()>) {
    fake_oauth_provider_with_capture(userinfo_body).await.0
}

/// Variant that also returns a shared cell capturing the last
/// `/token` form body so callers can assert PKCE parameters made
/// it to the provider.
pub(super) async fn fake_oauth_provider_with_capture(
    userinfo_body: serde_json::Value,
) -> (
    (String, tokio::task::JoinHandle<()>),
    std::sync::Arc<parking_lot::Mutex<Option<String>>>,
) {
    use axum::{Json, Router, extract::State as AxumState, routing::post};

    let captured: std::sync::Arc<parking_lot::Mutex<Option<String>>> =
        std::sync::Arc::new(parking_lot::Mutex::new(None));
    let captured_for_handler = captured.clone();

    async fn token(
        AxumState(captured): AxumState<std::sync::Arc<parking_lot::Mutex<Option<String>>>>,
        body: String,
    ) -> Json<serde_json::Value> {
        *captured.lock() = Some(body);
        Json(serde_json::json!({
            "access_token": "fake-access-token",
            "token_type": "Bearer",
            "expires_in": 3600,
        }))
    }
    let body_for_handler = userinfo_body;
    let userinfo_handler = move || {
        let body = body_for_handler.clone();
        async move { Json(body) }
    };
    let app: Router = Router::new()
        .route("/token", post(token))
        .with_state(captured_for_handler)
        .route("/userinfo", axum::routing::get(userinfo_handler));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    ((format!("http://{addr}"), handle), captured)
}

/// Seed a provider config in the per-app DB pointing at the stub.
/// The client_secret is encrypted under the AppState's KEK so the
/// callback path can decrypt it on use, matching production wiring.
pub(super) async fn seed_provider(
    state: &AppState,
    workspace: &str,
    _app: &str,
    provider: &str,
    base_url: &str,
) {
    // OAuth providers moved to workspace scope with shared identity.
    let pool = state
        .workspaces
        .pool_for(&rustbase_core::WorkspaceId::from(workspace.to_string()))
        .await
        .unwrap();
    let secret_enc =
        rustbase_auth::encrypt(b"test-secret", state.oauth_kek.as_ref().as_ref().unwrap()).unwrap();
    rustbase_db::oauth_providers::upsert_provider(
        &pool,
        &rustbase_db::oauth_providers::OAuthProvider {
            provider: provider.into(),
            client_id: "test-client".into(),
            secret_enc,
            config: rustbase_db::oauth_providers::OAuthProviderConfig {
                auth_url: format!("{base_url}/authorize"),
                token_url: format!("{base_url}/token"),
                userinfo_url: format!("{base_url}/userinfo"),
                scopes: vec!["openid".into(), "email".into()],
                userinfo_id_field: "/sub".into(),
                userinfo_email_field: "/email".into(),
            },
        },
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
pub(super) async fn oauth_authorize_returns_url_with_state_and_scopes() {
    let (base_url, _h) = fake_oauth_provider(serde_json::json!({})).await;
    let (state, _dir, _, _) = state_with_workspace_and_admin().await;
    seed_provider(&state, "acme", "mobile", "google", &base_url).await;

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/auth/oauth/google/authorize?redirect_uri=https%3A%2F%2Fapp%2Fcb",
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    let url = j["authorize_url"].as_str().unwrap();
    // The stub's authorize URL gets baked in; we don't follow it,
    // but every required query param has to be present.
    assert!(url.contains("client_id=test-client"), "got: {url}");
    assert!(url.contains("response_type=code"), "got: {url}");
    assert!(url.contains("state="), "got: {url}");
    assert!(url.contains("scope=openid+email"), "got: {url}");
    // PKCE (RFC 7636) — every authorize URL carries S256 challenge.
    assert!(url.contains("code_challenge="), "got: {url}");
    assert!(url.contains("code_challenge_method=S256"), "got: {url}");
    assert!(j["state"].as_str().unwrap().len() == 64);
}

#[tokio::test(flavor = "multi_thread")]
pub(super) async fn oauth_authorize_unknown_provider_returns_404() {
    let (state, _dir, _, _) = state_with_workspace_and_admin().await;
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/auth/oauth/ghost/authorize?redirect_uri=https%3A%2F%2Fapp%2Fcb",
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
pub(super) async fn oauth_callback_sends_pkce_verifier_to_token_endpoint() {
    let ((base_url, _h), captured) = fake_oauth_provider_with_capture(serde_json::json!({
        "sub": "google-sub-pkce",
        "email": "pkce@google.test",
    }))
    .await;
    let (state, _dir, _, _) = state_with_workspace_and_admin().await;
    seed_provider(&state, "acme", "mobile", "google", &base_url).await;

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
    let j = json_body(resp).await;
    let nonce = j["state"].as_str().unwrap().to_string();
    let auth_url = j["authorize_url"].as_str().unwrap().to_string();
    // The authorize URL carries the S256 challenge.
    assert!(auth_url.contains("code_challenge="), "got: {auth_url}");
    assert!(auth_url.contains("code_challenge_method=S256"));

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/oauth/google/callback",
            None,
            Some(&serde_json::json!({"code":"unused", "state": nonce})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // The token endpoint received the verifier in its form body.
    let body = captured
        .lock()
        .clone()
        .expect("token endpoint never reached");
    assert!(
        body.contains("code_verifier="),
        "token POST missing code_verifier: {body}"
    );
    assert!(body.contains("grant_type=authorization_code"));
    assert!(body.contains("client_id=test-client"));
}

#[tokio::test(flavor = "multi_thread")]
pub(super) async fn oauth_callback_round_trips_signup_via_stubbed_provider() {
    let (base_url, _h) = fake_oauth_provider(serde_json::json!({
        "sub": "google-sub-42",
        "email": "ada@google.test",
    }))
    .await;
    let (state, _dir, _, _) = state_with_workspace_and_admin().await;
    seed_provider(&state, "acme", "mobile", "google", &base_url).await;

    // 1. /authorize → get a real state nonce.
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

    // 2. /callback — the stub returns access_token + userinfo.
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/oauth/google/callback",
            None,
            Some(&serde_json::json!({"code":"unused", "state": nonce})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    assert!(j["access_token"].as_str().unwrap().starts_with("ey"));
    assert_eq!(j["user"]["email"], "ada@google.test");
    assert_eq!(j["user"]["verified"], true);

    // 3. The link row exists, and the user was created passwordless.
    let pool = state
        .workspaces
        .pool_for(&rustbase_core::WorkspaceId::from("acme".to_string()))
        .await
        .unwrap();
    let link = rustbase_db::oauth_links::find_by_provider_user(&pool, "google", "google-sub-42")
        .await
        .unwrap()
        .unwrap();
    let user = rustbase_db::users::find_user_by_id(&pool, &link.user_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        user.password_hash.is_none(),
        "new OAuth signup is passwordless"
    );
    assert!(user.verified);
}

#[tokio::test(flavor = "multi_thread")]
pub(super) async fn oauth_callback_links_to_existing_password_user_by_email() {
    // A user already exists with the same email (registered with
    // a password). OAuth callback should link the provider account
    // to the existing user, NOT create a duplicate.
    let (base_url, _h) = fake_oauth_provider(serde_json::json!({
        "sub": "google-sub-99",
        "email": "u@acme.com",  // matches state_with_collection_and_user
    }))
    .await;
    let (state, _dir, _, _) = state_with_collection_and_user().await;
    seed_provider(&state, "acme", "mobile", "google", &base_url).await;

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

    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/oauth/google/callback",
            None,
            Some(&serde_json::json!({"code":"unused", "state": nonce})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let pool = state
        .workspaces
        .pool_for(&rustbase_core::WorkspaceId::from("acme".to_string()))
        .await
        .unwrap();
    // Still exactly one user with that email.
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users WHERE email = ?")
        .bind("u@acme.com")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count.0, 1);
    // Password preserved.
    let user = rustbase_db::users::find_user_by_email(&pool, "u@acme.com")
        .await
        .unwrap()
        .unwrap();
    assert!(user.password_hash.is_some());
    // Link row exists pointing at the same user_id.
    let link = rustbase_db::oauth_links::find_by_provider_user(&pool, "google", "google-sub-99")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(link.user_id, user.id);
}

#[tokio::test(flavor = "multi_thread")]
pub(super) async fn oauth_callback_replayed_state_returns_409() {
    let (base_url, _h) = fake_oauth_provider(serde_json::json!({
        "sub": "google-sub-1", "email": "a@x"
    }))
    .await;
    let (state, _dir, _, _) = state_with_workspace_and_admin().await;
    seed_provider(&state, "acme", "mobile", "google", &base_url).await;

    let app = build_router(state.clone());
    let nonce = json_body(
        app.oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/auth/oauth/google/authorize?redirect_uri=https%3A%2F%2Fapp%2Fcb",
            None,
            None,
        ))
        .await
        .unwrap(),
    )
    .await["state"]
        .as_str()
        .unwrap()
        .to_string();

    // First consume succeeds.
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/oauth/google/callback",
            None,
            Some(&serde_json::json!({"code":"unused", "state": &nonce})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Replay must be rejected.
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/oauth/google/callback",
            None,
            Some(&serde_json::json!({"code":"unused", "state": &nonce})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test(flavor = "multi_thread")]
pub(super) async fn oauth_callback_unknown_state_returns_401() {
    let (base_url, _h) = fake_oauth_provider(serde_json::json!({})).await;
    let (state, _dir, _, _) = state_with_workspace_and_admin().await;
    seed_provider(&state, "acme", "mobile", "google", &base_url).await;

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/oauth/google/callback",
            None,
            Some(&serde_json::json!({"code":"unused", "state":"forged-or-stale"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
pub(super) async fn verify_email_request_rejects_admin_tokens() {
    // A workspace-admin token isn't tied to an end user, so /request
    // must reject it rather than try to mail a non-existent user.
    let (state, _dir, _, workspace_admin_id) = state_with_workspace_and_admin().await;
    let admin_tok = workspace_token(&state, "acme", &workspace_admin_id);
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/verify-email/request",
            Some(&admin_tok),
            Some(&serde_json::json!({})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
pub(super) async fn user_blocked_from_records_without_a_rule() {
    let (state, _dir, _, user_tok) = state_with_collection_and_user().await;
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/apps/mobile/collections/notes/records",
            Some(&user_tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
pub(super) async fn open_list_rule_lets_user_read() {
    let (state, _dir, master_tok, user_tok) = state_with_collection_and_user().await;
    // master opens 'list' rule
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "PUT",
            "/api/workspaces/acme/apps/mobile/collections/notes/access_rules/list",
            Some(&master_tok),
            Some(&serde_json::json!({"filter": ""})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // user can now read
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/apps/mobile/collections/notes/records",
            Some(&user_tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
pub(super) async fn user_in_one_realm_cannot_read_another_realms_records() {
    let (state, _dir, master_tok, user_tok) = state_with_collection_and_user().await;
    // master creates widgetco with an OPEN notes collection
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces",
        Some(&master_tok),
        Some(&serde_json::json!({"id":"widgetco","name":"W"})),
    ))
    .await
    .unwrap();
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/widgetco/apps",
        Some(&master_tok),
        Some(&serde_json::json!({"id":"web","name":"W"})),
    ))
    .await
    .unwrap();
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/widgetco/apps/web/collections",
        Some(&master_tok),
        Some(&serde_json::json!({
            "schema":{"id":"items","kind":"base",
                      "fields":[{"name":"name","kind":"text","required":true}]}
        })),
    ))
    .await
    .unwrap();
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "PUT",
        "/api/workspaces/widgetco/apps/web/collections/items/access_rules/list",
        Some(&master_tok),
        Some(&serde_json::json!({"filter": ""})),
    ))
    .await
    .unwrap();

    // acme's user tries widgetco — must be 403 even with an open rule
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/widgetco/apps/web/collections/items/records",
            Some(&user_tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
pub(super) async fn template_rule_scopes_user_to_own_rows() {
    let (state, _dir, master_tok, user_tok) = state_with_collection_and_user().await;

    // Add an `owner` text field to 'notes' so the rule has something to bind to.
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "DELETE",
            "/api/workspaces/acme/apps/mobile/collections/notes",
            Some(&master_tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/apps/mobile/collections",
            Some(&master_tok),
            Some(&serde_json::json!({
                "schema": {
                    "id": "notes",
                    "kind": "base",
                    "fields": [
                        {"name":"title","kind":"text","required":true},
                        {"name":"owner","kind":"text","required":true}
                    ]
                }
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Seed: one record owned by our user, one owned by someone else.
    let user_id: String = {
        let row: (String,) = sqlx::query_as("SELECT id FROM users WHERE email = ?")
            .bind("u@acme.com")
            .fetch_one(
                &state
                    .workspaces
                    .pool_for(&rustbase_core::WorkspaceId::from("acme"))
                    .await
                    .unwrap(),
            )
            .await
            .unwrap();
        row.0
    };
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/apps/mobile/collections/notes/records",
        Some(&master_tok),
        Some(&serde_json::json!({"title": "mine", "owner": user_id})),
    ))
    .await
    .unwrap();
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/apps/mobile/collections/notes/records",
        Some(&master_tok),
        Some(&serde_json::json!({"title": "theirs", "owner": "other-user-id"})),
    ))
    .await
    .unwrap();

    // Template rule: each user sees only their own rows.
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "PUT",
            "/api/workspaces/acme/apps/mobile/collections/notes/access_rules/list",
            Some(&master_tok),
            Some(&serde_json::json!({"filter": "owner = {{request.auth.id}}"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // The user should now see ONE row (their own).
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/apps/mobile/collections/notes/records",
            Some(&user_tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    assert_eq!(j["total_items"], 1);
    assert_eq!(j["items"][0]["fields"]["title"], "mine");
}

#[tokio::test]
pub(super) async fn template_rule_scoped_get_returns_404_for_unowned() {
    let (state, _dir, master_tok, user_tok) = state_with_collection_and_user().await;

    // Replace notes with an owner field.
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "DELETE",
        "/api/workspaces/acme/apps/mobile/collections/notes",
        Some(&master_tok),
        None,
    ))
    .await
    .unwrap();
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/apps/mobile/collections",
        Some(&master_tok),
        Some(&serde_json::json!({
            "schema": {
                "id":"notes","kind":"base","fields":[
                    {"name":"title","kind":"text","required":true},
                    {"name":"owner","kind":"text","required":true}
                ]
            }
        })),
    ))
    .await
    .unwrap();

    // Make a record owned by someone else.
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/apps/mobile/collections/notes/records",
            Some(&master_tok),
            Some(&serde_json::json!({"title":"x","owner":"other"})),
        ))
        .await
        .unwrap();
    let id = json_body(resp).await["id"].as_str().unwrap().to_string();

    // Open both view + list with the same per-row rule.
    for action in ["view", "list"] {
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "PUT",
            &format!("/api/workspaces/acme/apps/mobile/collections/notes/access_rules/{action}"),
            Some(&master_tok),
            Some(&serde_json::json!({"filter":"owner = {{request.auth.id}}"})),
        ))
        .await
        .unwrap();
    }

    // GET that record as the user → 404 (not their row).
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            &format!("/api/workspaces/acme/apps/mobile/collections/notes/records/{id}"),
            Some(&user_tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
pub(super) async fn template_with_unknown_placeholder_is_400() {
    let (state, _dir, master_tok, user_tok) = state_with_collection_and_user().await;

    // Open list with a bogus placeholder.
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "PUT",
        "/api/workspaces/acme/apps/mobile/collections/notes/access_rules/list",
        Some(&master_tok),
        Some(&serde_json::json!({"filter": "title = {{request.unknown}}"})),
    ))
    .await
    .unwrap();

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/apps/mobile/collections/notes/records",
            Some(&user_tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
pub(super) async fn user_refresh_rotates_token() {
    let (state, _dir, _, _) = state_with_collection_and_user().await;
    // login to capture refresh
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
    let j = json_body(resp).await;
    let first = j["refresh_token"].as_str().unwrap().to_string();

    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/users/refresh",
            None,
            Some(&serde_json::json!({"refresh_token": first})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    let second = j["refresh_token"].as_str().unwrap().to_string();
    assert_ne!(first, second);

    // reuse fails
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/users/refresh",
            None,
            Some(&serde_json::json!({"refresh_token": first})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
