#![cfg(feature = "mcp-runtime")]

mod support;

use axum::http::StatusCode;
use serde_json::{json, Value};
use support::{post_json_with_empty_root, response_json};

const HIGH_IMPACT_TOOL_NAMES: [&str; 14] = [
    "android_control",
    "android_platform",
    "command_execute",
    "delete_file",
    "exec",
    "high_impact",
    "install_package",
    "kill_process",
    "process_list",
    "run_command",
    "service_control",
    "shell",
    "spawn_process",
    "uninstall_package",
];

#[tokio::test]
async fn tool_discovery_omits_final_high_impact_surfaces() {
    let response = post_json_with_empty_root(json!({
        "jsonrpc": "2.0",
        "id": "final-high-impact-tools-list",
        "method": "tools/list"
    }))
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    let tools = body["result"]["tools"]
        .as_array()
        .expect("tools/list returns a tools array");

    for tool in tools {
        let tool_name = tool["name"]
            .as_str()
            .expect("every discovered tool has a string name");
        assert!(
            !HIGH_IMPACT_TOOL_NAMES.contains(&tool_name),
            "unexpected high-impact tool exposed by discovery: {tool_name}"
        );
        assert_no_exact_high_impact_tokens(tool);
    }
}

#[tokio::test]
async fn high_impact_tool_calls_stay_unavailable_even_with_dangerous_arguments() {
    for tool_name in HIGH_IMPACT_TOOL_NAMES {
        let response = post_json_with_empty_root(json!({
            "jsonrpc": "2.0",
            "id": tool_name,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": {
                    "command": "id",
                    "path": "/data/data/com.termux/files/home",
                    "service_name": "termux-mcp-edge",
                    "package": "com.termux",
                    "dry_run": false
                }
            }
        }))
        .await;

        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
        let body = response_json(response).await;
        assert_eq!(body["id"], tool_name);
        assert_eq!(body["error"]["code"], -32601);
        assert_eq!(body["error"]["message"], "Method not found");
    }
}

#[tokio::test]
async fn runtime_status_keeps_final_high_impact_stage_closed() {
    let response = post_json_with_empty_root(json!({
        "jsonrpc": "2.0",
        "id": "final-high-impact-runtime-status",
        "method": "tools/call",
        "params": {
            "name": "runtime_status",
            "arguments": {}
        }
    }))
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    let structured = &body["result"]["structuredContent"];

    assert_eq!(structured["androidPlatformTools"], false);
    assert_eq!(structured["androidBatteryStatusEnabled"], false);
    assert_eq!(structured["androidVolumeStatusEnabled"], false);
    assert_eq!(structured["androidDeviceControl"], false);
    assert_eq!(structured["commandExecution"], false);
    assert_eq!(structured["highImpactTools"], false);
}

fn assert_no_exact_high_impact_tokens(value: &Value) {
    assert_no_exact_high_impact_tokens_at(value, "$".to_owned());
}

fn assert_no_exact_high_impact_tokens_at(value: &Value, path: String) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                assert!(
                    !HIGH_IMPACT_TOOL_NAMES.contains(&key.as_str()),
                    "unexpected high-impact schema key at {path}.{key}: {key}"
                );
                assert_no_exact_high_impact_tokens_at(child, format!("{path}.{key}"));
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                assert_no_exact_high_impact_tokens_at(child, format!("{path}[{index}]"));
            }
        }
        Value::String(text) => {
            assert!(
                !HIGH_IMPACT_TOOL_NAMES.contains(&text.as_str()),
                "unexpected high-impact schema value at {path}: {text}"
            );
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}
