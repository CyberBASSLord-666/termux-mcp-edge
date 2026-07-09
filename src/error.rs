//! Centralized error types for the server.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Path traversal attempt blocked: {attempted}")]
    PathTraversal { attempted: String },

    #[error("File is too large to read: {size} bytes exceeds {max_size} byte limit")]
    FileTooLarge { size: u64, max_size: u64 },

    #[error("Write payload is too large: {size} bytes exceeds {max_size} byte limit")]
    WritePayloadTooLarge { size: u64, max_size: u64 },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Authentication failed")]
    Unauthorized,
}

impl axum::response::IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        use axum::http::StatusCode;
        let status = match self {
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::PathTraversal { .. } => StatusCode::FORBIDDEN,
            AppError::FileTooLarge { .. } | AppError::WritePayloadTooLarge { .. } => {
                StatusCode::PAYLOAD_TOO_LARGE
            }
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}
