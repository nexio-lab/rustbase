//! Integration tests for the HTTP router, organised by topic.
//!
//! Production router code lives in `super` (`router.rs`). Shared
//! helpers + a small set of bootstrap-flow tests live in `common`;
//! everything else is grouped by feature area. Each submodule
//! re-exports its helpers so any sibling can `use super::*;` and
//! get every fixture in one shot.

pub mod admin_users;
pub mod audit;
pub mod auth_flow;
pub mod collections_records;
pub mod common;
pub mod custom_routes;
pub mod email_otp;
pub mod email_verification;
pub mod end_user_access_rules;
pub mod files;
pub mod hooks_crud;
pub mod oauth_admin;
pub mod oauth_sign_in;
pub mod password_reset;
pub mod policy_engine;
pub mod totp;
pub mod user_lifecycle_hooks;
pub mod workspace_admin_app_crud;
pub mod workspace_crud;

#[allow(unused_imports)]
use admin_users::*;
#[allow(unused_imports)]
use audit::*;
#[allow(unused_imports)]
use auth_flow::*;
#[allow(unused_imports)]
use collections_records::*;
#[allow(unused_imports)]
use common::*;
#[allow(unused_imports)]
use custom_routes::*;
#[allow(unused_imports)]
use email_otp::*;
#[allow(unused_imports)]
use email_verification::*;
#[allow(unused_imports)]
use end_user_access_rules::*;
#[allow(unused_imports)]
use files::*;
#[allow(unused_imports)]
use hooks_crud::*;
#[allow(unused_imports)]
use oauth_admin::*;
#[allow(unused_imports)]
use oauth_sign_in::*;
#[allow(unused_imports)]
use password_reset::*;
#[allow(unused_imports)]
use policy_engine::*;
#[allow(unused_imports)]
use totp::*;
#[allow(unused_imports)]
use user_lifecycle_hooks::*;
#[allow(unused_imports)]
use workspace_admin_app_crud::*;
#[allow(unused_imports)]
use workspace_crud::*;
