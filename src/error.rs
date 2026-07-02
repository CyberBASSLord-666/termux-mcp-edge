//! Centralized error types for the server.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Path traversal attempt blocked: {attempted}")]
    PathTraversal { attempted: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Authentication failed")]
    Unauthorized,
}

impl axum::response::IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        use axum::http::StatusCode;
        let status = match self {
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::PathTraversal { .. } => StatusCode::FORBIDDEN,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}
