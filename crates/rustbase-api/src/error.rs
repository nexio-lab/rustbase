use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rustbase_auth::AuthError;
use rustbase_core::CoreError;
use rustbase_db::DbError;
use serde::Serialize;
use thiserror::Error;

/// Top-level API error. Every handler returns `Result<_, ApiError>`.
#[derive(Debug, Error)]
pub enum ApiError {
    #[error(transparent)]
    Core(#[from] CoreError),
}

impl From<DbError> for ApiError {
    fn from(e: DbError) -> Self {
        ApiError::Core(e.into())
    }
}

impl From<AuthError> for ApiError {
    fn from(e: AuthError) -> Self {
        ApiError::Core(e.into())
    }
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            ApiError::Core(e) => match e {
                CoreError::NotFound { .. } => (StatusCode::NOT_FOUND, "not_found"),
                CoreError::RealmNotFound(_) => (StatusCode::NOT_FOUND, "realm_not_found"),
                CoreError::AppNotFound { .. } => (StatusCode::NOT_FOUND, "app_not_found"),
                CoreError::Validation(_) => (StatusCode::BAD_REQUEST, "validation"),
                CoreError::PolicyViolation { .. } => (StatusCode::CONFLICT, "policy_violation"),
                CoreError::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
                CoreError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
                CoreError::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
                CoreError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
            },
        };
        let body = ErrorBody {
            code,
            message: self.to_string(),
        };
        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    fn status_of(err: ApiError) -> StatusCode {
        err.into_response().status()
    }

    #[test]
    fn validation_maps_to_400() {
        assert_eq!(
            status_of(CoreError::Validation("bad".into()).into()),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn unauthorized_maps_to_401() {
        assert_eq!(
            status_of(CoreError::Unauthorized.into()),
            StatusCode::UNAUTHORIZED
        );
    }

    #[test]
    fn forbidden_maps_to_403() {
        assert_eq!(
            status_of(CoreError::Forbidden.into()),
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn not_found_maps_to_404() {
        let e = CoreError::NotFound {
            collection: "users".into(),
            id: "abc".into(),
        };
        assert_eq!(status_of(e.into()), StatusCode::NOT_FOUND);
    }

    #[test]
    fn policy_violation_maps_to_409() {
        let e = CoreError::PolicyViolation {
            field: "password.length".into(),
            value: "{min: 2}".into(),
            bound: "{min: 4}".into(),
        };
        assert_eq!(status_of(e.into()), StatusCode::CONFLICT);
    }

    #[test]
    fn internal_maps_to_500() {
        assert_eq!(
            status_of(CoreError::Internal("boom".into()).into()),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn response_body_carries_code_and_message() {
        let resp = ApiError::Core(CoreError::Unauthorized).into_response();
        let bytes = to_bytes(resp.into_body(), 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["code"], "unauthorized");
        assert!(body["message"].as_str().unwrap().contains("unauthorized"));
    }
}
