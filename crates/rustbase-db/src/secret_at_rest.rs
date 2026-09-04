//! Shape of a secret that has to be readable again.
//!
//! Most bearer secrets in this workspace are digests: a refresh token
//! or a reset link only ever needs comparing, never recovering. Two
//! do not fit that mould — a TOTP secret must be fed back into the
//! code generator, and a PKCE `code_verifier` must be replayed to the
//! provider — so they are encrypted under the KEK instead.
//!
//! Encryption needs a key, and a key is not always configured. Rather
//! than refuse the feature outright, the row records which of the two
//! protections applies. Nothing downstream has to guess, and a row
//! written under a key that later goes missing is an error instead of
//! a silent fall back to treating ciphertext as a secret.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StoredSecret {
    Clear(String),
    Encrypted(Vec<u8>),
}

impl StoredSecret {
    /// Split into the pair of column values: exactly one is `Some`.
    /// The schema's `CHECK` enforces the same invariant on disk.
    pub fn columns(&self) -> (Option<String>, Option<Vec<u8>>) {
        match self {
            StoredSecret::Clear(s) => (Some(s.clone()), None),
            StoredSecret::Encrypted(b) => (None, Some(b.clone())),
        }
    }

    /// Rebuild from the pair. `None` when neither or both are set,
    /// which the `CHECK` should already have made impossible.
    pub fn from_columns(clear: Option<String>, enc: Option<Vec<u8>>) -> Option<Self> {
        match (clear, enc) {
            (Some(s), None) => Some(StoredSecret::Clear(s)),
            (None, Some(b)) => Some(StoredSecret::Encrypted(b)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clear_secret_occupies_only_the_clear_column() {
        let (clear, enc) = StoredSecret::Clear("s".into()).columns();
        assert_eq!(clear.as_deref(), Some("s"));
        assert_eq!(enc, None);
    }

    #[test]
    fn an_encrypted_secret_occupies_only_the_cipher_column() {
        let (clear, enc) = StoredSecret::Encrypted(vec![7]).columns();
        assert_eq!(clear, None);
        assert_eq!(enc, Some(vec![7]));
    }

    #[test]
    fn both_columns_set_or_neither_is_rejected_rather_than_guessed() {
        assert_eq!(
            StoredSecret::from_columns(Some("s".into()), Some(vec![7])),
            None
        );
        assert_eq!(StoredSecret::from_columns(None, None), None);
    }

    #[test]
    fn a_round_trip_through_the_columns_preserves_the_secret() {
        for original in [
            StoredSecret::Clear("ABCDEF".into()),
            StoredSecret::Encrypted(vec![1, 2, 3]),
        ] {
            let (clear, enc) = original.columns();
            assert_eq!(StoredSecret::from_columns(clear, enc), Some(original));
        }
    }
}
