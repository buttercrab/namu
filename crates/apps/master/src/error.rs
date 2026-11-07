//! Why a request failed, as the API answers it: a status, and `{"error": ".."}`.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use namu_proto::ErrorResponse;

#[derive(Debug)]
pub enum ApiError {
    /// The request is wrong: fix it and try again.
    BadRequest(String),
    NotFound(String),
    /// A version is already published, differently.
    Conflict(String),
    /// The worker is no longer a member.
    Gone(String),
    /// Nothing can serve it now, e.g. no worker has joined.
    Unavailable(String),
    /// The control plane failed.
    Internal(anyhow::Error),
}

impl ApiError {
    pub fn bad_request(message: impl std::fmt::Display) -> Self {
        Self::BadRequest(message.to_string())
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        Self::Internal(err)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadRequest(message)
            | Self::NotFound(message)
            | Self::Conflict(message)
            | Self::Gone(message)
            | Self::Unavailable(message) => f.write_str(message),
            Self::Internal(err) => write!(f, "{err:#}"),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Gone(_) => StatusCode::GONE,
            Self::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal(err) => {
                tracing::error!("{err:#}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let body = ErrorResponse {
            error: self.to_string(),
        };
        (status, Json(body)).into_response()
    }
}
