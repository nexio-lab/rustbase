//! Authentication for RustBase.
//!
//! Owns the master / realm / app admin model, end-user authentication, JWT
//! token issuance and verification, argon2 password hashing, OAuth2 flows,
//! and TOTP / email OTP. Revocation is tracked in an in-memory set that
//! auto-expires on the access-token TTL.
