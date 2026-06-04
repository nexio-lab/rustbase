//! Layout of `data/` on disk and helpers to compute the canonical paths.
//!
//! ```text
//! <data_dir>/
//!   system.db
//!   workspaces/
//!     <workspace_id>/
//!       workspace.db
//!       storage/
//!       apps/
//!         <app_id>/
//!           data.db
//!           storage/
//! ```

use rustbase_core::{AppId, WorkspaceId};
use std::path::{Path, PathBuf};

pub fn system_db(data_dir: &Path) -> PathBuf {
    data_dir.join("system.db")
}

pub fn workspace_dir(data_dir: &Path, workspace: &WorkspaceId) -> PathBuf {
    data_dir.join("workspaces").join(workspace.as_str())
}

pub fn workspace_db(data_dir: &Path, workspace: &WorkspaceId) -> PathBuf {
    workspace_dir(data_dir, workspace).join("workspace.db")
}

pub fn workspace_storage_dir(data_dir: &Path, workspace: &WorkspaceId) -> PathBuf {
    workspace_dir(data_dir, workspace).join("storage")
}

pub fn app_dir(data_dir: &Path, workspace: &WorkspaceId, app: &AppId) -> PathBuf {
    workspace_dir(data_dir, workspace)
        .join("apps")
        .join(app.as_str())
}

pub fn app_db(data_dir: &Path, workspace: &WorkspaceId, app: &AppId) -> PathBuf {
    app_dir(data_dir, workspace, app).join("data.db")
}

pub fn app_storage_dir(data_dir: &Path, workspace: &WorkspaceId, app: &AppId) -> PathBuf {
    app_dir(data_dir, workspace, app).join("storage")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_follow_the_documented_layout() {
        let root = Path::new("/var/lib/rustbase");
        let workspace = WorkspaceId::from("acme");
        let app = AppId::from("mobile");

        assert_eq!(system_db(root), Path::new("/var/lib/rustbase/system.db"));
        assert_eq!(
            workspace_db(root, &workspace),
            Path::new("/var/lib/rustbase/workspaces/acme/workspace.db")
        );
        assert_eq!(
            app_db(root, &workspace, &app),
            Path::new("/var/lib/rustbase/workspaces/acme/apps/mobile/data.db")
        );
        assert_eq!(
            workspace_storage_dir(root, &workspace),
            Path::new("/var/lib/rustbase/workspaces/acme/storage")
        );
        assert_eq!(
            app_storage_dir(root, &workspace, &app),
            Path::new("/var/lib/rustbase/workspaces/acme/apps/mobile/storage")
        );
    }
}
