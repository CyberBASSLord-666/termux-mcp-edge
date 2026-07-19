#![cfg(feature = "mcp-runtime")]

mod support;

use axum::{body::to_bytes, http::StatusCode};
use serde_json::{json, Value};
use support::{empty_test_file_tools, initialize_session, post_json_to_session, test_router};
use termux_mcp_server::{
    error::AppError,
    tools::{
        FileSystemTools, MAX_BINARY_READ_BASE64_BYTES, MAX_BINARY_READ_BYTES,
        MAX_BINARY_READ_RESPONSE_BYTES,
    },
};

fn binary_read_call(id: Value, path: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "read_binary_file",
            "arguments": {"path": path},
        },
    })
}

#[tokio::test]
async fn discovery_advertises_one_closed_binary_read_schema() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router,
        &session_id,
        json!({
            "jsonrpc": "2.0",
            "id": "binary-tools",
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
            .filter(|tool| tool["name"] == "read_binary_file")
            .count(),
        1
    );
    let schema = &tools
        .iter()
        .find(|tool| tool["name"] == "read_binary_file")
        .unwrap()["inputSchema"];
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["required"], json!(["path"]));
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"].as_object().unwrap().len(), 1);
    assert_eq!(schema["properties"]["path"]["type"], "string");
}

#[tokio::test]
async fn binary_read_returns_canonical_base64_without_path_or_host_metadata() {
    let (root, file_tools) = empty_test_file_tools();
    let path = root.path().join("private-binary.bin");
    let empty_path = root.path().join("empty.bin");
    let bytes = [0_u8, 0xff, 0x80, b'a', b'\n', 0x01, 0xfe];
    std::fs::write(&path, bytes).unwrap();
    std::fs::write(&empty_path, []).unwrap();

    let direct = file_tools
        .read_binary_file(path.to_string_lossy().to_string())
        .await
        .unwrap();
    assert_eq!(direct.encoding, "base64");
    assert_eq!(direct.data, "AP+AYQoB/g==");
    assert_eq!(direct.size_bytes, bytes.len());
    let empty = file_tools
        .read_binary_file(empty_path.to_string_lossy().to_string())
        .await
        .unwrap();
    assert!(empty.data.is_empty());
    assert_eq!(empty.size_bytes, 0);

    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router,
        &session_id,
        binary_read_call(json!("binary-read"), path.to_string_lossy().as_ref()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), MAX_BINARY_READ_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(body.len() <= MAX_BINARY_READ_RESPONSE_BYTES);
    let body_text = std::str::from_utf8(&body).unwrap();
    assert!(!body_text.contains("private-binary"));
    assert!(!body_text.contains(path.to_string_lossy().as_ref()));
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let structured = &payload["result"]["structuredContent"];
    assert_eq!(structured["encoding"], "base64");
    assert_eq!(structured["data"], "AP+AYQoB/g==");
    assert_eq!(structured["sizeBytes"], bytes.len());
    assert_eq!(structured["maxFileBytes"], MAX_BINARY_READ_BYTES);
    assert_eq!(
        structured["maxResponseBytes"],
        MAX_BINARY_READ_RESPONSE_BYTES
    );
    assert_eq!(structured.as_object().unwrap().len(), 5);
}

#[tokio::test]
async fn binary_read_accepts_exact_limit_and_rejects_one_byte_over() {
    let root = tempfile::tempdir().unwrap();
    let exact_path = root.path().join("exact.bin");
    let oversized_path = root.path().join("oversized.bin");
    std::fs::write(&exact_path, vec![0xa5; MAX_BINARY_READ_BYTES]).unwrap();
    std::fs::write(&oversized_path, vec![0x5a; MAX_BINARY_READ_BYTES + 1]).unwrap();
    let tools = FileSystemTools::try_new(vec![root.path().to_path_buf()])
        .expect("test safe root must validate");

    let result = tools
        .read_binary_file(exact_path.to_string_lossy().to_string())
        .await
        .unwrap();
    assert_eq!(result.size_bytes, MAX_BINARY_READ_BYTES);
    assert_eq!(result.data.len(), MAX_BINARY_READ_BASE64_BYTES);
    assert!(result.data.starts_with("paWlpaWl"));
    assert!(result.data.ends_with("pQ=="));

    let error = tools
        .read_binary_file(oversized_path.to_string_lossy().to_string())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AppError::FileTooLarge { size, max_size }
            if size == (MAX_BINARY_READ_BYTES + 1) as u64
                && max_size == MAX_BINARY_READ_BYTES as u64
    ));

    let router = test_router(tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router,
        &session_id,
        binary_read_call(json!("exact"), exact_path.to_string_lossy().as_ref()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), MAX_BINARY_READ_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(body.len() <= MAX_BINARY_READ_RESPONSE_BYTES);
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        payload["result"]["structuredContent"]["data"]
            .as_str()
            .unwrap()
            .len(),
        MAX_BINARY_READ_BASE64_BYTES
    );
}

#[cfg(unix)]
#[tokio::test]
async fn binary_read_rejects_missing_outside_symlinked_and_unsupported_targets() {
    use std::os::unix::{fs::symlink, net::UnixListener};

    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_file = outside.path().join("outside.bin");
    std::fs::write(&outside_file, b"outside-secret").unwrap();
    let link = root.path().join("link.bin");
    symlink(&outside_file, &link).unwrap();
    let socket = root.path().join("socket");
    let _listener = UnixListener::bind(&socket).unwrap();
    let linked_parent = root.path().join("linked-parent");
    symlink(outside.path(), &linked_parent).unwrap();
    let tools = FileSystemTools::try_new(vec![root.path().to_path_buf()])
        .expect("test safe root must validate");

    assert!(matches!(
        tools
            .read_binary_file(root.path().join("missing").to_string_lossy().to_string())
            .await,
        Err(AppError::PathNotFound)
    ));
    for target in [outside_file, link, linked_parent.join("outside.bin")] {
        assert!(matches!(
            tools
                .read_binary_file(target.to_string_lossy().to_string())
                .await,
            Err(AppError::PathTraversal { .. })
        ));
    }
    for target in [root.path().to_path_buf(), socket] {
        assert!(matches!(
            tools
                .read_binary_file(target.to_string_lossy().to_string())
                .await,
            Err(AppError::UnsupportedPathType) | Err(AppError::PathTraversal { .. })
        ));
    }
}

#[tokio::test]
async fn binary_read_transport_errors_and_audits_are_bounded_and_private() {
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
            Some(arguments) => json!({"name": "read_binary_file", "arguments": arguments}),
            None => json!({"name": "read_binary_file"}),
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
        binary_read_call(json!("allowed"), path.to_string_lossy().as_ref()),
    )
    .await;
    assert_eq!(allowed.status(), StatusCode::OK);
    let denied = post_json_to_session(
        router.clone(),
        &session_id,
        binary_read_call(json!("denied"), outside.path().to_string_lossy().as_ref()),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::BAD_REQUEST);

    std::fs::remove_file(&path).unwrap();
    let oversized_id = "x".repeat(MAX_BINARY_READ_BYTES / 4);
    let expected_id = json!(oversized_id.clone());
    let oversized_missing = post_json_to_session(
        router.clone(),
        &session_id,
        json!({
            "jsonrpc": "2.0",
            "id": oversized_id.clone(),
            "method": "tools/call",
            "params": {"name": "read_binary_file"},
        }),
    )
    .await;
    assert_eq!(oversized_missing.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = to_bytes(
        oversized_missing.into_body(),
        MAX_BINARY_READ_RESPONSE_BYTES + 1,
    )
    .await
    .unwrap();
    assert!(body.len() <= MAX_BINARY_READ_RESPONSE_BYTES);
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["id"], expected_id);
    assert_eq!(payload["error"]["code"], -32001);

    let oversized = post_json_to_session(
        router.clone(),
        &session_id,
        binary_read_call(json!(oversized_id.clone()), path.to_string_lossy().as_ref()),
    )
    .await;
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = to_bytes(oversized.into_body(), MAX_BINARY_READ_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(body.len() <= MAX_BINARY_READ_RESPONSE_BYTES);
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["id"], expected_id);
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
    let body = to_bytes(status.into_body(), 128 * 1024).await.unwrap();
    let text = std::str::from_utf8(&body).unwrap();
    for forbidden in [
        "private-target",
        "private-content-marker",
        "cHJpdmF0ZS1jb250ZW50LW1hcmtlcg==",
        path.to_string_lossy().as_ref(),
    ] {
        assert!(!text.contains(forbidden));
    }
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let runtime = &payload["result"]["structuredContent"];
    assert_eq!(runtime["binaryFileReads"], true);
    assert_eq!(runtime["binaryFileReadEncoding"], "base64");
    assert_eq!(runtime["binaryFileReadMaxBytes"], MAX_BINARY_READ_BYTES);
    assert_eq!(
        runtime["binaryFileReadMaxResponseBytes"],
        MAX_BINARY_READ_RESPONSE_BYTES
    );
    let counters = &runtime["auditCounters"];
    assert_eq!(counters["by_tool"]["read_binary_file"]["allowed"], 1);
    assert_eq!(counters["by_tool"]["read_binary_file"]["denied"], 6);
    assert_eq!(
        counters["by_reason_code"]["safe_root_binary_read"]["allowed"],
        1
    );
    assert_eq!(
        counters["by_reason_code"]["safe_root_rejected"]["denied"],
        1
    );
    assert_eq!(
        counters["by_reason_code"]["response_size_limit_exceeded"]["denied"],
        2
    );
}
