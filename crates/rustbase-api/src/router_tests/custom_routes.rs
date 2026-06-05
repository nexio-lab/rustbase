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
pub(super) async fn router_add_get_returns_handler_json() {
    let (state, _dir, _) = state_with_app_and_collection().await;
    plant_hook_in_app(
        &state,
        "acme",
        "mobile",
        r#"
        $app.routerAdd("GET", "/hello", (ctx) => ({
            status: 200,
            body: { method: ctx.method, who: ctx.query.who || "stranger" },
        }));
        "#,
    )
    .await;

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/apps/mobile/custom/hello?who=ada",
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    assert_eq!(j["method"], "GET");
    assert_eq!(j["who"], "ada");
}

#[tokio::test]
pub(super) async fn router_add_unknown_path_returns_404() {
    let (state, _dir, _) = state_with_app_and_collection().await;
    // No routerAdd at all → catch-all should answer 404.
    plant_hook_in_app(&state, "acme", "mobile", "/* nothing */").await;

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/apps/mobile/custom/missing",
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
pub(super) async fn router_add_post_sees_json_body_and_headers() {
    let (state, _dir, _) = state_with_app_and_collection().await;
    plant_hook_in_app(
        &state,
        "acme",
        "mobile",
        r#"
        $app.routerAdd("POST", "/echo", (ctx) => ({
            body: {
                got_body: ctx.body,
                saw_content_type: ctx.headers["content-type"],
            },
        }));
        "#,
    )
    .await;

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/apps/mobile/custom/echo",
            None,
            Some(&serde_json::json!({"hello":"world","n":42})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = json_body(resp).await;
    assert_eq!(j["got_body"]["hello"], "world");
    assert_eq!(j["got_body"]["n"], 42);
    assert_eq!(j["saw_content_type"], "application/json");
}

#[tokio::test]
pub(super) async fn router_add_handler_throw_returns_500() {
    let (state, _dir, _) = state_with_app_and_collection().await;
    plant_hook_in_app(
        &state,
        "acme",
        "mobile",
        r#"$app.routerAdd("GET", "/boom", () => { throw new Error("kapow"); });"#,
    )
    .await;

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/apps/mobile/custom/boom",
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let j = json_body(resp).await;
    assert_eq!(j["error"], "kapow");
}

#[tokio::test]
pub(super) async fn router_add_method_mismatch_returns_404() {
    let (state, _dir, _) = state_with_app_and_collection().await;
    plant_hook_in_app(
        &state,
        "acme",
        "mobile",
        r#"$app.routerAdd("GET", "/only-get", () => ({body:"ok"}));"#,
    )
    .await;

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/apps/mobile/custom/only-get",
            None,
            Some(&serde_json::json!({})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
pub(super) async fn router_add_routes_are_scoped_to_the_owning_app() {
    // Same path registered in app A; app B has nothing — the
    // request to B's namespace must miss even though A would
    // have answered.
    let (state, _dir, master_tok, workspace_admin_id) = {
        let (state, dir, master_id, workspace_admin_id) = state_with_workspace_and_admin().await;
        let master_tok = master_token(&state, &master_id);
        (state, dir, master_tok, workspace_admin_id)
    };
    let workspace_tok = workspace_token(&state, "acme", &workspace_admin_id);

    // Create two sibling apps under acme.
    for app_id in ["alpha", "beta"] {
        let app = build_router(state.clone());
        let resp = app
            .oneshot(req_with_auth(
                "POST",
                "/api/workspaces/acme/apps",
                Some(&workspace_tok),
                Some(&serde_json::json!({"id":app_id,"name":app_id})),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    // Plant a hook only in alpha.
    plant_hook_in_app(
        &state,
        "acme",
        "alpha",
        r#"$app.routerAdd("GET", "/hi", () => ({body:"from-alpha"}));"#,
    )
    .await;

    // alpha answers.
    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/apps/alpha/custom/hi",
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // beta does NOT — same path, different app namespace.
    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "GET",
            "/api/workspaces/acme/apps/beta/custom/hi",
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let _ = master_tok;
}
