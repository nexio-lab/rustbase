//! AES-256-GCM helper for at-rest encryption of small secrets.
//!
//! Used for OAuth client secrets so a database snapshot leak doesn't
//! also leak the upstream credentials. The key (KEK) lives in
//! `system.db._secrets` under `oauth_kek` — see `rustbase-server`'s
//! boot path for the generate-once-on-first-run dance.
//!
//! Wire format produced by [`encrypt`]:
//!
//! ```text
//! [12 bytes random nonce][N bytes ciphertext + 16-byte GCM tag]
//! ```
//!
//! The nonce is generated per call with the OS RNG; never reused for
//! a given key. The same 32-byte KEK is used for every encrypt call —
//! AES-GCM is safe to call up to ~2^32 times with the same key as
//! long as nonces are random, which fits any realistic OAuth-secret
//! workload comfortably.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand_core::{OsRng, RngCore};

#[derive(Debug, thiserror::Error)]
pub enum SecretBoxError {
    #[error("kek must be exactly 32 bytes (got {0})")]
    InvalidKekLen(usize),
    #[error("ciphertext too short to contain nonce + tag")]
    InvalidCiphertext,
    #[error("aes-gcm encrypt failed")]
    EncryptFailed,
    #[error("aes-gcm decrypt failed: wrong key or tampered ciphertext")]
    DecryptFailed,
    #[error("{0}")]
    BadKey(String),
}

const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

pub fn encrypt(plaintext: &[u8], kek: &[u8]) -> Result<Vec<u8>, SecretBoxError> {
    if kek.len() != KEY_LEN {
        return Err(SecretBoxError::InvalidKekLen(kek.len()));
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(kek));
    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| SecretBoxError::EncryptFailed)?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn decrypt(ciphertext: &[u8], kek: &[u8]) -> Result<Vec<u8>, SecretBoxError> {
    if kek.len() != KEY_LEN {
        return Err(SecretBoxError::InvalidKekLen(kek.len()));
    }
    if ciphertext.len() < NONCE_LEN + 16 {
        return Err(SecretBoxError::InvalidCiphertext);
    }
    let (nonce_bytes, body) = ciphertext.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(kek));
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, body)
        .map_err(|_| SecretBoxError::DecryptFailed)
}

/// Fresh 32-byte KEK from the OS RNG. Generated once at boot and
/// persisted via `rustbase-db::secrets`.
pub fn fresh_kek() -> [u8; KEY_LEN] {
    let mut k = [0u8; KEY_LEN];
    OsRng.fill_bytes(&mut k);
    k
}

/// Where the key-encryption key came from. The caller needs to know:
/// a legacy key is a nag, an environment key is the healthy state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KekSource {
    /// Read from `RUSTBASE_KEK`. Lives outside the data directory,
    /// which is the point.
    FromEnv([u8; KEY_LEN]),
    /// Read from `system.db._secrets`, i.e. the same file as the
    /// ciphertext it protects. Kept working so existing installs are
    /// not stranded; the caller should warn on every boot.
    FromDatabaseLegacy([u8; KEY_LEN]),
    /// No key anywhere: fresh install, variable unset. The server
    /// still boots — most deployments never touch OAuth — but any
    /// attempt to store an encrypted secret must be refused rather
    /// than served by a key generated into the data directory.
    Absent,
}

impl KekSource {
    pub fn key(&self) -> Option<[u8; KEY_LEN]> {
        match self {
            KekSource::FromEnv(k) | KekSource::FromDatabaseLegacy(k) => Some(*k),
            KekSource::Absent => None,
        }
    }
}

/// Decide which key-encryption key to use, given the environment
/// variable and whatever is already persisted.
///
/// Kept free of IO so the four cases can be exercised directly. The
/// refusal in the last case is deliberate: generating a key and
/// filing it beside the data it protects is the defect this exists to
/// remove, and doing it silently on a fresh install would carry the
/// defect forward forever.
pub fn resolve_kek(
    env_value: Option<&str>,
    stored: Option<Vec<u8>>,
) -> Result<KekSource, SecretBoxError> {
    if let Some(raw) = env_value {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(SecretBoxError::BadKey(
                "RUSTBASE_KEK is set but empty; unset it or give it 32 hex-encoded bytes".into(),
            ));
        }
        let bytes = decode_hex_key(raw)?;
        return Ok(KekSource::FromEnv(bytes));
    }
    if let Some(stored) = stored {
        let bytes: [u8; KEY_LEN] = stored.as_slice().try_into().map_err(|_| {
            SecretBoxError::BadKey("stored key-encryption key has the wrong length".into())
        })?;
        return Ok(KekSource::FromDatabaseLegacy(bytes));
    }
    Ok(KekSource::Absent)
}

fn decode_hex_key(raw: &str) -> Result<[u8; KEY_LEN], SecretBoxError> {
    if raw.len() != KEY_LEN * 2 {
        return Err(SecretBoxError::BadKey(format!(
            "RUSTBASE_KEK must be {} hex characters ({} bytes), got {}",
            KEY_LEN * 2,
            KEY_LEN,
            raw.len()
        )));
    }
    let mut out = [0u8; KEY_LEN];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&raw[i * 2..i * 2 + 2], 16)
            .map_err(|_| SecretBoxError::BadKey("RUSTBASE_KEK is not valid hex".into()))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex32(byte: u8) -> String {
        (0..32).map(|_| format!("{byte:02x}")).collect()
    }

    /// `RUSTBASE_KEK` wins whenever it is set: it is the whole point
    /// of the variable, keeping the key off the disk that holds the
    /// ciphertext.
    #[test]
    fn the_environment_key_takes_precedence_over_the_stored_one() {
        let stored = vec![0xAAu8; KEY_LEN];
        let got = resolve_kek(Some(&hex32(0xBB)), Some(stored)).unwrap();
        assert_eq!(got, KekSource::FromEnv([0xBB; KEY_LEN]));
    }

    /// An existing install already has ciphertext under the stored
    /// key. Refusing to boot would strand it, so the stored key keeps
    /// working — the caller is expected to say so loudly.
    #[test]
    fn an_existing_install_without_the_variable_keeps_its_stored_key() {
        let stored = vec![0xAAu8; KEY_LEN];
        let got = resolve_kek(None, Some(stored)).unwrap();
        assert_eq!(got, KekSource::FromDatabaseLegacy([0xAA; KEY_LEN]));
    }

    /// A fresh install still boots — most deployments never touch
    /// OAuth — but it must not silently generate a key and file it
    /// next to what it protects. No key is reported as no key; the
    /// refusal belongs at the point a secret would be stored.
    #[test]
    fn a_fresh_install_without_the_variable_reports_no_key() {
        assert_eq!(resolve_kek(None, None).unwrap(), KekSource::Absent);
        assert_eq!(resolve_kek(None, None).unwrap().key(), None);
    }

    #[test]
    fn a_malformed_environment_key_is_refused_not_silently_ignored() {
        assert!(resolve_kek(Some("nonsense"), None).is_err());
        assert!(resolve_kek(Some(&hex32(0xBB)[..40]), None).is_err());
        assert!(resolve_kek(Some(""), Some(vec![0xAA; KEY_LEN])).is_err());
    }

    #[test]
    fn encrypt_then_decrypt_round_trips() {
        let kek = fresh_kek();
        let pt = b"google-oauth-client-secret-xyz";
        let ct = encrypt(pt, &kek).unwrap();
        // Nonce + ciphertext + 16-byte tag. The exact length depends on
        // the plaintext size; just bound it below.
        assert!(ct.len() > pt.len() + NONCE_LEN);
        let out = decrypt(&ct, &kek).unwrap();
        assert_eq!(out, pt);
    }

    #[test]
    fn each_encrypt_uses_a_fresh_nonce() {
        // Two encryptions of the same plaintext with the same key must
        // produce different ciphertexts (random nonce).
        let kek = fresh_kek();
        let a = encrypt(b"same", &kek).unwrap();
        let b = encrypt(b"same", &kek).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn wrong_key_fails_to_decrypt() {
        let kek_a = fresh_kek();
        let kek_b = fresh_kek();
        let ct = encrypt(b"hello", &kek_a).unwrap();
        let err = decrypt(&ct, &kek_b).unwrap_err();
        assert!(matches!(err, SecretBoxError::DecryptFailed));
    }

    #[test]
    fn tampered_ciphertext_fails_to_decrypt() {
        let kek = fresh_kek();
        let mut ct = encrypt(b"hello", &kek).unwrap();
        // Flip a bit deep in the ciphertext (past the nonce).
        let last = ct.len() - 1;
        ct[last] ^= 0x01;
        assert!(matches!(
            decrypt(&ct, &kek),
            Err(SecretBoxError::DecryptFailed)
        ));
    }

    #[test]
    fn rejects_wrong_kek_length() {
        let too_short = vec![0u8; 16];
        assert!(matches!(
            encrypt(b"x", &too_short),
            Err(SecretBoxError::InvalidKekLen(16))
        ));
    }

    #[test]
    fn rejects_too_short_ciphertext() {
        let kek = fresh_kek();
        assert!(matches!(
            decrypt(&[0u8; 10], &kek),
            Err(SecretBoxError::InvalidCiphertext)
        ));
    }
}
