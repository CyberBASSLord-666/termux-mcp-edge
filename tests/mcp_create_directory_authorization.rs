#![cfg(feature = "mcp-runtime")]

mod support;

use std::{sync::Arc, time::SystemTime};

use axum::{
    body::{to_bytes, Body},
    http::{header, Method, Request, StatusCode},
};
use serde_json::{json, Value};
use support::{
    create_directory_authorized_test_router, empty_test_file_tools, initialize_session,
    issue_create_directory_grant, post_json_to_session, post_json_to_session_with_grant,
    test_router, TEST_CAPABILITY_KEY,
};
use termux_mcp_server::{
    create_directory_grant::{CreateDirectoryGrantAuthority, CREATE_DIRECTORY_GRANT_HEADER},
    mcp_transport::{MCP_PROTOCOL_VERSION, MCP_PROTOCOL_VERSION_HEADER, MCP_SESSION_ID_HEADER},
};
use tower::ServiceExt;

fn create_call(id: impl Into<Value>, path: &str, dry_run: bool) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.into(),
        "method": "tools/call",
        "params": {
            "name": "create_directory",
            "arguments": {
                "path": path,
                "dry_run": dry_run,
            },
        },
    })
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn corrupt_signature(token: &str) -> String {
    let mut token = token.as_bytes().to_vec();
    let last = token.last_mut().unwrap();
    *last = if *last == b'0' { b'1' } else { b'0' };
    String::from_utf8(token).unwrap()
}

#[tokio::test]
async fn disabled_gate_is_discoverable_only_as_dry_run_and_denies_mutation() {
    let (root, file_tools) = empty_test_file_tools();
    let target = root.path().join("disabled-mutation");
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let discovery = post_json_to_session(
        router.clone(),
        &session_id,
        json!({"jsonrpc":"2.0","id":"tools","method":"tools/list"}),
    )
    .await;
    assert_eq!(discovery.status(), StatusCode::OK);
    let discovery = response_json(discovery).await;
    let create = discovery["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "create_directory")
        .unwrap();
    assert_eq!(
        create["inputSchema"]["properties"]["dry_run"]["const"],
        true
    );
    assert!(create["description"]
        .as_str()
        .unwrap()
        .contains("mutation gate is disabled"));

    let denied = post_json_to_session(
        router.clone(),
        &session_id,
        create_call("disabled", target.to_string_lossy().as_ref(), false),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let denied = response_json(denied).await;
    assert_eq!(denied["error"]["code"], -32003);
    assert_eq!(
        denied["error"]["data"]["reason"],
        "create_directory_mutation_disabled"
    );
    assert!(!target.exists());

    let outside_root = tempfile::tempdir().unwrap();
    let outside = outside_root.path().join("disabled-outside");
    let outside_denied = post_json_to_session(
        router.clone(),
        &session_id,
        create_call(
            "disabled-outside",
            outside.to_string_lossy().as_ref(),
            false,
        ),
    )
    .await;
    assert_eq!(outside_denied.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(outside_denied).await["error"]["data"]["reason"],
        "create_directory_mutation_disabled"
    );
    assert!(!outside.exists());

    let status = post_json_to_session(
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
    let status = response_json(status).await;
    let runtime = &status["result"]["structuredContent"];
    assert_eq!(runtime["createDirectoryMutationEnabled"], false);
    assert_eq!(runtime["createDirectoryGrantRequired"], false);
    assert_eq!(
        runtime["createDirectoryMutationMode"],
        "dry_run_only_mutation_disabled"
    );
}

#[tokio::test]
async fn enabled_gate_discovery_requires_header_and_missing_grant_fails_closed() {
    let (root, file_tools) = empty_test_file_tools();
    let (router, _authority) = create_directory_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let target = root.path().join("missing-grant");

    let discovery = post_json_to_session(
        router.clone(),
        &session_id,
        json!({"jsonrpc":"2.0","id":"tools","method":"tools/list"}),
    )
    .await;
    let discovery = response_json(discovery).await;
    let create = discovery["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "create_directory")
        .unwrap();
    assert!(create["inputSchema"]["properties"]["dry_run"]
        .get("const")
        .is_none());
    assert!(create["description"]
        .as_str()
        .unwrap()
        .contains("MCP-Capability-Grant"));

    let denied = post_json_to_session(
        router.clone(),
        &session_id,
        create_call("missing", target.to_string_lossy().as_ref(), false),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let denied = response_json(denied).await;
    assert_eq!(
        denied["error"]["data"]["reason"],
        "capability_grant_missing"
    );
    assert!(!target.exists());

    let status = post_json_to_session(
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
    let status = response_json(status).await;
    let runtime = &status["result"]["structuredContent"];
    assert_eq!(runtime["createDirectoryMutationEnabled"], true);
    assert_eq!(runtime["createDirectoryGrantRequired"], true);
    assert_eq!(
        runtime["createDirectoryGrantHeader"],
        "mcp-capability-grant"
    );
    assert_eq!(runtime["createDirectoryGrantTtlSeconds"], 60);
}

#[tokio::test]
async fn malformed_expired_future_and_mismatched_grants_are_private_and_unconsumed() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let (router, authority) = create_directory_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let other_principal = CreateDirectoryGrantAuthority::from_hex_key(
        "test-key-1",
        TEST_CAPABILITY_KEY,
        "other-static-principal",
    )
    .unwrap();

    let targets = (0..9)
        .map(|index| root.path().join(format!("private-denial-{index}")))
        .collect::<Vec<_>>();
    let valid_signature = issue_create_directory_grant(
        &authority,
        &issuer_tools,
        &session_id,
        targets[2].to_string_lossy().as_ref(),
    );
    let target_for_other_path = root.path().join("private-other-binding");
    let other_target = issuer_tools
        .create_directory_grant_target(target_for_other_path.to_string_lossy().as_ref())
        .unwrap();
    let target_three = issuer_tools
        .create_directory_grant_target(targets[3].to_string_lossy().as_ref())
        .unwrap();
    let target_five = issuer_tools
        .create_directory_grant_target(targets[5].to_string_lossy().as_ref())
        .unwrap();
    let target_six = issuer_tools
        .create_directory_grant_target(targets[6].to_string_lossy().as_ref())
        .unwrap();
    let issued_now = now();
    let cases = vec![
        (None, "capability_grant_missing"),
        (Some("not-a-grant".to_owned()), "capability_grant_malformed"),
        (
            Some(corrupt_signature(&valid_signature)),
            "capability_grant_signature_invalid",
        ),
        (
            Some(
                authority
                    .issue_at(&uuid::Uuid::new_v4().to_string(), &target_three, issued_now)
                    .unwrap(),
            ),
            "capability_grant_binding_mismatch",
        ),
        (
            Some(
                authority
                    .issue_at(&session_id, &other_target, issued_now)
                    .unwrap(),
            ),
            "capability_grant_binding_mismatch",
        ),
        (
            Some(
                other_principal
                    .issue_at(&session_id, &target_five, issued_now)
                    .unwrap(),
            ),
            "capability_grant_binding_mismatch",
        ),
        (
            Some(
                authority
                    .issue_at(&session_id, &target_six, issued_now - 61)
                    .unwrap(),
            ),
            "capability_grant_expired",
        ),
        (
            Some(
                authority
                    .issue_at(
                        &session_id,
                        &issuer_tools
                            .create_directory_grant_target(targets[7].to_string_lossy().as_ref())
                            .unwrap(),
                        issued_now + 6,
                    )
                    .unwrap(),
            ),
            "capability_grant_future_issued",
        ),
        (
            Some(valid_signature.replacen("v1.", "v2.", 1)),
            "capability_grant_version_unknown",
        ),
    ];
    let mut reflected = String::new();
    for (index, (grant, expected_reason)) in cases.into_iter().enumerate() {
        let response = match grant.as_deref() {
            Some(grant) => {
                post_json_to_session_with_grant(
                    router.clone(),
                    &session_id,
                    create_call(
                        format!("denial-{index}"),
                        targets[index].to_string_lossy().as_ref(),
                        false,
                    ),
                    grant,
                )
                .await
            }
            None => {
                post_json_to_session(
                    router.clone(),
                    &session_id,
                    create_call(
                        format!("denial-{index}"),
                        targets[index].to_string_lossy().as_ref(),
                        false,
                    ),
                )
                .await
            }
        };
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "case {index}");
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], -32003);
        assert_eq!(body["error"]["data"]["reason"], expected_reason);
        reflected.push_str(&body.to_string());
        assert!(!targets[index].exists());
    }

    let status = post_json_to_session(
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
    reflected.push_str(&response_json(status).await.to_string());
    for forbidden in [
        TEST_CAPABILITY_KEY,
        "other-static-principal",
        "private-denial",
        "private-other-binding",
        valid_signature.as_str(),
    ] {
        assert!(!reflected.contains(forbidden));
    }
}

#[tokio::test]
async fn grants_are_single_use_under_replay_and_concurrent_replay() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let (router, authority) = create_directory_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let replay_target = root.path().join("replay-target");
    let replay_grant = issue_create_directory_grant(
        &authority,
        &issuer_tools,
        &session_id,
        replay_target.to_string_lossy().as_ref(),
    );
    let first = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        create_call("first", replay_target.to_string_lossy().as_ref(), false),
        &replay_grant,
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    std::fs::remove_dir(&replay_target).unwrap();
    let replay = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        create_call("replay", replay_target.to_string_lossy().as_ref(), false),
        &replay_grant,
    )
    .await;
    assert_eq!(replay.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(replay).await["error"]["data"]["reason"],
        "capability_grant_replayed"
    );

    let concurrent_target = root.path().join("concurrent-target");
    let concurrent_grant = Arc::new(issue_create_directory_grant(
        &authority,
        &issuer_tools,
        &session_id,
        concurrent_target.to_string_lossy().as_ref(),
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
                create_call(
                    format!("concurrent-{index}"),
                    target.to_string_lossy().as_ref(),
                    false,
                ),
                &grant,
            )
            .await
        });
    }
    barrier.wait().await;
    let mut statuses = Vec::new();
    while let Some(result) = calls.join_next().await {
        statuses.push(result.unwrap().status());
    }
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::OK)
            .count(),
        1
    );
    assert!(statuses.iter().all(|status| matches!(
        *status,
        StatusCode::OK | StatusCode::BAD_REQUEST | StatusCode::FORBIDDEN
    )));
    assert!(concurrent_target.is_dir());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn capability_header_is_rejected_outside_exact_tool_context_without_consumption() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let (router, authority) = create_directory_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let target = root.path().join("context-target");
    let grant = issue_create_directory_grant(
        &authority,
        &issuer_tools,
        &session_id,
        target.to_string_lossy().as_ref(),
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
    assert_eq!(response_json(wrong_context).await["error"]["code"], -32600);

    let argument_smuggling = post_json_to_session(
        router.clone(),
        &session_id,
        json!({
            "jsonrpc":"2.0",
            "id":"argument-smuggling",
            "method":"tools/call",
            "params":{
                "name":"create_directory",
                "arguments":{
                    "path":target.to_string_lossy(),
                    "dry_run":false,
                    "capability_grant":grant,
                }
            }
        }),
    )
    .await;
    assert_eq!(argument_smuggling.status(), StatusCode::BAD_REQUEST);
    let argument_smuggling = response_json(argument_smuggling).await;
    assert_eq!(argument_smuggling["error"]["code"], -32602);
    assert!(!argument_smuggling.to_string().contains(&grant));

    for method in [Method::GET, Method::DELETE] {
        let request = Request::builder()
            .method(method)
            .uri("/mcp")
            .header(header::HOST, "localhost:8000")
            .header(header::ORIGIN, "http://localhost:8000")
            .header(header::ACCEPT, "text/event-stream")
            .header(MCP_PROTOCOL_VERSION_HEADER, MCP_PROTOCOL_VERSION)
            .header(MCP_SESSION_ID_HEADER, &session_id)
            .header(CREATE_DIRECTORY_GRANT_HEADER, &grant)
            .body(Body::empty())
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response_json(response).await["error"]["code"], -32600);
    }

    let preview = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        create_call("preview", target.to_string_lossy().as_ref(), true),
        &grant,
    )
    .await;
    assert_eq!(preview.status(), StatusCode::OK);
    assert_eq!(
        response_json(preview).await["result"]["structuredContent"]["dryRun"],
        true
    );
    assert!(!target.exists());

    let allowed = post_json_to_session_with_grant(
        router,
        &session_id,
        create_call("allowed", target.to_string_lossy().as_ref(), false),
        &grant,
    )
    .await;
    assert_eq!(allowed.status(), StatusCode::OK);
    assert!(target.is_dir());
}
