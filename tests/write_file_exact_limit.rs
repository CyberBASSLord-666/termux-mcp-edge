#![cfg(feature = "mcp-runtime")]

use termux_mcp_server::tools::FileSystemTools;
use termux_mcp_server::write_policy::DEFAULT_MAX_WRITE_BYTES;

#[tokio::test]
async fn write_file_allows_exact_default_payload_limit_with_explicit_mutation() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("exact-limit.txt");
    let content = "x".repeat(DEFAULT_MAX_WRITE_BYTES);
    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

    let result = tools
        .write_file(
            target.to_string_lossy().to_string(),
            content,
            Some(false),
        )
        .await
        .unwrap();

    assert_eq!(result, format!("Wrote {DEFAULT_MAX_WRITE_BYTES} bytes"));
    assert_eq!(tokio::fs::metadata(&target).await.unwrap().len(), DEFAULT_MAX_WRITE_BYTES as u64);
}
