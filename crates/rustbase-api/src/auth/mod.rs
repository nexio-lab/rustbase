//! Auth handlers and extractors.
//!
//! - `login.rs` — `POST /_/auth/admin/login`
//! - `refresh.rs` — `POST /_/auth/refresh`
//! - `extract.rs` — `MasterAdminAuth` axum extractor
//!
//! Realm-admin and end-user flows come later, on their own feature branches.

pub mod email_otp;
pub mod extract;
pub mod login;
pub mod oauth;
pub mod oauth_admin;
pub mod password_reset;
pub mod refresh;
pub mod register;
pub mod totp;
pub mod verify_email;

pub use extract::{AdminAuth, PrincipalAuth};
pub use login::{master_admin_login, realm_admin_login, user_login};
pub use refresh::{master_admin_refresh, realm_admin_refresh, user_refresh};
pub use register::user_register;

use rand_core::{OsRng, RngCore};

/// Generate an opaque refresh token (64 hex chars from 32 random bytes,
/// prefixed with `rfsh_` for greppability in logs).
pub fn new_refresh_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let mut out = String::with_capacity(5 + 64);
    out.push_str("rfsh_");
    for b in &bytes {
        out.push_str(&format!("{b:02x}"));
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
