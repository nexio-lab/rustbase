//! Core domain types for RustBase. This crate is IO-free.
//!
//! Houses the canonical id newtypes, the `Record` and `Schema` types, the
//! `FilterNode` AST (parser lives alongside in the next layer), the
//! hierarchical configuration policy primitives, the request `Principal`
//! and `AppCtx` / `RealmCtx` carriers, and the workspace-wide `CoreError`
//! enum.

pub mod config;
pub mod ctx;
mod error;
pub mod filter;
pub mod id;
pub mod record;
pub mod schema;

pub use config::{EnumSetPolicy, PolicySpec, RangePolicy, TogglePolicy};
pub use ctx::{AppCtx, Principal, RealmCtx};
pub use error::{CoreError, Result};
pub use filter::FilterNode;
pub use id::{AdminId, AppId, CollectionId, MASTER_REALM_ID, RealmId, RecordId, UserId};
pub use record::Record;
pub use schema::{CollectionKind, Field, FieldType, Schema};
