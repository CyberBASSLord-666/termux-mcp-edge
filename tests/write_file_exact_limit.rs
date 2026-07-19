#![cfg(feature = "mcp-runtime")]

use termux_mcp_server::write_policy::DEFAULT_MAX_WRITE_BYTES;
use termux_mcp_server::{error::AppError, tools::FileSystemTools};

const EXPECTED_DRY_RUN_RESPONSE: &str = "DRY-RUN";

#[tokio::test]
async fn direct_write_file_rejects_exact_limit_mutation_without_request_authorization() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("exact-limit.txt");
    let content = "x".repeat(DEFAULT_MAX_WRITE_BYTES);
    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

    let result = tools
        .write_file(target.to_string_lossy().to_string(), content, Some(false))
        .await;

    assert!(matches!(
        result,
        Err(AppError::WriteMutationAuthorizationRequired)
    ));
    assert!(!target.exists());
}

#[tokio::test]
async fn write_file_allows_exact_default_payload_limit_with_dry_run() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("exact-limit-dry-run.txt");
    let content = "x".repeat(DEFAULT_MAX_WRITE_BYTES);
    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

    let result = tools
        .write_file(target.to_string_lossy().to_string(), content, None)
        .await
        .unwrap();

    assert_eq!(result, EXPECTED_DRY_RUN_RESPONSE);
    assert!(!target.exists());
}
