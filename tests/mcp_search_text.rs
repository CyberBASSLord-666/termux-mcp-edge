#![cfg(feature = "mcp-runtime")]

mod support;

use axum::{body::to_bytes, http::StatusCode};
use serde_json::{json, Value};
use support::{empty_test_file_tools, initialize_session, post_json_to_session, test_router};
use termux_mcp_server::tools::{
    MAX_SEARCH_DEPTH, MAX_SEARCH_QUERY_BYTES, MAX_SEARCH_RESPONSE_BYTES, MIN_SEARCH_DEPTH,
};

fn tool_call(id: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "search_text",
            "arguments": arguments,
        },
    })
}

#[tokio::test]
async fn discovery_advertises_one_closed_literal_search_schema() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router,
        &session_id,
        json!({
            "jsonrpc": "2.0",
            "id": "search-tools",
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
            .filter(|tool| tool["name"] == "search_text")
            .count(),
        1
    );
    let schema = tools
        .iter()
        .find(|tool| tool["name"] == "search_text")
        .unwrap()["inputSchema"]
        .clone();
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["required"], json!(["path", "query"]));
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"].as_object().unwrap().len(), 3);
    assert_eq!(schema["properties"]["query"]["minLength"], 1);
    assert_eq!(
        schema["properties"]["query"]["maxLength"],
        MAX_SEARCH_QUERY_BYTES
    );
    assert_eq!(
        schema["properties"]["query"]["x-maxBytes"],
        MAX_SEARCH_QUERY_BYTES
    );
    assert_eq!(
        schema["properties"]["max_depth"]["minimum"],
        MIN_SEARCH_DEPTH
    );
    assert_eq!(
        schema["properties"]["max_depth"]["maximum"],
        MAX_SEARCH_DEPTH
    );
}

#[tokio::test]
async fn search_call_returns_bounded_locations_without_file_content_or_query_echo() {
    let (root, file_tools) = empty_test_file_tools();
    let nested = root.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    std::fs::write(
        root.path().join("alpha.txt"),
        "private needle value\nneedle",
    )
    .unwrap();
    std::fs::write(nested.join("beta.txt"), "needle nested private value").unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router,
        &session_id,
        tool_call(
            "search-success",
            json!({
                "path": root.path().to_string_lossy(),
                "query": "needle",
                "max_depth": 2
            }),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), MAX_SEARCH_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(body.len() <= MAX_SEARCH_RESPONSE_BYTES);
    let body_text = std::str::from_utf8(&body).unwrap();
    assert!(!body_text.contains("private needle value"));
    assert!(!body_text.contains("needle nested private value"));
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["result"]["isError"], false);
    let structured = &payload["result"]["structuredContent"];
    assert_eq!(structured["truncated"], false);
    assert_eq!(structured["queryBytes"], 6);
    assert_eq!(structured["matches"].as_array().unwrap().len(), 3);
    assert_eq!(structured["matches"][0]["lineNumber"], 1);
    assert_eq!(structured["matches"][0]["columnByte"], 9);
    assert_eq!(structured["matches"][1]["lineNumber"], 2);
    assert_eq!(structured["matches"][1]["columnByte"], 1);
    assert_eq!(structured["matches"][2]["lineNumber"], 1);
    assert_eq!(structured["matches"][2]["columnByte"], 1);
}

#[tokio::test]
async fn search_rejects_query_depth_and_override_shapes_before_filesystem_work() {
    let (root, file_tools) = empty_test_file_tools();
    let marker = root.path().join("marker.txt");
    std::fs::write(&marker, "needle").unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let invalid = [
        json!({"path": root.path().to_string_lossy(), "query": ""}),
        json!({
            "path": root.path().to_string_lossy(),
            "query": "q".repeat(MAX_SEARCH_QUERY_BYTES + 1)
        }),
        json!({
            "path": root.path().to_string_lossy(),
            "query": "😀".repeat((MAX_SEARCH_QUERY_BYTES / 4) + 1)
        }),
        json!({"path": root.path().to_string_lossy(), "query": "two\nlines"}),
        json!({
            "path": root.path().to_string_lossy(),
            "query": "needle",
            "max_depth": MIN_SEARCH_DEPTH - 1
        }),
        json!({
            "path": root.path().to_string_lossy(),
            "query": "needle",
            "max_depth": MAX_SEARCH_DEPTH + 1
        }),
        json!({
            "path": root.path().to_string_lossy(),
            "query": "needle",
            "regex": true
        }),
        json!({
            "path": root.path().to_string_lossy(),
            "query": "needle",
            "command": "grep -R needle"
        }),
    ];

    for (index, arguments) in invalid.into_iter().enumerate() {
        let response = post_json_to_session(
            router.clone(),
            &session_id,
            tool_call(&format!("invalid-{index}"), arguments),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "case {index}");
        let body = to_bytes(response.into_body(), 8 * 1024).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["error"]["code"], -32602);
        assert_eq!(payload["error"]["message"], "Invalid params");
    }
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "needle");
}

#[tokio::test]
async fn search_audit_records_only_stable_labels_for_allowed_and_denied_calls() {
    let (root, file_tools) = empty_test_file_tools();
    std::fs::write(root.path().join("source.txt"), "needle private-content").unwrap();
    let outside = tempfile::tempdir().unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let allowed = post_json_to_session(
        router.clone(),
        &session_id,
        tool_call(
            "allowed",
            json!({"path": root.path().to_string_lossy(), "query": "needle"}),
        ),
    )
    .await;
    assert_eq!(allowed.status(), StatusCode::OK);
    let denied = post_json_to_session(
        router.clone(),
        &session_id,
        tool_call(
            "denied",
            json!({"path": outside.path().to_string_lossy(), "query": "private-query"}),
        ),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::BAD_REQUEST);

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
    assert!(!text.contains("private-query"));
    assert!(!text.contains("private-content"));
    assert!(!text.contains(outside.path().to_string_lossy().as_ref()));
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let counters = &payload["result"]["structuredContent"]["auditCounters"];
    assert_eq!(counters["by_tool"]["search_text"]["allowed"], 1);
    assert_eq!(counters["by_tool"]["search_text"]["denied"], 1);
    assert_eq!(
        counters["by_reason_code"]["safe_root_text_searched"]["allowed"],
        1
    );
    assert_eq!(
        counters["by_reason_code"]["safe_root_rejected"]["denied"],
        1
    );
}
