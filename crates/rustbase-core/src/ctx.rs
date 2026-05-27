use crate::id::{AdminId, AppId, RealmId, UserId};
use serde::{Deserialize, Serialize};

/// Identity of the principal making a request. Every API handler should
/// receive one of these and bypass it for nothing — there is no
/// "admin mode" that skips realm / app scoping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Principal {
    /// Master admin — full server access, scoped to the master realm.
    MasterAdmin { admin: AdminId },
    /// Realm admin — full access inside one realm.
    RealmAdmin { admin: AdminId, realm: RealmId },
    /// App admin — full access inside one app under one realm.
    AppAdmin {
        admin: AdminId,
        realm: RealmId,
        app: AppId,
    },
    /// End user authenticated against a realm; usable across all apps in
    /// that realm (subject to per-collection access rules).
    User { user: UserId, realm: RealmId },
    /// Unauthenticated.
    Guest,
}

impl Principal {
    pub fn realm(&self) -> Option<&RealmId> {
        match self {
            Principal::RealmAdmin { realm, .. } => Some(realm),
            Principal::AppAdmin { realm, .. } => Some(realm),
            Principal::User { realm, .. } => Some(realm),
            Principal::MasterAdmin { .. } | Principal::Guest => None,
        }
    }

    pub fn is_master(&self) -> bool {
        matches!(self, Principal::MasterAdmin { .. })
    }
}

/// Realm-scoped request context. Carried through realm-level endpoints
/// (admin management, OAuth config, etc.).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RealmCtx {
    pub realm: RealmId,
    pub principal: Principal,
}

/// App-scoped request context. Carried through every collection / record
/// endpoint and into every DB call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppCtx {
    pub realm: RealmId,
    pub app: AppId,
    pub principal: Principal,
}

impl AppCtx {
    pub fn realm_ctx(&self) -> RealmCtx {
        RealmCtx {
            realm: self.realm.clone(),
            principal: self.principal.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn principal_realm_lookup() {
        let p = Principal::User {
            user: UserId::from("u1"),
            realm: RealmId::from("acme"),
        };
        assert_eq!(p.realm(), Some(&RealmId::from("acme")));
        assert!(!p.is_master());
    }

    #[test]
    fn master_admin_has_no_realm_scope() {
        let p = Principal::MasterAdmin {
            admin: AdminId::from("a1"),
        };
        assert_eq!(p.realm(), None);
        assert!(p.is_master());
    }

    #[test]
    fn app_ctx_can_demote_to_realm_ctx() {
        let app = AppCtx {
            realm: RealmId::from("acme"),
            app: AppId::from("mobile"),
            principal: Principal::Guest,
        };
        let realm = app.realm_ctx();
        assert_eq!(realm.realm, RealmId::from("acme"));
        assert_eq!(realm.principal, Principal::Guest);
    }
}
