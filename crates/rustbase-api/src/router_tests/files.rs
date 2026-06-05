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
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;

#[tokio::test]
pub(super) async fn file_upload_then_download_round_trip() {
    let (state, _dir, _, workspace_admin_id) = state_with_workspace_and_admin().await;
    let tok = workspace_token(&state, "acme", &workspace_admin_id);

    // create app
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/apps",
        Some(&tok),
        Some(&serde_json::json!({"id":"mobile","name":"M"})),
    ))
    .await
    .unwrap();

    // upload
    let app = build_router(state.clone());
    let req = Request::builder()
        .uri("/api/workspaces/acme/apps/mobile/files")
        .method("POST")
        .header("authorization", format!("Bearer {tok}"))
        .header("content-type", "image/png")
        .header("x-filename", "kitten.png")
        .body(Body::from(b"\x89PNG\x0d\x0a\x1a\x0afakebytes".to_vec()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let j = json_body(resp).await;
    let id = j["id"].as_str().unwrap().to_string();
    assert_eq!(j["filename"], "kitten.png");
    assert_eq!(j["mime"], "image/png");
    assert_eq!(j["size"], 17);

    // download
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            &format!("/api/workspaces/acme/apps/mobile/files/{id}"),
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_string());
    assert_eq!(ct.as_deref(), Some("image/png"));
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(&bytes[..], b"\x89PNG\x0d\x0a\x1a\x0afakebytes");

    // list
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/apps/mobile/files",
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    let j = json_body(resp).await;
    assert_eq!(j.as_array().unwrap().len(), 1);

    // delete
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "DELETE",
            &format!("/api/workspaces/acme/apps/mobile/files/{id}"),
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
pub(super) async fn file_upload_without_filename_header_is_400() {
    let (state, _dir, _, workspace_admin_id) = state_with_workspace_and_admin().await;
    let tok = workspace_token(&state, "acme", &workspace_admin_id);
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/apps",
        Some(&tok),
        Some(&serde_json::json!({"id":"mobile","name":"M"})),
    ))
    .await
    .unwrap();

    let app = build_router(state);
    let req = Request::builder()
        .uri("/api/workspaces/acme/apps/mobile/files")
        .method("POST")
        .header("authorization", format!("Bearer {tok}"))
        .body(Body::from(b"x".to_vec()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
