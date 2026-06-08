//! `AdminAuth` axum extractor.
//!
//! Pulls a `Bearer` token from `Authorization`, decodes it against the
//! master signing key (we don't have per-workspace keys yet), consults the
//! revocation set, and exposes the resulting `Claims` to the handler.
//!
//! Authorization is the handler's responsibility — call the appropriate
//! `require_*` method to assert the principal has access to the scope.
//! This keeps the extractor cheap (it doesn't see path parameters) and
//! lets one endpoint serve multiple roles.

use axum::{extract::FromRequestParts, http::request::Parts};
use rustbase_auth::{Claims, SubjectKey, TokenRole};
use rustbase_core::CoreError;

use crate::error::ApiError;
use crate::state::AppState;

/// Decode the principal's access token into raw claims, no role
/// filtering. The token is taken from the `Authorization: Bearer …`
/// header (SDK clients, mobile apps, server-to-server), falling back
/// to the `rb_at` cookie set by the dashboard login flow.
fn extract_claims(parts: &Parts, state: &AppState) -> Result<Option<Claims>, ApiError> {
    let token = parts
        .headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer ").map(str::to_string))
        .or_else(|| super::cookies::read_cookie(&parts.headers, super::cookies::ACCESS_COOKIE));
    let Some(token) = token else {
        return Ok(None);
    };
    let claims = state.jwt.verify(&token)?;
    let key = match &claims.workspace {
        Some(r) => SubjectKey::scoped(r, &claims.sub),
        None => SubjectKey::master(&claims.sub),
    };
    if state.revocations.is_revoked(&key, claims.iat) {
        return Err(ApiError::Core(CoreError::Unauthorized));
    }
    Ok(Some(claims))
}

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

    /// Reject the request unless the principal can act on `workspace`.
    /// Master admins can act on every workspace; workspace admins only on the
    /// workspace in their token claim; app admins inherit workspace access from
    /// their app's workspace claim.
    pub fn require_workspace_access(&self, workspace: &str) -> Result<(), ApiError> {
        match self.claims.role {
            TokenRole::MasterAdmin => Ok(()),
            TokenRole::WorkspaceAdmin | TokenRole::AppAdmin => {
                if self.claims.workspace.as_deref() == Some(workspace) {
                    Ok(())
                } else {
                    Err(ApiError::Core(CoreError::Forbidden))
                }
            }
            TokenRole::User => Err(ApiError::Core(CoreError::Forbidden)),
        }
    }

    /// Reject the request unless the principal can act on `(workspace, app)`.
    /// Master admins always pass; workspace admins pass when the workspace
    /// matches; app admins must match both workspace and app.
    pub fn require_app_access(&self, workspace: &str, app: &str) -> Result<(), ApiError> {
        match self.claims.role {
            TokenRole::MasterAdmin => Ok(()),
            TokenRole::WorkspaceAdmin => {
                if self.claims.workspace.as_deref() == Some(workspace) {
                    Ok(())
                } else {
                    Err(ApiError::Core(CoreError::Forbidden))
                }
            }
            TokenRole::AppAdmin => {
                if self.claims.workspace.as_deref() == Some(workspace)
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
        let claims =
            extract_claims(parts, state)?.ok_or(ApiError::Core(CoreError::Unauthorized))?;
        if matches!(claims.role, TokenRole::User) {
            return Err(ApiError::Core(CoreError::Forbidden));
        }
        Ok(AdminAuth {
            admin_id: claims.sub.clone(),
            claims,
        })
    }
}

/// Permissive extractor: accepts any token (admin or user). Used by
/// endpoints whose authorization depends on the role + access rules,
/// not on a hard role match at the extractor layer.
#[derive(Debug, Clone)]
pub struct PrincipalAuth {
    pub subject_id: String,
    pub claims: Claims,
}

impl PrincipalAuth {
    pub fn is_admin_for_app(&self, workspace: &str, app: &str) -> bool {
        match self.claims.role {
            TokenRole::MasterAdmin => true,
            TokenRole::WorkspaceAdmin => self.claims.workspace.as_deref() == Some(workspace),
            TokenRole::AppAdmin => {
                self.claims.workspace.as_deref() == Some(workspace)
                    && self.claims.app.as_deref() == Some(app)
            }
            TokenRole::User => false,
        }
    }

    /// Workspace the (user) principal is bound to, or `None` if this
    /// is an admin principal. With workspace-shared identity, user
    /// tokens carry only a `workspace` claim — no `app`.
    pub fn user_workspace(&self) -> Option<&str> {
        match self.claims.role {
            TokenRole::User => self.claims.workspace.as_deref(),
            _ => None,
        }
    }

    /// Convenience: assert the (user) principal owns `workspace`.
    /// Returns `Forbidden` for admins (they don't match a user route)
    /// or for users in another workspace.
    pub fn require_user_in_workspace(&self, workspace: &str) -> Result<(), ApiError> {
        match self.user_workspace() {
            Some(w) if w == workspace => Ok(()),
            _ => Err(ApiError::Core(CoreError::Forbidden)),
        }
    }
}

impl FromRequestParts<AppState> for PrincipalAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let claims =
            extract_claims(parts, state)?.ok_or(ApiError::Core(CoreError::Unauthorized))?;
        Ok(PrincipalAuth {
            subject_id: claims.sub.clone(),
            claims,
        })
    }
}

/// Decode a token into `PrincipalAuth` directly, bypassing axum's
/// extractor chain. Used by the WebSocket handler — the browser
/// `WebSocket` constructor can't set request headers, so the SDK
/// passes the token as a `?token=` query string instead and the
/// handler resolves it here.
pub fn principal_from_token(state: &AppState, token: &str) -> Result<PrincipalAuth, ApiError> {
    let claims = state.jwt.verify(token)?;
    let key = match &claims.workspace {
        Some(r) => SubjectKey::scoped(r, &claims.sub),
        None => SubjectKey::master(&claims.sub),
    };
    if state.revocations.is_revoked(&key, claims.iat) {
        return Err(ApiError::Core(CoreError::Unauthorized));
    }
    Ok(PrincipalAuth {
        subject_id: claims.sub.clone(),
        claims,
    })
}
