//! Integration tests for the current Termux MCP Edge runtime posture.

use termux_mcp_server::tools::{FileSystemTools, SystemTools};

#[test]
fn filesystem_tools_preserve_anchored_safe_roots_without_transport_exposure() {
    let root = tempfile::tempdir().unwrap();
    let anchored = root.path().canonicalize().unwrap();
    let tools =
        FileSystemTools::try_new(vec![anchored.clone()]).expect("test safe root must validate");

    assert_eq!(tools.safe_roots(), &[anchored]);
}

#[test]
fn system_tools_instantiation_is_zero_state() {
    let system = SystemTools;
    let cloned = system.clone();
    assert_eq!(std::mem::size_of_val(&cloned), 0);
}
