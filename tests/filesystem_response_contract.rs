#![cfg(feature = "mcp-runtime")]

mod support;

use axum::{body::to_bytes, http::StatusCode};
use serde_json::{json, Value};
use support::{initialize_session, post_json_to_session, test_router};
use termux_mcp_server::{
    error::AppError,
    tools::{
        FileSystemTools, MAX_LIST_ENTRIES, MAX_LIST_RESPONSE_BYTES, MAX_READ_BYTES,
        MAX_READ_RESPONSE_BYTES,
    },
};

fn tool_call(id: &str, name: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": name,
            "arguments": arguments,
        }
    })
}

#[tokio::test]
async fn list_directory_is_deterministic_and_byte_bounded() {
    let root = tempfile::tempdir().unwrap();
    for index in (0..1_500).rev() {
        let name = format!("{index:04}-{}", "x".repeat(180));
        tokio::fs::write(root.path().join(name), b"x")
            .await
            .unwrap();
    }
    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
    let path = root.path().to_string_lossy().to_string();

    let first = tools.list_directory(path.clone(), Some(1)).await.unwrap();
    let second = tools.list_directory(path.clone(), Some(1)).await.unwrap();
    let first_value = serde_json::to_value(&first).unwrap();
    let second_value = serde_json::to_value(&second).unwrap();
    let paths: Vec<&str> = first
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    let mut sorted_paths = paths.clone();
    sorted_paths.sort_unstable();

    assert_eq!(first_value, second_value);
    assert_eq!(paths, sorted_paths);
    assert!(first.truncated);
    assert!(first.entries.len() < 1_500);
    assert!(first.entries.len() <= MAX_LIST_ENTRIES);
    assert_eq!(first.max_entries, MAX_LIST_ENTRIES);
    assert_eq!(first.max_response_bytes, MAX_LIST_RESPONSE_BYTES);
    assert!(serde_json::to_vec(&first).unwrap().len() < MAX_LIST_RESPONSE_BYTES);

    let router = test_router(tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router,
        &session_id,
        tool_call(
            "bounded-list",
            "list_directory",
            json!({"path": path, "max_depth": 1}),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), MAX_LIST_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert!(body.len() <= MAX_LIST_RESPONSE_BYTES);
    assert_eq!(payload["result"]["structuredContent"]["truncated"], true);
}

#[tokio::test]
async fn read_file_accepts_exact_limit_and_rejects_the_next_byte() {
    let root = tempfile::tempdir().unwrap();
    let exact = root.path().join("exact.txt");
    let oversized = root.path().join("oversized.txt");
    tokio::fs::write(&exact, vec![b'a'; MAX_READ_BYTES])
        .await
        .unwrap();
    tokio::fs::write(&oversized, vec![b'a'; MAX_READ_BYTES + 1])
        .await
        .unwrap();
    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

    let result = tools
        .read_file(exact.to_string_lossy().to_string())
        .await
        .unwrap();
    assert_eq!(result.size, MAX_READ_BYTES);
    assert_eq!(result.content.len(), MAX_READ_BYTES);

    let error = tools
        .read_file(oversized.to_string_lossy().to_string())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AppError::FileTooLarge { size, max_size }
            if size == (MAX_READ_BYTES + 1) as u64 && max_size == MAX_READ_BYTES as u64
    ));
}

#[tokio::test]
async fn read_file_rejects_invalid_utf8_explicitly() {
    let root = tempfile::tempdir().unwrap();
    let invalid = root.path().join("invalid.bin");
    tokio::fs::write(&invalid, [0xff, 0xfe, 0xfd])
        .await
        .unwrap();
    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

    let error = tools
        .read_file(invalid.to_string_lossy().to_string())
        .await
        .unwrap_err();

    assert!(matches!(error, AppError::InvalidFileEncoding));
}

#[tokio::test]
async fn transport_returns_file_content_once_with_a_bounded_summary() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("visible.txt");
    let marker = "unique-file-content-marker";
    tokio::fs::write(&file, marker).await.unwrap();
    let router = test_router(FileSystemTools::new(vec![root.path().to_path_buf()]));
    let session_id = initialize_session(&router).await;

    let response = post_json_to_session(
        router,
        &session_id,
        tool_call(
            "read-once",
            "read_file",
            json!({"path": file.to_string_lossy()}),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), MAX_READ_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let summary = payload["result"]["content"][0]["text"].as_str().unwrap();

    assert!(body.len() <= MAX_READ_RESPONSE_BYTES);
    assert_eq!(payload["result"]["structuredContent"]["content"], marker);
    assert_eq!(payload["result"]["structuredContent"]["size"], marker.len());
    assert_eq!(
        body.windows(marker.len())
            .filter(|window| *window == marker.as_bytes())
            .count(),
        1
    );
    assert_eq!(
        summary,
        format!("Read {} UTF-8 bytes from a safe-rooted file.", marker.len())
    );
}

#[tokio::test]
async fn transport_rejects_json_expansion_beyond_read_response_budget() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("escaped.txt");
    tokio::fs::write(&file, vec![0_u8; 200_000]).await.unwrap();
    let router = test_router(FileSystemTools::new(vec![root.path().to_path_buf()]));
    let session_id = initialize_session(&router).await;

    let response = post_json_to_session(
        router,
        &session_id,
        tool_call(
            "expanded-read",
            "read_file",
            json!({"path": file.to_string_lossy()}),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = to_bytes(response.into_body(), MAX_READ_RESPONSE_BYTES)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["id"], "expanded-read");
    assert_eq!(payload["error"]["code"], -32001);
    assert_eq!(
        payload["error"]["data"],
        "File content exceeds the staged read_file response byte limit."
    );
}
