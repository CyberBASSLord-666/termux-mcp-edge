//! Library exports for integration tests and downstream embedding.

pub mod android_status;
pub mod audit;
pub mod command_policy;
pub mod config;
pub mod error;
pub mod health;
#[cfg(feature = "mcp-runtime")]
pub mod mcp_transport;
pub mod platform_info;
pub mod service_status;
pub mod tools;
pub mod transport_security;
pub mod write_policy;
