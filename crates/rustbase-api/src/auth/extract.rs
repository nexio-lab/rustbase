//! `MasterAdminAuth` axum extractor.
//!
//! Pulls a `Bearer` token from the `Authorization` header, decodes it
//! against the master signing key, asserts `role == MasterAdmin`, and
//! consults the revocation set. Use it as a handler argument:
//!
//! ```ignore
//! pub async fn list_realms(
//!     auth: MasterAdminAuth,
//!     State(state): State<AppState>,
//! ) -> Result<Json<Vec<Realm>>, ApiError> { ... }
//! ```

use axum::{extract::FromRequestParts, http::request::Parts};
use rustbase_auth::{Claims, SubjectKey, TokenRole, decode_token};
use rustbase_core::CoreError;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Clone)]
pub struct MasterAdminAuth {
    pub admin_id: String,
    pub claims: Claims,
}

impl FromRequestParts<AppState> for MasterAdminAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let header = parts
            .headers
            .get("authorization")
            .and_then(|h| h.to_str().ok())
            .ok_or(ApiError::Core(CoreError::Unauthorized))?;

        let token = header
            .strip_prefix("Bearer ")
            .ok_or(ApiError::Core(CoreError::Unauthorized))?;

        let claims = decode_token(token, &state.master_key)?;

        if !matches!(claims.role, TokenRole::MasterAdmin) {
            return Err(ApiError::Core(CoreError::Forbidden));
        }

        if state
            .revocations
            .is_revoked(&SubjectKey::master(&claims.sub), claims.iat)
        {
            return Err(ApiError::Core(CoreError::Unauthorized));
        }

        Ok(MasterAdminAuth {
            admin_id: claims.sub.clone(),
            claims,
        })
    }
}
