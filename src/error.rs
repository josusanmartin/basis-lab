use axum::{
    Json,
    http::{HeaderValue, StatusCode, header},
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
    #[error("venue `{venue}` is temporarily rate-limited: {message}")]
    RateLimited {
        venue: String,
        message: String,
        retry_after_seconds: Option<u64>,
    },
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
        let retry_after = match &self {
            Self::Busy => Some(1),
            Self::RateLimited {
                retry_after_seconds,
                ..
            } => *retry_after_seconds,
            _ => None,
        };
        let (status, code) = match self {
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            Self::Upstream { .. } => (StatusCode::BAD_GATEWAY, "upstream_error"),
            Self::RateLimited { .. } => (StatusCode::TOO_MANY_REQUESTS, "upstream_rate_limited"),
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
        let mut response = (status, Json(body)).into_response();
        if let Some(seconds) = retry_after
            && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
        {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_response_includes_retry_guidance() {
        let response = AppError::Busy.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers().get(header::RETRY_AFTER).unwrap(), "1");
    }

    #[test]
    fn upstream_rate_limit_keeps_its_status_and_retry_guidance() {
        let response = AppError::RateLimited {
            venue: "binance_perp".into(),
            message: "too many requests".into(),
            retry_after_seconds: Some(90),
        }
        .into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers().get(header::RETRY_AFTER).unwrap(), "90");
    }
}
