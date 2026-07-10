use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

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
    audit::{
        filesystem_allowed_event, filesystem_denied_event, read_only_allowed_event,
        read_only_denied_event, AuditCounters, AuditMode,
    },
    error::AppError,
    json_rpc::{parse_incoming_message, IncomingJsonRpcMessage, JsonRpcEnvelopeError},
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

const RUNTIME_STATUS_GATE: &str = "runtime_metadata";
const PLATFORM_INFO_GATE: &str = "platform_metadata";
const ANDROID_STATUS_GATE: &str = "android_read_only_status";
const PROJECT_SERVICE_STATUS_GATE: &str = "project_service_state";
const FILESYSTEM_READ_GATE: &str = "filesystem_read";
const FILESYSTEM_WRITE_GATE: &str = "filesystem_write";

const RUNTIME_STATUS_ALLOWED: &str = "staged_runtime_metadata";
const PLATFORM_INFO_ALLOWED: &str = "read_only_platform_metadata";
const PLATFORM_INFO_ARGUMENTS_DENIED: &str = "arguments_not_supported";
const ANDROID_STATUS_ALLOWED: &str = "allowlisted_status_metadata";
const ANDROID_STATUS_ARGUMENTS_DENIED: &str = "arguments_not_empty_or_not_object";
const PROJECT_SERVICE_STATUS_ALLOWED: &str = "allowlisted_project_service";
const PROJECT_SERVICE_STATUS_MISSING_ARGUMENTS: &str = "missing_service_name";
const PROJECT_SERVICE_STATUS_INVALID_ARGUMENTS: &str = "invalid_service_arguments";
const PROJECT_SERVICE_STATUS_UNSUPPORTED: &str = "unsupported_service";
const FILESYSTEM_MISSING_ARGUMENTS: &str = "missing_arguments";
const FILESYSTEM_INVALID_ARGUMENTS: &str = "invalid_arguments";
const FILESYSTEM_INVALID_DEPTH: &str = "invalid_max_depth";
const FILESYSTEM_SAFE_ROOT_REJECTED: &str = "safe_root_rejected";
const FILESYSTEM_LIST_ALLOWED: &str = "safe_root_listed";
const FILESYSTEM_READ_ALLOWED: &str = "safe_root_read";
const FILESYSTEM_READ_TOO_LARGE: &str = "read_size_limit_exceeded";
const FILESYSTEM_READ_FAILED: &str = "filesystem_read_failed";
const FILESYSTEM_DRY_RUN_ALLOWED: &str = "dry_run_preview";
const FILESYSTEM_WRITE_ALLOWED: &str = "explicit_write_allowed";
const FILESYSTEM_WRITE_TOO_LARGE: &str = "write_size_limit_exceeded";
const FILESYSTEM_WRITE_FAILED: &str = "filesystem_write_failed";

type SharedAuditCounters = Arc<Mutex<AuditCounters>>;

#[derive(Clone)]
struct McpTransportState {
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    audit_counters: SharedAuditCounters,
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
#[rustfmt::skip]
pub fn router(security_policy: TransportSecurityPolicy, file_tools: FileSystemTools) -> Router {
    Router::new()
        .route("/mcp", post(handle_mcp_request))
        .with_state(McpTransportState {
            security_policy,
            file_tools,
            audit_counters: Arc::new(Mutex::new(AuditCounters::default())),
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

    let message = match parse_incoming_message(&body) {
        Ok(message) => message,
        Err(JsonRpcEnvelopeError::ParseError { detail }) => {
            return (
  StatusCode::BAD_REQUEST,
  Json(json!({
      "jsonrpc": "2.0",
      "id": Value::Null,
      "error": {
          "code": -32700,
          "message": "Parse error",
          "data": detail,
      },
  })),
            )
  .into_response();
        }
        Err(JsonRpcEnvelopeError::InvalidRequest { id, reason }) => {
            return invalid_request(id, reason);
        }
    };

    match message {
        IncomingJsonRpcMessage::Request { id, method, params } => match method.as_str() {
            "initialize" => initialize_response(Some(id)),
            "tools/list" => tools_list_response(Some(id)),
            "tools/call" => handle_tool_call(Some(id), params, &state).await,
            _ => method_not_available(
  Some(id),
  "Only initialize, tools/list, and tools/call are available in this staged runtime.",
            ),
        },
        IncomingJsonRpcMessage::Notification { method, params: _ } => {
            handle_notification(&method)
        }
    }

}

fn handle_notification(method: &str) -> Response {
    match method {
        "notifications/initialized" => StatusCode::NO_CONTENT.into_response(),
        _ => StatusCode::NO_CONTENT.into_response(),
    }
}

#[rustfmt::skip]
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
    state: &McpTransportState,
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
        RUNTIME_STATUS_TOOL => {
            record_read_only_allowed(
                &state.audit_counters,
                RUNTIME_STATUS_TOOL,
                RUNTIME_STATUS_GATE,
                RUNTIME_STATUS_ALLOWED,
            );
            runtime_status_response(id, &state.audit_counters)
        }
        PLATFORM_INFO_TOOL => platform_info_response(id, call.arguments, &state.audit_counters),
        ANDROID_STATUS_TOOL => android_status_response(id, call.arguments, &state.audit_counters),
        PROJECT_SERVICE_STATUS_TOOL => {
            project_service_status_response(id, call.arguments, &state.audit_counters)
        }
        LIST_DIRECTORY_TOOL => {
            handle_list_directory_call(id, call.arguments, &state.file_tools, &state.audit_counters).await
        }
        READ_FILE_TOOL => {
            handle_read_file_call(id, call.arguments, &state.file_tools, &state.audit_counters).await
        }
        WRITE_FILE_TOOL => {
            handle_write_file_call(id, call.arguments, &state.file_tools, &state.audit_counters).await
        }
        _ => method_not_available(
            id,
            "Only runtime_status, platform_info, android_status, project_service_status, list_directory, read_file, and write_file are available in this staged runtime.",
        ),
    }
}

#[rustfmt::skip]
fn runtime_status_response(id: Option<Value>, audit_counters: &SharedAuditCounters) -> Response {
    let audit_counters_snapshot = audit_counters_snapshot(audit_counters);

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
                    "auditCounters": audit_counters_snapshot,
                },
                "isError": false
            },
        })),
    )
        .into_response()
}

#[rustfmt::skip]
fn platform_info_response(
    id: Option<Value>,
    arguments: Option<Value>,
    audit_counters: &SharedAuditCounters,
) -> Response {
    if let Some(arguments) = arguments {
        if arguments
            .as_object()
            .is_some_and(|object| !object.is_empty())
        {
            record_read_only_denied(
                audit_counters,
                PLATFORM_INFO_TOOL,
                PLATFORM_INFO_GATE,
                PLATFORM_INFO_ARGUMENTS_DENIED,
            );
            return invalid_params(id, "platform_info does not accept arguments.");
        }
    }

    record_read_only_allowed(
        audit_counters,
        PLATFORM_INFO_TOOL,
        PLATFORM_INFO_GATE,
        PLATFORM_INFO_ALLOWED,
    );
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
fn android_status_response(
    id: Option<Value>,
    arguments: Option<Value>,
    audit_counters: &SharedAuditCounters,
) -> Response {
    if let Some(arguments) = arguments {
        if !arguments.is_object()
            || arguments
                .as_object()
                .is_some_and(|object| !object.is_empty())
        {
            record_read_only_denied(
                audit_counters,
                ANDROID_STATUS_TOOL,
                ANDROID_STATUS_GATE,
                ANDROID_STATUS_ARGUMENTS_DENIED,
            );
            return invalid_params(
                id,
                "android_status requires no arguments; arguments must be an empty object or omitted.",
            );
        }
    }

    record_read_only_allowed(
        audit_counters,
        ANDROID_STATUS_TOOL,
        ANDROID_STATUS_GATE,
        ANDROID_STATUS_ALLOWED,
    );
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

#[rustfmt::skip]
fn project_service_status_response(
    id: Option<Value>,
    arguments: Option<Value>,
    audit_counters: &SharedAuditCounters,
) -> Response {
    let arguments = match arguments {
        Some(arguments) => arguments,
        None => {
            record_read_only_denied(
                audit_counters,
                PROJECT_SERVICE_STATUS_TOOL,
                PROJECT_SERVICE_STATUS_GATE,
                PROJECT_SERVICE_STATUS_MISSING_ARGUMENTS,
            );
            return invalid_params(
                id,
                "project_service_status requires a service_name argument.",
            );
        }
    };

    if !arguments.is_object() {
        record_read_only_denied(
            audit_counters,
            PROJECT_SERVICE_STATUS_TOOL,
            PROJECT_SERVICE_STATUS_GATE,
            PROJECT_SERVICE_STATUS_INVALID_ARGUMENTS,
        );
        return invalid_params(
            id,
            "project_service_status arguments must be an object with service_name.",
        );
    }

    let args = match serde_json::from_value::<ProjectServiceStatusArguments>(arguments) {
        Ok(args) => args,
        Err(error) => {
            record_read_only_denied(
                audit_counters,
                PROJECT_SERVICE_STATUS_TOOL,
                PROJECT_SERVICE_STATUS_GATE,
                PROJECT_SERVICE_STATUS_INVALID_ARGUMENTS,
            );
            return invalid_params(
                id,
                &format!("Invalid project_service_status arguments: {error}"),
            );
        }
    };

    match collect_project_service_status(&args.service_name) {
        Ok(status) => {
            record_read_only_allowed(
                audit_counters,
                PROJECT_SERVICE_STATUS_TOOL,
                PROJECT_SERVICE_STATUS_GATE,
                PROJECT_SERVICE_STATUS_ALLOWED,
            );
            ok_result(
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
            )
        }
        Err(ProjectServiceStatusError::UnsupportedService { .. }) => {
            record_read_only_denied(
                audit_counters,
                PROJECT_SERVICE_STATUS_TOOL,
                PROJECT_SERVICE_STATUS_GATE,
                PROJECT_SERVICE_STATUS_UNSUPPORTED,
            );
            invalid_params_json(
                id,
                "Invalid params",
                json!(unsupported_project_service_error(&args.service_name)),
            )
        }
    }
}

#[rustfmt::skip]
async fn handle_list_directory_call(
    id: Option<Value>,
    arguments: Option<Value>,
    file_tools: &FileSystemTools,
    audit_counters: &SharedAuditCounters,
) -> Response {
    let arguments = match arguments {
        Some(arguments) => arguments,
        None => {
            record_filesystem_denied(
                audit_counters,
                LIST_DIRECTORY_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_MISSING_ARGUMENTS,
            );
            return invalid_params(id, "list_directory requires a path argument.");
        }
    };

    let args = match serde_json::from_value::<ListDirectoryArguments>(arguments) {
        Ok(args) => args,
        Err(error) => {
            record_filesystem_denied(
                audit_counters,
                LIST_DIRECTORY_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_INVALID_ARGUMENTS,
            );
            return invalid_params(id, &format!("Invalid list_directory arguments: {error}"));
        }
    };

    if let Some(max_depth) = args.max_depth {
        if !(MIN_LIST_DIRECTORY_DEPTH..=MAX_LIST_DIRECTORY_DEPTH).contains(&max_depth) {
            record_filesystem_denied(
                audit_counters,
                LIST_DIRECTORY_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_INVALID_DEPTH,
            );
            return invalid_params(
                id,
                &format!(
                    "list_directory.max_depth must be between {MIN_LIST_DIRECTORY_DEPTH} and {MAX_LIST_DIRECTORY_DEPTH}."
                ),
            );
        }
    }

    match file_tools.list_directory(args.path, args.max_depth).await {
        Ok(result) => {
            record_filesystem_allowed(
                audit_counters,
                LIST_DIRECTORY_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_LIST_ALLOWED,
            );
            ok_result(
                id,
                format!("Listed {} safe-rooted filesystem entries.", result.entries.len()),
                json!(result),
            )
        }
        Err(AppError::PathTraversal { .. }) => {
            record_filesystem_denied(
                audit_counters,
                LIST_DIRECTORY_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_SAFE_ROOT_REJECTED,
            );
            invalid_params(
                id,
                "Filesystem safe-root validation failed: requested path is outside the configured safe roots.",
            )
        }
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                LIST_DIRECTORY_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_READ_FAILED,
            );
            internal_error(id, "Filesystem operation failed.")
        }
    }
}

#[rustfmt::skip]
async fn handle_read_file_call(
    id: Option<Value>,
    arguments: Option<Value>,
    file_tools: &FileSystemTools,
    audit_counters: &SharedAuditCounters,
) -> Response {
    let arguments = match arguments {
        Some(arguments) => arguments,
        None => {
            record_filesystem_denied(
                audit_counters,
                READ_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_MISSING_ARGUMENTS,
            );
            return invalid_params(id, "read_file requires a path argument.");
        }
    };

    let args = match serde_json::from_value::<ReadFileArguments>(arguments) {
        Ok(args) => args,
        Err(error) => {
            record_filesystem_denied(
                audit_counters,
                READ_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_INVALID_ARGUMENTS,
            );
            return invalid_params(id, &format!("Invalid read_file arguments: {error}"));
        }
    };

    match file_tools.read_file(args.path).await {
        Ok(result) => {
            record_filesystem_allowed(
                audit_counters,
                READ_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_READ_ALLOWED,
            );
            ok_result(id, result.content.clone(), json!(result))
        }
        Err(AppError::PathTraversal { .. }) => {
            record_filesystem_denied(
                audit_counters,
                READ_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_SAFE_ROOT_REJECTED,
            );
            invalid_params(
                id,
                "Filesystem safe-root validation failed: requested path is outside the configured safe roots.",
            )
        }
        Err(AppError::FileTooLarge { .. }) => {
            record_filesystem_denied(
                audit_counters,
                READ_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_READ_TOO_LARGE,
            );
            payload_too_large(
                id,
                "File content exceeds the staged read_file byte limit.",
            )
        }
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                READ_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_READ_FAILED,
            );
            internal_error(id, "Filesystem read failed.")
        }
    }
}

#[rustfmt::skip]
async fn handle_write_file_call(
    id: Option<Value>,
    arguments: Option<Value>,
    file_tools: &FileSystemTools,
    audit_counters: &SharedAuditCounters,
) -> Response {
    let arguments = match arguments {
        Some(arguments) => arguments,
        None => {
            record_filesystem_denied(
                audit_counters,
                WRITE_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::DryRun,
                FILESYSTEM_MISSING_ARGUMENTS,
            );
            return invalid_params(id, "write_file requires path and content arguments.");
        }
    };

    let args = match serde_json::from_value::<WriteFileArguments>(arguments) {
        Ok(args) => args,
        Err(error) => {
            record_filesystem_denied(
                audit_counters,
                WRITE_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::DryRun,
                FILESYSTEM_INVALID_ARGUMENTS,
            );
            return invalid_params(id, &format!("Invalid write_file arguments: {error}"));
        }
    };

    let policy = WritePolicy::default();
    let bytes = args.content.len();
    let dry_run = matches!(policy.resolve_mode(args.dry_run), WriteMode::DryRun);
    let mode = filesystem_write_mode(dry_run);

    if policy.validate_payload_size(bytes).is_err() {
        record_filesystem_denied(
            audit_counters,
            WRITE_FILE_TOOL,
            FILESYSTEM_WRITE_GATE,
            mode,
            FILESYSTEM_WRITE_TOO_LARGE,
        );
        return payload_too_large(id, "File content exceeds the staged write_file byte limit.");
    }

    match file_tools
        .write_file(args.path, args.content, Some(dry_run))
        .await
    {
        Ok(message) => {
            record_filesystem_allowed(
                audit_counters,
                WRITE_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                mode,
                filesystem_write_allowed_reason(dry_run),
            );
            ok_result(
                id,
                message.clone(),
                json!({
                    "dryRun": dry_run,
                    "bytes": bytes,
                    "message": message,
                }),
            )
        }
        Err(AppError::PathTraversal { .. }) => {
            record_filesystem_denied(
                audit_counters,
                WRITE_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                mode,
                FILESYSTEM_SAFE_ROOT_REJECTED,
            );
            invalid_params(
                id,
                "Filesystem safe-root validation failed: requested path is outside the configured safe roots.",
            )
        }
        Err(AppError::FileTooLarge { .. }) => {
            record_filesystem_denied(
                audit_counters,
                WRITE_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                mode,
                FILESYSTEM_WRITE_TOO_LARGE,
            );
            payload_too_large(
                id,
                "File content exceeds the staged write_file byte limit.",
            )
        }
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                WRITE_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                mode,
                FILESYSTEM_WRITE_FAILED,
            );
            internal_error(id, "Filesystem write failed.")
        }
    }
}

#[rustfmt::skip]
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

#[rustfmt::skip]
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

#[rustfmt::skip]
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

#[rustfmt::skip]
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

#[rustfmt::skip]
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

#[rustfmt::skip]
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

#[rustfmt::skip]
fn record_read_only_allowed(
    counters: &SharedAuditCounters,
    tool_name: &'static str,
    gate_name: &'static str,
    reason_code: &'static str,
) {
    let event = read_only_allowed_event(
        current_unix_seconds(),
        tool_name,
        gate_name,
        reason_code,
    );
    record_audit_event(counters, &event);
}

#[rustfmt::skip]
fn record_read_only_denied(
    counters: &SharedAuditCounters,
    tool_name: &'static str,
    gate_name: &'static str,
    reason_code: &'static str,
) {
    let event = read_only_denied_event(
        current_unix_seconds(),
        tool_name,
        gate_name,
        reason_code,
    );
    record_audit_event(counters, &event);
}

#[rustfmt::skip]
fn record_filesystem_allowed(
    counters: &SharedAuditCounters,
    tool_name: &'static str,
    gate_name: &'static str,
    mode: AuditMode,
    reason_code: &'static str,
) {
    let event = filesystem_allowed_event(
        current_unix_seconds(),
        tool_name,
        gate_name,
        mode,
        reason_code,
    );
    record_audit_event(counters, &event);
}

#[rustfmt::skip]
fn record_filesystem_denied(
    counters: &SharedAuditCounters,
    tool_name: &'static str,
    gate_name: &'static str,
    mode: AuditMode,
    reason_code: &'static str,
) {
    let event = filesystem_denied_event(
        current_unix_seconds(),
        tool_name,
        gate_name,
        mode,
        reason_code,
    );
    record_audit_event(counters, &event);
}

fn record_audit_event(counters: &SharedAuditCounters, event: &crate::audit::AuditEvent) {
    if let Ok(mut counters) = counters.lock() {
        counters.record_event(event);
    }
}

#[rustfmt::skip]
fn audit_counters_snapshot(counters: &SharedAuditCounters) -> Value {
    counters
        .lock()
        .map(|counters| json!(counters.clone()))
        .unwrap_or_else(|_| {
            json!({
                "unavailable": true,
                "reason": "audit_counter_lock_poisoned",
            })
        })
}

fn filesystem_write_mode(dry_run: bool) -> AuditMode {
    if dry_run {
        AuditMode::DryRun
    } else {
        AuditMode::Mutating
    }
}

fn filesystem_write_allowed_reason(dry_run: bool) -> &'static str {
    if dry_run {
        FILESYSTEM_DRY_RUN_ALLOWED
    } else {
        FILESYSTEM_WRITE_ALLOWED
    }
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
#[rustfmt::skip]
mod tests {
    use super::*;

    #[test]
    fn read_only_audit_recorder_counts_allowed_status_call() {
        let counters = Arc::new(Mutex::new(AuditCounters::default()));

        record_read_only_allowed(
            &counters,
            ANDROID_STATUS_TOOL,
            ANDROID_STATUS_GATE,
            ANDROID_STATUS_ALLOWED,
        );

        let snapshot = counters.lock().unwrap().clone();
        assert_eq!(snapshot.allowed_total, 1);
        assert_eq!(snapshot.denied_total, 0);
        assert_eq!(snapshot.by_tool[ANDROID_STATUS_TOOL].allowed, 1);
        assert_eq!(snapshot.by_reason_code[ANDROID_STATUS_ALLOWED].allowed, 1);
    }

    #[test]
    fn read_only_audit_recorder_counts_denied_status_call_without_sensitive_values() {
        let counters = Arc::new(Mutex::new(AuditCounters::default()));

        record_read_only_denied(
            &counters,
            PROJECT_SERVICE_STATUS_TOOL,
            PROJECT_SERVICE_STATUS_GATE,
            PROJECT_SERVICE_STATUS_UNSUPPORTED,
        );

        let value = audit_counters_snapshot(&counters);
        assert_eq!(value["allowed_total"], 0);
        assert_eq!(value["denied_total"], 1);
        assert_eq!(value["by_tool"][PROJECT_SERVICE_STATUS_TOOL]["denied"], 1);
        assert_eq!(
            value["by_reason_code"][PROJECT_SERVICE_STATUS_UNSUPPORTED]["denied"],
            1
        );

        let serialized = value.to_string().to_ascii_lowercase();
        for token in ["password", "secret", "token", "/data/", "/home/", "bearer"] {
            assert!(
                !serialized.contains(token),
                "unexpected sensitive token: {token}"
            );
        }
    }

    #[test]
    fn filesystem_audit_recorder_counts_allowed_and_denied_decisions() {
        let counters = Arc::new(Mutex::new(AuditCounters::default()));

        record_filesystem_allowed(
            &counters,
            LIST_DIRECTORY_TOOL,
            FILESYSTEM_READ_GATE,
            AuditMode::ReadOnly,
            FILESYSTEM_LIST_ALLOWED,
        );
        record_filesystem_allowed(
            &counters,
            WRITE_FILE_TOOL,
            FILESYSTEM_WRITE_GATE,
            AuditMode::DryRun,
            FILESYSTEM_DRY_RUN_ALLOWED,
        );
        record_filesystem_denied(
            &counters,
            READ_FILE_TOOL,
            FILESYSTEM_READ_GATE,
            AuditMode::ReadOnly,
            FILESYSTEM_SAFE_ROOT_REJECTED,
        );

        let snapshot = counters.lock().unwrap().clone();
        assert_eq!(snapshot.allowed_total, 2);
        assert_eq!(snapshot.denied_total, 1);
        assert_eq!(snapshot.by_tool[LIST_DIRECTORY_TOOL].allowed, 1);
        assert_eq!(snapshot.by_tool[WRITE_FILE_TOOL].allowed, 1);
        assert_eq!(snapshot.by_tool[READ_FILE_TOOL].denied, 1);
        assert_eq!(snapshot.by_reason_code[FILESYSTEM_LIST_ALLOWED].allowed, 1);
        assert_eq!(snapshot.by_reason_code[FILESYSTEM_DRY_RUN_ALLOWED].allowed, 1);
        assert_eq!(snapshot.by_reason_code[FILESYSTEM_SAFE_ROOT_REJECTED].denied, 1);
    }

    #[test]
    fn filesystem_audit_snapshot_uses_stable_low_cardinality_values_only() {
        let counters = Arc::new(Mutex::new(AuditCounters::default()));

        record_filesystem_denied(
            &counters,
            WRITE_FILE_TOOL,
            FILESYSTEM_WRITE_GATE,
            AuditMode::Mutating,
            FILESYSTEM_WRITE_TOO_LARGE,
        );

        let value = audit_counters_snapshot(&counters);
        assert_eq!(value["denied_total"], 1);
        assert_eq!(value["by_tool"][WRITE_FILE_TOOL]["denied"], 1);
        assert_eq!(value["by_reason_code"][FILESYSTEM_WRITE_TOO_LARGE]["denied"], 1);

        let serialized = value.to_string().to_ascii_lowercase();
        for token in [
            "password", "secret", "token", "/data/", "/home/", "bearer", "content",
        ] {
            assert!(
                !serialized.contains(token),
                "unexpected sensitive token: {token}"
            );
        }
    }

    #[test]
    fn filesystem_write_audit_mode_and_reason_follow_dry_run_state() {
        assert_eq!(filesystem_write_mode(true), AuditMode::DryRun);
        assert_eq!(filesystem_write_mode(false), AuditMode::Mutating);
        assert_eq!(filesystem_write_allowed_reason(true), FILESYSTEM_DRY_RUN_ALLOWED);
        assert_eq!(filesystem_write_allowed_reason(false), FILESYSTEM_WRITE_ALLOWED);
    }
}
