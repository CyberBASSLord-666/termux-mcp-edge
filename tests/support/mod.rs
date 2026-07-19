#![cfg(feature = "mcp-runtime")]
#![allow(dead_code)]

use std::net::SocketAddr;

use axum::{
    body::{to_bytes, Body},
    extract::{ConnectInfo, Request as AxumRequest},
    http::{header, Request},
    middleware::{self, Next},
    response::Response,
    Router,
};
use serde_json::{json, Value};
use tempfile::TempDir;
use termux_mcp_server::{
    auth::McpAuthPolicy,
    create_directory_grant::{CreateDirectoryGrantAuthority, CREATE_DIRECTORY_GRANT_HEADER},
    mcp_transport::{
        protected_router, protected_router_with_create_directory_authority,
        protected_router_with_filesystem_authorities, protected_router_with_options,
        McpRouterProtection, McpTransportOptions, MCP_POST_ACCEPT, MCP_PROTOCOL_VERSION,
        MCP_PROTOCOL_VERSION_HEADER, MCP_SESSION_ID_HEADER,
    },
    request_limits::{
        McpRequestLimits, DEFAULT_MAX_BODY_BYTES, DEFAULT_MAX_CONCURRENT_REQUESTS,
        DEFAULT_REQUEST_TIMEOUT_SECONDS,
    },
    tools::FileSystemTools,
    transport_security::TransportSecurityPolicy,
    write_file_grant::{WriteFileDisposition, WriteFileGrantAuthority},
};
use tower::ServiceExt;

pub(super) const TEST_CAPABILITY_KEY: &str =
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
pub(super) const TEST_STATIC_PRINCIPAL: &str = "test-static-principal";

fn test_router_protection() -> McpRouterProtection {
    McpRouterProtection::new(
        "127.0.0.1",
        McpAuthPolicy::unauthenticated_localhost_only(),
        McpRequestLimits::from_seconds(
            DEFAULT_MAX_CONCURRENT_REQUESTS,
            DEFAULT_REQUEST_TIMEOUT_SECONDS,
            DEFAULT_MAX_BODY_BYTES,
        )
        .expect("default test request limits must be valid"),
    )
    .expect("unauthenticated test routers declare an exact loopback listener")
}

async fn attach_loopback_test_peer(mut request: AxumRequest, next: Next) -> Response {
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))));
    next.run(request).await
}

fn with_loopback_test_peer(router: Router) -> Router {
    router.route_layer(middleware::from_fn(attach_loopback_test_peer))
}

pub(super) fn test_file_tools() -> (TempDir, FileSystemTools) {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("visible.txt"), "safe content").unwrap();
    let tools = FileSystemTools::try_new(vec![root.path().to_path_buf()])
        .expect("test safe root must validate");
    (root, tools)
}

pub(super) fn empty_test_file_tools() -> (TempDir, FileSystemTools) {
    let root = tempfile::tempdir().unwrap();
    let tools = FileSystemTools::try_new(vec![root.path().to_path_buf()])
        .expect("test safe root must validate");
    (root, tools)
}

pub(super) fn test_router(file_tools: FileSystemTools) -> Router {
    with_loopback_test_peer(protected_router(
        test_router_protection(),
        TransportSecurityPolicy::localhost(8000, false)
            .expect("test localhost policy must be valid"),
        file_tools,
        false,
        false,
        false,
    ))
}

pub(super) fn sse_test_router(file_tools: FileSystemTools) -> Router {
    with_loopback_test_peer(protected_router_with_options(
        test_router_protection(),
        TransportSecurityPolicy::localhost(8000, false)
            .expect("test localhost policy must be valid"),
        file_tools,
        false,
        false,
        false,
        McpTransportOptions::default().with_sse_enabled(true),
    ))
}

pub(super) fn create_directory_authorized_test_router(
    file_tools: FileSystemTools,
) -> (Router, CreateDirectoryGrantAuthority) {
    let authority = CreateDirectoryGrantAuthority::from_hex_key(
        "test-key-1",
        TEST_CAPABILITY_KEY,
        TEST_STATIC_PRINCIPAL,
    )
    .unwrap();
    let router = with_loopback_test_peer(protected_router_with_create_directory_authority(
        test_router_protection(),
        TransportSecurityPolicy::localhost(8000, false)
            .expect("test localhost policy must be valid"),
        file_tools,
        false,
        false,
        false,
        authority.clone(),
    ));
    (router, authority)
}

pub(super) fn write_file_authorized_test_router(
    file_tools: FileSystemTools,
) -> (Router, WriteFileGrantAuthority) {
    let authority = WriteFileGrantAuthority::from_hex_key(
        "test-key-1",
        TEST_CAPABILITY_KEY,
        TEST_STATIC_PRINCIPAL,
    )
    .unwrap();
    let protection = McpRouterProtection::new(
        "127.0.0.1",
        McpAuthPolicy::unauthenticated_localhost_only(),
        McpRequestLimits::from_seconds(16, DEFAULT_REQUEST_TIMEOUT_SECONDS, DEFAULT_MAX_BODY_BYTES)
            .expect("write authorization tests require bounded parallel replay attempts"),
    )
    .expect("unauthenticated test routers declare an exact loopback listener");
    let router = with_loopback_test_peer(protected_router_with_filesystem_authorities(
        protection,
        TransportSecurityPolicy::localhost(8000, false)
            .expect("test localhost policy must be valid"),
        file_tools,
        false,
        false,
        false,
        None,
        Some(authority.clone()),
    ));
    (router, authority)
}

pub(super) fn issue_create_directory_grant(
    authority: &CreateDirectoryGrantAuthority,
    file_tools: &FileSystemTools,
    session_id: &str,
    target_path: &str,
) -> String {
    let target = file_tools
        .create_directory_grant_target(target_path)
        .expect("test grant target must be valid");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    authority.issue_at(session_id, &target, now).unwrap()
}

pub(super) fn issue_write_file_grant(
    authority: &WriteFileGrantAuthority,
    file_tools: &FileSystemTools,
    session_id: &str,
    target_path: &str,
    content: &[u8],
    disposition: WriteFileDisposition,
) -> String {
    let target = file_tools
        .write_file_grant_target(target_path, content, disposition)
        .expect("test grant target must be valid");
    authority.issue(session_id, &target).unwrap()
}

#[cfg(feature = "command-execution")]
pub(super) fn command_test_router(file_tools: FileSystemTools) -> Router {
    with_loopback_test_peer(protected_router(
        test_router_protection(),
        TransportSecurityPolicy::localhost(8000, false)
            .expect("test localhost policy must be valid"),
        file_tools,
        false,
        false,
        true,
    ))
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

pub(super) async fn post_json_to_session_with_grant(
    router: Router,
    session_id: &str,
    request_body: Value,
    grant: &str,
) -> Response {
    let mut request = session_request(request_body, session_id);
    request.headers_mut().insert(
        CREATE_DIRECTORY_GRANT_HEADER,
        header::HeaderValue::try_from(grant).unwrap(),
    );
    router.oneshot(request).await.unwrap()
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
