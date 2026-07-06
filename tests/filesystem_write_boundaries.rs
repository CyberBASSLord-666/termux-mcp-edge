#![cfg(all(unix, feature = "mcp-runtime"))]

use termux_mcp_server::{error::AppError, tools::FileSystemTools};

#[tokio::test]
async fn write_file_rejects_link_resolving_beyond_safe_root() {
    let root = tempfile::tempdir().unwrap();
    let peer = tempfile::tempdir().unwrap();
    let peer_file = peer.path().join("peer.txt");
    tokio::fs::write(&peer_file, "original peer data")
        .await
        .unwrap();
    let link_path = root.path().join("peer-link.txt");
    std::os::unix::fs::symlink(&peer_file, &link_path).unwrap();

    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
    let result = tools
        .write_file(
            link_path.to_string_lossy().to_string(),
            "replacement data".to_string(),
            Some(false),
        )
        .await;

    assert!(matches!(result, Err(AppError::PathTraversal { .. })));
    assert_eq!(
        tokio::fs::read_to_string(peer_file).await.unwrap(),
        "original peer data"
    );
}
