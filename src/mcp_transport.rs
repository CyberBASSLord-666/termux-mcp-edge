use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    error::AppError,
    tools::FileSystemTools,
    transport_security::TransportSecurityPolicy,
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

#[derive(Debug, Serialize)]
struct PlatformInfoResult {
    os: String,
    arch: String,
    family: String,
}

pub fn router(
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
) -> Router {
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
                "message": "MCP transport is reachable. Tool discovery, runtime_status, safe‑rooted directory listing, safe‑rooted file read/write (dry‑run by default), and platform_info are available; high‑impact tools remain disabled in this stage.",
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

    let JsonRpcRequest { id, method, params, .. } = request;
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
                            "description": "Return deterministic read‑only runtime metadata for the staged Termux MCP Edge server.",
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
                                        "description": "Optional bounded traversal depth; defaults to 1 and must not exceed 5.",
                                    },
                                },
                                "required": ["path"],
                                "additionalProperties": false,
                            },
                        },
                        {
                            "name": READ_FILE_TOOL,
                            "description": "Read the contents of a file within a configured safe root.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string", "description": "Absolute path to a file within one safe root." },
                                },
                                "required": ["path"],
                                "additionalProperties": false,
                            },
                        },
                        {
                            "name": WRITE_FILE_TOOL,
                            "description": "Write data to a file within a configured safe root. Supports optional dry‑run.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string", "description": "Absolute path to a file within one safe root." },
                                    "content": { "type": "string", "description": "File contents to write." },
                                    "dry_run": { "type": "boolean", "description": "If true, validate the write without persisting it." },
                                },
                                "required": ["path", "content"],
                                "additionalProperties": false,
                            },
                        },
                        {
                            "name": PLATFORM_INFO_TOOL,
                            "description": "Return basic host platform information such as operating system and architecture.",
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
        "tools/call" => handle_tool_call(id, params, &state.file_tools).await,
        _ => method_not_available(
            id,
            "Only initialize, tools/list, runtime_status, list_directory, read_file, write_file, and platform_info are available in this staged runtime.",
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
        READ_FILE_TOOL => handle_read_file_call(id, call.arguments, file_tools).await,
        WRITE_FILE_TOOL => handle_write_file_call(id, call.arguments, file_tools).await,
        PLATFORM_INFO_TOOL => handle_platform_info_call(id).await,
        _ => method_not_available(
            id,
            "Requested tool is not available in this staged runtime.",
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
                        "text": "termux-mcp-edge runtime_status: transport=staged, tools=read-only-runtime-status,list-directory,file-io,platform-info, filesystem=safe-rooted-read-write, android_platform=limited, command_execution=disabled",
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
                    "filesystemToolMode": "safe_root_read_write",
                    "fileReads": true,
                    "fileWrites": true,
                    "writesDefaultDryRun": true,
                    "platformTools": true,
                    "platformToolMode": "read_only_info",
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
        Ok(result) => (
            StatusCode::OK,
            Json(json!({
                "jsonrpc": "2.0",
                "id": id.unwrap_or(Value::Null),
                "result": {
                    "content": [
                        {
                            "type": "text",
                            "text": format!("Listed {} safe-rooted filesystem entries.", result.entries.len()),
                        },
                    ],
                    "structuredContent": result,
                    "isError": false,
                },
            })),
        )
            .into_response(),
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
        None => {
            return invalid_params(id, "read_file requires a path argument.");
        }
    };
    let args = match serde_json::from_value::<ReadFileArguments>(arguments) {
        Ok(args) => args,
        Err(error) => {
            return invalid_params(id, &format!("Invalid read_file arguments: {error}"));
        }
    };
    match file_tools.read_file(args.path).await {
        Ok(result) => (
            StatusCode::OK,
            Json(json!({
                "jsonrpc": "2.0",
                "id": id.unwrap_or(Value::Null),
                "result": {
                    "content": [
                        {
                            "type": "text",
                            "text": format!("Read {} bytes.", result.size),
                        },
                    ],
                    "structuredContent": result,
                    "isError": false,
                },
            })),
        )
            .into_response(),
        Err(AppError::PathTraversal { .. }) => invalid_params(
            id,
            "Filesystem safe-root validation failed: requested path is outside the configured safe roots.",
        ),
        Err(_error) => internal_error(id, "Filesystem operation failed."),
    }
}

async fn handle_write_file_call(
    id: Option<Value>,
    arguments: Option<Value>,
    file_tools: &FileSystemTools,
) -> Response {
    let arguments = match arguments {
        Some(arguments) => arguments,
        None => {
            return invalid_params(id, "write_file requires path and content arguments.");
        }
    };
    let args = match serde_json::from_value::<WriteFileArguments>(arguments) {
        Ok(args) => args,
        Err(error) => {
            return invalid_params(id, &format!("Invalid write_file arguments: {error}"));
        }
    };
    match file_tools.write_file(args.path, args.content, args.dry_run).await {
        Ok(message) => (
            StatusCode::OK,
            Json(json!({
                "jsonrpc": "2.0",
                "id": id.unwrap_or(Value::Null),
                "result": {
                    "content": [
                        {
                            "type": "text",
                            "text": message.clone(),
                        },
                    ],
                    "structuredContent": {
                        "message": message,
                    },
                    "isError": false,
                },
            })),
        )
            .into_response(),
        Err(AppError::PathTraversal { .. }) => invalid_params(
            id,
            "Filesystem safe-root validation failed: requested path is outside the configured safe roots.",
        ),
        Err(_error) => internal_error(id, "Filesystem operation failed."),
    }
}

async fn handle_platform_info_call(id: Option<Value>) -> Response {
    let info = PlatformInfoResult {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        family: std::env::consts::FAMILY.to_string(),
    };
    (
        StatusCode::OK,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "result": {
                "content": [
                    {
                        "type": "text",
                        "text": format!("Platform: {} {} {}", info.os, info.arch, info.family),
                    },
                ],
                "structuredContent": info,
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
