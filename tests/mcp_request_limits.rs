#![cfg(feature = "mcp-runtime")]

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    response::Response,
    Router,
};
use serde_json::{json, Value};
use termux_mcp_server::{
    auth::McpAuthPolicy,
    mcp_transport::{
        self, McpRouterProtection, MCP_POST_ACCEPT, MCP_PROTOCOL_VERSION,
        MCP_PROTOCOL_VERSION_HEADER, MCP_SESSION_ID_HEADER,
    },
    request_limits::McpRequestLimits,
    tools::FileSystemTools,
    transport_security::TransportSecurityPolicy,
};
use tower::ServiceExt;

fn protected_limited_router(max_body_bytes: usize) -> Router {
    let root = tempfile::tempdir().unwrap();
    let file_tools = FileSystemTools::try_new(vec![root.path().to_path_buf()])
        .expect("test safe root must validate");
    let limits = McpRequestLimits::from_seconds(2, 5, max_body_bytes).unwrap();
    let protection = McpRouterProtection::new(
        "127.0.0.1",
        McpAuthPolicy::static_bearer("expected-token").unwrap(),
        limits,
    )
    .unwrap();

    mcp_transport::protected_router(
        protection,
        TransportSecurityPolicy::localhost(8000, false)
            .expect("test localhost policy must be valid"),
        file_tools,
        false,
        false,
        false,
    )
}

fn request(body: impl Into<Body>, authorization: Option<&str>) -> Request<Body> {
    let mut builder = Request::post("/mcp")
        .header(header::HOST, "localhost:8000")
        .header(header::ORIGIN, "http://localhost:8000")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, MCP_POST_ACCEPT);

    if let Some(authorization) = authorization {
        builder = builder.header(header::AUTHORIZATION, authorization);
    }

    builder.body(body.into()).unwrap()
}

fn authenticated_json_request(body: Value, session_id: Option<&str>) -> Request<Body> {
    let mut request = request(body.to_string(), Some("Bearer expected-token"));
    if let Some(session_id) = session_id {
        request.headers_mut().insert(
            MCP_PROTOCOL_VERSION_HEADER,
            header::HeaderValue::from_static(MCP_PROTOCOL_VERSION),
        );
        request.headers_mut().insert(
            MCP_SESSION_ID_HEADER,
            header::HeaderValue::try_from(session_id).unwrap(),
        );
    }
    request
}

async fn response_json(response: Response) -> Value {
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn unauthenticated_oversized_request_is_rejected_before_body_limit() {
    let app = protected_limited_router(1024);
    let response = app.oneshot(request("x".repeat(2048), None)).await.unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let payload = response_json(response).await;
    assert_eq!(payload["error"], "unauthorized");
    assert!(!payload.to_string().contains("mcp_request_body_too_large"));
}

#[tokio::test]
async fn authenticated_oversized_request_is_rejected_with_body_limit() {
    let app = protected_limited_router(1024);
    let response = app
        .oneshot(request("x".repeat(2048), Some("Bearer expected-token")))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&header::HeaderValue::from_static("no-store"))
    );
    let payload = response_json(response).await;
    assert_eq!(payload["error"], "mcp_request_body_too_large");
}

#[tokio::test]
async fn authenticated_request_inside_limits_reaches_tool_discovery() {
    let app = protected_limited_router(8 * 1024);
    let initialize = app
        .clone()
        .oneshot(authenticated_json_request(
            json!({
                "jsonrpc": "2.0",
                "id": "limit-initialize",
                "method": "initialize",
                "params": {
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {"name": "limit-tests", "version": "1.0.0"}
                }
            }),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(initialize.status(), StatusCode::OK);
    let session_id = initialize
        .headers()
        .get(MCP_SESSION_ID_HEADER)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();

    let initialized = app
        .clone()
        .oneshot(authenticated_json_request(
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }),
            Some(&session_id),
        ))
        .await
        .unwrap();
    assert_eq!(initialized.status(), StatusCode::ACCEPTED);

    let response = app
        .oneshot(authenticated_json_request(
            json!({
                "jsonrpc": "2.0",
                "id": "limit-test",
                "method": "tools/list"
            }),
            Some(&session_id),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let payload = response_json(response).await;
    assert_eq!(payload["id"], "limit-test");
    assert!(payload["result"]["tools"].is_array());
}
