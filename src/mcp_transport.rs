use std::{
    convert::Infallible,
    net::IpAddr,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    body::{to_bytes, Body, Bytes},
    extract::{DefaultBodyLimit, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::any,
    Json, Router,
};
use futures_util::stream;
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

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
    auth::{require_mcp_auth, McpAuthPolicy},
    command_policy::{
        command_profile_ids, COMMAND_EXECUTION_GATE, COMMAND_INVALID_ARGUMENTS_REASON,
        COMMAND_MISSING_ARGUMENTS_REASON, RUN_COMMAND_PROFILE_TOOL,
    },
    copy_file_grant::{CopyFileGrantAuthority, CopyFileGrantError, COPY_FILE_GRANT_TTL_SECONDS},
    create_directory_grant::{
        CreateDirectoryGrantAuthority, CreateDirectoryGrantError,
        CREATE_DIRECTORY_GRANT_TTL_SECONDS,
    },
    error::{AppError, INVALID_BINARY_RANGE_PUBLIC_MESSAGE, INVALID_TEXT_RANGE_PUBLIC_MESSAGE},
    json_rpc::{parse_incoming_message, IncomingJsonRpcMessage, JsonRpcEnvelopeError},
    mcp_session::{
        McpSessionStore, SessionPhase, SessionStoreError, SseReplayError, SseReplayEvent,
        MAX_MCP_SSE_EVENTS_PER_STREAM, MAX_MCP_SSE_EVENT_DATA_BYTES,
        MAX_MCP_SSE_REPLAY_BYTES_PER_SESSION, MAX_MCP_SSE_STREAMS_PER_SESSION,
        SSE_RETRY_MILLISECONDS,
    },
    platform_info::collect_platform_info,
    request_grant_capability::{MAX_REQUEST_GRANT_HEADER_BYTES, REQUEST_GRANT_HEADER},
    request_limits::{enforce_mcp_request_limits, McpRequestLimits},
    service_status::{
        collect_project_service_status, ProjectServiceStatusError, PROJECT_SERVICE_ALLOWLIST,
    },
    tools::{
        AuthorizedCopyFileError, AuthorizedCreateDirectoryError, AuthorizedWriteFileError,
        FileSystemTools, FindPathFilter, PreparedCopyFileMutation, PreparedCreateDirectoryMutation,
        COPY_FILE_MODE, MAX_BINARY_RANGE_BASE64_BYTES, MAX_BINARY_RANGE_BYTES,
        MAX_BINARY_RANGE_FILE_BYTES, MAX_BINARY_RANGE_RESPONSE_BYTES, MAX_BINARY_READ_BASE64_BYTES,
        MAX_BINARY_READ_BYTES, MAX_BINARY_READ_RESPONSE_BYTES, MAX_COPY_FILE_BYTES,
        MAX_COPY_FILE_RESPONSE_BYTES, MAX_CREATE_DIRECTORY_RESPONSE_BYTES, MAX_FIND_DEPTH,
        MAX_FIND_ENTRIES, MAX_FIND_MATCHES, MAX_FIND_QUERY_BYTES, MAX_FIND_RESPONSE_BYTES,
        MAX_HASH_FILE_BYTES, MAX_HASH_FILE_RESPONSE_BYTES, MAX_LIST_RESPONSE_BYTES,
        MAX_PATH_METADATA_RESPONSE_BYTES, MAX_READ_RESPONSE_BYTES, MAX_SEARCH_DEPTH,
        MAX_SEARCH_QUERY_BYTES, MAX_SEARCH_RESPONSE_BYTES, MAX_TEXT_RANGE_BYTES,
        MAX_TEXT_RANGE_ESCAPED_BYTES, MAX_TEXT_RANGE_FILE_BYTES, MAX_TEXT_RANGE_RESPONSE_BYTES,
        MAX_WRITE_FILE_RESPONSE_BYTES, MIN_FIND_DEPTH, MIN_SEARCH_DEPTH, MIN_TEXT_RANGE_BYTES,
    },
    transport_security::TransportSecurityPolicy,
    write_file_grant::{
        WriteFileGrantAuthority, WriteFileGrantError, WRITE_FILE_GRANT_TTL_SECONDS,
    },
    write_policy::{WriteMode, WritePolicy, DEFAULT_MAX_WRITE_BYTES},
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
pub const MCP_LAST_EVENT_ID_HEADER: &str = "last-event-id";
pub const MCP_POST_ACCEPT: &str = "application/json, text/event-stream";
pub const MAX_MCP_LAST_EVENT_ID_BYTES: usize = 64;
/// Maximum canonical serialized byte length of one non-null JSON-RPC request id.
pub const MAX_MCP_JSON_RPC_ID_BYTES: usize = 1_048_576;
pub const MCP_SSE_RETRY_MILLISECONDS: u64 = SSE_RETRY_MILLISECONDS;
/// Fixed service-wide ceiling shared by all live filesystem mutation workers.
pub const MAX_CONCURRENT_FILESYSTEM_MUTATION_WORKERS: usize = 1;

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
const FIND_PATHS_TOOL: &str = "find_paths";
const HASH_FILE_TOOL: &str = "hash_file";
const LIST_DIRECTORY_TOOL: &str = "list_directory";
const PATH_METADATA_TOOL: &str = "path_metadata";
const READ_BINARY_FILE_TOOL: &str = "read_binary_file";
const READ_BINARY_RANGE_TOOL: &str = "read_binary_range";
const READ_FILE_TOOL: &str = "read_file";
const READ_TEXT_RANGE_TOOL: &str = "read_text_range";
const SEARCH_TEXT_TOOL: &str = "search_text";
const WRITE_FILE_TOOL: &str = "write_file";
const BASE_AVAILABLE_TOOLS: [&str; 16] = [
    RUNTIME_STATUS_TOOL,
    PLATFORM_INFO_TOOL,
    ANDROID_STATUS_TOOL,
    PROJECT_SERVICE_STATUS_TOOL,
    CREATE_DIRECTORY_TOOL,
    COPY_FILE_TOOL,
    FIND_PATHS_TOOL,
    HASH_FILE_TOOL,
    LIST_DIRECTORY_TOOL,
    PATH_METADATA_TOOL,
    READ_BINARY_FILE_TOOL,
    READ_BINARY_RANGE_TOOL,
    READ_FILE_TOOL,
    READ_TEXT_RANGE_TOOL,
    SEARCH_TEXT_TOOL,
    WRITE_FILE_TOOL,
];
const MIN_LIST_DIRECTORY_DEPTH: u32 = 1;
const MAX_LIST_DIRECTORY_DEPTH: u32 = 5;
const MAX_FIND_STRUCTURED_CONTENT_BYTES: usize = MAX_FIND_RESPONSE_BYTES - 1_024;

const fn maximum_response_contract(contracts: &[usize]) -> usize {
    let mut maximum = 0;
    let mut index = 0;
    while index < contracts.len() {
        if contracts[index] > maximum {
            maximum = contracts[index];
        }
        index += 1;
    }
    maximum
}

/// Largest complete JSON response contract among every filesystem tool that
/// can enter SSE conversion. Keeping the collector derived from the registry's
/// explicit budgets prevents a newly larger valid response from becoming an
/// internal error before it can take the required JSON fallback path.
const MAX_MCP_COLLECTED_JSON_RESPONSE_BYTES: usize = maximum_response_contract(&[
    MAX_LIST_RESPONSE_BYTES,
    MAX_READ_RESPONSE_BYTES,
    MAX_BINARY_READ_RESPONSE_BYTES,
    MAX_BINARY_RANGE_RESPONSE_BYTES,
    MAX_TEXT_RANGE_RESPONSE_BYTES,
    MAX_HASH_FILE_RESPONSE_BYTES,
    MAX_PATH_METADATA_RESPONSE_BYTES,
    MAX_CREATE_DIRECTORY_RESPONSE_BYTES,
    MAX_COPY_FILE_RESPONSE_BYTES,
    MAX_WRITE_FILE_RESPONSE_BYTES,
    MAX_FIND_RESPONSE_BYTES,
    MAX_SEARCH_RESPONSE_BYTES,
]);

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
const FILESYSTEM_BINARY_RANGE_ALLOWED: &str = "safe_root_binary_range_read";
const FILESYSTEM_BINARY_RANGE_NOT_FOUND: &str = "filesystem_binary_range_target_not_found";
const FILESYSTEM_BINARY_RANGE_UNSUPPORTED: &str = "filesystem_binary_range_type_unsupported";
const FILESYSTEM_BINARY_RANGE_INVALID: &str = "filesystem_binary_range_invalid";
const FILESYSTEM_BINARY_RANGE_TOO_LARGE: &str = "filesystem_binary_range_file_too_large";
const FILESYSTEM_BINARY_RANGE_CHANGED: &str = "filesystem_binary_range_changed_during_read";
const FILESYSTEM_BINARY_RANGE_FAILED: &str = "filesystem_binary_range_failed";
const FILESYSTEM_TEXT_RANGE_ALLOWED: &str = "safe_root_text_range_read";
const FILESYSTEM_TEXT_RANGE_NOT_FOUND: &str = "filesystem_text_range_target_not_found";
const FILESYSTEM_TEXT_RANGE_UNSUPPORTED: &str = "filesystem_text_range_type_unsupported";
const FILESYSTEM_TEXT_RANGE_INVALID: &str = "filesystem_text_range_invalid";
const FILESYSTEM_TEXT_RANGE_TOO_LARGE: &str = "filesystem_text_range_file_too_large";
const FILESYSTEM_TEXT_RANGE_ENCODING_INVALID: &str = "filesystem_text_range_encoding_invalid";
const FILESYSTEM_TEXT_RANGE_CHANGED: &str = "filesystem_text_range_changed_during_read";
const FILESYSTEM_TEXT_RANGE_FAILED: &str = "filesystem_text_range_failed";
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
const FILESYSTEM_COPY_SOURCE_CHANGED: &str = "filesystem_copy_source_changed";
const FILESYSTEM_COPY_DESTINATION_CHANGED: &str = "filesystem_copy_destination_changed";
const FILESYSTEM_COPY_MUTATION_DISABLED: &str = "copy_file_mutation_disabled";
const FILESYSTEM_COPY_QUARANTINE_FULL: &str = "filesystem_copy_quarantine_capacity_exceeded";
const FILESYSTEM_COPY_QUARANTINE_BUSY: &str = "filesystem_copy_quarantine_busy";
const FILESYSTEM_COPY_FAILED: &str = "filesystem_copy_failed";
const FILESYSTEM_FIND_ALLOWED: &str = "safe_root_paths_found";
const FILESYSTEM_FIND_INVALID_QUERY: &str = "find_query_invalid";
const FILESYSTEM_FIND_FAILED: &str = "filesystem_find_failed";
const FILESYSTEM_READ_FAILED: &str = "filesystem_read_failed";
const FILESYSTEM_DRY_RUN_ALLOWED: &str = "dry_run_preview";
const FILESYSTEM_WRITE_ALLOWED: &str = "explicit_write_allowed";
const FILESYSTEM_WRITE_TOO_LARGE: &str = "write_size_limit_exceeded";
const FILESYSTEM_WRITE_MUTATION_DISABLED: &str = "write_file_mutation_disabled";
const FILESYSTEM_WRITE_TARGET_CHANGED: &str = "filesystem_write_target_changed";
const FILESYSTEM_WRITE_TARGET_NOT_FOUND: &str = "filesystem_write_target_not_found";
const FILESYSTEM_WRITE_TARGET_UNSUPPORTED: &str = "filesystem_write_target_type_unsupported";
const FILESYSTEM_WRITE_QUARANTINE_FULL: &str = "write_quarantine_capacity_exceeded";
const FILESYSTEM_WRITE_QUARANTINE_BUSY: &str = "write_quarantine_busy";
const FILESYSTEM_WRITE_FAILED: &str = "filesystem_write_failed";
const FILESYSTEM_MUTATION_WORKER_CAPACITY_EXCEEDED: &str =
    "filesystem_mutation_worker_capacity_exceeded";
const FILESYSTEM_MUTATION_REQUEST_CANCELLED: &str = "filesystem_mutation_request_cancelled";

const COMMAND_EXECUTION_ERROR: &str = "command_profile_execution_failed";

const TOOL_CALL_PARAMS_INVALID: &str = "tools/call params do not match the required schema.";
const TOOL_ARGUMENTS_INVALID: &str = "Tool arguments do not match the advertised input schema.";

type SharedAuditCounters = Arc<Mutex<AuditCounters>>;

/// Owns the single aggregate audit decision for an authorized directory worker.
///
/// Moving this guard into the blocking worker ensures that an HTTP timeout or
/// disconnected waiter cannot suppress the outcome of a mutation that has
/// already started.
struct CreateDirectoryMutationAuditGuard {
    counters: SharedAuditCounters,
    recorded: bool,
}

impl CreateDirectoryMutationAuditGuard {
    fn new(counters: SharedAuditCounters) -> Self {
        Self {
            counters,
            recorded: false,
        }
    }

    fn finish<T>(mut self, outcome: &Result<T, AuthorizedCreateDirectoryError>) {
        match outcome {
            Ok(_) => record_filesystem_allowed(
                &self.counters,
                CREATE_DIRECTORY_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::Mutating,
                FILESYSTEM_CREATE_ALLOWED,
            ),
            Err(AuthorizedCreateDirectoryError::Authorization(error)) => record_filesystem_denied(
                &self.counters,
                CREATE_DIRECTORY_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::Mutating,
                error.reason_code(),
            ),
            Err(AuthorizedCreateDirectoryError::Filesystem(error)) => record_filesystem_denied(
                &self.counters,
                CREATE_DIRECTORY_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::Mutating,
                create_directory_filesystem_reason(error),
            ),
            Err(AuthorizedCreateDirectoryError::Cancelled) => record_filesystem_denied(
                &self.counters,
                CREATE_DIRECTORY_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::Mutating,
                FILESYSTEM_MUTATION_REQUEST_CANCELLED,
            ),
        }
        self.recorded = true;
    }

    fn cancelled(mut self) {
        record_filesystem_denied(
            &self.counters,
            CREATE_DIRECTORY_TOOL,
            FILESYSTEM_WRITE_GATE,
            AuditMode::Mutating,
            FILESYSTEM_MUTATION_REQUEST_CANCELLED,
        );
        self.recorded = true;
    }
}

impl Drop for CreateDirectoryMutationAuditGuard {
    fn drop(&mut self) {
        if !self.recorded {
            record_filesystem_denied(
                &self.counters,
                CREATE_DIRECTORY_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::Mutating,
                FILESYSTEM_CREATE_FAILED,
            );
            self.recorded = true;
        }
    }
}

/// Owns the single aggregate audit decision for an authorized copy worker.
///
/// Only stable reason labels cross this boundary. Source and destination paths,
/// file identity, content digests, and grant material remain confined to the
/// detached blocking worker and are never retained by audit state.
struct CopyFileMutationAuditGuard {
    counters: SharedAuditCounters,
    recorded: bool,
}

impl CopyFileMutationAuditGuard {
    fn new(counters: SharedAuditCounters) -> Self {
        Self {
            counters,
            recorded: false,
        }
    }

    fn finish<T>(mut self, outcome: &Result<T, AuthorizedCopyFileError>) {
        match outcome {
            Ok(_) => record_filesystem_allowed(
                &self.counters,
                COPY_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::Mutating,
                FILESYSTEM_COPY_ALLOWED,
            ),
            Err(AuthorizedCopyFileError::Authorization(error)) => record_filesystem_denied(
                &self.counters,
                COPY_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::Mutating,
                error.reason_code(),
            ),
            Err(AuthorizedCopyFileError::Filesystem(error)) => record_filesystem_denied(
                &self.counters,
                COPY_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::Mutating,
                copy_file_filesystem_reason(error),
            ),
            Err(AuthorizedCopyFileError::Cancelled) => record_filesystem_denied(
                &self.counters,
                COPY_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::Mutating,
                FILESYSTEM_MUTATION_REQUEST_CANCELLED,
            ),
        }
        self.recorded = true;
    }

    fn cancelled(mut self) {
        record_filesystem_denied(
            &self.counters,
            COPY_FILE_TOOL,
            FILESYSTEM_WRITE_GATE,
            AuditMode::Mutating,
            FILESYSTEM_MUTATION_REQUEST_CANCELLED,
        );
        self.recorded = true;
    }
}

impl Drop for CopyFileMutationAuditGuard {
    fn drop(&mut self) {
        if !self.recorded {
            record_filesystem_denied(
                &self.counters,
                COPY_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::Mutating,
                FILESYSTEM_COPY_FAILED,
            );
            self.recorded = true;
        }
    }
}

/// Owns the single aggregate audit decision for an authorized file-write worker.
///
/// The guard is moved into the blocking worker with the prepared mutation. This
/// makes the audit decision independent from the lifetime of the HTTP response
/// future: dropping a waiter cannot abandon or double-record the worker outcome.
/// Only stable labels are recorded; the grant, path, content, and target identity
/// never enter this object.
struct WriteFileMutationAuditGuard {
    counters: SharedAuditCounters,
    recorded: bool,
}

impl WriteFileMutationAuditGuard {
    fn new(counters: SharedAuditCounters) -> Self {
        Self {
            counters,
            recorded: false,
        }
    }

    fn finish<T>(mut self, outcome: &Result<T, AuthorizedWriteFileError>) {
        match outcome {
            Ok(_) => record_filesystem_allowed(
                &self.counters,
                WRITE_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::Mutating,
                FILESYSTEM_WRITE_ALLOWED,
            ),
            Err(AuthorizedWriteFileError::Authorization(error)) => record_filesystem_denied(
                &self.counters,
                WRITE_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::Mutating,
                error.reason_code(),
            ),
            Err(AuthorizedWriteFileError::Filesystem(error)) => record_filesystem_denied(
                &self.counters,
                WRITE_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::Mutating,
                write_file_filesystem_reason(error),
            ),
            Err(AuthorizedWriteFileError::Cancelled) => record_filesystem_denied(
                &self.counters,
                WRITE_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::Mutating,
                FILESYSTEM_MUTATION_REQUEST_CANCELLED,
            ),
        }
        self.recorded = true;
    }

    fn cancelled(mut self) {
        record_filesystem_denied(
            &self.counters,
            WRITE_FILE_TOOL,
            FILESYSTEM_WRITE_GATE,
            AuditMode::Mutating,
            FILESYSTEM_MUTATION_REQUEST_CANCELLED,
        );
        self.recorded = true;
    }
}

impl Drop for WriteFileMutationAuditGuard {
    fn drop(&mut self) {
        if !self.recorded {
            record_filesystem_denied(
                &self.counters,
                WRITE_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::Mutating,
                FILESYSTEM_WRITE_FAILED,
            );
            self.recorded = true;
        }
    }
}

#[cfg(feature = "android-volume-control")]
struct AndroidVolumeMutationAuditGuard {
    counters: SharedAuditCounters,
    recorded: bool,
}

#[cfg(feature = "android-volume-control")]
impl AndroidVolumeMutationAuditGuard {
    fn new(counters: SharedAuditCounters) -> Self {
        Self {
            counters,
            recorded: false,
        }
    }

    fn finish<T>(mut self, outcome: &Result<T, AndroidVolumeControlError>) {
        match outcome {
            Ok(_) => record_volume_control_decision(
                &self.counters,
                AuditMode::Mutating,
                AuditDecision::Allowed,
                ANDROID_VOLUME_CONTROL_MUTATION_ALLOWED,
            ),
            Err(error) => record_volume_control_decision(
                &self.counters,
                AuditMode::Mutating,
                AuditDecision::Denied,
                (*error).reason_code(),
            ),
        }
        self.recorded = true;
    }
}

#[cfg(feature = "android-volume-control")]
impl Drop for AndroidVolumeMutationAuditGuard {
    fn drop(&mut self) {
        if !self.recorded {
            record_volume_control_decision(
                &self.counters,
                AuditMode::Mutating,
                AuditDecision::Denied,
                AndroidVolumeControlError::WorkerFailed.reason_code(),
            );
            self.recorded = true;
        }
    }
}

const MUTATION_COMMIT_PENDING: u8 = 0;
const MUTATION_COMMIT_CANCELLED: u8 = 1;
const MUTATION_COMMIT_WORKER_OWNED: u8 = 2;

/// Reusable two-party commit guard for blocking filesystem mutations.
///
/// The async waiter changes `pending` to `cancelled` when it is dropped. After
/// descriptor preparation, the worker atomically changes `pending` to
/// `worker-owned` immediately before grant consumption. Exactly one transition
/// can win: cancellation therefore consumes no grant and mutates nothing, while
/// a worker-owned commit continues independently of the HTTP future.
struct FilesystemMutationWaiterGuard {
    state: Arc<AtomicU8>,
    armed: bool,
}

struct FilesystemMutationWorkerCommitGuard {
    state: Arc<AtomicU8>,
}

fn filesystem_mutation_commit_guards() -> (
    FilesystemMutationWaiterGuard,
    FilesystemMutationWorkerCommitGuard,
) {
    let state = Arc::new(AtomicU8::new(MUTATION_COMMIT_PENDING));
    (
        FilesystemMutationWaiterGuard {
            state: Arc::clone(&state),
            armed: true,
        },
        FilesystemMutationWorkerCommitGuard { state },
    )
}

impl FilesystemMutationWaiterGuard {
    fn complete(mut self) {
        self.armed = false;
    }
}

impl Drop for FilesystemMutationWaiterGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.state.compare_exchange(
                MUTATION_COMMIT_PENDING,
                MUTATION_COMMIT_CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
    }
}

impl FilesystemMutationWorkerCommitGuard {
    fn claim(self) -> bool {
        self.state
            .compare_exchange(
                MUTATION_COMMIT_PENDING,
                MUTATION_COMMIT_WORKER_OWNED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
}

enum FilesystemMutationWorkerOutcome<T, E> {
    Completed(Result<T, E>),
    Cancelled,
}

/// Service-owned, shared, fail-fast admission boundary for blocking filesystem
/// mutation workers. There is deliberately no waiter queue: a caller either
/// obtains one of the fixed permits immediately or is rejected before grant
/// consumption and mutation.
#[derive(Clone)]
struct FilesystemMutationWorkerCapacity {
    semaphore: Arc<Semaphore>,
}

impl FilesystemMutationWorkerCapacity {
    fn new(max_workers: usize) -> Self {
        debug_assert!(max_workers > 0);
        Self {
            semaphore: Arc::new(Semaphore::new(max_workers)),
        }
    }

    fn try_acquire(&self) -> Option<OwnedSemaphorePermit> {
        self.semaphore.clone().try_acquire_owned().ok()
    }
}

impl Default for FilesystemMutationWorkerCapacity {
    fn default() -> Self {
        Self::new(MAX_CONCURRENT_FILESYSTEM_MUTATION_WORKERS)
    }
}

fn spawn_filesystem_mutation_worker<T, F>(
    permit: OwnedSemaphorePermit,
    worker: F,
) -> tokio::task::JoinHandle<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        // This permit intentionally remains owned through the complete blocking
        // closure even when the async waiter times out or is cancelled.
        let _permit = permit;
        worker()
    })
}

fn run_create_directory_mutation_worker(
    file_tools: FileSystemTools,
    path: String,
    authority: CreateDirectoryGrantAuthority,
    capability_grant: Option<String>,
    session_id: String,
    commit: FilesystemMutationWorkerCommitGuard,
    audit: CreateDirectoryMutationAuditGuard,
) -> FilesystemMutationWorkerOutcome<
    crate::tools::CreateDirectoryResult,
    AuthorizedCreateDirectoryError,
> {
    let prepared = match file_tools.prepare_create_directory_mutation_blocking(path) {
        Ok(prepared) => prepared,
        Err(error) => {
            let outcome = Err(AuthorizedCreateDirectoryError::Filesystem(error));
            audit.finish(&outcome);
            return FilesystemMutationWorkerOutcome::Completed(outcome);
        }
    };
    run_prepared_create_directory_mutation(
        prepared,
        authority,
        capability_grant,
        session_id,
        commit,
        audit,
    )
}

fn run_prepared_create_directory_mutation(
    prepared: PreparedCreateDirectoryMutation,
    authority: CreateDirectoryGrantAuthority,
    capability_grant: Option<String>,
    session_id: String,
    commit: FilesystemMutationWorkerCommitGuard,
    audit: CreateDirectoryMutationAuditGuard,
) -> FilesystemMutationWorkerOutcome<
    crate::tools::CreateDirectoryResult,
    AuthorizedCreateDirectoryError,
> {
    let outcome = prepared.execute_authorized_with_commit(|target| {
        if !commit.claim() {
            return Err(AuthorizedCreateDirectoryError::Cancelled);
        }
        authority
            .consume_at(
                capability_grant.as_deref(),
                &session_id,
                target,
                current_unix_seconds(),
            )
            .map_err(AuthorizedCreateDirectoryError::Authorization)
    });
    if matches!(outcome, Err(AuthorizedCreateDirectoryError::Cancelled)) {
        audit.cancelled();
        return FilesystemMutationWorkerOutcome::Cancelled;
    }
    audit.finish(&outcome);
    FilesystemMutationWorkerOutcome::Completed(outcome)
}

struct CopyFileMutationWorker {
    file_tools: FileSystemTools,
    source_path: String,
    destination_path: String,
    authority: CopyFileGrantAuthority,
    capability_grant: Option<String>,
    session_id: String,
    commit: FilesystemMutationWorkerCommitGuard,
    audit: CopyFileMutationAuditGuard,
}

fn run_copy_file_mutation_worker(
    worker: CopyFileMutationWorker,
) -> FilesystemMutationWorkerOutcome<crate::tools::CopyFileResult, AuthorizedCopyFileError> {
    run_copy_file_mutation_worker_inner(worker, None::<fn()>)
}

#[cfg(test)]
fn run_copy_file_mutation_worker_with_lock_contention_hook(
    worker: CopyFileMutationWorker,
    on_lock_contention: impl FnOnce(),
) -> FilesystemMutationWorkerOutcome<crate::tools::CopyFileResult, AuthorizedCopyFileError> {
    run_copy_file_mutation_worker_inner(worker, Some(on_lock_contention))
}

fn run_copy_file_mutation_worker_inner(
    worker: CopyFileMutationWorker,
    on_lock_contention: Option<impl FnOnce()>,
) -> FilesystemMutationWorkerOutcome<crate::tools::CopyFileResult, AuthorizedCopyFileError> {
    let CopyFileMutationWorker {
        file_tools,
        source_path,
        destination_path,
        authority,
        capability_grant,
        session_id,
        commit,
        audit,
    } = worker;
    let prepared =
        match file_tools.prepare_copy_file_mutation_blocking(source_path, destination_path) {
            Ok(prepared) => prepared,
            Err(error) => {
                let outcome = Err(AuthorizedCopyFileError::Filesystem(error));
                audit.finish(&outcome);
                return FilesystemMutationWorkerOutcome::Completed(outcome);
            }
        };
    run_prepared_copy_file_mutation(
        prepared,
        authority,
        capability_grant,
        session_id,
        commit,
        audit,
        on_lock_contention,
    )
}

fn run_prepared_copy_file_mutation(
    prepared: PreparedCopyFileMutation,
    authority: CopyFileGrantAuthority,
    capability_grant: Option<String>,
    session_id: String,
    commit: FilesystemMutationWorkerCommitGuard,
    audit: CopyFileMutationAuditGuard,
    on_lock_contention: Option<impl FnOnce()>,
) -> FilesystemMutationWorkerOutcome<crate::tools::CopyFileResult, AuthorizedCopyFileError> {
    let authorize_and_commit = |target: &crate::copy_file_grant::CopyFileGrantTarget| {
        if !commit.claim() {
            return Err(AuthorizedCopyFileError::Cancelled);
        }
        authority
            .consume(capability_grant.as_deref(), &session_id, target)
            .map_err(AuthorizedCopyFileError::Authorization)
    };
    let outcome = match on_lock_contention {
        Some(on_lock_contention) => prepared
            .execute_authorized_with_commit_and_lock_contention_hook(
                authorize_and_commit,
                on_lock_contention,
            ),
        None => prepared.execute_authorized_with_commit(authorize_and_commit),
    };
    if matches!(outcome, Err(AuthorizedCopyFileError::Cancelled)) {
        audit.cancelled();
        return FilesystemMutationWorkerOutcome::Cancelled;
    }
    audit.finish(&outcome);
    FilesystemMutationWorkerOutcome::Completed(outcome)
}

struct WriteFileMutationWorker {
    file_tools: FileSystemTools,
    path: String,
    content: String,
    authority: WriteFileGrantAuthority,
    capability_grant: Option<String>,
    session_id: String,
    commit: FilesystemMutationWorkerCommitGuard,
    audit: WriteFileMutationAuditGuard,
}

fn run_write_file_mutation_worker(
    worker: WriteFileMutationWorker,
) -> FilesystemMutationWorkerOutcome<crate::tools::WriteFileResult, AuthorizedWriteFileError> {
    run_write_file_mutation_worker_inner(worker, None::<fn()>)
}

#[cfg(test)]
fn run_write_file_mutation_worker_with_lock_contention_hook(
    worker: WriteFileMutationWorker,
    on_lock_contention: impl FnOnce(),
) -> FilesystemMutationWorkerOutcome<crate::tools::WriteFileResult, AuthorizedWriteFileError> {
    run_write_file_mutation_worker_inner(worker, Some(on_lock_contention))
}

fn run_write_file_mutation_worker_inner(
    worker: WriteFileMutationWorker,
    on_lock_contention: Option<impl FnOnce()>,
) -> FilesystemMutationWorkerOutcome<crate::tools::WriteFileResult, AuthorizedWriteFileError> {
    let WriteFileMutationWorker {
        file_tools,
        path,
        content,
        authority,
        capability_grant,
        session_id,
        commit,
        audit,
    } = worker;
    let prepared = match file_tools.prepare_write_file_mutation_blocking(path, content) {
        Ok(prepared) => prepared,
        Err(error) => {
            let outcome = Err(AuthorizedWriteFileError::Filesystem(error));
            audit.finish(&outcome);
            return FilesystemMutationWorkerOutcome::Completed(outcome);
        }
    };
    let authorize_and_commit = |target: &crate::write_file_grant::WriteFileGrantTarget| {
        if !commit.claim() {
            return Err(AuthorizedWriteFileError::Cancelled);
        }
        authority
            .consume(capability_grant.as_deref(), &session_id, target)
            .map_err(AuthorizedWriteFileError::Authorization)
    };
    let outcome = match on_lock_contention {
        Some(on_lock_contention) => prepared
            .execute_authorized_with_commit_and_lock_contention_hook(
                authorize_and_commit,
                on_lock_contention,
            ),
        None => prepared.execute_authorized_with_commit(authorize_and_commit),
    };
    if matches!(outcome, Err(AuthorizedWriteFileError::Cancelled)) {
        audit.cancelled();
        return FilesystemMutationWorkerOutcome::Cancelled;
    }
    audit.finish(&outcome);
    FilesystemMutationWorkerOutcome::Completed(outcome)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct McpTransportOptions {
    sse_enabled: bool,
}

impl McpTransportOptions {
    pub const fn with_sse_enabled(mut self, enabled: bool) -> Self {
        self.sse_enabled = enabled;
        self
    }
}

struct FilesystemMutationAuthorities {
    create_directory: Option<CreateDirectoryGrantAuthority>,
    copy_file: Option<CopyFileGrantAuthority>,
    write_file: Option<WriteFileGrantAuthority>,
}

impl FilesystemMutationAuthorities {
    fn new(
        create_directory: Option<CreateDirectoryGrantAuthority>,
        write_file: Option<WriteFileGrantAuthority>,
    ) -> Self {
        Self {
            create_directory,
            copy_file: None,
            write_file,
        }
    }

    fn with_copy_file(mut self, copy_file: Option<CopyFileGrantAuthority>) -> Self {
        self.copy_file = copy_file;
        self
    }
}

/// Mandatory authentication and resource-limit boundary for public MCP routers.
///
/// Construction requires the host that the embedding application intends to
/// bind. An unauthenticated development policy is accepted only for an exact
/// loopback declaration, and every request under that policy additionally
/// requires Axum `ConnectInfo<SocketAddr>` proving the actual network peer is
/// loopback. Missing peer metadata and non-loopback peers fail closed. Embedders
/// must therefore serve the returned router with
/// `into_make_service_with_connect_info::<SocketAddr>()`; the declaration is
/// defense in depth, not the request-time trust boundary. The fields are private
/// to ensure every public router is protected by bearer authentication (or this
/// verified loopback-only development posture), request limits, and Axum's
/// streaming body limit in the required order.
///
/// Raw transport constructors are deliberately crate-private:
///
/// ```compile_fail
/// let _ = termux_mcp_server::mcp_transport::router;
/// ```
#[derive(Clone, Debug)]
pub struct McpRouterProtection {
    listener_host: String,
    auth_policy: McpAuthPolicy,
    request_limits: McpRequestLimits,
}

impl McpRouterProtection {
    /// Declare the listener host and construct a complete public route boundary.
    pub fn new(
        listener_host: impl AsRef<str>,
        auth_policy: McpAuthPolicy,
        request_limits: McpRequestLimits,
    ) -> anyhow::Result<Self> {
        let listener_host = listener_host.as_ref();
        validate_declared_listener_host(listener_host)?;

        if matches!(&auth_policy, McpAuthPolicy::UnauthenticatedLocalhostOnly)
            && !is_loopback_listener_host(listener_host)
        {
            anyhow::bail!(
                "unauthenticated MCP router protection requires a declared loopback listener host"
            );
        }

        Ok(Self {
            listener_host: listener_host.to_owned(),
            auth_policy,
            request_limits,
        })
    }

    /// Return the exact listener host declared by the embedding application.
    pub fn listener_host(&self) -> &str {
        &self.listener_host
    }
}

fn validate_declared_listener_host(listener_host: &str) -> anyhow::Result<()> {
    if listener_host.is_empty()
        || listener_host != listener_host.trim()
        || listener_host
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        anyhow::bail!("MCP router protection requires a valid declared listener host");
    }

    Ok(())
}

fn is_loopback_listener_host(listener_host: &str) -> bool {
    listener_host.eq_ignore_ascii_case("localhost")
        || listener_host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// Independently optional authorities for high-impact MCP capabilities.
///
/// Grouping these authorities keeps transport construction explicit while
/// preventing feature additions from growing an error-prone positional API.
#[cfg(feature = "android-volume-control")]
pub struct McpCapabilityAuthorities {
    create_directory: Option<CreateDirectoryGrantAuthority>,
    copy_file: Option<CopyFileGrantAuthority>,
    write_file: Option<WriteFileGrantAuthority>,
    android_volume_control: Option<AndroidVolumeGrantAuthority>,
}

#[cfg(feature = "android-volume-control")]
impl McpCapabilityAuthorities {
    pub fn new(
        create_directory: Option<CreateDirectoryGrantAuthority>,
        write_file: Option<WriteFileGrantAuthority>,
        android_volume_control: Option<AndroidVolumeGrantAuthority>,
    ) -> Self {
        Self {
            create_directory,
            copy_file: None,
            write_file,
            android_volume_control,
        }
    }

    /// Add the independently gated `copy_file` mutation authority.
    pub fn with_copy_file_authority(mut self, authority: CopyFileGrantAuthority) -> Self {
        self.copy_file = Some(authority);
        self
    }
}

#[derive(Clone)]
struct McpTransportState {
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    audit_counters: SharedAuditCounters,
    sessions: McpSessionStore,
    mutation_worker_capacity: FilesystemMutationWorkerCapacity,
    sse_enabled: bool,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    android_volume_control_enabled: bool,
    command_execution_enabled: bool,
    create_directory_authority: Option<CreateDirectoryGrantAuthority>,
    copy_file_authority: Option<CopyFileGrantAuthority>,
    write_file_authority: Option<WriteFileGrantAuthority>,
    #[cfg(feature = "android-battery-status")]
    android_battery_client: AndroidBatteryClient,
    #[cfg(feature = "android-volume-status")]
    android_volume_client: AndroidVolumeClient,
    #[cfg(feature = "android-volume-control")]
    android_volume_control_authority: Option<AndroidVolumeGrantAuthority>,
    #[cfg(feature = "android-volume-control")]
    android_volume_control_client: AndroidVolumeControlClient,
    #[cfg(feature = "command-execution")]
    command_execution_client: Option<CommandExecutionClient>,
}

impl McpTransportState {
    fn new(
        security_policy: TransportSecurityPolicy,
        file_tools: FileSystemTools,
        android_battery_status_enabled: bool,
        android_volume_status_enabled: bool,
        command_execution_enabled: bool,
        create_directory_authority: Option<CreateDirectoryGrantAuthority>,
        write_file_authority: Option<WriteFileGrantAuthority>,
    ) -> Self {
        #[cfg(feature = "command-execution")]
        let command_execution_client = command_execution_enabled
            .then(|| file_tools.safe_roots().first().cloned())
            .flatten()
            .and_then(|safe_root| CommandExecutionClient::current_server(safe_root).ok());
        #[cfg(feature = "command-execution")]
        let command_execution_enabled =
            command_execution_enabled && command_execution_client.is_some();

        Self {
            security_policy,
            file_tools,
            audit_counters: Arc::new(Mutex::new(AuditCounters::default())),
            sessions: McpSessionStore::new(),
            mutation_worker_capacity: FilesystemMutationWorkerCapacity::default(),
            sse_enabled: false,
            android_battery_status_enabled: android_battery_status_enabled
                && cfg!(feature = "android-battery-status"),
            android_volume_status_enabled: android_volume_status_enabled
                && cfg!(feature = "android-volume-status"),
            android_volume_control_enabled: false,
            command_execution_enabled: command_execution_enabled
                && cfg!(feature = "command-execution"),
            create_directory_authority,
            copy_file_authority: None,
            write_file_authority,
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

    fn with_options(mut self, options: McpTransportOptions) -> Self {
        self.sse_enabled = options.sse_enabled;
        self
    }

    fn with_copy_file_authority(mut self, authority: Option<CopyFileGrantAuthority>) -> Self {
        self.copy_file_authority = authority;
        self
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
        Self {
            security_policy,
            file_tools,
            audit_counters: Arc::new(Mutex::new(AuditCounters::default())),
            sessions: McpSessionStore::new(),
            mutation_worker_capacity: FilesystemMutationWorkerCapacity::default(),
            sse_enabled: false,
            android_battery_status_enabled,
            android_volume_status_enabled: false,
            android_volume_control_enabled: false,
            command_execution_enabled: false,
            create_directory_authority: None,
            copy_file_authority: None,
            write_file_authority: None,
            android_battery_client,
            #[cfg(feature = "android-volume-status")]
            android_volume_client: AndroidVolumeClient::termux(),
            #[cfg(feature = "android-volume-control")]
            android_volume_control_authority: None,
            #[cfg(feature = "android-volume-control")]
            android_volume_control_client: AndroidVolumeControlClient::termux(),
            #[cfg(feature = "command-execution")]
            command_execution_client: None,
        }
    }

    #[cfg(all(test, feature = "android-volume-status"))]
    fn with_android_volume_client(
        security_policy: TransportSecurityPolicy,
        file_tools: FileSystemTools,
        android_volume_status_enabled: bool,
        android_volume_client: AndroidVolumeClient,
    ) -> Self {
        Self {
            security_policy,
            file_tools,
            audit_counters: Arc::new(Mutex::new(AuditCounters::default())),
            sessions: McpSessionStore::new(),
            mutation_worker_capacity: FilesystemMutationWorkerCapacity::default(),
            sse_enabled: false,
            android_battery_status_enabled: false,
            android_volume_status_enabled,
            android_volume_control_enabled: false,
            command_execution_enabled: false,
            create_directory_authority: None,
            copy_file_authority: None,
            write_file_authority: None,
            #[cfg(feature = "android-battery-status")]
            android_battery_client: AndroidBatteryClient::termux(),
            android_volume_client,
            #[cfg(feature = "android-volume-control")]
            android_volume_control_authority: None,
            #[cfg(feature = "android-volume-control")]
            android_volume_control_client: AndroidVolumeControlClient::termux(),
            #[cfg(feature = "command-execution")]
            command_execution_client: None,
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
            mutation_worker_capacity: FilesystemMutationWorkerCapacity::default(),
            sse_enabled: false,
            android_battery_status_enabled: false,
            android_volume_status_enabled: false,
            android_volume_control_enabled: false,
            command_execution_enabled,
            create_directory_authority: None,
            copy_file_authority: None,
            write_file_authority: None,
            #[cfg(feature = "android-battery-status")]
            android_battery_client: AndroidBatteryClient::termux(),
            #[cfg(feature = "android-volume-status")]
            android_volume_client: AndroidVolumeClient::termux(),
            #[cfg(feature = "android-volume-control")]
            android_volume_control_authority: None,
            #[cfg(feature = "android-volume-control")]
            android_volume_control_client: AndroidVolumeControlClient::termux(),
            command_execution_client: Some(command_execution_client),
        }
    }

    #[cfg(all(test, feature = "android-volume-control"))]
    fn with_android_volume_control_client(
        security_policy: TransportSecurityPolicy,
        file_tools: FileSystemTools,
        authority: Option<AndroidVolumeGrantAuthority>,
        client: AndroidVolumeControlClient,
    ) -> Self {
        let mut state = Self::new(security_policy, file_tools, false, false, false, None, None)
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
struct ReadBinaryRangeArguments {
    path: String,
    offset_bytes: u64,
    length_bytes: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadTextRangeArguments {
    path: String,
    offset_bytes: u64,
    max_bytes: usize,
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
struct FindPathsArguments {
    path: String,
    query: String,
    #[serde(default)]
    kind: FindPathFilter,
    #[serde(default)]
    max_depth: Option<u32>,
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

/// Build the binary target's protected MCP router with filesystem authorities.
///
/// The same source file is compiled independently by the library and binary
/// targets. This constructor is crate-private, so only the binary target's own
/// crate can select the command lane; downstream library consumers have no
/// enabling symbol, including when Cargo selects this package in a workspace.
#[expect(
    clippy::too_many_arguments,
    reason = "the package binary constructor carries explicit protection boundaries"
)]
#[rustfmt::skip]
pub(crate) fn binary_server_router_with_filesystem_authorities_and_options(
    protection: McpRouterProtection,
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    command_execution_enabled: bool,
    create_directory_authority: Option<CreateDirectoryGrantAuthority>,
    copy_file_authority: Option<CopyFileGrantAuthority>,
    write_file_authority: Option<WriteFileGrantAuthority>,
    options: McpTransportOptions,
) -> Router {
    protect_router(
        router_with_filesystem_authorities_and_options(
            security_policy,
            file_tools,
            android_battery_status_enabled,
            android_volume_status_enabled,
            command_execution_enabled,
            FilesystemMutationAuthorities::new(
                create_directory_authority,
                write_file_authority,
            )
            .with_copy_file(copy_file_authority),
            options,
        ),
        protection,
    )
}

/// Build the binary target's protected MCP router with every optional mutation
/// authority.
#[cfg(feature = "android-volume-control")]
#[expect(
    clippy::too_many_arguments,
    reason = "the package binary constructor carries explicit protection boundaries"
)]
#[rustfmt::skip]
pub(crate) fn binary_server_router_with_capability_authorities_and_options(
    protection: McpRouterProtection,
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    command_execution_enabled: bool,
    authorities: McpCapabilityAuthorities,
    options: McpTransportOptions,
) -> Router {
    protect_router(
        router_with_capability_authorities_and_options(
            security_policy,
            file_tools,
            android_battery_status_enabled,
            android_volume_status_enabled,
            command_execution_enabled,
            authorities,
            options,
        ),
        protection,
    )
}

/// Build a publicly embeddable MCP router with mandatory authentication and
/// request-resource protections.
///
/// Public embeddings cannot enable the process-execution lane. Command
/// diagnostics are initialized only by this package's crate-owned server startup,
/// which prevents a downstream binary from substituting its own `current_exe`.
#[rustfmt::skip]
pub fn protected_router(
    protection: McpRouterProtection,
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
) -> Router {
    protect_router(
        router(
            security_policy,
            file_tools,
            android_battery_status_enabled,
            android_volume_status_enabled,
            false,
        ),
        protection,
    )
}

/// Build a protected MCP router with explicit additive transport options.
#[rustfmt::skip]
pub fn protected_router_with_options(
    protection: McpRouterProtection,
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    options: McpTransportOptions,
) -> Router {
    protect_router(
        router_with_options(
            security_policy,
            file_tools,
            android_battery_status_enabled,
            android_volume_status_enabled,
            false,
            options,
        ),
        protection,
    )
}

/// Build a protected MCP router with the request-authorized directory mutation
/// capability enabled.
#[rustfmt::skip]
pub fn protected_router_with_create_directory_authority(
    protection: McpRouterProtection,
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    create_directory_authority: CreateDirectoryGrantAuthority,
) -> Router {
    protect_router(
        router_with_create_directory_authority(
            security_policy,
            file_tools,
            android_battery_status_enabled,
            android_volume_status_enabled,
            false,
            create_directory_authority,
        ),
        protection,
    )
}

/// Build a protected directory-authorized MCP router with explicit additive
/// transport options.
#[rustfmt::skip]
pub fn protected_router_with_create_directory_authority_and_options(
    protection: McpRouterProtection,
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    create_directory_authority: CreateDirectoryGrantAuthority,
    options: McpTransportOptions,
) -> Router {
    protect_router(
        router_with_create_directory_authority_and_options(
            security_policy,
            file_tools,
            android_battery_status_enabled,
            android_volume_status_enabled,
            false,
            create_directory_authority,
            options,
        ),
        protection,
    )
}

/// Build a protected MCP router with the request-authorized `copy_file`
/// mutation capability enabled. Preview calls remain available without a
/// grant, while every live copy requires an exact single-use grant.
#[rustfmt::skip]
pub fn protected_router_with_copy_file_authority(
    protection: McpRouterProtection,
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    copy_file_authority: CopyFileGrantAuthority,
) -> Router {
    protect_router(
        router_with_filesystem_authorities_and_options(
            security_policy,
            file_tools,
            android_battery_status_enabled,
            android_volume_status_enabled,
            false,
            FilesystemMutationAuthorities::new(None, None)
                .with_copy_file(Some(copy_file_authority)),
            McpTransportOptions::default(),
        ),
        protection,
    )
}

/// Build a protected copy-authorized MCP router with explicit additive
/// transport options.
#[rustfmt::skip]
pub fn protected_router_with_copy_file_authority_and_options(
    protection: McpRouterProtection,
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    copy_file_authority: CopyFileGrantAuthority,
    options: McpTransportOptions,
) -> Router {
    protect_router(
        router_with_filesystem_authorities_and_options(
            security_policy,
            file_tools,
            android_battery_status_enabled,
            android_volume_status_enabled,
            false,
            FilesystemMutationAuthorities::new(None, None)
                .with_copy_file(Some(copy_file_authority)),
            options,
        ),
        protection,
    )
}

/// Build a protected MCP router with independently optional filesystem
/// mutation authorities.
#[rustfmt::skip]
pub fn protected_router_with_filesystem_authorities(
    protection: McpRouterProtection,
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    create_directory_authority: Option<CreateDirectoryGrantAuthority>,
    write_file_authority: Option<WriteFileGrantAuthority>,
) -> Router {
    protect_router(
        router_with_filesystem_authorities(
            security_policy,
            file_tools,
            android_battery_status_enabled,
            android_volume_status_enabled,
            false,
            create_directory_authority,
            write_file_authority,
        ),
        protection,
    )
}

/// Build a protected filesystem-authorized MCP router with explicit additive
/// transport options.
#[expect(
    clippy::too_many_arguments,
    reason = "each constructor argument represents an explicit protection boundary"
)]
#[rustfmt::skip]
pub fn protected_router_with_filesystem_authorities_and_options(
    protection: McpRouterProtection,
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    create_directory_authority: Option<CreateDirectoryGrantAuthority>,
    write_file_authority: Option<WriteFileGrantAuthority>,
    options: McpTransportOptions,
) -> Router {
    binary_server_router_with_filesystem_authorities_and_options(
        protection,
        security_policy,
        file_tools,
        android_battery_status_enabled,
        android_volume_status_enabled,
        false,
        create_directory_authority,
        None,
        write_file_authority,
        options,
    )
}

/// Build a protected MCP router with all independently optional filesystem
/// mutation authorities. This additive constructor preserves the established
/// two-authority API while making the copy gate explicit.
#[expect(
    clippy::too_many_arguments,
    reason = "each constructor argument represents an explicit protection boundary"
)]
#[rustfmt::skip]
pub fn protected_router_with_all_filesystem_authorities(
    protection: McpRouterProtection,
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    create_directory_authority: Option<CreateDirectoryGrantAuthority>,
    copy_file_authority: Option<CopyFileGrantAuthority>,
    write_file_authority: Option<WriteFileGrantAuthority>,
) -> Router {
    protected_router_with_all_filesystem_authorities_and_options(
        protection,
        security_policy,
        file_tools,
        android_battery_status_enabled,
        android_volume_status_enabled,
        create_directory_authority,
        copy_file_authority,
        write_file_authority,
        McpTransportOptions::default(),
    )
}

/// Build a protected MCP router with all independently optional filesystem
/// mutation authorities and explicit additive transport options.
#[expect(
    clippy::too_many_arguments,
    reason = "each constructor argument represents an explicit protection boundary"
)]
#[rustfmt::skip]
pub fn protected_router_with_all_filesystem_authorities_and_options(
    protection: McpRouterProtection,
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    create_directory_authority: Option<CreateDirectoryGrantAuthority>,
    copy_file_authority: Option<CopyFileGrantAuthority>,
    write_file_authority: Option<WriteFileGrantAuthority>,
    options: McpTransportOptions,
) -> Router {
    binary_server_router_with_filesystem_authorities_and_options(
        protection,
        security_policy,
        file_tools,
        android_battery_status_enabled,
        android_volume_status_enabled,
        false,
        create_directory_authority,
        copy_file_authority,
        write_file_authority,
        options,
    )
}

/// Build a protected MCP router with independently optional directory and
/// Android-volume mutation authorities.
#[cfg(feature = "android-volume-control")]
#[rustfmt::skip]
pub fn protected_router_with_capability_authorities(
    protection: McpRouterProtection,
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    create_directory_authority: Option<CreateDirectoryGrantAuthority>,
    android_volume_control_authority: Option<AndroidVolumeGrantAuthority>,
) -> Router {
    protect_router(
        router_with_capability_authorities(
            security_policy,
            file_tools,
            android_battery_status_enabled,
            android_volume_status_enabled,
            false,
            create_directory_authority,
            android_volume_control_authority,
        ),
        protection,
    )
}

/// Build a protected MCP router with all independently optional mutation
/// authorities and explicit additive transport options.
#[cfg(feature = "android-volume-control")]
#[rustfmt::skip]
pub fn protected_router_with_capability_authorities_and_options(
    protection: McpRouterProtection,
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    authorities: McpCapabilityAuthorities,
    options: McpTransportOptions,
) -> Router {
    binary_server_router_with_capability_authorities_and_options(
        protection,
        security_policy,
        file_tools,
        android_battery_status_enabled,
        android_volume_status_enabled,
        false,
        authorities,
        options,
    )
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
pub(crate) fn router(
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    command_execution_enabled: bool,
) -> Router {
    router_with_options(
        security_policy,
        file_tools,
        android_battery_status_enabled,
        android_volume_status_enabled,
        command_execution_enabled,
        McpTransportOptions::default(),
    )
}

/// Build the MCP transport with explicit additive transport options. SSE is
/// default-disabled and cannot be enabled accidentally through the legacy
/// constructors.
#[rustfmt::skip]
pub(crate) fn router_with_options(
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    command_execution_enabled: bool,
    options: McpTransportOptions,
) -> Router {
    router_from_state(McpTransportState::new(
        security_policy,
        file_tools,
        android_battery_status_enabled,
        android_volume_status_enabled,
        command_execution_enabled,
        None,
        None,
    ).with_options(options))
}

/// Build the MCP transport with the dedicated `create_directory` mutation gate
/// enabled. Every mutating call still requires one valid request-scoped grant.
#[rustfmt::skip]
pub(crate) fn router_with_create_directory_authority(
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    command_execution_enabled: bool,
    create_directory_authority: CreateDirectoryGrantAuthority,
) -> Router {
    router_with_create_directory_authority_and_options(
        security_policy,
        file_tools,
        android_battery_status_enabled,
        android_volume_status_enabled,
        command_execution_enabled,
        create_directory_authority,
        McpTransportOptions::default(),
    )
}

/// Build the request-authorized directory transport with explicit additive
/// transport options.
#[rustfmt::skip]
pub(crate) fn router_with_create_directory_authority_and_options(
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    command_execution_enabled: bool,
    create_directory_authority: CreateDirectoryGrantAuthority,
    options: McpTransportOptions,
) -> Router {
    router_with_filesystem_authorities_and_options(
        security_policy,
        file_tools,
        android_battery_status_enabled,
        android_volume_status_enabled,
        command_execution_enabled,
        FilesystemMutationAuthorities::new(Some(create_directory_authority), None),
        options,
    )
}

/// Build the MCP transport with independently optional, purpose-bound
/// filesystem mutation authorities. Preview calls remain available when an
/// authority is absent; every live mutation requires its exact request grant.
#[rustfmt::skip]
pub(crate) fn router_with_filesystem_authorities(
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    command_execution_enabled: bool,
    create_directory_authority: Option<CreateDirectoryGrantAuthority>,
    write_file_authority: Option<WriteFileGrantAuthority>,
) -> Router {
    router_with_filesystem_authorities_and_options(
        security_policy,
        file_tools,
        android_battery_status_enabled,
        android_volume_status_enabled,
        command_execution_enabled,
        FilesystemMutationAuthorities::new(
            create_directory_authority,
            write_file_authority,
        ),
        McpTransportOptions::default(),
    )
}

/// Build the independently authorized filesystem transport with explicit
/// additive transport options.
#[rustfmt::skip]
fn router_with_filesystem_authorities_and_options(
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    command_execution_enabled: bool,
    authorities: FilesystemMutationAuthorities,
    options: McpTransportOptions,
) -> Router {
    let FilesystemMutationAuthorities {
        create_directory,
        copy_file,
        write_file,
    } = authorities;
    router_from_state(McpTransportState::new(
        security_policy,
        file_tools,
        android_battery_status_enabled,
        android_volume_status_enabled,
        command_execution_enabled,
        create_directory,
        write_file,
    ).with_copy_file_authority(copy_file).with_options(options))
}

/// Build the MCP transport with independently optional filesystem and Android
/// mutation authorities. The volume-control tool remains hidden unless the
/// volume authority is present; every live call still requires an exact
/// request-scoped grant.
#[cfg(feature = "android-volume-control")]
#[rustfmt::skip]
pub(crate) fn router_with_capability_authorities(
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    command_execution_enabled: bool,
    create_directory_authority: Option<CreateDirectoryGrantAuthority>,
    android_volume_control_authority: Option<AndroidVolumeGrantAuthority>,
) -> Router {
    router_with_capability_authorities_and_options(
        security_policy,
        file_tools,
        android_battery_status_enabled,
        android_volume_status_enabled,
        command_execution_enabled,
        McpCapabilityAuthorities::new(
            create_directory_authority,
            None,
            android_volume_control_authority,
        ),
        McpTransportOptions::default(),
    )
}

/// Build the independently authorized mutation transport with explicit
/// additive transport options.
#[cfg(feature = "android-volume-control")]
#[rustfmt::skip]
pub(crate) fn router_with_capability_authorities_and_options(
    security_policy: TransportSecurityPolicy,
    file_tools: FileSystemTools,
    android_battery_status_enabled: bool,
    android_volume_status_enabled: bool,
    command_execution_enabled: bool,
    authorities: McpCapabilityAuthorities,
    options: McpTransportOptions,
) -> Router {
    let McpCapabilityAuthorities {
        create_directory,
        copy_file,
        write_file,
        android_volume_control,
    } = authorities;
    router_from_state(
        McpTransportState::new(
            security_policy,
            file_tools,
            android_battery_status_enabled,
            android_volume_status_enabled,
            command_execution_enabled,
            create_directory,
            write_file,
        )
        .with_copy_file_authority(copy_file)
        .with_android_volume_control_authority(android_volume_control)
        .with_options(options),
    )
}

fn protect_router(router: Router, protection: McpRouterProtection) -> Router {
    let McpRouterProtection {
        listener_host: _,
        auth_policy,
        request_limits,
    } = protection;
    let max_body_bytes = request_limits.max_body_bytes();

    // Axum applies the most recently added route layer first. Authentication is
    // therefore the outer boundary, followed by fail-fast concurrency/timeout
    // limits, then the streaming body limit immediately before transport body
    // extraction. Unauthenticated oversized bodies are rejected without body
    // parsing or body-limit detail.
    router
        .layer(DefaultBodyLimit::max(max_body_bytes))
        .route_layer(middleware::from_fn_with_state(
            request_limits,
            enforce_mcp_request_limits,
        ))
        .route_layer(middleware::from_fn_with_state(
            auth_policy,
            require_mcp_auth,
        ))
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
    } else if method != Method::POST && headers.contains_key(REQUEST_GRANT_HEADER) {
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

    let capability_grant = match single_header_value(headers, REQUEST_GRANT_HEADER) {
        Ok(Some(value))
            if !value.is_empty()
                && value.len() <= MAX_REQUEST_GRANT_HEADER_BYTES
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
            if id.as_ref().is_some_and(|id| !json_rpc_id_fits(id)) {
                return json_rpc_id_too_large();
            }
            return invalid_request(id, reason);
        }
    };

    if let IncomingJsonRpcMessage::Request { id, .. } = &message {
        if !json_rpc_id_fits(id) {
            return json_rpc_id_too_large();
        }
    }

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
            let response = initialize_response(Some(id.clone()), params.clone(), state);
            let Some(session_id) = response
                .headers()
                .get(MCP_SESSION_ID_HEADER)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
            else {
                return response;
            };
            return maybe_sse_response(state, &session_id, response).await;
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
                let response = ping_response(Some(id));
                return maybe_sse_response(state, &session_id, response).await;
            }
            if phase != SessionPhase::Active {
                return server_not_initialized(Some(id));
            }

            let response = match method.as_str() {
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
            };
            maybe_sse_response(state, &session_id, response).await
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

async fn maybe_sse_response(
    state: &McpTransportState,
    session_id: &str,
    response: Response,
) -> Response {
    if !state.sse_enabled
        || response.status() != StatusCode::OK
        || !response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value.split(';').next().is_some_and(|media_type| {
                    media_type.trim().eq_ignore_ascii_case(APPLICATION_JSON)
                })
            })
    {
        return response;
    }

    let (mut parts, body) = response.into_parts();
    // Collect under the largest explicit complete-response contract. Responses
    // above the replay-event ceiling can then fall back to JSON without an
    // unbounded second buffer or a false internal error.
    let body = match to_bytes(body, MAX_MCP_COLLECTED_JSON_RESPONSE_BYTES).await {
        Ok(body) => body,
        Err(_) => {
            return transport_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "sse_response_unavailable",
                "The JSON-RPC response could not be bounded for SSE delivery.",
            );
        }
    };
    let value = match serde_json::from_slice::<Value>(&body) {
        Ok(value) => value,
        Err(_) => return Response::from_parts(parts, Body::from(body)),
    };
    let data = serde_json::to_vec(&value).expect("a parsed JSON value must serialize");
    if data.len() > MAX_MCP_SSE_EVENT_DATA_BYTES {
        return Response::from_parts(parts, Body::from(body));
    }

    let events = match state.sessions.record_sse_response(session_id, data) {
        Ok(events) => events,
        Err(SseReplayError::EventDataTooLarge) => {
            return Response::from_parts(parts, Body::from(body));
        }
        Err(error) => return sse_replay_error_response(error),
    };
    let body = sse_events_body(&events);
    parts.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, body)
}

fn sse_events_body(events: &[SseReplayEvent]) -> Body {
    let frames = events
        .iter()
        .map(|event| {
            let mut frame = Vec::new();
            event.append_wire_bytes(&mut frame);
            Ok::<Bytes, Infallible>(Bytes::from(frame))
        })
        .collect::<Vec<_>>();
    Body::from_stream(stream::iter(frames))
}

fn sse_events_response(events: &[SseReplayEvent]) -> Response {
    let mut response = Response::new(sse_events_body(events));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    response
}

fn valid_last_event_id(value: &str) -> bool {
    if value.is_empty()
        || value.len() > MAX_MCP_LAST_EVENT_ID_BYTES
        || !value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
    {
        return false;
    }
    let Some((stream_id, sequence)) = value.rsplit_once(':') else {
        return false;
    };
    if sequence.is_empty()
        || (sequence.len() > 1 && sequence.starts_with('0'))
        || sequence.parse::<u64>().is_err()
    {
        return false;
    }
    Uuid::parse_str(stream_id).is_ok_and(|parsed| parsed.to_string() == stream_id)
}

fn sse_replay_error_response(error: SseReplayError) -> Response {
    match error {
        SseReplayError::SessionNotFound => transport_error(
            StatusCode::NOT_FOUND,
            "session_not_found",
            "The MCP session does not exist or has expired.",
        ),
        SseReplayError::CursorNotFound => transport_error(
            StatusCode::NOT_FOUND,
            "sse_cursor_not_found",
            "The SSE replay cursor is unavailable for this session.",
        ),
        SseReplayError::EventDataTooLarge => transport_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "sse_replay_unavailable",
            "The SSE replay response exceeds its bounded retention posture.",
        ),
        SseReplayError::Poisoned => transport_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "session_state_unavailable",
            "MCP session state is unavailable.",
        ),
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

    let (session_id, phase) = match validate_session_request(headers, &state.sessions) {
        Ok(session) => session,
        Err(error) => return session_request_error_response(error),
    };
    if phase != SessionPhase::Active {
        return server_not_initialized(None);
    }

    if !state.sse_enabled {
        let mut response = StatusCode::METHOD_NOT_ALLOWED.into_response();
        response
            .headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("POST, DELETE"));
        return response;
    }

    let last_event_id = match single_header_value(headers, MCP_LAST_EVENT_ID_HEADER) {
        Ok(Some(value)) if valid_last_event_id(value) => value,
        Ok(None) => {
            let mut response = StatusCode::METHOD_NOT_ALLOWED.into_response();
            response
                .headers_mut()
                .insert(header::ALLOW, HeaderValue::from_static("POST, DELETE"));
            return response;
        }
        Ok(Some(_)) | Err(()) => {
            return transport_error(
                StatusCode::BAD_REQUEST,
                "invalid_last_event_id",
                "Last-Event-ID must contain exactly one bounded server-issued event ID.",
            );
        }
    };

    match state.sessions.replay_sse_after(&session_id, last_event_id) {
        Ok(events) => sse_events_response(&events),
        Err(error) => sse_replay_error_response(error),
    }
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

    let error_id = id.clone();
    let body = json!({
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
    });
    if !json_response_fits(&body, MAX_MCP_COLLECTED_JSON_RESPONSE_BYTES) {
        return bounded_payload_too_large(
            error_id,
            "Initialize response exceeds the bounded transport response byte limit.",
            MAX_MCP_COLLECTED_JSON_RESPONSE_BYTES,
        );
    }

    let session_id = match state.sessions.create() {
        Ok(session_id) => session_id,
        Err(error) => return session_store_error_response(error),
    };

    let mut response = (StatusCode::OK, Json(body)).into_response();
    response.headers_mut().insert(
        MCP_SESSION_ID_HEADER,
        HeaderValue::try_from(session_id.as_str())
            .expect("UUID session IDs are valid header values"),
    );
    response
}

#[rustfmt::skip]
fn ping_response(id: Option<Value>) -> Response {
    let error_id = id.clone();
    let body = json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "result": {},
    });
    bounded_json_rpc_ok(
        error_id,
        body,
        "Ping response exceeds the bounded transport response byte limit.",
    )
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

fn bounded_capability_authorization_denied(
    id: Option<Value>,
    reason_code: &'static str,
    max_response_bytes: usize,
) -> Response {
    let body = json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": {
            "code": -32003,
            "message": "Capability authorization denied",
            "data": {
                "reason": reason_code,
            },
        },
    });
    if serde_json::to_vec(&body).is_ok_and(|serialized| serialized.len() <= max_response_bytes) {
        return (StatusCode::FORBIDDEN, Json(body)).into_response();
    }

    capability_authorization_denied(None, reason_code)
}

#[rustfmt::skip]
fn tools_list_response(id: Option<Value>, state: &McpTransportState) -> Response {
    let error_id = id.clone();
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
                        "name": FIND_PATHS_TOOL,
                        "description": "Locate bounded case-sensitive literal basename matches below one configured filesystem safe root without reading file contents.",
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
                                    "maxLength": MAX_FIND_QUERY_BYTES,
                                    "x-maxBytes": MAX_FIND_QUERY_BYTES,
                                    "description": "Case-sensitive literal UTF-8 basename substring of at most 256 bytes; path separators, regular expressions, and globs are not evaluated.",
                                },
                                "kind": {
                                    "type": "string",
                                    "enum": ["any", "regular_file", "directory"],
                                    "description": "Optional exact object-kind filter; defaults to any.",
                                },
                                "max_depth": {
                                    "type": "integer",
                                    "minimum": MIN_FIND_DEPTH,
                                    "maximum": MAX_FIND_DEPTH,
                                    "description": format!(
                                        "Optional bounded traversal depth; defaults to {MAX_FIND_DEPTH}."
                                    ),
                                },
                            },
                            "required": ["path", "query"],
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
                        "name": READ_BINARY_RANGE_TOOL,
                        "description": "Read one bounded byte range as canonical padded base64 from a larger regular file inside a configured filesystem safe root.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "path": {
                                    "type": "string",
                                    "description": format!(
                                        "Absolute regular-file path inside one configured safe root; the file may be at most {MAX_BINARY_RANGE_FILE_BYTES} bytes."
                                    ),
                                },
                                "offset_bytes": {
                                    "type": "integer",
                                    "minimum": 0,
                                    "maximum": MAX_BINARY_RANGE_FILE_BYTES,
                                    "description": "Zero-based raw byte offset; EOF itself is accepted and returns an empty range.",
                                },
                                "length_bytes": {
                                    "type": "integer",
                                    "minimum": 1,
                                    "maximum": MAX_BINARY_RANGE_BYTES,
                                    "description": format!(
                                        "Maximum raw bytes to return; fixed upper bound is {MAX_BINARY_RANGE_BYTES}."
                                    ),
                                },
                            },
                            "required": ["path", "offset_bytes", "length_bytes"],
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
                        "name": READ_TEXT_RANGE_TOOL,
                        "description": "Read one bounded UTF-8 byte range from a larger regular file inside a configured filesystem safe root.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "path": {
                                    "type": "string",
                                    "description": format!(
                                        "Absolute regular-file path inside one configured safe root; the file may be at most {MAX_TEXT_RANGE_FILE_BYTES} bytes."
                                    ),
                                },
                                "offset_bytes": {
                                    "type": "integer",
                                    "minimum": 0,
                                    "maximum": MAX_TEXT_RANGE_FILE_BYTES,
                                    "description": "Zero-based UTF-8 byte boundary; EOF itself is accepted and returns an empty range.",
                                },
                                "max_bytes": {
                                    "type": "integer",
                                    "minimum": MIN_TEXT_RANGE_BYTES,
                                    "maximum": MAX_TEXT_RANGE_BYTES,
                                    "description": format!(
                                        "Maximum UTF-8 bytes to return; incomplete code points at a non-EOF boundary are deferred. Fixed bounds are {MIN_TEXT_RANGE_BYTES} through {MAX_TEXT_RANGE_BYTES}."
                                    ),
                                },
                            },
                            "required": ["path", "offset_bytes", "max_bytes"],
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
                                    "maxLength": DEFAULT_MAX_WRITE_BYTES,
                                    "x-maxBytes": DEFAULT_MAX_WRITE_BYTES,
                                    "description": "UTF-8 text content to write, subject to the fixed 1 MiB encoded byte limit.",
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

    let copy_file_tool = body
        .pointer_mut("/result/tools")
        .and_then(Value::as_array_mut)
        .and_then(|tools| {
            tools
                .iter_mut()
                .find(|tool| tool.get("name") == Some(&json!(COPY_FILE_TOOL)))
        })
        .expect("baseline discovery owns copy_file");
    if state.copy_file_authority.is_some() {
        copy_file_tool["description"] = json!(
            "Validate one bounded safe-rooted regular-file copy, or publish it with fixed mode 0600 only when dry_run=false and one source-identity/content/destination-bound MCP-Capability-Grant is valid."
        );
        let dry_run_schema = copy_file_tool
            .pointer_mut("/inputSchema/properties/dry_run")
            .expect("copy_file owns a dry_run schema");
        dry_run_schema["description"] = json!(
            "Defaults to true. Explicit false additionally requires the enabled copy mutation gate and one exact request-scoped grant."
        );
    } else {
        copy_file_tool["description"] = json!(
            "Validate one bounded safe-rooted regular-file copy without mutation; the dedicated copy mutation gate is disabled."
        );
        let dry_run_schema = copy_file_tool
            .pointer_mut("/inputSchema/properties/dry_run")
            .expect("copy_file owns a dry_run schema");
        dry_run_schema["const"] = json!(true);
        dry_run_schema["description"] = json!(
            "Mutation is disabled in this runtime posture; omitted dry_run and explicit true are accepted."
        );
    }

    let write_file_tool = body
        .pointer_mut("/result/tools")
        .and_then(Value::as_array_mut)
        .and_then(|tools| {
            tools
                .iter_mut()
                .find(|tool| tool.get("name") == Some(&json!(WRITE_FILE_TOOL)))
        })
        .expect("baseline discovery owns write_file");
    if state.write_file_authority.is_some() {
        write_file_tool["description"] = json!(
            "Validate one bounded UTF-8 safe-root file write, or create/replace it with fixed mode 0600 only when dry_run=false and one target/content/disposition-bound MCP-Capability-Grant is valid."
        );
        let dry_run_schema = write_file_tool
            .pointer_mut("/inputSchema/properties/dry_run")
            .expect("write_file owns a dry_run schema");
        dry_run_schema["description"] = json!(
            "Defaults to true. Explicit false additionally requires the enabled mutation gate and one request-scoped grant bound to create or replace posture."
        );
    } else {
        write_file_tool["description"] = json!(
            "Validate one bounded UTF-8 safe-root file write without mutation; the dedicated mutation gate is disabled."
        );
        let dry_run_schema = write_file_tool
            .pointer_mut("/inputSchema/properties/dry_run")
            .expect("write_file owns a dry_run schema");
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

    bounded_json_rpc_ok(
        error_id,
        body,
        "Tool discovery response exceeds the bounded transport response byte limit.",
    )
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
            CREATE_DIRECTORY_TOOL | COPY_FILE_TOOL | WRITE_FILE_TOOL | SET_ANDROID_VOLUME_TOOL
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
                state,
                session_id,
                capability_grant,
            )
            .await
        }
        FIND_PATHS_TOOL => {
            handle_find_paths_call(
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
        READ_BINARY_RANGE_TOOL => {
            handle_read_binary_range_call(
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
        READ_TEXT_RANGE_TOOL => {
            handle_read_text_range_call(
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
                state,
                session_id,
                capability_grant,
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
    runtime_status_response(id, state)
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

        // The prepared operation and its terminal audit guard move into one
        // detached task. Dropping the HTTP waiter cannot abandon or duplicate
        // the verified mutation/recovery outcome.
        let audit = AndroidVolumeMutationAuditGuard::new(Arc::clone(&state.audit_counters));
        let worker = tokio::spawn(async move {
            let outcome = prepared.execute().await;
            audit.finish(&outcome);
            outcome
        });
        match worker.await {
            Ok(Ok(result)) => ok_result(
                id,
                "set_android_volume: exact stream mutation completed and was verified.".to_owned(),
                json!(result),
            ),
            Ok(Err(error)) => volume_control_worker_error_response(id, error),
            Err(_error) => {
                volume_control_worker_error_response(id, AndroidVolumeControlError::WorkerFailed)
            }
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

#[cfg(feature = "android-volume-control")]
fn volume_control_worker_error_response(
    id: Option<Value>,
    error: AndroidVolumeControlError,
) -> Response {
    tool_error_result(
        id,
        SET_ANDROID_VOLUME_TOOL,
        "android_volume_control_failed",
        error.reason_code(),
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
        let Some(command_execution_client) = state.command_execution_client.as_ref() else {
            let failure = CommandPolicyDecision {
                allowed: false,
                reason_code: COMMAND_PROGRAM_UNAVAILABLE_REASON,
                profile: Some(profile),
            };
            record_command_policy_decision(&state.audit_counters, &failure);
            return tool_error_result(
                id,
                RUN_COMMAND_PROFILE_TOOL,
                COMMAND_EXECUTION_ERROR,
                COMMAND_PROGRAM_UNAVAILABLE_REASON,
            );
        };
        match command_execution_client.execute(profile).await {
            Ok(result) => {
                record_command_policy_decision(&state.audit_counters, &decision);
                ok_result(
                    id,
                    format!(
                        "run_command_profile: fixed read-only profile {} completed within all bounds.",
                        profile.id(),
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
    state: &McpTransportState,
) -> Response {
    let error_id = id.clone();
    let audit_counters_snapshot = audit_counters_snapshot(&state.audit_counters);
    let create_directory_mutation_enabled = state.create_directory_authority.is_some();
    let copy_file_mutation_enabled = state.copy_file_authority.is_some();
    let write_file_mutation_enabled = state.write_file_authority.is_some();
    let android_battery_status_enabled = state.android_battery_status_enabled;
    let android_volume_status_enabled = state.android_volume_status_enabled;
    let android_volume_control_enabled = state.android_volume_control_enabled;
    let command_execution_enabled = state.command_execution_enabled;
    let sse_enabled = state.sse_enabled;
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
    let copy_file_mode = if copy_file_mutation_enabled {
        "dry_run_or_source_content_destination_scoped_single_use_grant"
    } else {
        "dry_run_only_mutation_disabled"
    };
    let write_file_mode = if write_file_mutation_enabled {
        "dry_run_or_target_content_disposition_scoped_single_use_grant"
    } else {
        "dry_run_only_mutation_disabled"
    };
    let transport_mode = if sse_enabled {
        "streamable-http-2025-11-25-session-scoped-bounded-sse-replay"
    } else {
        "streamable-http-2025-11-25-session-scoped-no-sse"
    };

    // Keep each macro invocation comfortably below the crate recursion ceiling.
    // Runtime status is intentionally flat on the wire, so merge bounded object
    // fragments before placing the result into the JSON-RPC envelope.
    let Value::Object(mut structured_content) = json!({
        "server": "termux-mcp-edge",
        "version": env!("CARGO_PKG_VERSION"),
        "transport": "streamable_http_2025_11_25",
        "sessionManagement": "bounded_uuid_idle_expiry",
        "serverSentEvents": sse_enabled,
        "serverSentEventsMode": if sse_enabled { "finite_request_response_with_origin_stream_replay" } else { "disabled" },
        "sseMaxStreamsPerSession": MAX_MCP_SSE_STREAMS_PER_SESSION,
        "sseMaxEventsPerStream": MAX_MCP_SSE_EVENTS_PER_STREAM,
        "sseMaxEventDataBytes": MAX_MCP_SSE_EVENT_DATA_BYTES,
        "sseMaxReplayBytesPerSession": MAX_MCP_SSE_REPLAY_BYTES_PER_SESSION,
        "sseMaxLastEventIdBytes": MAX_MCP_LAST_EVENT_ID_BYTES,
        "sseRetryMilliseconds": MCP_SSE_RETRY_MILLISECONDS,
        "jsonRpcIdMaxBytes": MAX_MCP_JSON_RPC_ID_BYTES,
        "availableTools": available_tools,
        "platformInfo": true,
        "platformInfoMode": "read_only_non_sensitive_metadata",
        "androidStatus": true,
        "androidStatusMode": "read_only_allowlisted_status_no_api_or_control",
        "projectServiceStatus": true,
        "projectServiceStatusMode": "read_only_allowlisted_project_service_status",
        "filesystemTools": true,
        "filesystemToolMode": "default_dry_run_grant_gated_create_directory_copy_file_write_file_plus_bounded_find_paths_hash_file_list_directory_path_metadata_read_binary_file_read_binary_range_read_file_read_text_range_search_text",
        "pathDiscovery": true,
        "pathDiscoveryMatchMode": "case_sensitive_literal_basename",
        "pathDiscoveryMaxDepth": MAX_FIND_DEPTH,
        "pathDiscoveryMaxEntries": MAX_FIND_ENTRIES,
        "pathDiscoveryMaxMatches": MAX_FIND_MATCHES,
        "pathDiscoveryMaxQueryBytes": MAX_FIND_QUERY_BYTES,
        "pathDiscoveryMaxResponseBytes": MAX_FIND_RESPONSE_BYTES,
        "binaryFileReads": true,
        "binaryFileReadEncoding": "base64",
        "binaryFileReadMaxBytes": MAX_BINARY_READ_BYTES,
        "binaryFileReadMaxResponseBytes": MAX_BINARY_READ_RESPONSE_BYTES,
        "binaryRangeReads": true,
        "binaryRangeReadEncoding": "base64",
        "binaryRangeReadMaxFileBytes": MAX_BINARY_RANGE_FILE_BYTES,
        "binaryRangeReadMaxBytes": MAX_BINARY_RANGE_BYTES,
        "binaryRangeReadMaxResponseBytes": MAX_BINARY_RANGE_RESPONSE_BYTES,
    }) else {
        unreachable!("runtime status core fields must form a JSON object");
    };
    let Value::Object(filesystem_and_platform_content) = json!({
        "textRangeReads": true,
        "textRangeReadEncoding": "utf-8",
        "textRangeReadMinBytes": MIN_TEXT_RANGE_BYTES,
        "textRangeReadMaxFileBytes": MAX_TEXT_RANGE_FILE_BYTES,
        "textRangeReadMaxBytes": MAX_TEXT_RANGE_BYTES,
        "textRangeReadMaxResponseBytes": MAX_TEXT_RANGE_RESPONSE_BYTES,
        "fileHashing": true,
        "fileHashAlgorithm": "sha256",
        "fileHashMaxBytes": MAX_HASH_FILE_BYTES,
        "createDirectoryMutationEnabled": create_directory_mutation_enabled,
        "createDirectoryMutationMode": create_directory_mode,
        "createDirectoryGrantRequired": create_directory_mutation_enabled,
        "createDirectoryGrantHeader": REQUEST_GRANT_HEADER,
        "createDirectoryGrantTtlSeconds": CREATE_DIRECTORY_GRANT_TTL_SECONDS,
        "copyFileMutationEnabled": copy_file_mutation_enabled,
        "copyFileMode": copy_file_mode,
        "copyFileGrantRequired": copy_file_mutation_enabled,
        "copyFileGrantHeader": REQUEST_GRANT_HEADER,
        "copyFileGrantTtlSeconds": COPY_FILE_GRANT_TTL_SECONDS,
        "copyFileGrantBinding": "source_root_path_identity_size_sha256_destination_root_path_absent_no_replace",
        "copyFileMaxBytes": MAX_COPY_FILE_BYTES,
        "copyFileMaxResponseBytes": MAX_COPY_FILE_RESPONSE_BYTES,
        "copyFileResponsePosture": "path_free_bounded_metadata_only",
        "fileWrites": true,
        "fileWriteMode": write_file_mode,
        "fileWriteMutationEnabled": write_file_mutation_enabled,
        "fileWriteGrantRequired": write_file_mutation_enabled,
        "fileWriteGrantHeader": REQUEST_GRANT_HEADER,
        "fileWriteGrantTtlSeconds": WRITE_FILE_GRANT_TTL_SECONDS,
        "fileWriteMaxBytes": DEFAULT_MAX_WRITE_BYTES,
        "fileWriteMaxResponseBytes": MAX_WRITE_FILE_RESPONSE_BYTES,
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
        "androidVolumeGrantHeader": REQUEST_GRANT_HEADER,
        "androidVolumeGrantTtlSeconds": ANDROID_VOLUME_GRANT_TTL_SECONDS_IF_COMPILED,
        "androidDeviceControl": android_volume_control_enabled,
        "commandExecutionCompiled": cfg!(feature = "command-execution"),
        "commandExecution": command_execution_enabled,
        "commandExecutionMode": command_execution_mode,
        "arbitraryCommandExecution": false,
        // Preserve the established runtime-status contract: this field reports
        // Android device-control exposure. Filesystem mutations have their own
        // explicit gate/mode fields above.
        "highImpactTools": android_volume_control_enabled,
        "auditCounters": audit_counters_snapshot,
    }) else {
        unreachable!("runtime status capability fields must form a JSON object");
    };
    structured_content.extend(filesystem_and_platform_content);

    let body = json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "result": {
            "content": [
                {
                    "type": "text",
                    "text": format!(
                        "termux-mcp-edge runtime_status: transport={}, platform_info=read-only-non-sensitive, android_status=read-only-allowlisted, android_platform={}, android_battery_status={}, android_volume_status={}, android_volume_control={}, project_service_status=read-only-allowlisted, create_directory_mutation={}, copy_file_mutation={}, write_file_mutation={}, filesystem=default-dry-run-grant-gated-create-directory-copy-file-write-file-plus-bounded-find-paths-hash-file-list-metadata-binary-read-binary-range-text-read-text-range-search, android_device_control={}, command_execution={}, arbitrary_command_execution=disabled",
                        transport_mode,
                        android_platform_mode,
                        battery_mode,
                        volume_mode,
                        volume_control_mode,
                        create_directory_mode,
                        copy_file_mode,
                        write_file_mode,
                        if android_volume_control_enabled { "bounded_request_authorized_volume" } else { "disabled" },
                        command_execution_mode,
                    ),
                },
            ],
            "structuredContent": structured_content,
            "isError": false
        },
    });
    bounded_json_rpc_ok(
        error_id,
        body,
        "Runtime status response exceeds the bounded transport response byte limit.",
    )
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

fn binary_range_success_envelope_fits(id: Option<Value>) -> bool {
    let maximum_summary = format!(
        "Read and base64-encoded {MAX_BINARY_RANGE_BYTES} bytes from one bounded safe-rooted file range."
    );
    let body = result_body(
        id,
        maximum_summary,
        json!({
            "encoding": "base64",
            "data": "",
            "offsetBytes": MAX_BINARY_RANGE_FILE_BYTES,
            "sizeBytes": MAX_BINARY_RANGE_BYTES,
            "fileSizeBytes": MAX_BINARY_RANGE_FILE_BYTES,
            "eof": false,
            "maxReadBytes": MAX_BINARY_RANGE_BYTES,
            "maxFileBytes": MAX_BINARY_RANGE_FILE_BYTES,
            "maxResponseBytes": MAX_BINARY_RANGE_RESPONSE_BYTES,
        }),
    );
    serde_json::to_vec(&body)
        .ok()
        .and_then(|serialized| serialized.len().checked_add(MAX_BINARY_RANGE_BASE64_BYTES))
        .is_some_and(|bytes| bytes <= MAX_BINARY_RANGE_RESPONSE_BYTES)
}

#[rustfmt::skip]
async fn handle_read_binary_range_call(
    id: Option<Value>,
    arguments: Option<Value>,
    file_tools: &FileSystemTools,
    audit_counters: &SharedAuditCounters,
) -> Response {
    if !binary_range_success_envelope_fits(id.clone()) {
        record_filesystem_denied(
            audit_counters,
            READ_BINARY_RANGE_TOOL,
            FILESYSTEM_READ_GATE,
            AuditMode::ReadOnly,
            FILESYSTEM_RESPONSE_TOO_LARGE,
        );
        return bounded_payload_too_large(
            id,
            "Binary range response exceeds the staged read_binary_range response byte limit.",
            MAX_BINARY_RANGE_RESPONSE_BYTES,
        );
    }

    let arguments = match arguments {
        Some(arguments) => arguments,
        None => {
            record_filesystem_denied(
                audit_counters,
                READ_BINARY_RANGE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_MISSING_ARGUMENTS,
            );
            return invalid_params(
                id,
                "read_binary_range requires path, offset_bytes, and length_bytes arguments.",
            );
        }
    };
    let args = match serde_json::from_value::<ReadBinaryRangeArguments>(arguments) {
        Ok(args) => args,
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                READ_BINARY_RANGE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_INVALID_ARGUMENTS,
            );
            return invalid_params(id, TOOL_ARGUMENTS_INVALID);
        }
    };

    match file_tools
        .read_binary_range(args.path, args.offset_bytes, args.length_bytes)
        .await
    {
        Ok(result) => {
            let error_id = id.clone();
            let summary = format!(
                "Read and base64-encoded {} bytes from one bounded safe-rooted file range.",
                result.size_bytes
            );
            let Some(response) = bounded_ok_result(
                id,
                summary,
                json!(result),
                MAX_BINARY_RANGE_RESPONSE_BYTES,
            ) else {
                record_filesystem_denied(
                    audit_counters,
                    READ_BINARY_RANGE_TOOL,
                    FILESYSTEM_READ_GATE,
                    AuditMode::ReadOnly,
                    FILESYSTEM_RESPONSE_TOO_LARGE,
                );
                return bounded_payload_too_large(
                    error_id,
                    "Binary range response exceeds the staged read_binary_range response byte limit.",
                    MAX_BINARY_RANGE_RESPONSE_BYTES,
                );
            };
            record_filesystem_allowed(
                audit_counters,
                READ_BINARY_RANGE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_BINARY_RANGE_ALLOWED,
            );
            response
        }
        Err(AppError::PathTraversal { .. }) => {
            record_filesystem_denied(
                audit_counters,
                READ_BINARY_RANGE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_SAFE_ROOT_REJECTED,
            );
            invalid_params(id, "Path is outside the configured filesystem safe roots.")
        }
        Err(AppError::PathNotFound) => {
            record_filesystem_denied(
                audit_counters,
                READ_BINARY_RANGE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_BINARY_RANGE_NOT_FOUND,
            );
            invalid_params(id, "Binary range file does not exist.")
        }
        Err(AppError::UnsupportedPathType) => {
            record_filesystem_denied(
                audit_counters,
                READ_BINARY_RANGE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_BINARY_RANGE_UNSUPPORTED,
            );
            invalid_params(id, "Binary range target must be one regular file.")
        }
        Err(AppError::InvalidBinaryRange) => {
            record_filesystem_denied(
                audit_counters,
                READ_BINARY_RANGE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_BINARY_RANGE_INVALID,
            );
            invalid_params(id, INVALID_BINARY_RANGE_PUBLIC_MESSAGE)
        }
        Err(AppError::FileTooLarge { .. }) => {
            record_filesystem_denied(
                audit_counters,
                READ_BINARY_RANGE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_BINARY_RANGE_TOO_LARGE,
            );
            payload_too_large(id, "Binary range file exceeds the staged file byte limit.")
        }
        Err(AppError::FileChangedDuringRead) => {
            record_filesystem_denied(
                audit_counters,
                READ_BINARY_RANGE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_BINARY_RANGE_CHANGED,
            );
            resource_changed(id, "Binary range file changed during the bounded read.")
        }
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                READ_BINARY_RANGE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_BINARY_RANGE_FAILED,
            );
            internal_error(id, "Binary range read failed.")
        }
    }
}

fn text_range_success_envelope_fits(id: Option<Value>) -> bool {
    let maximum_summary =
        format!("Read {MAX_TEXT_RANGE_BYTES} UTF-8 bytes from one bounded safe-rooted file range.");
    let body = result_body(
        id,
        maximum_summary,
        json!({
            "content": "",
            "offsetBytes": MAX_TEXT_RANGE_FILE_BYTES,
            "nextOffsetBytes": MAX_TEXT_RANGE_FILE_BYTES,
            "sizeBytes": MAX_TEXT_RANGE_BYTES,
            "fileSizeBytes": MAX_TEXT_RANGE_FILE_BYTES,
            "eof": false,
            "maxReadBytes": MAX_TEXT_RANGE_BYTES,
            "maxFileBytes": MAX_TEXT_RANGE_FILE_BYTES,
            "maxResponseBytes": MAX_TEXT_RANGE_RESPONSE_BYTES,
        }),
    );
    serde_json::to_vec(&body)
        .ok()
        .and_then(|serialized| serialized.len().checked_add(MAX_TEXT_RANGE_ESCAPED_BYTES))
        .is_some_and(|bytes| bytes <= MAX_TEXT_RANGE_RESPONSE_BYTES)
}

#[rustfmt::skip]
async fn handle_read_text_range_call(
    id: Option<Value>,
    arguments: Option<Value>,
    file_tools: &FileSystemTools,
    audit_counters: &SharedAuditCounters,
) -> Response {
    if !text_range_success_envelope_fits(id.clone()) {
        record_filesystem_denied(
            audit_counters,
            READ_TEXT_RANGE_TOOL,
            FILESYSTEM_READ_GATE,
            AuditMode::ReadOnly,
            FILESYSTEM_RESPONSE_TOO_LARGE,
        );
        return bounded_payload_too_large(
            id,
            "Text range response exceeds the staged read_text_range response byte limit.",
            MAX_TEXT_RANGE_RESPONSE_BYTES,
        );
    }

    let arguments = match arguments {
        Some(arguments) => arguments,
        None => {
            record_filesystem_denied(
                audit_counters,
                READ_TEXT_RANGE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_MISSING_ARGUMENTS,
            );
            return invalid_params(
                id,
                "read_text_range requires path, offset_bytes, and max_bytes arguments.",
            );
        }
    };
    let args = match serde_json::from_value::<ReadTextRangeArguments>(arguments) {
        Ok(args) => args,
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                READ_TEXT_RANGE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_INVALID_ARGUMENTS,
            );
            return invalid_params(id, TOOL_ARGUMENTS_INVALID);
        }
    };

    match file_tools
        .read_text_range(args.path, args.offset_bytes, args.max_bytes)
        .await
    {
        Ok(result) => {
            let error_id = id.clone();
            let summary = format!(
                "Read {} UTF-8 bytes from one bounded safe-rooted file range.",
                result.size_bytes
            );
            let Some(response) = bounded_ok_result(
                id,
                summary,
                json!(result),
                MAX_TEXT_RANGE_RESPONSE_BYTES,
            ) else {
                record_filesystem_denied(
                    audit_counters,
                    READ_TEXT_RANGE_TOOL,
                    FILESYSTEM_READ_GATE,
                    AuditMode::ReadOnly,
                    FILESYSTEM_RESPONSE_TOO_LARGE,
                );
                return bounded_payload_too_large(
                    error_id,
                    "Text range response exceeds the staged read_text_range response byte limit.",
                    MAX_TEXT_RANGE_RESPONSE_BYTES,
                );
            };
            record_filesystem_allowed(
                audit_counters,
                READ_TEXT_RANGE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_TEXT_RANGE_ALLOWED,
            );
            response
        }
        Err(AppError::PathTraversal { .. }) => {
            record_filesystem_denied(
                audit_counters,
                READ_TEXT_RANGE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_SAFE_ROOT_REJECTED,
            );
            invalid_params(id, "Path is outside the configured filesystem safe roots.")
        }
        Err(AppError::PathNotFound) => {
            record_filesystem_denied(
                audit_counters,
                READ_TEXT_RANGE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_TEXT_RANGE_NOT_FOUND,
            );
            invalid_params(id, "Text range file does not exist.")
        }
        Err(AppError::UnsupportedPathType) => {
            record_filesystem_denied(
                audit_counters,
                READ_TEXT_RANGE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_TEXT_RANGE_UNSUPPORTED,
            );
            invalid_params(id, "Text range target must be one regular file.")
        }
        Err(AppError::InvalidTextRange) => {
            record_filesystem_denied(
                audit_counters,
                READ_TEXT_RANGE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_TEXT_RANGE_INVALID,
            );
            invalid_params(id, INVALID_TEXT_RANGE_PUBLIC_MESSAGE)
        }
        Err(AppError::FileTooLarge { .. }) => {
            record_filesystem_denied(
                audit_counters,
                READ_TEXT_RANGE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_TEXT_RANGE_TOO_LARGE,
            );
            payload_too_large(id, "Text range file exceeds the staged file byte limit.")
        }
        Err(AppError::InvalidFileEncoding) => {
            record_filesystem_denied(
                audit_counters,
                READ_TEXT_RANGE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_TEXT_RANGE_ENCODING_INVALID,
            );
            invalid_params(id, "Selected text range must contain valid UTF-8.")
        }
        Err(AppError::FileChangedDuringRead) => {
            record_filesystem_denied(
                audit_counters,
                READ_TEXT_RANGE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_TEXT_RANGE_CHANGED,
            );
            resource_changed(id, "Text range file changed during the bounded read.")
        }
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                READ_TEXT_RANGE_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_TEXT_RANGE_FAILED,
            );
            internal_error(id, "Text range read failed.")
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
        return bounded_capability_authorization_denied(
            id,
            FILESYSTEM_CREATE_MUTATION_DISABLED,
            MAX_CREATE_DIRECTORY_RESPONSE_BYTES,
        );
    }
    if !dry_run && capability_grant.is_none() {
        let reason = CreateDirectoryGrantError::Missing.reason_code();
        record_filesystem_denied(
            audit_counters,
            CREATE_DIRECTORY_TOOL,
            FILESYSTEM_WRITE_GATE,
            mode,
            reason,
        );
        return bounded_capability_authorization_denied(
            id,
            reason,
            MAX_CREATE_DIRECTORY_RESPONSE_BYTES,
        );
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
        let worker_permit = match state.mutation_worker_capacity.try_acquire() {
            Some(permit) => permit,
            None => {
                record_filesystem_denied(
                    audit_counters,
                    CREATE_DIRECTORY_TOOL,
                    FILESYSTEM_WRITE_GATE,
                    mode,
                    FILESYSTEM_MUTATION_WORKER_CAPACITY_EXCEEDED,
                );
                return filesystem_mutation_worker_capacity_exhausted(id);
            }
        };
        let worker_file_tools = file_tools.clone();
        let mutation_path = args.path;
        let authority = state
            .create_directory_authority
            .clone()
            .expect("enabled create_directory mutation owns an authority");
        let session_id = session_id.to_owned();
        let capability_grant = capability_grant.map(str::to_owned);
        let worker_audit =
            CreateDirectoryMutationAuditGuard::new(Arc::clone(audit_counters));
        let (waiter_guard, worker_commit) = filesystem_mutation_commit_guards();
        let joined = spawn_filesystem_mutation_worker(worker_permit, move || {
            run_create_directory_mutation_worker(
                worker_file_tools,
                mutation_path,
                authority,
                capability_grant,
                session_id,
                worker_commit,
                worker_audit,
            )
        })
        .await;
        waiter_guard.complete();
        match joined {
            Ok(FilesystemMutationWorkerOutcome::Completed(Ok(result))) => Ok(result),
            Ok(FilesystemMutationWorkerOutcome::Completed(Err(
                AuthorizedCreateDirectoryError::Authorization(error),
            ))) => {
                return bounded_capability_authorization_denied(
                    id,
                    error.reason_code(),
                    MAX_CREATE_DIRECTORY_RESPONSE_BYTES,
                );
            }
            Ok(FilesystemMutationWorkerOutcome::Completed(Err(
                AuthorizedCreateDirectoryError::Filesystem(error),
            ))) => Err(error),
            Ok(FilesystemMutationWorkerOutcome::Completed(Err(
                AuthorizedCreateDirectoryError::Cancelled,
            )))
            | Ok(FilesystemMutationWorkerOutcome::Cancelled) => {
                return filesystem_mutation_request_cancelled(id);
            }
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
            if dry_run {
                record_filesystem_allowed(
                    audit_counters,
                    CREATE_DIRECTORY_TOOL,
                    FILESYSTEM_WRITE_GATE,
                    mode,
                    FILESYSTEM_DRY_RUN_ALLOWED,
                );
            }
            response
        }
        Err(error) if dry_run => {
            create_directory_filesystem_error(id, audit_counters, mode, error)
        }
        Err(error) => create_directory_filesystem_error_response(id, error),
    }
}

#[rustfmt::skip]
fn create_directory_filesystem_error(
    id: Option<Value>,
    audit_counters: &SharedAuditCounters,
    mode: AuditMode,
    error: AppError,
) -> Response {
    record_filesystem_denied(
        audit_counters,
        CREATE_DIRECTORY_TOOL,
        FILESYSTEM_WRITE_GATE,
        mode,
        create_directory_filesystem_reason(&error),
    );
    create_directory_filesystem_error_response(id, error)
}

fn create_directory_filesystem_reason(error: &AppError) -> &'static str {
    match error {
        AppError::PathTraversal { .. } => FILESYSTEM_SAFE_ROOT_REJECTED,
        AppError::PathNotFound => FILESYSTEM_CREATE_PARENT_NOT_FOUND,
        AppError::PathAlreadyExists => FILESYSTEM_CREATE_EXISTS,
        _ => FILESYSTEM_CREATE_FAILED,
    }
}

#[rustfmt::skip]
fn create_directory_filesystem_error_response(
    id: Option<Value>,
    error: AppError,
) -> Response {
    match error {
        AppError::PathTraversal { .. } => invalid_params(
            id,
            "Filesystem safe-root validation failed: requested path is outside the configured safe roots.",
        ),
        AppError::PathNotFound => {
            invalid_params(id, "Filesystem parent directory does not exist.")
        }
        AppError::PathAlreadyExists => {
            invalid_params(id, "Filesystem destination already exists.")
        }
        _error => internal_error(id, "Filesystem directory creation failed."),
    }
}

#[rustfmt::skip]
async fn handle_copy_file_call(
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
                COPY_FILE_TOOL,
                FILESYSTEM_WRITE_GATE,
                AuditMode::DryRun,
                FILESYSTEM_MISSING_ARGUMENTS,
            );
            if capability_grant.is_some() {
                return capability_context_not_allowed(id);
            }
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
            if capability_grant.is_some() {
                return capability_context_not_allowed(id);
            }
            return invalid_params(id, TOOL_ARGUMENTS_INVALID);
        }
    };

    let dry_run = args.dry_run.unwrap_or(true);
    let mode = filesystem_write_mode(dry_run);
    // A grant header is valid only for the explicit live-copy posture. Reject
    // preview smuggling before any path validation or filesystem access.
    if capability_grant.is_some() && args.dry_run != Some(false) {
        record_filesystem_denied(
            audit_counters,
            COPY_FILE_TOOL,
            FILESYSTEM_WRITE_GATE,
            mode,
            FILESYSTEM_INVALID_ARGUMENTS,
        );
        return capability_context_not_allowed(id);
    }
    if !dry_run && state.copy_file_authority.is_none() {
        record_filesystem_denied(
            audit_counters,
            COPY_FILE_TOOL,
            FILESYSTEM_WRITE_GATE,
            mode,
            FILESYSTEM_COPY_MUTATION_DISABLED,
        );
        return bounded_capability_authorization_denied(
            id,
            FILESYSTEM_COPY_MUTATION_DISABLED,
            MAX_COPY_FILE_RESPONSE_BYTES,
        );
    }
    if !dry_run && capability_grant.is_none() {
        let reason = CopyFileGrantError::Missing.reason_code();
        record_filesystem_denied(
            audit_counters,
            COPY_FILE_TOOL,
            FILESYSTEM_WRITE_GATE,
            mode,
            reason,
        );
        return bounded_capability_authorization_denied(
            id,
            reason,
            MAX_COPY_FILE_RESPONSE_BYTES,
        );
    }
    let success_text = if dry_run {
        "Validated one bounded safe-rooted file copy without mutation."
    } else {
        "Copied one bounded safe-rooted file with fixed mode 0600."
    };
    // Use the actual request id and the longest valid copy-result shape before
    // any path access, worker admission, descriptor preparation, grant
    // consumption, or mutation.
    let response_preflight = crate::tools::CopyFileResult {
        dry_run,
        size_bytes: MAX_COPY_FILE_BYTES,
        mode: format!("{COPY_FILE_MODE:04o}"),
        max_file_bytes: MAX_COPY_FILE_BYTES,
        max_response_bytes: MAX_COPY_FILE_RESPONSE_BYTES,
    };
    if bounded_ok_result(
        id.clone(),
        success_text.to_owned(),
        json!(response_preflight),
        MAX_COPY_FILE_RESPONSE_BYTES,
    )
    .is_none()
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
    if let Err(error) = file_tools.copy_file_response_preview(
        &args.source_path,
        &args.destination_path,
        dry_run,
    ) {
        return copy_file_filesystem_error(id, audit_counters, mode, error);
    }

    let operation = if dry_run {
        file_tools
            .copy_file(args.source_path, args.destination_path, Some(true))
            .await
    } else {
        let worker_permit = match state.mutation_worker_capacity.try_acquire() {
            Some(permit) => permit,
            None => {
                record_filesystem_denied(
                    audit_counters,
                    COPY_FILE_TOOL,
                    FILESYSTEM_WRITE_GATE,
                    mode,
                    FILESYSTEM_MUTATION_WORKER_CAPACITY_EXCEEDED,
                );
                return filesystem_mutation_worker_capacity_exhausted(id);
            }
        };
        let worker_file_tools = file_tools.clone();
        let source_path = args.source_path;
        let destination_path = args.destination_path;
        let authority = state
            .copy_file_authority
            .clone()
            .expect("enabled copy_file mutation owns an authority");
        let session_id = session_id.to_owned();
        let capability_grant = capability_grant.map(str::to_owned);
        let worker_audit = CopyFileMutationAuditGuard::new(Arc::clone(audit_counters));
        let (waiter_guard, worker_commit) = filesystem_mutation_commit_guards();
        let joined = spawn_filesystem_mutation_worker(worker_permit, move || {
            run_copy_file_mutation_worker(CopyFileMutationWorker {
                file_tools: worker_file_tools,
                source_path,
                destination_path,
                authority,
                capability_grant,
                session_id,
                commit: worker_commit,
                audit: worker_audit,
            })
        })
        .await;
        waiter_guard.complete();
        match joined {
            Ok(FilesystemMutationWorkerOutcome::Completed(Ok(result))) => Ok(result),
            Ok(FilesystemMutationWorkerOutcome::Completed(Err(
                AuthorizedCopyFileError::Authorization(error),
            ))) => {
                return bounded_capability_authorization_denied(
                    id,
                    error.reason_code(),
                    MAX_COPY_FILE_RESPONSE_BYTES,
                );
            }
            Ok(FilesystemMutationWorkerOutcome::Completed(Err(
                AuthorizedCopyFileError::Filesystem(error),
            ))) => Err(error),
            Ok(FilesystemMutationWorkerOutcome::Completed(Err(
                AuthorizedCopyFileError::Cancelled,
            )))
            | Ok(FilesystemMutationWorkerOutcome::Cancelled) => {
                return filesystem_mutation_request_cancelled(id);
            }
            Err(_error) => Err(AppError::Io(std::io::Error::other(
                "copy file worker failed",
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
            if dry_run {
                record_filesystem_allowed(
                    audit_counters,
                    COPY_FILE_TOOL,
                    FILESYSTEM_WRITE_GATE,
                    mode,
                    FILESYSTEM_DRY_RUN_ALLOWED,
                );
            }
            response
        }
        Err(error) if dry_run => copy_file_filesystem_error(id, audit_counters, mode, error),
        Err(error) => copy_file_filesystem_error_response(id, error),
    }
}

fn copy_file_filesystem_reason(error: &AppError) -> &'static str {
    match error {
        AppError::PathTraversal { .. } => FILESYSTEM_SAFE_ROOT_REJECTED,
        AppError::CopySourceNotFound => FILESYSTEM_COPY_SOURCE_NOT_FOUND,
        AppError::CopyDestinationParentNotFound => FILESYSTEM_COPY_PARENT_NOT_FOUND,
        AppError::CopySourceDestinationSame => FILESYSTEM_COPY_SAME_PATH,
        AppError::PathAlreadyExists => FILESYSTEM_CREATE_EXISTS,
        AppError::UnsupportedPathType => FILESYSTEM_COPY_SOURCE_UNSUPPORTED,
        AppError::FileTooLarge { .. } => FILESYSTEM_COPY_SOURCE_TOO_LARGE,
        AppError::CopySourceChanged => FILESYSTEM_COPY_SOURCE_CHANGED,
        AppError::CopyDestinationChanged => FILESYSTEM_COPY_DESTINATION_CHANGED,
        AppError::WriteQuarantineCapacityExceeded => FILESYSTEM_COPY_QUARANTINE_FULL,
        AppError::WriteQuarantineBusy => FILESYSTEM_COPY_QUARANTINE_BUSY,
        _ => FILESYSTEM_COPY_FAILED,
    }
}

#[rustfmt::skip]
fn copy_file_filesystem_error(
    id: Option<Value>,
    audit_counters: &SharedAuditCounters,
    mode: AuditMode,
    error: AppError,
) -> Response {
    record_filesystem_denied(
        audit_counters,
        COPY_FILE_TOOL,
        FILESYSTEM_WRITE_GATE,
        mode,
        copy_file_filesystem_reason(&error),
    );
    copy_file_filesystem_error_response(id, error)
}

#[rustfmt::skip]
fn copy_file_filesystem_error_response(id: Option<Value>, error: AppError) -> Response {
    match error {
        AppError::PathTraversal { .. } => invalid_params(
            id,
            "Filesystem safe-root validation failed: requested path is outside the configured safe roots.",
        ),
        AppError::CopySourceNotFound => {
            invalid_params(id, "Filesystem copy source does not exist.")
        }
        AppError::CopyDestinationParentNotFound => {
            invalid_params(id, "Filesystem copy destination parent does not exist.")
        }
        AppError::CopySourceDestinationSame => {
            invalid_params(id, "Filesystem copy source and destination must differ.")
        }
        AppError::PathAlreadyExists => {
            invalid_params(id, "Filesystem copy destination already exists.")
        }
        AppError::UnsupportedPathType => {
            invalid_params(id, "Filesystem copy source must be a single-link regular file.")
        }
        AppError::FileTooLarge { .. } => {
            payload_too_large(id, "File exceeds the staged copy_file byte limit.")
        }
        AppError::CopySourceChanged => {
            resource_changed(id, "Filesystem copy source changed after validation.")
        }
        AppError::CopyDestinationChanged => {
            resource_changed(id, "Filesystem copy destination changed after validation.")
        }
        AppError::WriteQuarantineCapacityExceeded => copy_staging_capacity_exhausted(id),
        AppError::WriteQuarantineBusy => copy_staging_busy(id),
        _error => internal_error(id, "Filesystem copy failed."),
    }
}

fn find_paths_success_envelope_fits(id: Option<Value>) -> bool {
    let structured = json!({
        "path": "",
        "matches": [],
        "truncated": true,
        "entriesExamined": MAX_FIND_ENTRIES,
        "skippedInvalidUtf8Entries": MAX_FIND_ENTRIES,
        "skippedUnsafeEntries": MAX_FIND_ENTRIES,
        "skippedUnreadableEntries": MAX_FIND_ENTRIES,
        "queryBytes": MAX_FIND_QUERY_BYTES,
        "kindFilter": "regular_file",
        "maxDepth": MAX_FIND_DEPTH,
        "maxEntries": MAX_FIND_ENTRIES,
        "maxMatches": MAX_FIND_MATCHES,
        "maxResponseBytes": MAX_FIND_RESPONSE_BYTES,
    });
    let Ok(structured_bytes) = serde_json::to_vec(&structured) else {
        return false;
    };
    if structured_bytes.len() > MAX_FIND_STRUCTURED_CONTENT_BYTES {
        return false;
    }
    let body = result_body(
        id,
        format!(
            "Located {MAX_FIND_MATCHES} safe-rooted literal basename matches; the bounded discovery was truncated."
        ),
        structured,
    );
    serde_json::to_vec(&body)
        .ok()
        .and_then(|serialized| {
            serialized.len().checked_add(
                MAX_FIND_STRUCTURED_CONTENT_BYTES.saturating_sub(structured_bytes.len()),
            )
        })
        .is_some_and(|bytes| bytes <= MAX_FIND_RESPONSE_BYTES)
}

#[rustfmt::skip]
async fn handle_find_paths_call(
    id: Option<Value>,
    arguments: Option<Value>,
    file_tools: &FileSystemTools,
    audit_counters: &SharedAuditCounters,
) -> Response {
    if !find_paths_success_envelope_fits(id.clone()) {
        record_filesystem_denied(
            audit_counters,
            FIND_PATHS_TOOL,
            FILESYSTEM_READ_GATE,
            AuditMode::ReadOnly,
            FILESYSTEM_RESPONSE_TOO_LARGE,
        );
        return bounded_payload_too_large(
            id,
            "Path-discovery response exceeds the staged find_paths response byte limit.",
            MAX_FIND_RESPONSE_BYTES,
        );
    }

    let arguments = match arguments {
        Some(arguments) => arguments,
        None => {
            record_filesystem_denied(
                audit_counters,
                FIND_PATHS_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_MISSING_ARGUMENTS,
            );
            return invalid_params(id, "find_paths requires path and query arguments.");
        }
    };
    let args = match serde_json::from_value::<FindPathsArguments>(arguments) {
        Ok(args) => args,
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                FIND_PATHS_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_INVALID_ARGUMENTS,
            );
            return invalid_params(id, TOOL_ARGUMENTS_INVALID);
        }
    };

    if args.query.is_empty()
        || args.query.len() > MAX_FIND_QUERY_BYTES
        || args.query.chars().any(|character| matches!(character, '\0' | '\n' | '\r' | '/'))
    {
        record_filesystem_denied(
            audit_counters,
            FIND_PATHS_TOOL,
            FILESYSTEM_READ_GATE,
            AuditMode::ReadOnly,
            FILESYSTEM_FIND_INVALID_QUERY,
        );
        return invalid_params(
            id,
            &format!(
                "find_paths.query must be one non-empty literal basename substring of at most {MAX_FIND_QUERY_BYTES} UTF-8 bytes."
            ),
        );
    }
    if let Some(max_depth) = args.max_depth {
        if !(MIN_FIND_DEPTH..=MAX_FIND_DEPTH).contains(&max_depth) {
            record_filesystem_denied(
                audit_counters,
                FIND_PATHS_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_INVALID_DEPTH,
            );
            return invalid_params(
                id,
                &format!(
                    "find_paths.max_depth must be between {MIN_FIND_DEPTH} and {MAX_FIND_DEPTH}."
                ),
            );
        }
    }

    match file_tools
        .find_paths(args.path, args.query, args.kind, args.max_depth)
        .await
    {
        Ok(result) => {
            let error_id = id.clone();
            let summary = if result.truncated {
                format!(
                    "Located {} safe-rooted literal basename matches; the bounded discovery was truncated.",
                    result.matches.len()
                )
            } else {
                format!(
                    "Located {} safe-rooted literal basename matches.",
                    result.matches.len()
                )
            };
            let Some(response) = bounded_ok_result(
                id,
                summary,
                json!(result),
                MAX_FIND_RESPONSE_BYTES,
            ) else {
                record_filesystem_denied(
                    audit_counters,
                    FIND_PATHS_TOOL,
                    FILESYSTEM_READ_GATE,
                    AuditMode::ReadOnly,
                    FILESYSTEM_RESPONSE_TOO_LARGE,
                );
                return bounded_payload_too_large(
                    error_id,
                    "Path-discovery results exceed the staged find_paths response byte limit.",
                    MAX_FIND_RESPONSE_BYTES,
                );
            };
            record_filesystem_allowed(
                audit_counters,
                FIND_PATHS_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_FIND_ALLOWED,
            );
            response
        }
        Err(AppError::PathTraversal { .. }) => {
            record_filesystem_denied(
                audit_counters,
                FIND_PATHS_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_SAFE_ROOT_REJECTED,
            );
            invalid_params(
                id,
                "Filesystem safe-root validation failed: requested path is outside the configured safe roots.",
            )
        }
        Err(AppError::InvalidFindQuery) => {
            record_filesystem_denied(
                audit_counters,
                FIND_PATHS_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_FIND_INVALID_QUERY,
            );
            invalid_params(id, "Path-discovery query does not satisfy the literal basename contract.")
        }
        Err(_error) => {
            record_filesystem_denied(
                audit_counters,
                FIND_PATHS_TOOL,
                FILESYSTEM_READ_GATE,
                AuditMode::ReadOnly,
                FILESYSTEM_FIND_FAILED,
            );
            internal_error(id, "Filesystem path discovery failed.")
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
    if !dry_run && state.write_file_authority.is_none() {
        record_filesystem_denied(
            audit_counters,
            WRITE_FILE_TOOL,
            FILESYSTEM_WRITE_GATE,
            mode,
            FILESYSTEM_WRITE_MUTATION_DISABLED,
        );
        return bounded_capability_authorization_denied(
            id,
            FILESYSTEM_WRITE_MUTATION_DISABLED,
            MAX_WRITE_FILE_RESPONSE_BYTES,
        );
    }
    if !dry_run && capability_grant.is_none() {
        let reason = WriteFileGrantError::Missing.reason_code();
        record_filesystem_denied(
            audit_counters,
            WRITE_FILE_TOOL,
            FILESYSTEM_WRITE_GATE,
            mode,
            reason,
        );
        return bounded_capability_authorization_denied(
            id,
            reason,
            MAX_WRITE_FILE_RESPONSE_BYTES,
        );
    }

    let success_text = if dry_run {
        "Validated one bounded safe-rooted UTF-8 file write without mutation."
    } else {
        "Wrote one bounded safe-rooted UTF-8 file with fixed mode 0600."
    };
    let response_preflight = json!(file_tools.write_file_response_preview(bytes));
    if bounded_ok_result(
        id.clone(),
        success_text.to_owned(),
        response_preflight,
        MAX_WRITE_FILE_RESPONSE_BYTES,
    )
    .is_none()
    {
        record_filesystem_denied(
            audit_counters,
            WRITE_FILE_TOOL,
            FILESYSTEM_WRITE_GATE,
            mode,
            FILESYSTEM_RESPONSE_TOO_LARGE,
        );
        return bounded_payload_too_large(
            id,
            "File write response exceeds the staged response byte limit.",
            MAX_WRITE_FILE_RESPONSE_BYTES,
        );
    }

    let operation = if dry_run {
        file_tools
            .write_file(args.path, args.content, Some(true))
            .await
    } else {
        let worker_permit = match state.mutation_worker_capacity.try_acquire() {
            Some(permit) => permit,
            None => {
                record_filesystem_denied(
                    audit_counters,
                    WRITE_FILE_TOOL,
                    FILESYSTEM_WRITE_GATE,
                    mode,
                    FILESYSTEM_MUTATION_WORKER_CAPACITY_EXCEEDED,
                );
                return filesystem_mutation_worker_capacity_exhausted(id);
            }
        };
        let worker_file_tools = file_tools.clone();
        let mutation_path = args.path;
        let mutation_content = args.content;
        let authority = state
            .write_file_authority
            .clone()
            .expect("enabled write_file mutation owns an authority");
        let session_id = session_id.to_owned();
        let capability_grant = capability_grant.map(str::to_owned);
        let worker_audit = WriteFileMutationAuditGuard::new(Arc::clone(audit_counters));
        let (waiter_guard, worker_commit) = filesystem_mutation_commit_guards();
        let joined = spawn_filesystem_mutation_worker(worker_permit, move || {
            run_write_file_mutation_worker(WriteFileMutationWorker {
                file_tools: worker_file_tools,
                path: mutation_path,
                content: mutation_content,
                authority,
                capability_grant,
                session_id,
                commit: worker_commit,
                audit: worker_audit,
            })
        })
        .await;
        waiter_guard.complete();
        match joined {
            Ok(FilesystemMutationWorkerOutcome::Completed(Ok(result))) => Ok(result),
            Ok(FilesystemMutationWorkerOutcome::Completed(Err(
                AuthorizedWriteFileError::Authorization(error),
            ))) => {
                return bounded_capability_authorization_denied(
                    id,
                    error.reason_code(),
                    MAX_WRITE_FILE_RESPONSE_BYTES,
                );
            }
            Ok(FilesystemMutationWorkerOutcome::Completed(Err(
                AuthorizedWriteFileError::Filesystem(error),
            ))) => Err(error),
            Ok(FilesystemMutationWorkerOutcome::Completed(Err(
                AuthorizedWriteFileError::Cancelled,
            )))
            | Ok(FilesystemMutationWorkerOutcome::Cancelled) => {
                return filesystem_mutation_request_cancelled(id);
            }
            Err(_error) => Err(AppError::Io(std::io::Error::other(
                "write file worker failed",
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
                MAX_WRITE_FILE_RESPONSE_BYTES,
            ) else {
                if dry_run {
                    record_filesystem_denied(
                        audit_counters,
                        WRITE_FILE_TOOL,
                        FILESYSTEM_WRITE_GATE,
                        mode,
                        FILESYSTEM_RESPONSE_TOO_LARGE,
                    );
                }
                return bounded_payload_too_large(
                    error_id,
                    "File write response exceeds the staged response byte limit.",
                    MAX_WRITE_FILE_RESPONSE_BYTES,
                );
            };
            if dry_run {
                record_filesystem_allowed(
                    audit_counters,
                    WRITE_FILE_TOOL,
                    FILESYSTEM_WRITE_GATE,
                    mode,
                    filesystem_write_allowed_reason(true),
                );
            }
            response
        }
        Err(error) if dry_run => write_file_filesystem_error(id, audit_counters, mode, error),
        Err(error) => write_file_filesystem_error_response(id, error),
    }
}

fn write_file_filesystem_reason(error: &AppError) -> &'static str {
    match error {
        AppError::PathTraversal { .. } => FILESYSTEM_SAFE_ROOT_REJECTED,
        AppError::WritePayloadTooLarge { .. } => FILESYSTEM_WRITE_TOO_LARGE,
        AppError::PathNotFound => FILESYSTEM_WRITE_TARGET_NOT_FOUND,
        AppError::UnsupportedPathType => FILESYSTEM_WRITE_TARGET_UNSUPPORTED,
        AppError::PathAlreadyExists | AppError::WriteTargetChanged => {
            FILESYSTEM_WRITE_TARGET_CHANGED
        }
        AppError::WriteQuarantineCapacityExceeded => FILESYSTEM_WRITE_QUARANTINE_FULL,
        AppError::WriteQuarantineBusy => FILESYSTEM_WRITE_QUARANTINE_BUSY,
        _ => FILESYSTEM_WRITE_FAILED,
    }
}

#[rustfmt::skip]
fn write_file_filesystem_error(
    id: Option<Value>,
    audit_counters: &SharedAuditCounters,
    mode: AuditMode,
    error: AppError,
) -> Response {
    record_filesystem_denied(
        audit_counters,
        WRITE_FILE_TOOL,
        FILESYSTEM_WRITE_GATE,
        mode,
        write_file_filesystem_reason(&error),
    );
    write_file_filesystem_error_response(id, error)
}

#[rustfmt::skip]
fn write_file_filesystem_error_response(
    id: Option<Value>,
    error: AppError,
) -> Response {
    match error {
        AppError::PathTraversal { .. } => {
            invalid_params(
                id,
                "Filesystem safe-root validation failed: requested path is outside the configured safe roots.",
            )
        }
        AppError::WritePayloadTooLarge { .. } => {
            payload_too_large(
                id,
                "File content exceeds the staged write_file byte limit.",
            )
        }
        AppError::PathNotFound => {
            invalid_params(id, "File write parent or prepared replacement target was not found.")
        }
        AppError::UnsupportedPathType => {
            invalid_params(id, "File write target must be absent or an existing regular file.")
        }
        AppError::PathAlreadyExists | AppError::WriteTargetChanged => {
            resource_changed(id, "File write target changed after validation.")
        }
        AppError::WriteQuarantineCapacityExceeded => write_recovery_capacity_exhausted(id),
        AppError::WriteQuarantineBusy => write_recovery_busy(id),
        _error => {
            internal_error(id, "Filesystem write failed.")
        }
    }
}

#[rustfmt::skip]
fn ok_result(id: Option<Value>, text: String, structured_content: Value) -> Response {
    let error_id = id.clone();
    bounded_ok_result(
        id,
        text,
        structured_content,
        MAX_MCP_COLLECTED_JSON_RESPONSE_BYTES,
    )
    .unwrap_or_else(|| {
        bounded_payload_too_large(
            error_id,
            "Tool result exceeds the bounded transport response byte limit.",
            MAX_MCP_COLLECTED_JSON_RESPONSE_BYTES,
        )
    })
}

#[rustfmt::skip]
fn tool_error_result(
    id: Option<Value>,
    tool_name: &'static str,
    error_name: &'static str,
    reason_code: &'static str,
) -> Response {
    let error_id = id.clone();
    let body = json!({
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
    });
    bounded_json_rpc_ok(
        error_id,
        body,
        "Tool error result exceeds the bounded transport response byte limit.",
    )
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

fn json_response_fits(body: &Value, max_response_bytes: usize) -> bool {
    serde_json::to_vec(body).is_ok_and(|serialized| serialized.len() <= max_response_bytes)
}

fn bounded_json_rpc_ok(id: Option<Value>, body: Value, message: &'static str) -> Response {
    if json_response_fits(&body, MAX_MCP_COLLECTED_JSON_RESPONSE_BYTES) {
        return result_response(body);
    }

    bounded_payload_too_large(id, message, MAX_MCP_COLLECTED_JSON_RESPONSE_BYTES)
}

fn json_rpc_id_fits(id: &Value) -> bool {
    serde_json::to_vec(id).is_ok_and(|serialized| serialized.len() <= MAX_MCP_JSON_RPC_ID_BYTES)
}

fn json_rpc_id_too_large() -> Response {
    bounded_payload_too_large(
        None,
        "JSON-RPC request id exceeds the bounded transport byte limit.",
        MAX_MCP_COLLECTED_JSON_RESPONSE_BYTES,
    )
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
fn resource_changed(id: Option<Value>, message: &str) -> Response {
    (
        StatusCode::CONFLICT,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "error": {
                "code": -32004,
                "message": "Resource changed",
                "data": message,
            },
        })),
    )
        .into_response()
}

#[rustfmt::skip]
fn write_recovery_capacity_exhausted(id: Option<Value>) -> Response {
    (
        StatusCode::INSUFFICIENT_STORAGE,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "error": {
                "code": -32005,
                "message": "Write recovery capacity exhausted",
                "data": "Write recovery quarantine capacity is exhausted.",
            },
        })),
    )
        .into_response()
}

#[rustfmt::skip]
fn write_recovery_busy(id: Option<Value>) -> Response {
    (
        StatusCode::CONFLICT,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "error": {
                "code": -32006,
                "message": "Write recovery busy",
                "data": "Another cooperating writer owns the recovery quarantine lock.",
            },
        })),
    )
        .into_response()
}

#[rustfmt::skip]
fn copy_staging_capacity_exhausted(id: Option<Value>) -> Response {
    (
        StatusCode::INSUFFICIENT_STORAGE,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "error": {
                "code": -32005,
                "message": "Copy staging capacity exhausted",
                "data": "Private copy staging capacity is exhausted.",
            },
        })),
    )
        .into_response()
}

#[rustfmt::skip]
fn copy_staging_busy(id: Option<Value>) -> Response {
    (
        StatusCode::CONFLICT,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "error": {
                "code": -32006,
                "message": "Copy staging busy",
                "data": "Another cooperating filesystem publisher owns the private staging lock.",
            },
        })),
    )
        .into_response()
}

#[rustfmt::skip]
fn filesystem_mutation_worker_capacity_exhausted(id: Option<Value>) -> Response {
    let mut response = (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "error": {
                "code": -32007,
                "message": "Filesystem mutation capacity unavailable",
                "data": "The filesystem mutation worker capacity is currently exhausted.",
            },
        })),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
    response
}

#[rustfmt::skip]
fn filesystem_mutation_request_cancelled(id: Option<Value>) -> Response {
    let mut response = (
        StatusCode::CONFLICT,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "error": {
                "code": -32008,
                "message": "Filesystem mutation cancelled",
                "data": "The request was cancelled before filesystem mutation commit.",
            },
        })),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
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

    async fn response_json(response: Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn exhaust_mutation_workers(
        state: &McpTransportState,
    ) -> Vec<OwnedSemaphorePermit> {
        (0..MAX_CONCURRENT_FILESYSTEM_MUTATION_WORKERS)
            .map(|_| {
                state
                    .mutation_worker_capacity
                    .try_acquire()
                    .expect("fixed test mutation capacity must be available")
            })
            .collect()
    }

    /// Parks a dedicated synchronous thread while it owns the audit mutex.
    ///
    /// Async tests use this to stop a completed blocking mutation at its audit
    /// boundary without carrying a `std::sync::MutexGuard` across an await.
    /// Drop always releases and joins the thread so a test panic cannot strand
    /// the shared process.
    struct AuditCounterBlocker {
        release: Option<std::sync::mpsc::Sender<()>>,
        worker: Option<std::thread::JoinHandle<()>>,
    }

    impl AuditCounterBlocker {
        fn acquire(counters: SharedAuditCounters) -> Self {
            let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                let _guard = counters
                    .lock()
                    .expect("test audit counter lock must not be poisoned");
                acquired_tx
                    .send(())
                    .expect("test audit blocker owner disappeared");
                let _ = release_rx.recv();
            });
            acquired_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("test audit counter lock was not acquired");
            Self {
                release: Some(release_tx),
                worker: Some(worker),
            }
        }

        fn release(mut self) {
            self.finish();
        }

        fn finish(&mut self) {
            if let Some(release) = self.release.take() {
                let _ = release.send(());
            }
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    impl Drop for AuditCounterBlocker {
        fn drop(&mut self) {
            self.finish();
        }
    }

    const COPY_TEST_KEY: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn copy_test_authority(principal: &str) -> CopyFileGrantAuthority {
        CopyFileGrantAuthority::from_hex_key("copy-transport-test-1", COPY_TEST_KEY, principal)
            .expect("test copy authority must validate")
    }

    fn copy_test_state(
        file_tools: FileSystemTools,
        authority: Option<CopyFileGrantAuthority>,
    ) -> McpTransportState {
        McpTransportState::new(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            file_tools,
            false,
            false,
            false,
            None,
            None,
        )
        .with_copy_file_authority(authority)
    }

    fn issue_copy_test_grant(
        authority: &CopyFileGrantAuthority,
        file_tools: &FileSystemTools,
        session_id: &str,
        source: &std::path::Path,
        destination: &std::path::Path,
    ) -> String {
        let target = file_tools
            .copy_file_grant_target(
                source.to_string_lossy().as_ref(),
                destination.to_string_lossy().as_ref(),
            )
            .expect("test copy target must validate");
        authority
            .issue(session_id, &target)
            .expect("test copy grant must issue")
    }

    #[tokio::test]
    async fn copy_discovery_and_runtime_status_report_exact_gate_posture() {
        let safe_root = tempfile::tempdir().unwrap();
        let file_tools = FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).unwrap();
        let disabled = copy_test_state(file_tools.clone(), None);

        let tools = response_json(tools_list_response(Some(json!("tools")), &disabled)).await;
        let copy = tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == COPY_FILE_TOOL)
            .unwrap();
        assert_eq!(copy["inputSchema"]["properties"]["dry_run"]["const"], true);
        assert!(copy["description"]
            .as_str()
            .unwrap()
            .contains("mutation gate is disabled"));

        let disabled_status = response_json(runtime_status_response(
            Some(json!("runtime-disabled")),
            &disabled,
        ))
        .await;
        let disabled_status = &disabled_status["result"]["structuredContent"];
        assert_eq!(disabled_status["copyFileMutationEnabled"], false);
        assert_eq!(disabled_status["copyFileGrantRequired"], false);
        assert_eq!(disabled_status["copyFileMode"], "dry_run_only_mutation_disabled");
        assert_eq!(disabled_status["copyFileGrantHeader"], REQUEST_GRANT_HEADER);
        assert_eq!(disabled_status["copyFileGrantTtlSeconds"], COPY_FILE_GRANT_TTL_SECONDS);
        assert_eq!(disabled_status["copyFileMaxBytes"], MAX_COPY_FILE_BYTES);
        assert_eq!(disabled_status["copyFileMaxResponseBytes"], MAX_COPY_FILE_RESPONSE_BYTES);
        assert_eq!(disabled_status["copyFileResponsePosture"], "path_free_bounded_metadata_only");

        let enabled = copy_test_state(
            file_tools,
            Some(copy_test_authority("copy-discovery-runtime-principal")),
        );
        let tools = response_json(tools_list_response(Some(json!("tools")), &enabled)).await;
        let copy = tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == COPY_FILE_TOOL)
            .unwrap();
        assert!(copy["inputSchema"]["properties"]["dry_run"].get("const").is_none());
        assert!(copy["description"]
            .as_str()
            .unwrap()
            .contains("source-identity/content/destination-bound"));

        let enabled_status = response_json(runtime_status_response(
            Some(json!("runtime-enabled")),
            &enabled,
        ))
        .await;
        let enabled_status = &enabled_status["result"]["structuredContent"];
        assert_eq!(enabled_status["copyFileMutationEnabled"], true);
        assert_eq!(enabled_status["copyFileGrantRequired"], true);
        assert_eq!(
            enabled_status["copyFileMode"],
            "dry_run_or_source_content_destination_scoped_single_use_grant"
        );
        assert_eq!(
            enabled_status["copyFileGrantBinding"],
            "source_root_path_identity_size_sha256_destination_root_path_absent_no_replace"
        );
    }

    #[tokio::test]
    async fn copy_live_gate_and_missing_grant_fail_before_filesystem_access() {
        let safe_root = tempfile::tempdir().unwrap();
        let file_tools = FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).unwrap();
        let disabled = copy_test_state(file_tools.clone(), None);
        let session_id = Uuid::new_v4().to_string();
        let inaccessible = json!({
            "source_path": "/outside/not-readable",
            "destination_path": "/outside/not-writable",
            "dry_run": false,
        });

        let denied = handle_copy_file_call(
            Some(json!("disabled")),
            Some(inaccessible.clone()),
            &disabled,
            &session_id,
            Some("not-a-real-grant"),
        )
        .await;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let denied = response_json(denied).await;
        assert_eq!(
            denied["error"]["data"]["reason"],
            FILESYSTEM_COPY_MUTATION_DISABLED
        );

        let enabled = copy_test_state(
            file_tools,
            Some(copy_test_authority("copy-missing-grant-principal")),
        );
        let missing = handle_copy_file_call(
            Some(json!("missing")),
            Some(inaccessible),
            &enabled,
            &session_id,
            None,
        )
        .await;
        assert_eq!(missing.status(), StatusCode::FORBIDDEN);
        let missing = response_json(missing).await;
        assert_eq!(
            missing["error"]["data"]["reason"],
            CopyFileGrantError::Missing.reason_code()
        );
    }

    #[tokio::test]
    async fn copy_preview_rejects_grant_smuggling_without_consuming_exact_grant() {
        let safe_root = tempfile::tempdir().unwrap();
        let source = safe_root.path().join("source.bin");
        let destination = safe_root.path().join("destination.bin");
        std::fs::write(&source, b"copy-preview-content").unwrap();
        let file_tools = FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).unwrap();
        let authority = copy_test_authority("copy-preview-smuggling-principal");
        let state = copy_test_state(file_tools.clone(), Some(authority.clone()));
        let session_id = Uuid::new_v4().to_string();
        let grant = issue_copy_test_grant(
            &authority,
            &file_tools,
            &session_id,
            &source,
            &destination,
        );

        let smuggled = handle_copy_file_call(
            Some(json!("smuggled")),
            Some(json!({
                "source_path": source,
                "destination_path": destination,
                "dry_run": true,
            })),
            &state,
            &session_id,
            Some(&grant),
        )
        .await;
        assert_eq!(smuggled.status(), StatusCode::BAD_REQUEST);
        assert!(!destination.exists());

        let preview = handle_copy_file_call(
            Some(json!("preview")),
            Some(json!({
                "source_path": source,
                "destination_path": destination,
            })),
            &state,
            &session_id,
            None,
        )
        .await;
        assert_eq!(preview.status(), StatusCode::OK);
        let preview = response_json(preview).await;
        assert_eq!(preview["result"]["structuredContent"]["dryRun"], true);
        assert!(preview["result"]["structuredContent"].get("sourcePath").is_none());
        assert!(preview["result"]["structuredContent"].get("destinationPath").is_none());
        assert!(!destination.exists());

        let live = handle_copy_file_call(
            Some(json!("live")),
            Some(json!({
                "source_path": source,
                "destination_path": destination,
                "dry_run": false,
            })),
            &state,
            &session_id,
            Some(&grant),
        )
        .await;
        assert_eq!(live.status(), StatusCode::OK);
        let live = response_json(live).await;
        assert_eq!(live["result"]["structuredContent"]["dryRun"], false);
        assert!(live["result"]["structuredContent"].get("sourcePath").is_none());
        assert!(live["result"]["structuredContent"].get("destinationPath").is_none());
        assert_eq!(std::fs::read(&destination).unwrap(), b"copy-preview-content");
    }

    #[tokio::test]
    async fn copy_response_and_worker_capacity_preflights_preserve_grant() {
        let safe_root = tempfile::tempdir().unwrap();
        let source = safe_root.path().join("source.bin");
        let destination = safe_root.path().join("destination.bin");
        std::fs::write(&source, b"copy-preflight-content").unwrap();
        let file_tools = FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).unwrap();
        let authority = copy_test_authority("copy-preflight-capacity-principal");
        let state = copy_test_state(file_tools.clone(), Some(authority.clone()));
        let session_id = Uuid::new_v4().to_string();
        let grant = issue_copy_test_grant(
            &authority,
            &file_tools,
            &session_id,
            &source,
            &destination,
        );
        let arguments = json!({
            "source_path": source,
            "destination_path": destination,
            "dry_run": false,
        });

        let oversized = handle_copy_file_call(
            Some(json!("x".repeat(MAX_COPY_FILE_RESPONSE_BYTES))),
            Some(arguments.clone()),
            &state,
            &session_id,
            Some(&grant),
        )
        .await;
        assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(!destination.exists());

        let permits = exhaust_mutation_workers(&state);
        let capacity = handle_copy_file_call(
            Some(json!("capacity")),
            Some(arguments.clone()),
            &state,
            &session_id,
            Some(&grant),
        )
        .await;
        assert_eq!(capacity.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(!destination.exists());
        drop(permits);

        let allowed = handle_copy_file_call(
            Some(json!("allowed")),
            Some(arguments),
            &state,
            &session_id,
            Some(&grant),
        )
        .await;
        assert_eq!(allowed.status(), StatusCode::OK);
        assert_eq!(std::fs::read(&destination).unwrap(), b"copy-preflight-content");
        let counters = state.audit_counters.lock().unwrap().clone();
        assert_eq!(counters.by_tool[COPY_FILE_TOOL].allowed, 1);
        assert_eq!(counters.by_tool[COPY_FILE_TOOL].denied, 2);
        assert_eq!(
            counters.by_reason_code[FILESYSTEM_RESPONSE_TOO_LARGE].denied,
            1
        );
        assert_eq!(
            counters.by_reason_code[FILESYSTEM_MUTATION_WORKER_CAPACITY_EXCEEDED].denied,
            1
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn live_copy_worker_records_exactly_once_after_http_waiter_is_dropped() {
        let safe_root = tempfile::tempdir().unwrap();
        let source = safe_root.path().join("source.bin");
        let destination = safe_root.path().join("destination.bin");
        std::fs::write(&source, b"detached-copy-content").unwrap();
        let file_tools = FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).unwrap();
        let issuer_tools = file_tools.clone();
        let authority = copy_test_authority("copy-detached-audit-principal");
        let state = copy_test_state(file_tools, Some(authority.clone()));
        let session_id = Uuid::new_v4().to_string();
        let binding = issuer_tools
            .copy_file_grant_target(
                source.to_string_lossy().as_ref(),
                destination.to_string_lossy().as_ref(),
            )
            .unwrap();
        let grant = authority.issue(&session_id, &binding).unwrap();

        let counters = Arc::clone(&state.audit_counters);
        let counter_blocker = AuditCounterBlocker::acquire(Arc::clone(&counters));
        let request_state = state.clone();
        let request_session = session_id.clone();
        let request_grant = grant.clone();
        let request_source = source.clone();
        let request_destination = destination.clone();
        let request_task = tokio::spawn(async move {
            handle_copy_file_call(
                Some(json!("detached-copy")),
                Some(json!({
                    "source_path": request_source,
                    "destination_path": request_destination,
                    "dry_run": false,
                })),
                &request_state,
                &request_session,
                Some(&request_grant),
            )
            .await
        });

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !destination.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "authorized copy did not publish before the test deadline"
            );
            tokio::task::yield_now().await;
        }
        request_task.abort();
        assert!(request_task.await.unwrap_err().is_cancelled());
        counter_blocker.release();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let snapshot = counters.lock().unwrap().clone();
            if snapshot.allowed_total == 1 {
                assert_eq!(snapshot.denied_total, 0);
                assert_eq!(snapshot.by_tool[COPY_FILE_TOOL].allowed, 1);
                assert_eq!(snapshot.by_tool[COPY_FILE_TOOL].denied, 0);
                assert_eq!(
                    snapshot.by_reason_code[FILESYSTEM_COPY_ALLOWED].allowed,
                    1
                );
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "detached copy worker did not record its terminal audit outcome"
            );
            tokio::task::yield_now().await;
        }
        assert_eq!(std::fs::read(&destination).unwrap(), b"detached-copy-content");
        assert_eq!(
            authority
                .consume(Some(&grant), &session_id, &binding)
                .unwrap_err(),
            CopyFileGrantError::Replayed
        );
    }

    #[tokio::test]
    async fn equivalent_copy_router_authorities_share_single_replay_domain() {
        use axum::{body::Body, http::{header, Request}};
        use tower::ServiceExt;

        let safe_root = tempfile::tempdir().unwrap();
        let source = safe_root.path().join("source.bin");
        let destination = safe_root.path().join("destination.bin");
        std::fs::write(&source, b"shared-copy-replay-content").unwrap();
        let file_tools = FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).unwrap();
        let first_authority = copy_test_authority("copy-shared-router-principal");
        let second_authority = copy_test_authority("copy-shared-router-principal");
        let first_state = copy_test_state(file_tools.clone(), Some(first_authority.clone()));
        let mut second_state = copy_test_state(file_tools.clone(), Some(second_authority));
        second_state.sessions = first_state.sessions.clone();
        let session = first_state.sessions.create().unwrap();
        first_state.sessions.activate(&session).unwrap();
        let grant = issue_copy_test_grant(
            &first_authority,
            &file_tools,
            &session,
            &source,
            &destination,
        );
        let first_router = router_from_state(first_state);
        let second_router = router_from_state(second_state);

        let request = |id: &str| {
            Request::post("/mcp")
                .header(header::HOST, "localhost:8000")
                .header(header::ORIGIN, "http://localhost:8000")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ACCEPT, MCP_POST_ACCEPT)
                .header(MCP_PROTOCOL_VERSION_HEADER, MCP_PROTOCOL_VERSION)
                .header(MCP_SESSION_ID_HEADER, &session)
                .header(REQUEST_GRANT_HEADER, &grant)
                .body(Body::from(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "method": "tools/call",
                    "params": {
                        "name": COPY_FILE_TOOL,
                        "arguments": {
                            "source_path": source,
                            "destination_path": destination,
                            "dry_run": false,
                        }
                    }
                }).to_string()))
                .unwrap()
        };

        let first = first_router.oneshot(request("copy-router-a")).await.unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(std::fs::read(&destination).unwrap(), b"shared-copy-replay-content");
        std::fs::remove_file(&destination).unwrap();

        let replay = second_router.oneshot(request("copy-router-b")).await.unwrap();
        assert_eq!(replay.status(), StatusCode::FORBIDDEN);
        let replay = response_json(replay).await;
        assert_eq!(
            replay["error"]["data"]["reason"],
            CopyFileGrantError::Replayed.reason_code()
        );
        assert!(!destination.exists());
        let serialized = replay.to_string();
        assert!(!serialized.contains(COPY_TEST_KEY));
        assert!(!serialized.contains("copy-shared-router-principal"));
        assert!(!serialized.contains(&grant));
    }

    #[test]
    fn copy_changed_errors_map_to_stable_private_reasons_and_path_free_conflicts() {
        assert_eq!(
            copy_file_filesystem_reason(&AppError::CopySourceChanged),
            FILESYSTEM_COPY_SOURCE_CHANGED
        );
        assert_eq!(
            copy_file_filesystem_reason(&AppError::CopyDestinationChanged),
            FILESYSTEM_COPY_DESTINATION_CHANGED
        );
        for error in [AppError::CopySourceChanged, AppError::CopyDestinationChanged] {
            let response = copy_file_filesystem_error_response(Some(json!("changed")), error);
            assert_eq!(response.status(), StatusCode::CONFLICT);
        }
    }

    #[test]
    fn copy_lock_contention_cancellation_preserves_grant_and_retry_commits() {
        let safe_root = tempfile::tempdir().unwrap();
        let source = safe_root.path().join("source.bin");
        let destination = safe_root.path().join("destination.bin");
        std::fs::write(&source, b"copy-cancel-content").unwrap();
        let file_tools = FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).unwrap();
        let retry_tools = file_tools.clone();
        let authority = copy_test_authority("copy-lock-cancel-principal");
        let retry_authority = authority.clone();
        let session_id = Uuid::new_v4().to_string();
        let retry_session = session_id.clone();
        let grant = issue_copy_test_grant(
            &authority,
            &file_tools,
            &session_id,
            &source,
            &destination,
        );
        let retry_grant = grant.clone();
        let counters = Arc::new(Mutex::new(AuditCounters::default()));
        let waiting_counters = Arc::clone(&counters);
        let publication_lock = crate::tools::acquire_filesystem_publication_lock_for_test();
        let (waiter, worker_commit) = filesystem_mutation_commit_guards();
        let (contended_tx, contended_rx) = std::sync::mpsc::channel();
        let waiting_source = source.to_string_lossy().to_string();
        let waiting_destination = destination.to_string_lossy().to_string();
        let worker = std::thread::spawn(move || {
            run_copy_file_mutation_worker_with_lock_contention_hook(
                CopyFileMutationWorker {
                    file_tools,
                    source_path: waiting_source,
                    destination_path: waiting_destination,
                    authority,
                    capability_grant: Some(grant),
                    session_id,
                    commit: worker_commit,
                    audit: CopyFileMutationAuditGuard::new(waiting_counters),
                },
                || contended_tx.send(()).unwrap(),
            )
        });
        contended_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("copy worker did not contend on publication lock");
        drop(waiter);
        drop(publication_lock);
        assert!(matches!(
            worker.join().unwrap(),
            FilesystemMutationWorkerOutcome::Cancelled
        ));
        assert!(!destination.exists());

        let (retry_waiter, retry_commit) = filesystem_mutation_commit_guards();
        let retry = run_copy_file_mutation_worker(CopyFileMutationWorker {
            file_tools: retry_tools,
            source_path: source.to_string_lossy().to_string(),
            destination_path: destination.to_string_lossy().to_string(),
            authority: retry_authority,
            capability_grant: Some(retry_grant),
            session_id: retry_session,
            commit: retry_commit,
            audit: CopyFileMutationAuditGuard::new(Arc::clone(&counters)),
        });
        retry_waiter.complete();
        assert!(matches!(
            retry,
            FilesystemMutationWorkerOutcome::Completed(Ok(_))
        ));
        assert_eq!(std::fs::read(&destination).unwrap(), b"copy-cancel-content");
        let counters = counters.lock().unwrap().clone();
        assert_eq!(counters.by_tool[COPY_FILE_TOOL].allowed, 1);
        assert_eq!(counters.by_tool[COPY_FILE_TOOL].denied, 1);
        assert_eq!(
            counters.by_reason_code[FILESYSTEM_MUTATION_REQUEST_CANCELLED].denied,
            1
        );
    }

    #[test]
    fn public_protection_rejects_unauthenticated_non_loopback_listener() {
        for listener_host in ["0.0.0.0", "::", "192.0.2.10", "example.com"] {
            let error = McpRouterProtection::new(
                listener_host,
                McpAuthPolicy::unauthenticated_localhost_only(),
                McpRequestLimits::from_seconds(1, 1, 1_024).unwrap(),
            )
            .expect_err("unauthenticated protected routers require loopback listeners");

            assert!(error.to_string().contains("loopback listener host"));
        }
    }

    #[test]
    fn sse_json_collector_tracks_largest_explicit_response_contract() {
        assert_eq!(
            MAX_MCP_COLLECTED_JSON_RESPONSE_BYTES,
            MAX_TEXT_RANGE_RESPONSE_BYTES
        );
    }

    #[test]
    fn oversized_initialize_response_does_not_allocate_a_session() {
        let safe_root = tempfile::tempdir().unwrap();
        let state = McpTransportState::new(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            FileSystemTools::try_new(vec![safe_root.path().to_path_buf()])
                .expect("test safe root must validate"),
            false,
            false,
            false,
            None,
            None,
        );
        let oversized_id = "x".repeat(MAX_MCP_COLLECTED_JSON_RESPONSE_BYTES);
        let response = initialize_response(
            Some(json!(oversized_id)),
            Some(json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "bounded-id-test", "version": "1.0.0"},
            })),
            &state,
        );

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(response.headers().get(MCP_SESSION_ID_HEADER).is_none());
        for _ in 0..crate::mcp_session::MAX_MCP_SESSIONS {
            assert!(state.sessions.create().is_ok());
        }
        assert_eq!(
            state.sessions.create(),
            Err(SessionStoreError::CapacityExhausted)
        );
    }

    #[tokio::test]
    async fn public_protected_router_authenticates_before_body_limit_and_reaches_transport() {
        use axum::{
            body::Body,
            http::{header, Request},
        };
        use tower::ServiceExt;

        let safe_root = tempfile::tempdir().unwrap();
        let app = protected_router(
            McpRouterProtection::new(
                "127.0.0.1",
                McpAuthPolicy::static_bearer("expected-token").unwrap(),
                McpRequestLimits::from_seconds(1, 5, 1_024).unwrap(),
            )
            .unwrap(),
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).expect("test safe root must validate"),
            false,
            false,
        );

        let unauthenticated = app
            .clone()
            .oneshot(
                Request::post("/mcp")
                    .header(header::HOST, "localhost:8000")
                    .header(header::ORIGIN, "http://localhost:8000")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("x".repeat(2_048)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

        let initialize = json!({
            "jsonrpc": "2.0",
            "id": "protected-initialize",
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "protected-router-test", "version": "1.0.0"}
            }
        });
        let authenticated = app
            .oneshot(
                Request::post("/mcp")
                    .header(header::HOST, "localhost:8000")
                    .header(header::ORIGIN, "http://localhost:8000")
                    .header(header::AUTHORIZATION, "Bearer expected-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::ACCEPT, MCP_POST_ACCEPT)
                    .body(Body::from(initialize.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(authenticated.status(), StatusCode::OK);
        assert!(authenticated.headers().contains_key(MCP_SESSION_ID_HEADER));
    }

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
            FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).expect("test safe root must validate"),
            true,
            true,
            false,
            None,
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
                "find_paths",
                "hash_file",
                "list_directory",
                "path_metadata",
                "read_binary_file",
                "read_binary_range",
                "read_file",
                "read_text_range",
                "search_text",
                "write_file",
                "android_battery_status",
                "android_volume_status",
            ]
        );

        let runtime = runtime_status_response(Some(json!("runtime")), &state);
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
            18
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mutation_worker_permit_covers_timed_out_preparation_until_worker_finishes() {
        let capacity = FilesystemMutationWorkerCapacity::new(1);
        let permit = capacity.try_acquire().unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let worker = spawn_filesystem_mutation_worker(permit, move || {
            // Production runs descriptor preparation first inside this same
            // permit-owned closure. Park at that phase before any authorization
            // or mutation to model a timed-out preparation deterministically.
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        });

        started_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("blocking worker did not start");
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), worker)
                .await
                .is_err(),
            "the async waiter should time out while blocking preparation continues"
        );
        assert!(
            capacity.try_acquire().is_none(),
            "dropping the timed-out waiter must not release a live preparation permit"
        );

        release_tx.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(released) = capacity.try_acquire() {
                drop(released);
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "worker completion did not release its mutation permit"
            );
            tokio::task::yield_now().await;
        }
    }

    #[test]
    fn create_directory_cancellation_winner_preserves_grant_and_worker_winner_commits() {
        let safe_root = tempfile::tempdir().unwrap();
        let file_tools = FileSystemTools::try_new(vec![safe_root.path().to_path_buf()])
            .expect("test safe root must validate");
        let authority = CreateDirectoryGrantAuthority::from_hex_key(
            "test-key-1",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "cancel-create-test-principal",
        )
        .unwrap();
        let session_id = Uuid::new_v4().to_string();
        let target = safe_root.path().join("cancelled-directory");
        let binding = file_tools
            .create_directory_grant_target(target.to_string_lossy().as_ref())
            .unwrap();
        let grant = authority
            .issue_at(&session_id, &binding, current_unix_seconds())
            .unwrap();
        let counters = Arc::new(Mutex::new(AuditCounters::default()));

        let (cancel_waiter, cancelled_worker) = filesystem_mutation_commit_guards();
        drop(cancel_waiter);
        let cancelled = run_create_directory_mutation_worker(
            file_tools.clone(),
            target.to_string_lossy().to_string(),
            authority.clone(),
            Some(grant.clone()),
            session_id.clone(),
            cancelled_worker,
            CreateDirectoryMutationAuditGuard::new(Arc::clone(&counters)),
        );
        assert!(matches!(
            cancelled,
            FilesystemMutationWorkerOutcome::Cancelled
        ));
        assert!(!target.exists());

        let (live_waiter, live_worker) = filesystem_mutation_commit_guards();
        let completed = run_create_directory_mutation_worker(
            file_tools,
            target.to_string_lossy().to_string(),
            authority,
            Some(grant),
            session_id,
            live_worker,
            CreateDirectoryMutationAuditGuard::new(Arc::clone(&counters)),
        );
        live_waiter.complete();
        assert!(matches!(
            completed,
            FilesystemMutationWorkerOutcome::Completed(Ok(_))
        ));
        assert!(target.is_dir());

        let counters = counters.lock().unwrap().clone();
        assert_eq!(counters.by_tool[CREATE_DIRECTORY_TOOL].allowed, 1);
        assert_eq!(counters.by_tool[CREATE_DIRECTORY_TOOL].denied, 1);
        assert_eq!(
            counters.by_reason_code[FILESYSTEM_MUTATION_REQUEST_CANCELLED].denied,
            1
        );
    }

    #[test]
    fn two_distinct_prepared_create_grants_preserve_the_stale_loser() {
        let safe_root = tempfile::tempdir().unwrap();
        let file_tools = FileSystemTools::try_new(vec![safe_root.path().to_path_buf()])
            .expect("test safe root must validate");
        let authority = CreateDirectoryGrantAuthority::from_hex_key(
            "test-key-1",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "stale-create-test-principal",
        )
        .unwrap();
        let session_id = Uuid::new_v4().to_string();
        let target = safe_root.path().join("serialized-directory");
        let path = target.to_string_lossy().to_string();
        let binding = file_tools.create_directory_grant_target(&path).unwrap();
        let now = current_unix_seconds();
        let winner_grant = authority.issue_at(&session_id, &binding, now).unwrap();
        let stale_grant = authority.issue_at(&session_id, &binding, now).unwrap();
        assert_ne!(winner_grant, stale_grant);

        let winner_prepared = file_tools
            .prepare_create_directory_mutation_blocking(path.clone())
            .unwrap();
        let stale_prepared = file_tools
            .prepare_create_directory_mutation_blocking(path.clone())
            .unwrap();
        let counters = Arc::new(Mutex::new(AuditCounters::default()));

        let (winner_waiter, winner_commit) = filesystem_mutation_commit_guards();
        let winner = run_prepared_create_directory_mutation(
            winner_prepared,
            authority.clone(),
            Some(winner_grant),
            session_id.clone(),
            winner_commit,
            CreateDirectoryMutationAuditGuard::new(Arc::clone(&counters)),
        );
        winner_waiter.complete();
        assert!(matches!(
            winner,
            FilesystemMutationWorkerOutcome::Completed(Ok(_))
        ));
        assert!(target.is_dir());

        let (stale_waiter, stale_commit) = filesystem_mutation_commit_guards();
        let stale = run_prepared_create_directory_mutation(
            stale_prepared,
            authority.clone(),
            Some(stale_grant.clone()),
            session_id.clone(),
            stale_commit,
            CreateDirectoryMutationAuditGuard::new(Arc::clone(&counters)),
        );
        stale_waiter.complete();
        assert!(matches!(
            stale,
            FilesystemMutationWorkerOutcome::Completed(Err(
                AuthorizedCreateDirectoryError::Filesystem(AppError::PathAlreadyExists)
            ))
        ));

        std::fs::remove_dir(&target).unwrap();
        let fresh_prepared = file_tools
            .prepare_create_directory_mutation_blocking(path)
            .unwrap();
        let (fresh_waiter, fresh_commit) = filesystem_mutation_commit_guards();
        let fresh = run_prepared_create_directory_mutation(
            fresh_prepared,
            authority.clone(),
            Some(stale_grant.clone()),
            session_id.clone(),
            fresh_commit,
            CreateDirectoryMutationAuditGuard::new(Arc::clone(&counters)),
        );
        fresh_waiter.complete();
        assert!(matches!(
            fresh,
            FilesystemMutationWorkerOutcome::Completed(Ok(_))
        ));
        assert!(target.is_dir());
        assert_eq!(
            authority
                .consume_at(
                    Some(&stale_grant),
                    &session_id,
                    &binding,
                    current_unix_seconds(),
                )
                .unwrap_err(),
            CreateDirectoryGrantError::Replayed
        );

        let counters = counters.lock().unwrap().clone();
        assert_eq!(counters.by_tool[CREATE_DIRECTORY_TOOL].allowed, 2);
        assert_eq!(counters.by_tool[CREATE_DIRECTORY_TOOL].denied, 1);
    }

    #[test]
    fn write_file_cancellation_winner_preserves_grant_and_worker_winner_commits() {
        use crate::write_file_grant::WriteFileDisposition;

        let safe_root = tempfile::tempdir().unwrap();
        let file_tools = FileSystemTools::try_new(vec![safe_root.path().to_path_buf()])
            .expect("test safe root must validate");
        let authority = WriteFileGrantAuthority::from_hex_key(
            "test-key-1",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "cancel-write-test-principal",
        )
        .unwrap();
        let session_id = Uuid::new_v4().to_string();
        let target = safe_root.path().join("cancelled-write.txt");
        let content = "cancelled-write-content";
        let binding = file_tools
            .write_file_grant_target(
                target.to_string_lossy().as_ref(),
                content.as_bytes(),
                WriteFileDisposition::Create,
            )
            .unwrap();
        let grant = authority.issue(&session_id, &binding).unwrap();
        let counters = Arc::new(Mutex::new(AuditCounters::default()));

        let (cancel_waiter, cancelled_worker) = filesystem_mutation_commit_guards();
        drop(cancel_waiter);
        let cancelled = run_write_file_mutation_worker(WriteFileMutationWorker {
            file_tools: file_tools.clone(),
            path: target.to_string_lossy().to_string(),
            content: content.to_owned(),
            authority: authority.clone(),
            capability_grant: Some(grant.clone()),
            session_id: session_id.clone(),
            commit: cancelled_worker,
            audit: WriteFileMutationAuditGuard::new(Arc::clone(&counters)),
        });
        assert!(matches!(
            cancelled,
            FilesystemMutationWorkerOutcome::Cancelled
        ));
        assert!(!target.exists());

        let (live_waiter, live_worker) = filesystem_mutation_commit_guards();
        let completed = run_write_file_mutation_worker(WriteFileMutationWorker {
            file_tools,
            path: target.to_string_lossy().to_string(),
            content: content.to_owned(),
            authority,
            capability_grant: Some(grant),
            session_id,
            commit: live_worker,
            audit: WriteFileMutationAuditGuard::new(Arc::clone(&counters)),
        });
        live_waiter.complete();
        assert!(matches!(
            completed,
            FilesystemMutationWorkerOutcome::Completed(Ok(_))
        ));
        assert_eq!(std::fs::read_to_string(target).unwrap(), content);

        let counters = counters.lock().unwrap().clone();
        assert_eq!(counters.by_tool[WRITE_FILE_TOOL].allowed, 1);
        assert_eq!(counters.by_tool[WRITE_FILE_TOOL].denied, 1);
        assert_eq!(
            counters.by_reason_code[FILESYSTEM_MUTATION_REQUEST_CANCELLED].denied,
            1
        );
    }

    #[test]
    fn cancellation_while_waiting_for_publication_lock_preserves_write_grant() {
        use crate::write_file_grant::WriteFileDisposition;

        let safe_root = tempfile::tempdir().unwrap();
        let waiting_tools = FileSystemTools::try_new(vec![safe_root.path().to_path_buf()])
            .expect("test safe root must validate");
        let retry_tools = FileSystemTools::try_new(vec![safe_root.path().to_path_buf()])
            .expect("test safe root must validate");
        let authority = WriteFileGrantAuthority::from_hex_key(
            "test-key-1",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "waiting-cancel-test-principal",
        )
        .unwrap();
        let session_id = Uuid::new_v4().to_string();
        let target = safe_root.path().join("waiting-cancel.txt");
        let content = "waiting-cancel-content";
        let binding = waiting_tools
            .write_file_grant_target(
                target.to_string_lossy().as_ref(),
                content.as_bytes(),
                WriteFileDisposition::Create,
            )
            .unwrap();
        let grant = authority.issue(&session_id, &binding).unwrap();
        let counters = Arc::new(Mutex::new(AuditCounters::default()));
        let publication_lock = crate::tools::acquire_filesystem_publication_lock_for_test();
        let (waiter, worker_commit) = filesystem_mutation_commit_guards();
        let (contended_tx, contended_rx) = std::sync::mpsc::channel();
        let waiting_counters = Arc::clone(&counters);
        let waiting_authority = authority.clone();
        let waiting_grant = grant.clone();
        let waiting_session = session_id.clone();
        let waiting_path = target.to_string_lossy().to_string();
        let worker = std::thread::spawn(move || {
            run_write_file_mutation_worker_with_lock_contention_hook(
                WriteFileMutationWorker {
                    file_tools: waiting_tools,
                    path: waiting_path,
                    content: content.to_owned(),
                    authority: waiting_authority,
                    capability_grant: Some(waiting_grant),
                    session_id: waiting_session,
                    commit: worker_commit,
                    audit: WriteFileMutationAuditGuard::new(waiting_counters),
                },
                || contended_tx.send(()).unwrap(),
            )
        });
        contended_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("mutation worker did not contend on the held publication lock");
        drop(waiter);
        drop(publication_lock);

        assert!(matches!(
            worker.join().unwrap(),
            FilesystemMutationWorkerOutcome::Cancelled
        ));
        assert!(!target.exists());

        let (retry_waiter, retry_commit) = filesystem_mutation_commit_guards();
        let retry = run_write_file_mutation_worker(WriteFileMutationWorker {
            file_tools: retry_tools,
            path: target.to_string_lossy().to_string(),
            content: content.to_owned(),
            authority,
            capability_grant: Some(grant),
            session_id,
            commit: retry_commit,
            audit: WriteFileMutationAuditGuard::new(Arc::clone(&counters)),
        });
        retry_waiter.complete();
        assert!(matches!(
            retry,
            FilesystemMutationWorkerOutcome::Completed(Ok(_))
        ));
        assert_eq!(std::fs::read_to_string(target).unwrap(), content);

        let counters = counters.lock().unwrap().clone();
        assert_eq!(counters.by_tool[WRITE_FILE_TOOL].allowed, 1);
        assert_eq!(counters.by_tool[WRITE_FILE_TOOL].denied, 1);
        assert_eq!(
            counters.by_reason_code[FILESYSTEM_MUTATION_REQUEST_CANCELLED].denied,
            1
        );
    }

    #[tokio::test]
    async fn create_directory_capacity_denial_is_private_audited_and_does_not_consume_grant() {
        let safe_root = tempfile::tempdir().unwrap();
        let file_tools = FileSystemTools::try_new(vec![safe_root.path().to_path_buf()])
            .expect("test safe root must validate");
        let issuer_tools = file_tools.clone();
        let authority = CreateDirectoryGrantAuthority::from_hex_key(
            "test-key-1",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "capacity-create-test-principal",
        )
        .unwrap();
        let state = McpTransportState::new(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            file_tools,
            false,
            false,
            false,
            Some(authority.clone()),
            None,
        );
        let session_id = Uuid::new_v4().to_string();
        let target = safe_root.path().join("capacity-directory");
        let binding = issuer_tools
            .create_directory_grant_target(target.to_string_lossy().as_ref())
            .unwrap();
        let grant = authority
            .issue_at(&session_id, &binding, current_unix_seconds())
            .unwrap();
        let held_permits = exhaust_mutation_workers(&state);

        let denied = handle_create_directory_call(
            Some(json!("capacity-create")),
            Some(json!({
                "path": target.to_string_lossy(),
                "dry_run": false,
            })),
            &state,
            &session_id,
            Some(&grant),
        )
        .await;
        assert_eq!(denied.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            denied.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-store"))
        );
        assert_eq!(
            denied.headers().get(header::RETRY_AFTER),
            Some(&HeaderValue::from_static("1"))
        );
        let denied = response_json(denied).await;
        assert_eq!(denied["id"], "capacity-create");
        assert_eq!(denied["error"]["code"], -32007);
        assert_eq!(
            denied["error"]["message"],
            "Filesystem mutation capacity unavailable"
        );
        assert!(!target.exists());
        let serialized = denied.to_string();
        assert!(!serialized.contains(&grant));
        assert!(!serialized.contains(target.to_string_lossy().as_ref()));

        drop(held_permits);
        let allowed = handle_create_directory_call(
            Some(json!("capacity-create-retry")),
            Some(json!({
                "path": target.to_string_lossy(),
                "dry_run": false,
            })),
            &state,
            &session_id,
            Some(&grant),
        )
        .await;
        assert_eq!(allowed.status(), StatusCode::OK);
        assert!(target.is_dir());

        let counters = state.audit_counters.lock().unwrap().clone();
        assert_eq!(counters.by_tool[CREATE_DIRECTORY_TOOL].allowed, 1);
        assert_eq!(counters.by_tool[CREATE_DIRECTORY_TOOL].denied, 1);
        assert_eq!(
            counters.by_reason_code[FILESYSTEM_MUTATION_WORKER_CAPACITY_EXCEEDED].denied,
            1
        );
    }

    #[tokio::test]
    async fn write_file_capacity_denial_is_private_audited_and_does_not_consume_grant() {
        use crate::write_file_grant::WriteFileDisposition;

        let safe_root = tempfile::tempdir().unwrap();
        let file_tools = FileSystemTools::try_new(vec![safe_root.path().to_path_buf()])
            .expect("test safe root must validate");
        let issuer_tools = file_tools.clone();
        let authority = WriteFileGrantAuthority::from_hex_key(
            "test-key-1",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "capacity-write-test-principal",
        )
        .unwrap();
        let state = McpTransportState::new(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            file_tools,
            false,
            false,
            false,
            None,
            Some(authority.clone()),
        );
        let session_id = Uuid::new_v4().to_string();
        let target = safe_root.path().join("capacity-write.txt");
        let content = "capacity-write-content";
        let binding = issuer_tools
            .write_file_grant_target(
                target.to_string_lossy().as_ref(),
                content.as_bytes(),
                WriteFileDisposition::Create,
            )
            .unwrap();
        let grant = authority.issue(&session_id, &binding).unwrap();
        let held_permits = exhaust_mutation_workers(&state);

        let denied = handle_write_file_call(
            Some(json!("capacity-write")),
            Some(json!({
                "path": target.to_string_lossy(),
                "content": content,
                "dry_run": false,
            })),
            &state,
            &session_id,
            Some(&grant),
        )
        .await;
        assert_eq!(denied.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            denied.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-store"))
        );
        assert_eq!(
            denied.headers().get(header::RETRY_AFTER),
            Some(&HeaderValue::from_static("1"))
        );
        let denied = response_json(denied).await;
        assert_eq!(denied["id"], "capacity-write");
        assert_eq!(denied["error"]["code"], -32007);
        assert_eq!(
            denied["error"]["message"],
            "Filesystem mutation capacity unavailable"
        );
        assert!(!target.exists());
        let serialized = denied.to_string();
        assert!(!serialized.contains(&grant));
        assert!(!serialized.contains(content));
        assert!(!serialized.contains(target.to_string_lossy().as_ref()));

        drop(held_permits);
        let allowed = handle_write_file_call(
            Some(json!("capacity-write-retry")),
            Some(json!({
                "path": target.to_string_lossy(),
                "content": content,
                "dry_run": false,
            })),
            &state,
            &session_id,
            Some(&grant),
        )
        .await;
        assert_eq!(allowed.status(), StatusCode::OK);
        assert_eq!(std::fs::read_to_string(target).unwrap(), content);

        let counters = state.audit_counters.lock().unwrap().clone();
        assert_eq!(counters.by_tool[WRITE_FILE_TOOL].allowed, 1);
        assert_eq!(counters.by_tool[WRITE_FILE_TOOL].denied, 1);
        assert_eq!(
            counters.by_reason_code[FILESYSTEM_MUTATION_WORKER_CAPACITY_EXCEEDED].denied,
            1
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn live_create_worker_records_exactly_once_after_http_waiter_is_dropped() {
        let safe_root = tempfile::tempdir().unwrap();
        let file_tools = FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).expect("test safe root must validate");
        let issuer_tools = file_tools.clone();
        let authority = CreateDirectoryGrantAuthority::from_hex_key(
            "test-key-1",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "detached-create-test-principal",
        )
        .unwrap();
        let state = McpTransportState::new(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            file_tools,
            false,
            false,
            false,
            Some(authority.clone()),
            None,
        );
        let session_id = Uuid::new_v4().to_string();
        let target = safe_root.path().join("detached-directory");
        let binding = issuer_tools
            .create_directory_grant_target(target.to_string_lossy().as_ref())
            .unwrap();
        let grant = authority
            .issue_at(&session_id, &binding, current_unix_seconds())
            .unwrap();

        // Holding the aggregate lock lets the mutation complete and then
        // deterministically parks its blocking worker at the audit boundary.
        let counters = Arc::clone(&state.audit_counters);
        let counter_blocker = AuditCounterBlocker::acquire(Arc::clone(&counters));
        let state_for_request = state.clone();
        let session_for_request = session_id.clone();
        let grant_for_request = grant.clone();
        let target_for_request = target.clone();
        let request_task = tokio::spawn(async move {
            handle_create_directory_call(
                Some(json!("detached-create")),
                Some(json!({
                    "path": target_for_request.to_string_lossy(),
                    "dry_run": false,
                })),
                &state_for_request,
                &session_for_request,
                Some(&grant_for_request),
            )
            .await
        });

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !target.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "authorized directory mutation did not publish before the test deadline"
            );
            tokio::task::yield_now().await;
        }

        request_task.abort();
        assert!(request_task.await.unwrap_err().is_cancelled());
        counter_blocker.release();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let snapshot = counters.lock().unwrap().clone();
            if snapshot.allowed_total == 1 {
                assert_eq!(snapshot.denied_total, 0);
                assert_eq!(snapshot.by_tool[CREATE_DIRECTORY_TOOL].allowed, 1);
                assert_eq!(snapshot.by_tool[CREATE_DIRECTORY_TOOL].denied, 0);
                assert_eq!(
                    snapshot.by_reason_code[FILESYSTEM_CREATE_ALLOWED].allowed,
                    1
                );
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "detached directory worker did not record its audit outcome"
            );
            tokio::task::yield_now().await;
        }

        assert!(target.is_dir());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn live_write_worker_records_exactly_once_after_http_waiter_is_dropped() {
        use crate::write_file_grant::{WriteFileDisposition, WriteFileGrantError};

        let safe_root = tempfile::tempdir().unwrap();
        let file_tools = FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).expect("test safe root must validate");
        let issuer_tools = file_tools.clone();
        let authority = WriteFileGrantAuthority::from_hex_key(
            "test-key-1",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "detached-write-test-principal",
        )
        .unwrap();
        let state = McpTransportState::new(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            file_tools,
            false,
            false,
            false,
            None,
            Some(authority.clone()),
        );
        let session_id = Uuid::new_v4().to_string();
        let target = safe_root.path().join("detached.txt");
        let content = "detached-content";
        let binding = issuer_tools
            .write_file_grant_target(
                target.to_string_lossy().as_ref(),
                content.as_bytes(),
                WriteFileDisposition::Create,
            )
            .unwrap();
        let grant = authority.issue(&session_id, &binding).unwrap();

        // Holding the aggregate lock lets the mutation complete and then
        // deterministically parks its blocking worker at the audit boundary.
        let counters = Arc::clone(&state.audit_counters);
        let counter_blocker = AuditCounterBlocker::acquire(Arc::clone(&counters));
        let state_for_request = state.clone();
        let session_for_request = session_id.clone();
        let grant_for_request = grant.clone();
        let target_for_request = target.clone();
        let request_task = tokio::spawn(async move {
            handle_write_file_call(
                Some(json!("detached")),
                Some(json!({
                    "path": target_for_request.to_string_lossy(),
                    "content": content,
                    "dry_run": false,
                })),
                &state_for_request,
                &session_for_request,
                Some(&grant_for_request),
            )
            .await
        });

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !target.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "authorized mutation did not publish before the test deadline"
            );
            tokio::task::yield_now().await;
        }

        request_task.abort();
        assert!(request_task.await.unwrap_err().is_cancelled());
        counter_blocker.release();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let snapshot = counters.lock().unwrap().clone();
            if snapshot.allowed_total == 1 {
                assert_eq!(snapshot.denied_total, 0);
                assert_eq!(snapshot.by_tool[WRITE_FILE_TOOL].allowed, 1);
                assert_eq!(snapshot.by_tool[WRITE_FILE_TOOL].denied, 0);
                assert_eq!(
                    snapshot.by_reason_code[FILESYSTEM_WRITE_ALLOWED].allowed,
                    1
                );
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "detached mutation worker did not record its audit outcome"
            );
            tokio::task::yield_now().await;
        }

        assert_eq!(std::fs::read_to_string(target).unwrap(), content);
        assert!(matches!(
            authority.consume(Some(&grant), &session_id, &binding),
            Err(WriteFileGrantError::Replayed)
        ));
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
            FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).expect("test safe root must validate"),
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
            FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).expect("test safe root must validate"),
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
            FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).expect("test safe root must validate"),
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
            FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).expect("test safe root must validate"),
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
            FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).expect("test safe root must validate"),
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
    async fn equivalent_router_authorities_share_replay_state() {
        use axum::{
            body::Body,
            http::{header, Request},
        };
        use tower::ServiceExt;

        const KEY: &str =
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        const PRINCIPAL: &str = "shared-router-replay-principal";
        let (program_root, client) = test_volume_control_client();
        let safe_root = tempfile::tempdir().unwrap();
        let first_authority = AndroidVolumeGrantAuthority::from_hex_key(
            "shared-router-1",
            KEY,
            PRINCIPAL,
        )
        .unwrap();
        let second_authority = AndroidVolumeGrantAuthority::from_hex_key(
            "shared-router-1",
            KEY,
            PRINCIPAL,
        )
        .unwrap();
        let first_state = McpTransportState::with_android_volume_control_client(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            FileSystemTools::try_new(vec![safe_root.path().to_path_buf()])
                .expect("test safe root must validate"),
            Some(first_authority.clone()),
            client.clone(),
        );
        let mut second_state = McpTransportState::with_android_volume_control_client(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            FileSystemTools::try_new(vec![safe_root.path().to_path_buf()])
                .expect("test safe root must validate"),
            Some(second_authority),
            client,
        );
        second_state.sessions = first_state.sessions.clone();
        let session = first_state.sessions.create().unwrap();
        first_state.sessions.activate(&session).unwrap();
        let target = AndroidVolumeGrantTarget::new(AndroidVolumeStreamName::Music, 9).unwrap();
        let grant = first_authority
            .issue_at(&session, target, current_unix_seconds())
            .unwrap();
        let first_router = router_from_state(first_state);
        let second_router = router_from_state(second_state);

        let request = |id: &str| {
            Request::post("/mcp")
                .header(header::HOST, "localhost:8000")
                .header(header::ORIGIN, "http://localhost:8000")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ACCEPT, MCP_POST_ACCEPT)
                .header(MCP_PROTOCOL_VERSION_HEADER, MCP_PROTOCOL_VERSION)
                .header(MCP_SESSION_ID_HEADER, &session)
                .header(REQUEST_GRANT_HEADER, &grant)
                .body(Body::from(
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "method": "tools/call",
                        "params": {
                            "name": SET_ANDROID_VOLUME_TOOL,
                            "arguments": {"stream": "music", "level": 9, "dry_run": false}
                        }
                    })
                    .to_string(),
                ))
                .unwrap()
        };

        let first = first_router.oneshot(request("router-a")).await.unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        let first = response_json(first).await;
        assert_eq!(first["result"]["structuredContent"]["verified"], true);
        assert_eq!(
            std::fs::read_to_string(program_root.path().join("calls")).unwrap(),
            "music:9:/\n"
        );

        let replay = second_router.oneshot(request("router-b")).await.unwrap();
        assert_eq!(replay.status(), StatusCode::FORBIDDEN);
        let replay = response_json(replay).await;
        assert_eq!(
            replay["error"]["data"]["reason"],
            "capability_grant_replayed"
        );
        let serialized = replay.to_string();
        assert!(!serialized.contains(KEY));
        assert!(!serialized.contains(PRINCIPAL));
        assert!(!serialized.contains(&grant));
    }

    #[cfg(feature = "android-volume-control")]
    #[test]
    fn volume_mutation_audit_guard_records_exactly_once_on_error_and_drop() {
        let counters = Arc::new(Mutex::new(AuditCounters::default()));
        let error = Err::<(), _>(AndroidVolumeControlError::SetFailedRollbackConfirmed);
        AndroidVolumeMutationAuditGuard::new(Arc::clone(&counters)).finish(&error);
        drop(AndroidVolumeMutationAuditGuard::new(Arc::clone(&counters)));

        let counters = counters.lock().unwrap().clone();
        assert_eq!(counters.by_tool[SET_ANDROID_VOLUME_TOOL].denied, 2);
        assert_eq!(
            counters.by_reason_code[
                AndroidVolumeControlError::SetFailedRollbackConfirmed.reason_code()
            ]
            .denied,
            1
        );
        assert_eq!(
            counters.by_reason_code[AndroidVolumeControlError::WorkerFailed.reason_code()].denied,
            1
        );
    }

    #[cfg(feature = "android-volume-control")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn live_volume_worker_records_after_http_waiter_is_dropped() {
        let _provider_guard = crate::android_provider::ANDROID_PROVIDER_TEST_LOCK
            .lock()
            .await;
        let (program_root, client) = test_volume_control_client();
        let safe_root = tempfile::tempdir().unwrap();
        let authority = test_volume_authority();
        let state = McpTransportState::with_android_volume_control_client(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            FileSystemTools::try_new(vec![safe_root.path().to_path_buf()])
                .expect("test safe root must validate"),
            Some(authority.clone()),
            client,
        );
        let session = "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee";
        let target = AndroidVolumeGrantTarget::new(AndroidVolumeStreamName::Music, 9).unwrap();
        let grant = authority
            .issue_at(session, target, current_unix_seconds())
            .unwrap();

        let counters = Arc::clone(&state.audit_counters);
        let counter_blocker = AuditCounterBlocker::acquire(Arc::clone(&counters));
        let request_state = state.clone();
        let request_grant = grant.clone();
        let request_task = tokio::spawn(async move {
            handle_set_android_volume_call(
                Some(json!("detached-volume")),
                Some(json!({"stream":"music", "level":9, "dry_run":false})),
                &request_state,
                session,
                Some(&request_grant),
            )
            .await
        });

        let level_path = program_root.path().join("level");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if std::fs::read_to_string(&level_path)
                .is_ok_and(|level| level.trim() == "9")
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "detached volume worker did not complete its mutation"
            );
            tokio::task::yield_now().await;
        }

        request_task.abort();
        assert!(request_task.await.unwrap_err().is_cancelled());
        counter_blocker.release();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let snapshot = counters.lock().unwrap().clone();
            if let Some(counts) = snapshot
                .by_tool
                .get(SET_ANDROID_VOLUME_TOOL)
                .filter(|counts| counts.allowed == 1)
            {
                assert_eq!(counts.denied, 0);
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "detached volume worker did not record its terminal audit"
            );
            tokio::task::yield_now().await;
        }
        assert_eq!(
            authority
                .consume_at(
                    Some(&grant),
                    session,
                    target,
                    current_unix_seconds(),
                )
                .unwrap_err()
                .reason_code(),
            "capability_grant_replayed"
        );
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
            FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).expect("test safe root must validate"),
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
            FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).expect("test safe root must validate"),
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
            FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).expect("test safe root must validate"),
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
            FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).expect("test safe root must validate"),
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
        use axum::body::to_bytes;

        let safe_root = tempfile::tempdir().unwrap();
        let marker = safe_root.path().join("must-not-exist");
        let script = format!("touch '{}'", marker.display());
        let (_program_root, client) = test_command_client(safe_root.path(), &script);
        let state = McpTransportState::with_command_execution_client(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).expect("test safe root must validate"),
            true,
            client,
        );

        for arguments in [
            json!({"profile": "server_version", "command": "sh -c id"}),
            json!({"profile": "server_version", "program": "/private/program"}),
            json!({"profile": "server_version", "argv": ["--private-argument"]}),
            json!({"profile": "server_version", "workingDirectory": "/private/cwd"}),
            json!({"profile": "server_version", "environment": {"TOKEN": "secret-token-value"}}),
            json!({"profile": "server_version", "stdin": "private-stdin"}),
            json!({"profile": "server_version", "timeout": 998877}),
            json!({"profile": "server_version", "stdoutLimit": 998878}),
            json!({"profile": "server_version", "stderrLimit": 998879}),
        ] {
            let response = handle_run_command_profile_call(
                Some(json!("command-invalid")),
                Some(arguments),
                &state,
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let response = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
            let response = String::from_utf8(response.to_vec()).unwrap();
            for private in [
                "sh -c id",
                "/private/program",
                "--private-argument",
                "/private/cwd",
                "secret-token-value",
                "private-stdin",
                "998877",
                "998878",
                "998879",
            ] {
                assert!(!response.contains(private), "response reflected {private}");
            }
        }
        assert!(!marker.exists());
        let counters = state.audit_counters.lock().unwrap().clone();
        assert_eq!(
            counters.by_reason_code[COMMAND_INVALID_ARGUMENTS_REASON].denied,
            9
        );
        let audit = serde_json::to_string(&counters).unwrap();
        for private in [
            "sh -c id",
            "/private/program",
            "--private-argument",
            "/private/cwd",
            "secret-token-value",
            "private-stdin",
            "998877",
            "998878",
            "998879",
        ] {
            assert!(!audit.contains(private), "audit reflected {private}");
        }
    }

    #[cfg(feature = "command-execution")]
    #[tokio::test]
    async fn unapproved_profile_is_rejected_before_spawn() {
        use axum::body::to_bytes;

        let safe_root = tempfile::tempdir().unwrap();
        let marker = safe_root.path().join("must-not-exist");
        let script = format!("touch '{}'", marker.display());
        let (_program_root, client) = test_command_client(safe_root.path(), &script);
        let state = McpTransportState::with_command_execution_client(
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            FileSystemTools::try_new(vec![safe_root.path().to_path_buf()]).expect("test safe root must validate"),
            true,
            client,
        );

        let response = handle_run_command_profile_call(
            Some(json!("command-unapproved")),
            Some(json!({"profile": "sh -c private-command"})),
            &state,
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let response = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        assert!(!String::from_utf8(response.to_vec())
            .unwrap()
            .contains("private-command"));
        assert!(!marker.exists());
        assert_eq!(
            state.audit_counters.lock().unwrap().by_reason_code
                [COMMAND_PROFILE_NOT_ALLOWLISTED_REASON]
                .denied,
            1
        );
    }
}
