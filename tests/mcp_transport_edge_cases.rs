#![cfg(feature = "mcp-runtime")]

mod mcp_test_harness;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use mcp_test_harness::{post_json, post_raw, response_json, test_file_tools, test_router};
use serde_json::{json, Value};
use tower::ServiceExt;

#[tokio::test]
async fn invalid_json_returns_immediate_parse_error_without_tool_dispatch() {
    let response = post_raw("not-json").await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = response_json(response).await;
    assert_eq!(payload["jsonrpc"], "2.0");
    assert_eq!(payload["id"], Value::Null);
    assert_eq!(payload["error"]["code"], -32700);
    assert_eq!(payload["error"]["message"], "Parse error");
}

#[tokio::test]
async fn valid_json_with_missing_method_returns_invalid_request_and_preserves_id() {
    let response = post_json(json!({
        "jsonrpc": "2.0",
        "id": 42,
        "params": {}
    }))
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = response_json(response).await;
    assert_eq!(payload["id"], 42);
    assert_eq!(payload["error"]["code"], -32600);
    assert_eq!(payload["error"]["message"], "Invalid Request");
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
    let data = payload["error"]["data"].as_str().unwrap();
    assert!(!data.is_empty(), "error data should list allowed methods");
    assert!(data.contains("initialize"), "should mention initialize");
    assert!(data.contains("tools/list"), "should mention tools/list");
}

#[tokio::test]
async fn invalid_origin_is_rejected_before_body_parsing() {
    let (_root, file_tools) = test_file_tools();
    let response = test_router(file_tools)
        .oneshot(
            Request::post("/mcp")
                .header(header::HOST, "localhost:8000")
                .header(header::ORIGIN, "https://example.invalid")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("not-json"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let payload = response_json(response).await;
    assert_eq!(payload["error"], "transport_security_rejected");
}
