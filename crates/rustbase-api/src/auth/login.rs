use axum::{
    Json,
    extract::{Path, State},
};
use rustbase_auth::{TokenRole, build_claims, encode_token, verify_password};
use rustbase_core::{CoreError, RealmId};
use rustbase_db::{
    admins::{find_master_admin_by_email, find_realm_admin_by_email},
    tokens::{SubjectKind, insert_refresh_token},
};
use serde::{Deserialize, Serialize};
use validator::Validate;

use super::{default_access_ttl, default_refresh_ttl, new_refresh_token};
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize, Validate)]
pub struct LoginRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 1))]
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AdminPublic {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub admin: AdminPublic,
}

pub async fn master_admin_login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;

    let admin = find_master_admin_by_email(state.system.pool(), &req.email)
        .await?
        .ok_or(ApiError::Core(CoreError::Unauthorized))?;

    if !verify_password(&req.password, &admin.password_hash)? {
        return Err(ApiError::Core(CoreError::Unauthorized));
    }

    let claims = build_claims(
        admin.id.clone(),
        TokenRole::MasterAdmin,
        None,
        None,
        default_access_ttl(),
    );
    let access_token = encode_token(&claims, &state.master_key)?;

    let refresh = insert_refresh_token(
        state.system.pool(),
        &new_refresh_token(),
        SubjectKind::MasterAdmin,
        &admin.id,
        default_refresh_ttl(),
    )
    .await?;

    tracing::info!(admin_id = %admin.id, email = %admin.email, "master admin login");

    Ok(Json(LoginResponse {
        access_token,
        refresh_token: refresh.token,
        admin: AdminPublic {
            id: admin.id,
            email: admin.email,
            name: admin.name,
        },
    }))
}

pub async fn realm_admin_login(
    State(state): State<AppState>,
    Path(realm): Path<String>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;

    // Look up the realm's pool — if the realm doesn't exist, the row
    // will be missing in system.db. We don't surface that here; just
    // return Unauthorized so an attacker can't enumerate realms.
    let realm_id = RealmId::from(realm.clone());
    let pool = state.realms.pool_for(&realm_id).await?;

    let admin = find_realm_admin_by_email(&pool, &req.email)
        .await?
        .ok_or(ApiError::Core(CoreError::Unauthorized))?;

    if !verify_password(&req.password, &admin.password_hash)? {
        return Err(ApiError::Core(CoreError::Unauthorized));
    }

    let claims = build_claims(
        admin.id.clone(),
        TokenRole::RealmAdmin,
        Some(realm.clone()),
        None,
        default_access_ttl(),
    );
    let access_token = encode_token(&claims, &state.master_key)?;

    let refresh = insert_refresh_token(
        &pool,
        &new_refresh_token(),
        SubjectKind::RealmAdmin,
        &admin.id,
        default_refresh_ttl(),
    )
    .await?;

    tracing::info!(realm = %realm, admin_id = %admin.id, "realm admin login");

    Ok(Json(LoginResponse {
        access_token,
        refresh_token: refresh.token,
        admin: AdminPublic {
            id: admin.id,
            email: admin.email,
            name: admin.name,
        },
    }))
}
