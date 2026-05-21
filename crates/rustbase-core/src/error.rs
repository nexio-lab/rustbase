use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("record not found: {collection}/{id}")]
    NotFound { collection: String, id: String },

    #[error("validation failed: {0}")]
    Validation(String),

    #[error("realm not found: {0}")]
    RealmNotFound(String),

    #[error("app not found: {realm}/{app}")]
    AppNotFound { realm: String, app: String },

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

    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;
