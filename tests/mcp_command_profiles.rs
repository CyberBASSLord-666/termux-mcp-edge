#![cfg(all(feature = "mcp-runtime", feature = "command-execution"))]

mod support;

use axum::http::StatusCode;
use serde_json::json;
use support::{
    command_test_router, empty_test_file_tools, initialize_session, post_json_to_session,
    response_json,
};

#[tokio::test]
async fn enabled_command_gate_discovers_only_closed_fixed_profile_schema() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = command_test_router(file_tools);
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
    let command = payload["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "run_command_profile")
        .unwrap();
    assert_eq!(
        command["inputSchema"]["properties"]["profile"]["enum"],
        json!(["server_version", "server_help", "execution_boundary"])
    );
    assert_eq!(command["inputSchema"]["required"], json!(["profile"]));
    assert_eq!(command["inputSchema"]["additionalProperties"], false);
    assert_eq!(
        command["inputSchema"]["properties"]
            .as_object()
            .unwrap()
            .len(),
        1
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
        let router = command_test_router(file_tools);
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
async fn runtime_status_distinguishes_fixed_profiles_from_arbitrary_execution() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = command_test_router(file_tools);
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
    assert_eq!(structured["commandExecution"], true);
    assert_eq!(
        structured["commandExecutionMode"],
        "fixed_read_only_server_diagnostics"
    );
    assert_eq!(structured["arbitraryCommandExecution"], false);
    assert_eq!(structured["highImpactTools"], false);
}
