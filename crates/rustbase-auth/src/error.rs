use rustbase_core::CoreError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("invalid credentials")]
    InvalidCredentials,

    #[error("token expired")]
    TokenExpired,

    #[error("invalid token: {0}")]
    InvalidToken(String),

    #[error("password hash error: {0}")]
    PasswordHash(String),

    #[error("jwt error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
}

impl From<AuthError> for CoreError {
    fn from(e: AuthError) -> Self {
        match e {
            AuthError::InvalidCredentials
            | AuthError::TokenExpired
            | AuthError::InvalidToken(_) => CoreError::Unauthorized,
            AuthError::PasswordHash(s) => CoreError::Internal(format!("password hash: {s}")),
            AuthError::Jwt(_) => CoreError::Unauthorized,
        }
    }
}

pub type Result<T> = std::result::Result<T, AuthError>;
