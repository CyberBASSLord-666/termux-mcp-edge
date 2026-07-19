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
    copy_file_grant::CopyFileGrantAuthority,
    create_directory_grant::CreateDirectoryGrantAuthority,
    mcp_transport::{
        McpRouterBuildError, McpRouterBuilder, MCP_POST_ACCEPT, MCP_PROTOCOL_VERSION,
        MCP_PROTOCOL_VERSION_HEADER, MCP_SESSION_ID_HEADER,
    },
    request_limits::{
        McpRequestLimits, DEFAULT_MAX_BODY_BYTES, DEFAULT_MAX_CONCURRENT_REQUESTS,
        DEFAULT_REQUEST_TIMEOUT_SECONDS,
    },
    tools::{FileSystemTools, SafeRootConfigurationError},
    transport_security::TransportSecurityPolicy,
    write_file_grant::WriteFileGrantAuthority,
};
use tower::ServiceExt;

fn protected_router(policy: McpAuthPolicy, file_tools: FileSystemTools) -> Router {
    McpRouterBuilder::new(
        "127.0.0.1",
        policy,
        McpRequestLimits::from_seconds(
            DEFAULT_MAX_CONCURRENT_REQUESTS,
            DEFAULT_REQUEST_TIMEOUT_SECONDS,
            DEFAULT_MAX_BODY_BYTES,
        )
        .unwrap(),
        TransportSecurityPolicy::localhost(8000, false)
            .expect("test localhost policy must be valid"),
        file_tools.safe_roots().to_vec(),
    )
    .expect("test MCP router builder configuration must be valid")
    .build()
    .expect("test MCP router must build")
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
    let root = tempfile::tempdir().unwrap();
    for listener_host in ["0.0.0.0", "::", "192.0.2.10", "example.com"] {
        let error = McpRouterBuilder::new(
            listener_host,
            McpAuthPolicy::unauthenticated_localhost_only(),
            McpRequestLimits::from_seconds(
                DEFAULT_MAX_CONCURRENT_REQUESTS,
                DEFAULT_REQUEST_TIMEOUT_SECONDS,
                DEFAULT_MAX_BODY_BYTES,
            )
            .unwrap(),
            TransportSecurityPolicy::localhost(8000, false)
                .expect("test localhost policy must be valid"),
            vec![root.path().to_path_buf()],
        )
        .expect_err("unauthenticated public routers must declare a loopback listener");

        assert_eq!(
            error,
            McpRouterBuildError::UnauthenticatedListenerRequiresLoopback
        );
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


fn builder_error_for_roots(safe_roots: Vec<std::path::PathBuf>) -> McpRouterBuildError {
    match McpRouterBuilder::new(
        "127.0.0.1",
        McpAuthPolicy::static_bearer("builder-error-test-token").unwrap(),
        McpRequestLimits::from_seconds(
            DEFAULT_MAX_CONCURRENT_REQUESTS,
            DEFAULT_REQUEST_TIMEOUT_SECONDS,
            DEFAULT_MAX_BODY_BYTES,
        )
        .unwrap(),
        TransportSecurityPolicy::localhost(8000, false)
            .expect("test localhost policy must be valid"),
        safe_roots,
    ) {
        Ok(_) => panic!("invalid safe-root configuration unexpectedly built a public router"),
        Err(error) => error,
    }
}

#[test]
fn public_builder_returns_typed_errors_for_every_invalid_root_class() {
    let parent = tempfile::tempdir().unwrap();
    let missing = parent.path().join("missing-root");
    let regular_file = parent.path().join("regular-file");
    std::fs::write(&regular_file, b"not a directory").unwrap();

    let cases = [
        (
            Vec::new(),
            SafeRootConfigurationError::EmptyConfiguration,
        ),
        (
            vec![std::path::PathBuf::from("relative/root")],
            SafeRootConfigurationError::RelativePath,
        ),
        (
            vec![std::path::PathBuf::from("/")],
            SafeRootConfigurationError::FilesystemRoot,
        ),
        (vec![missing], SafeRootConfigurationError::Unresolved),
        (vec![regular_file], SafeRootConfigurationError::NotDirectory),
    ];

    for (safe_roots, expected) in cases {
        assert_eq!(
            builder_error_for_roots(safe_roots),
            McpRouterBuildError::SafeRoots(expected)
        );
    }
}

#[cfg(not(feature = "android-battery-status"))]
#[test]
fn public_builder_rejects_requested_uncompiled_battery_client() {
    let root = tempfile::tempdir().unwrap();
    let builder = McpRouterBuilder::new(
        "127.0.0.1",
        McpAuthPolicy::static_bearer("uncompiled-client-test-token").unwrap(),
        McpRequestLimits::from_seconds(
            DEFAULT_MAX_CONCURRENT_REQUESTS,
            DEFAULT_REQUEST_TIMEOUT_SECONDS,
            DEFAULT_MAX_BODY_BYTES,
        )
        .unwrap(),
        TransportSecurityPolicy::localhost(8000, false).unwrap(),
        vec![root.path().to_path_buf()],
    )
    .unwrap()
    .with_android_battery_status_enabled(true);

    let error = match builder.build() {
        Ok(_) => panic!("an uncompiled optional client unexpectedly built"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        McpRouterBuildError::CapabilityNotCompiled {
            capability: "android_battery_status"
        }
    );
}

#[tokio::test]
async fn one_public_builder_authenticates_before_every_transport_surface() {
    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    let root = tempfile::tempdir().unwrap();
    let existing = root.path().join("existing.txt");
    let created = root.path().join("created");
    let copied = root.path().join("copied.txt");
    let written = root.path().join("written.txt");
    std::fs::write(&existing, b"existing-content").unwrap();

    let create_authority = CreateDirectoryGrantAuthority::from_hex_key(
        "test-key-1",
        KEY,
        "builder-auth-test-principal",
    )
    .unwrap();
    let copy_authority = CopyFileGrantAuthority::from_hex_key(
        "test-key-1",
        KEY,
        "builder-auth-test-principal",
    )
    .unwrap();
    let write_authority = WriteFileGrantAuthority::from_hex_key(
        "test-key-1",
        KEY,
        "builder-auth-test-principal",
    )
    .unwrap();

    let app = McpRouterBuilder::new(
        "127.0.0.1",
        McpAuthPolicy::static_bearer("expected-token").unwrap(),
        McpRequestLimits::from_seconds(1, 5, 1_024).unwrap(),
        TransportSecurityPolicy::localhost(8000, false).unwrap(),
        vec![root.path().to_path_buf()],
    )
    .unwrap()
    .with_create_directory_authority(create_authority)
    .with_copy_file_authority(copy_authority)
    .with_write_file_authority(write_authority)
    .build()
    .unwrap();

    let requests = [
        json!({
            "jsonrpc": "2.0",
            "id": "blocked-session",
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "builder-auth-test", "version": "1.0.0"}
            }
        }),
        json!({"jsonrpc": "2.0", "id": "blocked-discovery", "method": "tools/list"}),
        json!({
            "jsonrpc": "2.0",
            "id": "blocked-read",
            "method": "tools/call",
            "params": {
                "name": "read_file",
                "arguments": {"path": existing.to_string_lossy()}
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": "blocked-create",
            "method": "tools/call",
            "params": {
                "name": "create_directory",
                "arguments": {"path": created.to_string_lossy(), "dry_run": false}
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": "blocked-copy",
            "method": "tools/call",
            "params": {
                "name": "copy_file",
                "arguments": {
                    "source_path": existing.to_string_lossy(),
                    "destination_path": copied.to_string_lossy(),
                    "dry_run": false
                }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": "blocked-write",
            "method": "tools/call",
            "params": {
                "name": "write_file",
                "arguments": {
                    "path": written.to_string_lossy(),
                    "content": "blocked-content",
                    "dry_run": false
                }
            }
        }),
    ];

    for body in requests {
        let response = app
            .clone()
            .oneshot(
                Request::post("/mcp")
                    .header(header::HOST, "localhost:8000")
                    .header(header::ORIGIN, "http://localhost:8000")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::ACCEPT, MCP_POST_ACCEPT)
                    .header("mcp-capability-grant", "attacker-controlled-grant")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let payload = response_json(response).await;
        assert_eq!(payload["error"], "unauthorized");
    }

    for request in [
        Request::get("/mcp").body(Body::empty()).unwrap(),
        Request::delete("/mcp").body(Body::empty()).unwrap(),
    ] {
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    assert_eq!(std::fs::read(&existing).unwrap(), b"existing-content");
    assert!(!created.exists());
    assert!(!copied.exists());
    assert!(!written.exists());
}
