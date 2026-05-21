//! Server configuration loading.
//!
//! Resolution order (later sources override earlier ones):
//!   1. compiled-in defaults
//!   2. `rustbase.toml` in the current working directory (optional)
//!   3. `RUSTBASE_*` environment variables
//!
//! Top-level keys map to flat env vars: `RUSTBASE_LISTEN`,
//! `RUSTBASE_DATA_DIR`, `RUSTBASE_REALM_POOL_CAP`, `RUSTBASE_APP_POOL_CAP`.
//! Nested keys use `__` as the separator: `RUSTBASE_LITESTREAM__BUCKET`,
//! `RUSTBASE_LITESTREAM__PREFIX`, etc.

use anyhow::Result;
use config::{Config, Environment, File};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_listen")]
    pub listen: String,
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
    #[serde(default = "default_realm_pool_cap")]
    pub realm_pool_cap: usize,
    #[serde(default = "default_app_pool_cap")]
    pub app_pool_cap: usize,
    #[serde(default)]
    pub litestream: LitestreamConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LitestreamConfig {
    #[serde(default)]
    pub enabled: bool,
    /// S3-compatible bucket URI (e.g. `s3://my-rustbase-backups`).
    /// Required when `enabled` is true.
    #[serde(default)]
    pub bucket: Option<String>,
    /// Optional path prefix inside the bucket (e.g. `prod`).
    #[serde(default)]
    pub prefix: Option<String>,
    /// How often litestream syncs WAL pages, in seconds.
    #[serde(default = "default_replicate_interval")]
    pub replicate_interval_sec: u32,
}

fn default_listen() -> String {
    "0.0.0.0:8080".into()
}
fn default_data_dir() -> PathBuf {
    PathBuf::from("./data")
}
fn default_realm_pool_cap() -> usize {
    32
}
fn default_app_pool_cap() -> usize {
    64
}
fn default_replicate_interval() -> u32 {
    10
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            data_dir: default_data_dir(),
            realm_pool_cap: default_realm_pool_cap(),
            app_pool_cap: default_app_pool_cap(),
            litestream: LitestreamConfig::default(),
        }
    }
}

pub fn load() -> Result<ServerConfig> {
    let builder = Config::builder()
        // defaults
        .set_default("listen", default_listen())?
        .set_default("data_dir", default_data_dir().to_string_lossy().into_owned())?
        .set_default("realm_pool_cap", default_realm_pool_cap() as i64)?
        .set_default("app_pool_cap", default_app_pool_cap() as i64)?
        .set_default("litestream.enabled", false)?
        .set_default("litestream.replicate_interval_sec", default_replicate_interval() as i64)?
        // file
        .add_source(File::with_name("rustbase").required(false))
        // env: prefix=RUSTBASE, "_" splits prefix from key, "__" splits
        // nested keys (`RUSTBASE_LITESTREAM__BUCKET` → litestream.bucket).
        .add_source(
            Environment::with_prefix("RUSTBASE")
                .prefix_separator("_")
                .separator("__")
                .try_parsing(true),
        );

    let raw = builder.build()?;
    let cfg: ServerConfig = raw.try_deserialize()?;

    if cfg.litestream.enabled && cfg.litestream.bucket.is_none() {
        anyhow::bail!("litestream.enabled = true but litestream.bucket is not set");
    }

    Ok(cfg)
}
