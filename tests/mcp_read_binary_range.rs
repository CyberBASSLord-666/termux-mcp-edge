#![cfg(feature = "mcp-runtime")]

mod support;

use axum::{body::to_bytes, http::StatusCode};
use serde_json::{json, Value};
use support::{
    create_fifo, empty_test_file_tools, initialize_session, post_json_to_session, test_router,
};
use termux_mcp_server::{
    error::{AppError, INVALID_BINARY_RANGE_PUBLIC_MESSAGE},
    tools::{
        FileSystemTools, MAX_BINARY_RANGE_BASE64_BYTES, MAX_BINARY_RANGE_BYTES,
        MAX_BINARY_RANGE_FILE_BYTES, MAX_BINARY_RANGE_RESPONSE_BYTES,
    },
};

fn range_call(id: Value, path: &str, offset_bytes: u64, length_bytes: usize) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "read_binary_range",
            "arguments": {
                "path": path,
                "offset_bytes": offset_bytes,
                "length_bytes": length_bytes,
            },
        },
    })
}

#[tokio::test]
async fn discovery_advertises_one_closed_binary_range_schema() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router,
        &session_id,
        json!({"jsonrpc":"2.0","id":"range-tools","method":"tools/list"}),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await.unwrap()).unwrap();
    let tools = payload["result"]["tools"].as_array().unwrap();
    assert_eq!(
        tools
            .iter()
            .filter(|tool| tool["name"] == "read_binary_range")
            .count(),
        1
    );
    let schema = &tools
        .iter()
        .find(|tool| tool["name"] == "read_binary_range")
        .unwrap()["inputSchema"];
    assert_eq!(schema["type"], "object");
    assert_eq!(
        schema["required"],
        json!(["path", "offset_bytes", "length_bytes"])
    );
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"].as_object().unwrap().len(), 3);
    assert_eq!(schema["properties"]["path"]["type"], "string");
    assert_eq!(schema["properties"]["offset_bytes"]["type"], "integer");
    assert_eq!(schema["properties"]["offset_bytes"]["minimum"], 0);
    assert_eq!(
        schema["properties"]["offset_bytes"]["maximum"],
        MAX_BINARY_RANGE_FILE_BYTES
    );
    assert_eq!(schema["properties"]["length_bytes"]["minimum"], 1);
    assert_eq!(
        schema["properties"]["length_bytes"]["maximum"],
        MAX_BINARY_RANGE_BYTES
    );
}

#[tokio::test]
async fn range_read_returns_canonical_slice_and_explicit_eof_without_path_metadata() {
    let (root, file_tools) = empty_test_file_tools();
    let path = root.path().join("private-range.bin");
    std::fs::write(&path, [0x00, 0xff, 0x80, b'a', b'\n', 0x01, 0xfe]).unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let response = post_json_to_session(
        router.clone(),
        &session_id,
        range_call(json!("slice"), path.to_string_lossy().as_ref(), 2, 4),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), MAX_BINARY_RANGE_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(body.len() <= MAX_BINARY_RANGE_RESPONSE_BYTES);
    let body_text = std::str::from_utf8(&body).unwrap();
    assert!(!body_text.contains("private-range"));
    assert!(!body_text.contains(path.to_string_lossy().as_ref()));
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let result = &payload["result"]["structuredContent"];
    assert_eq!(result["encoding"], "base64");
    assert_eq!(result["data"], "gGEKAQ==");
    assert_eq!(result["offsetBytes"], 2);
    assert_eq!(result["sizeBytes"], 4);
    assert_eq!(result["fileSizeBytes"], 7);
    assert_eq!(result["eof"], false);
    assert_eq!(result["maxReadBytes"], MAX_BINARY_RANGE_BYTES);
    assert_eq!(result["maxFileBytes"], MAX_BINARY_RANGE_FILE_BYTES);
    assert_eq!(result["maxResponseBytes"], MAX_BINARY_RANGE_RESPONSE_BYTES);
    assert_eq!(result.as_object().unwrap().len(), 9);

    let short_final = post_json_to_session(
        router.clone(),
        &session_id,
        range_call(json!("short-final"), path.to_string_lossy().as_ref(), 5, 10),
    )
    .await;
    assert_eq!(short_final.status(), StatusCode::OK);
    let payload: Value = serde_json::from_slice(
        &to_bytes(short_final.into_body(), MAX_BINARY_RANGE_RESPONSE_BYTES + 1)
            .await
            .unwrap(),
    )
    .unwrap();
    let result = &payload["result"]["structuredContent"];
    assert_eq!(result["data"], "Af4=");
    assert_eq!(result["offsetBytes"], 5);
    assert_eq!(result["sizeBytes"], 2);
    assert_eq!(result["fileSizeBytes"], 7);
    assert_eq!(result["eof"], true);

    let eof = post_json_to_session(
        router,
        &session_id,
        range_call(json!("eof"), path.to_string_lossy().as_ref(), 7, 1),
    )
    .await;
    assert_eq!(eof.status(), StatusCode::OK);
    let payload: Value = serde_json::from_slice(
        &to_bytes(eof.into_body(), MAX_BINARY_RANGE_RESPONSE_BYTES + 1)
            .await
            .unwrap(),
    )
    .unwrap();
    let result = &payload["result"]["structuredContent"];
    assert_eq!(result["data"], "");
    assert_eq!(result["sizeBytes"], 0);
    assert_eq!(result["eof"], true);
}

#[tokio::test]
async fn range_read_enforces_exact_range_and_sparse_file_limits() {
    use std::fs::File;

    let root = tempfile::tempdir().unwrap();
    let exact_range = root.path().join("exact-range.bin");
    let exact_file = root.path().join("exact-file.bin");
    let oversized_file = root.path().join("oversized-file.bin");
    std::fs::write(&exact_range, vec![0xa5; MAX_BINARY_RANGE_BYTES]).unwrap();
    File::create(&exact_file)
        .unwrap()
        .set_len(MAX_BINARY_RANGE_FILE_BYTES as u64)
        .unwrap();
    File::create(&oversized_file)
        .unwrap()
        .set_len((MAX_BINARY_RANGE_FILE_BYTES + 1) as u64)
        .unwrap();
    let tools = FileSystemTools::try_new(vec![root.path().to_path_buf()])
        .expect("test safe root must validate");

    let result = tools
        .read_binary_range(
            exact_range.to_string_lossy().to_string(),
            0,
            MAX_BINARY_RANGE_BYTES,
        )
        .await
        .unwrap();
    assert_eq!(result.size_bytes, MAX_BINARY_RANGE_BYTES);
    assert_eq!(result.data.len(), MAX_BINARY_RANGE_BASE64_BYTES);
    assert!(result.eof);

    let eof = tools
        .read_binary_range(
            exact_file.to_string_lossy().to_string(),
            MAX_BINARY_RANGE_FILE_BYTES as u64,
            1,
        )
        .await
        .unwrap();
    assert_eq!(eof.size_bytes, 0);
    assert!(eof.eof);

    let error = tools
        .read_binary_range(oversized_file.to_string_lossy().to_string(), 0, 1)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AppError::FileTooLarge { size, max_size }
            if size == (MAX_BINARY_RANGE_FILE_BYTES + 1) as u64
                && max_size == MAX_BINARY_RANGE_FILE_BYTES as u64
    ));

    let router = test_router(tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router,
        &session_id,
        range_call(
            json!("exact-range"),
            exact_range.to_string_lossy().as_ref(),
            0,
            MAX_BINARY_RANGE_BYTES,
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), MAX_BINARY_RANGE_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(body.len() <= MAX_BINARY_RANGE_RESPONSE_BYTES);
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        payload["result"]["structuredContent"]["data"]
            .as_str()
            .unwrap()
            .len(),
        MAX_BINARY_RANGE_BASE64_BYTES
    );
}

#[cfg(unix)]
#[tokio::test]
async fn range_read_rejects_missing_outside_symlinked_and_unsupported_targets() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_file = outside.path().join("outside.bin");
    std::fs::write(&outside_file, b"outside-secret").unwrap();
    let link = root.path().join("link.bin");
    symlink(&outside_file, &link).unwrap();
    let fifo = root.path().join("fifo");
    create_fifo(&fifo);
    let linked_parent = root.path().join("linked-parent");
    symlink(outside.path(), &linked_parent).unwrap();
    let tools = FileSystemTools::try_new(vec![root.path().to_path_buf()])
        .expect("test safe root must validate");

    assert!(matches!(
        tools
            .read_binary_range(
                root.path().join("missing").to_string_lossy().to_string(),
                0,
                1
            )
            .await,
        Err(AppError::PathNotFound)
    ));
    for target in [outside_file, link, linked_parent.join("outside.bin")] {
        assert!(matches!(
            tools
                .read_binary_range(target.to_string_lossy().to_string(), 0, 1)
                .await,
            Err(AppError::PathTraversal { .. })
        ));
    }
    for target in [root.path().to_path_buf(), fifo] {
        assert!(matches!(
            tools
                .read_binary_range(target.to_string_lossy().to_string(), 0, 1)
                .await,
            Err(AppError::UnsupportedPathType) | Err(AppError::PathTraversal { .. })
        ));
    }
}

#[tokio::test]
async fn range_read_arguments_response_preflight_and_audits_are_bounded_and_private() {
    let (root, file_tools) = empty_test_file_tools();
    let path = root.path().join("private-range-target.bin");
    std::fs::write(&path, b"private-content-marker").unwrap();
    let outside = tempfile::tempdir().unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    for (index, arguments) in [
        None,
        Some(json!({
            "path": path.to_string_lossy(),
            "offset_bytes": 0,
            "length_bytes": 1,
            "unexpected": true
        })),
        Some(json!({"path": false, "offset_bytes": 0, "length_bytes": 1})),
        Some(json!({
            "path": path.to_string_lossy(),
            "offset_bytes": -1,
            "length_bytes": 1
        })),
        Some(json!({
            "path": path.to_string_lossy(),
            "offset_bytes": 0,
            "length_bytes": -1
        })),
    ]
    .into_iter()
    .enumerate()
    {
        let params = match arguments {
            Some(arguments) => json!({"name":"read_binary_range","arguments":arguments}),
            None => json!({"name":"read_binary_range"}),
        };
        let response = post_json_to_session(
            router.clone(),
            &session_id,
            json!({
                "jsonrpc": "2.0",
                "id": format!("invalid-{index}"),
                "method": "tools/call",
                "params": params
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    for (index, (offset, length)) in [
        (0, 0),
        (0, MAX_BINARY_RANGE_BYTES + 1),
        (MAX_BINARY_RANGE_FILE_BYTES as u64 + 1, 1),
        (b"private-content-marker".len() as u64 + 1, 1),
    ]
    .into_iter()
    .enumerate()
    {
        let response = post_json_to_session(
            router.clone(),
            &session_id,
            range_call(
                json!(format!("bad-range-{index}")),
                path.to_string_lossy().as_ref(),
                offset,
                length,
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let payload: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), MAX_BINARY_RANGE_RESPONSE_BYTES)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["error"]["data"],
            INVALID_BINARY_RANGE_PUBLIC_MESSAGE
        );
    }

    let allowed = post_json_to_session(
        router.clone(),
        &session_id,
        range_call(json!("allowed"), path.to_string_lossy().as_ref(), 0, 7),
    )
    .await;
    assert_eq!(allowed.status(), StatusCode::OK);
    let denied = post_json_to_session(
        router.clone(),
        &session_id,
        range_call(
            json!("denied"),
            outside.path().to_string_lossy().as_ref(),
            0,
            1,
        ),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::BAD_REQUEST);

    std::fs::remove_file(&path).unwrap();
    let oversized_id = "x".repeat(MAX_BINARY_RANGE_RESPONSE_BYTES);
    let oversized_missing = post_json_to_session(
        router.clone(),
        &session_id,
        json!({
            "jsonrpc": "2.0",
            "id": oversized_id.clone(),
            "method": "tools/call",
            "params": {"name": "read_binary_range"}
        }),
    )
    .await;
    assert_eq!(oversized_missing.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let oversized = post_json_to_session(
        router.clone(),
        &session_id,
        range_call(json!(oversized_id), path.to_string_lossy().as_ref(), 0, 1),
    )
    .await;
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let payload: Value = serde_json::from_slice(
        &to_bytes(oversized.into_body(), MAX_BINARY_RANGE_RESPONSE_BYTES + 1)
            .await
            .unwrap(),
    )
    .unwrap();
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
    let body = to_bytes(status.into_body(), 128 * 1024).await.unwrap();
    let text = std::str::from_utf8(&body).unwrap();
    for forbidden in [
        "private-range-target",
        "private-content-marker",
        path.to_string_lossy().as_ref(),
    ] {
        assert!(!text.contains(forbidden));
    }
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let runtime = &payload["result"]["structuredContent"];
    assert_eq!(runtime["binaryRangeReads"], true);
    assert_eq!(runtime["binaryRangeReadEncoding"], "base64");
    assert_eq!(
        runtime["binaryRangeReadMaxFileBytes"],
        MAX_BINARY_RANGE_FILE_BYTES
    );
    assert_eq!(runtime["binaryRangeReadMaxBytes"], MAX_BINARY_RANGE_BYTES);
    assert_eq!(
        runtime["binaryRangeReadMaxResponseBytes"],
        MAX_BINARY_RANGE_RESPONSE_BYTES
    );
    let counters = &runtime["auditCounters"];
    assert_eq!(counters["by_tool"]["read_binary_range"]["allowed"], 1);
    assert_eq!(counters["by_tool"]["read_binary_range"]["denied"], 12);
    assert_eq!(
        counters["by_reason_code"]["safe_root_binary_range_read"]["allowed"],
        1
    );
    assert_eq!(
        counters["by_reason_code"]["filesystem_binary_range_invalid"]["denied"],
        4
    );
    assert_eq!(counters["by_reason_code"]["invalid_arguments"]["denied"], 4);
    assert_eq!(
        counters["by_reason_code"]["response_size_limit_exceeded"]["denied"],
        2
    );
}
