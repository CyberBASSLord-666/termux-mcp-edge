use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde_json::json;

use crate::transport_security::TransportSecurityPolicy;

#[derive(Clone)]
struct McpTransportState {
    security_policy: TransportSecurityPolicy,
}

/// Build the staged MCP transport shell.
///
/// This route intentionally exposes transport liveness only. Tool discovery,
/// filesystem access, Android platform access, and high-impact actions remain
/// unavailable until later independently validated stages.
pub fn router(security_policy: TransportSecurityPolicy) -> Router {
    Router::new()
        .route("/mcp", post(handle_mcp_request))
        .with_state(McpTransportState { security_policy })
}

async fn handle_mcp_request(
    State(state): State<McpTransportState>,
    headers: HeaderMap,
    _body: Bytes,
) -> Response {
    let host = header_value(&headers, header::HOST);
    let origin = header_value(&headers, header::ORIGIN);

    if let Err(error) = state.security_policy.validate_request(host, origin) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "transport_security_rejected",
                "message": error.to_string(),
            })),
        )
            .into_response();
    }

    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "status": "mcp_transport_shell",
            "message": "MCP transport is reachable, but tool discovery and tool execution are not enabled in this stage.",
        })),
    )
        .into_response()
}

fn header_value(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn transport_shell_accepts_valid_host_and_origin_without_tools() {
        let app = router(TransportSecurityPolicy::localhost(8000, false));

        let response = app
            .oneshot(
                Request::post("/mcp")
                    .header(header::HOST, "localhost:8000")
                    .header(header::ORIGIN, "http://localhost:8000")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }

    #[tokio::test]
    async fn transport_shell_rejects_untrusted_host_before_transport_handling() {
        let app = router(TransportSecurityPolicy::localhost(8000, false));

        let response = app
            .oneshot(
                Request::post("/mcp")
                    .header(header::HOST, "example.com:8000")
                    .header(header::ORIGIN, "http://localhost:8000")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn transport_shell_rejects_malformed_origin_before_transport_handling() {
        let app = router(TransportSecurityPolicy::localhost(8000, false));

        let response = app
            .oneshot(
                Request::post("/mcp")
                    .header(header::HOST, "localhost:8000")
                    .header(header::ORIGIN, "http://localhost:8000/path")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
