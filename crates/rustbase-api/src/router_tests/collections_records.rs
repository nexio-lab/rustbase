#[allow(unused_imports)]
use super::admin_users::*;
#[allow(unused_imports)]
use super::auth_flow::*;
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

/// Idempotent: PUTs the `mobile` app inside `acme` using a freshly
/// minted workspace-admin token if it isn't there yet. Returns the
/// workspace-admin token so callers can keep using it.
pub(super) async fn ensure_mobile_app(state: &AppState, workspace_admin_id: &str) -> String {
    let tok = workspace_token(state, "acme", workspace_admin_id);
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/apps",
            Some(&tok),
            Some(&serde_json::json!({"id":"mobile","name":"M"})),
        ))
        .await
        .unwrap();
    // 201 on first call, 409 on later calls — both are fine.
    assert!(
        resp.status() == StatusCode::CREATED || resp.status() == StatusCode::CONFLICT,
        "unexpected status creating mobile app: {}",
        resp.status()
    );
    tok
}

/// Bootstrap to: workspace 'acme', workspace-admin token, app 'mobile',
/// collection 'notes' with fields {title:text, pinned:bool,
/// metadata:json}. Returns (state, dir, workspace_token).
pub(super) async fn state_with_app_and_collection() -> (AppState, tempfile::TempDir, String) {
    let (state, dir, _, workspace_admin_id) = state_with_workspace_and_admin().await;
    let tok = ensure_mobile_app(&state, &workspace_admin_id).await;

    // create collection
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/apps/mobile/collections",
            Some(&tok),
            Some(&serde_json::json!({
                "schema": {
                    "id": "notes",
                    "kind": "base",
                    "fields": [
                        {"name": "title", "kind": "text", "required": true},
                        {"name": "pinned", "kind": "bool"},
                        {"name": "metadata", "kind": "json"}
                    ]
                }
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    (state, dir, tok)
}

#[tokio::test]
pub(super) async fn collection_reserved_id_is_rejected() {
    let (state, _dir, tok) = state_with_app_and_collection().await;
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/apps/mobile/collections",
            Some(&tok),
            Some(&serde_json::json!({
                "schema": {"id": "policies", "kind": "base", "fields": []}
            })),
        ))
        .await
        .unwrap();
    // collections::create_collection returns InvalidIdentifier →
    // CoreError::Validation → 400
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
pub(super) async fn record_full_lifecycle() {
    let (state, _dir, tok) = state_with_app_and_collection().await;

    // CREATE
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/apps/mobile/collections/notes/records",
            Some(&tok),
            Some(&serde_json::json!({
                "title": "Hello",
                "pinned": true,
                "metadata": {"tags": ["greeting"], "version": 1}
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created = json_body(resp).await;
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["fields"]["title"], "Hello");
    assert_eq!(created["fields"]["pinned"], true);
    assert_eq!(created["fields"]["metadata"]["version"], 1);

    // GET
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            &format!("/api/workspaces/acme/apps/mobile/collections/notes/records/{id}"),
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let got = json_body(resp).await;
    assert_eq!(got["fields"]["title"], "Hello");

    // PATCH — only "title" supplied; "pinned" stays true
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "PATCH",
            &format!("/api/workspaces/acme/apps/mobile/collections/notes/records/{id}"),
            Some(&tok),
            Some(&serde_json::json!({"title": "Goodbye"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let patched = json_body(resp).await;
    assert_eq!(patched["fields"]["title"], "Goodbye");
    assert_eq!(patched["fields"]["pinned"], true);

    // LIST (pagination response shape)
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/apps/mobile/collections/notes/records?per_page=10",
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    let listed = json_body(resp).await;
    assert_eq!(listed["total_items"], 1);
    assert_eq!(listed["page"], 1);
    assert_eq!(listed["per_page"], 10);
    assert_eq!(listed["items"].as_array().unwrap().len(), 1);

    // DELETE
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "DELETE",
            &format!("/api/workspaces/acme/apps/mobile/collections/notes/records/{id}"),
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // GET-after-delete → 404
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            &format!("/api/workspaces/acme/apps/mobile/collections/notes/records/{id}"),
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
pub(super) async fn delete_collection_drops_table() {
    let (state, _dir, tok) = state_with_app_and_collection().await;
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "DELETE",
            "/api/workspaces/acme/apps/mobile/collections/notes",
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // creating a record now fails because the collection (and table) are gone
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/apps/mobile/collections/notes/records",
            Some(&tok),
            Some(&serde_json::json!({"title": "x"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
pub(super) async fn list_records_with_filter_returns_only_matching() {
    let (state, _dir, tok) = state_with_app_and_collection().await;

    // Add 3 notes — only 2 have pinned=true.
    for (title, pinned) in [("a", true), ("b", false), ("c", true)] {
        let app = build_router(state.clone());
        app.oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/apps/mobile/collections/notes/records",
            Some(&tok),
            Some(&serde_json::json!({"title": title, "pinned": pinned})),
        ))
        .await
        .unwrap();
    }

    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/apps/mobile/collections/notes/records?filter=pinned%20%3D%20true",
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    assert_eq!(j["total_items"], 2);
    assert_eq!(j["items"].as_array().unwrap().len(), 2);
}

#[tokio::test]
pub(super) async fn list_records_with_unknown_filter_column_is_400() {
    let (state, _dir, tok) = state_with_app_and_collection().await;
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/apps/mobile/collections/notes/records?filter=nope%20%3D%20%22x%22",
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let j = json_body(resp).await;
    let msg = j["message"].as_str().unwrap();
    assert!(msg.contains("nope"), "got message: {msg}");
}
