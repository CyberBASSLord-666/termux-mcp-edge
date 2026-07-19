#![cfg(feature = "mcp-runtime")]

use std::net::SocketAddr;

use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
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
    request_limits::{
        McpRequestLimits, DEFAULT_MAX_BODY_BYTES, DEFAULT_MAX_CONCURRENT_REQUESTS,
        DEFAULT_REQUEST_TIMEOUT_SECONDS,
    },
    tools::FileSystemTools,
    transport_security::TransportSecurityPolicy,
};
use tower::ServiceExt;

fn protected_router(policy: McpAuthPolicy, file_tools: FileSystemTools) -> Router {
    let protection = McpRouterProtection::new(
        "127.0.0.1",
        policy,
        McpRequestLimits::from_seconds(
            DEFAULT_MAX_CONCURRENT_REQUESTS,
            DEFAULT_REQUEST_TIMEOUT_SECONDS,
            DEFAULT_MAX_BODY_BYTES,
        )
        .unwrap(),
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

async fn post_tools_list(policy: McpAuthPolicy, authorization: Option<&str>) -> Response {
    let root = tempfile::tempdir().unwrap();
    let file_tools = FileSystemTools::try_new(vec![root.path().to_path_buf()])
        .expect("test safe root must validate");
    let app = protected_router(policy, file_tools);

    let initialize = authenticated_request(
        json!({
            "jsonrpc": "2.0",
            "id": "auth-initialize",
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "auth-tests", "version": "1.0.0"}
            }
        }),
        authorization,
        None,
    );
    let initialize_response = app.clone().oneshot(initialize).await.unwrap();
    if initialize_response.status() != StatusCode::OK {
        return initialize_response;
    }
    let session_id = initialize_response
        .headers()
        .get(MCP_SESSION_ID_HEADER)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();

    let initialized = authenticated_request(
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
        authorization,
        Some(&session_id),
    );
    let initialized_response = app.clone().oneshot(initialized).await.unwrap();
    assert_eq!(initialized_response.status(), StatusCode::ACCEPTED);

    app.oneshot(authenticated_request(
        json!({
            "jsonrpc": "2.0",
            "id": "auth-test",
            "method": "tools/list"
        }),
        authorization,
        Some(&session_id),
    ))
    .await
    .unwrap()
}

fn authenticated_request(
    body: Value,
    authorization: Option<&str>,
    session_id: Option<&str>,
) -> Request<Body> {
    let mut request = Request::post("/mcp")
        .header(header::HOST, "localhost:8000")
        .header(header::ORIGIN, "http://localhost:8000")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, MCP_POST_ACCEPT);
    if let Some(authorization) = authorization {
        request = request.header(header::AUTHORIZATION, authorization);
    }
    if let Some(session_id) = session_id {
        request = request
            .header(MCP_PROTOCOL_VERSION_HEADER, MCP_PROTOCOL_VERSION)
            .header(MCP_SESSION_ID_HEADER, session_id);
    }
    let mut request = request.body(Body::from(body.to_string())).unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))));
    request
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
async fn public_local_development_router_fails_closed_without_connect_info() {
    let root = tempfile::tempdir().unwrap();
    let file_tools = FileSystemTools::try_new(vec![root.path().to_path_buf()])
        .expect("test safe root must validate");
    let app = protected_router(McpAuthPolicy::unauthenticated_localhost_only(), file_tools);
    let mut request = authenticated_request(
        json!({
            "jsonrpc": "2.0",
            "id": "missing-connect-info",
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "auth-tests", "version": "1.0.0"}
            }
        }),
        None,
        None,
    );
    request.extensions_mut().remove::<ConnectInfo<SocketAddr>>();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let payload = response_json(response).await;
    assert_eq!(payload["error"], "localhost_peer_required");
    assert!(payload.get("result").is_none());
}

#[tokio::test]
async fn public_local_development_router_rejects_non_loopback_connect_info() {
    let root = tempfile::tempdir().unwrap();
    let file_tools = FileSystemTools::try_new(vec![root.path().to_path_buf()])
        .expect("test safe root must validate");
    let app = protected_router(McpAuthPolicy::unauthenticated_localhost_only(), file_tools);
    let mut request = authenticated_request(
        json!({
            "jsonrpc": "2.0",
            "id": "remote-connect-info",
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "auth-tests", "version": "1.0.0"}
            }
        }),
        None,
        None,
    );
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([192, 0, 2, 10], 40_001))));

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let payload = response_json(response).await;
    assert_eq!(payload["error"], "localhost_peer_required");
    assert!(!payload.to_string().contains("192.0.2.10"));
}

#[test]
fn unauthenticated_policy_rejects_non_loopback_listener_declarations() {
    for listener_host in ["0.0.0.0", "::", "192.0.2.10", "example.com"] {
        let error = McpRouterProtection::new(
            listener_host,
            McpAuthPolicy::unauthenticated_localhost_only(),
            McpRequestLimits::from_seconds(
                DEFAULT_MAX_CONCURRENT_REQUESTS,
                DEFAULT_REQUEST_TIMEOUT_SECONDS,
                DEFAULT_MAX_BODY_BYTES,
            )
            .unwrap(),
        )
        .expect_err("unauthenticated public routers must declare a loopback listener");

        assert!(error.to_string().contains("loopback listener host"));
    }
}

#[tokio::test]
async fn authentication_rejects_before_transport_validation_or_body_dispatch() {
    let root = tempfile::tempdir().unwrap();
    let file_tools = FileSystemTools::try_new(vec![root.path().to_path_buf()])
        .expect("test safe root must validate");
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
