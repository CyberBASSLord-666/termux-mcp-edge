#![cfg(feature = "mcp-runtime")]

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    middleware,
    response::Response,
    Router,
};
use serde_json::{json, Value};
use termux_mcp_server::{
    auth::{require_mcp_auth, McpAuthPolicy},
    mcp_transport,
    tools::FileSystemTools,
    transport_security::TransportSecurityPolicy,
};
use tower::ServiceExt;

fn protected_router(policy: McpAuthPolicy, file_tools: FileSystemTools) -> Router {
    mcp_transport::router(
        TransportSecurityPolicy::localhost(8000, false),
        file_tools,
    )
    .route_layer(middleware::from_fn_with_state(policy, require_mcp_auth))
}

async fn post_tools_list(policy: McpAuthPolicy, authorization: Option<&str>) -> Response {
    let root = tempfile::tempdir().unwrap();
    let file_tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
    let app = protected_router(policy, file_tools);

    let mut request = Request::post("/mcp")
        .header(header::HOST, "localhost:8000")
        .header(header::ORIGIN, "http://localhost:8000")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(authorization) = authorization {
        request = request.header(header::AUTHORIZATION, authorization);
    }

    app.oneshot(
        request
            .body(Body::from(
                json!({
                    "jsonrpc": "2.0",
                    "id": "auth-test",
                    "method": "tools/list"
                })
                .to_string(),
            ))
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
async fn unauthorized_client_cannot_reach_tool_discovery() {
    let response = post_tools_list(
        McpAuthPolicy::static_bearer("expected-token").unwrap(),
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.headers().get(header::WWW_AUTHENTICATE),
        Some(&header::HeaderValue::from_static("Bearer"))
    );
    let payload = response_json(response).await;
    assert_eq!(payload["error"], "unauthorized");
    assert!(payload.get("result").is_none());
    assert!(!payload.to_string().contains("runtime_status"));
}

#[tokio::test]
async fn correct_bearer_token_reaches_tool_discovery() {
    let response = post_tools_list(
        McpAuthPolicy::static_bearer("expected-token").unwrap(),
        Some("Bearer expected-token"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = response_json(response).await;
    assert_eq!(payload["id"], "auth-test");
    let tools = payload["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|tool| tool["name"] == "runtime_status"));
}

#[tokio::test]
async fn explicit_loopback_development_policy_reaches_discovery_without_header() {
    let response = post_tools_list(McpAuthPolicy::unauthenticated_localhost_only(), None).await;

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn authentication_rejects_before_transport_validation_or_body_dispatch() {
    let root = tempfile::tempdir().unwrap();
    let file_tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
    let app = protected_router(
        McpAuthPolicy::static_bearer("expected-token").unwrap(),
        file_tools,
    );

    let response = app
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

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let payload = response_json(response).await;
    assert_eq!(payload["error"], "unauthorized");
}
