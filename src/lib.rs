//! Library exports for integration tests and downstream embedding.
//! Platform metadata remains non-sensitive and read-only at the transport boundary.

pub mod config;
pub mod error;
pub mod health;
#[cfg(feature = "mcp-runtime")]
pub mod mcp_transport;
pub mod platform_info;
pub mod tools;
pub mod transport_security;
pub mod write_policy;
