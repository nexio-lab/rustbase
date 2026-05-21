//! Core domain types for RustBase. This crate is IO-free.
//!
//! Houses the canonical `RealmId` / `AppId` / `Record` / `Schema` / `FilterNode`
//! types, the `nom`-based filter parser, the hierarchical `ConfigPolicy` model,
//! and the workspace-wide `CoreError` enum.

mod error;

pub use error::{CoreError, Result};
