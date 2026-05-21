//! Per-collection per-action access rules.
//!
//! A rule is an optional filter expression stored as a TEXT column in
//! `_access_rules`. Semantics:
//!
//! - **No row** for a `(collection, action)` pair → admin-only.
//! - **`filter = NULL`** → admin-only (explicit lock).
//! - **`filter = ""`** or **`filter = "true"`** → any authenticated user
//!   of the realm.
//! - **Other filter expressions** → evaluated per request after the
//!   template substitution layer in `rustbase_core::rule_template`
//!   resolves `{{request.auth.id}}` etc. The resulting filter is ANDed
//!   into the records query.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessAction {
    List,
    View,
    Create,
    Update,
    Delete,
}

impl AccessAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            AccessAction::List => "list",
            AccessAction::View => "view",
            AccessAction::Create => "create",
            AccessAction::Update => "update",
            AccessAction::Delete => "delete",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "list" => Self::List,
            "view" => Self::View,
            "create" => Self::Create,
            "update" => Self::Update,
            "delete" => Self::Delete,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessRule {
    pub collection: String,
    pub action: AccessAction,
    pub filter: Option<String>,
}

pub async fn get_rule(
    pool: &SqlitePool,
    collection: &str,
    action: AccessAction,
) -> Result<Option<Option<String>>> {
    // The outer Option means "row exists in _access_rules"; the inner
    // Option carries the NULL-or-not state of the `filter` column.
    let row: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT filter FROM _access_rules WHERE collection_id = ? AND action = ?",
    )
    .bind(collection)
    .bind(action.as_str())
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(f,)| f))
}

pub async fn set_rule(
    pool: &SqlitePool,
    collection: &str,
    action: AccessAction,
    filter: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO _access_rules (collection_id, action, filter) VALUES (?, ?, ?) \
         ON CONFLICT(collection_id, action) DO UPDATE SET filter = excluded.filter",
    )
    .bind(collection)
    .bind(action.as_str())
    .bind(filter)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_rules(pool: &SqlitePool, collection: &str) -> Result<Vec<AccessRule>> {
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT action, filter FROM _access_rules WHERE collection_id = ? ORDER BY action",
    )
    .bind(collection)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(action, filter)| {
            AccessAction::from_str(&action).map(|action| AccessRule {
                collection: collection.to_string(),
                action,
                filter,
            })
        })
        .collect())
}

/// Three-way decision for what a stored rule permits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleDecision {
    /// Admin-only (no row, or filter = NULL).
    Deny,
    /// Any authenticated user of the realm (filter = "" or "true").
    Allow,
    /// Evaluate the filter template against the request context.
    Evaluate(String),
}

pub fn classify_rule(rule: &Option<Option<String>>) -> RuleDecision {
    match rule {
        None => RuleDecision::Deny,
        Some(None) => RuleDecision::Deny,
        Some(Some(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("true") {
                RuleDecision::Allow
            } else {
                RuleDecision::Evaluate(s.clone())
            }
        }
    }
}

/// Backwards-compatible helper. Returns `true` only for the open-rule
/// forms; template rules require the per-request evaluator.
pub fn rule_allows_user(rule: &Option<Option<String>>) -> bool {
    matches!(classify_rule(rule), RuleDecision::Allow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::{APP_MIGRATIONS, apply_migrations};
    use crate::pool::open_memory_pool;

    async fn fresh_pool() -> SqlitePool {
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(pool.clone(), APP_MIGRATIONS).await.unwrap();
        // _access_rules has a FK to _collections; insert a stub.
        sqlx::query(
            "INSERT INTO _collections (id, name, kind, schema_json, created_at, updated_at) \
             VALUES (?, ?, 'base', '{}', ?, ?)",
        )
        .bind("notes")
        .bind("notes")
        .bind(chrono::Utc::now())
        .bind(chrono::Utc::now())
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn missing_row_returns_none() {
        let pool = fresh_pool().await;
        assert!(get_rule(&pool, "notes", AccessAction::List)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn set_then_get_round_trips_open_rule() {
        let pool = fresh_pool().await;
        set_rule(&pool, "notes", AccessAction::List, Some("")).await.unwrap();
        let r = get_rule(&pool, "notes", AccessAction::List)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r, Some(String::new()));
        assert!(rule_allows_user(&Some(r)));
    }

    #[tokio::test]
    async fn null_filter_locks_to_admins() {
        let pool = fresh_pool().await;
        set_rule(&pool, "notes", AccessAction::List, None).await.unwrap();
        let r = get_rule(&pool, "notes", AccessAction::List).await.unwrap();
        // row exists with NULL filter
        assert_eq!(r, Some(None));
        assert!(!rule_allows_user(&r));
    }

    #[tokio::test]
    async fn unsupported_filter_denies_until_substitution_lands() {
        let r = Some(Some("owner = @request.auth.id".to_string()));
        assert!(!rule_allows_user(&r));
    }

    #[tokio::test]
    async fn list_rules_returns_all_actions() {
        let pool = fresh_pool().await;
        set_rule(&pool, "notes", AccessAction::List, Some("")).await.unwrap();
        set_rule(&pool, "notes", AccessAction::Create, Some("true")).await.unwrap();
        let rules = list_rules(&pool, "notes").await.unwrap();
        assert_eq!(rules.len(), 2);
    }
}
