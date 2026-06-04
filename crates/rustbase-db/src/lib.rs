//! SQLite persistence layer for RustBase.
//!
//! Manages the system pool (`data/system.db`), per-workspace pools
//! (`data/workspaces/<id>/workspace.db`), and per-app pools
//! (`data/workspaces/<id>/apps/<id>/data.db`) with LRU eviction.
//!
//! Translates `rustbase_core::FilterNode` into parameterized SQL `WHERE`
//! fragments (no string interpolation of user input), runs scoped
//! migrations, and (in a later feature) will drive the auto-clamp engine
//! when a parent tightens a policy bound.

pub mod access_rules;
pub mod admins;
pub mod apps;
pub mod audit;
pub mod collections;
pub mod email_otps;
pub mod email_verifications;
pub mod error;
pub mod files;
pub mod filter_sql;
pub mod mfa_challenges;
pub mod migrations;
pub mod oauth_links;
pub mod oauth_providers;
pub mod oauth_states;
pub mod password_resets;
pub mod paths;
pub mod policies;
pub mod policy_engine;
pub mod pool;
pub mod records;
pub mod secrets;
pub mod tokens;
pub mod user_totp;
pub mod users;
pub mod workspaces;

pub use admins::{AppAdmin, MasterAdmin, WorkspaceAdmin};
pub use apps::App;
pub use collections::Collection;
pub use error::{DbError, Result};
pub use files::FileMeta;
pub use filter_sql::{SqlFragment, filter_to_sql};
pub use migrations::{
    APP_MIGRATIONS, Migration, MigrationScope, SYSTEM_MIGRATIONS, WORKSPACE_MIGRATIONS,
    apply_migrations,
};
pub use pool::{AppPoolManager, SystemPool, WorkspacePoolManager, open_memory_pool, open_pool};
pub use records::{ListPage, ListedRecords};
pub use tokens::{RefreshToken, SubjectKind};
pub use workspaces::Workspace;
