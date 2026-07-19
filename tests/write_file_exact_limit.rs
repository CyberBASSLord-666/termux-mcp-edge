#![cfg(feature = "mcp-runtime")]

use termux_mcp_server::write_policy::DEFAULT_MAX_WRITE_BYTES;
use termux_mcp_server::{
    error::AppError,
    tools::{FileSystemTools, WRITE_FILE_MODE},
    write_file_grant::WriteFileDisposition,
};

#[tokio::test]
async fn direct_write_file_api_rejects_live_mutation_without_request_grant() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("exact-limit.txt");
    let content = "x".repeat(DEFAULT_MAX_WRITE_BYTES);
    let tools = FileSystemTools::try_new(vec![root.path().to_path_buf()])
        .expect("test safe root must validate");

    let result = tools
        .write_file(target.to_string_lossy().to_string(), content, Some(false))
        .await;

    assert!(matches!(
        result,
        Err(AppError::WriteMutationAuthorizationRequired)
    ));
    assert_eq!(WRITE_FILE_MODE, 0o600);
    assert!(!target.exists());
}

#[tokio::test]
async fn write_file_allows_exact_default_payload_limit_with_dry_run() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("exact-limit-dry-run.txt");
    let content = "x".repeat(DEFAULT_MAX_WRITE_BYTES);
    let tools = FileSystemTools::try_new(vec![root.path().to_path_buf()])
        .expect("test safe root must validate");

    let result = tools
        .write_file(target.to_string_lossy().to_string(), content, None)
        .await
        .unwrap();

    assert!(result.dry_run);
    assert_eq!(result.size_bytes, DEFAULT_MAX_WRITE_BYTES);
    assert_eq!(result.disposition, WriteFileDisposition::Create);
    assert_eq!(result.mode, "0600");
    assert!(!target.exists());
}
