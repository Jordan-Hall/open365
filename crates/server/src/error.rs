//! API errors. Bodies are fixed strings so that no internal detail or token ever reaches a client or log.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug)]
pub enum ApiError {
    BadRequest(&'static str),
    Unauthorized(&'static str),
    Forbidden(&'static str),
    NotFound,
    Conflict(&'static str),
    TooLarge,
    RateLimited,
    Internal,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::BadRequest(c) => (StatusCode::BAD_REQUEST, c),
            Self::Unauthorized(c) => (StatusCode::UNAUTHORIZED, c),
            Self::Forbidden(c) => (StatusCode::FORBIDDEN, c),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::Conflict(c) => (StatusCode::CONFLICT, c),
            Self::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "too_large"),
            Self::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            Self::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        };
        (status, Json(serde_json::json!({ "error": code }))).into_response()
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        eprintln!("database error: {e}");
        Self::Internal
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        eprintln!("io error: {e}");
        Self::Internal
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
