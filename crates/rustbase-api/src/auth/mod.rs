//! Auth handlers and extractors.
//!
//! - `login.rs` — `POST /_/auth/admin/login`
//! - `refresh.rs` — `POST /_/auth/refresh`
//! - `extract.rs` — `MasterAdminAuth` axum extractor
//!
//! Realm-admin and end-user flows come later, on their own feature branches.

pub mod extract;
pub mod login;
pub mod refresh;

pub use extract::MasterAdminAuth;
pub use login::master_admin_login;
pub use refresh::master_admin_refresh;

use rand_core::{OsRng, RngCore};

/// Generate an opaque refresh token (64 hex chars from 32 random bytes,
/// prefixed with `rfsh_` for greppability in logs).
pub fn new_refresh_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    use std::fmt::Write;
    let mut out = String::with_capacity(5 + 64);
    out.push_str("rfsh_");
    for b in &bytes {
        write!(&mut out, "{b:02x}").unwrap();
    }
    out
}

/// Default access-token TTL until the policy engine surfaces a
/// configurable value (15 min per the design spec).
pub fn default_access_ttl() -> chrono::Duration {
    chrono::Duration::minutes(15)
}

/// Default refresh-token TTL (30 days).
pub fn default_refresh_ttl() -> chrono::Duration {
    chrono::Duration::days(30)
}
