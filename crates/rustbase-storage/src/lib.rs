//! File storage for RustBase.
//!
//! Wraps `object_store` to give a stable, async API the rest of the
//! workspace can use. Backends:
//!
//! - `LocalBackend` — files under `data/realms/<id>/apps/<id>/storage/`.
//! - S3-compatible backends (AWS / R2 / MinIO) — wired in a later
//!   branch via `object_store`'s builders.
//!
//! Binary bytes go through here; metadata lives in `_files` rows
//! managed by `rustbase-db::files`.

use object_store::{ObjectStore, PutPayload, local::LocalFileSystem, path::Path as ObjectPath};
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("object_store error: {0}")]
    Store(#[from] object_store::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, StorageError>;

/// Erased backend, so callers don't depend on a specific
/// `object_store` impl. `Clone` is cheap — the inner store is
/// `Arc`'d.
#[derive(Clone)]
pub struct Storage {
    inner: Arc<dyn ObjectStore>,
}

impl Storage {
    /// Open (or create) a local directory backend at `root`.
    pub async fn local(root: &Path) -> Result<Self> {
        tokio::fs::create_dir_all(root).await?;
        let store = LocalFileSystem::new_with_prefix(root)?;
        Ok(Self {
            inner: Arc::new(store),
        })
    }

    pub async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<()> {
        let path = ObjectPath::from(key);
        self.inner.put(&path, PutPayload::from(bytes)).await?;
        Ok(())
    }

    pub async fn get(&self, key: &str) -> Result<Vec<u8>> {
        let path = ObjectPath::from(key);
        let res = self.inner.get(&path).await?;
        Ok(res.bytes().await?.to_vec())
    }

    pub async fn delete(&self, key: &str) -> Result<()> {
        let path = ObjectPath::from(key);
        self.inner.delete(&path).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn put_get_round_trips_bytes() {
        let dir = tempdir().unwrap();
        let storage = Storage::local(dir.path()).await.unwrap();
        storage.put("hello.txt", b"hello world".to_vec()).await.unwrap();
        let got = storage.get("hello.txt").await.unwrap();
        assert_eq!(got, b"hello world");
    }

    #[tokio::test]
    async fn delete_removes_object() {
        let dir = tempdir().unwrap();
        let storage = Storage::local(dir.path()).await.unwrap();
        storage.put("k", b"v".to_vec()).await.unwrap();
        storage.delete("k").await.unwrap();
        let err = storage.get("k").await.unwrap_err();
        assert!(matches!(err, StorageError::Store(_)));
    }

    #[tokio::test]
    async fn nested_keys_work() {
        let dir = tempdir().unwrap();
        let storage = Storage::local(dir.path()).await.unwrap();
        storage.put("a/b/c.bin", vec![1, 2, 3]).await.unwrap();
        let got = storage.get("a/b/c.bin").await.unwrap();
        assert_eq!(got, vec![1, 2, 3]);
    }
}
