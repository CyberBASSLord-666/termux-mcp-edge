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
    std::fs::write(root.path().join("visible.txt"), "safe content").unwrap();
    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
    (root, tools)
}

fn test_router(file_tools: FileSystemTools) -> Router {
    router(TransportSecurityPolicy::localhost(8000, false), file_tools)
}

async fn post_raw(body: impl Into<Body>) -> Response {
    let (_root, file_tools) = test_file_tools();
    test_router(file_tools)
        .oneshot(
            Request::post("/mcp")
                .header(header::HOST, "localhost:8000")
                .header(header::ORIGIN, "http://localhost:8000")
                .header(header::CONTENT_TYPE, "application/json")
                .body(body.into())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn post_json(request_body: Value) -> Response {
    post_raw(request_body.to_string()).await
}

async fn response_json(response: Response) -> Value {
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn malformed_json_returns_immediate_parse_error_without_tool_dispatch() {
    let response = post_raw("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":").await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = response_json(response).await;
    assert_eq!(payload["jsonrpc"], "2.0");
    assert_eq!(payload["id"], Value::Null);
    assert_eq!(payload["error"]["code"], -32700);
    assert_eq!(payload["error"]["message"], "Parse error");
}

#[tokio::test]
async fn valid_json_with_missing_method_returns_safe_parse_error() {
    let response = post_json(json!({
        "jsonrpc": "2.0",
        "id": 42,
        "params": {}
    }))
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = response_json(response).await;
    assert_eq!(payload["id"], Value::Null);
    assert_eq!(payload["error"]["code"], -32700);
    assert_eq!(payload["error"]["message"], "Parse error");
}

#[tokio::test]
async fn invalid_tools_call_params_return_bounded_invalid_params_response() {
    let response = post_json(json!({
        "jsonrpc": "2.0",
        "id": "bad-params",
        "method": "tools/call",
        "params": ["not", "an", "object"]
    }))
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = response_json(response).await;
    assert_eq!(payload["id"], "bad-params");
    assert_eq!(payload["error"]["code"], -32602);
    assert_eq!(payload["error"]["message"], "Invalid params");
}

#[tokio::test]
async fn unknown_method_returns_safe_method_not_found_without_runtime_expansion() {
    let response = post_json(json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "resources/list"
    }))
    .await;

    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    let payload = response_json(response).await;
    assert_eq!(payload["id"], 7);
    assert_eq!(payload["error"]["code"], -32601);
    assert_eq!(payload["error"]["message"], "Method not found");
    assert!(payload["error"]["data"]
        .as_str()
        .unwrap()
        .contains("Only initialize, tools/list"));
}

#[tokio::test]
async fn invalid_origin_is_rejected_before_malformed_body_parsing() {
    let (_root, file_tools) = test_file_tools();
    let response = test_router(file_tools)
        .oneshot(
            Request::post("/mcp")
                .header(header::HOST, "localhost:8000")
                .header(header::ORIGIN, "https://evil.example")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let payload = response_json(response).await;
    assert_eq!(payload["error"], "transport_security_rejected");
}
