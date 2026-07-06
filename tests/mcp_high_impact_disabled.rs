#![cfg(feature = "mcp-runtime")]

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    response::Response,
    Router,
};
use serde_json::{json, Value};
use tempfile::TempDir;
use termux_mcp_server::{
    mcp_transport::router, tools::FileSystemTools, transport_security::TransportSecurityPolicy,
};
use tower::ServiceExt;

fn test_file_tools() -> (TempDir, FileSystemTools) {
    let root = tempfile::tempdir().unwrap();
    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
    (root, tools)
}

fn test_router(file_tools: FileSystemTools) -> Router {
    router(TransportSecurityPolicy::localhost(8000, false), file_tools)
}

async fn post_json(request_body: Value) -> Response {
    let (_root, file_tools) = test_file_tools();
    test_router(file_tools)
        .oneshot(
            Request::post("/mcp")
                .header(header::HOST, "localhost:8000")
                .header(header::ORIGIN, "http://localhost:8000")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(request_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn response_json(response: Response) -> Value {
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn tool_discovery_does_not_expose_command_or_high_impact_surfaces() {
    let response = post_json(json!({
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
    let response = post_json(json!({
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
    assert_eq!(structured["commandExecution"], false);
    assert_eq!(structured["highImpactTools"], false);

    let text = payload["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(text.contains("android_platform=disabled"));
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
        let response = post_json(json!({
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
