//! Centralized error types for the MCP server.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Path traversal attempt blocked: {attempted}")]
    PathTraversal { attempted: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    InvalidConfiguration(String),

    #[error("MCP protocol error: {0}")]
    Mcp(#[from] rmcp::Error),

    #[error("Configuration error: {0}")]
    Config(#[from] config::ConfigError),

    #[error("Command `{command}` failed with exit code {exit_code}: {stderr}")]
    CommandFailed {
        command: String,
        exit_code: i32,
        stderr: String,
    },

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

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
