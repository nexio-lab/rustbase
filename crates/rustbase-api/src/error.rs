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

impl From<rustbase_runtime::RuntimeError> for ApiError {
    fn from(e: rustbase_runtime::RuntimeError) -> Self {
        match e {
            rustbase_runtime::RuntimeError::Veto(msg) => ApiError::Core(CoreError::Validation(msg)),
            other => ApiError::Core(CoreError::Internal(format!("hook: {other}"))),
        }
    }
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut retry_after: Option<u64> = None;
        let (status, code) = match &self {
            ApiError::Core(e) => match e {
                CoreError::NotFound { .. } => (StatusCode::NOT_FOUND, "not_found"),
                CoreError::WorkspaceNotFound(_) => (StatusCode::NOT_FOUND, "workspace_not_found"),
                CoreError::AppNotFound { .. } => (StatusCode::NOT_FOUND, "app_not_found"),
                CoreError::Validation(_) => (StatusCode::BAD_REQUEST, "validation"),
                CoreError::PolicyViolation { .. } => (StatusCode::CONFLICT, "policy_violation"),
                CoreError::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
                CoreError::Unavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
                CoreError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
                CoreError::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
                CoreError::TooManyRequests { retry_after_secs } => {
                    retry_after = Some(*retry_after_secs);
                    (StatusCode::TOO_MANY_REQUESTS, "too_many_requests")
                }
                CoreError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
            },
        };
        let body = ErrorBody {
            code,
            message: self.to_string(),
        };
        let mut resp = (status, Json(body)).into_response();
        if let Some(secs) = retry_after
            && let Ok(value) = axum::http::HeaderValue::from_str(&secs.to_string())
        {
            resp.headers_mut()
                .insert(axum::http::header::RETRY_AFTER, value);
        }
        resp
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

    #[test]
    fn too_many_requests_maps_to_429() {
        let e = CoreError::TooManyRequests {
            retry_after_secs: 42,
        };
        assert_eq!(status_of(e.into()), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn too_many_requests_sets_retry_after_header() {
        let resp = ApiError::Core(CoreError::TooManyRequests {
            retry_after_secs: 90,
        })
        .into_response();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        let header = resp
            .headers()
            .get(axum::http::header::RETRY_AFTER)
            .expect("Retry-After header missing");
        assert_eq!(header.to_str().unwrap(), "90");
        let bytes = to_bytes(resp.into_body(), 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["code"], "too_many_requests");
        assert!(body["message"].as_str().unwrap().contains("90"));
    }
}
