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
use super::workspace_crud::*;
use crate::router::*;
use axum::http::StatusCode;
use tower::ServiceExt;

/// Bootstrap state through to "workspace 'acme' exists, has one workspace
/// admin (ops@acme/secretpw)". Returns the workspace-admin's id.
pub(super) async fn state_with_workspace_and_admin() -> (AppState, tempfile::TempDir, String, String)
{
    let (state, dir, master_id) = initialized_state_with_admin("hunter22").await;
    let master_tok = master_token(&state, &master_id);
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces",
            Some(&master_tok),
            Some(&serde_json::json!({"id":"acme","name":"Acme"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/admins",
            Some(&master_tok),
            Some(&serde_json::json!({
                "email":"ops@acme.com","password":"secretpw","name":"Ops"
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let admin = json_body(resp).await;
    let admin_id = admin["id"].as_str().unwrap().to_string();

    // Provision the canonical `mobile` app so every downstream test
    // that hits `/apps/mobile/...` has a target. This is the home
    // for end-user / OAuth state after the users-per-app refactor.
    let _ = ensure_mobile_app(&state, &admin_id).await;

    (state, dir, master_id, admin_id)
}

pub(super) fn workspace_token(state: &AppState, workspace: &str, admin_id: &str) -> String {
    let claims = rustbase_auth::build_claims(
        admin_id,
        rustbase_auth::TokenRole::WorkspaceAdmin,
        Some(workspace.into()),
        None,
        chrono::Duration::minutes(15),
    );
    state.jwt.issue(&claims).unwrap()
}

#[tokio::test]
pub(super) async fn workspace_admin_creation_requires_master() {
    let (state, _dir, _, workspace_admin_id) = state_with_workspace_and_admin().await;
    let workspace_tok = workspace_token(&state, "acme", &workspace_admin_id);
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/admins",
            Some(&workspace_tok),
            Some(&serde_json::json!({"email":"x@y.z","password":"longenough"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
pub(super) async fn workspace_admin_login_returns_workspace_scoped_token() {
    let (state, _dir, _, _) = state_with_workspace_and_admin().await;
    let app = build_router(state);
    let resp = post_json(
        app,
        "/api/workspaces/acme/auth/admin/login",
        &serde_json::json!({"email":"ops@acme.com","password":"secretpw"}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    assert!(j["access_token"].as_str().unwrap().starts_with("ey"));
    assert!(j["refresh_token"].as_str().unwrap().starts_with("rfsh_"));
    assert_eq!(j["admin"]["email"], "ops@acme.com");
}

#[tokio::test]
pub(super) async fn workspace_admin_login_wrong_password_is_401() {
    let (state, _dir, _, _) = state_with_workspace_and_admin().await;
    let app = build_router(state);
    let resp = post_json(
        app,
        "/api/workspaces/acme/auth/admin/login",
        &serde_json::json!({"email":"ops@acme.com","password":"wrong"}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
pub(super) async fn workspace_admin_refresh_rotates() {
    let (state, _dir, _, _) = state_with_workspace_and_admin().await;
    let app = build_router(state.clone());
    let resp = post_json(
        app,
        "/api/workspaces/acme/auth/admin/login",
        &serde_json::json!({"email":"ops@acme.com","password":"secretpw"}),
    )
    .await;
    let j = json_body(resp).await;
    let first = j["refresh_token"].as_str().unwrap().to_string();

    let app = build_router(state.clone());
    let resp = post_json(
        app,
        "/api/workspaces/acme/auth/refresh",
        &serde_json::json!({"refresh_token": first}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    let second = j["refresh_token"].as_str().unwrap().to_string();
    assert_ne!(first, second);

    let app = build_router(state);
    let resp = post_json(
        app,
        "/api/workspaces/acme/auth/refresh",
        &serde_json::json!({"refresh_token": first}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
pub(super) async fn workspace_admin_creates_and_lists_apps_in_own_workspace() {
    let (state, _dir, _, workspace_admin_id) = state_with_workspace_and_admin().await;
    let workspace_tok = workspace_token(&state, "acme", &workspace_admin_id);

    // `state_with_workspace_and_admin` already provisioned a `mobile`
    // app; use a different id for the create-step here.
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/apps",
            Some(&workspace_tok),
            Some(&serde_json::json!({"id":"crm","name":"CRM"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let j = json_body(resp).await;
    assert_eq!(j["id"], "crm");

    // data.db is initialized — listing collections (the meta table)
    // should be empty without erroring.
    let app_pool = state
        .apps
        .pool_for(
            &rustbase_core::WorkspaceId::from("acme"),
            &rustbase_core::AppId::from("crm"),
        )
        .await
        .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _collections")
        .fetch_one(&app_pool)
        .await
        .unwrap();
    assert_eq!(n, 0);

    // list returns both apps (the bootstrap `mobile` + this `crm`).
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/apps",
            Some(&workspace_tok),
            None,
        ))
        .await
        .unwrap();
    let j = json_body(resp).await;
    assert_eq!(j.as_array().unwrap().len(), 2);
}

#[tokio::test]
pub(super) async fn workspace_admin_cannot_act_on_other_workspace() {
    // Create workspace 'acme' with admin 'ops@acme', and a second workspace
    // 'widgetco'. The acme admin must not be able to list widgetco's
    // apps.
    let (state, _dir, master_id, workspace_admin_id) = state_with_workspace_and_admin().await;
    let master_tok = master_token(&state, &master_id);

    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces",
        Some(&master_tok),
        Some(&serde_json::json!({"id":"widgetco","name":"WidgetCo"})),
    ))
    .await
    .unwrap();

    let acme_tok = workspace_token(&state, "acme", &workspace_admin_id);
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/widgetco/apps",
            Some(&acme_tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
pub(super) async fn create_app_with_uppercase_id_is_400() {
    let (state, _dir, master_id, _) = state_with_workspace_and_admin().await;
    let master_tok = master_token(&state, &master_id);
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/apps",
            Some(&master_tok),
            Some(&serde_json::json!({"id":"Mobile","name":"x"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
pub(super) async fn create_duplicate_app_returns_409() {
    let (state, _dir, master_id, _) = state_with_workspace_and_admin().await;
    let master_tok = master_token(&state, &master_id);
    let body = serde_json::json!({"id":"mobile","name":"M"});

    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/apps",
        Some(&master_tok),
        Some(&body),
    ))
    .await
    .unwrap();

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/apps",
            Some(&master_tok),
            Some(&body),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
pub(super) async fn create_app_in_unknown_realm_is_404() {
    let (state, _dir, master_id, _) = state_with_workspace_and_admin().await;
    let master_tok = master_token(&state, &master_id);
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/no-such-workspace/apps",
            Some(&master_tok),
            Some(&serde_json::json!({"id":"mobile","name":"Mobile"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
pub(super) async fn delete_app_removes_row_and_folder() {
    let (state, dir, master_id, _) = state_with_workspace_and_admin().await;
    let master_tok = master_token(&state, &master_id);

    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/apps",
        Some(&master_tok),
        Some(&serde_json::json!({"id":"mobile","name":"M"})),
    ))
    .await
    .unwrap();
    let app_folder = dir.path().join("workspaces/acme/apps/mobile");
    assert!(app_folder.exists());

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "DELETE",
            "/api/workspaces/acme/apps/mobile",
            Some(&master_tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert!(!app_folder.exists());
}

#[tokio::test]
pub(super) async fn rename_app_updates_name() {
    let (state, _dir, _, workspace_admin_id) = state_with_workspace_and_admin().await;
    let tok = workspace_token(&state, "acme", &workspace_admin_id);

    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/apps",
        Some(&tok),
        Some(&serde_json::json!({"id":"mobile","name":"Original"})),
    ))
    .await
    .unwrap();

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "PATCH",
            "/api/workspaces/acme/apps/mobile",
            Some(&tok),
            Some(&serde_json::json!({"name":"Renamed"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    assert_eq!(j["name"], "Renamed");
}
