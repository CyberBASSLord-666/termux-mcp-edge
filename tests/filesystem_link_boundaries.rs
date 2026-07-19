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

#[tokio::test]
async fn create_list_hash_metadata_read_and_search_reject_symlinked_parent_components() {
    let root = tempfile::tempdir().unwrap();
    let peer = tempfile::tempdir().unwrap();
    let peer_file = peer.path().join("peer.txt");
    tokio::fs::write(&peer_file, "peer data").await.unwrap();
    let linked_parent = root.path().join("linked-parent");
    std::os::unix::fs::symlink(peer.path(), &linked_parent).unwrap();
    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

    let list_result = tools
        .list_directory(linked_parent.to_string_lossy().to_string(), Some(1))
        .await;
    let create_result = tools
        .create_directory(
            linked_parent.join("created").to_string_lossy().to_string(),
            Some(true),
        )
        .await;
    let read_result = tools
        .read_file(linked_parent.join("peer.txt").to_string_lossy().to_string())
        .await;
    let hash_result = tools
        .hash_file(linked_parent.join("peer.txt").to_string_lossy().to_string())
        .await;
    let metadata_result = tools
        .path_metadata(linked_parent.join("peer.txt").to_string_lossy().to_string())
        .await;
    let search_result = tools
        .search_text(
            linked_parent.to_string_lossy().to_string(),
            "peer".to_string(),
            Some(1),
        )
        .await;

    assert_path_traversal(create_result);
    assert_path_traversal(list_result);
    assert_path_traversal(hash_result);
    assert_path_traversal(metadata_result);
    assert_path_traversal(read_result);
    assert_path_traversal(search_result);
    assert!(!peer.path().join("created").exists());
}
