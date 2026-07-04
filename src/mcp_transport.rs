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

use crate::{
    error::AppError,
    tools::FileSystemTools,
    transport_security::TransportSecurityPolicy,
    write_policy::{WriteMode, WritePolicy},
};

const RUNTIME_STATUS_TOOL: &str = "runtime_status";
const LIST_DIRECTORY_TOOL: &str = "list_directory";
const READ_FILE_TOOL: &str = "read_file";
const WRITE_FILE_TOOL: &str = "write_file";
const PLATFORM_INFO_TOOL: &str = "platform_info";
const MIN_LIST_DIRECTORY_DEPTH: u32 = 1;
const MAX_LIST_DIRECTORY_DEPTH: u32 = 5;

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

#[derive(Debug, Deserialize)]
struct ReadFileArguments {
    path: String,
}

#[derive(Debug, Deserialize)]
struct WriteFileArguments {
    path: String,
    content: String,
    #[serde(default)]
    dry_run: Option<bool>,
}

/// Build the staged MCP transport shell.
///
/// The staged runtime exposes transport liveness, MCP discovery,
/// deterministic runtime metadata, safe-rooted directory listing,
/// bounded safe-rooted UTF-8 reads, default-dry-run safe-rooted writes,
/// and read-only platform metadata. Android API access, command execution,
/// and high-impact actions remain unavailable until later independently
/// validated stages.
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
                "message": "MCP transport is reachable. Tool discovery, runtime_status, safe-rooted directory listing, bounded safe-rooted file reads, default-dry-run file writes, and read-only platform_info are available; Android APIs, command, and high-impact tools are not enabled in this stage.",
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
        "initialize" => initialize_response(id),
        "tools/list" => tools_list_response(id),
        "tools/call" => handle_tool_call(id, params, &state.file_tools).await,
        _ => method_not_available(
            id,
            "Only initialize, tools/list, runtime_status, safe-rooted list_directory, bounded safe-rooted read_file, default-dry-run write_file, and read-only platform_info are available in this staged runtime.",
        ),
    }
}

fn initialize_response(id: Option<Value>) -> Response {
    (
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
        .into_response()
}

fn tools_list_response(id: Option<Value>) -> Response {
    (
        StatusCode::OK,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "result": {
                "tools": [
                    {
                        "name": RUNTIME_STATUS_TOOL,
                        "description": "Return deterministic runtime metadata for the staged Termux MCP Edge server.",
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
                                    "minimum": MIN_LIST_DIRECTORY_DEPTH,
                                    "maximum": MAX_LIST_DIRECTORY_DEPTH,
                                    "description": format!(
                                        "Optional bounded traversal depth; defaults to {MIN_LIST_DIRECTORY_DEPTH} and must not exceed {MAX_LIST_DIRECTORY_DEPTH}."
                                    ),
                                },
                            },
                            "required": ["path"],
                            "additionalProperties": false,
                        },
                    },
                    {
                        "name": READ_FILE_TOOL,
                        "description": "Read a bounded UTF-8 text file from inside a configured filesystem safe root without writing data.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "path": {
                                    "type": "string",
                                    "description": "Absolute path to a UTF-8 text file inside one configured safe root.",
                                },
                            },
                            "required": ["path"],
                            "additionalProperties": false,
                        },
                    },
                    {
                        "name": WRITE_FILE_TOOL,
                        "description": "Write UTF-8 text to a safe-rooted file. Defaults to dry-run; mutation requires explicit dry_run=false.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "path": {
                                    "type": "string",
                                    "description": "Absolute destination path inside one configured safe root.",
                                },
                                "content": {
                                    "type": "string",
                                    "description": "UTF-8 text content to write, subject to the staged byte limit.",
                                },
                                "dry_run": {
                                    "type": "boolean",
                                    "description": "Defaults to true. Set explicitly to false to perform the write.",
                                },
                            },
                            "required": ["path", "content"],
                            "additionalProperties": false,
                        },
                    },
                    {
                        "name": PLATFORM_INFO_TOOL,
                        "description": "Return read-only compile-time platform metadata without Android APIs, filesystem reads, command execution, or device identifiers.",
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
        .into_response()
}

async fn handle_tool_call(
    id: Option<Value>,
    params: Option<Value>,
    file_tools: &FileSystemTools,
) -> Response {
    let params = match params {
        Some(params) => params,
        None => return invalid_params(id, "tools/call requires params with a tool name."),
    };

    let call = match serde_json::from_value::<ToolCallParams>(params) {
        Ok(call) => call,
        Err(error) => return invalid_params(id, &format!("Invalid tools/call params: {error}")),
    };

    match call.name.as_str() {
        RUNTIME_STATUS_TOOL => runtime_status_response(id),
        LIST_DIRECTORY_TOOL => handle_list_directory_call(id, call.arguments, file_tools).await,
        READ_FILE_TOOL => handle_read_file_call(id, call.arguments, file_tools).await,
        WRITE_FILE_TOOL => handle_write_file_call(id, call.arguments, file_tools).await,
        PLATFORM_INFO_TOOL => platform_info_response(id, call.arguments),
        _ => method_not_available(
            id,
            "Only runtime_status, safe-rooted list_directory, bounded safe-rooted read_file, default-dry-run write_file, and read-only platform_info are available in this staged runtime.",
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
                        "text": "termux-mcp-edge runtime_status: transport=staged, tools=runtime-status-directory-listing-read-file-default-dry-run-write-file-and-platform-info, filesystem=list-read-and-dry-run-write-file, platform_info=read_only_compile_time_metadata, android_platform=disabled, command_execution=disabled",
                    },
                ],
                "structuredContent": {
                    "server": "termux-mcp-edge",
                    "version": env!("CARGO_PKG_VERSION"),
                    "transport": "staged_mcp_runtime",
                    "availableTools": [
                        RUNTIME_STATUS_TOOL,
                        LIST_DIRECTORY_TOOL,
                        READ_FILE_TOOL,
                        WRITE_FILE_TOOL,
                        PLATFORM_INFO_TOOL,
                    ],
                    "filesystemTools": true,
                    "filesystemToolMode": "list_directory_read_file_and_default_dry_run_write_file",
                    "fileWrites": true,
                    "fileWriteMode": "dry_run_by_default_explicit_false_required",
                    "platformTools": true,
                    "platformToolMode": "read_only_compile_time_metadata",
                    "androidPlatformTools": false,
                    "commandExecution": false,
                    "highImpactTools": false,
                },
                "isError": false
            },
        })),
    )
        .into_response()
}

fn platform_info_response(id: Option<Value>, arguments: Option<Value>) -> Response {
    match arguments {
        None | Some(Value::Null) => {}
        Some(Value::Object(ref map)) if map.is_empty() => {}
        _ => return invalid_params(id, "platform_info does not accept arguments."),
    }

    ok_result(
        id,
        format!(
            "platform_info: os={}, arch={}, family={}",
            std::env::consts::OS,
            std::env::consts::ARCH,
            std::env::consts::FAMILY
        ),
        json!({
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "family": std::env::consts::FAMILY,
            "source": "std::env::consts",
            "androidApiAccess": false,
            "filesystemAccess": false,
            "commandExecution": false,
            "deviceIdentifiers": false,
        }),
    )
}

async fn handle_list_directory_call(
    id: Option<Value>,
    arguments: Option<Value>,
    file_tools: &FileSystemTools,
) -> Response {
    let arguments = match arguments {
        Some(arguments) => arguments,
        None => return invalid_params(id, "list_directory requires a path argument."),
    };

    let args = match serde_json::from_value::<ListDirectoryArguments>(arguments) {
        Ok(args) => args,
        Err(error) => {
            return invalid_params(id, &format!("Invalid list_directory arguments: {error}"))
        }
    };

    if let Some(max_depth) = args.max_depth {
        if !(MIN_LIST_DIRECTORY_DEPTH..=MAX_LIST_DIRECTORY_DEPTH).contains(&max_depth) {
            return invalid_params(
                id,
                &format!(
                    "list_directory.max_depth must be between {MIN_LIST_DIRECTORY_DEPTH} and {MAX_LIST_DIRECTORY_DEPTH}."
                ),
            );
        }
    }

    match file_tools.list_directory(args.path, args.max_depth).await {
        Ok(result) => ok_result(
            id,
            format!("Listed {} safe-rooted filesystem entries.", result.entries.len()),
            json!(result),
        ),
        Err(AppError::PathTraversal { .. }) => invalid_params(
            id,
            "Filesystem safe-root validation failed: requested path is outside the configured safe roots.",
        ),
        Err(_error) => internal_error(id, "Filesystem operation failed."),
    }
}

async fn handle_read_file_call(
    id: Option<Value>,
    arguments: Option<Value>,
    file_tools: &FileSystemTools,
) -> Response {
    let arguments = match arguments {
        Some(arguments) => arguments,
        None => return invalid_params(id, "read_file requires a path argument."),
    };

    let args = match serde_json::from_value::<ReadFileArguments>(arguments) {
        Ok(args) => args,
        Err(error) => return invalid_params(id, &format!("Invalid read_file arguments: {error}")),
    };

    match file_tools.read_file(args.path).await {
        Ok(result) => ok_result(id, result.content.clone(), json!(result)),
        Err(AppError::PathTraversal { .. }) => invalid_params(
            id,
            "Filesystem safe-root validation failed: requested path is outside the configured safe roots.",
        ),
        Err(AppError::FileTooLarge { .. }) => payload_too_large(
            id,
            "File content exceeds the staged read_file byte limit.",
        ),
        Err(_error) => internal_error(id, "Filesystem read failed."),
    }
}

async fn handle_write_file_call(
    id: Option<Value>,
    arguments: Option<Value>,
    file_tools: &FileSystemTools,
) -> Response {
    let arguments = match arguments {
        Some(arguments) => arguments,
        None => return invalid_params(id, "write_file requires path and content arguments."),
    };

    let args = match serde_json::from_value::<WriteFileArguments>(arguments) {
        Ok(args) => args,
        Err(error) => return invalid_params(id, &format!("Invalid write_file arguments: {error}")),
    };

    let policy = WritePolicy::default();
    let bytes = args.content.len();
    if policy.validate_payload_size(bytes).is_err() {
        return payload_too_large(id, "File content exceeds the staged write_file byte limit.");
    }

    let dry_run = matches!(policy.resolve_mode(args.dry_run), WriteMode::DryRun);

    match file_tools
        .write_file(args.path, args.content, Some(dry_run))
        .await
    {
        Ok(message) => ok_result(
            id,
            message.clone(),
            json!({
                "dryRun": dry_run,
                "bytes": bytes,
                "message": message,
            }),
        ),
        Err(AppError::PathTraversal { .. }) => invalid_params(
            id,
            "Filesystem safe-root validation failed: requested path is outside the configured safe roots.",
        ),
        Err(AppError::FileTooLarge { .. }) => payload_too_large(
            id,
            "File content exceeds the staged write_file byte limit.",
        ),
        Err(_error) => internal_error(id, "Filesystem write failed."),
    }
}

fn ok_result(id: Option<Value>, text: String, structured_content: Value) -> Response {
    (
        StatusCode::OK,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "result": {
                "content": [
                    {
                        "type": "text",
                        "text": text,
                    },
                ],
                "structuredContent": structured_content,
                "isError": false
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

fn internal_error(id: Option<Value>, message: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "error": {
                "code": -32603,
                "message": "Internal error",
                "data": message,
            },
        })),
    )
        .into_response()
}

fn payload_too_large(id: Option<Value>, message: &str) -> Response {
    (
        StatusCode::PAYLOAD_TOO_LARGE,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "error": {
                "code": -32001,
                "message": "Payload too large",
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

    async fn post_json(app: Router, request_body: Value) -> Response {
        app.oneshot(
            Request::post("/mcp")
                .header(header::HOST, "localhost:8000")
                .header(header::ORIGIN, "http://localhost:8000")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(request_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn response_json(response: Response) -> Value {
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
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
    async fn tool_discovery_returns_staged_filesystem_and_platform_tools() {
        let (_root, file_tools) = test_file_tools();
        let app = test_router(file_tools);
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
        });

        let response = post_json(app, request_body).await;
        assert_eq!(response.status(), StatusCode::OK);

        let payload = response_json(response).await;
        let tools = payload["result"]["tools"].as_array().unwrap();

        assert_eq!(tools.len(), 5);
        assert_eq!(tools[0]["name"], RUNTIME_STATUS_TOOL);
        assert_eq!(tools[1]["name"], LIST_DIRECTORY_TOOL);
        assert_eq!(tools[2]["name"], READ_FILE_TOOL);
        assert_eq!(tools[3]["name"], WRITE_FILE_TOOL);
        assert_eq!(tools[4]["name"], PLATFORM_INFO_TOOL);
        assert_eq!(tools[4]["inputSchema"]["additionalProperties"], false);
    }

    #[tokio::test]
    async fn runtime_status_tool_call_reports_default_dry_run_write_mode() {
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

        let response = post_json(app, request_body).await;
        assert_eq!(response.status(), StatusCode::OK);

        let payload = response_json(response).await;
        assert_eq!(payload["result"]["isError"], false);
        assert_eq!(
            payload["result"]["structuredContent"]["filesystemToolMode"],
            "list_directory_read_file_and_default_dry_run_write_file"
        );
        assert_eq!(payload["result"]["structuredContent"]["fileWrites"], true);
        assert_eq!(
            payload["result"]["structuredContent"]["fileWriteMode"],
            "dry_run_by_default_explicit_false_required"
        );
        assert_eq!(
            payload["result"]["structuredContent"]["platformTools"],
            true
        );
        assert_eq!(
            payload["result"]["structuredContent"]["platformToolMode"],
            "read_only_compile_time_metadata"
        );
        assert_eq!(
            payload["result"]["structuredContent"]["androidPlatformTools"],
            false
        );
        assert_eq!(
            payload["result"]["structuredContent"]["commandExecution"],
            false
        );
        assert_eq!(
            payload["result"]["structuredContent"]["highImpactTools"],
            false
        );
    }

    #[tokio::test]
    async fn platform_info_tool_call_returns_read_only_compile_time_metadata() {
        let (_root, file_tools) = test_file_tools();
        let app = test_router(file_tools);
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": PLATFORM_INFO_TOOL,
                "arguments": {},
            }
        });

        let response = post_json(app, request_body).await;
        assert_eq!(response.status(), StatusCode::OK);

        let payload = response_json(response).await;
        assert_eq!(payload["result"]["isError"], false);
        assert_eq!(
            payload["result"]["structuredContent"]["os"],
            std::env::consts::OS
        );
        assert_eq!(
            payload["result"]["structuredContent"]["arch"],
            std::env::consts::ARCH
        );
        assert_eq!(
            payload["result"]["structuredContent"]["family"],
            std::env::consts::FAMILY
        );
        assert_eq!(
            payload["result"]["structuredContent"]["androidApiAccess"],
            false
        );
        assert_eq!(
            payload["result"]["structuredContent"]["commandExecution"],
            false
        );
    }

    #[tokio::test]
    async fn platform_info_tool_call_rejects_arguments() {
        for arguments in [
            json!({"unexpected": true}),
            json!(["unexpected"]),
            json!("unexpected"),
            json!(true),
        ] {
            let (_root, file_tools) = test_file_tools();
            let app = test_router(file_tools);
            let request_body = json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": PLATFORM_INFO_TOOL,
                    "arguments": arguments,
                }
            });

            let response = post_json(app, request_body).await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
    }

    #[tokio::test]
    async fn list_directory_tool_call_returns_safe_rooted_directory_entries() {
        let (root, file_tools) = test_file_tools();
        let app = test_router(file_tools);
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": LIST_DIRECTORY_TOOL,
                "arguments": {
                    "path": root.path().to_string_lossy().to_string(),
                    "max_depth": 1,
                }
            }
        });

        let response = post_json(app, request_body).await;
        assert_eq!(response.status(), StatusCode::OK);

        let payload = response_json(response).await;
        assert!(payload["result"]["structuredContent"]["entries"][0]["path"]
            .as_str()
            .unwrap()
            .ends_with("visible.txt"));
    }

    #[tokio::test]
    async fn read_file_tool_call_returns_safe_rooted_file_content() {
        let (root, file_tools) = test_file_tools();
        let app = test_router(file_tools);
        let safe_file = root
            .path()
            .join("visible.txt")
            .to_string_lossy()
            .to_string();
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": READ_FILE_TOOL,
                "arguments": {
                    "path": safe_file,
                }
            }
        });

        let response = post_json(app, request_body).await;
        assert_eq!(response.status(), StatusCode::OK);

        let payload = response_json(response).await;
        assert_eq!(payload["result"]["content"][0]["text"], "safe content");
        assert_eq!(
            payload["result"]["structuredContent"]["content"],
            "safe content"
        );
    }

    #[tokio::test]
    async fn read_file_tool_call_rejects_oversized_files() {
        let (root, file_tools) = test_file_tools();
        let app = test_router(file_tools);
        let too_large_file = root.path().join("too_large.txt");
        std::fs::write(&too_large_file, vec![b'a'; 1_048_577]).unwrap();
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "tools/call",
            "params": {
                "name": READ_FILE_TOOL,
                "arguments": {
                    "path": too_large_file.to_string_lossy().to_string(),
                }
            }
        });

        let response = post_json(app, request_body).await;
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn write_file_tool_call_defaults_to_dry_run_without_mutating() {
        let (root, file_tools) = test_file_tools();
        let app = test_router(file_tools);
        let target = root.path().join("dry_run_default.txt");
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "tools/call",
            "params": {
                "name": WRITE_FILE_TOOL,
                "arguments": {
                    "path": target.to_string_lossy().to_string(),
                    "content": "not written",
                }
            }
        });

        let response = post_json(app, request_body).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!target.exists());

        let payload = response_json(response).await;
        assert_eq!(payload["result"]["structuredContent"]["dryRun"], true);
    }

    #[tokio::test]
    async fn write_file_tool_call_allows_explicit_safe_rooted_mutation() {
        let (root, file_tools) = test_file_tools();
        let app = test_router(file_tools);
        let target = root.path().join("write_enabled.txt");
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "tools/call",
            "params": {
                "name": WRITE_FILE_TOOL,
                "arguments": {
                    "path": target.to_string_lossy().to_string(),
                    "content": "written through mcp",
                    "dry_run": false,
                }
            }
        });

        let response = post_json(app, request_body).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            std::fs::read_to_string(target).unwrap(),
            "written through mcp"
        );

        let payload = response_json(response).await;
        assert_eq!(payload["result"]["structuredContent"]["dryRun"], false);
    }

    #[tokio::test]
    async fn write_file_tool_call_rejects_oversized_payloads() {
        let (root, file_tools) = test_file_tools();
        let app = test_router(file_tools);
        let target = root.path().join("too_large_write.txt");
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": 12,
            "method": "tools/call",
            "params": {
                "name": WRITE_FILE_TOOL,
                "arguments": {
                    "path": target.to_string_lossy().to_string(),
                    "content": "a".repeat(1_048_577),
                    "dry_run": false,
                }
            }
        });

        let response = post_json(app, request_body).await;
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn write_file_tool_call_rejects_paths_outside_safe_roots() {
        let (_root, file_tools) = test_file_tools();
        let app = test_router(file_tools);
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": 13,
            "method": "tools/call",
            "params": {
                "name": WRITE_FILE_TOOL,
                "arguments": {
                    "path": "/etc/write_file_blocked.txt",
                    "content": "blocked",
                    "dry_run": false,
                }
            }
        });

        let response = post_json(app, request_body).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
