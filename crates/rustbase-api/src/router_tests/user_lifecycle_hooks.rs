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
use super::workspace_admin_app_crud::*;
#[allow(unused_imports)]
use super::workspace_crud::*;
use crate::router::*;
use axum::http::StatusCode;
use tower::ServiceExt;

/// Plant a hook source file under the AppState's data_dir at the
/// path apps::create would normally read from, then load it via
/// the HookEngine using the same bridge + mailer wiring as
/// production. Returns once the hook is live.
pub(super) async fn plant_hook_in_app(state: &AppState, workspace: &str, app: &str, src: &str) {
    let dir = state.data_dir.join("hooks").join(workspace).join(app);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("user_lifecycle.js"), src).unwrap();
    let bridge = crate::hook_bridge::ApiBridge::new(
        rustbase_core::WorkspaceId::from(workspace.to_string()),
        rustbase_core::AppId::from(app.to_string()),
        state.apps.clone(),
    )
    .into_sync();
    let quoted = std::sync::Arc::new(crate::mailer::QuotedMailer::new(
        state.mailer.clone(),
        rustbase_core::WorkspaceId::from(workspace.to_string()),
        rustbase_core::AppId::from(app.to_string()),
        state.apps.clone(),
    )) as std::sync::Arc<dyn rustbase_core::Mailer>;
    state
        .hooks
        .load_app(workspace, app, &dir, Some(bridge), Some(quoted))
        .await
        .unwrap();
}

#[tokio::test]
pub(super) async fn user_after_register_hook_fires_on_password_signup() {
    let (state, _dir, tok) = state_with_app_and_collection().await;
    plant_hook_in_app(
        &state,
        "acme",
        "mobile",
        r#"$app.onUserAfterRegister((u) => $app.log("welcome " + u.email));"#,
    )
    .await;

    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/users/register",
            None,
            Some(&serde_json::json!({"email":"ada@acme.com","password":"hunter22"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let logs = state
        .hooks
        .get("acme", "mobile")
        .unwrap()
        .drain_logs()
        .await
        .unwrap();
    assert_eq!(logs, vec!["welcome ada@acme.com".to_string()]);
    let _ = tok; // keep workspace-admin token alive through the test
}

#[tokio::test]
pub(super) async fn user_after_login_hook_fires_on_password_login() {
    let (state, _dir, _, _) = state_with_collection_and_user().await;
    plant_hook_in_app(
        &state,
        "acme",
        "mobile",
        r#"$app.onUserAfterLogin((u) => $app.log("login:" + u.email));"#,
    )
    .await;

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
    assert_eq!(resp.status(), StatusCode::OK);

    let logs = state
        .hooks
        .get("acme", "mobile")
        .unwrap()
        .drain_logs()
        .await
        .unwrap();
    assert_eq!(logs, vec!["login:u@acme.com".to_string()]);
}

#[tokio::test]
pub(super) async fn user_before_login_hook_can_veto_password_login() {
    let (state, _dir, _, _) = state_with_collection_and_user().await;
    plant_hook_in_app(
        &state,
        "acme",
        "mobile",
        r#"$app.onUserBeforeLogin((u) => { throw new Error("banned:" + u.email); });"#,
    )
    .await;

    let app = build_router(state);
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/users/login",
            None,
            // Credentials are CORRECT — the hook is what blocks.
            Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
pub(super) async fn user_after_login_hook_does_not_see_password_hash() {
    // Defensive: the user object handed to hooks must only carry
    // public fields. Even though the underlying User struct has
    // password_hash, the JSON we pass to the JS runtime must not.
    let (state, _dir, _, _) = state_with_collection_and_user().await;
    plant_hook_in_app(
        &state,
        "acme",
        "mobile",
        r#"$app.onUserAfterLogin((u) => $app.log("keys:" + Object.keys(u).sort().join(",")));"#,
    )
    .await;

    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/auth/users/login",
        None,
        Some(&serde_json::json!({"email":"u@acme.com","password":"userpass1"})),
    ))
    .await
    .unwrap();

    let logs = state
        .hooks
        .get("acme", "mobile")
        .unwrap()
        .drain_logs()
        .await
        .unwrap();
    assert_eq!(logs, vec!["keys:email,id,verified".to_string()]);
}

#[tokio::test]
pub(super) async fn user_after_register_and_login_both_fire_on_otp_signup() {
    // OTP signup creates a brand-new user — register + login
    // events should both fire from the same /otp/login call.
    let (mut state, _dir, _tok) = state_with_app_and_collection().await;
    let mail = install_capturing_mailer(&mut state);
    plant_hook_in_app(
        &state,
        "acme",
        "mobile",
        r#"
        $app.onUserAfterRegister((u) => $app.log("reg:" + u.email));
        $app.onUserAfterLogin((u)    => $app.log("log:" + u.email));
        "#,
    )
    .await;

    let app = build_router(state.clone());
    app.oneshot(req_with_auth(
        "POST",
        "/api/workspaces/acme/auth/otp/request",
        None,
        Some(&serde_json::json!({"email":"fresh@acme.com"})),
    ))
    .await
    .unwrap();
    let code = mail.last_code();

    let app = build_router(state.clone());
    let resp = app
        .oneshot(req_with_auth(
            "POST",
            "/api/workspaces/acme/auth/otp/login",
            None,
            Some(&serde_json::json!({"email":"fresh@acme.com","code":code})),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let mut logs = state
        .hooks
        .get("acme", "mobile")
        .unwrap()
        .drain_logs()
        .await
        .unwrap();
    logs.sort();
    assert_eq!(
        logs,
        vec![
            "log:fresh@acme.com".to_string(),
            "reg:fresh@acme.com".to_string(),
        ]
    );
}
