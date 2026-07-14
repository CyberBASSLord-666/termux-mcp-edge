#![cfg(feature = "mcp-runtime")]

mod support;

use axum::{body::to_bytes, http::StatusCode};
use serde_json::{json, Value};
use support::{empty_test_file_tools, initialize_session, post_json_to_session, test_router};
use termux_mcp_server::tools::MAX_PATH_METADATA_RESPONSE_BYTES;

fn metadata_call(id: Value, path: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "path_metadata",
            "arguments": {"path": path},
        },
    })
}

#[tokio::test]
async fn discovery_advertises_one_closed_path_metadata_schema() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router,
        &session_id,
        json!({
            "jsonrpc": "2.0",
            "id": "metadata-tools",
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
            .filter(|tool| tool["name"] == "path_metadata")
            .count(),
        1
    );
    let schema = &tools
        .iter()
        .find(|tool| tool["name"] == "path_metadata")
        .unwrap()["inputSchema"];
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["required"], json!(["path"]));
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"].as_object().unwrap().len(), 1);
    assert_eq!(schema["properties"]["path"]["type"], "string");
}

#[tokio::test]
async fn metadata_call_returns_bounded_non_sensitive_file_and_root_metadata() {
    let (root, file_tools) = empty_test_file_tools();
    let file = root.path().join("visible.txt");
    std::fs::write(&file, "private-file-content").unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let response = post_json_to_session(
        router.clone(),
        &session_id,
        metadata_call(json!("metadata-file"), file.to_string_lossy().as_ref()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), MAX_PATH_METADATA_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(body.len() <= MAX_PATH_METADATA_RESPONSE_BYTES);
    let body_text = std::str::from_utf8(&body).unwrap();
    assert!(!body_text.contains("private-file-content"));
    for forbidden in ["inode", "device", "uid", "gid", "mode", "accessTime"] {
        assert!(!body_text.contains(forbidden));
    }
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let structured = &payload["result"]["structuredContent"];
    assert_eq!(structured["path"], file.to_string_lossy().as_ref());
    assert_eq!(structured["kind"], "regular_file");
    assert_eq!(structured["sizeBytes"], 20);
    assert!(structured["modified"].is_string());
    assert_eq!(
        structured["maxResponseBytes"],
        MAX_PATH_METADATA_RESPONSE_BYTES
    );

    let response = post_json_to_session(
        router,
        &session_id,
        metadata_call(
            json!("metadata-root"),
            root.path().to_string_lossy().as_ref(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), MAX_PATH_METADATA_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let structured = &payload["result"]["structuredContent"];
    assert_eq!(structured["path"], root.path().to_string_lossy().as_ref());
    assert_eq!(structured["kind"], "directory");
    assert_eq!(structured["sizeBytes"], Value::Null);
}

#[tokio::test]
async fn metadata_rejects_invalid_outside_missing_and_symlink_requests() {
    use std::os::unix::fs::symlink;

    let (root, file_tools) = empty_test_file_tools();
    let file = root.path().join("visible.txt");
    std::fs::write(&file, "visible").unwrap();
    symlink(&file, root.path().join("link.txt")).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    for (index, arguments) in [
        None,
        Some(json!({"path": file.to_string_lossy(), "unexpected": true})),
        Some(json!({"path": false})),
        Some(json!({"path": "relative.txt"})),
        Some(json!({"path": "bad\0path"})),
        Some(json!({"path": root.path().join("..").join("outside").to_string_lossy()})),
        Some(json!({"path": outside.path().to_string_lossy()})),
        Some(json!({"path": root.path().join("missing").to_string_lossy()})),
        Some(json!({"path": root.path().join("link.txt").to_string_lossy()})),
    ]
    .into_iter()
    .enumerate()
    {
        let params = match arguments {
            Some(arguments) => json!({"name": "path_metadata", "arguments": arguments}),
            None => json!({"name": "path_metadata"}),
        };
        let response = post_json_to_session(
            router.clone(),
            &session_id,
            json!({
                "jsonrpc": "2.0",
                "id": format!("metadata-invalid-{index}"),
                "method": "tools/call",
                "params": params,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "case {index}");
        let body = to_bytes(response.into_body(), 8 * 1024).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["error"]["code"], -32602);
        assert_eq!(payload["error"]["message"], "Invalid params");
    }
}

#[tokio::test]
async fn metadata_response_bound_and_audit_counters_remain_private() {
    let (root, file_tools) = empty_test_file_tools();
    let file = root.path().join("private-name.txt");
    std::fs::write(&file, "private-content").unwrap();
    let outside = tempfile::tempdir().unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let allowed = post_json_to_session(
        router.clone(),
        &session_id,
        metadata_call(json!("allowed"), file.to_string_lossy().as_ref()),
    )
    .await;
    assert_eq!(allowed.status(), StatusCode::OK);
    let denied = post_json_to_session(
        router.clone(),
        &session_id,
        metadata_call(json!("denied"), outside.path().to_string_lossy().as_ref()),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::BAD_REQUEST);
    let oversized_id = "x".repeat(MAX_PATH_METADATA_RESPONSE_BYTES);
    let oversized = post_json_to_session(
        router.clone(),
        &session_id,
        metadata_call(json!(oversized_id), file.to_string_lossy().as_ref()),
    )
    .await;
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = to_bytes(oversized.into_body(), MAX_PATH_METADATA_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(body.len() <= MAX_PATH_METADATA_RESPONSE_BYTES);
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
    for forbidden in ["private-name", "private-content", "tmp"] {
        assert!(!text.contains(forbidden));
    }
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let counters = &payload["result"]["structuredContent"]["auditCounters"];
    assert_eq!(counters["by_tool"]["path_metadata"]["allowed"], 1);
    assert_eq!(counters["by_tool"]["path_metadata"]["denied"], 2);
    assert_eq!(
        counters["by_reason_code"]["safe_root_metadata_read"]["allowed"],
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
