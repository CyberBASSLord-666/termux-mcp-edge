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

#[cfg(feature = "android-battery-status")]
use crate::android_battery::AndroidBatteryClient;
#[cfg(feature = "android-volume-status")]
use crate::android_volume::AndroidVolumeClient;
#[cfg(feature = "command-execution")]
use crate::command_execution::{CommandExecutionClient, CommandExecutionError};
#[cfg(not(feature = "command-execution"))]
use crate::command_policy::COMMAND_FEATURE_DISABLED_REASON;
#[cfg(feature = "command-execution")]
use crate::command_policy::{
    CommandExecutionPolicy, CommandPolicyDecision, COMMAND_CONCURRENCY_LIMIT_REASON,
    COMMAND_OUTPUT_INVALID_UTF8_REASON, COMMAND_PROFILE_NOT_ALLOWLISTED_REASON,
    COMMAND_PROGRAM_FAILED_REASON, COMMAND_PROGRAM_UNAVAILABLE_REASON, COMMAND_SPAWN_FAILED_REASON,
    COMMAND_STDERR_LIMIT_REASON, COMMAND_STDOUT_LIMIT_REASON, COMMAND_TIMEOUT_REASON,
    COMMAND_WAIT_FAILED_REASON,
};
use crate::{
    android_status::collect_android_status,
    audit::{
        filesystem_allowed_event, filesystem_denied_event, read_only_allowed_event,
        read_only_denied_event, AuditCounters, AuditDecision, AuditEvent, AuditMode,
    },
    command_policy::{
        command_profile_ids, COMMAND_EXECUTION_GATE, COMMAND_INVALID_ARGUMENTS_REASON,
        COMMAND_MISSING_ARGUMENTS_REASON, RUN_COMMAND_PROFILE_TOOL,
    },
    create_directory_grant::{
        CreateDirectoryGrantAuthority, CREATE_DIRECTORY_GRANT_HEADER,
        CREATE_DIRECTORY_GRANT_TTL_SECONDS, MAX_CREATE_DIRECTORY_GRANT_HEADER_BYTES,
    },
    error::AppError,
    json_rpc::{parse_incoming_message, IncomingJsonRpcMessage, JsonRpcEnvelopeError},
    mcp_session::{McpSessionStore, SessionPhase, SessionStoreError},
    platform_info::collect_platform_info,
    service_status::{
        collect_project_service_status, ProjectServiceStatusError, PROJECT_SERVICE_ALLOWLIST,
    },
    tools::{
        AuthorizedCreateDirectoryError, FileSystemTools, PreparedCreateDirectoryMutation,
        MAX_BINARY_READ_BASE64_BYTES, MAX_BINARY_READ_BYTES, MAX_BINARY_READ_RESPONSE_BYTES,
        MAX_COPY_FILE_RESPONSE_BYTES, MAX_CREATE_DIRECTORY_RESPONSE_BYTES, MAX_HASH_FILE_BYTES,
        MAX_HASH_FILE_RESPONSE_BYTES, MAX_LIST_RESPONSE_BYTES, MAX_PATH_METADATA_RESPONSE_BYTES,
        MAX_READ_RESPONSE_BYTES, MAX_SEARCH_DEPTH, MAX_SEARCH_QUERY_BYTES,
        MAX_SEARCH_RESPONSE_BYTES, MIN_SEARCH_DEPTH,
    },
    transport_security::TransportSecurityPolicy,
    write_policy::{WriteMode, WritePolicy},
};
#[cfg(feature = "android-volume-control")]
use crate::{
    android_volume_control::{
        AndroidVolumeControlClient, AndroidVolumeControlError, AndroidVolumeStreamName,
    },
    android_volume_grant::{
        AndroidVolumeGrantAuthority, AndroidVolumeGrantTarget, ANDROID_VOLUME_GRANT_TTL_SECONDS,
    },
};

pub const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
pub const MCP_PROTOCOL_VERSION_HEADER: &str = "mcp-protocol-version";
pub const MCP_SESSION_ID_HEADER: &str = "mcp-session-id";
pub const MCP_POST_ACCEPT: &str = "application/json, text/event-stream";

const APPLICATION_JSON: &str = "application/json";
const TEXT_EVENT_STREAM: &str = "text/event-stream";

#[cfg(feature = "android-volume-control")]
const ANDROID_VOLUME_GRANT_TTL_SECONDS_IF_COMPILED: u64 = ANDROID_VOLUME_GRANT_TTL_SECONDS;
#[cfg(not(feature = "android-volume-control"))]
const ANDROID_VOLUME_GRANT_TTL_SECONDS_IF_COMPILED: u64 = 0;

const RUNTIME_STATUS_TOOL: &str = "runtime_status";
const PLATFORM_INFO_TOOL: &str = "platform_info";
const ANDROID_STATUS_TOOL: &str = "android_status";
const ANDROID_BATTERY_STATUS_TOOL: &str = "android_battery_status";
const ANDROID_VOLUME_STATUS_TOOL: &str = "android_volume_status";
const SET_ANDROID_VOLUME_TOOL: &str = "set_android_volume";
const PROJECT_SERVICE_STATUS_TOOL: &str = "project_service_status";
const CREATE_DIRECTORY_TOOL: &str = "create_directory";
const COPY_FILE_TOOL: &str = "copy_file";
const HASH_FILE_TOOL: &str = "hash_file";
const LIST_DIRECTORY_TOOL: &str = "list_directory";
const PATH_METADATA_TOOL: &str = "path_metadata";
const READ_BINARY_FILE_TOOL: &str = "read_binary_file";
const READ_FILE_TOOL: &str = "read_file";
const SEARCH_TEXT_TOOL: &str = "search_text";
const WRITE_FILE_TOOL: &str = "write_file";
const BASE_AVAILABLE_TOOLS: [&str; 13] = [
    RUNTIME_STATUS_TOOL,
    PLATFORM_INFO_TOOL,
    ANDROID_STATUS_TOOL,
    PROJECT_SERVICE_STATUS_TOOL,
    CREATE_DIRECTORY_TOOL,
    COPY_FILE_TOOL,
    HASH_FILE_TOOL,
    LIST_DIRECTORY_TOOL,
    PATH_METADATA_TOOL,
    READ_BINARY_FILE_TOOL,
    READ_FILE_TOOL,
    SEARCH_TEXT_TOOL,
    WRITE_FILE_TOOL,
];
const MIN_LIST_DIRECTORY_DEPTH: u32 = 1;
const MAX_LIST_DIRECTORY_DEPTH: u32 = 5;

const RUNTIME_STATUS_GATE: &str = "runtime_metadata";
const PLATFORM_INFO_GATE: &str = "platform_metadata";
const ANDROID_STATUS_GATE: &str = "android_read_only_status";
const ANDROID_BATTERY_STATUS_GATE: &str = "android_battery_status";
const ANDROID_VOLUME_STATUS_GATE: &str = "android_volume_status";
const ANDROID_VOLUME_CONTROL_GATE: &str = "android_volume_control";
const PROJECT_SERVICE_STATUS_GATE: &str = "project_service_state";
const FILESYSTEM_METADATA_GATE: &str = "filesystem_metadata";
const FILESYSTEM_READ_GATE: &str = "filesystem_read";
const FILESYSTEM_WRITE_GATE: &str = "filesystem_write";

const RUNTIME_STATUS_ALLOWED: &str = "staged_runtime_metadata";
const RUNTIME_STATUS_ARGUMENTS_DENIED: &str = "arguments_not_empty_or_not_object";
const PLATFORM_INFO_ALLOWED: &str = "read_only_platform_metadata";
const PLATFORM_INFO_ARGUMENTS_DENIED: &str = "arguments_not_supported";
const ANDROID_STATUS_ALLOWED: &str = "allowlisted_status_metadata";
const ANDROID_STATUS_ARGUMENTS_DENIED: &str = "arguments_not_empty_or_not_object";
#[cfg(feature = "android-battery-status")]
const ANDROID_BATTERY_STATUS_ALLOWED: &str = "battery_status_read";
const ANDROID_BATTERY_STATUS_ARGUMENTS_DENIED: &str = "arguments_not_empty_or_not_object";
#[cfg(not(feature = "android-battery-status"))]
const ANDROID_BATTERY_STATUS_FEATURE_DISABLED: &str = "battery_feature_not_compiled";
#[cfg(feature = "android-battery-status")]
const ANDROID_BATTERY_STATUS_RUNTIME_DISABLED: &str = "battery_runtime_disabled";
#[cfg(feature = "android-volume-status")]
const ANDROID_VOLUME_STATUS_ALLOWED: &str = "volume_status_read";
const ANDROID_VOLUME_STATUS_ARGUMENTS_DENIED: &str = "arguments_not_empty_or_not_object";
const ANDROID_VOLUME_CONTROL_INVALID_ARGUMENTS: &str = "volume_control_arguments_invalid";
const ANDROID_VOLUME_CONTROL_PREVIEW_ALLOWED: &str = "volume_control_preview";
const ANDROID_VOLUME_CONTROL_MUTATION_ALLOWED: &str = "volume_control_mutation_verified";
#[cfg(not(feature = "android-volume-status"))]
const ANDROID_VOLUME_STATUS_FEATURE_DISABLED: &str = "volume_feature_not_compiled";
#[cfg(feature = "android-volume-status")]
const ANDROID_VOLUME_STATUS_RUNTIME_DISABLED: &str = "volume_runtime_disabled";
#[cfg(not(feature = "android-volume-control"))]
const ANDROID_VOLUME_CONTROL_FEATURE_DISABLED: &str = "volume_control_feature_not_compiled";
#[cfg(feature = "android-volume-control")]
const ANDROID_VOLUME_CONTROL_RUNTIME_DISABLED: &str = "volume_control_runtime_disabled";
const PROJECT_SERVICE_STATUS_ALLOWED: &str = "allowlisted_project_service";
const PROJECT_SERVICE_STATUS_MISSING_ARGUMENTS: &str = "missing_service_name";
const PROJECT_SERVICE_STATUS_INVALID_ARGUMENTS: &str = "invalid_service_arguments";
const PROJECT_SERVICE_STATUS_UNSUPPORTED: &str = "unsupported_service";
const FILESYSTEM_MISSING_ARGUMENTS: &str = "missing_arguments";
const FILESYSTEM_INVALID_ARGUMENTS: &str = "invalid_arguments";
const FILESYSTEM_INVALID_DEPTH: &str = "invalid_max_depth";
const FILESYSTEM_SAFE_ROOT_REJECTED: &str = "safe_root_rejected";
const FILESYSTEM_LIST_ALLOWED: &str = "safe_root_listed";
const FILESYSTEM_METADATA_ALLOWED: &str = "safe_root_metadata_read";
const FILESYSTEM_METADATA_NOT_FOUND: &str = "filesystem_path_not_found";
const FILESYSTEM_METADATA_UNSUPPORTED: &str = "filesystem_path_type_unsupported";
const FILESYSTEM_METADATA_FAILED: &str = "filesystem_metadata_failed";
const FILESYSTEM_READ_ALLOWED: &str = "safe_root_read";
const FILESYSTEM_BINARY_READ_ALLOWED: &str = "safe_root_binary_read";
const FILESYSTEM_BINARY_READ_NOT_FOUND: &str = "filesystem_binary_read_target_not_found";
const FILESYSTEM_BINARY_READ_UNSUPPORTED: &str = "filesystem_binary_read_type_unsupported";
const FILESYSTEM_BINARY_READ_TOO_LARGE: &str = "filesystem_binary_read_size_limit_exceeded";
const FILESYSTEM_BINARY_READ_FAILED: &str = "filesystem_binary_read_failed";
const FILESYSTEM_HASH_ALLOWED: &str = "safe_root_file_hashed";
const FILESYSTEM_HASH_NOT_FOUND: &str = "filesystem_hash_target_not_found";
const FILESYSTEM_HASH_UNSUPPORTED: &str = "filesystem_hash_target_type_unsupported";
const FILESYSTEM_HASH_TOO_LARGE: &str = "filesystem_hash_size_limit_exceeded";
const FILESYSTEM_HASH_FAILED: &str = "filesystem_hash_failed";
const FILESYSTEM_SEARCH_ALLOWED: &str = "safe_root_text_searched";
const FILESYSTEM_SEARCH_INVALID_QUERY: &str = "search_query_invalid";
const FILESYSTEM_SEARCH_FAILED: &str = "filesystem_search_failed";
const FILESYSTEM_READ_TOO_LARGE: &str = "read_size_limit_exceeded";
const FILESYSTEM_READ_ENCODING_INVALID: &str = "read_encoding_invalid";
const FILESYSTEM_RESPONSE_TOO_LARGE: &str = "response_size_limit_exceeded";
const FILESYSTEM_CREATE_ALLOWED: &str = "safe_root_directory_created";
const FILESYSTEM_CREATE_EXISTS: &str = "filesystem_destination_exists";
const FILESYSTEM_CREATE_PARENT_NOT_FOUND: &str = "filesystem_parent_not_found";
const FILESYSTEM_CREATE_FAILED: &str = "filesystem_directory_create_failed";
const FILESYSTEM_CREATE_MUTATION_DISABLED: &str = "create_directory_mutation_disabled";
const FILESYSTEM_COPY_ALLOWED: &str = "safe_root_file_copied";
const FILESYSTEM_COPY_SOURCE_NOT_FOUND: &str = "filesystem_copy_source_not_found";
const FILESYSTEM_COPY_PARENT_NOT_FOUND: &str = "filesystem_copy_parent_not_found";
const FILESYSTEM_COPY_SAME_PATH: &str = "filesystem_copy_same_path";
const FILESYSTEM_COPY_SOURCE_UNSUPPORTED: &str = "filesystem_copy_source_type_unsupported";
const FILESYSTEM_COPY_SOURCE_TOO_LARGE: &str = "filesystem_copy_source_too_large";
const FILESYSTEM_COPY_FAILED: &str = "filesystem_copy_failed";
const FILESYSTEM_READ_FAILED: &str = "filesystem_read_failed";
const FILESYSTEM_DRY_RUN_ALLOWED: &str = "dry_run_preview";
const FILESYSTEM_WRITE_ALLOWED: &str = "explicit_write_allowed";
const FILESYSTEM_WRITE_TOO_LARGE: &str = "write_size_limit_exceeded";
const FILESYSTEM_WRITE_FAILED: &str = "filesystem_write_failed";

const COMMAND_EXECUTION_ERROR: &str = "command_profile_execution_failed";

const TOOL_CALL_PARAMS_INVALID: &str = "tools/call params do not match the required schema.";
const TOOL_ARGUMENTS_INVALID: &str = "Tool arguments do not match the advertised input schema.";

type SharedAuditCounters = Arc<Mutex<AuditCounters>>;

#[derive(Clone)]
struct McpTransportState {
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    audit_counters: SharedAuditCounters,
    sessions: McpSessionStore,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    android_volume_control_enabled: bool,
    command_execution_enabled: bool,
    create_directory_authority: Option<CreateDirectoryGrantAuthority>,
    #[cfg(feature = "android-battery-status")]
    android_battery_client: AndroidBatteryClient,
    #[cfg(feature = "android-volume-status")]
    android_volume_client: AndroidVolumeClient,
    #[cfg(feature = "android-volume-control")]
    android_volume_control_authority: Option<AndroidVolumeGrantAuthority>,
    #[cfg(feature = "android-volume-control")]
    android_volume_control_client: AndroidVolumeControlClient,
    #[cfg(feature = "command-execution")]
    command_execution_client: CommandExecutionClient,
}

impl McpTransportState {
    fn new(
        security_policy: TransportSecurityPolicy,
        file_tools: FileSystemTools,
        android_battery_status_enabled: bool,
        android_volume_status_enabled: bool,
        command_execution_enabled: bool,
        create_directory_authority: Option<CreateDirectoryGrantAuthority>,
    ) -> Self {
        #[cfg(feature = "command-execution")]
        let command_execution_client = CommandExecutionClient::current_server(
            file_tools
                .safe_roots()
                .first()
                .cloned()
                .expect("validated MCP filesystem tools own at least one safe root"),
        )
        .expect("current server executable and anchored command safe root must be usable");

        Self {
            security_policy,
            file_tools,
            audit_counters: Arc::new(Mutex::new(AuditCounters::default())),
            sessions: McpSessionStore::new(),
            android_battery_status_enabled: android_battery_status_enabled
                && cfg!(feature = "android-battery-status"),
            android_volume_status_enabled: android_volume_status_enabled
                && cfg!(feature = "android-volume-status"),
            android_volume_control_enabled: false,
            command_execution_enabled: command_execution_enabled
                && cfg!(feature = "command-execution"),
            create_directory_authority,
            #[cfg(feature = "android-battery-status")]
            android_battery_client: AndroidBatteryClient::termux(),
            #[cfg(feature = "android-volume-status")]
            android_volume_client: AndroidVolumeClient::termux(),
            #[cfg(feature = "android-volume-control")]
            android_volume_control_authority: None,
            #[cfg(feature = "android-volume-control")]
            android_volume_control_client: AndroidVolumeControlClient::termux(),
            #[cfg(feature = "command-execution")]
            command_execution_client,
        }
    }

    #[cfg(feature = "android-volume-control")]
    fn with_android_volume_control_authority(
        mut self,
        authority: Option<AndroidVolumeGrantAuthority>,
    ) -> Self {
        self.android_volume_control_enabled = authority.is_some();
        self.android_volume_control_authority = authority;
        self
    }

    #[cfg(all(test, feature = "android-battery-status"))]
    fn with_android_battery_client(
        security_policy: TransportSecurityPolicy,
        file_tools: FileSystemTools,
        android_battery_status_enabled: bool,
        android_battery_client: AndroidBatteryClient,
    ) -> Self {
        #[cfg(feature = "command-execution")]
        let command_execution_client = CommandExecutionClient::current_server(
            file_tools
                .safe_roots()
                .first()
                .cloned()
                .expect("test filesystem tools own a safe root"),
        )
        .expect("test command client construction must succeed");

        Self {
            security_policy,
            file_tools,
            audit_counters: Arc::new(Mutex::new(AuditCounters::default())),
            sessions: McpSessionStore::new(),
            android_battery_status_enabled,
            android_volume_status_enabled: false,
            android_volume_control_enabled: false,
            command_execution_enabled: false,
            create_directory_authority: None,
            android_battery_client,
            #[cfg(feature = "android-volume-status")]
            android_volume_client: AndroidVolumeClient::termux(),
            #[cfg(feature = "android-volume-control")]
            android_volume_control_authority: None,
            #[cfg(feature = "android-volume-control")]
            android_volume_control_client: AndroidVolumeControlClient::termux(),
            #[cfg(feature = "command-execution")]
            command_execution_client,
        }
    }

    #[cfg(all(test, feature = "android-volume-status"))]
    fn with_android_volume_client(
        security_policy: TransportSecurityPolicy,
        file_tools: FileSystemTools,
        android_volume_status_enabled: bool,
        android_volume_client: AndroidVolumeClient,
    ) -> Self {
        #[cfg(feature = "command-execution")]
        let command_execution_client = CommandExecutionClient::current_server(
            file_tools
                .safe_roots()
                .first()
                .cloned()
                .expect("test filesystem tools own a safe root"),
        )
        .expect("test command client construction must succeed");

        Self {
            security_policy,
            file_tools,
            audit_counters: Arc::new(Mutex::new(AuditCounters::default())),
            sessions: McpSessionStore::new(),
            android_battery_status_enabled: false,
            android_volume_status_enabled,
            android_volume_control_enabled: false,
            command_execution_enabled: false,
            create_directory_authority: None,
            #[cfg(feature = "android-battery-status")]
            android_battery_client: AndroidBatteryClient::termux(),
            android_volume_client,
            #[cfg(feature = "android-volume-control")]
            android_volume_control_authority: None,
            #[cfg(feature = "android-volume-control")]
            android_volume_control_client: AndroidVolumeControlClient::termux(),
            #[cfg(feature = "command-execution")]
            command_execution_client,
        }
    }

    #[cfg(all(test, feature = "command-execution"))]
    fn with_command_execution_client(
        security_policy: TransportSecurityPolicy,
        file_tools: FileSystemTools,
        command_execution_enabled: bool,
        command_execution_client: CommandExecutionClient,
    ) -> Self {
        Self {
            security_policy,
            file_tools,
            audit_counters: Arc::new(Mutex::new(AuditCounters::default())),
            sessions: McpSessionStore::new(),
            android_battery_status_enabled: false,
            android_volume_status_enabled: false,
            android_volume_control_enabled: false,
            command_execution_enabled,
            create_directory_authority: None,
            #[cfg(feature = "android-battery-status")]
            android_battery_client: AndroidBatteryClient::termux(),
            #[cfg(feature = "android-volume-status")]
            android_volume_client: AndroidVolumeClient::termux(),
            #[cfg(feature = "android-volume-control")]
            android_volume_control_authority: None,
            #[cfg(feature = "android-volume-control")]
            android_volume_control_client: AndroidVolumeControlClient::termux(),
            command_execution_client,
        }
    }

    #[cfg(all(test, feature = "android-volume-control"))]
    fn with_android_volume_control_client(
        security_policy: TransportSecurityPolicy,
        file_tools: FileSystemTools,
        authority: Option<AndroidVolumeGrantAuthority>,
        client: AndroidVolumeControlClient,
    ) -> Self {
        let mut state = Self::new(security_policy, file_tools, false, false, false, None)
            .with_android_volume_control_authority(authority);
        state.android_volume_control_client = client;
        state
    }
}

enum SessionRequestError {
    ProtocolVersionRequired,
    UnsupportedProtocolVersion,
    InvalidProtocolVersionHeader,
    SessionRequired,
    SessionNotFound,
    Store(SessionStoreError),
}

impl From<SessionStoreError> for SessionRequestError {
    fn from(error: SessionStoreError) -> Self {
        Self::Store(error)
    }
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
    response_builder: fn(Option<Value>, &SharedAuditCounters, bool) -> Response,
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
struct ReadBinaryFileArguments {
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HashFileArguments {
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PathMetadataArguments {
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateDirectoryArguments {
    path: String,
    #[serde(default)]
    dry_run: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CopyFileArguments {
    source_path: String,
    destination_path: String,
    #[serde(default)]
    dry_run: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchTextArguments {
    path: String,
    query: String,
    #[serde(default)]
    max_depth: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteFileArguments {
    path: String,
    content: String,
    #[serde(default)]
    dry_run: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(not(feature = "command-execution"), allow(dead_code))]
struct RunCommandProfileArguments {
    profile: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetAndroidVolumeArguments {
    stream: String,
    level: i64,
    #[serde(default)]
    dry_run: Option<bool>,
}

/// Build the stable MCP 2025-11-25 Streamable HTTP transport.
///
/// The runtime exposes negotiated, session-scoped MCP discovery,
/// deterministic runtime metadata, non-sensitive platform metadata,
/// read-only Android/Termux status metadata, read-only project-owned service
/// status metadata, safe-rooted directory listing and object metadata, bounded
/// safe-rooted canonical-base64 and UTF-8 reads, default-dry-run directory creation, bounded
/// binary file copy and file writes, and optionally compiled and enabled fixed-profile command diagnostics. Android platform control,
/// arbitrary command execution, and high-impact actions remain unavailable.
#[rustfmt::skip]
pub fn router(
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    command_execution_enabled: bool,
) -> Router {
    router_from_state(McpTransportState::new(
        security_policy,
        file_tools,
        android_battery_status_enabled,
        android_volume_status_enabled,
        command_execution_enabled,
        None,
    ))
}

/// Build the MCP transport with the dedicated `create_directory` mutation gate
/// enabled. Every mutating call still requires one valid request-scoped grant.
#[rustfmt::skip]
pub fn router_with_create_directory_authority(
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    command_execution_enabled: bool,
    create_directory_authority: CreateDirectoryGrantAuthority,
) -> Router {
    router_from_state(McpTransportState::new(
        security_policy,
        file_tools,
        android_battery_status_enabled,
        android_volume_status_enabled,
        command_execution_enabled,
        Some(create_directory_authority),
    ))
}

/// Build the MCP transport with independently optional filesystem and Android
/// mutation authorities. The volume-control tool remains hidden unless the
/// volume authority is present; every live call still requires an exact
/// request-scoped grant.
#[cfg(feature = "android-volume-control")]
#[rustfmt::skip]
pub fn router_with_capability_authorities(
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    command_execution_enabled: bool,
    create_directory_authority: Option<CreateDirectoryGrantAuthority>,
    android_volume_control_authority: Option<AndroidVolumeGrantAuthority>,
) -> Router {
    router_from_state(
        McpTransportState::new(
            security_policy,
            file_tools,
            android_battery_status_enabled,
            android_volume_status_enabled,
            command_execution_enabled,
            create_directory_authority,
        )
        .with_android_volume_control_authority(android_volume_control_authority),
    )
}

fn router_from_state(state: McpTransportState) -> Router {
    Router::new()
        .route("/mcp", any(handle_mcp_request))
        .with_state(state)
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
    } else if method != Method::POST && headers.contains_key(CREATE_DIRECTORY_GRANT_HEADER) {
        capability_context_not_allowed(None)
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

async fn handle_mcp_post(state: &McpTransportState, headers: &HeaderMap, body: Bytes) -> Response {
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

    let capability_grant = match single_header_value(headers, CREATE_DIRECTORY_GRANT_HEADER) {
        Ok(Some(value))
            if !value.is_empty()
                && value.len() <= MAX_CREATE_DIRECTORY_GRANT_HEADER_BYTES
                && value.is_ascii() =>
        {
            Some(value.to_owned())
        }
        Ok(None) => None,
        Ok(Some(_)) | Err(()) => {
            return transport_error(
                StatusCode::BAD_REQUEST,
                "invalid_capability_grant_header",
                "MCP-Capability-Grant must contain exactly one bounded ASCII value.",
            );
        }
    };

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
            if capability_grant.is_some() {
                return transport_error(
                    StatusCode::BAD_REQUEST,
                    "capability_context_not_allowed",
                    "A request-scoped capability grant is not accepted during initialization.",
                );
            }
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
        Err(error) => return session_request_error_response(error),
    };

    match message {
        IncomingJsonRpcMessage::Request { id, method, params } => {
            if capability_grant.is_some() && method != "tools/call" {
                return capability_context_not_allowed(Some(id));
            }
            if method == "ping" {
                return ping_response(Some(id));
            }
            if phase != SessionPhase::Active {
                return server_not_initialized(Some(id));
            }

            match method.as_str() {
                "tools/list" => tools_list_response(Some(id), state),
                "tools/call" => {
                    handle_tool_call(
                        Some(id),
                        params,
                        state,
                        &session_id,
                        capability_grant.as_deref(),
                    )
                    .await
                }
                _ => method_not_available(
                    Some(id),
                    "Only ping, tools/list, and tools/call are available after initialization.",
                ),
            }
        }
        IncomingJsonRpcMessage::Notification { method, params: _ } => {
            if capability_grant.is_some() {
                return capability_context_not_allowed(None);
            }
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
            if capability_grant.is_some() {
                return capability_context_not_allowed(None);
            }
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
        Err(error) => return session_request_error_response(error),
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
        Err(error) => return session_request_error_response(error),
    };

    match state.sessions.terminate(&session_id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => session_store_error_response(error),
    }
}

fn method_not_allowed() -> Response {
    let mut response = StatusCode::METHOD_NOT_ALLOWED.into_response();
    response
        .headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static("POST, GET, DELETE"));
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

    params.get("protocolVersion").is_some_and(Value::is_string)
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
        if capabilities
            .get(key)
            .is_some_and(|value| !value.is_object())
        {
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
            && icon.get("mimeType").is_none_or(|value| value.is_string())
            && icon.get("sizes").is_none_or(|sizes| {
                sizes
                    .as_array()
                    .is_some_and(|sizes| sizes.iter().all(Value::is_string))
            })
            && icon
                .get("theme")
                .is_none_or(|theme| matches!(theme.as_str(), Some("light" | "dark")))
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
) -> Result<(String, SessionPhase), SessionRequestError> {
    let protocol = match single_header_value(headers, MCP_PROTOCOL_VERSION_HEADER) {
        Ok(Some(protocol)) if protocol == MCP_PROTOCOL_VERSION => protocol,
        Ok(Some(_)) => {
            return Err(SessionRequestError::UnsupportedProtocolVersion);
        }
        Ok(None) => {
            return Err(SessionRequestError::ProtocolVersionRequired);
        }
        Err(()) => {
            return Err(SessionRequestError::InvalidProtocolVersionHeader);
        }
    };
    debug_assert_eq!(protocol, MCP_PROTOCOL_VERSION);

    let session_id = match single_header_value(headers, MCP_SESSION_ID_HEADER) {
        Ok(Some(session_id))
            if !session_id.is_empty()
                && session_id.len() <= 128
                && session_id.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) =>
        {
            session_id.to_owned()
        }
        Ok(None) => {
            return Err(SessionRequestError::SessionRequired);
        }
        Ok(Some(_)) | Err(()) => {
            return Err(SessionRequestError::SessionNotFound);
        }
    };

    sessions
        .phase(&session_id)
        .map(|phase| (session_id, phase))
        .map_err(SessionRequestError::from)
}

fn single_header_value<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>, ()> {
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

fn session_request_error_response(error: SessionRequestError) -> Response {
    match error {
        SessionRequestError::ProtocolVersionRequired => transport_error(
            StatusCode::BAD_REQUEST,
            "protocol_version_required",
            "MCP-Protocol-Version is required after initialization.",
        ),
        SessionRequestError::UnsupportedProtocolVersion => transport_error(
            StatusCode::BAD_REQUEST,
            "unsupported_protocol_version",
            "MCP-Protocol-Version must match the negotiated protocol version.",
        ),
        SessionRequestError::InvalidProtocolVersionHeader => transport_error(
            StatusCode::BAD_REQUEST,
            "invalid_protocol_version_header",
            "MCP-Protocol-Version must contain exactly one valid value.",
        ),
        SessionRequestError::SessionRequired => transport_error(
            StatusCode::BAD_REQUEST,
            "session_required",
            "MCP-Session-Id is required after initialization.",
        ),
        SessionRequestError::SessionNotFound => transport_error(
            StatusCode::NOT_FOUND,
            "session_not_found",
            "The MCP session does not exist or has expired.",
        ),
        SessionRequestError::Store(error) => session_store_error_response(error),
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
fn capability_context_not_allowed(id: Option<Value>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "error": {
                "code": -32600,
                "message": "Invalid Request",
                "data": "A request-scoped capability grant is accepted only for an exact grant-authorized tool call.",
            },
        })),
    )
        .into_response()
}

#[rustfmt::skip]
fn capability_authorization_denied(
    id: Option<Value>,
    reason_code: &'static str,
) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "error": {
                "code": -32003,
                "message": "Capability authorization denied",
                "data": {
                    "reason": reason_code,
                },
            },
        })),
    )
        .into_response()
}

#[rustfmt::skip]
fn tools_list_response(id: Option<Value>, state: &McpTransportState) -> Response {
    let mut body = json!({
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
                        "name": CREATE_DIRECTORY_TOOL,
                        "description": "Create one safe-rooted directory with fixed mode 0700. Defaults to dry-run; mutation requires explicit dry_run=false.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "path": {
                                    "type": "string",
                                    "description": "Absolute new-directory path inside one configured safe root; the parent must already exist.",
                                },
                                "dry_run": {
                                    "type": "boolean",
                                    "description": "Defaults to true. Set explicitly to false to create exactly one directory.",
                                },
                            },
                            "required": ["path"],
                            "additionalProperties": false,
                        },
                    },
                    {
                        "name": COPY_FILE_TOOL,
                        "description": "Copy one bounded regular file between configured filesystem safe roots without returning contents. Defaults to dry-run; mutation requires explicit dry_run=false.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "source_path": {
                                    "type": "string",
                                    "description": "Absolute regular-file path inside one configured safe root; size must not exceed 1 MiB.",
                                },
                                "destination_path": {
                                    "type": "string",
                                    "description": "Absolute absent destination path inside one configured safe root; the parent must already exist.",
                                },
                                "dry_run": {
                                    "type": "boolean",
                                    "description": "Defaults to true. Set explicitly to false to copy exactly one file with fixed mode 0600.",
                                },
                            },
                            "required": ["source_path", "destination_path"],
                            "additionalProperties": false,
                        },
                    },
                    {
                        "name": HASH_FILE_TOOL,
                        "description": "Compute a bounded SHA-256 digest for one safe-rooted regular file without returning file contents.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "path": {
                                    "type": "string",
                                    "description": format!(
                                        "Absolute regular-file path inside one configured safe root; at most {MAX_HASH_FILE_BYTES} bytes are hashed through the retained no-follow descriptor."
                                    ),
                                },
                            },
                            "required": ["path"],
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
                        "name": PATH_METADATA_TOOL,
                        "description": "Return bounded non-sensitive metadata for one regular file or directory inside a configured filesystem safe root.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "path": {
                                    "type": "string",
                                    "description": "Absolute regular-file or directory path inside one configured safe root.",
                                },
                            },
                            "required": ["path"],
                            "additionalProperties": false,
                        },
                    },
                    {
                        "name": READ_BINARY_FILE_TOOL,
                        "description": "Read one bounded regular file as canonical padded base64 from inside a configured filesystem safe root.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "path": {
                                    "type": "string",
                                    "description": format!(
                                        "Absolute regular-file path inside one configured safe root; at most {MAX_BINARY_READ_BYTES} raw bytes are returned through the retained no-follow descriptor."
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
                        "name": SEARCH_TEXT_TOOL,
                        "description": "Locate bounded literal UTF-8 text matches under a configured filesystem safe root without returning file contents.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "path": {
                                    "type": "string",
                                    "description": "Absolute directory path inside one configured safe root.",
                                },
                                "query": {
                                    "type": "string",
                                    "minLength": 1,
                                    "maxLength": MAX_SEARCH_QUERY_BYTES,
                                    "x-maxBytes": MAX_SEARCH_QUERY_BYTES,
                                    "description": "Case-sensitive literal single-line UTF-8 query of at most 256 bytes; regular expressions and globs are not evaluated.",
                                },
                                "max_depth": {
                                    "type": "integer",
                                    "minimum": MIN_SEARCH_DEPTH,
                                    "maximum": MAX_SEARCH_DEPTH,
                                    "description": format!(
                                        "Optional bounded traversal depth; defaults to {MAX_SEARCH_DEPTH}."
                                    ),
                                },
                            },
                            "required": ["path", "query"],
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
        });

    let create_directory_tool = body
        .pointer_mut("/result/tools")
        .and_then(Value::as_array_mut)
        .and_then(|tools| {
            tools
                .iter_mut()
                .find(|tool| tool.get("name") == Some(&json!(CREATE_DIRECTORY_TOOL)))
        })
        .expect("baseline discovery owns create_directory");
    if state.create_directory_authority.is_some() {
        create_directory_tool["description"] = json!(
            "Validate one safe-rooted directory creation, or create it with fixed mode 0700 only when dry_run=false and one request-scoped MCP-Capability-Grant is valid."
        );
        let dry_run_schema = create_directory_tool
            .pointer_mut("/inputSchema/properties/dry_run")
            .expect("create_directory owns a dry_run schema");
        dry_run_schema["description"] = json!(
            "Defaults to true. Explicit false additionally requires the enabled mutation gate and one request-scoped grant."
        );
    } else {
        create_directory_tool["description"] = json!(
            "Validate one safe-rooted directory creation without mutation; the dedicated mutation gate is disabled."
        );
        let dry_run_schema = create_directory_tool
            .pointer_mut("/inputSchema/properties/dry_run")
            .expect("create_directory owns a dry_run schema");
        dry_run_schema["const"] = json!(true);
        dry_run_schema["description"] = json!(
            "Mutation is disabled in this runtime posture; omitted dry_run and explicit true are accepted."
        );
    }

    if state.android_battery_status_enabled {
        body.pointer_mut("/result/tools")
            .and_then(Value::as_array_mut)
            .expect("tools/list response owns an array")
            .push(json!({
                "name": ANDROID_BATTERY_STATUS_TOOL,
                "description": "Return bounded read-only battery and thermal telemetry through the explicitly enabled Termux:API gate.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false,
                },
            }));
    }

    if state.android_volume_status_enabled {
        body.pointer_mut("/result/tools")
            .and_then(Value::as_array_mut)
            .expect("tools/list response owns an array")
            .push(json!({
                "name": ANDROID_VOLUME_STATUS_TOOL,
                "description": "Return normalized read-only Android audio-stream volume levels through the explicitly enabled Termux:API gate.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false,
                },
            }));
    }

    if state.android_volume_control_enabled {
        body.pointer_mut("/result/tools")
            .and_then(Value::as_array_mut)
            .expect("tools/list response owns an array")
            .push(json!({
                "name": SET_ANDROID_VOLUME_TOOL,
                "description": "Preview one exact Android audio-stream level, or apply it with fresh bounds validation and one principal/session/stream/level-bound single-use MCP-Capability-Grant.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "stream": {
                            "type": "string",
                            "enum": ["alarm", "call", "music", "notification", "ring", "system"],
                            "description": "Exact documented Termux:API audio stream.",
                        },
                        "level": {
                            "type": "integer",
                            "minimum": 0,
                            "description": "Exact target level; a fresh status read enforces the live stream maximum.",
                        },
                        "dry_run": {
                            "type": "boolean",
                            "description": "Defaults to true. Explicit false additionally requires one exact request-scoped grant.",
                        },
                    },
                    "required": ["stream", "level"],
                    "additionalProperties": false,
                },
            }));
    }

    if state.command_execution_enabled {
        body.pointer_mut("/result/tools")
            .and_then(Value::as_array_mut)
            .expect("tools/list response owns an array")
            .push(json!({
                "name": RUN_COMMAND_PROFILE_TOOL,
                "description": "Run one reviewed read-only diagnostic profile with fixed executable identity, argv, safe-root working directory, empty environment, null stdin, and bounded output.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "profile": {
                            "type": "string",
                            "enum": command_profile_ids().collect::<Vec<_>>(),
                            "description": "Reviewed project-owned diagnostic profile identifier.",
                        },
                    },
                    "required": ["profile"],
                    "additionalProperties": false,
                },
            }));
    }

    (StatusCode::OK, Json(body)).into_response()
}

#[rustfmt::skip]
async fn handle_tool_call(
    id: Option<Value>,
    params: Option<Value>,
    state: &McpTransportState,
    session_id: &str,
    capability_grant: Option<&str>,
) -> Response {
    let params = match params {
        Some(params) => params,
        None => return invalid_params(id, TOOL_CALL_PARAMS_INVALID),
    };

    let call = match serde_json::from_value::<ToolCallParams>(params) {
        Ok(call) => call,
        Err(_error) => return invalid_params(id, TOOL_CALL_PARAMS_INVALID),
    };

    if capability_grant.is_some()
        && !matches!(
            call.name.as_str(),
            CREATE_DIRECTORY_TOOL | SET_ANDROID_VOLUME_TOOL
        )
    {
        return capability_context_not_allowed(id);
    }

    match call.name.as_str() {
        RUNTIME_STATUS_TOOL => handle_runtime_status_call(id, call.arguments, state),
        PLATFORM_INFO_TOOL => handle_no_argument_tool_call(
            id,
            call.arguments,
            &state.audit_counters,
            state.command_execution_enabled,
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
            state.command_execution_enabled,
            NoArgumentToolContract {
                tool_name: ANDROID_STATUS_TOOL,
                gate_name: ANDROID_STATUS_GATE,
                allowed_reason: ANDROID_STATUS_ALLOWED,
                denied_reason: ANDROID_STATUS_ARGUMENTS_DENIED,
                response_builder: android_status_response,
            },
        ),
        ANDROID_BATTERY_STATUS_TOOL => {
            handle_android_battery_status_call(id, call.arguments, state).await
        }
        ANDROID_VOLUME_STATUS_TOOL => {
            handle_android_volume_status_call(id, call.arguments, state).await
        }
        SET_ANDROID_VOLUME_TOOL => {
            handle_set_android_volume_call(
                id,
                call.arguments.into_value(),
                state,
                session_id,
                capability_grant,
            )
            .await
        }
        PROJECT_SERVICE_STATUS_TOOL => {
            project_service_status_response(
                id,
                call.arguments.into_value(),
                &state.audit_counters,
            )
        }
        CREATE_DIRECTORY_TOOL => {
            handle_create_directory_call(
                id,
                call.arguments.into_value(),
                state,
                session_id,
                capability_grant,
            )
            .await
        }
        COPY_FILE_TOOL => {
            handle_copy_file_call(
                id,
                call.arguments.into_value(),
                &state.file_tools,
                &state.audit_counters,
            )
            .await
        }
        HASH_FILE_TOOL => {
            handle_hash_file_call(
                id,
                call.arguments.into_value(),
                &state.file_tools,
                &state.audit_counters,
            )
            .await
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
        PATH_METADATA_TOOL => {
            handle_path_metadata_call(
                id,
                call.arguments.into_value(),
                &state.file_tools,
                &state.audit_counters,
            )
            .await
        }
        READ_BINARY_FILE_TOOL => {
            handle_read_binary_file_call(
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
        SEARCH_TEXT_TOOL => {
            handle_search_text_call(
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
        RUN_COMMAND_PROFILE_TOOL => {
            handle_run_command_profile_call(
                id,
                call.arguments.into_value(),
                state,
            )
            .await
        }
        _ => method_not_available(id, "The requested tool is not available in this runtime posture."),
    }
}

fn handle_runtime_status_call(
    id: Option<Value>,
    arguments: ToolArguments,
    state: &McpTransportState,
) -> Response {
    if !arguments.is_omitted_or_empty_object() {
        record_read_only_denied(
            &state.audit_counters,
            RUNTIME_STATUS_TOOL,
            RUNTIME_STATUS_GATE,
            RUNTIME_STATUS_ARGUMENTS_DENIED,
        );
        return invalid_params(id, TOOL_ARGUMENTS_INVALID);
    }

    record_read_only_allowed(
        &state.audit_counters,
        RUNTIME_STATUS_TOOL,
        RUNTIME_STATUS_GATE,
        RUNTIME_STATUS_ALLOWED,
    );
    runtime_status_response(
        id,
        &state.audit_counters,
        state.create_directory_authority.is_some(),
        state.android_battery_status_enabled,
        state.android_volume_status_enabled,
        state.android_volume_control_enabled,
        state.command_execution_enabled,
    )
}

async fn handle_android_battery_status_call(
    id: Option<Value>,
    arguments: ToolArguments,
    state: &McpTransportState,
) -> Response {
    if !arguments.is_omitted_or_empty_object() {
        record_read_only_denied(
            &state.audit_counters,
            ANDROID_BATTERY_STATUS_TOOL,
            ANDROID_BATTERY_STATUS_GATE,
            ANDROID_BATTERY_STATUS_ARGUMENTS_DENIED,
        );
        return invalid_params(id, TOOL_ARGUMENTS_INVALID);
    }

    #[cfg(not(feature = "android-battery-status"))]
    {
        record_read_only_denied(
            &state.audit_counters,
            ANDROID_BATTERY_STATUS_TOOL,
            ANDROID_BATTERY_STATUS_GATE,
            ANDROID_BATTERY_STATUS_FEATURE_DISABLED,
        );
        tool_error_result(
            id,
            ANDROID_BATTERY_STATUS_TOOL,
            "android_battery_status_unavailable",
            ANDROID_BATTERY_STATUS_FEATURE_DISABLED,
        )
    }

    #[cfg(feature = "android-battery-status")]
    {
        if !state.android_battery_status_enabled {
            record_read_only_denied(
                &state.audit_counters,
                ANDROID_BATTERY_STATUS_TOOL,
                ANDROID_BATTERY_STATUS_GATE,
                ANDROID_BATTERY_STATUS_RUNTIME_DISABLED,
            );
            return tool_error_result(
                id,
                ANDROID_BATTERY_STATUS_TOOL,
                "android_battery_status_unavailable",
                ANDROID_BATTERY_STATUS_RUNTIME_DISABLED,
            );
        }

        match state.android_battery_client.collect().await {
            Ok(status) => {
                record_read_only_allowed(
                    &state.audit_counters,
                    ANDROID_BATTERY_STATUS_TOOL,
                    ANDROID_BATTERY_STATUS_GATE,
                    ANDROID_BATTERY_STATUS_ALLOWED,
                );
                ok_result(
                    id,
                    "android_battery_status: bounded read-only Termux:API telemetry collected."
                        .to_owned(),
                    json!(status),
                )
            }
            Err(error) => {
                let reason_code = error.reason_code();
                record_read_only_denied(
                    &state.audit_counters,
                    ANDROID_BATTERY_STATUS_TOOL,
                    ANDROID_BATTERY_STATUS_GATE,
                    reason_code,
                );
                tool_error_result(
                    id,
                    ANDROID_BATTERY_STATUS_TOOL,
                    "android_battery_status_unavailable",
                    reason_code,
                )
            }
        }
    }
}

async fn handle_android_volume_status_call(
    id: Option<Value>,
    arguments: ToolArguments,
    state: &McpTransportState,
) -> Response {
    if !arguments.is_omitted_or_empty_object() {
        record_read_only_denied(
            &state.audit_counters,
            ANDROID_VOLUME_STATUS_TOOL,
            ANDROID_VOLUME_STATUS_GATE,
            ANDROID_VOLUME_STATUS_ARGUMENTS_DENIED,
        );
        return invalid_params(id, TOOL_ARGUMENTS_INVALID);
    }

    #[cfg(not(feature = "android-volume-status"))]
    {
        record_read_only_denied(
            &state.audit_counters,
            ANDROID_VOLUME_STATUS_TOOL,
            ANDROID_VOLUME_STATUS_GATE,
            ANDROID_VOLUME_STATUS_FEATURE_DISABLED,
        );
        tool_error_result(
            id,
            ANDROID_VOLUME_STATUS_TOOL,
            "android_volume_status_unavailable",
            ANDROID_VOLUME_STATUS_FEATURE_DISABLED,
        )
    }

    #[cfg(feature = "android-volume-status")]
    {
        if !state.android_volume_status_enabled {
            record_read_only_denied(
                &state.audit_counters,
                ANDROID_VOLUME_STATUS_TOOL,
                ANDROID_VOLUME_STATUS_GATE,
                ANDROID_VOLUME_STATUS_RUNTIME_DISABLED,
            );
            return tool_error_result(
                id,
                ANDROID_VOLUME_STATUS_TOOL,
                "android_volume_status_unavailable",
                ANDROID_VOLUME_STATUS_RUNTIME_DISABLED,
            );
        }

        match state.android_volume_client.collect().await {
            Ok(status) => {
                record_read_only_allowed(
                    &state.audit_counters,
                    ANDROID_VOLUME_STATUS_TOOL,
                    ANDROID_VOLUME_STATUS_GATE,
                    ANDROID_VOLUME_STATUS_ALLOWED,
                );
                ok_result(
                    id,
                    "android_volume_status: bounded read-only Termux:API telemetry collected."
                        .to_owned(),
                    json!(status),
                )
            }
            Err(error) => {
                let reason_code = error.reason_code();
                record_read_only_denied(
                    &state.audit_counters,
                    ANDROID_VOLUME_STATUS_TOOL,
                    ANDROID_VOLUME_STATUS_GATE,
                    reason_code,
                );
                tool_error_result(
                    id,
                    ANDROID_VOLUME_STATUS_TOOL,
                    "android_volume_status_unavailable",
                    reason_code,
                )
            }
        }
    }
}

async fn handle_set_android_volume_call(
    id: Option<Value>,
    arguments: Option<Value>,
    state: &McpTransportState,
    session_id: &str,
    capability_grant: Option<&str>,
) -> Response {
    let args = match arguments
        .and_then(|arguments| serde_json::from_value::<SetAndroidVolumeArguments>(arguments).ok())
    {
        Some(args) => args,
        None => {
            record_volume_control_decision(
                &state.audit_counters,
                AuditMode::DryRun,
                AuditDecision::Denied,
                ANDROID_VOLUME_CONTROL_INVALID_ARGUMENTS,
            );
            return invalid_params(
                id,
                "set_android_volume requires exact stream, level, and optional dry_run arguments.",
            );
        }
    };
    let dry_run = args.dry_run.unwrap_or(true);
    let mode = if dry_run {
        AuditMode::DryRun
    } else {
        AuditMode::Mutating
    };

    #[cfg(not(feature = "android-volume-control"))]
    {
        let _ = (args, session_id, capability_grant);
        record_volume_control_decision(
            &state.audit_counters,
            mode,
            AuditDecision::Denied,
            ANDROID_VOLUME_CONTROL_FEATURE_DISABLED,
        );
        tool_error_result(
            id,
            SET_ANDROID_VOLUME_TOOL,
            "android_volume_control_unavailable",
            ANDROID_VOLUME_CONTROL_FEATURE_DISABLED,
        )
    }

    #[cfg(feature = "android-volume-control")]
    {
        if !state.android_volume_control_enabled {
            record_volume_control_decision(
                &state.audit_counters,
                mode,
                AuditDecision::Denied,
                ANDROID_VOLUME_CONTROL_RUNTIME_DISABLED,
            );
            return tool_error_result(
                id,
                SET_ANDROID_VOLUME_TOOL,
                "android_volume_control_unavailable",
                ANDROID_VOLUME_CONTROL_RUNTIME_DISABLED,
            );
        }

        let stream = match args.stream.parse::<AndroidVolumeStreamName>() {
            Ok(stream) => stream,
            Err(error) => {
                record_volume_control_decision(
                    &state.audit_counters,
                    mode,
                    AuditDecision::Denied,
                    error.reason_code(),
                );
                return invalid_params(id, "set_android_volume stream is not allowlisted.");
            }
        };

        if dry_run {
            return match state
                .android_volume_control_client
                .preview(stream, args.level)
                .await
            {
                Ok(result) => {
                    record_volume_control_decision(
                        &state.audit_counters,
                        mode,
                        AuditDecision::Allowed,
                        ANDROID_VOLUME_CONTROL_PREVIEW_ALLOWED,
                    );
                    ok_result(
                        id,
                        "set_android_volume: validated one exact stream and level without mutation."
                            .to_owned(),
                        json!(result),
                    )
                }
                Err(error) => volume_control_error_response(id, state, mode, error),
            };
        }

        let prepared = match state
            .android_volume_control_client
            .prepare_mutation(stream, args.level)
            .await
        {
            Ok(prepared) => prepared,
            Err(error) => return volume_control_error_response(id, state, mode, error),
        };
        let target = match AndroidVolumeGrantTarget::new(stream, args.level) {
            Ok(target) => target,
            Err(error) => {
                record_volume_control_decision(
                    &state.audit_counters,
                    mode,
                    AuditDecision::Denied,
                    error.reason_code(),
                );
                return capability_authorization_denied(id, error.reason_code());
            }
        };
        let authority = state
            .android_volume_control_authority
            .as_ref()
            .expect("enabled Android volume control owns an authority");
        if let Err(error) =
            authority.consume_at(capability_grant, session_id, target, current_unix_seconds())
        {
            record_volume_control_decision(
                &state.audit_counters,
                mode,
                AuditDecision::Denied,
                error.reason_code(),
            );
            return capability_authorization_denied(id, error.reason_code());
        }

        // The prepared operation owns the one mutation permit and is detached
        // from request cancellation. If the HTTP future is dropped after grant
        // consumption, the fixed command, verification, and rollback sequence
        // still runs to completion under its own strict process deadlines.
        let worker = tokio::spawn(prepared.execute());
        match worker.await {
            Ok(Ok(result)) => {
                record_volume_control_decision(
                    &state.audit_counters,
                    mode,
                    AuditDecision::Allowed,
                    ANDROID_VOLUME_CONTROL_MUTATION_ALLOWED,
                );
                ok_result(
                    id,
                    "set_android_volume: exact stream mutation completed and was verified."
                        .to_owned(),
                    json!(result),
                )
            }
            Ok(Err(error)) => volume_control_error_response(id, state, mode, error),
            Err(_error) => volume_control_error_response(
                id,
                state,
                mode,
                AndroidVolumeControlError::WorkerFailed,
            ),
        }
    }
}

#[cfg(feature = "android-volume-control")]
fn volume_control_error_response(
    id: Option<Value>,
    state: &McpTransportState,
    mode: AuditMode,
    error: AndroidVolumeControlError,
) -> Response {
    let reason_code = error.reason_code();
    record_volume_control_decision(
        &state.audit_counters,
        mode,
        AuditDecision::Denied,
        reason_code,
    );
    tool_error_result(
        id,
        SET_ANDROID_VOLUME_TOOL,
        "android_volume_control_failed",
        reason_code,
    )
}

async fn handle_run_command_profile_call(
    id: Option<Value>,
    arguments: Option<Value>,
    state: &McpTransportState,
) -> Response {
    let arguments = match arguments {
        Some(arguments) => arguments,
        None => {
            record_read_only_denied(
                &state.audit_counters,
                RUN_COMMAND_PROFILE_TOOL,
                COMMAND_EXECUTION_GATE,
                COMMAND_MISSING_ARGUMENTS_REASON,
            );
            return invalid_params(id, "run_command_profile requires a profile argument.");
        }
    };
    let args = match serde_json::from_value::<RunCommandProfileArguments>(arguments) {
        Ok(args) => args,
        Err(_error) => {
            record_read_only_denied(
                &state.audit_counters,
                RUN_COMMAND_PROFILE_TOOL,
                COMMAND_EXECUTION_GATE,
                COMMAND_INVALID_ARGUMENTS_REASON,
            );
            return invalid_params(id, TOOL_ARGUMENTS_INVALID);
        }
    };

    #[cfg(not(feature = "command-execution"))]
    {
        let _ = args;
        record_read_only_denied(
            &state.audit_counters,
            RUN_COMMAND_PROFILE_TOOL,
            COMMAND_EXECUTION_GATE,
            COMMAND_FEATURE_DISABLED_REASON,
        );
        tool_error_result(
            id,
            RUN_COMMAND_PROFILE_TOOL,
            COMMAND_EXECUTION_ERROR,
            COMMAND_FEATURE_DISABLED_REASON,
        )
    }

    #[cfg(feature = "command-execution")]
    {
        let policy = CommandExecutionPolicy::new();
        let decision = policy.evaluate(
            &args.profile,
            state.command_execution_enabled,
            !state.file_tools.safe_roots().is_empty(),
        );
        if !decision.allowed {
            record_command_policy_decision(&state.audit_counters, &decision);
            return if decision.reason_code == COMMAND_PROFILE_NOT_ALLOWLISTED_REASON {
                invalid_params(id, TOOL_ARGUMENTS_INVALID)
            } else {
                tool_error_result(
                    id,
                    RUN_COMMAND_PROFILE_TOOL,
                    COMMAND_EXECUTION_ERROR,
                    decision.reason_code,
                )
            };
        }

        let profile = decision
            .profile
            .expect("allowed command policy decisions own a fixed profile");
        match state.command_execution_client.execute(profile).await {
            Ok(result) => {
                record_command_policy_decision(&state.audit_counters, &decision);
                ok_result(
                    id,
                    format!(
                        "run_command_profile: fixed read-only profile {} completed within all bounds.",
                        profile.id,
                    ),
                    json!(result),
                )
            }
            Err(error) => {
                let failure = CommandPolicyDecision {
                    allowed: false,
                    reason_code: command_execution_error_reason(error),
                    profile: Some(profile),
                };
                record_command_policy_decision(&state.audit_counters, &failure);
                tool_error_result(
                    id,
                    RUN_COMMAND_PROFILE_TOOL,
                    COMMAND_EXECUTION_ERROR,
                    failure.reason_code,
                )
            }
        }
    }
}

#[cfg(feature = "command-execution")]
fn command_execution_error_reason(error: CommandExecutionError) -> &'static str {
    match error {
        CommandExecutionError::ProgramUnavailable => COMMAND_PROGRAM_UNAVAILABLE_REASON,
        CommandExecutionError::SpawnFailed => COMMAND_SPAWN_FAILED_REASON,
        CommandExecutionError::WaitFailed => COMMAND_WAIT_FAILED_REASON,
        CommandExecutionError::TimedOut => COMMAND_TIMEOUT_REASON,
        CommandExecutionError::StdoutLimitExceeded => COMMAND_STDOUT_LIMIT_REASON,
        CommandExecutionError::StderrLimitExceeded => COMMAND_STDERR_LIMIT_REASON,
        CommandExecutionError::ProgramFailed => COMMAND_PROGRAM_FAILED_REASON,
        CommandExecutionError::InvalidUtf8 => COMMAND_OUTPUT_INVALID_UTF8_REASON,
        CommandExecutionError::ConcurrencyLimitExceeded => COMMAND_CONCURRENCY_LIMIT_REASON,
    }
}

#[cfg(feature = "command-execution")]
fn record_command_policy_decision(
    counters: &SharedAuditCounters,
    decision: &CommandPolicyDecision,
) {
    let event = CommandExecutionPolicy::new().audit_decision(current_unix_seconds(), decision);
    record_audit_event(counters, &event);
}

fn handle_no_argument_tool_call(
    id: Option<Value>,
    arguments: ToolArguments,
    audit_counters: &SharedAuditCounters,
    command_execution_enabled: bool,
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
    (contract.response_builder)(id, audit_counters, command_execution_enabled)
}

fn available_tools(
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    android_volume_control_enabled: bool,
    command_execution_enabled: bool,
) -> Vec<&'static str> {
    let mut tools = BASE_AVAILABLE_TOOLS.to_vec();
    if android_battery_status_enabled {
        tools.push(ANDROID_BATTERY_STATUS_TOOL);
    }
    if android_volume_status_enabled {
        tools.push(ANDROID_VOLUME_STATUS_TOOL);
    }
    if android_volume_control_enabled {
        tools.push(SET_ANDROID_VOLUME_TOOL);
    }
    if command_execution_enabled {
        tools.push(RUN_COMMAND_PROFILE_TOOL);
    }
    tools
}

#[rustfmt::skip]
fn runtime_status_response(
    id: Option<Value>,
    audit_counters: &SharedAuditCounters,
    create_directory_mutation_enabled: bool,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    android_volume_control_enabled: bool,
    command_execution_enabled: bool,
) -> Response {
    let audit_counters_snapshot = audit_counters_snapshot(audit_counters);
    let available_tools = available_tools(
        android_battery_status_enabled,
        android_volume_status_enabled,
        android_volume_control_enabled,
        command_execution_enabled,
    );
    let battery_mode = if android_battery_status_enabled {
        "read_only_battery_telemetry"
    } else {
        "disabled"
    };
    let volume_mode = if android_volume_status_enabled {
        "read_only_volume_telemetry"
    } else {
        "disabled"
    };
    let volume_control_mode = if android_volume_control_enabled {
        "preview_or_request_scoped_single_use_grant"
    } else {
        "disabled"
    };
    let android_platform_mode = match (
        android_battery_status_enabled,
        android_volume_status_enabled,
        android_volume_control_enabled,
    ) {
        (true, true, true) => {
            "read_only_battery_and_volume_telemetry_plus_bounded_volume_control"
        }
        (true, false, true) => "read_only_battery_telemetry_plus_bounded_volume_control",
        (false, true, true) => "read_only_volume_telemetry_plus_bounded_volume_control",
        (false, false, true) => "bounded_request_authorized_volume_control",
        (true, true, false) => "read_only_battery_and_volume_telemetry",
        (true, false, false) => "read_only_battery_telemetry",
        (false, true, false) => "read_only_volume_telemetry",
        (false, false, false) => "disabled",
    };
    let command_execution_mode = if command_execution_enabled {
        "fixed_read_only_server_diagnostics"
    } else {
        "disabled"
    };
    let create_directory_mode = if create_directory_mutation_enabled {
        "dry_run_or_request_scoped_single_use_grant"
    } else {
        "dry_run_only_mutation_disabled"
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
                        "text": format!(
                            "termux-mcp-edge runtime_status: transport=streamable-http-2025-11-25-session-scoped-no-sse, platform_info=read-only-non-sensitive, android_status=read-only-allowlisted, android_platform={}, android_battery_status={}, android_volume_status={}, android_volume_control={}, project_service_status=read-only-allowlisted, create_directory_mutation={}, filesystem=create-directory-copy-file-hash-file-list-metadata-read-search-and-dry-run-write-file, android_device_control={}, command_execution={}, arbitrary_command_execution=disabled",
                            android_platform_mode,
                            battery_mode,
                            volume_mode,
                            volume_control_mode,
                            create_directory_mode,
                            if android_volume_control_enabled { "bounded_request_authorized_volume" } else { "disabled" },
                            command_execution_mode,
                        ),
                    },
                ],
                "structuredContent": {
                    "server": "termux-mcp-edge",
                    "version": env!("CARGO_PKG_VERSION"),
                    "transport": "streamable_http_2025_11_25",
                    "sessionManagement": "bounded_uuid_idle_expiry",
                    "serverSentEvents": false,
                    "availableTools": available_tools,
                    "platformInfo": true,
                    "platformInfoMode": "read_only_non_sensitive_metadata",
                    "androidStatus": true,
                    "androidStatusMode": "read_only_allowlisted_status_no_api_or_control",
                    "projectServiceStatus": true,
                    "projectServiceStatusMode": "read_only_allowlisted_project_service_status",
                    "filesystemTools": true,
                    "filesystemToolMode": "create_directory_copy_file_hash_file_list_directory_path_metadata_read_binary_file_read_file_search_text_and_default_dry_run_write_file",
                    "binaryFileReads": true,
                    "binaryFileReadEncoding": "base64",
                    "binaryFileReadMaxBytes": MAX_BINARY_READ_BYTES,
                    "binaryFileReadMaxResponseBytes": MAX_BINARY_READ_RESPONSE_BYTES,
                    "fileHashing": true,
                    "fileHashAlgorithm": "sha256",
                    "fileHashMaxBytes": MAX_HASH_FILE_BYTES,
                    "createDirectoryMutationEnabled": create_directory_mutation_enabled,
                    "createDirectoryMutationMode": create_directory_mode,
                    "createDirectoryGrantRequired": create_directory_mutation_enabled,
                    "createDirectoryGrantHeader": CREATE_DIRECTORY_GRANT_HEADER,
                    "createDirectoryGrantTtlSeconds": CREATE_DIRECTORY_GRANT_TTL_SECONDS,
                    "fileWrites": true,
                    "fileWriteMode": "dry_run_by_default_explicit_false_required",
                    "androidPlatformTools": android_battery_status_enabled || android_volume_status_enabled || android_volume_control_enabled,
                    "androidPlatformToolMode": android_platform_mode,
                    "androidBatteryStatusCompiled": cfg!(feature = "android-battery-status"),
                    "androidBatteryStatusEnabled": android_battery_status_enabled,
                    "androidVolumeStatusCompiled": cfg!(feature = "android-volume-status"),
                    "androidVolumeStatusEnabled": android_volume_status_enabled,
                    "androidVolumeControlCompiled": cfg!(feature = "android-volume-control"),
                    "androidVolumeControlEnabled": android_volume_control_enabled,
                    "androidVolumeControlMode": volume_control_mode,
                    "androidVolumeGrantRequired": android_volume_control_enabled,
                    "androidVolumeGrantHeader": CREATE_DIRECTORY_GRANT_HEADER,
                    "androidVolumeGrantTtlSeconds": ANDROID_VOLUME_GRANT_TTL_SECONDS_IF_COMPILED,
                    "androidDeviceControl": android_volume_control_enabled,
                    "commandExecutionCompiled": cfg!(feature = "command-execution"),
                    "commandExecution": command_execution_enabled,
                    "commandExecutionMode": command_execution_mode,
                    "arbitraryCommandExecution": false,
                    "highImpactTools": android_volume_control_enabled,
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
    _command_execution_enabled: bool,
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
    command_execution_enabled: bool,
) -> Response {
    let status = collect_android_status(command_execution_enabled);
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
            let error_id = id.clone();
            let summary = if result.truncated {
                format!(
                    "Listed {} safe-rooted filesystem entries; the bounded result was truncated.",
                    result.entries.len()
                )
            } else {
                format!("Listed {} safe-rooted filesystem entries.", result.entries.len())
            };
            let Some(response) = bounded_ok_result(
                id,
                summary,
                json!(result),
                MAX_LIST_RESPONSE_BYTES,
            ) else {
                record_filesystem_denied(
                    audit_counters,
                    LIST_DIRECTORY_TOOL,
                    FILESYSTEM_READ_GATE,
                    AuditMode::ReadOnly,
                    FILESYSTEM_RESPONSE_TOO_LARGE,
                );
                return bounded_payload_too_large(
                    error_id,
                    "Directory listing exceeds the staged response byte limit.",
                    MAX_LIST_RESPONSE_BYTES,
                );
            };
            record_filesystem_allowed(
                audit_counters,
                LIST_DIRECTORY_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_LIST_ALLOWED,
            );
            response
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
async fn handle_path_metadata_call(
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
                PATH_METADATA_TOOL,
                FILESYSTEM_METADATA_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_MISSING_ARGUMENTS,
            );
            return invalid_params(id, "path_metadata requires a path argument.");
        }
    };

    let args = match serde_json::from_value::<PathMetadataArguments>(arguments) {
        Ok(args) => args,
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                PATH_METADATA_TOOL,
                FILESYSTEM_METADATA_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_INVALID_ARGUMENTS,
            );
            return invalid_params(id, TOOL_ARGUMENTS_INVALID);
        }
    };

    match file_tools.path_metadata(args.path).await {
        Ok(result) => {
            let error_id = id.clone();
            let Some(response) = bounded_ok_result(
                id,
                "Read bounded metadata for one safe-rooted filesystem object.".to_owned(),
                json!(result),
                MAX_PATH_METADATA_RESPONSE_BYTES,
            ) else {
                record_filesystem_denied(
                    audit_counters,
                    PATH_METADATA_TOOL,
                    FILESYSTEM_METADATA_GATE,
                    AuditMode::ReadOnly,
                    FILESYSTEM_RESPONSE_TOO_LARGE,
                );
                return bounded_payload_too_large(
                    error_id,
                    "Filesystem metadata exceeds the staged response byte limit.",
                    MAX_PATH_METADATA_RESPONSE_BYTES,
                );
            };
            record_filesystem_allowed(
                audit_counters,
                PATH_METADATA_TOOL,
                FILESYSTEM_METADATA_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_METADATA_ALLOWED,
            );
            response
        }
        Err(AppError::PathTraversal { .. }) => {
            record_filesystem_denied(
                audit_counters,
                PATH_METADATA_TOOL,
                FILESYSTEM_METADATA_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_SAFE_ROOT_REJECTED,
            );
            invalid_params(
                id,
                "Filesystem safe-root validation failed: requested path is outside the configured safe roots.",
            )
        }
        Err(AppError::PathNotFound) => {
            record_filesystem_denied(
                audit_counters,
                PATH_METADATA_TOOL,
                FILESYSTEM_METADATA_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_METADATA_NOT_FOUND,
            );
            invalid_params(id, "Filesystem object does not exist.")
        }
        Err(AppError::UnsupportedPathType) => {
            record_filesystem_denied(
                audit_counters,
                PATH_METADATA_TOOL,
                FILESYSTEM_METADATA_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_METADATA_UNSUPPORTED,
            );
            invalid_params(id, "Filesystem object type is not supported.")
        }
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                PATH_METADATA_TOOL,
                FILESYSTEM_METADATA_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_METADATA_FAILED,
            );
            internal_error(id, "Filesystem metadata lookup failed.")
        }
    }
}

fn binary_read_success_envelope_fits(id: Option<Value>) -> bool {
    let maximum_summary = format!(
        "Read and base64-encoded {MAX_BINARY_READ_BYTES} bytes from one safe-rooted regular file."
    );
    let body = result_body(
        id,
        maximum_summary,
        json!({
            "encoding": "base64",
            "data": "",
            "sizeBytes": MAX_BINARY_READ_BYTES,
            "maxFileBytes": MAX_BINARY_READ_BYTES,
            "maxResponseBytes": MAX_BINARY_READ_RESPONSE_BYTES,
        }),
    );
    serde_json::to_vec(&body)
        .ok()
        .and_then(|serialized| serialized.len().checked_add(MAX_BINARY_READ_BASE64_BYTES))
        .is_some_and(|bytes| bytes <= MAX_BINARY_READ_RESPONSE_BYTES)
}

#[rustfmt::skip]
async fn handle_read_binary_file_call(
    id: Option<Value>,
    arguments: Option<Value>,
    file_tools: &FileSystemTools,
    audit_counters: &SharedAuditCounters,
) -> Response {
    if !binary_read_success_envelope_fits(id.clone()) {
        record_filesystem_denied(
            audit_counters,
            READ_BINARY_FILE_TOOL,
            FILESYSTEM_READ_GATE,
            AuditMode::ReadOnly,
            FILESYSTEM_RESPONSE_TOO_LARGE,
        );
        return bounded_payload_too_large(
            id,
            "Binary file response exceeds the staged read_binary_file response byte limit.",
            MAX_BINARY_READ_RESPONSE_BYTES,
        );
    }

    let arguments = match arguments {
        Some(arguments) => arguments,
        None => {
            record_filesystem_denied(
                audit_counters,
                READ_BINARY_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_MISSING_ARGUMENTS,
            );
            return invalid_params(id, "read_binary_file requires a path argument.");
        }
    };
    let args = match serde_json::from_value::<ReadBinaryFileArguments>(arguments) {
        Ok(args) => args,
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                READ_BINARY_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_INVALID_ARGUMENTS,
            );
            return invalid_params(id, TOOL_ARGUMENTS_INVALID);
        }
    };

    match file_tools.read_binary_file(args.path).await {
        Ok(result) => {
            let error_id = id.clone();
            let summary = format!(
                "Read and base64-encoded {} bytes from one safe-rooted regular file.",
                result.size_bytes
            );
            let Some(response) = bounded_ok_result(
                id,
                summary,
                json!(result),
                MAX_BINARY_READ_RESPONSE_BYTES,
            ) else {
                record_filesystem_denied(
                    audit_counters,
                    READ_BINARY_FILE_TOOL,
                    FILESYSTEM_READ_GATE,
                    AuditMode::ReadOnly,
                    FILESYSTEM_RESPONSE_TOO_LARGE,
                );
                return bounded_payload_too_large(
                    error_id,
                    "Binary file response exceeds the staged read_binary_file response byte limit.",
                    MAX_BINARY_READ_RESPONSE_BYTES,
                );
            };
            record_filesystem_allowed(
                audit_counters,
                READ_BINARY_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_BINARY_READ_ALLOWED,
            );
            response
        }
        Err(AppError::PathTraversal { .. }) => {
            record_filesystem_denied(
                audit_counters,
                READ_BINARY_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_SAFE_ROOT_REJECTED,
            );
            invalid_params(id, "Path is outside the configured filesystem safe roots.")
        }
        Err(AppError::PathNotFound) => {
            record_filesystem_denied(
                audit_counters,
                READ_BINARY_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_BINARY_READ_NOT_FOUND,
            );
            invalid_params(id, "Binary file does not exist.")
        }
        Err(AppError::UnsupportedPathType) => {
            record_filesystem_denied(
                audit_counters,
                READ_BINARY_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_BINARY_READ_UNSUPPORTED,
            );
            invalid_params(id, "Binary read target must be one regular file.")
        }
        Err(AppError::FileTooLarge { .. }) => {
            record_filesystem_denied(
                audit_counters,
                READ_BINARY_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_BINARY_READ_TOO_LARGE,
            );
            payload_too_large(id, "Binary file exceeds the staged read_binary_file byte limit.")
        }
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                READ_BINARY_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_BINARY_READ_FAILED,
            );
            internal_error(id, "Binary file read failed.")
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
            let error_id = id.clone();
            let summary = format!(
                "Read {} UTF-8 bytes from a safe-rooted file.",
                result.size
            );
            let Some(response) = bounded_ok_result(
                id,
                summary,
                json!(result),
                MAX_READ_RESPONSE_BYTES,
            ) else {
                record_filesystem_denied(
                    audit_counters,
                    READ_FILE_TOOL,
                    FILESYSTEM_READ_GATE,
                    AuditMode::ReadOnly,
                    FILESYSTEM_RESPONSE_TOO_LARGE,
                );
                return bounded_payload_too_large(
                    error_id,
                    "File content exceeds the staged read_file response byte limit.",
                    MAX_READ_RESPONSE_BYTES,
                );
            };
            record_filesystem_allowed(
                audit_counters,
                READ_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_READ_ALLOWED,
            );
            response
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
        Err(AppError::InvalidFileEncoding) => {
            record_filesystem_denied(
                audit_counters,
                READ_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_READ_ENCODING_INVALID,
            );
            invalid_params(id, "File content must be valid UTF-8.")
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
async fn handle_create_directory_call(
    id: Option<Value>,
    arguments: Option<Value>,
    state: &McpTransportState,
    session_id: &str,
    capability_grant: Option<&str>,
) -> Response {
    let file_tools = &state.file_tools;
    let audit_counters = &state.audit_counters;
    let arguments = match arguments {
        Some(arguments) => arguments,
        None => {
            record_filesystem_denied(
                audit_counters,
                CREATE_DIRECTORY_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::DryRun,
                FILESYSTEM_MISSING_ARGUMENTS,
            );
            return invalid_params(id, "create_directory requires a path argument.");
        }
    };

    let args = match serde_json::from_value::<CreateDirectoryArguments>(arguments) {
        Ok(args) => args,
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                CREATE_DIRECTORY_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::DryRun,
                FILESYSTEM_INVALID_ARGUMENTS,
            );
            return invalid_params(id, TOOL_ARGUMENTS_INVALID);
        }
    };

    let dry_run = args.dry_run.unwrap_or(true);
    let mode = filesystem_write_mode(dry_run);
    if !dry_run && state.create_directory_authority.is_none() {
        record_filesystem_denied(
            audit_counters,
            CREATE_DIRECTORY_TOOL,
            FILESYSTEM_WRITE_GATE,
            mode,
            FILESYSTEM_CREATE_MUTATION_DISABLED,
        );
        return capability_authorization_denied(id, FILESYSTEM_CREATE_MUTATION_DISABLED);
    }
    let success_text = if dry_run {
        "Validated one safe-rooted directory creation without mutation."
    } else {
        "Created one safe-rooted directory with fixed mode 0700."
    };
    if file_tools
        .create_directory_response_preview(&args.path, dry_run)
        .ok()
        .is_some_and(|preview| {
            bounded_ok_result(
                id.clone(),
                success_text.to_owned(),
                json!(preview),
                MAX_CREATE_DIRECTORY_RESPONSE_BYTES,
            )
            .is_none()
        })
    {
        record_filesystem_denied(
            audit_counters,
            CREATE_DIRECTORY_TOOL,
            FILESYSTEM_WRITE_GATE,
            mode,
            FILESYSTEM_RESPONSE_TOO_LARGE,
        );
        return bounded_payload_too_large(
            id,
            "Directory creation response exceeds the staged response byte limit.",
            MAX_CREATE_DIRECTORY_RESPONSE_BYTES,
        );
    }
    let operation = if dry_run {
        file_tools.create_directory(args.path, Some(true)).await
    } else {
        let prepared: PreparedCreateDirectoryMutation = match file_tools
            .prepare_create_directory_mutation(args.path)
            .await
        {
            Ok(prepared) => prepared,
            Err(error) => return create_directory_filesystem_error(id, audit_counters, mode, error),
        };
        let authority = state
            .create_directory_authority
            .clone()
            .expect("enabled create_directory mutation owns an authority");
        let session_id = session_id.to_owned();
        let capability_grant = capability_grant.map(str::to_owned);
        match tokio::task::spawn_blocking(move || {
            prepared.execute_authorized(|target| {
                authority.consume_at(
                    capability_grant.as_deref(),
                    &session_id,
                    target,
                    current_unix_seconds(),
                )
            })
        })
        .await
        {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(AuthorizedCreateDirectoryError::Authorization(error))) => {
                record_filesystem_denied(
                    audit_counters,
                    CREATE_DIRECTORY_TOOL,
                    FILESYSTEM_WRITE_GATE,
                    mode,
                    error.reason_code(),
                );
                return capability_authorization_denied(id, error.reason_code());
            }
            Ok(Err(AuthorizedCreateDirectoryError::Filesystem(error))) => Err(error),
            Err(_error) => Err(AppError::Io(std::io::Error::other(
                "create directory worker failed",
            ))),
        }
    };

    match operation {
        Ok(result) => {
            let error_id = id.clone();
            let Some(response) = bounded_ok_result(
                id,
                success_text.to_owned(),
                json!(result),
                MAX_CREATE_DIRECTORY_RESPONSE_BYTES,
            ) else {
                record_filesystem_denied(
                    audit_counters,
                    CREATE_DIRECTORY_TOOL,
                    FILESYSTEM_WRITE_GATE,
                    mode,
                    FILESYSTEM_RESPONSE_TOO_LARGE,
                );
                return bounded_payload_too_large(
                    error_id,
                    "Directory creation response exceeds the staged response byte limit.",
                    MAX_CREATE_DIRECTORY_RESPONSE_BYTES,
                );
            };
            record_filesystem_allowed(
                audit_counters,
                CREATE_DIRECTORY_TOOL,
                FILESYSTEM_WRITE_GATE,
                mode,
                if dry_run {
                    FILESYSTEM_DRY_RUN_ALLOWED
                } else {
                    FILESYSTEM_CREATE_ALLOWED
                },
            );
            response
        }
        Err(error) => create_directory_filesystem_error(id, audit_counters, mode, error),
    }
}

#[rustfmt::skip]
fn create_directory_filesystem_error(
    id: Option<Value>,
    audit_counters: &SharedAuditCounters,
    mode: AuditMode,
    error: AppError,
) -> Response {
    match error {
        AppError::PathTraversal { .. } => {
            record_filesystem_denied(
                audit_counters,
                CREATE_DIRECTORY_TOOL,
                FILESYSTEM_WRITE_GATE,
                mode,
                FILESYSTEM_SAFE_ROOT_REJECTED,
            );
            invalid_params(
                id,
                "Filesystem safe-root validation failed: requested path is outside the configured safe roots.",
            )
        }
        AppError::PathNotFound => {
            record_filesystem_denied(
                audit_counters,
                CREATE_DIRECTORY_TOOL,
                FILESYSTEM_WRITE_GATE,
                mode,
                FILESYSTEM_CREATE_PARENT_NOT_FOUND,
            );
            invalid_params(id, "Filesystem parent directory does not exist.")
        }
        AppError::PathAlreadyExists => {
            record_filesystem_denied(
                audit_counters,
                CREATE_DIRECTORY_TOOL,
                FILESYSTEM_WRITE_GATE,
                mode,
                FILESYSTEM_CREATE_EXISTS,
            );
            invalid_params(id, "Filesystem destination already exists.")
        }
        _error => {
            record_filesystem_denied(
                audit_counters,
                CREATE_DIRECTORY_TOOL,
                FILESYSTEM_WRITE_GATE,
                mode,
                FILESYSTEM_CREATE_FAILED,
            );
            internal_error(id, "Filesystem directory creation failed.")
        }
    }
}

#[rustfmt::skip]
async fn handle_copy_file_call(
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
                COPY_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::DryRun,
                FILESYSTEM_MISSING_ARGUMENTS,
            );
            return invalid_params(
                id,
                "copy_file requires source_path and destination_path arguments.",
            );
        }
    };

    let args = match serde_json::from_value::<CopyFileArguments>(arguments) {
        Ok(args) => args,
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                COPY_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::DryRun,
                FILESYSTEM_INVALID_ARGUMENTS,
            );
            return invalid_params(id, TOOL_ARGUMENTS_INVALID);
        }
    };

    let dry_run = args.dry_run.unwrap_or(true);
    let mode = filesystem_write_mode(dry_run);
    let success_text = if dry_run {
        "Validated one bounded safe-rooted file copy without mutation."
    } else {
        "Copied one bounded safe-rooted file with fixed mode 0600."
    };
    if file_tools
        .copy_file_response_preview(&args.source_path, &args.destination_path, dry_run)
        .ok()
        .is_some_and(|preview| {
            bounded_ok_result(
                id.clone(),
                success_text.to_owned(),
                json!(preview),
                MAX_COPY_FILE_RESPONSE_BYTES,
            )
            .is_none()
        })
    {
        record_filesystem_denied(
            audit_counters,
            COPY_FILE_TOOL,
            FILESYSTEM_WRITE_GATE,
            mode,
            FILESYSTEM_RESPONSE_TOO_LARGE,
        );
        return bounded_payload_too_large(
            id,
            "File copy response exceeds the staged response byte limit.",
            MAX_COPY_FILE_RESPONSE_BYTES,
        );
    }

    match file_tools
        .copy_file(
            args.source_path,
            args.destination_path,
            Some(dry_run),
        )
        .await
    {
        Ok(result) => {
            let error_id = id.clone();
            let Some(response) = bounded_ok_result(
                id,
                success_text.to_owned(),
                json!(result),
                MAX_COPY_FILE_RESPONSE_BYTES,
            ) else {
                record_filesystem_denied(
                    audit_counters,
                    COPY_FILE_TOOL,
                    FILESYSTEM_WRITE_GATE,
                    mode,
                    FILESYSTEM_RESPONSE_TOO_LARGE,
                );
                return bounded_payload_too_large(
                    error_id,
                    "File copy response exceeds the staged response byte limit.",
                    MAX_COPY_FILE_RESPONSE_BYTES,
                );
            };
            record_filesystem_allowed(
                audit_counters,
                COPY_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                mode,
                if dry_run {
                    FILESYSTEM_DRY_RUN_ALLOWED
                } else {
                    FILESYSTEM_COPY_ALLOWED
                },
            );
            response
        }
        Err(AppError::PathTraversal { .. }) => {
            record_filesystem_denied(
                audit_counters,
                COPY_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                mode,
                FILESYSTEM_SAFE_ROOT_REJECTED,
            );
            invalid_params(
                id,
                "Filesystem safe-root validation failed: requested path is outside the configured safe roots.",
            )
        }
        Err(AppError::CopySourceNotFound) => {
            record_filesystem_denied(
                audit_counters,
                COPY_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                mode,
                FILESYSTEM_COPY_SOURCE_NOT_FOUND,
            );
            invalid_params(id, "Filesystem copy source does not exist.")
        }
        Err(AppError::CopyDestinationParentNotFound) => {
            record_filesystem_denied(
                audit_counters,
                COPY_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                mode,
                FILESYSTEM_COPY_PARENT_NOT_FOUND,
            );
            invalid_params(id, "Filesystem copy destination parent does not exist.")
        }
        Err(AppError::CopySourceDestinationSame) => {
            record_filesystem_denied(
                audit_counters,
                COPY_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                mode,
                FILESYSTEM_COPY_SAME_PATH,
            );
            invalid_params(id, "Filesystem copy source and destination must differ.")
        }
        Err(AppError::PathAlreadyExists) => {
            record_filesystem_denied(
                audit_counters,
                COPY_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                mode,
                FILESYSTEM_CREATE_EXISTS,
            );
            invalid_params(id, "Filesystem destination already exists.")
        }
        Err(AppError::UnsupportedPathType) => {
            record_filesystem_denied(
                audit_counters,
                COPY_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                mode,
                FILESYSTEM_COPY_SOURCE_UNSUPPORTED,
            );
            invalid_params(id, "Filesystem copy source must be a regular file.")
        }
        Err(AppError::FileTooLarge { .. }) => {
            record_filesystem_denied(
                audit_counters,
                COPY_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                mode,
                FILESYSTEM_COPY_SOURCE_TOO_LARGE,
            );
            payload_too_large(id, "File exceeds the staged copy_file byte limit.")
        }
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                COPY_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                mode,
                FILESYSTEM_COPY_FAILED,
            );
            internal_error(id, "Filesystem copy failed.")
        }
    }
}

#[rustfmt::skip]
async fn handle_hash_file_call(
    id: Option<Value>,
    arguments: Option<Value>,
    file_tools: &FileSystemTools,
    audit_counters: &SharedAuditCounters,
) -> Response {
    let maximum_summary = format!(
        "Computed a SHA-256 digest for {MAX_HASH_FILE_BYTES} bytes from one safe-rooted regular file."
    );
    if bounded_ok_result(
        id.clone(),
        maximum_summary,
        json!({
            "algorithm": "sha256",
            "digest": "f".repeat(64),
            "sizeBytes": MAX_HASH_FILE_BYTES,
        }),
        MAX_HASH_FILE_RESPONSE_BYTES,
    )
    .is_none()
    {
        record_filesystem_denied(
            audit_counters,
            HASH_FILE_TOOL,
            FILESYSTEM_READ_GATE,
            AuditMode::ReadOnly,
            FILESYSTEM_RESPONSE_TOO_LARGE,
        );
        return bounded_payload_too_large(
            id,
            "File digest response exceeds the staged hash_file response byte limit.",
            MAX_HASH_FILE_RESPONSE_BYTES,
        );
    }

    let arguments = match arguments {
        Some(arguments) => arguments,
        None => {
            record_filesystem_denied(
                audit_counters,
                HASH_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_MISSING_ARGUMENTS,
            );
            return invalid_params(id, "hash_file requires a path argument.");
        }
    };

    let args = match serde_json::from_value::<HashFileArguments>(arguments) {
        Ok(args) => args,
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                HASH_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_INVALID_ARGUMENTS,
            );
            return invalid_params(id, TOOL_ARGUMENTS_INVALID);
        }
    };

    match file_tools.hash_file(args.path).await {
        Ok(result) => {
            let error_id = id.clone();
            let summary = format!(
                "Computed a SHA-256 digest for {} bytes from one safe-rooted regular file.",
                result.size_bytes
            );
            let Some(response) = bounded_ok_result(
                id,
                summary,
                json!(result),
                MAX_HASH_FILE_RESPONSE_BYTES,
            ) else {
                record_filesystem_denied(
                    audit_counters,
                    HASH_FILE_TOOL,
                    FILESYSTEM_READ_GATE,
                    AuditMode::ReadOnly,
                    FILESYSTEM_RESPONSE_TOO_LARGE,
                );
                return bounded_payload_too_large(
                    error_id,
                    "File digest response exceeds the staged hash_file response byte limit.",
                    MAX_HASH_FILE_RESPONSE_BYTES,
                );
            };
            record_filesystem_allowed(
                audit_counters,
                HASH_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_HASH_ALLOWED,
            );
            response
        }
        Err(AppError::PathTraversal { .. }) => {
            record_filesystem_denied(
                audit_counters,
                HASH_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_SAFE_ROOT_REJECTED,
            );
            invalid_params(
                id,
                "Filesystem safe-root validation failed: requested path is outside the configured safe roots.",
            )
        }
        Err(AppError::PathNotFound) => {
            record_filesystem_denied(
                audit_counters,
                HASH_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_HASH_NOT_FOUND,
            );
            invalid_params(id, "Filesystem hash target does not exist.")
        }
        Err(AppError::UnsupportedPathType) => {
            record_filesystem_denied(
                audit_counters,
                HASH_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_HASH_UNSUPPORTED,
            );
            invalid_params(id, "Filesystem hash target must be a regular file.")
        }
        Err(AppError::FileTooLarge { .. }) => {
            record_filesystem_denied(
                audit_counters,
                HASH_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_HASH_TOO_LARGE,
            );
            payload_too_large(id, "File exceeds the staged hash_file byte limit.")
        }
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                HASH_FILE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_HASH_FAILED,
            );
            internal_error(id, "Filesystem hashing failed.")
        }
    }
}

#[rustfmt::skip]
async fn handle_search_text_call(
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
                SEARCH_TEXT_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_MISSING_ARGUMENTS,
            );
            return invalid_params(id, "search_text requires path and query arguments.");
        }
    };

    let args = match serde_json::from_value::<SearchTextArguments>(arguments) {
        Ok(args) => args,
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                SEARCH_TEXT_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_INVALID_ARGUMENTS,
            );
            return invalid_params(id, TOOL_ARGUMENTS_INVALID);
        }
    };

    if args.query.is_empty()
        || args.query.len() > MAX_SEARCH_QUERY_BYTES
        || args.query.chars().any(|character| matches!(character, '\0' | '\n' | '\r'))
    {
        record_filesystem_denied(
            audit_counters,
            SEARCH_TEXT_TOOL,
            FILESYSTEM_READ_GATE,
            AuditMode::ReadOnly,
            FILESYSTEM_SEARCH_INVALID_QUERY,
        );
        return invalid_params(
            id,
            &format!(
                "search_text.query must be one non-empty line of at most {MAX_SEARCH_QUERY_BYTES} UTF-8 bytes."
            ),
        );
    }
    if let Some(max_depth) = args.max_depth {
        if !(MIN_SEARCH_DEPTH..=MAX_SEARCH_DEPTH).contains(&max_depth) {
            record_filesystem_denied(
                audit_counters,
                SEARCH_TEXT_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_INVALID_DEPTH,
            );
            return invalid_params(
                id,
                &format!(
                    "search_text.max_depth must be between {MIN_SEARCH_DEPTH} and {MAX_SEARCH_DEPTH}."
                ),
            );
        }
    }

    match file_tools.search_text(args.path, args.query, args.max_depth).await {
        Ok(result) => {
            let error_id = id.clone();
            let summary = if result.truncated {
                format!(
                    "Located {} safe-rooted literal text matches; the bounded search was truncated.",
                    result.matches.len()
                )
            } else {
                format!(
                    "Located {} safe-rooted literal text matches.",
                    result.matches.len()
                )
            };
            let Some(response) = bounded_ok_result(
                id,
                summary,
                json!(result),
                MAX_SEARCH_RESPONSE_BYTES,
            ) else {
                record_filesystem_denied(
                    audit_counters,
                    SEARCH_TEXT_TOOL,
                    FILESYSTEM_READ_GATE,
                    AuditMode::ReadOnly,
                    FILESYSTEM_RESPONSE_TOO_LARGE,
                );
                return bounded_payload_too_large(
                    error_id,
                    "Text-search results exceed the staged response byte limit.",
                    MAX_SEARCH_RESPONSE_BYTES,
                );
            };
            record_filesystem_allowed(
                audit_counters,
                SEARCH_TEXT_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_SEARCH_ALLOWED,
            );
            response
        }
        Err(AppError::PathTraversal { .. }) => {
            record_filesystem_denied(
                audit_counters,
                SEARCH_TEXT_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_SAFE_ROOT_REJECTED,
            );
            invalid_params(
                id,
                "Filesystem safe-root validation failed: requested path is outside the configured safe roots.",
            )
        }
        Err(AppError::InvalidSearchQuery) => {
            record_filesystem_denied(
                audit_counters,
                SEARCH_TEXT_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_SEARCH_INVALID_QUERY,
            );
            invalid_params(id, "Search query does not satisfy the literal text-search contract.")
        }
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                SEARCH_TEXT_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_SEARCH_FAILED,
            );
            internal_error(id, "Filesystem search failed.")
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
    result_response(result_body(id, text, structured_content))
}

#[rustfmt::skip]
fn tool_error_result(
    id: Option<Value>,
    tool_name: &'static str,
    error_name: &'static str,
    reason_code: &'static str,
) -> Response {
    (
        StatusCode::OK,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "result": {
                "content": [
                    {
                        "type": "text",
                        "text": format!("{tool_name} unavailable: {reason_code}"),
                    },
                ],
                "structuredContent": {
                    "error": error_name,
                    "reasonCode": reason_code,
                },
                "isError": true
            },
        })),
    )
        .into_response()
}

fn bounded_ok_result(
    id: Option<Value>,
    text: String,
    structured_content: Value,
    max_response_bytes: usize,
) -> Option<Response> {
    let body = result_body(id, text, structured_content);
    if serde_json::to_vec(&body).ok()?.len() > max_response_bytes {
        return None;
    }

    Some(result_response(body))
}

fn result_body(id: Option<Value>, text: String, structured_content: Value) -> Value {
    json!({
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
    })
}

fn result_response(body: Value) -> Response {
    (StatusCode::OK, Json(body)).into_response()
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

fn bounded_payload_too_large(
    id: Option<Value>,
    message: &str,
    max_response_bytes: usize,
) -> Response {
    let body = json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": {
            "code": -32001,
            "message": "Payload too large",
            "data": message,
        },
    });
    if serde_json::to_vec(&body).is_ok_and(|serialized| serialized.len() <= max_response_bytes) {
        return (StatusCode::PAYLOAD_TOO_LARGE, Json(body)).into_response();
    }

    payload_too_large(None, message)
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

fn record_volume_control_decision(
    counters: &SharedAuditCounters,
    mode: AuditMode,
    decision: AuditDecision,
    reason_code: &'static str,
) {
    let event = AuditEvent::new(
        current_unix_seconds(),
        SET_ANDROID_VOLUME_TOOL,
        ANDROID_VOLUME_CONTROL_GATE,
        mode,
        decision,
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

    #[cfg(feature = "command-execution")]
    use crate::command_policy::{
        COMMAND_PROFILE_ALLOWED_REASON, COMMAND_RUNTIME_DISABLED_REASON,
    };

    #[cfg(all(
        feature = "android-battery-status",
        feature = "android-volume-status"
    ))]
    #[tokio::test]
    async fn combined_read_only_android_posture_is_deterministic() {
        use axum::body::to_bytes;

        let safe_root = tempfile::tempdir().unwrap();
        let state = McpTransportState::new(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            FileSystemTools::new(vec![safe_root.path().to_path_buf()]),
            true,
            true,
            false,
            None,
        );

        let tools = tools_list_response(Some(json!("tools")), &state);
        let tools: Value = serde_json::from_slice(
            &to_bytes(tools.into_body(), 64 * 1024).await.unwrap(),
        )
        .unwrap();
        assert_eq!(
            tools["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tool| tool["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "runtime_status",
                "platform_info",
                "android_status",
                "project_service_status",
                "create_directory",
                "copy_file",
                "hash_file",
                "list_directory",
                "path_metadata",
                "read_binary_file",
                "read_file",
                "search_text",
                "write_file",
                "android_battery_status",
                "android_volume_status",
            ]
        );

        let runtime = runtime_status_response(
            Some(json!("runtime")),
            &state.audit_counters,
            state.create_directory_authority.is_some(),
            state.android_battery_status_enabled,
            state.android_volume_status_enabled,
            state.android_volume_control_enabled,
            state.command_execution_enabled,
        );
        let runtime: Value = serde_json::from_slice(
            &to_bytes(runtime.into_body(), 64 * 1024).await.unwrap(),
        )
        .unwrap();
        assert_eq!(
            runtime["result"]["structuredContent"]["androidPlatformToolMode"],
            "read_only_battery_and_volume_telemetry"
        );
        assert_eq!(
            runtime["result"]["structuredContent"]["availableTools"]
                .as_array()
                .unwrap()
                .len(),
            15
        );
    }

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

    #[cfg(feature = "android-battery-status")]
    #[tokio::test]
    async fn enabled_battery_tool_returns_allowlisted_telemetry_and_audits_success() {
        use std::{fs, os::unix::fs::PermissionsExt, time::Duration};

        use axum::body::to_bytes;

        let _test_guard = crate::android_battery::ANDROID_BATTERY_TEST_LOCK
            .lock()
            .await;

        let program_root = tempfile::tempdir().unwrap();
        let program = program_root.path().join("battery-status");
        fs::write(
            &program,
            concat!(
                "#!/bin/sh\n",
                "set -eu\n",
                "test \"$#\" -eq 0\n",
                "printf '%s' '{\"percentage\":73,\"temperature\":30.5,\"status\":\"DISCHARGING\",\"android_id\":\"redacted\"}'\n",
            ),
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let client = AndroidBatteryClient::with_program_and_limits(
            program,
            Duration::from_secs(1),
            crate::android_battery::MAX_BATTERY_STDOUT_BYTES,
            crate::android_battery::MAX_BATTERY_STDERR_BYTES,
        );
        let safe_root = tempfile::tempdir().unwrap();
        let state = McpTransportState::with_android_battery_client(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            FileSystemTools::new(vec![safe_root.path().to_path_buf()]),
            true,
            client,
        );

        let response = handle_android_battery_status_call(
            Some(json!("battery-success")),
            ToolArguments::Present(json!({})),
            &state,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 64 * 1024).await.unwrap(),
        )
        .unwrap();
        assert_eq!(payload["result"]["isError"], false);
        assert_eq!(payload["result"]["structuredContent"]["percentage"], 73);
        assert_eq!(
            payload["result"]["structuredContent"]["temperature_celsius"],
            30.5
        );
        assert!(payload["result"]["structuredContent"]
            .get("android_id")
            .is_none());

        let counters = state.audit_counters.lock().unwrap().clone();
        assert_eq!(counters.by_tool[ANDROID_BATTERY_STATUS_TOOL].allowed, 1);
        assert_eq!(
            counters.by_reason_code[ANDROID_BATTERY_STATUS_ALLOWED].allowed,
            1
        );

        let tools = tools_list_response(Some(json!("tools")), &state);
        let tools: Value = serde_json::from_slice(
            &to_bytes(tools.into_body(), 64 * 1024).await.unwrap(),
        )
        .unwrap();
        let names = tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert!(names.contains(&ANDROID_BATTERY_STATUS_TOOL));
    }

    #[cfg(feature = "android-battery-status")]
    #[tokio::test]
    async fn disabled_battery_tool_is_hidden_and_returns_stable_audited_error() {
        use std::time::Duration;

        use axum::body::to_bytes;

        let program_root = tempfile::tempdir().unwrap();
        let client = AndroidBatteryClient::with_program_and_limits(
            program_root.path().join("must-not-run"),
            Duration::from_secs(1),
            crate::android_battery::MAX_BATTERY_STDOUT_BYTES,
            crate::android_battery::MAX_BATTERY_STDERR_BYTES,
        );
        let safe_root = tempfile::tempdir().unwrap();
        let state = McpTransportState::with_android_battery_client(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            FileSystemTools::new(vec![safe_root.path().to_path_buf()]),
            false,
            client,
        );

        let response = handle_android_battery_status_call(
            Some(json!("battery-disabled")),
            ToolArguments::Omitted,
            &state,
        )
        .await;
        let payload: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 64 * 1024).await.unwrap(),
        )
        .unwrap();
        assert_eq!(payload["result"]["isError"], true);
        assert_eq!(
            payload["result"]["structuredContent"]["reasonCode"],
            ANDROID_BATTERY_STATUS_RUNTIME_DISABLED
        );

        let tools = tools_list_response(Some(json!("tools")), &state);
        let tools: Value = serde_json::from_slice(
            &to_bytes(tools.into_body(), 64 * 1024).await.unwrap(),
        )
        .unwrap();
        assert!(tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool["name"] != ANDROID_BATTERY_STATUS_TOOL));

        let counters = state.audit_counters.lock().unwrap().clone();
        assert_eq!(counters.by_tool[ANDROID_BATTERY_STATUS_TOOL].denied, 1);
        assert_eq!(
            counters.by_reason_code[ANDROID_BATTERY_STATUS_RUNTIME_DISABLED].denied,
            1
        );
    }

    #[cfg(feature = "android-volume-status")]
    #[tokio::test]
    async fn enabled_volume_tool_returns_canonical_streams_and_audits_success() {
        use std::{fs, os::unix::fs::PermissionsExt, time::Duration};

        use axum::body::to_bytes;

        let _test_guard = crate::android_provider::ANDROID_PROVIDER_TEST_LOCK
            .lock()
            .await;
        let program_root = tempfile::tempdir().unwrap();
        let program = program_root.path().join("volume-status");
        fs::write(
            &program,
            concat!(
                "#!/bin/sh\n",
                "set -eu\n",
                "test \"$#\" -eq 0\n",
                "printf '%s' '[{\"stream\":\"system\",\"volume\":2,\"max_volume\":7},{\"stream\":\"alarm\",\"volume\":4,\"max_volume\":7},{\"stream\":\"call\",\"volume\":1,\"max_volume\":5},{\"stream\":\"notification\",\"volume\":3,\"max_volume\":7},{\"stream\":\"ring\",\"volume\":6,\"max_volume\":7},{\"stream\":\"music\",\"volume\":5,\"max_volume\":15}]'\n",
            ),
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let client = AndroidVolumeClient::with_program_and_limits(
            program,
            Duration::from_secs(1),
            crate::android_volume::MAX_VOLUME_STDOUT_BYTES,
            crate::android_volume::MAX_VOLUME_STDERR_BYTES,
        );
        let safe_root = tempfile::tempdir().unwrap();
        let state = McpTransportState::with_android_volume_client(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            FileSystemTools::new(vec![safe_root.path().to_path_buf()]),
            true,
            client,
        );

        let response = handle_android_volume_status_call(
            Some(json!("volume-success")),
            ToolArguments::Present(json!({})),
            &state,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 64 * 1024).await.unwrap(),
        )
        .unwrap();
        assert_eq!(payload["result"]["isError"], false);
        assert_eq!(
            payload["result"]["structuredContent"]["streams"][0]["stream"],
            "alarm"
        );
        assert_eq!(
            payload["result"]["structuredContent"]["streams"][2]["maxVolume"],
            15
        );

        let counters = state.audit_counters.lock().unwrap().clone();
        assert_eq!(counters.by_tool[ANDROID_VOLUME_STATUS_TOOL].allowed, 1);
        assert_eq!(
            counters.by_reason_code[ANDROID_VOLUME_STATUS_ALLOWED].allowed,
            1
        );

        let tools = tools_list_response(Some(json!("tools")), &state);
        let tools: Value = serde_json::from_slice(
            &to_bytes(tools.into_body(), 64 * 1024).await.unwrap(),
        )
        .unwrap();
        assert!(tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == ANDROID_VOLUME_STATUS_TOOL));
    }

    #[cfg(feature = "android-volume-status")]
    #[tokio::test]
    async fn disabled_volume_tool_is_hidden_and_returns_stable_audited_error() {
        use std::time::Duration;

        use axum::body::to_bytes;

        let program_root = tempfile::tempdir().unwrap();
        let client = AndroidVolumeClient::with_program_and_limits(
            program_root.path().join("must-not-run"),
            Duration::from_secs(1),
            crate::android_volume::MAX_VOLUME_STDOUT_BYTES,
            crate::android_volume::MAX_VOLUME_STDERR_BYTES,
        );
        let safe_root = tempfile::tempdir().unwrap();
        let state = McpTransportState::with_android_volume_client(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            FileSystemTools::new(vec![safe_root.path().to_path_buf()]),
            false,
            client,
        );

        let response = handle_android_volume_status_call(
            Some(json!("volume-disabled")),
            ToolArguments::Omitted,
            &state,
        )
        .await;
        let payload: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 64 * 1024).await.unwrap(),
        )
        .unwrap();
        assert_eq!(payload["result"]["isError"], true);
        assert_eq!(
            payload["result"]["structuredContent"]["error"],
            "android_volume_status_unavailable"
        );
        assert_eq!(
            payload["result"]["structuredContent"]["reasonCode"],
            ANDROID_VOLUME_STATUS_RUNTIME_DISABLED
        );

        let tools = tools_list_response(Some(json!("tools")), &state);
        let tools: Value = serde_json::from_slice(
            &to_bytes(tools.into_body(), 64 * 1024).await.unwrap(),
        )
        .unwrap();
        assert!(tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool["name"] != ANDROID_VOLUME_STATUS_TOOL));

        let counters = state.audit_counters.lock().unwrap().clone();
        assert_eq!(counters.by_tool[ANDROID_VOLUME_STATUS_TOOL].denied, 1);
        assert_eq!(
            counters.by_reason_code[ANDROID_VOLUME_STATUS_RUNTIME_DISABLED].denied,
            1
        );
    }

    #[cfg(feature = "android-volume-control")]
    fn test_volume_control_client(
    ) -> (tempfile::TempDir, AndroidVolumeControlClient) {
        use std::{fs, os::unix::fs::PermissionsExt, time::Duration};

        let root = tempfile::tempdir().unwrap();
        let program = root.path().join("termux-volume");
        let level = root.path().join("level");
        let calls = root.path().join("calls");
        fs::write(&level, "5\n").unwrap();
        fs::write(
            &program,
            format!(
                r#"#!/bin/sh
set -eu
level='{}'
calls='{}'
if [ "$#" -eq 0 ]; then
  IFS= read -r music <"$level"
  printf '[{{"stream":"alarm","volume":1,"max_volume":7}},{{"stream":"call","volume":1,"max_volume":5}},{{"stream":"music","volume":%s,"max_volume":15}},{{"stream":"notification","volume":2,"max_volume":7}},{{"stream":"ring","volume":3,"max_volume":7}},{{"stream":"system","volume":2,"max_volume":7}}]' "$music"
  exit 0
fi
test "$#" -eq 2
printf '%s:%s:%s\n' "$1" "$2" "$PWD" >>"$calls"
printf '%s\n' "$2" >"$level"
"#,
                level.display(),
                calls.display(),
            ),
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let client = AndroidVolumeControlClient::with_program_and_limits(
            program,
            Duration::from_secs(1),
            crate::android_volume::MAX_VOLUME_STDOUT_BYTES,
            crate::android_volume::MAX_VOLUME_STDERR_BYTES,
        );
        (root, client)
    }

    #[cfg(feature = "android-volume-control")]
    fn test_volume_authority() -> AndroidVolumeGrantAuthority {
        AndroidVolumeGrantAuthority::from_hex_key(
            "test-volume-1",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "private-static-principal",
        )
        .unwrap()
    }

    #[cfg(feature = "android-volume-control")]
    #[tokio::test]
    async fn enabled_volume_control_is_preview_first_exact_and_single_use() {
        use axum::body::to_bytes;

        let _guard = crate::android_provider::ANDROID_PROVIDER_TEST_LOCK.lock().await;
        let (program_root, client) = test_volume_control_client();
        let safe_root = tempfile::tempdir().unwrap();
        let authority = test_volume_authority();
        let state = McpTransportState::with_android_volume_control_client(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            FileSystemTools::new(vec![safe_root.path().to_path_buf()]),
            Some(authority.clone()),
            client,
        );
        let session = "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee";
        let target = AndroidVolumeGrantTarget::new(AndroidVolumeStreamName::Music, 9).unwrap();
        let grant = authority
            .issue_at(session, target, current_unix_seconds())
            .unwrap();

        let tools = tools_list_response(Some(json!("tools")), &state);
        let tools: Value = serde_json::from_slice(
            &to_bytes(tools.into_body(), 64 * 1024).await.unwrap(),
        )
        .unwrap();
        let tool = tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == SET_ANDROID_VOLUME_TOOL)
            .unwrap();
        assert_eq!(
            tool["inputSchema"]["properties"]["stream"]["enum"],
            json!(["alarm", "call", "music", "notification", "ring", "system"])
        );
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);

        let preview = handle_set_android_volume_call(
            Some(json!("preview")),
            Some(json!({"stream":"music", "level":9})),
            &state,
            session,
            Some(&grant),
        )
        .await;
        assert_eq!(preview.status(), StatusCode::OK);
        let preview: Value = serde_json::from_slice(
            &to_bytes(preview.into_body(), 64 * 1024).await.unwrap(),
        )
        .unwrap();
        assert_eq!(preview["result"]["structuredContent"]["dryRun"], true);
        assert_eq!(preview["result"]["structuredContent"]["previousLevel"], 5);
        assert!(!program_root.path().join("calls").exists());

        let mutation = handle_set_android_volume_call(
            Some(json!("mutation")),
            Some(json!({"stream":"music", "level":9, "dry_run":false})),
            &state,
            session,
            Some(&grant),
        )
        .await;
        assert_eq!(mutation.status(), StatusCode::OK);
        let mutation: Value = serde_json::from_slice(
            &to_bytes(mutation.into_body(), 64 * 1024).await.unwrap(),
        )
        .unwrap();
        assert_eq!(mutation["result"]["structuredContent"]["verified"], true);
        assert_eq!(
            std::fs::read_to_string(program_root.path().join("calls")).unwrap(),
            "music:9:/\n"
        );

        let replay = handle_set_android_volume_call(
            Some(json!("replay")),
            Some(json!({"stream":"music", "level":9, "dry_run":false})),
            &state,
            session,
            Some(&grant),
        )
        .await;
        assert_eq!(replay.status(), StatusCode::FORBIDDEN);
        let replay: Value = serde_json::from_slice(
            &to_bytes(replay.into_body(), 64 * 1024).await.unwrap(),
        )
        .unwrap();
        assert_eq!(replay["error"]["data"]["reason"], "capability_grant_replayed");

        let runtime = handle_runtime_status_call(
            Some(json!("runtime")),
            ToolArguments::Omitted,
            &state,
        );
        let runtime: Value = serde_json::from_slice(
            &to_bytes(runtime.into_body(), 64 * 1024).await.unwrap(),
        )
        .unwrap();
        assert_eq!(
            runtime["result"]["structuredContent"]["androidVolumeControlEnabled"],
            true
        );
        assert_eq!(
            runtime["result"]["structuredContent"]["androidDeviceControl"],
            true
        );
        assert_eq!(
            runtime["result"]["structuredContent"]["androidPlatformToolMode"],
            "bounded_request_authorized_volume_control"
        );
        assert_eq!(runtime["result"]["structuredContent"]["highImpactTools"], true);

        let counters = state.audit_counters.lock().unwrap().clone();
        assert_eq!(counters.by_tool[SET_ANDROID_VOLUME_TOOL].allowed, 2);
        assert_eq!(counters.by_tool[SET_ANDROID_VOLUME_TOOL].denied, 1);
        let serialized = serde_json::to_string(&counters).unwrap();
        assert!(!serialized.contains(&grant));
        assert!(!serialized.contains("private-static-principal"));
    }

    #[cfg(feature = "android-volume-control")]
    #[tokio::test]
    async fn disabled_or_invalid_volume_control_never_spawns() {
        use axum::body::to_bytes;

        let program_root = tempfile::tempdir().unwrap();
        let client = AndroidVolumeControlClient::with_program_and_limits(
            program_root.path().join("must-not-run"),
            std::time::Duration::from_secs(1),
            crate::android_volume::MAX_VOLUME_STDOUT_BYTES,
            crate::android_volume::MAX_VOLUME_STDERR_BYTES,
        );
        let safe_root = tempfile::tempdir().unwrap();
        let state = McpTransportState::with_android_volume_control_client(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            FileSystemTools::new(vec![safe_root.path().to_path_buf()]),
            None,
            client,
        );

        let tools = tools_list_response(Some(json!("tools")), &state);
        let tools: Value = serde_json::from_slice(
            &to_bytes(tools.into_body(), 64 * 1024).await.unwrap(),
        )
        .unwrap();
        assert!(tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool["name"] != SET_ANDROID_VOLUME_TOOL));

        let disabled = handle_set_android_volume_call(
            Some(json!("disabled")),
            Some(json!({"stream":"music", "level":9, "dry_run":false})),
            &state,
            "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee",
            None,
        )
        .await;
        let disabled: Value = serde_json::from_slice(
            &to_bytes(disabled.into_body(), 64 * 1024).await.unwrap(),
        )
        .unwrap();
        assert_eq!(
            disabled["result"]["structuredContent"]["reasonCode"],
            ANDROID_VOLUME_CONTROL_RUNTIME_DISABLED
        );

        let authority = test_volume_authority();
        let (_active_root, active_client) = test_volume_control_client();
        let active = McpTransportState::with_android_volume_control_client(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            FileSystemTools::new(vec![safe_root.path().to_path_buf()]),
            Some(authority),
            active_client,
        );
        for arguments in [
            json!({"stream":"media", "level":9}),
            json!({"stream":"music", "level":9, "program":"sh"}),
            json!({"stream":"music", "level":9, "argv":[]}),
            json!({"stream":"music", "level":9, "environment":{}}),
            json!({"stream":"music", "level":9, "timeout":999}),
        ] {
            let response = handle_set_android_volume_call(
                Some(json!("invalid")),
                Some(arguments),
                &active,
                "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee",
                None,
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
    }

    #[cfg(feature = "command-execution")]
    fn test_command_client(
        safe_root: &std::path::Path,
        script: &str,
    ) -> (tempfile::TempDir, CommandExecutionClient) {
        use std::{fs, os::unix::fs::PermissionsExt};

        let program_root = tempfile::tempdir().unwrap();
        let program = program_root.path().join("fixed-command-program");
        fs::write(&program, format!("#!/bin/sh\nset -eu\n{script}\n")).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let client = CommandExecutionClient::with_program_and_concurrency(
            program,
            safe_root.to_path_buf(),
            2,
        )
        .unwrap();
        (program_root, client)
    }

    #[cfg(feature = "command-execution")]
    #[tokio::test]
    async fn enabled_command_tool_is_discovered_executes_and_audits_fixed_profile() {
        use axum::body::to_bytes;

        let _guard = crate::bounded_process::BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let safe_root = tempfile::tempdir().unwrap();
        let (_program_root, client) = test_command_client(
            safe_root.path(),
            "test \"$#\" -eq 1; test \"$1\" = --version; printf version-output; printf warning >&2",
        );
        let state = McpTransportState::with_command_execution_client(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            FileSystemTools::new(vec![safe_root.path().to_path_buf()]),
            true,
            client,
        );

        let tools = tools_list_response(Some(json!("tools")), &state);
        let tools: Value = serde_json::from_slice(
            &to_bytes(tools.into_body(), 64 * 1024).await.unwrap(),
        )
        .unwrap();
        let command = tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == RUN_COMMAND_PROFILE_TOOL)
            .unwrap();
        assert_eq!(
            command["inputSchema"]["properties"]["profile"]["enum"],
            json!(["server_version", "server_help", "execution_boundary"])
        );
        assert_eq!(command["inputSchema"]["additionalProperties"], false);

        let response = handle_run_command_profile_call(
            Some(json!("command-success")),
            Some(json!({"profile": "server_version"})),
            &state,
        )
        .await;
        let payload: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 64 * 1024).await.unwrap(),
        )
        .unwrap();
        assert_eq!(payload["result"]["isError"], false);
        assert_eq!(
            payload["result"]["structuredContent"]["profile"],
            "server_version"
        );
        assert_eq!(
            payload["result"]["structuredContent"]["stdout"],
            "version-output"
        );
        assert_eq!(payload["result"]["structuredContent"]["stderr"], "warning");

        let counters = state.audit_counters.lock().unwrap().clone();
        assert_eq!(counters.by_tool[RUN_COMMAND_PROFILE_TOOL].allowed, 1);
        assert_eq!(
            counters.by_reason_code[COMMAND_PROFILE_ALLOWED_REASON].allowed,
            1
        );
    }

    #[cfg(feature = "command-execution")]
    #[tokio::test]
    async fn disabled_command_tool_is_hidden_and_direct_calls_never_spawn() {
        use axum::body::to_bytes;

        let safe_root = tempfile::tempdir().unwrap();
        let marker = safe_root.path().join("must-not-exist");
        let script = format!("touch '{}'", marker.display());
        let (_program_root, client) = test_command_client(safe_root.path(), &script);
        let state = McpTransportState::with_command_execution_client(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            FileSystemTools::new(vec![safe_root.path().to_path_buf()]),
            false,
            client,
        );

        let tools = tools_list_response(Some(json!("tools")), &state);
        let tools: Value = serde_json::from_slice(
            &to_bytes(tools.into_body(), 64 * 1024).await.unwrap(),
        )
        .unwrap();
        assert!(tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool["name"] != RUN_COMMAND_PROFILE_TOOL));

        let response = handle_run_command_profile_call(
            Some(json!("command-disabled")),
            Some(json!({"profile": "server_version"})),
            &state,
        )
        .await;
        let payload: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 64 * 1024).await.unwrap(),
        )
        .unwrap();
        assert_eq!(payload["result"]["isError"], true);
        assert_eq!(
            payload["result"]["structuredContent"]["reasonCode"],
            COMMAND_RUNTIME_DISABLED_REASON
        );
        assert!(!marker.exists());
        assert_eq!(
            state.audit_counters.lock().unwrap().by_reason_code
                [COMMAND_RUNTIME_DISABLED_REASON]
                .denied,
            1
        );
    }

    #[cfg(feature = "command-execution")]
    #[tokio::test]
    async fn raw_command_override_fields_are_rejected_before_spawn() {
        let safe_root = tempfile::tempdir().unwrap();
        let marker = safe_root.path().join("must-not-exist");
        let script = format!("touch '{}'", marker.display());
        let (_program_root, client) = test_command_client(safe_root.path(), &script);
        let state = McpTransportState::with_command_execution_client(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            FileSystemTools::new(vec![safe_root.path().to_path_buf()]),
            true,
            client,
        );

        for arguments in [
            json!({"profile": "server_version", "command": "sh -c id"}),
            json!({"profile": "server_version", "argv": ["--help"]}),
            json!({"profile": "server_version", "workingDirectory": "/"}),
            json!({"profile": "server_version", "environment": {"TOKEN": "secret"}}),
            json!({"profile": "server_version", "timeout": 999}),
            json!({"profile": "server_version", "stdoutLimit": 999999}),
        ] {
            let response = handle_run_command_profile_call(
                Some(json!("command-invalid")),
                Some(arguments),
                &state,
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
        assert!(!marker.exists());
        let counters = state.audit_counters.lock().unwrap().clone();
        assert_eq!(
            counters.by_reason_code[COMMAND_INVALID_ARGUMENTS_REASON].denied,
            6
        );
    }
}
