use crate::error::{AuthError, Result};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use rand_core::OsRng;

/// Hash a plain-text password with Argon2id and a fresh random salt.
/// Returns the PHC string, suitable to store directly.
pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AuthError::PasswordHash(e.to_string()))?;
    Ok(hash.to_string())
}

/// Verify a plain-text password against a stored PHC hash. Returns
/// `Ok(true)` on match, `Ok(false)` on mismatch, and an error only if the
/// stored hash is malformed.
pub fn verify_password(password: &str, stored_hash: &str) -> Result<bool> {
    let parsed =
        PasswordHash::new(stored_hash).map_err(|e| AuthError::PasswordHash(e.to_string()))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_round_trip() {
        let hash = hash_password("hunter2").unwrap();
        assert!(verify_password("hunter2", &hash).unwrap());
        assert!(!verify_password("wrong", &hash).unwrap());
    }

    #[test]
    fn salts_produce_distinct_hashes_for_same_password() {
        let h1 = hash_password("same").unwrap();
        let h2 = hash_password("same").unwrap();
        assert_ne!(h1, h2);
    }

    #[test]
    fn malformed_hash_is_an_error() {
        let err = verify_password("anything", "not-a-phc-string").unwrap_err();
        assert!(matches!(err, AuthError::PasswordHash(_)));
    }
}
