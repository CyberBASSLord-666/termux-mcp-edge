#![cfg(feature = "mcp-runtime")]

mod support;

use std::{
    os::unix::fs::{symlink, PermissionsExt},
    sync::Arc,
};

use axum::{body::to_bytes, http::StatusCode};
use serde_json::{json, Value};
use support::{
    empty_test_file_tools, initialize_session, issue_write_file_grant, post_json_to_session,
    post_json_to_session_with_grant, test_router, write_file_authorized_test_router,
    TEST_CAPABILITY_KEY,
};
use termux_mcp_server::{
    create_directory_grant::CreateDirectoryGrantAuthority,
    tools::{MAX_WRITE_FILE_RESPONSE_BYTES, WRITE_FILE_MODE},
    write_file_grant::{content_sha256, WriteFileGrantAuthority},
};

fn write_call(id: impl Into<Value>, path: &str, content: &str, dry_run: Option<bool>) -> Value {
    let mut arguments = json!({"path": path, "content": content});
    if let Some(dry_run) = dry_run {
        arguments["dry_run"] = json!(dry_run);
    }
    json!({
        "jsonrpc": "2.0",
        "id": id.into(),
        "method": "tools/call",
        "params": {
            "name": "write_file",
            "arguments": arguments,
        },
    })
}

async fn response_json(response: axum::response::Response, limit: usize) -> Value {
    let body = to_bytes(response.into_body(), limit).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn corrupt_signature(token: &str) -> String {
    let mut token = token.as_bytes().to_vec();
    let last = token.last_mut().unwrap();
    *last = if *last == b'0' { b'1' } else { b'0' };
    String::from_utf8(token).unwrap()
}

fn assert_no_write_staging_entries(root: &std::path::Path) {
    for entry in std::fs::read_dir(root).unwrap() {
        let name = entry.unwrap().file_name();
        assert!(
            !name.to_string_lossy().starts_with(".termux-mcp-write-file-"),
            "operation-owned staging entry leaked"
        );
    }
}

#[tokio::test]
async fn disabled_gate_is_dry_run_only_and_denies_before_filesystem_resolution() {
    let (root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let discovery = post_json_to_session(
        router.clone(),
        &session_id,
        json!({"jsonrpc":"2.0","id":"tools","method":"tools/list"}),
    )
    .await;
    assert_eq!(discovery.status(), StatusCode::OK);
    let discovery = response_json(discovery, 128 * 1024).await;
    let write = discovery["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "write_file")
        .unwrap();
    assert_eq!(write["inputSchema"]["properties"]["dry_run"]["const"], true);
    assert!(write["description"]
        .as_str()
        .unwrap()
        .contains("mutation gate is disabled"));

    let target = root.path().join("disabled.txt");
    let denied = post_json_to_session(
        router.clone(),
        &session_id,
        write_call(
            "disabled",
            target.to_string_lossy().as_ref(),
            "private-content",
            Some(false),
        ),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let denied = response_json(denied, 16 * 1024).await;
    assert_eq!(denied["error"]["code"], -32003);
    assert_eq!(
        denied["error"]["data"]["reason"],
        "write_file_mutation_disabled"
    );
    assert!(!target.exists());
    assert!(!denied.to_string().contains("private-content"));

    let outside = tempfile::tempdir().unwrap().path().join("outside.txt");
    let outside_denied = post_json_to_session(
        router.clone(),
        &session_id,
        write_call(
            "disabled-outside",
            outside.to_string_lossy().as_ref(),
            "secret",
            Some(false),
        ),
    )
    .await;
    assert_eq!(outside_denied.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(outside_denied, 16 * 1024).await["error"]["data"]["reason"],
        "write_file_mutation_disabled"
    );

    let runtime = post_json_to_session(
        router,
        &session_id,
        json!({
            "jsonrpc":"2.0",
            "id":"status",
            "method":"tools/call",
            "params":{"name":"runtime_status","arguments":{}}
        }),
    )
    .await;
    let runtime = response_json(runtime, 128 * 1024).await;
    let structured = &runtime["result"]["structuredContent"];
    assert_eq!(structured["fileWriteMutationEnabled"], false);
    assert_eq!(structured["fileWriteGrantRequired"], false);
    assert_eq!(structured["fileWriteMode"], "dry_run_only_mutation_disabled");
    assert_eq!(structured["fileWriteMaxBytes"], 1_048_576);
    assert_eq!(
        structured["fileWriteMaxResponseBytes"],
        MAX_WRITE_FILE_RESPONSE_BYTES
    );
}

#[tokio::test]
async fn enabled_gate_discovers_grant_contract_and_valid_create_replace_are_exact() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let (router, authority) = write_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let discovery = post_json_to_session(
        router.clone(),
        &session_id,
        json!({"jsonrpc":"2.0","id":"tools","method":"tools/list"}),
    )
    .await;
    let discovery = response_json(discovery, 128 * 1024).await;
    let write = discovery["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "write_file")
        .unwrap();
    assert!(write["inputSchema"]["properties"]["dry_run"]
        .get("const")
        .is_none());
    assert!(write["description"]
        .as_str()
        .unwrap()
        .contains("MCP-Capability-Grant"));

    let target = root.path().join("authorized.txt");
    let create_content = "created content";
    let create_grant = issue_write_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        target.to_string_lossy().as_ref(),
        create_content,
    );
    let create = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        write_call(
            "create",
            target.to_string_lossy().as_ref(),
            create_content,
            Some(false),
        ),
        &create_grant,
    )
    .await;
    assert_eq!(create.status(), StatusCode::OK);
    let create = response_json(create, MAX_WRITE_FILE_RESPONSE_BYTES + 1).await;
    assert_eq!(create["result"]["structuredContent"]["dryRun"], false);
    assert_eq!(
        create["result"]["structuredContent"]["bytes"],
        create_content.len()
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), create_content);
    assert_eq!(
        std::fs::symlink_metadata(&target)
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        WRITE_FILE_MODE
    );

    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o666)).unwrap();
    let replace_content = "replacement content";
    let replace_grant = issue_write_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        target.to_string_lossy().as_ref(),
        replace_content,
    );
    let replace = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        write_call(
            "replace",
            target.to_string_lossy().as_ref(),
            replace_content,
            Some(false),
        ),
        &replace_grant,
    )
    .await;
    assert_eq!(replace.status(), StatusCode::OK);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), replace_content);
    assert_eq!(
        std::fs::symlink_metadata(&target)
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        WRITE_FILE_MODE
    );
    assert_no_write_staging_entries(root.path());

    let runtime = post_json_to_session(
        router,
        &session_id,
        json!({
            "jsonrpc":"2.0",
            "id":"status",
            "method":"tools/call",
            "params":{"name":"runtime_status","arguments":{}}
        }),
    )
    .await;
    let runtime = response_json(runtime, 128 * 1024).await;
    let structured = &runtime["result"]["structuredContent"];
    assert_eq!(structured["fileWriteMutationEnabled"], true);
    assert_eq!(structured["fileWriteGrantRequired"], true);
    assert_eq!(structured["fileWriteGrantHeader"], "mcp-capability-grant");
    assert_eq!(structured["fileWriteGrantTtlSeconds"], 60);
    assert_eq!(
        structured["fileWriteMode"],
        "dry_run_or_request_scoped_single_use_grant"
    );
    assert_eq!(structured["highImpactTools"], false);
}

#[tokio::test]
async fn missing_malformed_and_binding_mismatched_grants_are_private_and_unconsumed() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let (router, authority) = write_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let target = root.path().join("private-target.txt");
    let content = "private-content-value";
    let valid = issue_write_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        target.to_string_lossy().as_ref(),
        content,
    );

    let missing = post_json_to_session(
        router.clone(),
        &session_id,
        write_call(
            "missing",
            target.to_string_lossy().as_ref(),
            content,
            Some(false),
        ),
    )
    .await;
    assert_eq!(missing.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(missing, 16 * 1024).await["error"]["data"]["reason"],
        "capability_grant_missing"
    );

    for (index, (grant, path, attempted_content, expected_reason)) in [
        (
            "not-a-grant".to_owned(),
            target.clone(),
            content,
            "capability_grant_malformed",
        ),
        (
            corrupt_signature(&valid),
            target.clone(),
            content,
            "capability_grant_signature_invalid",
        ),
        (
            valid.clone(),
            root.path().join("other-target.txt"),
            content,
            "capability_grant_binding_mismatch",
        ),
        (
            valid.clone(),
            target.clone(),
            "different-content",
            "capability_grant_binding_mismatch",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let denied = post_json_to_session_with_grant(
            router.clone(),
            &session_id,
            write_call(
                format!("private-{index}"),
                path.to_string_lossy().as_ref(),
                attempted_content,
                Some(false),
            ),
            &grant,
        )
        .await;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN, "case {index}");
        let body = response_json(denied, 16 * 1024).await;
        assert_eq!(body["error"]["data"]["reason"], expected_reason);
        let serialized = body.to_string();
        assert!(!serialized.contains(content));
        assert!(!serialized.contains(&grant));
    }
    assert!(!target.exists());

    // A create grant cannot authorize replacement. Binding failure must not
    // consume it, so restoring the original create posture permits one retry.
    std::fs::write(&target, "interposed").unwrap();
    let posture_denied = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        write_call(
            "posture",
            target.to_string_lossy().as_ref(),
            content,
            Some(false),
        ),
        &valid,
    )
    .await;
    assert_eq!(posture_denied.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(posture_denied, 16 * 1024).await["error"]["data"]["reason"],
        "capability_grant_binding_mismatch"
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "interposed");
    std::fs::remove_file(&target).unwrap();

    let allowed = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        write_call(
            "allowed",
            target.to_string_lossy().as_ref(),
            content,
            Some(false),
        ),
        &valid,
    )
    .await;
    assert_eq!(allowed.status(), StatusCode::OK);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), content);

    let other_session = uuid::Uuid::new_v4().to_string();
    let other_target = root.path().join("other-session.txt");
    let other_session_target = issuer_tools
        .write_file_grant_target(
            other_target.to_string_lossy().as_ref(),
            content_sha256(content.as_bytes()),
        )
        .unwrap();
    let other_session_grant = authority
        .issue_at(
            &other_session,
            &other_session_target,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
        .unwrap();
    let session_denied = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        write_call(
            "other-session",
            other_target.to_string_lossy().as_ref(),
            content,
            Some(false),
        ),
        &other_session_grant,
    )
    .await;
    assert_eq!(session_denied.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(session_denied, 16 * 1024).await["error"]["data"]["reason"],
        "capability_grant_binding_mismatch"
    );

    let other_principal = WriteFileGrantAuthority::from_hex_key(
        "test-key-1",
        TEST_CAPABILITY_KEY,
        "other-private-principal",
    )
    .unwrap();
    let principal_target = root.path().join("other-principal.txt");
    let principal_binding = issuer_tools
        .write_file_grant_target(
            principal_target.to_string_lossy().as_ref(),
            content_sha256(content.as_bytes()),
        )
        .unwrap();
    let other_principal_grant = other_principal
        .issue_at(
            &session_id,
            &principal_binding,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
        .unwrap();
    let principal_denied = post_json_to_session_with_grant(
        router,
        &session_id,
        write_call(
            "other-principal",
            principal_target.to_string_lossy().as_ref(),
            content,
            Some(false),
        ),
        &other_principal_grant,
    )
    .await;
    assert_eq!(principal_denied.status(), StatusCode::FORBIDDEN);
    let principal_denied = response_json(principal_denied, 16 * 1024).await;
    assert_eq!(
        principal_denied["error"]["data"]["reason"],
        "capability_grant_binding_mismatch"
    );
    let serialized = principal_denied.to_string();
    assert!(!serialized.contains("other-private-principal"));
    assert!(!serialized.contains(&other_principal_grant));
}

#[tokio::test]
async fn dry_run_and_response_preflight_do_not_consume_or_mutate() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let (router, authority) = write_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let target = root.path().join("preflight.txt");
    let content = "preflight-content";
    let grant = issue_write_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        target.to_string_lossy().as_ref(),
        content,
    );

    let preview = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        write_call(
            "preview",
            target.to_string_lossy().as_ref(),
            content,
            None,
        ),
        &grant,
    )
    .await;
    assert_eq!(preview.status(), StatusCode::OK);
    assert_eq!(
        response_json(preview, MAX_WRITE_FILE_RESPONSE_BYTES + 1).await["result"]
            ["structuredContent"]["dryRun"],
        true
    );
    assert!(!target.exists());

    let oversized_id = "x".repeat(MAX_WRITE_FILE_RESPONSE_BYTES);
    let bounded = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        write_call(
            oversized_id,
            target.to_string_lossy().as_ref(),
            content,
            Some(false),
        ),
        &grant,
    )
    .await;
    assert_eq!(bounded.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let bounded = response_json(bounded, MAX_WRITE_FILE_RESPONSE_BYTES + 1).await;
    assert_eq!(bounded["id"], Value::Null);
    assert_eq!(bounded["error"]["code"], -32001);
    assert!(!target.exists());

    let allowed = post_json_to_session_with_grant(
        router,
        &session_id,
        write_call(
            "allowed",
            target.to_string_lossy().as_ref(),
            content,
            Some(false),
        ),
        &grant,
    )
    .await;
    assert_eq!(allowed.status(), StatusCode::OK);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), content);
    assert_no_write_staging_entries(root.path());
}

#[tokio::test]
async fn grant_is_single_use_under_sequential_and_concurrent_replay() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let (router, authority) = write_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let target = root.path().join("replay.txt");
    let grant = issue_write_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        target.to_string_lossy().as_ref(),
        "once",
    );
    let first = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        write_call(
            "first",
            target.to_string_lossy().as_ref(),
            "once",
            Some(false),
        ),
        &grant,
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    std::fs::remove_file(&target).unwrap();
    let replay = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        write_call(
            "replay",
            target.to_string_lossy().as_ref(),
            "once",
            Some(false),
        ),
        &grant,
    )
    .await;
    assert_eq!(replay.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(replay, 16 * 1024).await["error"]["data"]["reason"],
        "capability_grant_replayed"
    );

    let concurrent_target = root.path().join("concurrent.txt");
    let concurrent_grant = Arc::new(issue_write_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        concurrent_target.to_string_lossy().as_ref(),
        "concurrent-content",
    ));
    let barrier = Arc::new(tokio::sync::Barrier::new(9));
    let mut calls = tokio::task::JoinSet::new();
    for index in 0..8 {
        let router = router.clone();
        let session_id = session_id.clone();
        let target = concurrent_target.clone();
        let grant = Arc::clone(&concurrent_grant);
        let barrier = Arc::clone(&barrier);
        calls.spawn(async move {
            barrier.wait().await;
            post_json_to_session_with_grant(
                router,
                &session_id,
                write_call(
                    format!("concurrent-{index}"),
                    target.to_string_lossy().as_ref(),
                    "concurrent-content",
                    Some(false),
                ),
                &grant,
            )
            .await
        });
    }
    barrier.wait().await;
    let mut responses = Vec::new();
    while let Some(result) = calls.join_next().await {
        responses.push(result.unwrap());
    }
    assert_eq!(
        responses
            .iter()
            .filter(|response| response.status() == StatusCode::OK)
            .count(),
        1
    );
    assert!(responses.iter().all(|response| matches!(
        response.status(),
        StatusCode::OK | StatusCode::BAD_REQUEST | StatusCode::FORBIDDEN
    )));
    assert_eq!(
        std::fs::read_to_string(&concurrent_target).unwrap(),
        "concurrent-content"
    );
    assert_no_write_staging_entries(root.path());
}

#[tokio::test]
async fn capability_header_is_confined_to_exact_tool_context() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let (router, authority) = write_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let target = root.path().join("context.txt");
    let grant = issue_write_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        target.to_string_lossy().as_ref(),
        "context-content",
    );

    let wrong_context = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        json!({
            "jsonrpc":"2.0",
            "id":"wrong-context",
            "method":"tools/call",
            "params":{"name":"runtime_status","arguments":{}}
        }),
        &grant,
    )
    .await;
    assert_eq!(wrong_context.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(wrong_context, 16 * 1024).await["error"]["code"],
        -32600
    );

    let smuggled = post_json_to_session(
        router.clone(),
        &session_id,
        json!({
            "jsonrpc":"2.0",
            "id":"smuggled",
            "method":"tools/call",
            "params":{
                "name":"write_file",
                "arguments":{
                    "path":target.to_string_lossy(),
                    "content":"context-content",
                    "dry_run":false,
                    "capability_grant":grant,
                }
            }
        }),
    )
    .await;
    assert_eq!(smuggled.status(), StatusCode::BAD_REQUEST);
    let smuggled = response_json(smuggled, 16 * 1024).await;
    assert_eq!(smuggled["error"]["code"], -32602);
    assert!(!smuggled.to_string().contains(&grant));
    assert!(!target.exists());

    let allowed = post_json_to_session_with_grant(
        router,
        &session_id,
        write_call(
            "allowed",
            target.to_string_lossy().as_ref(),
            "context-content",
            Some(false),
        ),
        &grant,
    )
    .await;
    assert_eq!(allowed.status(), StatusCode::OK);
    assert_eq!(
        std::fs::read_to_string(target).unwrap(),
        "context-content"
    );
}

#[tokio::test]
async fn other_capability_family_grant_cannot_authorize_write_file() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let (router, _write_authority) = write_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let directory_target = root.path().join("directory-capability-target");
    let write_target = root.path().join("write-capability-target.txt");
    let create_authority = CreateDirectoryGrantAuthority::from_hex_key(
        "test-key-1",
        TEST_CAPABILITY_KEY,
        support::TEST_STATIC_PRINCIPAL,
    )
    .unwrap();
    let create_binding = issuer_tools
        .create_directory_grant_target(directory_target.to_string_lossy().as_ref())
        .unwrap();
    let create_grant = create_authority
        .issue_at(
            &session_id,
            &create_binding,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
        .unwrap();

    let denied = post_json_to_session_with_grant(
        router,
        &session_id,
        write_call(
            "cross-capability",
            write_target.to_string_lossy().as_ref(),
            "must-not-write",
            Some(false),
        ),
        &create_grant,
    )
    .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(denied, 16 * 1024).await["error"]["data"]["reason"],
        "capability_grant_binding_mismatch"
    );
    assert!(!directory_target.exists());
    assert!(!write_target.exists());
}

#[tokio::test]
async fn exact_limit_succeeds_over_limit_fails_and_unsafe_types_are_preserved() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let (router, authority) = write_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let exact_target = root.path().join("exact-limit.txt");
    let exact_content = "x".repeat(1_048_576);
    let exact_grant = issue_write_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        exact_target.to_string_lossy().as_ref(),
        &exact_content,
    );
    let exact = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        write_call(
            "exact-limit",
            exact_target.to_string_lossy().as_ref(),
            &exact_content,
            Some(false),
        ),
        &exact_grant,
    )
    .await;
    assert_eq!(exact.status(), StatusCode::OK);
    assert_eq!(std::fs::metadata(&exact_target).unwrap().len(), 1_048_576);

    let over_target = root.path().join("over-limit.txt");
    let over_content = "y".repeat(1_048_577);
    let over_grant = issue_write_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        over_target.to_string_lossy().as_ref(),
        &over_content,
    );
    let over = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        write_call(
            "over-limit",
            over_target.to_string_lossy().as_ref(),
            &over_content,
            Some(false),
        ),
        &over_grant,
    )
    .await;
    assert_eq!(over.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(!over_target.exists());

    let outside = tempfile::tempdir().unwrap();
    let outside_target = outside.path().join("outside.txt");
    std::fs::write(&outside_target, "outside-safe").unwrap();
    let link = root.path().join("linked.txt");
    symlink(&outside_target, &link).unwrap();
    let directory = root.path().join("directory-target");
    std::fs::create_dir(&directory).unwrap();

    for (index, path) in [link, directory].into_iter().enumerate() {
        let denied = post_json_to_session(
            router.clone(),
            &session_id,
            write_call(
                format!("unsafe-{index}"),
                path.to_string_lossy().as_ref(),
                "must-not-write",
                Some(false),
            ),
        )
        .await;
        assert!(matches!(
            denied.status(),
            StatusCode::BAD_REQUEST | StatusCode::FORBIDDEN
        ));
    }
    assert_eq!(std::fs::read_to_string(outside_target).unwrap(), "outside-safe");
    assert_no_write_staging_entries(root.path());
}
