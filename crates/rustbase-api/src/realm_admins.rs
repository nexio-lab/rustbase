//! Master-admin endpoints for managing realm admins.
//!
//! Realm admins live in their realm's `realm.db`. They can be created
//! by a master admin (typically right after creating a realm), and they
//! authenticate at `POST /api/realms/:realm/auth/admin/login`.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rustbase_auth::hash_password;
use rustbase_core::{CoreError, RealmId};
use rustbase_db::{
    admins::{find_realm_admin_by_email, insert_realm_admin},
    realms::find_realm,
};
use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::auth::AdminAuth;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize, Validate)]
pub struct CreateRealmAdminRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 8, max = 256))]
    pub password: String,
    #[validate(length(max = 100))]
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RealmAdminResponse {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// `POST /api/realms/:realm/admins` — master only.
pub async fn create(
    auth: AdminAuth,
    State(state): State<AppState>,
    Path(realm): Path<String>,
    Json(req): Json<CreateRealmAdminRequest>,
) -> Result<(StatusCode, Json<RealmAdminResponse>), ApiError> {
    auth.require_master()?;
    req.validate()
        .map_err(|e| ApiError::Core(CoreError::Validation(e.to_string())))?;

    // Verify the realm exists in system.db before touching its DB.
    find_realm(state.system.pool(), &realm)
        .await?
        .ok_or(ApiError::Core(CoreError::RealmNotFound(realm.clone())))?;

    let realm_id = RealmId::from(realm.clone());
    let pool = state.realms.pool_for(&realm_id).await?;

    if find_realm_admin_by_email(&pool, &req.email)
        .await?
        .is_some()
    {
        return Err(ApiError::Core(CoreError::Conflict(format!(
            "realm admin '{}' already exists in realm '{}'",
            req.email, realm
        ))));
    }

    let hash = hash_password(&req.password)?;
    let admin = insert_realm_admin(&pool, &req.email, &hash, req.name.as_deref()).await?;

    tracing::info!(realm = %realm, admin_id = %admin.id, email = %admin.email, "realm admin created");

    Ok((
        StatusCode::CREATED,
        Json(RealmAdminResponse {
            id: admin.id,
            email: admin.email,
            name: admin.name,
            created_at: admin.created_at,
        }),
    ))
}
