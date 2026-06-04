//! File storage for RustBase.
//!
//! Wraps `object_store` to give a stable, async API the rest of the
//! workspace can use. Backends:
//!
//! - `Storage::local(root)` — files under
//!   `data/workspaces/<id>/apps/<id>/storage/`.
//! - `Storage::s3(cfg)` — any S3-compatible bucket. The `endpoint`
//!   field opts into a non-AWS host (MinIO on `http://localhost:9000`,
//!   Cloudflare R2 at `https://<acct>.r2.cloudflarestorage.com`, etc.)
//!   while leaving the AWS path-style request format intact.
//!
//! Binary bytes go through here; metadata lives in `_files` rows
//! managed by `rustbase-db::files`.

use object_store::{
    ObjectStore, PutPayload, aws::AmazonS3Builder, local::LocalFileSystem, path::Path as ObjectPath,
};
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("object_store error: {0}")]
    Store(#[from] object_store::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("storage config: {0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, StorageError>;

/// S3-compatible backend configuration. Deserialised straight out of
/// `[storage.s3]` in `rustbase.toml` by the server crate; passing it
/// to [`Storage::s3`] never touches the network — credentials and
/// the endpoint are validated lazily on the first request.
#[derive(Debug, Clone, Deserialize)]
pub struct S3Config {
    pub bucket: String,
    /// AWS region (e.g. `us-east-1`). MinIO accepts any non-empty
    /// region; R2 conventionally uses `auto`. Required by the S3
    /// signing process even when `endpoint` overrides the host.
    pub region: String,
    /// Optional non-AWS endpoint URL. Set to `http://localhost:9000`
    /// for the dev MinIO container under `infra/docker-compose.yml`.
    #[serde(default)]
    pub endpoint: Option<String>,
    pub access_key_id: String,
    pub secret_access_key: String,
    /// If true, force path-style addressing (`<endpoint>/<bucket>/key`
    /// instead of `<bucket>.<endpoint>/key`). Required for MinIO and
    /// most non-AWS S3 clones; harmless against AWS.
    #[serde(default = "default_path_style")]
    pub virtual_hosted_style_request: bool,
}

fn default_path_style() -> bool {
    // Default to path-style — works for both MinIO and AWS, and
    // avoids the DNS wildcard requirement of virtual-hosted style.
    false
}

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

    /// Build an S3-compatible backend. Does not touch the network —
    /// credentials are exchanged on the first put/get. A bad bucket
    /// name or endpoint URL surfaces as a [`StorageError::Config`].
    pub fn s3(cfg: &S3Config) -> Result<Self> {
        if cfg.bucket.is_empty() {
            return Err(StorageError::Config("s3 bucket is empty".into()));
        }
        if cfg.region.is_empty() {
            return Err(StorageError::Config(
                "s3 region is required (use \"auto\" for R2)".into(),
            ));
        }
        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(&cfg.bucket)
            .with_region(&cfg.region)
            .with_access_key_id(&cfg.access_key_id)
            .with_secret_access_key(&cfg.secret_access_key)
            .with_virtual_hosted_style_request(cfg.virtual_hosted_style_request);
        if let Some(ep) = cfg.endpoint.as_deref() {
            builder = builder.with_endpoint(ep);
            // Allow plain HTTP for `http://localhost:9000` etc. AWS
            // endpoints aren't accepted with `with_allow_http(true)`,
            // but for MinIO / dev S3 we explicitly need it.
            if ep.starts_with("http://") {
                builder = builder.with_allow_http(true);
            }
        }
        let store = builder
            .build()
            .map_err(|e| StorageError::Config(format!("build s3: {e}")))?;
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
        storage
            .put("hello.txt", b"hello world".to_vec())
            .await
            .unwrap();
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

    // ---- S3 backend ----

    fn minio_cfg() -> S3Config {
        S3Config {
            bucket: "rustbase-dev".into(),
            region: "us-east-1".into(),
            endpoint: Some("http://localhost:9000".into()),
            access_key_id: "minioadmin".into(),
            secret_access_key: "minioadmin".into(),
            virtual_hosted_style_request: false,
        }
    }

    #[test]
    fn s3_builds_for_minio_config_without_network() {
        // Builder validation only — no requests issued.
        Storage::s3(&minio_cfg()).expect("MinIO config must build");
    }

    #[test]
    fn s3_rejects_empty_bucket_and_region() {
        let mut cfg = minio_cfg();
        cfg.bucket = "".into();
        assert!(matches!(Storage::s3(&cfg), Err(StorageError::Config(_))));
        let mut cfg = minio_cfg();
        cfg.region = "".into();
        assert!(matches!(Storage::s3(&cfg), Err(StorageError::Config(_))));
    }

    /// Live round-trip against the MinIO container from
    /// `infra/docker-compose.yml`. Skipped by default — opt in with:
    ///
    ///     docker compose -f infra/docker-compose.yml up -d
    ///     cargo test -p rustbase-storage s3_minio_round_trip \
    ///         -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires MinIO at localhost:9000 (infra/docker-compose.yml)"]
    async fn s3_minio_round_trip() {
        let storage = Storage::s3(&minio_cfg()).expect("build S3");

        // Unique per-test key so successive runs don't read stale data.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros();
        let key = format!("rustbase-storage-smoke/{stamp}.bin");
        let body = b"hello from S3 backend";

        storage.put(&key, body.to_vec()).await.expect("put");
        let got = storage.get(&key).await.expect("get");
        assert_eq!(got, body);

        storage.delete(&key).await.expect("delete");
        let err = storage.get(&key).await.unwrap_err();
        assert!(matches!(err, StorageError::Store(_)));
    }
}
