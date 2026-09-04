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

/// An uploaded file is served back from the API's own origin, the one
/// that carries the `rb_at` cookie. Handing back `text/html` inline
/// would let an uploaded page run as first-party script; `nosniff`
/// does not help, since the type is declared rather than guessed.
/// Every download therefore comes back as an attachment.
#[tokio::test]
pub(super) async fn download_is_always_an_attachment_never_inline() {
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

    let app = build_router(state.clone());
    let req = Request::builder()
        .uri("/api/workspaces/acme/apps/mobile/files")
        .method("POST")
        .header("authorization", format!("Bearer {tok}"))
        .header("content-type", "text/html")
        .header("x-filename", "payload.html")
        .body(Body::from(b"<script>alert(1)</script>".to_vec()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let id = json_body(resp).await["id"].as_str().unwrap().to_string();

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
    let disposition = resp
        .headers()
        .get("content-disposition")
        .map(|v| v.to_str().unwrap().to_string())
        .expect("download must carry a Content-Disposition");
    assert!(
        disposition.starts_with("attachment"),
        "html upload served inline; got: {disposition}"
    );
}

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
