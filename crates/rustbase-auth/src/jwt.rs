//! Asymmetric (RS256) JWT issuance + verification with a JWKS export.
//!
//! Bootstrap layout — the server generates an RSA-2048 keypair once on
//! first boot, persists the PKCS#8 DER under `_secrets`, and rebuilds
//! the keyset from the stored DER on every subsequent start. Tokens are
//! always **issued** with RS256 + a `kid` header so external clients
//! can rotate against the JWKS endpoint without restarts.
//!
//! Verification accepts:
//!   1. The active RS256 key (matched by `kid` when present).
//!   2. A legacy HS256 `SigningKey` — kept transient during the
//!      RustBase 0.1.x → 0.2 transition so already-issued symmetric
//!      tokens keep validating until they expire on their own. New
//!      installs never touch the HS256 path.
//!
//! `Jwks` is the public surface: a JSON Web Key Set with one key
//! entry per accepted RS256 key. The `JwksKey` shape is the RFC 7517
//! representation `{kty, alg, use, kid, n, e}` — `n` and `e` are
//! base64url-encoded (no padding).

use crate::error::{AuthError, Result};
use crate::token::{Claims, SigningKey};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, decode_header, encode,
};
use rsa::pkcs1::{EncodeRsaPrivateKey, EncodeRsaPublicKey};
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// One RSA-2048 signing keypair. Held in memory at run time; persisted
/// as PKCS#8 DER in `system.db._secrets`.
#[derive(Clone)]
pub struct RsaSigningKey {
    /// Stable identifier — the leading 16 base64url characters of
    /// SHA-256 over the SubjectPublicKeyInfo (PKCS#1 RSA public key
    /// DER). The hash makes the kid deterministic across restarts.
    pub kid: String,
    /// `EncodingKey::from_rsa_der` over the PKCS#1 RSAPrivateKey DER.
    encoding: EncodingKey,
    /// `DecodingKey::from_rsa_raw_components(n, e)` over the public
    /// modulus / exponent.
    decoding: DecodingKey,
    /// Raw modulus / exponent kept around for JWKS export.
    n: Vec<u8>,
    e: Vec<u8>,
}

impl std::fmt::Debug for RsaSigningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RsaSigningKey")
            .field("kid", &self.kid)
            .finish_non_exhaustive()
    }
}

impl RsaSigningKey {
    /// Generate a fresh 2048-bit RSA keypair. ~200–500 ms on a typical
    /// dev box; only ever called on first boot.
    pub fn generate() -> Result<Self> {
        let mut rng = rsa::rand_core::OsRng;
        let private = RsaPrivateKey::new(&mut rng, 2048)
            .map_err(|e| AuthError::Internal(format!("rsa keygen failed: {e}")))?;
        Self::from_private(private)
    }

    /// Re-hydrate from PKCS#8 DER persisted at first boot.
    pub fn from_pkcs8_der(der: &[u8]) -> Result<Self> {
        let private = RsaPrivateKey::from_pkcs8_der(der)
            .map_err(|e| AuthError::Internal(format!("rsa pkcs8 parse failed: {e}")))?;
        Self::from_private(private)
    }

    fn from_private(private: RsaPrivateKey) -> Result<Self> {
        let public = RsaPublicKey::from(&private);

        let pkcs1_der = private
            .to_pkcs1_der()
            .map_err(|e| AuthError::Internal(format!("rsa pkcs1 export failed: {e}")))?;
        let encoding = EncodingKey::from_rsa_der(pkcs1_der.as_bytes());

        let n = public.n().to_bytes_be();
        let e = public.e().to_bytes_be();
        let decoding = DecodingKey::from_rsa_raw_components(&n, &e);

        let public_der = public
            .to_pkcs1_der()
            .map_err(|e| AuthError::Internal(format!("rsa pkcs1 public export failed: {e}")))?;
        let mut hasher = Sha256::new();
        hasher.update(public_der.as_bytes());
        let digest = hasher.finalize();
        let kid = URL_SAFE_NO_PAD.encode(digest)[..16].to_string();

        Ok(Self {
            kid,
            encoding,
            decoding,
            n,
            e,
        })
    }

    /// Public JWKS entry — `{kty, alg, use, kid, n, e}`. Used by the
    /// JWKS endpoint to publish the verification key.
    pub fn to_jwks_key(&self) -> JwksKey {
        JwksKey {
            kty: "RSA".into(),
            alg: "RS256".into(),
            r#use: "sig".into(),
            kid: self.kid.clone(),
            n: URL_SAFE_NO_PAD.encode(&self.n),
            e: URL_SAFE_NO_PAD.encode(&self.e),
        }
    }
}

/// `RsaSigningKey::generate` plus the PKCS#8 DER bytes ready for
/// persistence. Returned as a pair so callers never have to round-trip
/// the private key through DER themselves.
pub fn generate_rsa_with_pkcs8() -> Result<(RsaSigningKey, Vec<u8>)> {
    let mut rng = rsa::rand_core::OsRng;
    let private = RsaPrivateKey::new(&mut rng, 2048)
        .map_err(|e| AuthError::Internal(format!("rsa keygen failed: {e}")))?;
    let pkcs8 = private
        .to_pkcs8_der()
        .map_err(|e| AuthError::Internal(format!("rsa pkcs8 export failed: {e}")))?
        .as_bytes()
        .to_vec();
    let key = RsaSigningKey::from_private(private)?;
    Ok((key, pkcs8))
}

/// JWT issuer/verifier with one active RS256 key plus an optional
/// HS256 fallback used to keep already-issued legacy tokens valid
/// across a 0.1.x → 0.2 upgrade.
#[derive(Clone, Debug)]
pub struct JwtIssuer {
    pub active: RsaSigningKey,
    pub legacy_hmac: Option<SigningKey>,
}

impl JwtIssuer {
    pub fn new(active: RsaSigningKey) -> Self {
        Self {
            active,
            legacy_hmac: None,
        }
    }

    pub fn with_legacy_hmac(mut self, hmac: SigningKey) -> Self {
        self.legacy_hmac = Some(hmac);
        self
    }

    /// Sign `claims` with the active RS256 key and stamp the `kid`
    /// header so external verifiers can match against the JWKS.
    pub fn issue(&self, claims: &Claims) -> Result<String> {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.active.kid.clone());
        encode(&header, claims, &self.active.encoding).map_err(AuthError::from)
    }

    /// Verify and return the claims. Tries RS256 with the active
    /// `kid`; falls back to HS256 only if `legacy_hmac` is set AND the
    /// header's algorithm is `HS256`. Anything else is rejected.
    pub fn verify(&self, token: &str) -> Result<Claims> {
        let header = decode_header(token).map_err(|e| match e.kind() {
            jsonwebtoken::errors::ErrorKind::ExpiredSignature => AuthError::TokenExpired,
            _ => AuthError::Jwt(e),
        })?;
        match header.alg {
            Algorithm::RS256 => {
                if let Some(kid) = &header.kid
                    && kid != &self.active.kid
                {
                    return Err(AuthError::Internal(format!(
                        "token kid {kid:?} does not match active key"
                    )));
                }
                let validation = Validation::new(Algorithm::RS256);
                decode::<Claims>(token, &self.active.decoding, &validation)
                    .map(|d| d.claims)
                    .map_err(|e| match e.kind() {
                        jsonwebtoken::errors::ErrorKind::ExpiredSignature => {
                            AuthError::TokenExpired
                        }
                        _ => AuthError::Jwt(e),
                    })
            }
            Algorithm::HS256 => {
                let Some(legacy) = &self.legacy_hmac else {
                    return Err(AuthError::Internal(
                        "HS256 token rejected — no legacy HMAC key in this issuer".into(),
                    ));
                };
                let validation = Validation::new(Algorithm::HS256);
                decode::<Claims>(
                    token,
                    &DecodingKey::from_secret(legacy.as_bytes()),
                    &validation,
                )
                .map(|d| d.claims)
                .map_err(|e| match e.kind() {
                    jsonwebtoken::errors::ErrorKind::ExpiredSignature => AuthError::TokenExpired,
                    _ => AuthError::Jwt(e),
                })
            }
            other => Err(AuthError::Internal(format!(
                "token algorithm {other:?} not accepted"
            ))),
        }
    }

    /// Build the JWKS the server publishes. Contains the active RS256
    /// key only — the legacy HMAC secret is never exposed (HMAC keys
    /// can't be published in a JWKS without leaking the signing
    /// material).
    pub fn jwks(&self) -> Jwks {
        Jwks {
            keys: vec![self.active.to_jwks_key()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JwksKey {
    pub kty: String,
    pub alg: String,
    #[serde(rename = "use")]
    pub r#use: String,
    pub kid: String,
    pub n: String,
    pub e: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Jwks {
    pub keys: Vec<JwksKey>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::{TokenRole, build_claims};
    use chrono::Duration;

    fn fresh_issuer() -> JwtIssuer {
        let (key, _) = generate_rsa_with_pkcs8().unwrap();
        JwtIssuer::new(key)
    }

    #[test]
    fn rs256_round_trip() {
        let issuer = fresh_issuer();
        let claims = build_claims(
            "user-1",
            TokenRole::User,
            Some("acme".into()),
            Some("mobile".into()),
            Duration::minutes(15),
        );
        let token = issuer.issue(&claims).unwrap();
        let decoded = issuer.verify(&token).unwrap();
        assert_eq!(decoded.sub, "user-1");
        assert_eq!(decoded.role, TokenRole::User);
        assert_eq!(decoded.realm.as_deref(), Some("acme"));
    }

    #[test]
    fn rs256_token_header_carries_active_kid() {
        let issuer = fresh_issuer();
        let claims = build_claims("u", TokenRole::User, None, None, Duration::minutes(1));
        let token = issuer.issue(&claims).unwrap();
        let header = decode_header(&token).unwrap();
        assert_eq!(header.alg, Algorithm::RS256);
        assert_eq!(header.kid.as_deref(), Some(issuer.active.kid.as_str()));
    }

    #[test]
    fn tokens_from_other_issuer_are_rejected() {
        let issuer_a = fresh_issuer();
        let issuer_b = fresh_issuer();
        let claims = build_claims("u", TokenRole::User, None, None, Duration::minutes(1));
        let token = issuer_a.issue(&claims).unwrap();
        assert!(issuer_b.verify(&token).is_err());
    }

    #[test]
    fn jwks_round_trips_through_json() {
        let issuer = fresh_issuer();
        let jwks = issuer.jwks();
        let json = serde_json::to_string(&jwks).unwrap();
        let back: Jwks = serde_json::from_str(&json).unwrap();
        assert_eq!(back.keys.len(), 1);
        assert_eq!(back.keys[0].kty, "RSA");
        assert_eq!(back.keys[0].alg, "RS256");
        assert_eq!(back.keys[0].r#use, "sig");
        assert_eq!(back.keys[0].kid, issuer.active.kid);
        // n and e are base64url-no-pad.
        assert!(!back.keys[0].n.contains('='));
        assert!(!back.keys[0].e.contains('='));
    }

    #[test]
    fn pkcs8_round_trips() {
        let (k1, pkcs8) = generate_rsa_with_pkcs8().unwrap();
        let k2 = RsaSigningKey::from_pkcs8_der(&pkcs8).unwrap();
        assert_eq!(k1.kid, k2.kid);
        // Tokens issued by k1 verify under an issuer built from k2.
        let issuer1 = JwtIssuer::new(k1);
        let issuer2 = JwtIssuer::new(k2);
        let claims = build_claims("u", TokenRole::User, None, None, Duration::minutes(1));
        let t = issuer1.issue(&claims).unwrap();
        assert!(issuer2.verify(&t).is_ok());
    }

    #[test]
    fn legacy_hs256_token_validates_when_legacy_set() {
        let (rsa, _) = generate_rsa_with_pkcs8().unwrap();
        let hmac = SigningKey::generate();
        let issuer = JwtIssuer::new(rsa).with_legacy_hmac(hmac.clone());

        let claims = build_claims("u", TokenRole::User, None, None, Duration::minutes(1));
        let legacy_token = crate::token::encode_token(&claims, &hmac).unwrap();
        assert!(issuer.verify(&legacy_token).is_ok());
    }

    #[test]
    fn hs256_token_rejected_when_no_legacy_set() {
        let issuer = fresh_issuer();
        let hmac = SigningKey::generate();
        let claims = build_claims("u", TokenRole::User, None, None, Duration::minutes(1));
        let legacy_token = crate::token::encode_token(&claims, &hmac).unwrap();
        assert!(issuer.verify(&legacy_token).is_err());
    }

    #[test]
    fn expired_token_surfaces_as_token_expired() {
        let issuer = fresh_issuer();
        let claims = build_claims("u", TokenRole::User, None, None, Duration::seconds(-120));
        let token = issuer.issue(&claims).unwrap();
        let err = issuer.verify(&token).unwrap_err();
        assert!(matches!(err, AuthError::TokenExpired));
    }
}
