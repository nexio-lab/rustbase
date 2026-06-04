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
use rustbase_api::mailer::SmtpConfig;
use rustbase_storage::S3Config;
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
    /// `[mail]` section. Absent → server boots with a `LogMailer`.
    #[serde(default)]
    pub mail: MailConfig,
    /// `[storage]` section. Absent → local-disk backend rooted at
    /// `data_dir`.
    #[serde(default)]
    pub storage: StorageConfig,
    /// HTTP server hardening: body cap, security headers.
    #[serde(default)]
    pub http: HttpConfig,
    /// CORS allowlist for the REST API. Defaults to "no cross-origin".
    #[serde(default)]
    pub cors: CorsConfig,
    /// Per-IP rate limiter for the HTTP entry layer.
    #[serde(default)]
    pub rate_limit: RateLimitConfig,
    /// Per-subject auth lockout (login_failed → lock for N seconds).
    #[serde(default)]
    pub lockout: LockoutConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct StorageConfig {
    /// `[storage.s3]`. Present means: use S3Storage; absent → local
    /// directory backend under `data_dir/.../storage/`.
    #[serde(default)]
    pub s3: Option<S3Config>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MailConfig {
    /// `[mail.smtp]`. Present means: use SmtpMailer; absent → LogMailer.
    #[serde(default)]
    pub smtp: Option<SmtpConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HttpConfig {
    /// Maximum HTTP request body size in bytes. Applied at the entry
    /// layer via tower-http's `RequestBodyLimitLayer`.
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: usize,
    /// Toggle the default-on bundle of security response headers
    /// (HSTS, X-Content-Type-Options, Referrer-Policy, X-Frame-Options,
    /// a baseline CSP). Disable only when an upstream reverse proxy
    /// already injects them.
    #[serde(default = "default_security_headers")]
    pub security_headers: bool,
    /// HSTS `max-age=` value in seconds. 0 → omit the header. The
    /// default (`63072000` ≈ 2 years) is what most security baselines
    /// recommend once the deployment is TLS-only.
    #[serde(default = "default_hsts_max_age")]
    pub hsts_max_age_secs: u64,
    /// Whether to add `includeSubDomains` to the HSTS header.
    #[serde(default = "default_hsts_include_subdomains")]
    pub hsts_include_subdomains: bool,
    /// Whether the dashboard session cookies (`rb_at`, `rb_rt`) are
    /// emitted with the `Secure` attribute. Defaults to `true`
    /// (production assumption: TLS-terminated). Set to `false` for
    /// local dev over plain HTTP — browsers reject `Secure` cookies
    /// on non-TLS origins.
    #[serde(default = "default_cookie_secure")]
    pub cookie_secure: bool,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            max_body_bytes: default_max_body_bytes(),
            security_headers: default_security_headers(),
            hsts_max_age_secs: default_hsts_max_age(),
            hsts_include_subdomains: default_hsts_include_subdomains(),
            cookie_secure: default_cookie_secure(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CorsConfig {
    /// Explicit allowlist of `Origin` values accepted on the REST API.
    /// Empty (default) → CORS layer denies everything cross-origin. The
    /// dashboard is same-origin so does not need an entry here.
    #[serde(default)]
    pub allow_origins: Vec<String>,
    /// Whether browsers may send credentials (cookies, auth headers) on
    /// allowed origins. Defaults to `false`.
    #[serde(default)]
    pub allow_credentials: bool,
    /// Preflight cache TTL in seconds. Defaults to 600.
    #[serde(default = "default_cors_max_age")]
    pub max_age_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitConfig {
    /// Master toggle. Defaults to `true`. When false the rate-limit
    /// layer is not installed at all.
    #[serde(default = "default_rate_limit_enabled")]
    pub enabled: bool,
    /// Steady-state allowance, requests-per-second per source IP.
    #[serde(default = "default_rate_limit_per_second")]
    pub per_second: u32,
    /// Maximum burst size — how many tokens the bucket holds before it
    /// throttles.
    #[serde(default = "default_rate_limit_burst")]
    pub burst: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: default_rate_limit_enabled(),
            per_second: default_rate_limit_per_second(),
            burst: default_rate_limit_burst(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LockoutConfig {
    /// Master toggle. Defaults to `true`.
    #[serde(default = "default_lockout_enabled")]
    pub enabled: bool,
    /// Number of failed authentication attempts allowed inside the
    /// rolling window before a subject is locked.
    #[serde(default = "default_lockout_max_failures")]
    pub max_failures: u32,
    /// Rolling window over which failures accumulate, in seconds.
    #[serde(default = "default_lockout_window_secs")]
    pub window_secs: u64,
    /// Lockout duration once `max_failures` is reached, in seconds.
    /// Surfaced as the `Retry-After` header on the 429 response.
    #[serde(default = "default_lockout_duration_secs")]
    pub lockout_secs: u64,
}

impl Default for LockoutConfig {
    fn default() -> Self {
        Self {
            enabled: default_lockout_enabled(),
            max_failures: default_lockout_max_failures(),
            window_secs: default_lockout_window_secs(),
            lockout_secs: default_lockout_duration_secs(),
        }
    }
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
fn default_max_body_bytes() -> usize {
    8 * 1024 * 1024
}
fn default_security_headers() -> bool {
    true
}
fn default_hsts_max_age() -> u64 {
    63_072_000
}
fn default_hsts_include_subdomains() -> bool {
    true
}
fn default_cookie_secure() -> bool {
    true
}
fn default_cors_max_age() -> u64 {
    600
}
fn default_rate_limit_enabled() -> bool {
    true
}
fn default_rate_limit_per_second() -> u32 {
    50
}
fn default_rate_limit_burst() -> u32 {
    100
}
fn default_lockout_enabled() -> bool {
    true
}
fn default_lockout_max_failures() -> u32 {
    5
}
fn default_lockout_window_secs() -> u64 {
    300
}
fn default_lockout_duration_secs() -> u64 {
    300
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            data_dir: default_data_dir(),
            realm_pool_cap: default_realm_pool_cap(),
            app_pool_cap: default_app_pool_cap(),
            litestream: LitestreamConfig::default(),
            mail: MailConfig::default(),
            storage: StorageConfig::default(),
            http: HttpConfig::default(),
            cors: CorsConfig::default(),
            rate_limit: RateLimitConfig::default(),
            lockout: LockoutConfig::default(),
        }
    }
}

pub fn load() -> Result<ServerConfig> {
    let builder = Config::builder()
        // defaults
        .set_default("listen", default_listen())?
        .set_default(
            "data_dir",
            default_data_dir().to_string_lossy().into_owned(),
        )?
        .set_default("realm_pool_cap", default_realm_pool_cap() as i64)?
        .set_default("app_pool_cap", default_app_pool_cap() as i64)?
        .set_default("litestream.enabled", false)?
        .set_default(
            "litestream.replicate_interval_sec",
            default_replicate_interval() as i64,
        )?
        // http
        .set_default("http.max_body_bytes", default_max_body_bytes() as i64)?
        .set_default("http.security_headers", default_security_headers())?
        .set_default("http.hsts_max_age_secs", default_hsts_max_age() as i64)?
        .set_default(
            "http.hsts_include_subdomains",
            default_hsts_include_subdomains(),
        )?
        .set_default("http.cookie_secure", default_cookie_secure())?
        // cors
        .set_default::<&str, Vec<String>>("cors.allow_origins", Vec::new())?
        .set_default("cors.allow_credentials", false)?
        .set_default("cors.max_age_secs", default_cors_max_age() as i64)?
        // rate limit
        .set_default("rate_limit.enabled", default_rate_limit_enabled())?
        .set_default(
            "rate_limit.per_second",
            default_rate_limit_per_second() as i64,
        )?
        .set_default("rate_limit.burst", default_rate_limit_burst() as i64)?
        // lockout
        .set_default("lockout.enabled", default_lockout_enabled())?
        .set_default(
            "lockout.max_failures",
            default_lockout_max_failures() as i64,
        )?
        .set_default("lockout.window_secs", default_lockout_window_secs() as i64)?
        .set_default(
            "lockout.lockout_secs",
            default_lockout_duration_secs() as i64,
        )?
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
