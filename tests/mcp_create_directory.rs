#![cfg(feature = "mcp-runtime")]

mod support;

use std::os::unix::fs::{symlink, PermissionsExt};

use axum::{body::to_bytes, http::StatusCode};
use serde_json::{json, Value};
use support::{
    create_directory_authorized_test_router, empty_test_file_tools, initialize_session,
    issue_create_directory_grant, post_json_to_session, post_json_to_session_with_grant,
    test_router,
};
use termux_mcp_server::tools::{CREATE_DIRECTORY_MODE, MAX_CREATE_DIRECTORY_RESPONSE_BYTES};

fn create_call(id: Value, path: &str, dry_run: Option<bool>) -> Value {
    let mut arguments = json!({"path": path});
    if let Some(dry_run) = dry_run {
        arguments["dry_run"] = json!(dry_run);
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "create_directory",
            "arguments": arguments,
        },
    })
}

#[tokio::test]
async fn discovery_advertises_one_closed_create_directory_schema() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router,
        &session_id,
        json!({
            "jsonrpc": "2.0",
            "id": "create-directory-tools",
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
            .filter(|tool| tool["name"] == "create_directory")
            .count(),
        1
    );
    let schema = &tools
        .iter()
        .find(|tool| tool["name"] == "create_directory")
        .unwrap()["inputSchema"];
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["required"], json!(["path"]));
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"].as_object().unwrap().len(), 2);
    assert_eq!(schema["properties"]["path"]["type"], "string");
    assert_eq!(schema["properties"]["dry_run"]["type"], "boolean");
    assert_eq!(schema["properties"]["dry_run"]["const"], true);
}

#[tokio::test]
async fn create_directory_is_dry_run_first_and_explicit_mutation_is_exact() {
    let (root, file_tools) = empty_test_file_tools();
    let preview = root.path().join("preview-only");
    let created = root.path().join("created");
    let issuer_tools = file_tools.clone();
    let (router, authority) = create_directory_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let grant = issue_create_directory_grant(
        &authority,
        &issuer_tools,
        &session_id,
        created.to_string_lossy().as_ref(),
    );

    for dry_run in [None, Some(true)] {
        let response = post_json_to_session_with_grant(
            router.clone(),
            &session_id,
            create_call(
                json!(format!("preview-{dry_run:?}")),
                preview.to_string_lossy().as_ref(),
                dry_run,
            ),
            &grant,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(
            response.into_body(),
            MAX_CREATE_DIRECTORY_RESPONSE_BYTES + 1,
        )
        .await
        .unwrap();
        assert!(body.len() <= MAX_CREATE_DIRECTORY_RESPONSE_BYTES);
        let payload: Value = serde_json::from_slice(&body).unwrap();
        let structured = &payload["result"]["structuredContent"];
        assert_eq!(structured["path"], preview.to_string_lossy().as_ref());
        assert_eq!(structured["dryRun"], true);
        assert_eq!(structured["mode"], "0700");
        assert_eq!(
            structured["maxResponseBytes"],
            MAX_CREATE_DIRECTORY_RESPONSE_BYTES
        );
        assert!(!preview.exists());
    }

    let response = post_json_to_session_with_grant(
        router,
        &session_id,
        create_call(
            json!("create"),
            created.to_string_lossy().as_ref(),
            Some(false),
        ),
        &grant,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(
        response.into_body(),
        MAX_CREATE_DIRECTORY_RESPONSE_BYTES + 1,
    )
    .await
    .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["result"]["structuredContent"]["dryRun"], false);
    assert_eq!(
        std::fs::symlink_metadata(&created)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        CREATE_DIRECTORY_MODE
    );
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn create_directory_rejects_invalid_existing_and_boundary_requests() {
    let (root, file_tools) = empty_test_file_tools();
    let outside = tempfile::tempdir().unwrap();
    let existing_file = root.path().join("existing-file");
    let existing_directory = root.path().join("existing-directory");
    let linked_parent = root.path().join("linked-parent");
    let linked_target = root.path().join("linked-target");
    std::fs::write(&existing_file, "unchanged").unwrap();
    std::fs::create_dir(&existing_directory).unwrap();
    symlink(outside.path(), &linked_parent).unwrap();
    symlink(outside.path().join("redirected"), &linked_target).unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let cases = [
        None,
        Some(json!({"path": root.path().join("unknown").to_string_lossy(), "unknown": true})),
        Some(json!({"path": false})),
        Some(
            json!({"path": root.path().join("wrong-dry-run").to_string_lossy(), "dry_run": "false"}),
        ),
        Some(json!({"path": "relative"})),
        Some(json!({"path": "bad\0path"})),
        Some(json!({"path": root.path().to_string_lossy()})),
        Some(
            json!({"path": outside.path().join("outside-created").to_string_lossy(), "dry_run": false}),
        ),
        Some(
            json!({"path": root.path().join("missing").join("child").to_string_lossy(), "dry_run": false}),
        ),
        Some(json!({"path": existing_file.to_string_lossy(), "dry_run": false})),
        Some(json!({"path": existing_directory.to_string_lossy(), "dry_run": false})),
        Some(json!({"path": linked_parent.join("child").to_string_lossy(), "dry_run": false})),
        Some(json!({"path": linked_target.to_string_lossy(), "dry_run": false})),
    ];

    for (index, arguments) in cases.into_iter().enumerate() {
        let params = match arguments {
            Some(arguments) => json!({"name": "create_directory", "arguments": arguments}),
            None => json!({"name": "create_directory"}),
        };
        let response = post_json_to_session(
            router.clone(),
            &session_id,
            json!({
                "jsonrpc": "2.0",
                "id": format!("create-invalid-{index}"),
                "method": "tools/call",
                "params": params,
            }),
        )
        .await;
        let mutation_gate_denial = index >= 7;
        assert_eq!(
            response.status(),
            if mutation_gate_denial {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::BAD_REQUEST
            },
            "case {index}"
        );
        let body = to_bytes(response.into_body(), 8 * 1024).await.unwrap();
        let text = std::str::from_utf8(&body).unwrap();
        assert!(!text.contains(outside.path().to_string_lossy().as_ref()));
        let payload: Value = serde_json::from_slice(&body).unwrap();
        if mutation_gate_denial {
            assert_eq!(payload["error"]["code"], -32003);
            assert_eq!(
                payload["error"]["data"]["reason"],
                "create_directory_mutation_disabled"
            );
        } else {
            assert_eq!(payload["error"]["code"], -32602);
            assert_eq!(payload["error"]["message"], "Invalid params");
        }
    }

    assert_eq!(std::fs::read_to_string(existing_file).unwrap(), "unchanged");
    assert!(existing_directory.is_dir());
    assert!(!outside.path().join("outside-created").exists());
    assert!(!outside.path().join("child").exists());
    assert!(!outside.path().join("redirected").exists());
}

#[tokio::test]
async fn create_directory_response_bound_and_audit_counters_remain_private() {
    let (root, file_tools) = empty_test_file_tools();
    let preview = root.path().join("private-preview-name");
    let created = root.path().join("private-created-name");
    let outside = tempfile::tempdir().unwrap();
    let issuer_tools = file_tools.clone();
    let (router, authority) = create_directory_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let preview_response = post_json_to_session(
        router.clone(),
        &session_id,
        create_call(json!("preview"), preview.to_string_lossy().as_ref(), None),
    )
    .await;
    assert_eq!(preview_response.status(), StatusCode::OK);
    let grant = issue_create_directory_grant(
        &authority,
        &issuer_tools,
        &session_id,
        created.to_string_lossy().as_ref(),
    );
    let create_response = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        create_call(
            json!("create"),
            created.to_string_lossy().as_ref(),
            Some(false),
        ),
        &grant,
    )
    .await;
    assert_eq!(create_response.status(), StatusCode::OK);
    let denied_response = post_json_to_session(
        router.clone(),
        &session_id,
        create_call(
            json!("denied"),
            outside
                .path()
                .join("private-outside")
                .to_string_lossy()
                .as_ref(),
            Some(false),
        ),
    )
    .await;
    assert_eq!(denied_response.status(), StatusCode::BAD_REQUEST);

    let oversized_id = "x".repeat(MAX_CREATE_DIRECTORY_RESPONSE_BYTES);
    let oversized_response = post_json_to_session(
        router.clone(),
        &session_id,
        create_call(
            json!(oversized_id),
            root.path()
                .join("bounded-preview")
                .to_string_lossy()
                .as_ref(),
            None,
        ),
    )
    .await;
    assert_eq!(oversized_response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = to_bytes(
        oversized_response.into_body(),
        MAX_CREATE_DIRECTORY_RESPONSE_BYTES + 1,
    )
    .await
    .unwrap();
    assert!(body.len() <= MAX_CREATE_DIRECTORY_RESPONSE_BYTES);
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["id"], Value::Null);
    assert_eq!(payload["error"]["code"], -32001);

    let bounded_mutation = root.path().join("bounded-mutation");
    let oversized_mutation_response = post_json_to_session(
        router.clone(),
        &session_id,
        create_call(
            json!("y".repeat(MAX_CREATE_DIRECTORY_RESPONSE_BYTES)),
            bounded_mutation.to_string_lossy().as_ref(),
            Some(false),
        ),
    )
    .await;
    assert_eq!(
        oversized_mutation_response.status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert!(!bounded_mutation.exists());

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
        "private-preview-name",
        "private-created-name",
        "private-outside",
    ] {
        assert!(!text.contains(forbidden));
    }
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let counters = &payload["result"]["structuredContent"]["auditCounters"];
    assert_eq!(counters["by_tool"]["create_directory"]["allowed"], 2);
    assert_eq!(counters["by_tool"]["create_directory"]["denied"], 3);
    assert_eq!(counters["by_reason_code"]["dry_run_preview"]["allowed"], 1);
    assert_eq!(
        counters["by_reason_code"]["safe_root_directory_created"]["allowed"],
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
