use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
    Json, Router,
};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

use crate::{
    android_status::collect_android_status,
    audit::{
        filesystem_allowed_event, filesystem_denied_event, read_only_allowed_event,
        read_only_denied_event, AuditCounters, AuditMode,
    },
    error::AppError,
    json_rpc::{parse_incoming_message, IncomingJsonRpcMessage, JsonRpcEnvelopeError},
    mcp_session::{McpSessionStore, SessionPhase, SessionStoreError},
    platform_info::collect_platform_info,
    service_status::{
        collect_project_service_status, ProjectServiceStatusError, PROJECT_SERVICE_ALLOWLIST,
    },
    tools::FileSystemTools,
    transport_security::TransportSecurityPolicy,
    write_policy::{WriteMode, WritePolicy},
};

pub const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
pub const MCP_PROTOCOL_VERSION_HEADER: &str = "mcp-protocol-version";
pub const MCP_SESSION_ID_HEADER: &str = "mcp-session-id";
pub const MCP_POST_ACCEPT: &str = "application/json, text/event-stream";

const APPLICATION_JSON: &str = "application/json";
const TEXT_EVENT_STREAM: &str = "text/event-stream";

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
const RUNTIME_STATUS_ARGUMENTS_DENIED: &str = "arguments_not_empty_or_not_object";
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

const TOOL_CALL_PARAMS_INVALID: &str = "tools/call params do not match the required schema.";
const TOOL_ARGUMENTS_INVALID: &str = "Tool arguments do not match the advertised input schema.";

type SharedAuditCounters = Arc<Mutex<AuditCounters>>;

#[derive(Clone)]
struct McpTransportState {
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    audit_counters: SharedAuditCounters,
    sessions: McpSessionStore,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolCallParams {
    name: String,
    #[serde(default, deserialize_with = "deserialize_tool_arguments")]
    arguments: ToolArguments,
}

#[derive(Debug, Default)]
enum ToolArguments {
    #[default]
    Omitted,
    Present(Value),
}

struct NoArgumentToolContract {
    tool_name: &'static str,
    gate_name: &'static str,
    allowed_reason: &'static str,
    denied_reason: &'static str,
    response_builder: fn(Option<Value>, &SharedAuditCounters) -> Response,
}

impl ToolArguments {
    fn into_value(self) -> Option<Value> {
        match self {
            Self::Omitted => None,
            Self::Present(value) => Some(value),
        }
    }

    fn is_omitted_or_empty_object(&self) -> bool {
        match self {
            Self::Omitted => true,
            Self::Present(Value::Object(object)) => object.is_empty(),
            Self::Present(_) => false,
        }
    }
}

fn deserialize_tool_arguments<'de, D>(deserializer: D) -> Result<ToolArguments, D::Error>
where
    D: Deserializer<'de>,
{
    Value::deserialize(deserializer).map(ToolArguments::Present)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectServiceStatusArguments {
    service_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListDirectoryArguments {
    path: String,
    #[serde(default)]
    max_depth: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadFileArguments {
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteFileArguments {
    path: String,
    content: String,
    #[serde(default)]
    dry_run: Option<bool>,
}

/// Build the stable MCP 2025-11-25 Streamable HTTP transport.
///
/// The runtime exposes negotiated, session-scoped MCP discovery,
/// deterministic runtime metadata, non-sensitive platform metadata,
/// read-only Android/Termux status metadata, read-only project-owned service
/// status metadata, safe-rooted directory listing, bounded safe-rooted UTF-8
/// reads, and default-dry-run safe-rooted writes. Android platform control,
/// command execution, and high-impact actions remain unavailable until later
/// independently validated stages.
#[rustfmt::skip]
pub fn router(security_policy: TransportSecurityPolicy, file_tools: FileSystemTools) -> Router {
    Router::new()
        .route("/mcp", any(handle_mcp_request))
        .with_state(McpTransportState {
            security_policy,
            file_tools,
            audit_counters: Arc::new(Mutex::new(AuditCounters::default())),
            sessions: McpSessionStore::new(),
        })
}

async fn handle_mcp_request(
    State(state): State<McpTransportState>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let host = header_value(&headers, header::HOST);
    let origin = header_value(&headers, header::ORIGIN);

    let mut response = if let Err(error) = state.security_policy.validate_request(host, origin) {
        (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "transport_security_rejected",
                "message": error.to_string(),
            })),
        )
            .into_response()
    } else {
        match method {
            Method::POST => handle_mcp_post(&state, &headers, body).await,
            Method::GET => handle_mcp_get(&state, &headers),
            Method::DELETE => handle_mcp_delete(&state, &headers),
            _ => method_not_allowed(),
        }
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn handle_mcp_post(
    state: &McpTransportState,
    headers: &HeaderMap,
    body: Bytes,
) -> Response {
    if !has_json_content_type(headers) {
        return transport_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_content_type",
            "MCP POST requests require Content-Type application/json.",
        );
    }

    if !accepts_media_type(headers, APPLICATION_JSON)
        || !accepts_media_type(headers, TEXT_EVENT_STREAM)
    {
        return transport_error(
            StatusCode::NOT_ACCEPTABLE,
            "unsupported_accept",
            "MCP POST requests must accept application/json and text/event-stream.",
        );
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

    if let IncomingJsonRpcMessage::Request { id, method, params } = &message {
        if method == "initialize" {
            if headers.contains_key(MCP_SESSION_ID_HEADER) {
                return transport_error(
                    StatusCode::BAD_REQUEST,
                    "session_not_allowed",
                    "Initialize requests must not include MCP-Session-Id.",
                );
            }
            return initialize_response(Some(id.clone()), params.clone(), state);
        }
    }

    let (session_id, phase) = match validate_session_request(headers, &state.sessions) {
        Ok(session) => session,
        Err(response) => return response,
    };

    match message {
        IncomingJsonRpcMessage::Request { id, method, params } => {
            if method == "ping" {
                return ping_response(Some(id));
            }
            if phase != SessionPhase::Active {
                return server_not_initialized(Some(id));
            }

            match method.as_str() {
                "tools/list" => tools_list_response(Some(id)),
                "tools/call" => handle_tool_call(Some(id), params, state).await,
                _ => method_not_available(
                    Some(id),
                    "Only ping, tools/list, and tools/call are available after initialization.",
                ),
            }
        }
        IncomingJsonRpcMessage::Notification { method, params: _ } => {
            if method == "notifications/initialized" {
                return match state.sessions.activate(&session_id) {
                    Ok(()) => StatusCode::ACCEPTED.into_response(),
                    Err(error) => session_store_error_response(error),
                };
            }
            if phase != SessionPhase::Active {
                return server_not_initialized(None);
            }
            StatusCode::ACCEPTED.into_response()
        }
        IncomingJsonRpcMessage::Response => {
            if phase != SessionPhase::Active {
                return server_not_initialized(None);
            }
            StatusCode::ACCEPTED.into_response()
        }
    }
}

fn handle_mcp_get(state: &McpTransportState, headers: &HeaderMap) -> Response {
    if !accepts_media_type(headers, TEXT_EVENT_STREAM) {
        return transport_error(
            StatusCode::NOT_ACCEPTABLE,
            "unsupported_accept",
            "MCP GET requests must accept text/event-stream.",
        );
    }

    let (_, phase) = match validate_session_request(headers, &state.sessions) {
        Ok(session) => session,
        Err(response) => return response,
    };
    if phase != SessionPhase::Active {
        return server_not_initialized(None);
    }

    let mut response = StatusCode::METHOD_NOT_ALLOWED.into_response();
    response
        .headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static("POST, DELETE"));
    response
}

fn handle_mcp_delete(state: &McpTransportState, headers: &HeaderMap) -> Response {
    let (session_id, _) = match validate_session_request(headers, &state.sessions) {
        Ok(session) => session,
        Err(response) => return response,
    };

    match state.sessions.terminate(&session_id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => session_store_error_response(error),
    }
}

fn method_not_allowed() -> Response {
    let mut response = StatusCode::METHOD_NOT_ALLOWED.into_response();
    response.headers_mut().insert(
        header::ALLOW,
        HeaderValue::from_static("POST, GET, DELETE"),
    );
    response
}

#[rustfmt::skip]
fn initialize_response(
    id: Option<Value>,
    params: Option<Value>,
    state: &McpTransportState,
) -> Response {
    if !valid_initialize_params(params.as_ref()) {
        return invalid_params(
            id,
            "initialize params must match the MCP 2025-11-25 schema.",
        );
    }

    let session_id = match state.sessions.create() {
        Ok(session_id) => session_id,
        Err(error) => return session_store_error_response(error),
    };

    let mut response = (
        StatusCode::OK,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "result": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
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
        .into_response();
    response.headers_mut().insert(
        MCP_SESSION_ID_HEADER,
        HeaderValue::try_from(session_id.as_str())
            .expect("UUID session IDs are valid header values"),
    );
    response
}

#[rustfmt::skip]
fn ping_response(id: Option<Value>) -> Response {
    (
        StatusCode::OK,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "result": {},
        })),
    )
        .into_response()
}

fn valid_initialize_params(params: Option<&Value>) -> bool {
    let Some(params) = params.and_then(Value::as_object) else {
        return false;
    };

    params
        .get("protocolVersion")
        .is_some_and(Value::is_string)
        && params
            .get("capabilities")
            .and_then(Value::as_object)
            .is_some_and(valid_client_capabilities)
        && params
            .get("clientInfo")
            .and_then(Value::as_object)
            .is_some_and(valid_client_implementation)
        && params.get("_meta").is_none_or(valid_meta)
}

fn valid_client_capabilities(capabilities: &serde_json::Map<String, Value>) -> bool {
    for key in ["roots", "sampling", "elicitation", "tasks"] {
        if capabilities.get(key).is_some_and(|value| !value.is_object()) {
            return false;
        }
    }

    if let Some(experimental) = capabilities.get("experimental") {
        let Some(experimental) = experimental.as_object() else {
            return false;
        };
        if experimental.values().any(|value| !value.is_object()) {
            return false;
        }
    }

    if let Some(roots) = capabilities.get("roots").and_then(Value::as_object) {
        if roots
            .get("listChanged")
            .is_some_and(|value| !value.is_boolean())
        {
            return false;
        }
    }

    if let Some(sampling) = capabilities.get("sampling").and_then(Value::as_object) {
        if ["context", "tools"]
            .into_iter()
            .any(|key| sampling.get(key).is_some_and(|value| !value.is_object()))
        {
            return false;
        }
    }

    if let Some(elicitation) = capabilities.get("elicitation").and_then(Value::as_object) {
        if ["form", "url"]
            .into_iter()
            .any(|key| elicitation.get(key).is_some_and(|value| !value.is_object()))
        {
            return false;
        }
    }

    capabilities
        .get("tasks")
        .and_then(Value::as_object)
        .is_none_or(valid_task_capabilities)
}

fn valid_task_capabilities(tasks: &serde_json::Map<String, Value>) -> bool {
    if ["cancel", "list"]
        .into_iter()
        .any(|key| tasks.get(key).is_some_and(|value| !value.is_object()))
    {
        return false;
    }

    let Some(requests) = tasks.get("requests") else {
        return true;
    };
    let Some(requests) = requests.as_object() else {
        return false;
    };

    for (group, request) in [("elicitation", "create"), ("sampling", "createMessage")] {
        let Some(group) = requests.get(group) else {
            continue;
        };
        let Some(group) = group.as_object() else {
            return false;
        };
        if group.get(request).is_some_and(|value| !value.is_object()) {
            return false;
        }
    }

    true
}

fn valid_client_implementation(info: &serde_json::Map<String, Value>) -> bool {
    if !info.get("name").is_some_and(Value::is_string)
        || !info.get("version").is_some_and(Value::is_string)
    {
        return false;
    }

    if ["title", "description", "websiteUrl"]
        .into_iter()
        .any(|key| info.get(key).is_some_and(|value| !value.is_string()))
    {
        return false;
    }

    let Some(icons) = info.get("icons") else {
        return true;
    };
    let Some(icons) = icons.as_array() else {
        return false;
    };

    icons.iter().all(|icon| {
        let Some(icon) = icon.as_object() else {
            return false;
        };
        icon.get("src").is_some_and(Value::is_string)
            && icon
                .get("mimeType")
                .is_none_or(|value| value.is_string())
            && icon.get("sizes").is_none_or(|sizes| {
                sizes
                    .as_array()
                    .is_some_and(|sizes| sizes.iter().all(Value::is_string))
            })
            && icon.get("theme").is_none_or(|theme| {
                matches!(theme.as_str(), Some("light" | "dark"))
            })
    })
}

fn valid_meta(meta: &Value) -> bool {
    meta.as_object().is_some_and(|meta| {
        meta.get("progressToken").is_none_or(|token| {
            token.is_string()
                || token
                    .as_number()
                    .is_some_and(|number| number.is_i64() || number.is_u64())
        })
    })
}

fn has_json_content_type(headers: &HeaderMap) -> bool {
    let Ok(Some(value)) = single_header_value(headers, header::CONTENT_TYPE.as_str()) else {
        return false;
    };
    value
        .split(';')
        .next()
        .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case(APPLICATION_JSON))
}

fn accepts_media_type(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|item| acceptable_media_range(item, expected))
}

fn acceptable_media_range(item: &str, expected: &str) -> bool {
    let mut segments = item.split(';');
    if !segments
        .next()
        .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case(expected))
    {
        return false;
    }

    let mut quality = 1.0_f32;
    let mut saw_quality = false;
    for parameter in segments {
        let Some((name, value)) = parameter.trim().split_once('=') else {
            return false;
        };
        if name.trim().eq_ignore_ascii_case("q") {
            if saw_quality {
                return false;
            }
            saw_quality = true;
            let Ok(parsed) = value.trim().parse::<f32>() else {
                return false;
            };
            if !(0.0..=1.0).contains(&parsed) {
                return false;
            }
            quality = parsed;
        }
    }

    quality > 0.0
}

fn validate_session_request(
    headers: &HeaderMap,
    sessions: &McpSessionStore,
) -> Result<(String, SessionPhase), Response> {
    let protocol = match single_header_value(headers, MCP_PROTOCOL_VERSION_HEADER) {
        Ok(Some(protocol)) if protocol == MCP_PROTOCOL_VERSION => protocol,
        Ok(Some(_)) => {
            return Err(transport_error(
                StatusCode::BAD_REQUEST,
                "unsupported_protocol_version",
                "MCP-Protocol-Version must match the negotiated protocol version.",
            ));
        }
        Ok(None) => {
            return Err(transport_error(
                StatusCode::BAD_REQUEST,
                "protocol_version_required",
                "MCP-Protocol-Version is required after initialization.",
            ));
        }
        Err(()) => {
            return Err(transport_error(
                StatusCode::BAD_REQUEST,
                "invalid_protocol_version_header",
                "MCP-Protocol-Version must contain exactly one valid value.",
            ));
        }
    };
    debug_assert_eq!(protocol, MCP_PROTOCOL_VERSION);

    let session_id = match single_header_value(headers, MCP_SESSION_ID_HEADER) {
        Ok(Some(session_id))
            if !session_id.is_empty()
                && session_id.len() <= 128
                && session_id
                    .bytes()
                    .all(|byte| (0x21..=0x7e).contains(&byte)) =>
        {
            session_id.to_owned()
        }
        Ok(None) => {
            return Err(transport_error(
                StatusCode::BAD_REQUEST,
                "session_required",
                "MCP-Session-Id is required after initialization.",
            ));
        }
        Ok(Some(_)) | Err(()) => {
            return Err(transport_error(
                StatusCode::NOT_FOUND,
                "session_not_found",
                "The MCP session does not exist or has expired.",
            ));
        }
    };

    sessions
        .phase(&session_id)
        .map(|phase| (session_id, phase))
        .map_err(session_store_error_response)
}

fn single_header_value<'a>(
    headers: &'a HeaderMap,
    name: &str,
) -> Result<Option<&'a str>, ()> {
    let mut values = headers.get_all(name).iter();
    let Some(first) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(());
    }
    first.to_str().map(Some).map_err(|_| ())
}

fn session_store_error_response(error: SessionStoreError) -> Response {
    match error {
        SessionStoreError::CapacityExhausted => transport_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "session_capacity_reached",
            "The bounded MCP session capacity is currently exhausted.",
        ),
        SessionStoreError::NotFound => transport_error(
            StatusCode::NOT_FOUND,
            "session_not_found",
            "The MCP session does not exist or has expired.",
        ),
        SessionStoreError::Poisoned => transport_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "session_state_unavailable",
            "MCP session state is unavailable.",
        ),
    }
}

#[rustfmt::skip]
fn transport_error(status: StatusCode, error: &'static str, message: &'static str) -> Response {
    (
        status,
        Json(json!({
            "error": error,
            "message": message,
        })),
    )
        .into_response()
}

#[rustfmt::skip]
fn server_not_initialized(id: Option<Value>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "error": {
                "code": -32000,
                "message": "Server not initialized",
                "data": "Send notifications/initialized before normal operations.",
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
        None => return invalid_params(id, TOOL_CALL_PARAMS_INVALID),
    };

    let call = match serde_json::from_value::<ToolCallParams>(params) {
        Ok(call) => call,
        Err(_error) => return invalid_params(id, TOOL_CALL_PARAMS_INVALID),
    };

    match call.name.as_str() {
        RUNTIME_STATUS_TOOL => handle_no_argument_tool_call(
            id,
            call.arguments,
            &state.audit_counters,
            NoArgumentToolContract {
                tool_name: RUNTIME_STATUS_TOOL,
                gate_name: RUNTIME_STATUS_GATE,
                allowed_reason: RUNTIME_STATUS_ALLOWED,
                denied_reason: RUNTIME_STATUS_ARGUMENTS_DENIED,
                response_builder: runtime_status_response,
            },
        ),
        PLATFORM_INFO_TOOL => handle_no_argument_tool_call(
            id,
            call.arguments,
            &state.audit_counters,
            NoArgumentToolContract {
                tool_name: PLATFORM_INFO_TOOL,
                gate_name: PLATFORM_INFO_GATE,
                allowed_reason: PLATFORM_INFO_ALLOWED,
                denied_reason: PLATFORM_INFO_ARGUMENTS_DENIED,
                response_builder: platform_info_response,
            },
        ),
        ANDROID_STATUS_TOOL => handle_no_argument_tool_call(
            id,
            call.arguments,
            &state.audit_counters,
            NoArgumentToolContract {
                tool_name: ANDROID_STATUS_TOOL,
                gate_name: ANDROID_STATUS_GATE,
                allowed_reason: ANDROID_STATUS_ALLOWED,
                denied_reason: ANDROID_STATUS_ARGUMENTS_DENIED,
                response_builder: android_status_response,
            },
        ),
        PROJECT_SERVICE_STATUS_TOOL => {
            project_service_status_response(
                id,
                call.arguments.into_value(),
                &state.audit_counters,
            )
        }
        LIST_DIRECTORY_TOOL => {
            handle_list_directory_call(
                id,
                call.arguments.into_value(),
                &state.file_tools,
                &state.audit_counters,
            )
            .await
        }
        READ_FILE_TOOL => {
            handle_read_file_call(
                id,
                call.arguments.into_value(),
                &state.file_tools,
                &state.audit_counters,
            )
            .await
        }
        WRITE_FILE_TOOL => {
            handle_write_file_call(
                id,
                call.arguments.into_value(),
                &state.file_tools,
                &state.audit_counters,
            )
            .await
        }
        _ => method_not_available(
            id,
            "Only runtime_status, platform_info, android_status, project_service_status, list_directory, read_file, and write_file are available in this staged runtime.",
        ),
    }
}

fn handle_no_argument_tool_call(
    id: Option<Value>,
    arguments: ToolArguments,
    audit_counters: &SharedAuditCounters,
    contract: NoArgumentToolContract,
) -> Response {
    if !arguments.is_omitted_or_empty_object() {
        record_read_only_denied(
            audit_counters,
            contract.tool_name,
            contract.gate_name,
            contract.denied_reason,
        );
        return invalid_params(id, TOOL_ARGUMENTS_INVALID);
    }

    record_read_only_allowed(
        audit_counters,
        contract.tool_name,
        contract.gate_name,
        contract.allowed_reason,
    );
    (contract.response_builder)(id, audit_counters)
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
                        "text": "termux-mcp-edge runtime_status: transport=streamable-http-2025-11-25-session-scoped-no-sse, tools=runtime-status-platform-info-android-status-project-service-status-directory-listing-read-file-and-default-dry-run-write-file, platform_info=read-only-non-sensitive, android_status=read-only-allowlisted-no-api-or-control, project_service_status=read-only-allowlisted, filesystem=list-read-and-dry-run-write-file, android_platform=disabled, command_execution=disabled",
                    },
                ],
                "structuredContent": {
                    "server": "termux-mcp-edge",
                    "version": env!("CARGO_PKG_VERSION"),
                    "transport": "streamable_http_2025_11_25",
                    "sessionManagement": "bounded_uuid_idle_expiry",
                    "serverSentEvents": false,
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
    _audit_counters: &SharedAuditCounters,
) -> Response {
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
    _audit_counters: &SharedAuditCounters,
) -> Response {
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

    let args = match serde_json::from_value::<ProjectServiceStatusArguments>(arguments) {
        Ok(args) => args,
        Err(_error) => {
            record_read_only_denied(
                audit_counters,
                PROJECT_SERVICE_STATUS_TOOL,
                PROJECT_SERVICE_STATUS_GATE,
                PROJECT_SERVICE_STATUS_INVALID_ARGUMENTS,
            );
            return invalid_params(id, TOOL_ARGUMENTS_INVALID);
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
            invalid_params(id, TOOL_ARGUMENTS_INVALID)
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
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                LIST_DIRECTORY_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_INVALID_ARGUMENTS,
            );
            return invalid_params(id, TOOL_ARGUMENTS_INVALID);
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
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                READ_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_INVALID_ARGUMENTS,
            );
            return invalid_params(id, TOOL_ARGUMENTS_INVALID);
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
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                WRITE_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::DryRun,
                FILESYSTEM_INVALID_ARGUMENTS,
            );
            return invalid_params(id, TOOL_ARGUMENTS_INVALID);
        }
    };

    let policy = WritePolicy::default();
    let bytes = args.content.len();
    let dry_run = matches!(policy.resolve_mode(args.dry_run), WriteMode::DryRun);
    let mode = filesystem_write_mode(dry_run);

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
        Err(AppError::WritePayloadTooLarge { .. }) => {
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
