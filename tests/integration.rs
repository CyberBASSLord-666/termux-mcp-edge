//! Integration tests for the current Termux MCP Edge runtime posture.

use std::path::PathBuf;

use termux_mcp_server::tools::{FileSystemTools, SystemTools};

#[test]
fn filesystem_tools_preserve_configured_safe_roots_without_transport_exposure() {
    let root = PathBuf::from("/storage/emulated/0/Documents");
    let tools = FileSystemTools::new(vec![root.clone()]);

    assert_eq!(tools.safe_roots(), &[root]);
}

#[test]
fn system_tools_instantiation_is_zero_state() {
    let system = SystemTools;
    let cloned = system.clone();
    assert_eq!(std::mem::size_of_val(&cloned), 0);
}
