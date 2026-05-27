use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rustbase_auth::hash_password;
use rustbase_core::{AppId, CoreError, RealmId};
use rustbase_db::users::{find_user_by_email, insert_user};
use serde::{Deserialize, Serialize};
use validator::Validate;

use super::require_app_exists;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize, Validate)]
pub struct RegisterRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 8, max = 256))]
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub id: String,
    pub email: String,
}

/// `POST /api/realms/:realm/apps/:app/auth/users/register` — self-service
/// end-user signup. The created user must still call `…/login` to receive
/// tokens. End-users live per-app: the same email can exist in two apps
/// of the same realm without colliding.
pub async fn user_register(
    State(state): State<AppState>,
    Path((realm, app)): Path<(String, String)>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<RegisterResponse>), ApiError> {
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;

    require_app_exists(&state, &realm, &app).await?;

    let realm_id = RealmId::from(realm.clone());
    let app_id = AppId::from(app.clone());
    let pool = state.apps.pool_for(&realm_id, &app_id).await?;

    if find_user_by_email(&pool, &req.email).await?.is_some() {
        return Err(ApiError::Core(CoreError::Conflict(format!(
            "email '{}' already registered in app '{}/{}'",
            req.email, realm, app
        ))));
    }

    let hash = hash_password(&req.password)?;
    let user = insert_user(&pool, &req.email, &hash).await?;

    tracing::info!(
        realm = %realm,
        app = %app,
        user_id = %user.id,
        email = %user.email,
        "user registered"
    );

    let public = serde_json::json!({
        "id": user.id,
        "email": user.email,
        "verified": user.verified,
    });
    let hook_req = rustbase_runtime::HookRequest::system(&realm, &app, "_user");
    if let Err(e) = state
        .hooks
        .dispatch_user_after_register(&realm, &app, &hook_req, &public)
        .await
    {
        tracing::warn!(error = %e, realm = %realm, app = %app, "user_after_register hook errored");
    }

    Ok((
        StatusCode::CREATED,
        Json(RegisterResponse {
            id: user.id,
            email: user.email,
        }),
    ))
}
