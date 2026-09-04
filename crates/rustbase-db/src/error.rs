use rustbase_core::CoreError;
use thiserror::Error;

/// Errors from the SQLite persistence layer.
///
/// At the API boundary, `DbError` is mapped to `rustbase_core::CoreError`
/// via the `From` impl below.
#[derive(Debug, Error)]
pub enum DbError {
    #[error("sqlx error: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid identifier: {0}")]
    InvalidIdentifier(String),

    /// A stored row breaks an invariant this layer relies on. Not a
    /// user error and not a transport failure: the data on disk is
    /// shaped in a way no code path here can produce.
    #[error("storage invariant violated: {0}")]
    Invariant(String),

    #[error("migration {migration} failed: {source}")]
    Migration {
        migration: String,
        #[source]
        source: sqlx::Error,
    },

    #[error(transparent)]
    Core(#[from] CoreError),
}

impl From<DbError> for CoreError {
    fn from(e: DbError) -> Self {
        match e {
            DbError::Core(c) => c,
            DbError::InvalidIdentifier(name) => {
                CoreError::Validation(format!("invalid identifier: {name}"))
            }
            DbError::Invariant(msg) => {
                CoreError::Internal(format!("storage invariant violated: {msg}"))
            }
            DbError::Sqlx(s) => CoreError::Internal(format!("database error: {s}")),
            DbError::Io(i) => CoreError::Internal(format!("io error: {i}")),
            DbError::Migration { migration, source } => {
                CoreError::Internal(format!("migration {migration} failed: {source}"))
            }
        }
    }
}

pub type Result<T> = std::result::Result<T, DbError>;
