#![cfg(all(feature = "mcp-runtime", feature = "command-execution"))]

mod support;

use axum::http::StatusCode;
use serde_json::json;
use support::{
    public_command_embedding_test_router, empty_test_file_tools, initialize_session, post_json_to_session,
    response_json,
};

#[tokio::test]
async fn public_embedding_cannot_discover_the_server_owned_command_lane() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = public_command_embedding_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router,
        &session_id,
        json!({
            "jsonrpc": "2.0",
            "id": "command-tools",
            "method": "tools/list"
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = response_json(response).await;
    assert!(payload["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .all(|tool| tool["name"] != "run_command_profile"));
}

#[tokio::test]
async fn public_embedding_direct_command_call_fails_closed_without_spawn_authority() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = public_command_embedding_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router,
        &session_id,
        json!({
            "jsonrpc":"2.0",
            "id":"public-command-direct",
            "method":"tools/call",
            "params": {
                "name":"run_command_profile",
                "arguments":{"profile":"server_version"}
            }
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = response_json(response).await;
    assert_eq!(payload["result"]["isError"], true);
    assert_eq!(
        payload["result"]["structuredContent"]["reasonCode"],
        "command_runtime_disabled"
    );
}

#[tokio::test]
async fn command_gate_rejects_every_caller_override_before_execution() {
    let override_cases = [
        json!({"profile": "server_version", "command": "sh -c id"}),
        json!({"profile": "server_version", "program": "/bin/sh"}),
        json!({"profile": "server_version", "argv": ["--help"]}),
        json!({"profile": "server_version", "workingDirectory": "/"}),
        json!({"profile": "server_version", "environment": {"TOKEN": "secret"}}),
        json!({"profile": "server_version", "stdin": "private"}),
        json!({"profile": "server_version", "timeout": 999}),
        json!({"profile": "server_version", "stdoutLimit": 999999}),
        json!({"profile": "server_version", "stderrLimit": 999999}),
    ];

    for (index, arguments) in override_cases.into_iter().enumerate() {
        let (_root, file_tools) = empty_test_file_tools();
        let router = public_command_embedding_test_router(file_tools);
        let session_id = initialize_session(&router).await;
        let response = post_json_to_session(
            router,
            &session_id,
            json!({
                "jsonrpc": "2.0",
                "id": format!("command-override-{index}"),
                "method": "tools/call",
                "params": {
                    "name": "run_command_profile",
                    "arguments": arguments
                }
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let payload = response_json(response).await;
        assert_eq!(payload["error"]["code"], -32602);
        assert_eq!(payload["error"]["message"], "Invalid params");
    }
}

#[tokio::test]
async fn public_embedding_reports_compiled_but_server_owned_command_lane_disabled() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = public_command_embedding_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router,
        &session_id,
        json!({
            "jsonrpc": "2.0",
            "id": "command-runtime-status",
            "method": "tools/call",
            "params": {"name": "runtime_status", "arguments": {}}
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = response_json(response).await;
    let structured = &payload["result"]["structuredContent"];
    assert_eq!(structured["commandExecutionCompiled"], true);
    assert_eq!(structured["commandExecution"], false);
    assert_eq!(structured["commandExecutionMode"], "disabled");
    assert_eq!(structured["arbitraryCommandExecution"], false);
    assert_eq!(structured["highImpactTools"], false);
}
