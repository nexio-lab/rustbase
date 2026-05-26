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

#[cfg(test)]
mod tests {
    use super::*;

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
