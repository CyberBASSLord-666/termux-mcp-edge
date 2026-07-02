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

use crate::{tools::FileSystemTools, transport_security::TransportSecurityPolicy};

const RUNTIME_STATUS_TOOL: &str = "runtime_status";
const LIST_DIRECTORY_TOOL: &str = "list_directory";

#[derive(Clone)]
struct McpTransportState {
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
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
    arguments: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ListDirectoryArguments {
    path: String,
    #[serde(default)]
    max_depth: Option<u32>,
}

/// Build the staged MCP transport shell.
///
/// This route intentionally exposes only transport liveness, MCP discovery, one
/// deterministic read-only status tool, and a safe-rooted read-only directory
/// listing tool. File writes, Android platform access, command execution, and
/// high-impact actions remain unavailable until later independently validated
/// stages.
pub fn router(security_policy: TransportSecurityPolicy, file_tools: FileSystemTools) -> Router {
    Router::new()
        .route("/mcp", post(handle_mcp_request))
        .with_state(McpTransportState {
            security_policy,
            file_tools,
        })
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
                "message": "MCP transport is reachable. Tool discovery, runtime_status, and safe-rooted read-only directory listing are available; file writes, platform, command, and high-impact tools are not enabled in this stage.",
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
        id,
        method,
        params,
        ..
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
                        {
                            "name": LIST_DIRECTORY_TOOL,
                            "description": "List entries under a configured filesystem safe root without reading file contents or writing data.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": {
                                        "type": "string",
                                        "description": "Absolute path inside one configured safe root.",
                                    },
                                    "max_depth": {
                                        "type": "integer",
                                        "minimum": 1,
                                        "maximum": 5,
                                        "description": "Optional bounded traversal depth; defaults to 1 and is clamped to 5.",
                                    },
                                },
                                "required": ["path"],
                                "additionalProperties": false,
                            },
                        },
                    ],
                },
            })),
        )
            .into_response(),
        "tools/call" => handle_tool_call(id, params, &state.file_tools).await,
        _ => method_not_available(
            id,
            "Only initialize, tools/list, runtime_status, and safe-rooted list_directory are available in this staged runtime.",
        ),
    }
}

async fn handle_tool_call(
    id: Option<Value>,
    params: Option<Value>,
    file_tools: &FileSystemTools,
) -> Response {
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

    match call.name.as_str() {
        RUNTIME_STATUS_TOOL => runtime_status_response(id),
        LIST_DIRECTORY_TOOL => handle_list_directory_call(id, call.arguments, file_tools).await,
        _ => method_not_available(
            id,
            "Only runtime_status and safe-rooted read-only list_directory are available in this staged runtime.",
        ),
    }
}

fn runtime_status_response(id: Option<Value>) -> Response {
    (
        StatusCode::OK,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "result": {
                "content": [
                    {
                        "type": "text",
                        "text": "termux-mcp-edge runtime_status: transport=staged, tools=read-only-runtime-status-and-directory-listing, filesystem=list-directory-read-only, android_platform=disabled, command_execution=disabled",
                    },
                ],
                "structuredContent": {
                    "server": "termux-mcp-edge",
                    "version": env!("CARGO_PKG_VERSION"),
                    "transport": "staged_mcp_runtime",
                    "availableTools": [RUNTIME_STATUS_TOOL, LIST_DIRECTORY_TOOL],
                    "filesystemTools": "read_only_list_directory",
                    "fileWrites": false,
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

async fn handle_list_directory_call(
    id: Option<Value>,
    arguments: Option<Value>,
    file_tools: &FileSystemTools,
) -> Response {
    let arguments = match arguments {
        Some(arguments) => arguments,
        None => {
            return invalid_params(id, "list_directory requires a path argument.");
        }
    };

    let args = match serde_json::from_value::<ListDirectoryArguments>(arguments) {
        Ok(args) => args,
        Err(error) => {
            return invalid_params(id, &format!("Invalid list_directory arguments: {error}"));
        }
    };

    match file_tools.list_directory(args.path, args.max_depth).await {
        Ok(result) => (
            StatusCode::OK,
            Json(json!({
                "jsonrpc": "2.0",
                "id": id.unwrap_or(Value::Null),
                "result": {
                    "content": [
                        {
                            "type": "text",
                            "text": format!(
                                "Listed {} safe-rooted filesystem entries.",
                                result.entries.len()
                            ),
                        },
                    ],
                    "structuredContent": result,
                    "isError": false
                },
            })),
        )
            .into_response(),
        Err(error) => invalid_params(
            id,
            &format!("Filesystem safe-root validation failed: {error}"),
        ),
    }
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
    use tempfile::TempDir;
    use tower::ServiceExt;

    use super::*;

    fn test_file_tools() -> (TempDir, FileSystemTools) {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("visible.txt"), "safe content").unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        (root, tools)
    }

    fn test_router(file_tools: FileSystemTools) -> Router {
        router(TransportSecurityPolicy::localhost(8000, false), file_tools)
    }

    #[tokio::test]
    async fn transport_shell_accepts_valid_host_and_origin_without_tools() {
        let (_root, file_tools) = test_file_tools();
        let app = test_router(file_tools);

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
        let (_root, file_tools) = test_file_tools();
        let app = test_router(file_tools);

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
        let (_root, file_tools) = test_file_tools();
        let app = test_router(file_tools);

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
    async fn tool_discovery_returns_runtime_status_and_directory_listing_only() {
        let (_root, file_tools) = test_file_tools();
        let app = test_router(file_tools);
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
        });

        let response = app
            .oneshot(
                Request::post("/mcp")
                    .header(header::HOST, "localhost:8000")
                    .header(header::ORIGIN, "http://localhost:8000")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(request_body.to_string()))
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
        let tools = payload["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["name"], RUNTIME_STATUS_TOOL);
        assert_eq!(tools[1]["name"], LIST_DIRECTORY_TOOL);
        assert_eq!(tools[1]["inputSchema"]["additionalProperties"], false);
    }

    #[tokio::test]
    async fn runtime_status_tool_call_returns_deterministic_read_only_metadata() {
        let (_root, file_tools) = test_file_tools();
        let app = test_router(file_tools);
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": RUNTIME_STATUS_TOOL,
                "arguments": {},
            }
        });

        let response = app
            .oneshot(
                Request::post("/mcp")
                    .header(header::HOST, "localhost:8000")
                    .header(header::ORIGIN, "http://localhost:8000")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(request_body.to_string()))
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
            payload["result"]["structuredContent"]["availableTools"][1],
            LIST_DIRECTORY_TOOL
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
    async fn list_directory_tool_call_returns_safe_rooted_directory_entries() {
        let (root, file_tools) = test_file_tools();
        let app = test_router(file_tools);
        let safe_root = root.path().to_string_lossy().to_string();
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": LIST_DIRECTORY_TOOL,
                "arguments": {
                    "path": safe_root,
                    "max_depth": 1,
                }
            }
        });

        let response = app
            .oneshot(
                Request::post("/mcp")
                    .header(header::HOST, "localhost:8000")
                    .header(header::ORIGIN, "http://localhost:8000")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(request_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(payload["result"]["isError"], false);
        assert_eq!(
            payload["result"]["structuredContent"]["entries"][0]["is_dir"],
            false
        );
        assert!(payload["result"]["structuredContent"]["entries"][0]["path"]
            .as_str()
            .unwrap()
            .ends_with("visible.txt"));
    }

    #[tokio::test]
    async fn list_directory_tool_call_rejects_paths_outside_safe_roots() {
        let (_root, file_tools) = test_file_tools();
        let app = test_router(file_tools);
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": LIST_DIRECTORY_TOOL,
                "arguments": {
                    "path": "/",
                    "max_depth": 1,
                }
            }
        });

        let response = app
            .oneshot(
                Request::post("/mcp")
                    .header(header::HOST, "localhost:8000")
                    .header(header::ORIGIN, "http://localhost:8000")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(request_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(payload["jsonrpc"], "2.0");
        assert_eq!(payload["id"], 5);
        assert_eq!(payload["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn unknown_tool_call_remains_unavailable() {
        let (_root, file_tools) = test_file_tools();
        let app = test_router(file_tools);
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "unknown_tool",
            }
        });

        let response = app
            .oneshot(
                Request::post("/mcp")
                    .header(header::HOST, "localhost:8000")
                    .header(header::ORIGIN, "http://localhost:8000")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(request_body.to_string()))
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
