//! SQLite persistence layer for RustBase.
//!
//! Manages the system pool (`data/system.db`), per-realm pools
//! (`data/realms/<id>/realm.db`), and per-app pools
//! (`data/realms/<id>/apps/<id>/data.db`) with LRU eviction.
//!
//! Translates `rustbase_core::FilterNode` into parameterized SQL `WHERE`
//! clauses (no string interpolation of user input), runs scoped migrations,
//! and drives the auto-clamp engine when a parent tightens a policy bound.
