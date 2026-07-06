#![cfg(all(unix, feature = "mcp-runtime"))]

use termux_mcp_server::{error::AppError, tools::FileSystemTools};

fn assert_path_traversal<T>(result: Result<T, AppError>) {
    assert!(
        matches!(result, Err(AppError::PathTraversal { .. })),
        "expected filesystem boundary rejection"
    );
}

#[test]
fn sanitize_rejects_link_resolving_beyond_safe_root() {
    let root = tempfile::tempdir().unwrap();
    let peer = tempfile::tempdir().unwrap();
    let peer_file = peer.path().join("peer.txt");
    std::fs::write(&peer_file, "peer data").unwrap();
    let link_path = root.path().join("peer-link.txt");
    std::os::unix::fs::symlink(&peer_file, &link_path).unwrap();

    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

    assert_path_traversal(tools.sanitize(link_path.to_string_lossy().as_ref()));
}

#[tokio::test]
async fn read_file_rejects_link_resolving_beyond_safe_root() {
    let root = tempfile::tempdir().unwrap();
    let peer = tempfile::tempdir().unwrap();
    let peer_file = peer.path().join("peer.txt");
    tokio::fs::write(&peer_file, "peer data").await.unwrap();
    let link_path = root.path().join("peer-link.txt");
    std::os::unix::fs::symlink(&peer_file, &link_path).unwrap();

    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
    let result = tools
        .read_file(link_path.to_string_lossy().to_string())
        .await;

    assert!(matches!(result, Err(AppError::PathTraversal { .. })));
}
