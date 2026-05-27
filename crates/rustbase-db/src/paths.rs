//! Layout of `data/` on disk and helpers to compute the canonical paths.
//!
//! ```text
//! <data_dir>/
//!   system.db
//!   realms/
//!     <realm_id>/
//!       realm.db
//!       storage/
//!       apps/
//!         <app_id>/
//!           data.db
//!           storage/
//! ```

use rustbase_core::{AppId, RealmId};
use std::path::{Path, PathBuf};

pub fn system_db(data_dir: &Path) -> PathBuf {
    data_dir.join("system.db")
}

pub fn realm_dir(data_dir: &Path, realm: &RealmId) -> PathBuf {
    data_dir.join("realms").join(realm.as_str())
}

pub fn realm_db(data_dir: &Path, realm: &RealmId) -> PathBuf {
    realm_dir(data_dir, realm).join("realm.db")
}

pub fn realm_storage_dir(data_dir: &Path, realm: &RealmId) -> PathBuf {
    realm_dir(data_dir, realm).join("storage")
}

pub fn app_dir(data_dir: &Path, realm: &RealmId, app: &AppId) -> PathBuf {
    realm_dir(data_dir, realm).join("apps").join(app.as_str())
}

pub fn app_db(data_dir: &Path, realm: &RealmId, app: &AppId) -> PathBuf {
    app_dir(data_dir, realm, app).join("data.db")
}

pub fn app_storage_dir(data_dir: &Path, realm: &RealmId, app: &AppId) -> PathBuf {
    app_dir(data_dir, realm, app).join("storage")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_follow_the_documented_layout() {
        let root = Path::new("/var/lib/rustbase");
        let realm = RealmId::from("acme");
        let app = AppId::from("mobile");

        assert_eq!(system_db(root), Path::new("/var/lib/rustbase/system.db"));
        assert_eq!(
            realm_db(root, &realm),
            Path::new("/var/lib/rustbase/realms/acme/realm.db")
        );
        assert_eq!(
            app_db(root, &realm, &app),
            Path::new("/var/lib/rustbase/realms/acme/apps/mobile/data.db")
        );
        assert_eq!(
            realm_storage_dir(root, &realm),
            Path::new("/var/lib/rustbase/realms/acme/storage")
        );
        assert_eq!(
            app_storage_dir(root, &realm, &app),
            Path::new("/var/lib/rustbase/realms/acme/apps/mobile/storage")
        );
    }
}
