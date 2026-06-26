//! Lightweight mock-client style coverage for direct tool invocation.

use std::time::{Duration, Instant};

use termux_mcp_server::tools::FileSystemTools;

#[tokio::test]
async fn filesystem_tool_can_be_invoked_directly_with_safe_paths() {
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let root = temp_dir.path().canonicalize().expect("canonicalize temp root");
    let tools = FileSystemTools::new(vec![root.clone()]);
    let target = root.join("mock-client.txt");

    let write_result = tools
        .write_file(
            target.to_string_lossy().into_owned(),
            "mock client payload".to_string(),
            None,
        )
        .await
        .expect("direct write should succeed");

    assert_eq!(write_result, "Wrote 19 bytes");

    let read_result = tools
        .read_file(target.to_string_lossy().into_owned())
        .await
        .expect("direct read should succeed");

    assert_eq!(read_result.content, "mock client payload");
}

#[tokio::test]
async fn async_runtime_timer_advances_for_latency_sensitive_tests() {
    let start = Instant::now();
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(start.elapsed() >= Duration::from_millis(10));
}
