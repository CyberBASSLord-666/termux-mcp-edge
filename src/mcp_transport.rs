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

const RUNTIME_STATUS_TOOL: &str = "runtime_status";

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
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ToolCallParams {
    name: String,
    #[allow(dead_code)]
    arguments: Option<Value>,
}

/// Build the staged MCP transport shell.
///
/// This route intentionally exposes only transport liveness, MCP discovery, and
/// one deterministic read-only status tool. Filesystem access, Android platform
/// access, command execution, and high-impact actions remain unavailable until
/// later independently validated stages.
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
                "message": "MCP transport is reachable. Tool discovery and the read-only runtime_status tool are available; filesystem, platform, command, and high-impact tools are not enabled in this stage.",
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

    let JsonRpcRequest {
        id, method, params, ..
    } = request;

    match method.as_str() {
        "initialize" => (
            StatusCode::OK,
            Json(json!({
                "jsonrpc": "2.0",
                "id": id.unwrap_or(Value::Null),
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
                "id": id.unwrap_or(Value::Null),
                "result": {
                    "tools": [
                        {
                            "name": RUNTIME_STATUS_TOOL,
                            "description": "Return deterministic read-only runtime metadata for the staged Termux MCP Edge server.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {},
                                "additionalProperties": false,
                            },
                        },
                    ],
                },
            })),
        )
            .into_response(),
        "tools/call" => handle_tool_call(id, params),
        _ => method_not_available(
            id,
            "Only initialize, tools/list, and the runtime_status tools/call path are available in this staged runtime.",
        ),
    }
}

fn handle_tool_call(id: Option<Value>, params: Option<Value>) -> Response {
    let params = match params {
        Some(params) => params,
        None => {
            return invalid_params(id, "tools/call requires params with a tool name.");
        }
    };

    let call = match serde_json::from_value::<ToolCallParams>(params) {
        Ok(call) => call,
        Err(error) => {
            return invalid_params(id, &format!("Invalid tools/call params: {error}"));
        }
    };

    if call.name != RUNTIME_STATUS_TOOL {
        return method_not_available(
            id,
            "Only the read-only runtime_status tool is available in this staged runtime.",
        );
    }

    (
        StatusCode::OK,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "result": {
                "content": [
                    {
                        "type": "text",
                        "text": "termux-mcp-edge runtime_status: transport=staged, tools=read-only-runtime-status, filesystem=disabled, android_platform=disabled, command_execution=disabled",
                    },
                ],
                "structuredContent": {
                    "server": "termux-mcp-edge",
                    "version": env!("CARGO_PKG_VERSION"),
                    "transport": "staged_mcp_runtime",
                    "availableTools": [RUNTIME_STATUS_TOOL],
                    "filesystemTools": false,
                    "androidPlatformTools": false,
                    "commandExecution": false,
                    "highImpactTools": false,
                },
                "isError": false,
            },
        })),
    )
        .into_response()
}

fn invalid_params(id: Option<Value>, message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "error": {
                "code": -32602,
                "message": "Invalid params",
                "data": message,
            },
        })),
    )
        .into_response()
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
    async fn tool_discovery_returns_runtime_status_tool_only() {
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
        assert_eq!(payload["result"]["tools"][0]["name"], RUNTIME_STATUS_TOOL);
        assert_eq!(payload["result"]["tools"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn runtime_status_tool_call_returns_deterministic_read_only_metadata() {
        let app = router(TransportSecurityPolicy::localhost(8000, false));

        let response = app
            .oneshot(
                Request::post("/mcp")
                    .header(header::HOST, "localhost:8000")
                    .header(header::ORIGIN, "http://localhost:8000")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"runtime_status","arguments":{}}}"#,
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
        assert_eq!(payload["id"], 2);
        assert_eq!(payload["result"]["isError"], false);
        assert_eq!(
            payload["result"]["structuredContent"]["availableTools"][0],
            RUNTIME_STATUS_TOOL
        );
        assert_eq!(
            payload["result"]["structuredContent"]["filesystemTools"],
            false
        );
        assert_eq!(
            payload["result"]["structuredContent"]["androidPlatformTools"],
            false
        );
        assert_eq!(
            payload["result"]["structuredContent"]["commandExecution"],
            false
        );
    }

    #[tokio::test]
    async fn unknown_tool_call_remains_unavailable() {
        let app = router(TransportSecurityPolicy::localhost(8000, false));

        let response = app
            .oneshot(
                Request::post("/mcp")
                    .header(header::HOST, "localhost:8000")
                    .header(header::ORIGIN, "http://localhost:8000")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"anything"}}"#,
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
        assert_eq!(payload["id"], 3);
        assert_eq!(payload["error"]["code"], -32601);
    }
}
