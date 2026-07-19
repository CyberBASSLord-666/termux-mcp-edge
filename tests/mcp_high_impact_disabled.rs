#![cfg(feature = "mcp-runtime")]

mod support;

use axum::http::StatusCode;
use serde_json::json;
use support::{post_json_with_empty_root, response_json};
use termux_mcp_server::mcp_transport::MAX_MCP_JSON_RPC_ID_BYTES;

const EXPECTED_STAGED_TOOLS: [&str; 16] = [
    "runtime_status",
    "platform_info",
    "android_status",
    "project_service_status",
    "create_directory",
    "copy_file",
    "find_paths",
    "hash_file",
    "list_directory",
    "path_metadata",
    "read_binary_file",
    "read_binary_range",
    "read_file",
    "read_text_range",
    "search_text",
    "write_file",
];

#[tokio::test]
async fn tool_discovery_exposes_only_the_staged_allowlist() {
    let response = post_json_with_empty_root(json!({
        "jsonrpc": "2.0",
        "id": "list-tools",
        "method": "tools/list"
    }))
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = response_json(response).await;
    let tools = payload["result"]["tools"].as_array().unwrap();
    let tool_names = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(tool_names, EXPECTED_STAGED_TOOLS);

    for forbidden_tool in [
        "command_execute",
        "run_command",
        "shell",
        "android_control",
        "android_platform",
        "high_impact",
        "service_control",
    ] {
        assert!(
            !tool_names.contains(&forbidden_tool),
            "unexpected high-impact or command-capable tool exposed: {forbidden_tool}"
        );
    }
}

#[tokio::test]
async fn runtime_status_keeps_command_and_high_impact_gates_disabled() {
    let response = post_json_with_empty_root(json!({
        "jsonrpc": "2.0",
        "id": "runtime-status",
        "method": "tools/call",
        "params": {
            "name": "runtime_status",
            "arguments": {}
        }
    }))
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = response_json(response).await;
    let structured = &payload["result"]["structuredContent"];

    assert_eq!(structured["androidPlatformTools"], false);
    assert_eq!(
        structured["androidBatteryStatusCompiled"],
        cfg!(feature = "android-battery-status")
    );
    assert_eq!(structured["androidBatteryStatusEnabled"], false);
    assert_eq!(
        structured["androidVolumeStatusCompiled"],
        cfg!(feature = "android-volume-status")
    );
    assert_eq!(structured["androidVolumeStatusEnabled"], false);
    assert_eq!(
        structured["androidVolumeControlCompiled"],
        cfg!(feature = "android-volume-control")
    );
    assert_eq!(structured["androidVolumeControlEnabled"], false);
    assert_eq!(structured["androidVolumeControlMode"], "disabled");
    assert_eq!(structured["androidVolumeGrantRequired"], false);
    assert_eq!(structured["androidDeviceControl"], false);
    assert_eq!(structured["commandExecution"], false);
    assert_eq!(structured["highImpactTools"], false);
    assert_eq!(structured["serverSentEvents"], false);
    assert_eq!(structured["serverSentEventsMode"], "disabled");
    assert_eq!(structured["sseMaxStreamsPerSession"], 8);
    assert_eq!(structured["sseMaxEventsPerStream"], 2);
    assert_eq!(structured["sseMaxEventDataBytes"], 128 * 1024);
    assert_eq!(structured["sseMaxReplayBytesPerSession"], 256 * 1024);
    assert_eq!(structured["sseMaxLastEventIdBytes"], 64);
    assert_eq!(structured["sseRetryMilliseconds"], 1_000);
    assert_eq!(structured["jsonRpcIdMaxBytes"], MAX_MCP_JSON_RPC_ID_BYTES);

    let text = payload["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(text.contains("android_platform=disabled"));
    assert!(text.contains("android_battery_status=disabled"));
    assert!(text.contains("android_volume_status=disabled"));
    assert!(text.contains("android_volume_control=disabled"));
    assert!(text.contains("command_execution=disabled"));
}

#[tokio::test]
async fn command_capable_tool_calls_remain_method_not_found() {
    for forbidden_tool in [
        "command_execute",
        "run_command",
        "shell",
        "android_control",
        "android_platform",
        "high_impact",
        "service_control",
    ] {
        let response = post_json_with_empty_root(json!({
            "jsonrpc": "2.0",
            "id": forbidden_tool,
            "method": "tools/call",
            "params": {
                "name": forbidden_tool,
                "arguments": {}
            }
        }))
        .await;

        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
        let payload = response_json(response).await;
        assert_eq!(payload["id"], forbidden_tool);
        assert_eq!(payload["error"]["code"], -32601);
        assert_eq!(payload["error"]["message"], "Method not found");
    }
}
