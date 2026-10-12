use std::{net::SocketAddr, time::Duration};

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use scafra_foundation::{capture_error_backtrace, ShutdownPolicy, ShutdownReason};
use serde::Serialize;

/// Standard application error that can be returned directly from an Scafra
/// route handler. Internal details are traced, while the HTTP response keeps
/// the public message deliberately small.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("resource not found")]
    NotFound,

    #[error("validation failed: {0}")]
    Validation(String),

    #[error("authentication required")]
    Unauthorized,

    #[error("internal application error: {0}")]
    Internal(String),
}

impl AppError {
    pub fn internal(error: impl std::error::Error) -> Self {
        Self::Internal(error.to_string())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match &self {
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", self.to_string()),
            Self::Validation(message) => {
                (StatusCode::BAD_REQUEST, "validation_error", message.clone())
            }
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "authentication required".to_owned(),
            ),
            Self::Internal(_) => {
                scafra_foundation::error!(
                    error_kind = "internal_application_error",
                    backtrace = ?capture_error_backtrace(),
                    "request failed"
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "internal server error".to_owned(),
                )
            }
        };
        error_response(status, code, message)
    }
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    code: &'static str,
    message: String,
}

pub(crate) fn error_response(
    status: StatusCode,
    code: &'static str,
    message: impl Into<String>,
) -> Response {
    IntoResponse::into_response((
        status,
        axum::Json(ErrorResponse {
            code,
            message: message.into(),
        }),
    ))
}

#[derive(Debug, thiserror::Error)]
pub enum WebError {
    #[error("duplicate route {method} {path} declared by {first} and {second}")]
    DuplicateRoute {
        method: String,
        path: String,
        first: &'static str,
        second: &'static str,
    },

    #[error("could not bind Scafra server to {address}: {source}")]
    Bind {
        address: SocketAddr,
        #[source]
        source: std::io::Error,
    },

    #[error("Scafra server failed: {0}")]
    Server(#[source] std::io::Error),
}

impl From<scafra_actuator::HealthRegistryError> for WebError {
    fn from(error: scafra_actuator::HealthRegistryError) -> Self {
        Self::Server(std::io::Error::new(std::io::ErrorKind::InvalidInput, error))
    }
}

/// A typed failure from the Scafra server lifecycle boundary.
///
/// The legacy [`WebError`] API intentionally remains unchanged. New code can
/// inspect a graceful-drain timeout without parsing an I/O error, while the
/// compatibility wrappers translate that outcome back to `WebError::Server`.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error(transparent)]
    Web(#[from] WebError),

    #[error(
        "Scafra graceful shutdown exceeded its {:?} deadline after {:?}",
        policy.grace_period(),
        reason
    )]
    GracefulShutdownTimeout {
        reason: ShutdownReason,
        policy: ShutdownPolicy,
    },
}

impl ServerError {
    /// Converts this typed outcome to the source-compatible web error API.
    pub fn into_web_error(self) -> WebError {
        match self {
            Self::Web(error) => error,
            Self::GracefulShutdownTimeout { .. } => WebError::Server(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Scafra graceful shutdown exceeded its configured deadline",
            )),
        }
    }

    /// Returns the shutdown reason that was active when a typed timeout was
    /// reported.
    pub fn timeout_reason(&self) -> Option<ShutdownReason> {
        match self {
            Self::GracefulShutdownTimeout { reason, .. } => Some(*reason),
            Self::Web(_) => None,
        }
    }

    /// Returns the policy that bounded a typed graceful-drain timeout.
    pub fn timeout_policy(&self) -> Option<ShutdownPolicy> {
        match self {
            Self::GracefulShutdownTimeout { policy, .. } => Some(*policy),
            Self::Web(_) => None,
        }
    }

    /// Returns the timeout duration without exposing any error payload.
    pub fn timeout_duration(&self) -> Option<Duration> {
        self.timeout_policy().map(ShutdownPolicy::grace_period)
    }
}
