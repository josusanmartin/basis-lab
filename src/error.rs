use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("venue `{venue}` rejected the request: {message}")]
    Upstream { venue: String, message: String },
    #[error("venue `{0}` timed out")]
    Timeout(String),
    #[error("no candles overlap at identical timestamps")]
    NoOverlap,
    #[error("service is busy; retry shortly")]
    Busy,
    #[error("internal server error")]
    Internal,
}

#[derive(Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            Self::Upstream { .. } => (StatusCode::BAD_GATEWAY, "upstream_error"),
            Self::Timeout(_) => (StatusCode::GATEWAY_TIMEOUT, "upstream_timeout"),
            Self::NoOverlap => (StatusCode::UNPROCESSABLE_ENTITY, "no_overlap"),
            Self::Busy => (StatusCode::SERVICE_UNAVAILABLE, "busy"),
            Self::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        };
        let body = ErrorBody {
            error: ErrorDetail {
                code,
                message: self.to_string(),
            },
        };
        (status, Json(body)).into_response()
    }
}
