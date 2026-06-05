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
use super::workspace_admin_app_crud::*;
#[allow(unused_imports)]
use super::workspace_crud::*;
use crate::router::*;
use axum::http::StatusCode;
use tower::ServiceExt;

#[tokio::test]
pub(super) async fn system_audit_lists_master_scope_entries() {
    // policy_set on master writes a row into system.audit_log via
    // policy_engine::set_master_policy. Driving it through the
    // public PUT endpoint is the simplest seed.
    let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
    let tok = master_token(&state, &admin_id);

    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "PUT",
            "/api/system/policies/password.length",
            Some(&tok),
            Some(&serde_json::json!({"kind":"range","min":4,"max":64})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/system/audit?per_page=10",
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    assert!(j["total_items"].as_u64().unwrap() >= 1);
    let actions: Vec<String> = j["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap().to_string())
        .collect();
    assert!(actions.iter().any(|a| a == "policy_set"));
}

#[tokio::test]
pub(super) async fn system_audit_filters_by_action_substring() {
    let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
    let tok = master_token(&state, &admin_id);

    // seed two distinct actions
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "PUT",
        "/api/system/policies/password.length",
        Some(&tok),
        Some(&serde_json::json!({"kind":"range","min":4,"max":64})),
    ))
    .await
    .unwrap();
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "DELETE",
        "/api/system/policies/password.length",
        Some(&tok),
        None,
    ))
    .await
    .unwrap();

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/system/audit?action=delete",
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    let j = json_body(resp).await;
    let items = j["items"].as_array().unwrap();
    assert!(!items.is_empty());
    for e in items {
        assert!(e["action"].as_str().unwrap().contains("delete"), "{e:?}");
    }
}

#[tokio::test]
pub(super) async fn system_audit_requires_master() {
    let (state, _dir, _, workspace_admin_id) = state_with_workspace_and_admin().await;
    let workspace_tok = workspace_token(&state, "acme", &workspace_admin_id);
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/system/audit",
            Some(&workspace_tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
pub(super) async fn workspace_audit_lists_entries_after_clamp() {
    // Tightening a master bound auto-clamps any workspace value that
    // falls outside it AND writes "policy_clamped" into the workspace's
    // audit log. That's the cascade we want to surface in the UI.
    let (state, _dir, master_id, workspace_admin_id) = state_with_workspace_and_admin().await;
    let master_tok = master_token(&state, &master_id);
    let workspace_tok = workspace_token(&state, "acme", &workspace_admin_id);

    // 1. master opens the bound wide
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "PUT",
        "/api/system/policies/password.length",
        Some(&master_tok),
        Some(&serde_json::json!({"kind":"range","min":4,"max":64})),
    ))
    .await
    .unwrap();
    // 2. workspace picks a tighter range inside the master bound
    // (workspace bound must fit inside the master bound)
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "PUT",
        "/api/workspaces/acme/policies/password.length",
        Some(&workspace_tok),
        Some(&serde_json::json!({"kind":"range","min":6,"max":12})),
    ))
    .await
    .unwrap();
    // 3. master tightens the upper bound below the workspace value
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "PUT",
            "/api/system/policies/password.length",
            Some(&master_tok),
            Some(&serde_json::json!({"kind":"range","min":4,"max":10})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // workspace-scoped audit should now mention policy_clamped
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/audit",
            Some(&workspace_tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    let actions: Vec<String> = j["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap().to_string())
        .collect();
    assert!(actions.iter().any(|a| a == "policy_clamped"), "{actions:?}");
}

#[tokio::test]
pub(super) async fn audit_on_unknown_realm_is_404() {
    let (state, _dir, admin_id) = initialized_state_with_admin("hunter22").await;
    let tok = master_token(&state, &admin_id);
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/nope/audit",
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
