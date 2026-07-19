//! Library exports for integration tests and downstream embedding.

#![recursion_limit = "256"]

#[cfg(feature = "android-battery-status")]
pub mod android_battery;
#[cfg(any(feature = "android-battery-status", feature = "android-volume-status"))]
mod android_provider;
pub mod android_status;
#[cfg(feature = "android-volume-status")]
pub mod android_volume;
#[cfg(feature = "android-volume-control")]
pub mod android_volume_control;
#[cfg(feature = "android-volume-control")]
pub mod android_volume_grant;
pub mod audit;
pub mod auth;
#[cfg(any(
    feature = "android-battery-status",
    feature = "android-volume-status",
    feature = "android-volume-control",
    feature = "command-execution"
))]
mod bounded_process;
pub mod capability_token;
/// Raw process-client construction and execution are intentionally not part of
/// the downstream embedding API.
#[cfg(feature = "command-execution")]
mod command_execution;
/// The public command-policy surface exposes only closed profile identifiers;
/// profile construction remains policy-owned.
///
/// ```compile_fail
/// use termux_mcp_server::command_policy::CommandProfile;
/// ```
///
/// Raw execution construction is likewise inaccessible:
///
/// ```compile_fail
/// use termux_mcp_server::command_execution::CommandExecutionClient;
/// ```
///
/// Resolved policy handles remain opaque:
///
/// ```compile_fail
/// use termux_mcp_server::command_policy::CommandExecutionPolicy;
/// let decision = CommandExecutionPolicy::new().evaluate("server_version", true, true);
/// let _ = decision.profile;
/// ```
pub mod command_policy;
pub mod config;
#[cfg(feature = "mcp-runtime")]
pub mod create_directory_grant;
pub mod error;
pub mod health;
pub mod json_rpc;
#[cfg(feature = "mcp-runtime")]
mod mcp_session;
#[cfg(feature = "mcp-runtime")]
pub mod mcp_transport;
pub mod platform_info;
#[cfg(feature = "mcp-runtime")]
mod request_grant_capability;
pub mod request_limits;
pub mod service_status;
pub mod tools;
pub mod transport_security;
#[cfg(feature = "mcp-runtime")]
pub mod write_file_grant;
pub mod write_policy;
