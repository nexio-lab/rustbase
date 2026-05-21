//! `AdminAuth` axum extractor.
//!
//! Pulls a `Bearer` token from `Authorization`, decodes it against the
//! master signing key (we don't have per-realm keys yet), consults the
//! revocation set, and exposes the resulting `Claims` to the handler.
//!
//! Authorization is the handler's responsibility — call the appropriate
//! `require_*` method to assert the principal has access to the scope.
//! This keeps the extractor cheap (it doesn't see path parameters) and
//! lets one endpoint serve multiple roles.

use axum::{extract::FromRequestParts, http::request::Parts};
use rustbase_auth::{Claims, SubjectKey, TokenRole, decode_token};
use rustbase_core::CoreError;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Clone)]
pub struct AdminAuth {
    pub admin_id: String,
    pub claims: Claims,
}

impl AdminAuth {
    /// Reject the request unless the token belongs to a master admin.
    pub fn require_master(&self) -> Result<(), ApiError> {
        if matches!(self.claims.role, TokenRole::MasterAdmin) {
            Ok(())
        } else {
            Err(ApiError::Core(CoreError::Forbidden))
        }
    }

    /// Reject the request unless the principal can act on `realm`.
    /// Master admins can act on every realm; realm admins only on the
    /// realm in their token claim; app admins inherit realm access from
    /// their app's realm claim.
    pub fn require_realm_access(&self, realm: &str) -> Result<(), ApiError> {
        match self.claims.role {
            TokenRole::MasterAdmin => Ok(()),
            TokenRole::RealmAdmin | TokenRole::AppAdmin => {
                if self.claims.realm.as_deref() == Some(realm) {
                    Ok(())
                } else {
                    Err(ApiError::Core(CoreError::Forbidden))
                }
            }
            TokenRole::User => Err(ApiError::Core(CoreError::Forbidden)),
        }
    }

    /// Reject the request unless the principal can act on `(realm, app)`.
    /// Master admins always pass; realm admins pass when the realm
    /// matches; app admins must match both realm and app.
    pub fn require_app_access(&self, realm: &str, app: &str) -> Result<(), ApiError> {
        match self.claims.role {
            TokenRole::MasterAdmin => Ok(()),
            TokenRole::RealmAdmin => {
                if self.claims.realm.as_deref() == Some(realm) {
                    Ok(())
                } else {
                    Err(ApiError::Core(CoreError::Forbidden))
                }
            }
            TokenRole::AppAdmin => {
                if self.claims.realm.as_deref() == Some(realm)
                    && self.claims.app.as_deref() == Some(app)
                {
                    Ok(())
                } else {
                    Err(ApiError::Core(CoreError::Forbidden))
                }
            }
            TokenRole::User => Err(ApiError::Core(CoreError::Forbidden)),
        }
    }
}

impl FromRequestParts<AppState> for AdminAuth {
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

        if matches!(claims.role, TokenRole::User) {
            return Err(ApiError::Core(CoreError::Forbidden));
        }

        let subject_key = match &claims.realm {
            Some(r) => SubjectKey::scoped(r, &claims.sub),
            None => SubjectKey::master(&claims.sub),
        };
        if state.revocations.is_revoked(&subject_key, claims.iat) {
            return Err(ApiError::Core(CoreError::Unauthorized));
        }

        Ok(AdminAuth {
            admin_id: claims.sub.clone(),
            claims,
        })
    }
}
