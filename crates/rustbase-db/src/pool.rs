//! SQLite pool management for the three scopes.
//!
//! - `SystemPool`: a single pool for `data/system.db`, opened at boot and
//!   always live.
//! - `RealmPoolManager`: LRU-bounded cache of `RealmId → SqlitePool` for
//!   `data/realms/<id>/realm.db`.
//! - `AppPoolManager`: LRU-bounded cache of `(RealmId, AppId) → SqlitePool`
//!   for `data/realms/<id>/apps/<id>/data.db`.
//!
//! Every pool is opened with WAL mode, `foreign_keys=ON`, a 5s busy
//! timeout, and `synchronous=NORMAL`.

use crate::error::{DbError, Result};
use crate::paths;
use lru::LruCache;
use parking_lot::Mutex;
use rustbase_core::{AppId, RealmId};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

const DEFAULT_BUSY_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_MAX_CONNECTIONS: u32 = 8;

/// Open a new `SqlitePool` against `path`, creating it (and its parent
/// directory) if missing, and applying the workspace-standard PRAGMAs.
pub async fn open_pool(path: &Path) -> Result<SqlitePool> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
        .map_err(DbError::Sqlx)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_millis(DEFAULT_BUSY_TIMEOUT_MS));
    let pool = SqlitePoolOptions::new()
        .max_connections(DEFAULT_MAX_CONNECTIONS)
        .connect_with(options)
        .await?;
    Ok(pool)
}

/// Open an in-memory SQLite pool. Used by the test suite.
pub async fn open_memory_pool() -> Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .map_err(DbError::Sqlx)?
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_millis(DEFAULT_BUSY_TIMEOUT_MS));
    let pool = SqlitePoolOptions::new()
        .max_connections(1) // shared in-memory DB requires single connection
        .connect_with(options)
        .await?;
    Ok(pool)
}

/// The system pool — one pool for `data/system.db`, always open.
pub struct SystemPool {
    pool: SqlitePool,
}

impl SystemPool {
    pub async fn open(data_dir: &Path) -> Result<Self> {
        let pool = open_pool(&paths::system_db(data_dir)).await?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

/// LRU-bounded cache of realm-scoped SQLite pools.
pub struct RealmPoolManager {
    data_dir: PathBuf,
    cache: Mutex<LruCache<RealmId, SqlitePool>>,
}

impl RealmPoolManager {
    pub fn new(data_dir: PathBuf, cap: usize) -> Self {
        let cap = NonZeroUsize::new(cap.max(1)).expect("cap is at least 1");
        Self {
            data_dir,
            cache: Mutex::new(LruCache::new(cap)),
        }
    }

    /// Return the pool for `realm`, opening (and caching) one if necessary.
    /// Concurrent first-time opens for the same realm may briefly hold two
    /// pools; SQLite tolerates this under WAL mode and the loser is dropped
    /// at the next `get`.
    pub async fn pool_for(&self, realm: &RealmId) -> Result<SqlitePool> {
        if let Some(pool) = self.cache.lock().get(realm).cloned() {
            return Ok(pool);
        }
        let path = paths::realm_db(&self.data_dir, realm);
        let pool = open_pool(&path).await?;
        let mut cache = self.cache.lock();
        if let Some(existing) = cache.get(realm).cloned() {
            return Ok(existing);
        }
        cache.put(realm.clone(), pool.clone());
        Ok(pool)
    }

    pub fn evict(&self, realm: &RealmId) {
        self.cache.lock().pop(realm);
    }

    pub fn len(&self) -> usize {
        self.cache.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.lock().is_empty()
    }
}

/// LRU-bounded cache of app-scoped SQLite pools.
pub struct AppPoolManager {
    data_dir: PathBuf,
    cache: Mutex<LruCache<(RealmId, AppId), SqlitePool>>,
}

impl AppPoolManager {
    pub fn new(data_dir: PathBuf, cap: usize) -> Self {
        let cap = NonZeroUsize::new(cap.max(1)).expect("cap is at least 1");
        Self {
            data_dir,
            cache: Mutex::new(LruCache::new(cap)),
        }
    }

    pub async fn pool_for(&self, realm: &RealmId, app: &AppId) -> Result<SqlitePool> {
        let key = (realm.clone(), app.clone());
        if let Some(pool) = self.cache.lock().get(&key).cloned() {
            return Ok(pool);
        }
        let path = paths::app_db(&self.data_dir, realm, app);
        let pool = open_pool(&path).await?;
        let mut cache = self.cache.lock();
        if let Some(existing) = cache.get(&key).cloned() {
            return Ok(existing);
        }
        cache.put(key, pool.clone());
        Ok(pool)
    }

    pub fn evict(&self, realm: &RealmId, app: &AppId) {
        let key = (realm.clone(), app.clone());
        self.cache.lock().pop(&key);
    }

    /// Drop every cached pool whose key starts with `realm`. Used when a
    /// realm is being cascade-deleted.
    pub fn evict_realm(&self, realm: &RealmId) {
        let mut cache = self.cache.lock();
        let keys: Vec<_> = cache
            .iter()
            .filter_map(|(k, _)| if &k.0 == realm { Some(k.clone()) } else { None })
            .collect();
        for k in keys {
            cache.pop(&k);
        }
    }

    pub fn len(&self) -> usize {
        self.cache.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.lock().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn open_pool_creates_parent_directory() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested/dirs/test.db");
        let pool = open_pool(&path).await.unwrap();
        sqlx::query("CREATE TABLE t (x INTEGER)")
            .execute(&pool)
            .await
            .unwrap();
        assert!(path.exists());
    }

    #[tokio::test]
    async fn pragmas_are_applied() {
        let dir = tempdir().unwrap();
        let pool = open_pool(&dir.path().join("p.db")).await.unwrap();
        let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
        let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(fk, 1);
    }

    #[tokio::test]
    async fn realm_pool_manager_caches_and_evicts() {
        let dir = tempdir().unwrap();
        let mgr = RealmPoolManager::new(dir.path().to_path_buf(), 2);

        let _p1 = mgr.pool_for(&RealmId::from("a")).await.unwrap();
        let _p2 = mgr.pool_for(&RealmId::from("b")).await.unwrap();
        assert_eq!(mgr.len(), 2);

        // adding a third should evict the LRU
        let _p3 = mgr.pool_for(&RealmId::from("c")).await.unwrap();
        assert_eq!(mgr.len(), 2);
    }

    #[tokio::test]
    async fn realm_pool_manager_does_not_reopen_on_second_call() {
        let dir = tempdir().unwrap();
        let mgr = RealmPoolManager::new(dir.path().to_path_buf(), 4);
        let _a1 = mgr.pool_for(&RealmId::from("acme")).await.unwrap();
        let _a2 = mgr.pool_for(&RealmId::from("acme")).await.unwrap();
        assert_eq!(mgr.len(), 1);
    }

    #[tokio::test]
    async fn app_pool_manager_keys_by_realm_and_app() {
        let dir = tempdir().unwrap();
        let mgr = AppPoolManager::new(dir.path().to_path_buf(), 4);

        let _a = mgr
            .pool_for(&RealmId::from("acme"), &AppId::from("web"))
            .await
            .unwrap();
        let _b = mgr
            .pool_for(&RealmId::from("acme"), &AppId::from("mobile"))
            .await
            .unwrap();
        let _c = mgr
            .pool_for(&RealmId::from("widgetco"), &AppId::from("web"))
            .await
            .unwrap();
        assert_eq!(mgr.len(), 3);
    }
}
