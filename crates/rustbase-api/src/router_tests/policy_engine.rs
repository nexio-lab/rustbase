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
pub(super) async fn master_can_set_policy_and_realm_clamps_below_master_bound() {
    let (state, _dir, master_id, workspace_admin_id) = state_with_workspace_and_admin().await;
    let m_tok = master_token(&state, &master_id);
    let r_tok = workspace_token(&state, "acme", &workspace_admin_id);

    // master sets range [4, 64]
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "PUT",
            "/api/system/policies/password.length",
            Some(&m_tok),
            Some(&serde_json::json!({"kind":"range","min":4,"max":64})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // workspace sets [8, 32] — inside master, OK
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "PUT",
            "/api/workspaces/acme/policies/password.length",
            Some(&r_tok),
            Some(&serde_json::json!({"kind":"range","min":8,"max":32})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // workspace tries [2, 100] — violates master → 409
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "PUT",
            "/api/workspaces/acme/policies/password.length",
            Some(&r_tok),
            Some(&serde_json::json!({"kind":"range","min":2,"max":100})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
pub(super) async fn master_tighten_cascades_into_realm_value() {
    let (state, _dir, master_id, workspace_admin_id) = state_with_workspace_and_admin().await;
    let m_tok = master_token(&state, &master_id);
    let r_tok = workspace_token(&state, "acme", &workspace_admin_id);

    // master [4, 64], workspace [8, 32]
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "PUT",
        "/api/system/policies/password.length",
        Some(&m_tok),
        Some(&serde_json::json!({"kind":"range","min":4,"max":64})),
    ))
    .await
    .unwrap();
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "PUT",
        "/api/workspaces/acme/policies/password.length",
        Some(&r_tok),
        Some(&serde_json::json!({"kind":"range","min":8,"max":32})),
    ))
    .await
    .unwrap();

    // master tightens to [10, 20]; cascade flag should report it
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "PUT",
            "/api/system/policies/password.length",
            Some(&m_tok),
            Some(&serde_json::json!({"kind":"range","min":10,"max":20})),
        ))
        .await
        .unwrap();
    let j = json_body(resp).await;
    assert_eq!(j["cascaded"].as_array().unwrap().len(), 1);
    let outcome = &j["cascaded"][0];
    assert_eq!(outcome["workspace"], "acme");
    assert_eq!(outcome["after"]["min"], 10);
    assert_eq!(outcome["after"]["max"], 20);

    // workspace value reflects the clamp
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/policies/password.length",
            Some(&r_tok),
            None,
        ))
        .await
        .unwrap();
    let j = json_body(resp).await;
    assert_eq!(j["min"], 10);
    assert_eq!(j["max"], 20);
}

#[tokio::test]
pub(super) async fn workspace_admin_cannot_set_master_policy() {
    let (state, _dir, _, workspace_admin_id) = state_with_workspace_and_admin().await;
    let r_tok = workspace_token(&state, "acme", &workspace_admin_id);
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "PUT",
            "/api/system/policies/password.length",
            Some(&r_tok),
            Some(&serde_json::json!({"kind":"range","min":4,"max":64})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
pub(super) async fn list_records_with_malformed_filter_is_400() {
    let (state, _dir, tok) = state_with_app_and_collection().await;
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/apps/mobile/collections/notes/records?filter=this+is+not+valid",
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
pub(super) async fn cross_realm_admin_cannot_read_records() {
    let (state, _dir, _) = state_with_app_and_collection().await;
    // create workspace 'widgetco' + its own admin, and try to list acme/mobile/notes/records
    let master_admin_id = rustbase_db::admins::count_master_admins(state.system.pool())
        .await
        .map(|_| {
            // we can't get the id back from count; just decode the master admin from email
            // — simpler: pull one row
        });
    let _ = master_admin_id;

    // simpler: derive the master id from the inserted row
    let row: (String,) = sqlx::query_as("SELECT id FROM master_admins LIMIT 1")
        .fetch_one(state.system.pool())
        .await
        .unwrap();
    let master_tok = master_token(&state, &row.0);

    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces",
        Some(&master_tok),
        Some(&serde_json::json!({"id":"widgetco","name":"WidgetCo"})),
    ))
    .await
    .unwrap();
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/widgetco/admins",
        Some(&master_tok),
        Some(&serde_json::json!({"email":"w@w","password":"longenough"})),
    ))
    .await
    .unwrap();

    // widgetco admin token (we know the id from the create response? we didn't capture
    // it — just use master_token + role swap)
    let claims = rustbase_auth::build_claims(
        "fake-admin",
        rustbase_auth::TokenRole::WorkspaceAdmin,
        Some("widgetco".into()),
        None,
        chrono::Duration::minutes(15),
    );
    let w_tok = state.jwt.issue(&claims).unwrap();

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/apps/mobile/collections/notes/records",
            Some(&w_tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
