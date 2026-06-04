use crate::id::{AdminId, AppId, UserId, WorkspaceId};
use serde::{Deserialize, Serialize};

/// Identity of the principal making a request. Every API handler should
/// receive one of these and bypass it for nothing — there is no
/// "admin mode" that skips workspace / app scoping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Principal {
    /// Master admin — full server access, scoped to the master workspace.
    MasterAdmin { admin: AdminId },
    /// Workspace admin — full access inside one workspace.
    WorkspaceAdmin {
        admin: AdminId,
        workspace: WorkspaceId,
    },
    /// App admin — full access inside one app under one workspace.
    AppAdmin {
        admin: AdminId,
        workspace: WorkspaceId,
        app: AppId,
    },
    /// End user authenticated against a workspace; usable across all
    /// apps in that workspace (subject to per-collection access rules).
    User {
        user: UserId,
        workspace: WorkspaceId,
    },
    /// Unauthenticated.
    Guest,
}

impl Principal {
    pub fn workspace(&self) -> Option<&WorkspaceId> {
        match self {
            Principal::WorkspaceAdmin { workspace, .. } => Some(workspace),
            Principal::AppAdmin { workspace, .. } => Some(workspace),
            Principal::User { workspace, .. } => Some(workspace),
            Principal::MasterAdmin { .. } | Principal::Guest => None,
        }
    }

    pub fn is_master(&self) -> bool {
        matches!(self, Principal::MasterAdmin { .. })
    }
}

/// Workspace-scoped request context. Carried through workspace-level
/// endpoints (admin management, OAuth config, etc.).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceCtx {
    pub workspace: WorkspaceId,
    pub principal: Principal,
}

/// App-scoped request context. Carried through every collection / record
/// endpoint and into every DB call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppCtx {
    pub workspace: WorkspaceId,
    pub app: AppId,
    pub principal: Principal,
}

impl AppCtx {
    pub fn workspace_ctx(&self) -> WorkspaceCtx {
        WorkspaceCtx {
            workspace: self.workspace.clone(),
            principal: self.principal.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn principal_workspace_lookup() {
        let p = Principal::User {
            user: UserId::from("u1"),
            workspace: WorkspaceId::from("acme"),
        };
        assert_eq!(p.workspace(), Some(&WorkspaceId::from("acme")));
        assert!(!p.is_master());
    }

    #[test]
    fn master_admin_has_no_workspace_scope() {
        let p = Principal::MasterAdmin {
            admin: AdminId::from("a1"),
        };
        assert_eq!(p.workspace(), None);
        assert!(p.is_master());
    }

    #[test]
    fn app_ctx_can_demote_to_workspace_ctx() {
        let app = AppCtx {
            workspace: WorkspaceId::from("acme"),
            app: AppId::from("mobile"),
            principal: Principal::Guest,
        };
        let ws = app.workspace_ctx();
        assert_eq!(ws.workspace, WorkspaceId::from("acme"));
        assert_eq!(ws.principal, Principal::Guest);
    }
}
