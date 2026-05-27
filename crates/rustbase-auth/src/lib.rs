//! Authentication for RustBaas.
//!
//! Argon2id password hashing, HS256 JWT issuance / verification, and an
//! in-memory revocation set keyed by `SubjectKey`. Admin record storage
//! lives in `rustbase-db` (it needs sqlx); this crate provides only the
//! IO-free auth primitives.
//!
//! OAuth2 flows and OTP come in later feature branches.

pub mod error;
pub mod password;
pub mod revocation;
pub mod secret_box;
pub mod token;

pub use error::{AuthError, Result};
pub use password::{hash_password, verify_password};
pub use revocation::{RevocationSet, SubjectKey};
pub use secret_box::{SecretBoxError, decrypt, encrypt, fresh_kek};
pub use token::{Claims, SigningKey, TokenRole, build_claims, decode_token, encode_token};
