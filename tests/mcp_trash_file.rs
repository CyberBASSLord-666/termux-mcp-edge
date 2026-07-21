#![cfg(feature = "mcp-runtime")]

mod support;

use std::os::unix::fs::symlink;

use axum::{body::to_bytes, http::StatusCode};
use serde_json::{json, Value};
use support::{empty_test_file_tools, initialize_session, post_json_to_session, test_router};
use termux_mcp_server::tools::{MAX_TRASH_FILE_BYTES, MAX_TRASH_FILE_RESPONSE_BYTES};

const PREVIEW_SUMMARY: &str =
    "Validated one bounded safe-rooted file for reversible trashing without mutation.";

fn trash_call(id: impl Into<Value>, path: &str, dry_run: Option<bool>) -> Value {
    let mut arguments = json!({"path": path});
    if let Some(dry_run) = dry_run {
        arguments["dry_run"] = json!(dry_run);
    }
    json!({
        "jsonrpc": "2.0",
        "id": id.into(),
        "method": "tools/call",
        "params": {"name": "trash_file", "arguments": arguments},
    })
}

async fn runtime_status(router: axum::Router, session_id: &str) -> Value {
    let response = post_json_to_session(
        router,
        session_id,
        json!({
            "jsonrpc":"2.0",
            "id":"trash-runtime-status",
            "method":"tools/call",
            "params":{"name":"runtime_status","arguments":{}}
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 128 * 1024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn disabled_discovery_is_one_closed_preview_only_trash_schema() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router.clone(),
        &session_id,
        json!({"jsonrpc":"2.0","id":"tools","method":"tools/list"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 128 * 1024).await.unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let tools = payload["result"]["tools"].as_array().unwrap();
    assert_eq!(
        tools
            .iter()
            .filter(|tool| tool["name"] == "trash_file")
            .count(),
        1
    );
    let tool = tools
        .iter()
        .find(|tool| tool["name"] == "trash_file")
        .unwrap();
    let schema = &tool["inputSchema"];
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["required"], json!(["path"]));
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"].as_object().unwrap().len(), 2);
    assert_eq!(schema["properties"]["path"]["type"], "string");
    assert_eq!(schema["properties"]["dry_run"]["type"], "boolean");
    assert_eq!(schema["properties"]["dry_run"]["const"], true);
    assert!(tool["description"]
        .as_str()
        .unwrap()
        .contains("mutation gate is disabled"));

    let status = runtime_status(router, &session_id).await;
    let runtime = &status["result"]["structuredContent"];
    assert_eq!(runtime["trashFileMutationEnabled"], false);
    assert_eq!(runtime["trashFileGrantRequired"], false);
    assert_eq!(runtime["trashFileMode"], "dry_run_only_mutation_disabled");
    assert_eq!(runtime["trashFileMaxBytes"], MAX_TRASH_FILE_BYTES);
    assert_eq!(
        runtime["trashFileMaxResponseBytes"],
        MAX_TRASH_FILE_RESPONSE_BYTES
    );
}

#[tokio::test]
async fn trash_preview_is_path_free_nonmutating_and_live_gate_precedes_path_access() {
    let (root, file_tools) = empty_test_file_tools();
    let target = root.path().join("private-trash-target.bin");
    let content = b"private-trash-content\0\xff\x80";
    std::fs::write(&target, content).unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    for dry_run in [None, Some(true)] {
        let response = post_json_to_session(
            router.clone(),
            &session_id,
            trash_call(
                format!("preview-{dry_run:?}"),
                target.to_string_lossy().as_ref(),
                dry_run,
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), MAX_TRASH_FILE_RESPONSE_BYTES + 1)
            .await
            .unwrap();
        assert!(body.len() <= MAX_TRASH_FILE_RESPONSE_BYTES);
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["result"]["content"][0]["text"], PREVIEW_SUMMARY);
        assert_eq!(
            payload["result"]["structuredContent"],
            json!({
                "dryRun": true,
                "sizeBytes": content.len(),
                "recoveryArtifactRetained": false,
                "maxFileBytes": MAX_TRASH_FILE_BYTES,
                "maxResponseBytes": MAX_TRASH_FILE_RESPONSE_BYTES,
            })
        );
        let serialized = payload.to_string();
        assert!(!serialized.contains(target.to_string_lossy().as_ref()));
        assert!(!serialized.contains("private-trash-content"));
        assert!(target.exists());
        assert!(!root.path().join(".termux-mcp-trash-quarantine").exists());
    }

    let outside = tempfile::tempdir().unwrap();
    let outside_target = outside.path().join("must-not-be-read");
    let denied = post_json_to_session(
        router,
        &session_id,
        trash_call(
            "disabled-live",
            outside_target.to_string_lossy().as_ref(),
            Some(false),
        ),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let body = to_bytes(denied.into_body(), MAX_TRASH_FILE_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["error"]["code"], -32003);
    assert_eq!(
        payload["error"]["data"]["reason"],
        "trash_file_mutation_disabled"
    );
    assert!(!outside_target.exists());
}

#[tokio::test]
async fn trash_preview_enforces_object_link_root_size_and_response_boundaries() {
    let (root, file_tools) = empty_test_file_tools();
    let regular = root.path().join("regular.bin");
    let hardlink = root.path().join("hardlink.bin");
    let directory = root.path().join("directory");
    let symlink_path = root.path().join("symlink.bin");
    let oversized = root.path().join("oversized.bin");
    std::fs::write(&regular, b"private-boundary-content").unwrap();
    std::fs::hard_link(&regular, &hardlink).unwrap();
    std::fs::create_dir(&directory).unwrap();
    symlink(&regular, &symlink_path).unwrap();
    std::fs::write(&oversized, vec![0x5a; MAX_TRASH_FILE_BYTES + 1]).unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    for (label, path, expected) in [
        (
            "missing",
            root.path().join("missing.bin"),
            StatusCode::BAD_REQUEST,
        ),
        ("directory", directory, StatusCode::BAD_REQUEST),
        ("symlink", symlink_path, StatusCode::BAD_REQUEST),
        ("hardlink", regular.clone(), StatusCode::BAD_REQUEST),
        ("oversized", oversized, StatusCode::PAYLOAD_TOO_LARGE),
    ] {
        let response = post_json_to_session(
            router.clone(),
            &session_id,
            trash_call(label, path.to_string_lossy().as_ref(), Some(true)),
        )
        .await;
        assert_eq!(response.status(), expected, "{label}");
        let body = to_bytes(response.into_body(), MAX_TRASH_FILE_RESPONSE_BYTES + 1)
            .await
            .unwrap();
        let serialized = String::from_utf8_lossy(&body);
        assert!(!serialized.contains("private-boundary-content"));
        assert!(!serialized.contains(root.path().to_string_lossy().as_ref()));
    }
    assert!(regular.exists());
    assert!(hardlink.exists());

    let outside = tempfile::tempdir().unwrap();
    let outside_target = outside.path().join("outside.bin");
    std::fs::write(&outside_target, b"outside-secret").unwrap();
    let response = post_json_to_session(
        router.clone(),
        &session_id,
        trash_call(
            "outside",
            outside_target.to_string_lossy().as_ref(),
            Some(true),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), MAX_TRASH_FILE_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&body).contains(outside.path().to_string_lossy().as_ref()));

    let response = post_json_to_session(
        router.clone(),
        &session_id,
        trash_call(
            "x".repeat(MAX_TRASH_FILE_RESPONSE_BYTES),
            "/outside/not-readable",
            Some(true),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = to_bytes(response.into_body(), MAX_TRASH_FILE_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(body.len() <= MAX_TRASH_FILE_RESPONSE_BYTES);
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["id"], Value::Null);

    let status = runtime_status(router, &session_id).await;
    let counters = &status["result"]["structuredContent"]["auditCounters"];
    assert_eq!(counters["by_tool"]["trash_file"]["denied"], 7);
    assert_eq!(
        counters["by_reason_code"]["response_size_limit_exceeded"]["denied"],
        1
    );
}

#[tokio::test]
async fn trash_preview_accepts_the_exact_one_mib_limit_without_creating_recovery_state() {
    let (root, file_tools) = empty_test_file_tools();
    let target = root.path().join("exact.bin");
    std::fs::write(&target, vec![0xa5; MAX_TRASH_FILE_BYTES]).unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router,
        &session_id,
        trash_call("exact", target.to_string_lossy().as_ref(), Some(true)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), MAX_TRASH_FILE_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        payload["result"]["structuredContent"]["sizeBytes"],
        MAX_TRASH_FILE_BYTES
    );
    assert!(target.exists());
    assert!(!root.path().join(".termux-mcp-trash-quarantine").exists());
}
