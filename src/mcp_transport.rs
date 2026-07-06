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
    android_status::collect_android_status,
    error::AppError,
    platform_info::collect_platform_info,
    service_status::{
        collect_project_service_status, unsupported_project_service_error,
        ProjectServiceStatusError, PROJECT_SERVICE_ALLOWLIST,
    },
    tools::FileSystemTools,
    transport_security::TransportSecurityPolicy,
    write_policy::{WriteMode, WritePolicy},
};

const RUNTIME_STATUS_TOOL: &str = "runtime_status";
const PLATFORM_INFO_TOOL: &str = "platform_info";
const ANDROID_STATUS_TOOL: &str = "android_status";
const PROJECT_SERVICE_STATUS_TOOL: &str = "project_service_status";
const LIST_DIRECTORY_TOOL: &str = "list_directory";
const READ_FILE_TOOL: &str = "read_file";
const WRITE_FILE_TOOL: &str = "write_file";
const AVAILABLE_TOOLS: [&str; 7] = [
    RUNTIME_STATUS_TOOL,
    PLATFORM_INFO_TOOL,
    ANDROID_STATUS_TOOL,
    PROJECT_SERVICE_STATUS_TOOL,
    LIST_DIRECTORY_TOOL,
    READ_FILE_TOOL,
    WRITE_FILE_TOOL,
];
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
    method: Option<String>,
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ToolCallParams {
    name: String,
    arguments: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectServiceStatusArguments {
    service_name: String,
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
/// deterministic runtime metadata, non-sensitive platform metadata,
/// read-only Android/Termux status metadata, read-only project-owned service
/// status metadata, safe-rooted directory listing, bounded safe-rooted UTF-8
/// reads, and default-dry-run safe-rooted writes. Android platform control,
/// command execution, and high-impact actions remain unavailable until later
/// independently validated stages.
pub fn router(security_policy: TransportSecurityPolicy, file_tools: FileSystemTools) -> Router {
    Router::new()
        .route("/mcp", post(handle_mcp_request))
        .with_state(McpTransportState {
            security_policy,
            file_tools,
        })
}

#[rustfmt::skip]
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
                "message": "MCP transport is reachable. Tool discovery, runtime_status, platform_info, android_status, project_service_status, list_directory, read_file, and write_file are available in this stage; later high-risk surfaces remain disabled.",
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

    let Some(method) = method else {
        return invalid_request(id, "JSON-RPC request is missing required method.");
    };

    match method.as_str() {
        "initialize" => initialize_response(id),
        "tools/list" => tools_list_response(id),
        "tools/call" => handle_tool_call(id, params, &state.file_tools).await,
        _ => method_not_available(
            id,
            "Only initialize, tools/list, runtime_status, platform_info, android_status, project_service_status, list_directory, read_file, and write_file are available in this staged runtime.",
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

#[rustfmt::skip]
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
                        "description": "Return deterministic staged runtime metadata.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {},
                            "additionalProperties": false,
                        },
                    },
                    {
                        "name": PLATFORM_INFO_TOOL,
                        "description": "Return non-sensitive platform metadata.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {},
                            "additionalProperties": false,
                        },
                    },
                    {
                        "name": ANDROID_STATUS_TOOL,
                        "description": "Return read-only allowlisted Android/Termux status metadata.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {},
                            "additionalProperties": false,
                        },
                    },
                    {
                        "name": PROJECT_SERVICE_STATUS_TOOL,
                        "description": "Return read-only status for one allowlisted project-owned logical service.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "service_name": {
                                    "type": "string",
                                    "enum": PROJECT_SERVICE_ALLOWLIST,
                                    "description": "Allowlisted logical project service identifier.",
                                },
                            },
                            "required": ["service_name"],
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
                ],
            },
        })),
    )
        .into_response()
}

#[rustfmt::skip]
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
        PLATFORM_INFO_TOOL => platform_info_response(id, call.arguments),
        ANDROID_STATUS_TOOL => android_status_response(id, call.arguments),
        PROJECT_SERVICE_STATUS_TOOL => project_service_status_response(id, call.arguments),
        LIST_DIRECTORY_TOOL => handle_list_directory_call(id, call.arguments, file_tools).await,
        READ_FILE_TOOL => handle_read_file_call(id, call.arguments, file_tools).await,
        WRITE_FILE_TOOL => handle_write_file_call(id, call.arguments, file_tools).await,
        _ => method_not_available(
            id,
            "Only runtime_status, platform_info, android_status, project_service_status, list_directory, read_file, and write_file are available in this staged runtime.",
        ),
    }
}

#[rustfmt::skip]
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
                        "text": "termux-mcp-edge runtime_status: transport=staged, tools=runtime-status-platform-info-android-status-project-service-status-directory-listing-read-file-and-default-dry-run-write-file, platform_info=read-only-non-sensitive, android_status=read-only-allowlisted-no-api-or-control, project_service_status=read-only-allowlisted, filesystem=list-read-and-dry-run-write-file, android_platform=disabled, command_execution=disabled",
                    },
                ],
                "structuredContent": {
                    "server": "termux-mcp-edge",
                    "version": env!("CARGO_PKG_VERSION"),
                    "transport": "staged_mcp_runtime",
                    "availableTools": AVAILABLE_TOOLS,
                    "platformInfo": true,
                    "platformInfoMode": "read_only_non_sensitive_metadata",
                    "androidStatus": true,
                    "androidStatusMode": "read_only_allowlisted_status_no_api_or_control",
                    "projectServiceStatus": true,
                    "projectServiceStatusMode": "read_only_allowlisted_project_service_status",
                    "filesystemTools": true,
                    "filesystemToolMode": "list_directory_read_file_and_default_dry_run_write_file",
                    "fileWrites": true,
                    "fileWriteMode": "dry_run_by_default_explicit_false_required",
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
    if let Some(arguments) = arguments {
        if arguments
            .as_object()
            .is_some_and(|object| !object.is_empty())
        {
            return invalid_params(id, "platform_info does not accept arguments.");
        }
    }

    let info = collect_platform_info();
    ok_result(
        id,
        format!(
            "platform_info: os={}, arch={}, family={}, parallelism={}, version={}",
            info.os, info.arch, info.family, info.available_parallelism, info.package_version
        ),
        json!(info),
    )
}

#[rustfmt::skip]
fn android_status_response(id: Option<Value>, arguments: Option<Value>) -> Response {
    if let Some(arguments) = arguments {
        if !arguments.is_object()
            || arguments
                .as_object()
                .is_some_and(|object| !object.is_empty())
        {
            return invalid_params(
                id,
                "android_status requires no arguments; arguments must be an empty object or omitted.",
            );
        }
    }

    let status = collect_android_status();
    ok_result(
        id,
        format!(
            "android_status: mode={}, target_os={}, target_arch={}, android_api_access={}, android_control_enabled={}, shell_fallback_enabled={}, command_execution_enabled={}, high_impact_controls_enabled={}",
            status.status_mode,
            status.target_os,
            status.target_arch,
            status.android_api_access,
            status.android_control_enabled,
            status.shell_fallback_enabled,
            status.command_execution_enabled,
            status.high_impact_controls_enabled,
        ),
        json!(status),
    )
}

fn project_service_status_response(id: Option<Value>, arguments: Option<Value>) -> Response {
    let arguments = match arguments {
        Some(arguments) => arguments,
        None => {
            return invalid_params(
                id,
                "project_service_status requires a service_name argument.",
            );
        }
    };

    if !arguments.is_object() {
        return invalid_params(
            id,
            "project_service_status arguments must be an object with service_name.",
        );
    }

    let args = match serde_json::from_value::<ProjectServiceStatusArguments>(arguments) {
        Ok(args) => args,
        Err(error) => {
            return invalid_params(
                id,
                &format!("Invalid project_service_status arguments: {error}"),
            );
        }
    };

    match collect_project_service_status(&args.service_name) {
        Ok(status) => ok_result(
            id,
            format!(
                "project_service_status: service_name={}, ownership={}, mode={}, lifecycle_state={}, health={}",
                status.service_name,
                status.ownership,
                status.status_mode,
                status.lifecycle_state,
                status.health,
            ),
            json!(status),
        ),
        Err(ProjectServiceStatusError::UnsupportedService { .. }) => invalid_params_json(
            id,
            "Invalid params",
            json!(unsupported_project_service_error(&args.service_name)),
        ),
    }
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

fn invalid_request(id: Option<Value>, message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "error": {
                "code": -32600,
                "message": "Invalid Request",
                "data": message,
            },
        })),
    )
        .into_response()
}

fn invalid_params(id: Option<Value>, message: &str) -> Response {
    invalid_params_json(id, "Invalid params", json!(message))
}

fn invalid_params_json(id: Option<Value>, message: &str, data: Value) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "error": {
                "code": -32602,
                "message": message,
                "data": data,
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
