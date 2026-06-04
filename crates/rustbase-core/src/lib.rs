//! Core domain types for RustBase. This crate is IO-free.
//!
//! Houses the canonical id newtypes, the `Record` and `Schema` types, the
//! `FilterNode` AST (parser lives alongside in the next layer), the
//! hierarchical configuration policy primitives, the request `Principal`
//! and `AppCtx` / `WorkspaceCtx` carriers, and the project-wide
//! `CoreError` enum.

pub mod config;
pub mod ctx;
mod error;
pub mod filter;
pub mod filter_parser;
pub mod id;
pub mod mailer;
pub mod record;
pub mod rule_template;
pub mod schema;

pub use config::{
    EnumSetPolicy, PolicyChange, PolicyLevel, PolicySpec, RangePolicy, TogglePolicy, cascade_clamp,
    validate_chain,
};
pub use ctx::{AppCtx, Principal, WorkspaceCtx};
pub use error::{CoreError, Result};
pub use filter::FilterNode;
pub use filter_parser::parse_filter;
pub use id::{AdminId, AppId, CollectionId, MASTER_WORKSPACE_ID, RecordId, UserId, WorkspaceId};
pub use mailer::{EmailMessage, Mailer, MailerError};
pub use record::Record;
pub use rule_template::{RuleContext, substitute as substitute_rule_template};
pub use schema::{CollectionKind, Field, FieldType, Schema};
