use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::transport_security::TransportSecurityPolicy;

#[derive(Clone)]
struct McpTransportState {
    security_policy: TransportSecurityPolicy,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    #[allow(dead_code)]
    params: Option<Value>,
}

/// Build the staged MCP transport shell.
///
/// This route intentionally exposes only transport liveness and a minimal MCP
/// discovery contract. Tool execution, filesystem access, Android platform
/// access, and high-impact actions remain unavailable until later independently
/// validated stages.
pub fn router(security_policy: TransportSecurityPolicy) -> Router {
    Router::new()
        .route("/mcp", post(handle_mcp_request))
        .with_state(McpTransportState { security_policy })
}

async fn handle_mcp_request(
    State(state): State<McpTransportState>,
    headers: HeaderMap,
    body: Bytes,
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

    if body.is_empty() {
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({
                "status": "mcp_transport_shell",
                "message": "MCP transport is reachable. Tool discovery is available with an intentionally empty registry; tool execution is not enabled in this stage.",
            })),
        )
            .into_response();
    }

    let request = match serde_json::from_slice::<JsonRpcRequest>(&body) {
        Ok(request) => request,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": {
                        "code": -32700,
                        "message": "Parse error",
                        "data": error.to_string(),
                    },
                })),
            )
                .into_response();
        }
    };

    match request.method.as_str() {
        "initialize" => (
            StatusCode::OK,
            Json(json!({
                "jsonrpc": "2.0",
                "id": request.id.unwrap_or(Value::Null),
                "result": {
                    "protocolVersion": "2024-11-05",
                    "serverInfo": {
                        "name": "termux-mcp-edge",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "capabilities": {
                        "tools": {
                            "listChanged": false,
                        },
                    },
                },
            })),
        )
            .into_response(),
        "tools/list" => (
            StatusCode::OK,
            Json(json!({
                "jsonrpc": "2.0",
                "id": request.id.unwrap_or(Value::Null),
                "result": {
                    "tools": [],
                },
            })),
        )
            .into_response(),
        "tools/call" => method_not_available(
            request.id,
            "Tool execution is intentionally disabled in this staged runtime.",
        ),
        _ => method_not_available(
            request.id,
            "Only initialize and tools/list are available in this staged runtime.",
        ),
    }
}

fn method_not_available(id: Option<Value>, message: &'static str) -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "error": {
                "code": -32601,
                "message": "Method not found",
                "data": message,
            },
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

    #[tokio::test]
    async fn tool_discovery_returns_empty_registry() {
        let app = router(TransportSecurityPolicy::localhost(8000, false));

        let response = app
            .oneshot(
                Request::post("/mcp")
                    .header(header::HOST, "localhost:8000")
                    .header(header::ORIGIN, "http://localhost:8000")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(payload["jsonrpc"], "2.0");
        assert_eq!(payload["id"], 1);
        assert_eq!(payload["result"]["tools"], json!([]));
    }

    #[tokio::test]
    async fn tool_call_remains_unavailable() {
        let app = router(TransportSecurityPolicy::localhost(8000, false));

        let response = app
            .oneshot(
                Request::post("/mcp")
                    .header(header::HOST, "localhost:8000")
                    .header(header::ORIGIN, "http://localhost:8000")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"anything"}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(payload["jsonrpc"], "2.0");
        assert_eq!(payload["id"], 2);
        assert_eq!(payload["error"]["code"], -32601);
    }
}
