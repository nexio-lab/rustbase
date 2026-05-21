//! SQLite persistence layer for RustBase.
//!
//! Manages the system pool (`data/system.db`), per-realm pools
//! (`data/realms/<id>/realm.db`), and per-app pools
//! (`data/realms/<id>/apps/<id>/data.db`) with LRU eviction.
//!
//! Translates `rustbase_core::FilterNode` into parameterized SQL `WHERE`
//! fragments (no string interpolation of user input), runs scoped
//! migrations, and (in a later feature) will drive the auto-clamp engine
//! when a parent tightens a policy bound.

pub mod admins;
pub mod apps;
pub mod error;
pub mod filter_sql;
pub mod migrations;
pub mod paths;
pub mod pool;
pub mod realms;
pub mod secrets;
pub mod tokens;

pub use admins::{AppAdmin, MasterAdmin, RealmAdmin};
pub use apps::App;
pub use realms::Realm;
pub use tokens::{RefreshToken, SubjectKind};
pub use error::{DbError, Result};
pub use filter_sql::{SqlFragment, filter_to_sql};
pub use migrations::{
    APP_MIGRATIONS, Migration, MigrationScope, REALM_MIGRATIONS, SYSTEM_MIGRATIONS,
    apply_migrations,
};
pub use pool::{AppPoolManager, RealmPoolManager, SystemPool, open_memory_pool, open_pool};
