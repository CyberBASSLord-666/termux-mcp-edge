#![cfg(feature = "mcp-runtime")]

mod support;

use std::os::unix::fs::{symlink, PermissionsExt};

use axum::{body::to_bytes, http::StatusCode};
use serde_json::{json, Value};
use support::{empty_test_file_tools, initialize_session, post_json_to_session, test_router};
use termux_mcp_server::tools::{COPY_FILE_MODE, MAX_COPY_FILE_BYTES, MAX_COPY_FILE_RESPONSE_BYTES};

fn copy_call(id: Value, source_path: &str, destination_path: &str, dry_run: Option<bool>) -> Value {
    let mut arguments = json!({
        "source_path": source_path,
        "destination_path": destination_path,
    });
    if let Some(dry_run) = dry_run {
        arguments["dry_run"] = json!(dry_run);
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "copy_file",
            "arguments": arguments,
        },
    })
}

async fn runtime_status(router: axum::Router, session_id: &str) -> Value {
    let response = post_json_to_session(
        router,
        session_id,
        json!({
            "jsonrpc": "2.0",
            "id": "copy-runtime-status",
            "method": "tools/call",
            "params": {"name": "runtime_status", "arguments": {}}
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn discovery_advertises_one_closed_copy_file_schema() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router,
        &session_id,
        json!({
            "jsonrpc": "2.0",
            "id": "copy-file-tools",
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
            .filter(|tool| tool["name"] == "copy_file")
            .count(),
        1
    );
    let schema = &tools
        .iter()
        .find(|tool| tool["name"] == "copy_file")
        .unwrap()["inputSchema"];
    assert_eq!(schema["type"], "object");
    assert_eq!(
        schema["required"],
        json!(["source_path", "destination_path"])
    );
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"].as_object().unwrap().len(), 3);
    for field in ["source_path", "destination_path"] {
        assert_eq!(schema["properties"][field]["type"], "string");
    }
    assert_eq!(schema["properties"]["dry_run"]["type"], "boolean");
}

#[tokio::test]
async fn copy_file_is_dry_run_first_and_explicit_binary_copy_is_exact() {
    let (root, file_tools) = empty_test_file_tools();
    let source = root.path().join("private-source.bin");
    let preview = root.path().join("preview.bin");
    let destination = root.path().join("destination.bin");
    let private_content = b"private-copy-content\0\xff\x80";
    std::fs::write(&source, private_content).unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o777)).unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    for dry_run in [None, Some(true)] {
        let response = post_json_to_session(
            router.clone(),
            &session_id,
            copy_call(
                json!(format!("preview-{dry_run:?}")),
                source.to_string_lossy().as_ref(),
                preview.to_string_lossy().as_ref(),
                dry_run,
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), MAX_COPY_FILE_RESPONSE_BYTES + 1)
            .await
            .unwrap();
        assert!(body.len() <= MAX_COPY_FILE_RESPONSE_BYTES);
        assert!(!String::from_utf8_lossy(&body).contains("private-copy-content"));
        let payload: Value = serde_json::from_slice(&body).unwrap();
        let result = &payload["result"]["structuredContent"];
        assert_eq!(result["sourcePath"], source.to_string_lossy().as_ref());
        assert_eq!(
            result["destinationPath"],
            preview.to_string_lossy().as_ref()
        );
        assert_eq!(result["dryRun"], true);
        assert_eq!(result["sizeBytes"], private_content.len());
        assert_eq!(result["mode"], "0600");
        assert_eq!(result["maxFileBytes"], MAX_COPY_FILE_BYTES);
        assert_eq!(result["maxResponseBytes"], MAX_COPY_FILE_RESPONSE_BYTES);
        assert!(!preview.exists());
    }

    let response = post_json_to_session(
        router,
        &session_id,
        copy_call(
            json!("explicit-copy"),
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
            Some(false),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), MAX_COPY_FILE_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&body).contains("private-copy-content"));
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["result"]["structuredContent"]["dryRun"], false);
    assert_eq!(std::fs::read(&destination).unwrap(), private_content);
    assert_eq!(
        std::fs::symlink_metadata(destination)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        COPY_FILE_MODE
    );
}

#[tokio::test]
async fn copy_file_transport_accepts_the_exact_one_mib_limit() {
    let (root, file_tools) = empty_test_file_tools();
    let source = root.path().join("exact-limit.bin");
    let destination = root.path().join("exact-limit-copy.bin");
    std::fs::write(&source, vec![0x5a; MAX_COPY_FILE_BYTES]).unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let response = post_json_to_session(
        router,
        &session_id,
        copy_call(
            json!("exact-limit"),
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
            Some(false),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), MAX_COPY_FILE_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(body.len() <= MAX_COPY_FILE_RESPONSE_BYTES);
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        payload["result"]["structuredContent"]["sizeBytes"],
        MAX_COPY_FILE_BYTES
    );
    assert_eq!(std::fs::metadata(&destination).unwrap().len(), 1_048_576);
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        vec![0x5a; MAX_COPY_FILE_BYTES]
    );
}

#[tokio::test]
async fn copy_file_rejects_invalid_existing_missing_and_unsupported_requests() {
    let (root, file_tools) = empty_test_file_tools();
    let source = root.path().join("private-source.txt");
    let existing = root.path().join("private-existing.txt");
    let directory = root.path().join("private-directory");
    std::fs::write(&source, "private-source-content").unwrap();
    std::fs::write(&existing, "unchanged").unwrap();
    std::fs::create_dir(&directory).unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let cases = [
        None,
        Some(json!({
            "source_path": source.to_string_lossy(),
            "destination_path": root.path().join("unknown").to_string_lossy(),
            "unknown": true
        })),
        Some(json!({
            "source_path": false,
            "destination_path": root.path().join("bad-source-type").to_string_lossy()
        })),
        Some(json!({
            "source_path": source.to_string_lossy(),
            "destination_path": false
        })),
        Some(json!({
            "source_path": source.to_string_lossy(),
            "destination_path": root.path().join("bad-dry-run").to_string_lossy(),
            "dry_run": "false"
        })),
        Some(json!({
            "source_path": root.path().join("missing-source").to_string_lossy(),
            "destination_path": root.path().join("unused-a").to_string_lossy(),
            "dry_run": false
        })),
        Some(json!({
            "source_path": source.to_string_lossy(),
            "destination_path": root.path().join("missing-parent").join("copy").to_string_lossy(),
            "dry_run": false
        })),
        Some(json!({
            "source_path": source.to_string_lossy(),
            "destination_path": source.to_string_lossy(),
            "dry_run": false
        })),
        Some(json!({
            "source_path": source.to_string_lossy(),
            "destination_path": existing.to_string_lossy(),
            "dry_run": false
        })),
        Some(json!({
            "source_path": directory.to_string_lossy(),
            "destination_path": root.path().join("directory-copy").to_string_lossy(),
            "dry_run": false
        })),
    ];

    for (index, arguments) in cases.into_iter().enumerate() {
        let params = match arguments {
            Some(arguments) => json!({"name": "copy_file", "arguments": arguments}),
            None => json!({"name": "copy_file"}),
        };
        let response = post_json_to_session(
            router.clone(),
            &session_id,
            json!({
                "jsonrpc": "2.0",
                "id": format!("copy-invalid-{index}"),
                "method": "tools/call",
                "params": params,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "case {index}");
        let body = to_bytes(response.into_body(), 8 * 1024).await.unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(!text.contains("private-source-content"));
        assert!(!text.contains("private-existing"));
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["error"]["code"], -32602);
        assert_eq!(payload["error"]["message"], "Invalid params");
    }

    assert_eq!(std::fs::read_to_string(existing).unwrap(), "unchanged");
    let status = runtime_status(router, &session_id).await;
    let serialized = serde_json::to_string(&status).unwrap();
    assert!(!serialized.contains("private-source-content"));
    assert!(!serialized.contains("private-existing"));
    let counters = &status["result"]["structuredContent"]["auditCounters"];
    assert_eq!(counters["by_tool"]["copy_file"]["denied"], 10);
    assert_eq!(counters["by_reason_code"]["missing_arguments"]["denied"], 1);
    assert_eq!(counters["by_reason_code"]["invalid_arguments"]["denied"], 4);
    assert_eq!(
        counters["by_reason_code"]["filesystem_copy_source_not_found"]["denied"],
        1
    );
    assert_eq!(
        counters["by_reason_code"]["filesystem_copy_parent_not_found"]["denied"],
        1
    );
    assert_eq!(
        counters["by_reason_code"]["filesystem_copy_same_path"]["denied"],
        1
    );
    assert_eq!(
        counters["by_reason_code"]["filesystem_destination_exists"]["denied"],
        1
    );
    assert_eq!(
        counters["by_reason_code"]["filesystem_copy_source_type_unsupported"]["denied"],
        1
    );
}

#[tokio::test]
async fn copy_file_rejects_outside_symlink_and_oversized_sources_without_leaks() {
    let (root, file_tools) = empty_test_file_tools();
    let outside = tempfile::tempdir().unwrap();
    let source = root.path().join("private-source.txt");
    let source_link = root.path().join("private-source-link");
    let destination_link = root.path().join("private-destination-link");
    let linked_parent = root.path().join("private-linked-parent");
    let oversized = root.path().join("private-oversized.bin");
    std::fs::write(&source, "private-content-must-not-leak").unwrap();
    std::fs::write(&oversized, vec![0x61; MAX_COPY_FILE_BYTES + 1]).unwrap();
    symlink(&source, &source_link).unwrap();
    symlink(outside.path().join("redirected"), &destination_link).unwrap();
    symlink(outside.path(), &linked_parent).unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let boundary_cases = [
        (
            outside.path().join("outside-source"),
            root.path().join("copy-a"),
        ),
        (source.clone(), outside.path().join("outside-destination")),
        (source_link, root.path().join("copy-b")),
        (source.clone(), destination_link),
        (source.clone(), linked_parent.join("copy-c")),
    ];
    for (index, (copy_source, copy_destination)) in boundary_cases.into_iter().enumerate() {
        let response = post_json_to_session(
            router.clone(),
            &session_id,
            copy_call(
                json!(format!("copy-boundary-{index}")),
                copy_source.to_string_lossy().as_ref(),
                copy_destination.to_string_lossy().as_ref(),
                Some(false),
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), 8 * 1024).await.unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(!text.contains("private-content-must-not-leak"));
        assert!(!text.contains(outside.path().to_string_lossy().as_ref()));
    }

    let oversized_destination = root.path().join("oversized-copy");
    let response = post_json_to_session(
        router.clone(),
        &session_id,
        copy_call(
            json!("copy-oversized"),
            oversized.to_string_lossy().as_ref(),
            oversized_destination.to_string_lossy().as_ref(),
            Some(false),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(!oversized_destination.exists());
    assert!(!outside.path().join("outside-destination").exists());
    assert!(!outside.path().join("redirected").exists());
    assert!(!outside.path().join("copy-c").exists());

    let status = runtime_status(router, &session_id).await;
    let serialized = serde_json::to_string(&status).unwrap();
    assert!(!serialized.contains("private-content-must-not-leak"));
    assert!(!serialized.contains(outside.path().to_string_lossy().as_ref()));
    let counters = &status["result"]["structuredContent"]["auditCounters"];
    assert_eq!(counters["by_tool"]["copy_file"]["denied"], 6);
    assert_eq!(
        counters["by_reason_code"]["safe_root_rejected"]["denied"],
        5
    );
    assert_eq!(
        counters["by_reason_code"]["filesystem_copy_source_too_large"]["denied"],
        1
    );
}

#[tokio::test]
async fn copy_file_preflights_full_response_before_mutation_and_audit_stays_private() {
    let (root, file_tools) = empty_test_file_tools();
    let source = root.path().join("private-preflight-source.txt");
    let blocked = root.path().join("blocked-by-response-bound.txt");
    let preview = root.path().join("private-preview.txt");
    let copied = root.path().join("private-copied.txt");
    std::fs::write(&source, "private-preflight-content").unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let response = post_json_to_session(
        router.clone(),
        &session_id,
        copy_call(
            json!("x".repeat(MAX_COPY_FILE_RESPONSE_BYTES)),
            source.to_string_lossy().as_ref(),
            blocked.to_string_lossy().as_ref(),
            Some(false),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = to_bytes(response.into_body(), MAX_COPY_FILE_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(body.len() <= MAX_COPY_FILE_RESPONSE_BYTES);
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["id"], Value::Null);
    assert_eq!(payload["error"]["code"], -32001);
    assert!(!blocked.exists());

    for (id, destination, dry_run) in [
        ("private-preview", &preview, None),
        ("private-copy", &copied, Some(false)),
    ] {
        let response = post_json_to_session(
            router.clone(),
            &session_id,
            copy_call(
                json!(id),
                source.to_string_lossy().as_ref(),
                destination.to_string_lossy().as_ref(),
                dry_run,
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }
    assert!(!preview.exists());
    assert_eq!(
        std::fs::read_to_string(copied).unwrap(),
        "private-preflight-content"
    );

    let status = runtime_status(router, &session_id).await;
    let serialized = serde_json::to_string(&status).unwrap();
    for forbidden in [
        "private-preflight-source",
        "private-preflight-content",
        "private-preview",
        "private-copied",
        "blocked-by-response-bound",
    ] {
        assert!(!serialized.contains(forbidden));
    }
    let counters = &status["result"]["structuredContent"]["auditCounters"];
    assert_eq!(counters["by_tool"]["copy_file"]["allowed"], 2);
    assert_eq!(counters["by_tool"]["copy_file"]["denied"], 1);
    assert_eq!(counters["by_reason_code"]["dry_run_preview"]["allowed"], 1);
    assert_eq!(
        counters["by_reason_code"]["safe_root_file_copied"]["allowed"],
        1
    );
    assert_eq!(
        counters["by_reason_code"]["response_size_limit_exceeded"]["denied"],
        1
    );
}
