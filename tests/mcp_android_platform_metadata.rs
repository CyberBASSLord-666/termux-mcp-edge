#![cfg(feature = "mcp-runtime")]

mod support;

use axum::http::StatusCode;
use serde_json::{json, Value};
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

    for expected_key in ["os", "arch", "family", "available_parallelism", "package_version"] {
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

fn assert_no_denied_android_platform_tokens(value: &Value) {
    let serialized = value.to_string().to_ascii_lowercase();

    for token in DENIED_ANDROID_PLATFORM_TOKENS {
        assert!(
            !serialized.contains(token),
            "unexpected Android/platform-sensitive token in metadata: {token}"
        );
    }
}
