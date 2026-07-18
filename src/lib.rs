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
#[cfg(feature = "command-execution")]
pub mod command_execution;
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
pub mod request_limits;
pub mod service_status;
pub mod tools;
pub mod transport_security;
pub mod write_policy;
