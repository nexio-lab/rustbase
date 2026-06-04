use crate::error::{AuthError, Result};
use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// JWT claims carried by every RustBase access token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    /// Subject id — `UserId` or `AdminId` depending on `role`.
    pub sub: String,
    /// What kind of principal this token represents.
    pub role: TokenRole,
    /// Workspace scope. Unset only for master-admin tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// App scope. Optional even for app-level operations: a workspace-scoped
    /// user token without an `app` claim can address any app under
    /// `workspace`, subject to per-collection access rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// Issued-at (unix seconds).
    pub iat: i64,
    /// Expires-at (unix seconds).
    pub exp: i64,
    /// Token id. Unique per token; used for revocation auditing.
    pub jti: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenRole {
    MasterAdmin,
    WorkspaceAdmin,
    AppAdmin,
    User,
}

/// Symmetric signing key for HS256. Per workspace, in the production
/// configuration; the master workspace has its own. RS256 support comes
/// later when we wire OAuth.
#[derive(Debug, Clone)]
pub struct SigningKey {
    bytes: Vec<u8>,
}

impl SigningKey {
    pub fn from_secret(secret: &[u8]) -> Self {
        Self {
            bytes: secret.to_vec(),
        }
    }

    /// Generate a fresh 32-byte key from the OS RNG.
    pub fn generate() -> Self {
        use rand_core::{OsRng, RngCore};
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        Self {
            bytes: key.to_vec(),
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Build a `Claims` envelope with a fresh `jti` and an exp derived from
/// `ttl` relative to `Utc::now()`.
pub fn build_claims(
    subject: impl Into<String>,
    role: TokenRole,
    workspace: Option<String>,
    app: Option<String>,
    ttl: Duration,
) -> Claims {
    let now = Utc::now();
    let exp = now + ttl;
    Claims {
        sub: subject.into(),
        role,
        workspace,
        app,
        iat: now.timestamp(),
        exp: exp.timestamp(),
        jti: Uuid::new_v4().to_string(),
    }
}

/// Sign `claims` and return the compact JWT string.
pub fn encode_token(claims: &Claims, key: &SigningKey) -> Result<String> {
    encode(
        &Header::new(Algorithm::HS256),
        claims,
        &EncodingKey::from_secret(key.as_bytes()),
    )
    .map_err(AuthError::from)
}

/// Verify the signature, check `exp`, and return the claims.
pub fn decode_token(token: &str, key: &SigningKey) -> Result<Claims> {
    let validation = Validation::new(Algorithm::HS256);
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(key.as_bytes()),
        &validation,
    )
    .map_err(|e| match e.kind() {
        jsonwebtoken::errors::ErrorKind::ExpiredSignature => AuthError::TokenExpired,
        _ => AuthError::Jwt(e),
    })?;
    Ok(data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_then_verify_round_trip() {
        let key = SigningKey::generate();
        let claims = build_claims(
            "user-1",
            TokenRole::User,
            Some("acme".into()),
            Some("mobile".into()),
            Duration::minutes(15),
        );
        let token = encode_token(&claims, &key).unwrap();
        let decoded = decode_token(&token, &key).unwrap();
        assert_eq!(decoded, claims);
    }

    #[test]
    fn wrong_key_rejects_token() {
        let key1 = SigningKey::generate();
        let key2 = SigningKey::generate();
        let claims = build_claims("u", TokenRole::User, None, None, Duration::minutes(15));
        let token = encode_token(&claims, &key1).unwrap();
        assert!(decode_token(&token, &key2).is_err());
    }

    #[test]
    fn expired_token_is_detected() {
        // jsonwebtoken's default Validation has 60s leeway; use -120s to clear it.
        let key = SigningKey::generate();
        let claims = build_claims("u", TokenRole::User, None, None, Duration::seconds(-120));
        let token = encode_token(&claims, &key).unwrap();
        let err = decode_token(&token, &key).unwrap_err();
        assert!(matches!(err, AuthError::TokenExpired));
    }

    #[test]
    fn master_admin_token_has_no_realm_claim() {
        let key = SigningKey::generate();
        let claims = build_claims(
            "admin-1",
            TokenRole::MasterAdmin,
            None,
            None,
            Duration::minutes(15),
        );
        let token = encode_token(&claims, &key).unwrap();
        let decoded = decode_token(&token, &key).unwrap();
        assert_eq!(decoded.workspace, None);
        assert_eq!(decoded.app, None);
        assert_eq!(decoded.role, TokenRole::MasterAdmin);
    }
}
