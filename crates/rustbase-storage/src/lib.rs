//! File storage for RustBase. Local disk or any S3-compatible backend
//! via the `object_store` crate.
//!
//! Binary data always goes through the storage backend; only metadata
//! lives in the relevant app's `data.db`.
