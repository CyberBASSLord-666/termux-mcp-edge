#![cfg(feature = "mcp-runtime")]

mod support;

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use serde_json::{json, Value};
use support::{post_json, post_raw, response_json, test_router};
use termux_mcp_server::tools::FileSystemTools;
use tower::ServiceExt;

#[tokio::test]
async fn valid_json_with_invalid_request_shapes_returns_invalid_request() {
    for value in [json!(null), json!(true), json!(1), json!("text"), json!([]), json!([{}])] {
        let response = post_json(value).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let payload = response_json(response).await;
        assert_eq!(payload["jsonrpc"], "2.0");
        assert_eq!(payload["id"], Value::Null);
        assert_eq!(payload["error"]["code"], -32600);
        assert_eq!(payload["error"]["message"], "Invalid Request");
    }
}

#[tokio::test]
async fn requires_exact_json_rpc_version_and_string_method() {
    let cases = [
        json!({"id":"missing-version","method":"tools/list"}),
        json!({"jsonrpc":"1.0","id":"wrong-version","method":"tools/list"}),
        json!({"jsonrpc":2.0,"id":"numeric-version","method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":"missing-method"}),
        json!({"jsonrpc":"2.0","id":"numeric-method","method":7}),
    ];

    for value in cases {
        let expected_id = value["id"].clone();
        let response = post_json(value).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let payload = response_json(response).await;
        assert_eq!(payload["id"], expected_id);
        assert_eq!(payload["error"]["code"], -32600);
    }
}

#[tokio::test]
async fn rejects_invalid_mcp_request_ids_without_reflecting_them() {
    for id in [json!(null), json!(true), json!([]), json!({"private":"value"}), json!(1.5)] {
        let response = post_json(json!({
            "jsonrpc":"2.0",
            "id":id,
            "method":"tools/list"
        }))
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let payload = response_json(response).await;
        assert_eq!(payload["id"], Value::Null);
        assert_eq!(payload["error"]["code"], -32600);
        assert!(!payload.to_string().contains("private"));
    }
}

#[tokio::test]
async fn params_must_be_structured_before_method_dispatch() {
    for params in [json!(null), json!(true), json!(7), json!("value")] {
        let response = post_json(json!({
            "jsonrpc":"2.0",
            "id":"bad-params-shape",
            "method":"tools/call",
            "params":params
        }))
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let payload = response_json(response).await;
        assert_eq!(payload["id"], "bad-params-shape");
        assert_eq!(payload["error"]["code"], -32600);
    }
}

#[tokio::test]
async fn initialized_and_unknown_notifications_return_no_content() {
    for method in ["notifications/initialized", "notifications/unknown"] {
        let response = post_json(json!({
            "jsonrpc":"2.0",
            "method":method,
            "params":{}
        }))
        .await;

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(body.is_empty());
    }
}

#[tokio::test]
async fn notification_shaped_tool_call_does_not_dispatch_or_mutate() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("notification-write.txt");
    let file_tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
    let body = json!({
        "jsonrpc":"2.0",
        "method":"tools/call",
        "params":{
            "name":"write_file",
            "arguments":{
                "path":target.to_string_lossy(),
                "content":"must not be written",
                "dry_run":false
            }
        }
    });

    let response = test_router(file_tools)
        .oneshot(
            Request::post("/mcp")
                .header(header::HOST, "localhost:8000")
                .header(header::ORIGIN, "http://localhost:8000")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(!target.exists());
}

#[tokio::test]
async fn malformed_json_remains_parse_error() {
    let response = post_raw("{not-json").await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = response_json(response).await;
    assert_eq!(payload["id"], Value::Null);
    assert_eq!(payload["error"]["code"], -32700);
}
