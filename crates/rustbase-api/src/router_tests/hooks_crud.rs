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

/// Bootstrap to: workspace 'acme' + app 'mobile' (no hooks yet). Returns
/// (state, dir, workspace_admin_token).
pub(super) async fn state_with_app() -> (AppState, tempfile::TempDir, String) {
    let (state, dir, _, workspace_admin_id) = state_with_workspace_and_admin().await;
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
    (state, dir, tok)
}

#[tokio::test]
pub(super) async fn hooks_list_is_empty_for_fresh_app() {
    let (state, _dir, tok) = state_with_app().await;
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/apps/mobile/hooks",
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    assert!(j.as_array().unwrap().is_empty());
}

#[tokio::test]
pub(super) async fn hooks_put_writes_file_and_returns_reload_outcome() {
    let (state, dir, tok) = state_with_app().await;
    // Use the actual hook surface — `$app.onRecordAfterCreate` is
    // defined in the sandbox, while `console` is not.
    let src = "$app.onRecordAfterCreate(\"notes\", function(rec){});";
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "PUT",
            "/api/workspaces/acme/apps/mobile/hooks/log.js",
            Some(&tok),
            Some(&serde_json::json!({"source": src})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    assert_eq!(j["file"]["filename"], "log.js");
    assert_eq!(j["file"]["source"], src);
    // Reload reports zero errors for valid JS.
    assert_eq!(j["reload"]["errors"].as_array().unwrap().len(), 0);
    assert_eq!(j["reload"]["loaded"], 1);

    // File landed in the expected path.
    let path = dir.path().join("hooks/acme/mobile/log.js");
    assert!(path.exists());
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(on_disk, src);

    // GET round-trips the body.
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/apps/mobile/hooks/log.js",
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    assert_eq!(j["source"], src);
}

#[tokio::test]
pub(super) async fn hooks_put_with_invalid_filename_is_400() {
    let (state, _dir, tok) = state_with_app().await;
    // axum normalizes "../" and "subdir/" before reaching the
    // handler, so those route to 404. The cases below all reach
    // the handler and exercise its own validation.
    for bad in ["no-ext", ".hidden.js", "weird name.js"] {
        let encoded = bad.replace(' ', "%20");
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "PUT",
                &format!("/api/workspaces/acme/apps/mobile/hooks/{encoded}"),
                Some(&tok),
                Some(&serde_json::json!({"source": ""})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "for filename {bad}");
    }
}

#[tokio::test]
pub(super) async fn hooks_put_then_list_returns_metadata() {
    let (state, _dir, tok) = state_with_app().await;

    // create two files
    for (name, src) in [("a.js", "var x = 1;"), ("b.ts", "const y: number = 2;")] {
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "PUT",
                &format!("/api/workspaces/acme/apps/mobile/hooks/{name}"),
                Some(&tok),
                Some(&serde_json::json!({"source": src})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/apps/mobile/hooks",
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    let j = json_body(resp).await;
    let arr = j.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["filename"], "a.js");
    assert_eq!(arr[1]["filename"], "b.ts");
    assert!(arr[0]["size"].as_u64().unwrap() > 0);
}

#[tokio::test]
pub(super) async fn hooks_delete_removes_file_and_reloads() {
    let (state, dir, tok) = state_with_app().await;

    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "PUT",
        "/api/workspaces/acme/apps/mobile/hooks/keep.js",
        Some(&tok),
        Some(&serde_json::json!({"source": "// noop"})),
    ))
    .await
    .unwrap();

    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "DELETE",
            "/api/workspaces/acme/apps/mobile/hooks/keep.js",
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    assert_eq!(j["loaded"], 0);

    let path = dir.path().join("hooks/acme/mobile/keep.js");
    assert!(!path.exists());

    // GET-after-delete → 404
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/apps/mobile/hooks/keep.js",
            Some(&tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
pub(super) async fn hooks_put_surfaces_compile_errors_in_reload_outcome() {
    let (state, _dir, tok) = state_with_app().await;
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "PUT",
            "/api/workspaces/acme/apps/mobile/hooks/broken.js",
            Some(&tok),
            // Unterminated string — script eval will throw.
            Some(&serde_json::json!({"source": "console.log('"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    // File is on disk and reload reports the error so the editor
    // can pin it next to the source.
    assert!(!j["reload"]["errors"].as_array().unwrap().is_empty());
}

#[tokio::test]
pub(super) async fn hooks_require_admin_token() {
    let (state, _dir, _) = state_with_app().await;

    // register + login an end-user
    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/auth/users/register",
        None,
        Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
    ))
    .await
    .unwrap();
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
    let user_tok = json_body(resp).await["access_token"]
        .as_str()
        .unwrap()
        .to_string();

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/apps/mobile/hooks",
            Some(&user_tok),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
