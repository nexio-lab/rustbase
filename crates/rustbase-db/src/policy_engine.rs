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
//!   policy `PUT`. Clamps every realm's value to the new master
//!   bound, then for each realm clamps every app's value to that
//!   realm's (now-clamped) value.
//! - `cascade_realm_to_apps` — run after a realm-scope policy `PUT`.
//!   Clamps every app in that realm to the realm's value.
//!
//! Both are idempotent: if a stored value already fits, nothing is
//! changed and nothing is logged.

use crate::error::Result;
use crate::{apps, audit, policies, pool::AppPoolManager, pool::RealmPoolManager, realms};
use rustbase_core::{AppId, MASTER_REALM_ID, PolicySpec, RealmId};
use serde_json::json;
use sqlx::SqlitePool;
use std::sync::Arc;

/// One auto-clamp event for telemetry / API responses.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ClampOutcome {
    pub realm: String,
    pub app: Option<String>,
    pub field: String,
    pub before: PolicySpec,
    pub after: PolicySpec,
}

pub async fn cascade_master_to_realms_and_apps(
    system_pool: &SqlitePool,
    realm_pools: Arc<RealmPoolManager>,
    app_pools: Arc<AppPoolManager>,
    field: &str,
    new_master_spec: &PolicySpec,
    actor: Option<&str>,
) -> Result<Vec<ClampOutcome>> {
    let mut outcomes = Vec::new();
    let all_realms = realms::list_realms(system_pool).await?;
    for r in &all_realms {
        if r.id == MASTER_REALM_ID {
            continue;
        }
        let realm_id = RealmId::from(r.id.clone());
        let realm_pool = realm_pools.pool_for(&realm_id).await?;

        // The bound that this realm's *children* (apps) must satisfy
        // after this pass. If the realm overrode the master value, use
        // that (post-clamp); otherwise the master value cascades.
        let mut realm_current = new_master_spec.clone();

        if let Some(stored) = policies::get_policy(&realm_pool, field).await? {
            if !new_master_spec.allows(&stored) {
                let after = new_master_spec.clamp(stored.clone());
                policies::upsert_policy(&realm_pool, field, &after).await?;
                audit::append(
                    &realm_pool,
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
                    realm: r.id.clone(),
                    app: None,
                    field: field.to_string(),
                    before: stored,
                    after: after.clone(),
                });
                realm_current = after;
            } else {
                realm_current = stored;
            }
        }

        // Now sweep the realm's apps.
        let apps_in_realm = apps::list_apps(&realm_pool).await?;
        for a in &apps_in_realm {
            let app_id = AppId::from(a.id.clone());
            let app_pool = app_pools.pool_for(&realm_id, &app_id).await?;
            let Some(app_stored) = policies::get_policy(&app_pool, field).await? else {
                continue;
            };
            if realm_current.allows(&app_stored) {
                continue;
            }
            let after = realm_current.clamp(app_stored.clone());
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
                realm: r.id.clone(),
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
    realm_pool: &SqlitePool,
    app_pools: Arc<AppPoolManager>,
    realm_id: &str,
    field: &str,
    new_realm_spec: &PolicySpec,
    actor: Option<&str>,
) -> Result<Vec<ClampOutcome>> {
    let mut outcomes = Vec::new();
    let realm_ref = RealmId::from(realm_id.to_string());
    let apps_in_realm = apps::list_apps(realm_pool).await?;
    for a in &apps_in_realm {
        let app_id = AppId::from(a.id.clone());
        let app_pool = app_pools.pool_for(&realm_ref, &app_id).await?;
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
                "trigger": "realm_tightened",
                "before": stored,
                "after": after,
            }),
        )
        .await?;
        outcomes.push(ClampOutcome {
            realm: realm_id.to_string(),
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
        APP_MIGRATIONS, REALM_MIGRATIONS, SYSTEM_MIGRATIONS, apply_migrations,
    };
    use crate::pool::{AppPoolManager, RealmPoolManager, open_memory_pool};
    use crate::realms::{create_realm, ensure_master_realm};
    use rustbase_core::{RangePolicy, RealmId};
    use tempfile::tempdir;

    fn r(min: i64, max: i64) -> PolicySpec {
        PolicySpec::Range(RangePolicy::new(min, max).unwrap())
    }

    /// Build a test universe: system + one realm 'acme' + one app
    /// 'mobile'. Pool managers point at a tempdir.
    async fn universe() -> (
        SqlitePool,
        Arc<RealmPoolManager>,
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

        let realm_pools = Arc::new(RealmPoolManager::new(dir.path().to_path_buf(), 4));
        let realm_pool = realm_pools.pool_for(&RealmId::from("acme")).await.unwrap();
        apply_migrations(realm_pool.clone(), REALM_MIGRATIONS)
            .await
            .unwrap();
        create_app(&realm_pool, "mobile", "Mobile").await.unwrap();

        let app_pools = Arc::new(AppPoolManager::new(dir.path().to_path_buf(), 4));
        let app_pool = app_pools
            .pool_for(&RealmId::from("acme"), &AppId::from("mobile"))
            .await
            .unwrap();
        apply_migrations(app_pool.clone(), APP_MIGRATIONS)
            .await
            .unwrap();

        (system, realm_pools, app_pools, dir)
    }

    #[tokio::test]
    async fn master_tighten_clamps_realm_and_app() {
        let (system, realm_pools, app_pools, _dir) = universe().await;
        let realm_pool = realm_pools.pool_for(&RealmId::from("acme")).await.unwrap();
        let app_pool = app_pools
            .pool_for(&RealmId::from("acme"), &AppId::from("mobile"))
            .await
            .unwrap();

        // Initial state: master [4, 64], realm [8, 32], app [12, 28].
        policies::upsert_policy(&system, "password.length", &r(4, 64))
            .await
            .unwrap();
        policies::upsert_policy(&realm_pool, "password.length", &r(8, 32))
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
            realm_pools.clone(),
            app_pools.clone(),
            "password.length",
            &new_master,
            Some("master-admin-1"),
        )
        .await
        .unwrap();

        // Both realm and app should have been clamped.
        assert_eq!(outcomes.len(), 2);

        // Realm went [8, 32] → [10, 20].
        let realm_after = policies::get_policy(&realm_pool, "password.length")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(realm_after, r(10, 20));

        // App was [12, 28]; after master clamp the realm is [10, 20],
        // and app [12, 28] is NOT inside [10, 20]; clamp to [12, 20].
        let app_after = policies::get_policy(&app_pool, "password.length")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(app_after, r(12, 20));

        // Audit entries written on both child DBs.
        let realm_audit = audit::list_recent(&realm_pool, 10).await.unwrap();
        assert!(realm_audit.iter().any(|e| e.action == "policy_clamped"));
        let app_audit = audit::list_recent(&app_pool, 10).await.unwrap();
        assert!(app_audit.iter().any(|e| e.action == "policy_clamped"));
    }

    #[tokio::test]
    async fn master_loosen_is_a_noop() {
        let (system, realm_pools, app_pools, _dir) = universe().await;
        let realm_pool = realm_pools.pool_for(&RealmId::from("acme")).await.unwrap();
        policies::upsert_policy(&system, "password.length", &r(4, 64))
            .await
            .unwrap();
        policies::upsert_policy(&realm_pool, "password.length", &r(8, 32))
            .await
            .unwrap();

        // Master loosens to [2, 100]; realm [8, 32] is still inside.
        let new_master = r(2, 100);
        let outcomes = cascade_master_to_realms_and_apps(
            &system,
            realm_pools,
            app_pools,
            "password.length",
            &new_master,
            None,
        )
        .await
        .unwrap();
        assert!(outcomes.is_empty());
        // Audit log untouched.
        let realm_audit = audit::list_recent(&realm_pool, 10).await.unwrap();
        assert!(realm_audit.is_empty());
    }

    #[tokio::test]
    async fn realm_tighten_clamps_only_apps_under_it() {
        let (system, realm_pools, app_pools, _dir) = universe().await;
        let realm_pool = realm_pools.pool_for(&RealmId::from("acme")).await.unwrap();
        let app_pool = app_pools
            .pool_for(&RealmId::from("acme"), &AppId::from("mobile"))
            .await
            .unwrap();
        policies::upsert_policy(&app_pool, "password.length", &r(8, 32))
            .await
            .unwrap();

        // Realm tightens to [12, 20]; existing app [8, 32] is outside.
        let new_realm = r(12, 20);
        policies::upsert_policy(&realm_pool, "password.length", &new_realm)
            .await
            .unwrap();
        let outcomes = cascade_realm_to_apps(
            &realm_pool,
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
        let (system, realm_pools, app_pools, _dir) = universe().await;
        // No realm/app values stored. Master change → no clamps.
        let outcomes = cascade_master_to_realms_and_apps(
            &system,
            realm_pools,
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
