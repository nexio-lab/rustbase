use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rustbase_auth::hash_password;
use rustbase_core::{CoreError, RealmId};
use rustbase_db::{
    realms::find_realm,
    users::{find_user_by_email, insert_user},
};
use serde::{Deserialize, Serialize};
use validator::Validate;

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

/// `POST /api/realms/:realm/auth/users/register` — self-service end-user
/// signup. The created user must still call `…/login` to receive tokens.
pub async fn user_register(
    State(state): State<AppState>,
    Path(realm): Path<String>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<RegisterResponse>), ApiError> {
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;

    find_realm(state.system.pool(), &realm)
        .await?
        .ok_or(ApiError::Core(CoreError::RealmNotFound(realm.clone())))?;

    let pool = state.realms.pool_for(&RealmId::from(realm.clone())).await?;

    if find_user_by_email(&pool, &req.email).await?.is_some() {
        return Err(ApiError::Core(CoreError::Conflict(format!(
            "email '{}' already registered in realm '{}'",
            req.email, realm
        ))));
    }

    let hash = hash_password(&req.password)?;
    let user = insert_user(&pool, &req.email, &hash).await?;

    tracing::info!(realm = %realm, user_id = %user.id, email = %user.email, "user registered");

    Ok((
        StatusCode::CREATED,
        Json(RegisterResponse {
            id: user.id,
            email: user.email,
        }),
    ))
}
