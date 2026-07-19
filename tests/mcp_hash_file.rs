#![cfg(feature = "mcp-runtime")]

mod support;

use axum::{body::to_bytes, http::StatusCode};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use support::{empty_test_file_tools, initialize_session, post_json_to_session, test_router};
use termux_mcp_server::{
    error::AppError,
    tools::{FileSystemTools, MAX_HASH_FILE_BYTES, MAX_HASH_FILE_RESPONSE_BYTES},
};

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn hash_call(id: Value, path: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "hash_file",
            "arguments": {"path": path},
        },
    })
}

#[tokio::test]
async fn discovery_advertises_one_closed_hash_file_schema() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router,
        &session_id,
        json!({
            "jsonrpc": "2.0",
            "id": "hash-tools",
            "method": "tools/list"
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 128 * 1024).await.unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let tools = payload["result"]["tools"].as_array().unwrap();
    assert_eq!(
        tools
            .iter()
            .filter(|tool| tool["name"] == "hash_file")
            .count(),
        1
    );
    let schema = &tools
        .iter()
        .find(|tool| tool["name"] == "hash_file")
        .unwrap()["inputSchema"];
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["required"], json!(["path"]));
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"].as_object().unwrap().len(), 1);
    assert_eq!(schema["properties"]["path"]["type"], "string");
}

#[tokio::test]
async fn hash_file_returns_exact_binary_digest_without_content_or_path() {
    let (root, file_tools) = empty_test_file_tools();
    let path = root.path().join("private-binary.bin");
    let empty_path = root.path().join("empty.bin");
    let bytes = [0_u8, 0xff, 0x80, b'a', b'\n', 0x01, 0xfe];
    std::fs::write(&path, bytes).unwrap();
    std::fs::write(&empty_path, []).unwrap();

    let direct = file_tools
        .hash_file(path.to_string_lossy().to_string())
        .await
        .unwrap();
    assert_eq!(direct.algorithm, "sha256");
    assert_eq!(direct.digest, digest(&bytes));
    assert_eq!(direct.size_bytes, bytes.len());
    let empty = file_tools
        .hash_file(empty_path.to_string_lossy().to_string())
        .await
        .unwrap();
    assert_eq!(empty.algorithm, "sha256");
    assert_eq!(empty.digest, digest(&[]));
    assert_eq!(empty.size_bytes, 0);

    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router,
        &session_id,
        hash_call(json!("hash-binary"), path.to_string_lossy().as_ref()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), MAX_HASH_FILE_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(body.len() <= MAX_HASH_FILE_RESPONSE_BYTES);
    let body_text = std::str::from_utf8(&body).unwrap();
    assert!(!body_text.contains("private-binary"));
    assert!(!body_text.contains(path.to_string_lossy().as_ref()));
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let structured = &payload["result"]["structuredContent"];
    assert_eq!(structured["algorithm"], "sha256");
    assert_eq!(structured["digest"], digest(&bytes));
    assert_eq!(structured["sizeBytes"], bytes.len());
    assert_eq!(structured.as_object().unwrap().len(), 3);
}

#[tokio::test]
async fn hash_file_accepts_exact_limit_and_rejects_one_byte_over() {
    let root = tempfile::tempdir().unwrap();
    let exact_path = root.path().join("exact.bin");
    let oversized_path = root.path().join("oversized.bin");
    let exact = vec![0xa5; MAX_HASH_FILE_BYTES];
    std::fs::write(&exact_path, &exact).unwrap();
    std::fs::write(&oversized_path, vec![0x5a; MAX_HASH_FILE_BYTES + 1]).unwrap();
    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

    let result = tools
        .hash_file(exact_path.to_string_lossy().to_string())
        .await
        .unwrap();
    assert_eq!(result.size_bytes, MAX_HASH_FILE_BYTES);
    assert_eq!(result.digest, digest(&exact));

    let error = tools
        .hash_file(oversized_path.to_string_lossy().to_string())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AppError::FileTooLarge { size, max_size }
            if size == (MAX_HASH_FILE_BYTES + 1) as u64
                && max_size == MAX_HASH_FILE_BYTES as u64
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn hash_file_rejects_missing_outside_symlinked_and_unsupported_targets() {
    use std::os::unix::{fs::symlink, net::UnixListener};

    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_file = outside.path().join("outside.bin");
    std::fs::write(&outside_file, b"outside").unwrap();
    let link = root.path().join("link.bin");
    symlink(&outside_file, &link).unwrap();
    let socket = root.path().join("socket");
    let _listener = UnixListener::bind(&socket).unwrap();
    let linked_parent = root.path().join("linked-parent");
    symlink(outside.path(), &linked_parent).unwrap();
    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

    assert!(matches!(
        tools
            .hash_file(root.path().join("missing").to_string_lossy().to_string())
            .await,
        Err(AppError::PathNotFound)
    ));
    for target in [outside_file, link, linked_parent.join("outside.bin")] {
        assert!(matches!(
            tools.hash_file(target.to_string_lossy().to_string()).await,
            Err(AppError::PathTraversal { .. })
        ));
    }
    for target in [root.path().to_path_buf(), socket] {
        assert!(matches!(
            tools.hash_file(target.to_string_lossy().to_string()).await,
            Err(AppError::UnsupportedPathType) | Err(AppError::PathTraversal { .. })
        ));
    }
}

#[tokio::test]
async fn transport_hash_errors_and_audit_counters_are_bounded_and_private() {
    let (root, file_tools) = empty_test_file_tools();
    let path = root.path().join("private-target.bin");
    let bytes = b"private-content-marker";
    std::fs::write(&path, bytes).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    for (index, arguments) in [
        None,
        Some(json!({"path": path.to_string_lossy(), "unexpected": true})),
        Some(json!({"path": false})),
    ]
    .into_iter()
    .enumerate()
    {
        let params = match arguments {
            Some(arguments) => json!({"name": "hash_file", "arguments": arguments}),
            None => json!({"name": "hash_file"}),
        };
        let response = post_json_to_session(
            router.clone(),
            &session_id,
            json!({
                "jsonrpc": "2.0",
                "id": format!("invalid-{index}"),
                "method": "tools/call",
                "params": params,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    let allowed = post_json_to_session(
        router.clone(),
        &session_id,
        hash_call(json!("allowed"), path.to_string_lossy().as_ref()),
    )
    .await;
    assert_eq!(allowed.status(), StatusCode::OK);
    let denied = post_json_to_session(
        router.clone(),
        &session_id,
        hash_call(json!("denied"), outside.path().to_string_lossy().as_ref()),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::BAD_REQUEST);

    std::fs::remove_file(&path).unwrap();
    let oversized_id = "x".repeat(MAX_HASH_FILE_RESPONSE_BYTES);
    let oversized = post_json_to_session(
        router.clone(),
        &session_id,
        hash_call(json!(oversized_id), path.to_string_lossy().as_ref()),
    )
    .await;
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = to_bytes(oversized.into_body(), MAX_HASH_FILE_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(body.len() <= MAX_HASH_FILE_RESPONSE_BYTES);
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["id"], Value::Null);
    assert_eq!(payload["error"]["code"], -32001);

    let status = post_json_to_session(
        router,
        &session_id,
        json!({
            "jsonrpc": "2.0",
            "id": "status",
            "method": "tools/call",
            "params": {"name": "runtime_status", "arguments": {}}
        }),
    )
    .await;
    assert_eq!(status.status(), StatusCode::OK);
    let body = to_bytes(status.into_body(), 64 * 1024).await.unwrap();
    let text = std::str::from_utf8(&body).unwrap();
    for forbidden in [
        "private-target",
        "private-content-marker",
        path.to_string_lossy().as_ref(),
        &digest(bytes),
    ] {
        assert!(!text.contains(forbidden));
    }
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let runtime = &payload["result"]["structuredContent"];
    assert_eq!(runtime["fileHashing"], true);
    assert_eq!(runtime["fileHashAlgorithm"], "sha256");
    assert_eq!(runtime["fileHashMaxBytes"], MAX_HASH_FILE_BYTES);
    let counters = &runtime["auditCounters"];
    assert_eq!(counters["by_tool"]["hash_file"]["allowed"], 1);
    assert_eq!(counters["by_tool"]["hash_file"]["denied"], 5);
    assert_eq!(
        counters["by_reason_code"]["safe_root_file_hashed"]["allowed"],
        1
    );
    assert_eq!(
        counters["by_reason_code"]["safe_root_rejected"]["denied"],
        1
    );
    assert_eq!(
        counters["by_reason_code"]["response_size_limit_exceeded"]["denied"],
        1
    );
}
