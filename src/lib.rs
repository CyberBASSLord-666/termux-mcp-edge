//! Library exports for integration tests and downstream embedding.

#[cfg(feature = "android-battery-status")]
pub mod android_battery_status;
pub mod android_status;
pub mod audit;
pub mod auth;
pub mod capability_token;
pub mod command_policy;
pub mod config;
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
