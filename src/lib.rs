//! Library exports for integration tests and downstream embedding.

pub mod config;
pub mod error;
#[cfg(feature = "mcp-runtime")]
pub mod mcp_transport;
pub mod tools;
pub mod transport_security;
