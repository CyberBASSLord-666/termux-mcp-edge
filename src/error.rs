//! Centralized error types for the server.

use axum::http::StatusCode;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Path traversal attempt blocked: {attempted}")]
    PathTraversal { attempted: String },

    #[error("File is too large to read: {size} bytes exceeds {max_size} byte limit")]
    FileTooLarge { size: u64, max_size: u64 },

    #[error("File content is not valid UTF-8")]
    InvalidFileEncoding,

    #[error("Requested binary range is outside the bounded file contract")]
    InvalidBinaryRange,

    #[error("Requested text range is outside the bounded UTF-8 file contract")]
    InvalidTextRange,

    #[error("File size changed during the bounded read")]
    FileChangedDuringRead,

    #[error("Path-discovery query does not satisfy the literal basename contract")]
    InvalidFindQuery,

    #[error("Search query does not satisfy the literal text-search contract")]
    InvalidSearchQuery,

    #[error("Requested filesystem object does not exist")]
    PathNotFound,

    #[error("Requested filesystem object type is not supported")]
    UnsupportedPathType,

    #[error("Requested filesystem destination already exists")]
    PathAlreadyExists,

    #[error("Requested copy source does not exist")]
    CopySourceNotFound,

    #[error("Requested copy destination parent does not exist")]
    CopyDestinationParentNotFound,

    #[error("Copy source and destination must be different paths")]
    CopySourceDestinationSame,

    #[error("Write payload is too large: {size} bytes exceeds {max_size} byte limit")]
    WritePayloadTooLarge { size: u64, max_size: u64 },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Authentication failed")]
    Unauthorized,
}

impl AppError {
    /// Return the stable, non-sensitive HTTP representation for this error.
    ///
    /// Internal `Display` output remains available for trusted diagnostics, but
    /// request-derived paths and operating-system error text must never cross
    /// the HTTP boundary.
    fn public_response(&self) -> (StatusCode, &'static str) {
        match self {
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "Authentication failed"),
            AppError::PathTraversal { .. } => {
                (StatusCode::FORBIDDEN, "Requested path is not permitted")
            }
            AppError::FileTooLarge { .. } => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "File exceeds the configured read limit",
            ),
            AppError::InvalidFileEncoding => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "File content must be valid UTF-8",
            ),
            AppError::InvalidBinaryRange => (
                StatusCode::BAD_REQUEST,
                "Requested binary range is not valid",
            ),
            AppError::InvalidTextRange => (
                StatusCode::BAD_REQUEST,
                "Requested text range is not valid",
            ),
            AppError::FileChangedDuringRead => {
                (StatusCode::CONFLICT, "File changed during the bounded read")
            }
            AppError::InvalidFindQuery => (
                StatusCode::BAD_REQUEST,
                "Path-discovery query does not satisfy the literal basename contract",
            ),
            AppError::InvalidSearchQuery => (
                StatusCode::BAD_REQUEST,
                "Search query does not satisfy the literal text-search contract",
            ),
            AppError::PathNotFound => (
                StatusCode::NOT_FOUND,
                "Requested filesystem object does not exist",
            ),
            AppError::UnsupportedPathType => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "Requested filesystem object type is not supported",
            ),
            AppError::PathAlreadyExists => (
                StatusCode::CONFLICT,
                "Requested filesystem destination already exists",
            ),
            AppError::CopySourceNotFound => (
                StatusCode::NOT_FOUND,
                "Requested copy source does not exist",
            ),
            AppError::CopyDestinationParentNotFound => (
                StatusCode::NOT_FOUND,
                "Requested copy destination parent does not exist",
            ),
            AppError::CopySourceDestinationSame => (
                StatusCode::BAD_REQUEST,
                "Copy source and destination must be different paths",
            ),
            AppError::WritePayloadTooLarge { .. } => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "Write payload exceeds the configured limit",
            ),
            AppError::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error"),
        }
    }
}

impl axum::response::IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = self.public_response();
        (status, message).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_traversal_http_response_redacts_attempted_path() {
        let attempted = "/data/data/com.termux/files/home/.ssh/id_ed25519";
        let error = AppError::PathTraversal {
            attempted: attempted.to_owned(),
        };

        let (status, message) = error.public_response();

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(message, "Requested path is not permitted");
        assert!(!message.contains(attempted));
    }

    #[test]
    fn io_http_response_redacts_operating_system_error_text() {
        let sensitive = "/data/data/com.termux/files/home/private/runtime.env";
        let error = AppError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("permission denied while opening {sensitive}"),
        ));

        let (status, message) = error.public_response();

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(message, "Internal server error");
        assert!(!message.contains(sensitive));
        assert!(!message.contains("permission denied"));
    }

    #[test]
    fn bounded_input_errors_keep_distinct_safe_contracts() {
        assert_eq!(
            AppError::FileTooLarge {
                size: 1025,
                max_size: 1024,
            }
            .public_response(),
            (
                StatusCode::PAYLOAD_TOO_LARGE,
                "File exceeds the configured read limit",
            )
        );
        assert_eq!(
            AppError::WritePayloadTooLarge {
                size: 1025,
                max_size: 1024,
            }
            .public_response(),
            (
                StatusCode::PAYLOAD_TOO_LARGE,
                "Write payload exceeds the configured limit",
            )
        );
        assert_eq!(
            AppError::InvalidFileEncoding.public_response(),
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                "File content must be valid UTF-8",
            )
        );
        assert_eq!(
            AppError::InvalidBinaryRange.public_response(),
            (
                StatusCode::BAD_REQUEST,
                "Requested binary range is not valid",
            )
        );
        assert_eq!(
            AppError::InvalidTextRange.public_response(),
            (
                StatusCode::BAD_REQUEST,
                "Requested text range is not valid",
            )
        );
        assert_eq!(
            AppError::FileChangedDuringRead.public_response(),
            (StatusCode::CONFLICT, "File changed during the bounded read",)
        );
        assert_eq!(
            AppError::InvalidFindQuery.public_response(),
            (
                StatusCode::BAD_REQUEST,
                "Path-discovery query does not satisfy the literal basename contract",
            )
        );
        assert_eq!(
            AppError::InvalidSearchQuery.public_response(),
            (
                StatusCode::BAD_REQUEST,
                "Search query does not satisfy the literal text-search contract",
            )
        );
        assert_eq!(
            AppError::PathNotFound.public_response(),
            (
                StatusCode::NOT_FOUND,
                "Requested filesystem object does not exist",
            )
        );
        assert_eq!(
            AppError::UnsupportedPathType.public_response(),
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                "Requested filesystem object type is not supported",
            )
        );
        assert_eq!(
            AppError::PathAlreadyExists.public_response(),
            (
                StatusCode::CONFLICT,
                "Requested filesystem destination already exists",
            )
        );
        assert_eq!(
            AppError::CopySourceNotFound.public_response(),
            (
                StatusCode::NOT_FOUND,
                "Requested copy source does not exist",
            )
        );
        assert_eq!(
            AppError::CopyDestinationParentNotFound.public_response(),
            (
                StatusCode::NOT_FOUND,
                "Requested copy destination parent does not exist",
            )
        );
        assert_eq!(
            AppError::CopySourceDestinationSame.public_response(),
            (
                StatusCode::BAD_REQUEST,
                "Copy source and destination must be different paths",
            )
        );
    }
}
