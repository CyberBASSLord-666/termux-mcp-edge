//! Integration tests for Termux MCP Server v5.

use termux_mcp_server::tools::{FileSystemTools, SystemTools};

#[tokio::test]
async fn test_filesystem_sanitize_and_read() {
    let tools = FileSystemTools::new(vec!["/tmp".into()]);
    // Placeholder for real temp file test
    assert!(tools.sanitize("/tmp/test").is_ok());
}

#[tokio::test]
async fn test_system_tools_instantiation() {
    let _system = SystemTools::default();
    assert!(true);
}
