//! Auto-clamp cascade for hierarchical policies.
//!
//! When a parent scope tightens (or otherwise mutates) its bound for a
//! field, every child scope whose currently-stored value would now
//! violate that bound has its value clamped back into range. Each
//! clamp is written to the child's `audit_log`.
//!
//! Two cascades are exposed:
//!
//! - `cascade_master_to_realms_and_apps` — run after a master-scope
//!   policy `PUT`. Clamps every workspace's value to the new master
//!   bound, then for each workspace clamps every app's value to that
//!   workspace's (now-clamped) value.
//! - `cascade_realm_to_apps` — run after a workspace-scope policy `PUT`.
//!   Clamps every app in that workspace to the workspace's value.
//!
//! Both are idempotent: if a stored value already fits, nothing is
//! changed and nothing is logged.

use crate::error::Result;
use crate::{apps, audit, policies, pool::AppPoolManager, pool::WorkspacePoolManager, workspaces};
use rustbase_core::{AppId, MASTER_WORKSPACE_ID, PolicySpec, WorkspaceId};
use serde_json::json;
use sqlx::SqlitePool;
use std::sync::Arc;

/// One auto-clamp event for telemetry / API responses.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ClampOutcome {
    pub workspace: String,
    pub app: Option<String>,
    pub field: String,
    pub before: PolicySpec,
    pub after: PolicySpec,
}

pub async fn cascade_master_to_realms_and_apps(
    system_pool: &SqlitePool,
    workspace_pools: Arc<WorkspacePoolManager>,
    app_pools: Arc<AppPoolManager>,
    field: &str,
    new_master_spec: &PolicySpec,
    actor: Option<&str>,
) -> Result<Vec<ClampOutcome>> {
    let mut outcomes = Vec::new();
    let all_realms = workspaces::list_realms(system_pool).await?;
    for r in &all_realms {
        if r.id == MASTER_WORKSPACE_ID {
            continue;
        }
        let workspace_id = WorkspaceId::from(r.id.clone());
        let workspace_pool = workspace_pools.pool_for(&workspace_id).await?;

        // The bound that this workspace's *children* (apps) must satisfy
        // after this pass. If the workspace overrode the master value, use
        // that (post-clamp); otherwise the master value cascades.
        let mut workspace_current = new_master_spec.clone();

        if let Some(stored) = policies::get_policy(&workspace_pool, field).await? {
            if !new_master_spec.allows(&stored) {
                let after = new_master_spec.clamp(stored.clone());
                policies::upsert_policy(&workspace_pool, field, &after).await?;
                audit::append(
                    &workspace_pool,
                    actor,
                    "policy_clamped",
                    Some(field),
                    &json!({
                        "trigger": "master_tightened",
                        "before": stored,
                        "after": after,
                    }),
                )
                .await?;
                outcomes.push(ClampOutcome {
                    workspace: r.id.clone(),
                    app: None,
                    field: field.to_string(),
                    before: stored,
                    after: after.clone(),
                });
                workspace_current = after;
            } else {
                workspace_current = stored;
            }
        }

        // Now sweep the workspace's apps.
        let apps_in_realm = apps::list_apps(&workspace_pool).await?;
        for a in &apps_in_realm {
            let app_id = AppId::from(a.id.clone());
            let app_pool = app_pools.pool_for(&workspace_id, &app_id).await?;
            let Some(app_stored) = policies::get_policy(&app_pool, field).await? else {
                continue;
            };
            if workspace_current.allows(&app_stored) {
                continue;
            }
            let after = workspace_current.clamp(app_stored.clone());
            policies::upsert_policy(&app_pool, field, &after).await?;
            audit::append(
                &app_pool,
                actor,
                "policy_clamped",
                Some(field),
                &json!({
                    "trigger": "master_tightened",
                    "before": app_stored,
                    "after": after,
                }),
            )
            .await?;
            outcomes.push(ClampOutcome {
                workspace: r.id.clone(),
                app: Some(a.id.clone()),
                field: field.to_string(),
                before: app_stored,
                after,
            });
        }
    }
    Ok(outcomes)
}

pub async fn cascade_realm_to_apps(
    workspace_pool: &SqlitePool,
    app_pools: Arc<AppPoolManager>,
    workspace_id: &str,
    field: &str,
    new_realm_spec: &PolicySpec,
    actor: Option<&str>,
) -> Result<Vec<ClampOutcome>> {
    let mut outcomes = Vec::new();
    let workspace_ref = WorkspaceId::from(workspace_id.to_string());
    let apps_in_realm = apps::list_apps(workspace_pool).await?;
    for a in &apps_in_realm {
        let app_id = AppId::from(a.id.clone());
        let app_pool = app_pools.pool_for(&workspace_ref, &app_id).await?;
        let Some(stored) = policies::get_policy(&app_pool, field).await? else {
            continue;
        };
        if new_realm_spec.allows(&stored) {
            continue;
        }
        let after = new_realm_spec.clamp(stored.clone());
        policies::upsert_policy(&app_pool, field, &after).await?;
        audit::append(
            &app_pool,
            actor,
            "policy_clamped",
            Some(field),
            &json!({
                "trigger": "workspace_tightened",
                "before": stored,
                "after": after,
            }),
        )
        .await?;
        outcomes.push(ClampOutcome {
            workspace: workspace_id.to_string(),
            app: Some(a.id.clone()),
            field: field.to_string(),
            before: stored,
            after,
        });
    }
    Ok(outcomes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::create_app;
    use crate::migrations::{
        APP_MIGRATIONS, SYSTEM_MIGRATIONS, WORKSPACE_MIGRATIONS, apply_migrations,
    };
    use crate::pool::{AppPoolManager, WorkspacePoolManager, open_memory_pool};
    use crate::workspaces::{create_realm, ensure_master_realm};
    use rustbase_core::{RangePolicy, WorkspaceId};
    use tempfile::tempdir;

    fn r(min: i64, max: i64) -> PolicySpec {
        PolicySpec::Range(RangePolicy::new(min, max).unwrap())
    }

    /// Build a test universe: system + one workspace 'acme' + one app
    /// 'mobile'. Pool managers point at a tempdir.
    async fn universe() -> (
        SqlitePool,
        Arc<WorkspacePoolManager>,
        Arc<AppPoolManager>,
        tempfile::TempDir,
    ) {
        let dir = tempdir().unwrap();
        let system = open_memory_pool().await.unwrap();
        apply_migrations(system.clone(), SYSTEM_MIGRATIONS)
            .await
            .unwrap();
        ensure_master_realm(&system).await.unwrap();
        create_realm(&system, "acme", "Acme").await.unwrap();

        let workspace_pools = Arc::new(WorkspacePoolManager::new(dir.path().to_path_buf(), 4));
        let workspace_pool = workspace_pools
            .pool_for(&WorkspaceId::from("acme"))
            .await
            .unwrap();
        apply_migrations(workspace_pool.clone(), WORKSPACE_MIGRATIONS)
            .await
            .unwrap();
        create_app(&workspace_pool, "mobile", "Mobile")
            .await
            .unwrap();

        let app_pools = Arc::new(AppPoolManager::new(dir.path().to_path_buf(), 4));
        let app_pool = app_pools
            .pool_for(&WorkspaceId::from("acme"), &AppId::from("mobile"))
            .await
            .unwrap();
        apply_migrations(app_pool.clone(), APP_MIGRATIONS)
            .await
            .unwrap();

        (system, workspace_pools, app_pools, dir)
    }

    #[tokio::test]
    async fn master_tighten_clamps_realm_and_app() {
        let (system, workspace_pools, app_pools, _dir) = universe().await;
        let workspace_pool = workspace_pools
            .pool_for(&WorkspaceId::from("acme"))
            .await
            .unwrap();
        let app_pool = app_pools
            .pool_for(&WorkspaceId::from("acme"), &AppId::from("mobile"))
            .await
            .unwrap();

        // Initial state: master [4, 64], workspace [8, 32], app [12, 28].
        policies::upsert_policy(&system, "password.length", &r(4, 64))
            .await
            .unwrap();
        policies::upsert_policy(&workspace_pool, "password.length", &r(8, 32))
            .await
            .unwrap();
        policies::upsert_policy(&app_pool, "password.length", &r(12, 28))
            .await
            .unwrap();

        // Master tightens to [10, 20].
        let new_master = r(10, 20);
        policies::upsert_policy(&system, "password.length", &new_master)
            .await
            .unwrap();

        let outcomes = cascade_master_to_realms_and_apps(
            &system,
            workspace_pools.clone(),
            app_pools.clone(),
            "password.length",
            &new_master,
            Some("master-admin-1"),
        )
        .await
        .unwrap();

        // Both workspace and app should have been clamped.
        assert_eq!(outcomes.len(), 2);

        // Workspace went [8, 32] → [10, 20].
        let workspace_after = policies::get_policy(&workspace_pool, "password.length")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(workspace_after, r(10, 20));

        // App was [12, 28]; after master clamp the workspace is [10, 20],
        // and app [12, 28] is NOT inside [10, 20]; clamp to [12, 20].
        let app_after = policies::get_policy(&app_pool, "password.length")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(app_after, r(12, 20));

        // Audit entries written on both child DBs.
        let workspace_audit = audit::list_recent(&workspace_pool, 10).await.unwrap();
        assert!(workspace_audit.iter().any(|e| e.action == "policy_clamped"));
        let app_audit = audit::list_recent(&app_pool, 10).await.unwrap();
        assert!(app_audit.iter().any(|e| e.action == "policy_clamped"));
    }

    #[tokio::test]
    async fn master_loosen_is_a_noop() {
        let (system, workspace_pools, app_pools, _dir) = universe().await;
        let workspace_pool = workspace_pools
            .pool_for(&WorkspaceId::from("acme"))
            .await
            .unwrap();
        policies::upsert_policy(&system, "password.length", &r(4, 64))
            .await
            .unwrap();
        policies::upsert_policy(&workspace_pool, "password.length", &r(8, 32))
            .await
            .unwrap();

        // Master loosens to [2, 100]; workspace [8, 32] is still inside.
        let new_master = r(2, 100);
        let outcomes = cascade_master_to_realms_and_apps(
            &system,
            workspace_pools,
            app_pools,
            "password.length",
            &new_master,
            None,
        )
        .await
        .unwrap();
        assert!(outcomes.is_empty());
        // Audit log untouched.
        let workspace_audit = audit::list_recent(&workspace_pool, 10).await.unwrap();
        assert!(workspace_audit.is_empty());
    }

    #[tokio::test]
    async fn workspace_tighten_clamps_only_apps_under_it() {
        let (system, workspace_pools, app_pools, _dir) = universe().await;
        let workspace_pool = workspace_pools
            .pool_for(&WorkspaceId::from("acme"))
            .await
            .unwrap();
        let app_pool = app_pools
            .pool_for(&WorkspaceId::from("acme"), &AppId::from("mobile"))
            .await
            .unwrap();
        policies::upsert_policy(&app_pool, "password.length", &r(8, 32))
            .await
            .unwrap();

        // Workspace tightens to [12, 20]; existing app [8, 32] is outside.
        let new_realm = r(12, 20);
        policies::upsert_policy(&workspace_pool, "password.length", &new_realm)
            .await
            .unwrap();
        let outcomes = cascade_realm_to_apps(
            &workspace_pool,
            app_pools.clone(),
            "acme",
            "password.length",
            &new_realm,
            None,
        )
        .await
        .unwrap();
        assert_eq!(outcomes.len(), 1);
        let app_after = policies::get_policy(&app_pool, "password.length")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(app_after, r(12, 20));

        // System pool untouched.
        let _ = system; // keep alive
    }

    #[tokio::test]
    async fn child_without_a_value_is_left_alone() {
        let (system, workspace_pools, app_pools, _dir) = universe().await;
        // No workspace/app values stored. Master change → no clamps.
        let outcomes = cascade_master_to_realms_and_apps(
            &system,
            workspace_pools,
            app_pools,
            "password.length",
            &r(10, 20),
            None,
        )
        .await
        .unwrap();
        assert!(outcomes.is_empty());
    }
}
