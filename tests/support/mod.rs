#![cfg(feature = "mcp-runtime")]
#![allow(dead_code)]

use axum::{
    body::{to_bytes, Body},
    http::{header, Request},
    response::Response,
    Router,
};
use serde_json::{json, Value};
use tempfile::TempDir;
use termux_mcp_server::{
    mcp_transport::{
        router, MCP_POST_ACCEPT, MCP_PROTOCOL_VERSION, MCP_PROTOCOL_VERSION_HEADER,
        MCP_SESSION_ID_HEADER,
    },
    tools::FileSystemTools,
    transport_security::TransportSecurityPolicy,
};
use tower::ServiceExt;

pub(super) fn test_file_tools() -> (TempDir, FileSystemTools) {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("visible.txt"), "safe content").unwrap();
    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
    (root, tools)
}

pub(super) fn empty_test_file_tools() -> (TempDir, FileSystemTools) {
    let root = tempfile::tempdir().unwrap();
    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
    (root, tools)
}

pub(super) fn test_router(file_tools: FileSystemTools) -> Router {
    router(
        TransportSecurityPolicy::localhost(8000, false)
            .expect("test localhost policy must be valid"),
        file_tools,
        false,
        false,
        false,
    )
}

#[cfg(feature = "command-execution")]
pub(super) fn command_test_router(file_tools: FileSystemTools) -> Router {
    router(
        TransportSecurityPolicy::localhost(8000, false)
            .expect("test localhost policy must be valid"),
        file_tools,
        false,
        false,
        true,
    )
}

pub(super) async fn post_raw(body: impl Into<Body>) -> Response {
    let (_root, file_tools) = test_file_tools();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;
    post_raw_to_session(router, &session_id, body).await
}

pub(super) async fn post_json(request_body: Value) -> Response {
    post_raw(request_body.to_string()).await
}

pub(super) async fn post_json_with_empty_root(request_body: Value) -> Response {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;
    post_json_to_session(router, &session_id, request_body).await
}

pub(super) async fn initialize_session(router: &Router) -> String {
    let initialize = json_request(json!({
        "jsonrpc": "2.0",
        "id": "test-initialize",
        "method": "initialize",
        "params": {
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": "termux-mcp-edge-tests",
                "version": "1.0.0"
            }
        }
    }));
    let response = router.clone().oneshot(initialize).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let session_id = response
        .headers()
        .get(MCP_SESSION_ID_HEADER)
        .expect("initialize response missing MCP-Session-Id")
        .to_str()
        .unwrap()
        .to_owned();

    let initialized = session_request(
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
        &session_id,
    );
    let response = router.clone().oneshot(initialized).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::ACCEPTED);

    session_id
}

pub(super) async fn post_json_to_session(
    router: Router,
    session_id: &str,
    request_body: Value,
) -> Response {
    router
        .oneshot(session_request(request_body, session_id))
        .await
        .unwrap()
}

pub(super) async fn post_raw_to_session(
    router: Router,
    session_id: &str,
    body: impl Into<Body>,
) -> Response {
    router
        .oneshot(
            Request::post("/mcp")
                .header(header::HOST, "localhost:8000")
                .header(header::ORIGIN, "http://localhost:8000")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ACCEPT, MCP_POST_ACCEPT)
                .header(MCP_PROTOCOL_VERSION_HEADER, MCP_PROTOCOL_VERSION)
                .header(MCP_SESSION_ID_HEADER, session_id)
                .body(body.into())
                .unwrap(),
        )
        .await
        .unwrap()
}

pub(super) fn json_request(request_body: Value) -> Request<Body> {
    Request::post("/mcp")
        .header(header::HOST, "localhost:8000")
        .header(header::ORIGIN, "http://localhost:8000")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, MCP_POST_ACCEPT)
        .body(Body::from(request_body.to_string()))
        .unwrap()
}

pub(super) fn session_request(request_body: Value, session_id: &str) -> Request<Body> {
    let mut request = json_request(request_body);
    request.headers_mut().insert(
        MCP_PROTOCOL_VERSION_HEADER,
        header::HeaderValue::from_static(MCP_PROTOCOL_VERSION),
    );
    request.headers_mut().insert(
        MCP_SESSION_ID_HEADER,
        header::HeaderValue::try_from(session_id).unwrap(),
    );
    request
}

pub(super) async fn response_json(response: Response) -> Value {
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}
