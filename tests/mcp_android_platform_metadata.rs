#![cfg(feature = "mcp-runtime")]

mod support;

use axum::http::StatusCode;
use serde_json::{json, Value};
#[cfg(any(
    not(feature = "android-battery-status"),
    not(feature = "android-volume-status")
))]
use support::{empty_test_file_tools, initialize_session, post_json_to_session, test_router};
use support::{post_json_with_empty_root, response_json};

const DENIED_ANDROID_PLATFORM_TOKENS: [&str; 22] = [
    "android_id",
    "advertising_id",
    "serial",
    "imei",
    "imsi",
    "subscriber",
    "phone_number",
    "accounts",
    "contacts",
    "sms",
    "notification",
    "location",
    "latitude",
    "longitude",
    "camera",
    "microphone",
    "accessibility",
    "installed_packages",
    "package_inventory",
    "processes",
    "shell",
    "command_output",
];

#[tokio::test]
async fn android_status_metadata_stays_read_only_and_non_sensitive() {
    let response = post_json_with_empty_root(json!({
        "jsonrpc": "2.0",
        "id": "android-status",
        "method": "tools/call",
        "params": {
            "name": "android_status",
            "arguments": {}
        }
    }))
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    let structured = body
        .pointer("/result/structuredContent")
        .expect("android_status returns structured content");

    assert_eq!(structured["status_mode"], "read_only_allowlisted_status");
    assert_eq!(structured["android_api_access"], "not_used");
    assert_eq!(structured["android_control_enabled"], false);
    assert_eq!(structured["shell_fallback_enabled"], false);
    assert_eq!(structured["command_execution_enabled"], false);
    assert_eq!(structured["high_impact_controls_enabled"], false);
    assert_no_denied_android_platform_tokens(structured);
}

#[tokio::test]
async fn android_status_rejects_argument_expansion() {
    let response = post_json_with_empty_root(json!({
        "jsonrpc": "2.0",
        "id": "android-status-extra-args",
        "method": "tools/call",
        "params": {
            "name": "android_status",
            "arguments": {
                "include_packages": true
            }
        }
    }))
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response_json(response).await;
    assert_eq!(body["error"]["code"], -32602);
    assert_eq!(body["error"]["message"], "Invalid params");
}

#[tokio::test]
async fn platform_info_metadata_stays_non_sensitive() {
    let response = post_json_with_empty_root(json!({
        "jsonrpc": "2.0",
        "id": "platform-info",
        "method": "tools/call",
        "params": {
            "name": "platform_info",
            "arguments": {}
        }
    }))
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    let structured = body
        .pointer("/result/structuredContent")
        .expect("platform_info returns structured content");

    for expected_key in [
        "os",
        "arch",
        "family",
        "available_parallelism",
        "package_version",
    ] {
        assert!(
            structured.get(expected_key).is_some(),
            "missing expected platform metadata key: {expected_key}"
        );
    }

    for forbidden_key in [
        "env",
        "environment",
        "home",
        "path",
        "processes",
        "shell",
        "username",
        "hostname",
        "android_id",
    ] {
        assert_eq!(
            structured.get(forbidden_key),
            None,
            "unexpected sensitive platform metadata key: {forbidden_key}"
        );
    }

    assert_no_denied_android_platform_tokens(structured);
}

#[cfg(not(feature = "android-battery-status"))]
#[tokio::test]
async fn battery_tool_fails_closed_when_the_compile_feature_is_absent() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router.clone(),
        &session_id,
        json!({
            "jsonrpc": "2.0",
            "id": "battery-feature-disabled",
            "method": "tools/call",
            "params": {
                "name": "android_battery_status",
                "arguments": {}
            }
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["result"]["isError"], true);
    assert_eq!(
        body["result"]["structuredContent"]["reasonCode"],
        "battery_feature_not_compiled"
    );

    let runtime = post_json_to_session(
        router,
        &session_id,
        json!({
            "jsonrpc": "2.0",
            "id": "battery-feature-disabled-runtime",
            "method": "tools/call",
            "params": {"name": "runtime_status", "arguments": {}}
        }),
    )
    .await;
    let runtime = response_json(runtime).await;
    let structured = &runtime["result"]["structuredContent"];
    assert_eq!(structured["androidBatteryStatusCompiled"], false);
    assert_eq!(structured["androidBatteryStatusEnabled"], false);
    assert_eq!(
        structured["auditCounters"]["by_tool"]["android_battery_status"]["denied"],
        1
    );
    assert_eq!(
        structured["auditCounters"]["by_reason_code"]["battery_feature_not_compiled"]["denied"],
        1
    );
}

#[cfg(not(feature = "android-volume-status"))]
#[tokio::test]
async fn volume_tool_fails_closed_when_the_compile_feature_is_absent() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router.clone(),
        &session_id,
        json!({
            "jsonrpc": "2.0",
            "id": "volume-feature-disabled",
            "method": "tools/call",
            "params": {
                "name": "android_volume_status",
                "arguments": {}
            }
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["result"]["isError"], true);
    assert_eq!(
        body["result"]["structuredContent"]["reasonCode"],
        "volume_feature_not_compiled"
    );

    let runtime = post_json_to_session(
        router,
        &session_id,
        json!({
            "jsonrpc": "2.0",
            "id": "volume-feature-disabled-runtime",
            "method": "tools/call",
            "params": {"name": "runtime_status", "arguments": {}}
        }),
    )
    .await;
    let runtime = response_json(runtime).await;
    let structured = &runtime["result"]["structuredContent"];
    assert_eq!(structured["androidVolumeStatusCompiled"], false);
    assert_eq!(structured["androidVolumeStatusEnabled"], false);
    assert_eq!(
        structured["auditCounters"]["by_tool"]["android_volume_status"]["denied"],
        1
    );
    assert_eq!(
        structured["auditCounters"]["by_reason_code"]["volume_feature_not_compiled"]["denied"],
        1
    );
}

fn assert_no_denied_android_platform_tokens(value: &Value) {
    assert_no_denied_android_platform_tokens_at(value, "$".to_owned());
}

fn assert_no_denied_android_platform_tokens_at(value: &Value, path: String) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let key_lower = key.to_ascii_lowercase();
                assert!(
                    !DENIED_ANDROID_PLATFORM_TOKENS.contains(&key_lower.as_str()),
                    "unexpected Android/platform-sensitive metadata key at {path}.{key}: {key}"
                );
                assert_no_denied_android_platform_tokens_at(child, format!("{path}.{key}"));
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                assert_no_denied_android_platform_tokens_at(child, format!("{path}[{index}]"));
            }
        }
        Value::String(text) => {
            let value_lower = text.to_ascii_lowercase();
            assert!(
                !DENIED_ANDROID_PLATFORM_TOKENS.contains(&value_lower.as_str()),
                "unexpected Android/platform-sensitive metadata value at {path}: {text}"
            );
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}
