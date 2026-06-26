//! Integration tests for Termux MCP Server v5.

use termux_mcp_server::tools::{FileSystemTools, SystemTools};

#[tokio::test]
async fn filesystem_sanitize_read_write_and_list_are_safe_rooted() {
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let root = temp_dir
        .path()
        .canonicalize()
        .expect("canonicalize temp root");
    let tools = FileSystemTools::new(vec![root.clone()]);

    let file_path = root.join("notes.txt");
    let dry_run = tools
        .write_file(
            file_path.to_string_lossy().into_owned(),
            "ignored".to_string(),
            Some(true),
        )
        .await
        .expect("dry-run write succeeds");
    assert_eq!(dry_run, "DRY-RUN");
    assert!(
        !file_path.exists(),
        "dry-run must not create the target file"
    );

    let write_result = tools
        .write_file(
            file_path.to_string_lossy().into_owned(),
            "hello".to_string(),
            Some(false),
        )
        .await
        .expect("write succeeds");
    assert_eq!(write_result, "Wrote 5 bytes");

    let read_result = tools
        .read_file(file_path.to_string_lossy().into_owned())
        .await
        .expect("read succeeds");
    assert_eq!(read_result.content, "hello");
    assert_eq!(read_result.size, 5);

    let listing = tools
        .list_directory(root.to_string_lossy().into_owned(), Some(1))
        .await
        .expect("list succeeds");
    assert!(
        listing
            .entries
            .iter()
            .any(|entry| entry.path.ends_with("notes.txt")),
        "listing should include the file written by the test"
    );

    assert!(tools.sanitize("relative/path.txt").is_err());
    assert!(tools.sanitize("/etc/passwd").is_err());
    assert!(tools
        .sanitize(&format!("{}/../escape.txt", root.display()))
        .is_err());
}

#[tokio::test]
async fn filesystem_write_rejects_exact_safe_root_target() {
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let root = temp_dir
        .path()
        .canonicalize()
        .expect("canonicalize temp root");
    let tools = FileSystemTools::new(vec![root.clone()]);

    let result = tools
        .write_file(
            root.to_string_lossy().into_owned(),
            "blocked".to_string(),
            Some(true),
        )
        .await;

    assert!(
        result.is_err(),
        "safe root itself is not a writable file target"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn filesystem_listing_skips_broken_symlinks() {
    use std::os::unix::fs::symlink;

    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let root = temp_dir
        .path()
        .canonicalize()
        .expect("canonicalize temp root");
    let tools = FileSystemTools::new(vec![root.clone()]);
    let broken_link = root.join("broken-link");

    symlink(root.join("missing-target"), &broken_link).expect("create broken symlink");

    let listing = tools
        .list_directory(root.to_string_lossy().into_owned(), Some(1))
        .await
        .expect("listing should skip broken symlinks");

    assert!(
        listing
            .entries
            .iter()
            .all(|entry| entry.path != broken_link.to_string_lossy()),
        "broken symlink should not be returned as a safe file entry"
    );
}

#[tokio::test]
async fn system_tools_instantiation_is_zero_state() {
    let system = SystemTools::default();
    let cloned = system.clone();
    assert_eq!(std::mem::size_of_val(&cloned), 0);
}
