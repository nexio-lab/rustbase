use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("record not found: {collection}/{id}")]
    NotFound { collection: String, id: String },

    #[error("validation failed: {0}")]
    Validation(String),

    #[error("workspace not found: {0}")]
    WorkspaceNotFound(String),

    #[error("app not found: {workspace}/{app}")]
    AppNotFound { workspace: String, app: String },

    #[error("policy violation: {field} = {value} outside bound {bound}")]
    PolicyViolation {
        field: String,
        value: String,
        bound: String,
    },

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden")]
    Forbidden,

    #[error("conflict: {0}")]
    Conflict(String),

    /// Rate limit or account lockout. The wrapped value is the number of
    /// seconds the client should wait before retrying; surfaced as the
    /// `Retry-After` HTTP header at the API boundary.
    #[error("too many requests; retry after {retry_after_secs}s")]
    TooManyRequests { retry_after_secs: u64 },

    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;
