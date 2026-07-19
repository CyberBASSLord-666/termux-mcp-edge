#![cfg(feature = "mcp-runtime")]

use std::{net::SocketAddr, path::PathBuf};

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
    create_directory_grant::{CreateDirectoryGrantAuthority, CREATE_DIRECTORY_GRANT_HEADER},
    mcp_transport::{
        McpRouterBuildError, McpRouterBuilder, MCP_POST_ACCEPT, MCP_PROTOCOL_VERSION,
        MCP_PROTOCOL_VERSION_HEADER, MCP_SESSION_ID_HEADER,
    },
    request_limits::{
        McpRequestLimits, DEFAULT_MAX_BODY_BYTES, DEFAULT_MAX_CONCURRENT_REQUESTS,
        DEFAULT_REQUEST_TIMEOUT_SECONDS,
    },
    tools::{FileSystemTools, SafeRootConfigurationError, MAX_SAFE_ROOTS},
    transport_security::TransportSecurityPolicy,
    write_file_grant::WriteFileGrantAuthority,
};
use tower::ServiceExt;

#[cfg(feature = "android-volume-control")]
use termux_mcp_server::android_volume_grant::AndroidVolumeGrantAuthority;

const TEST_CAPABILITY_KEY: &str =
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn protected_router(policy: McpAuthPolicy, file_tools: FileSystemTools) -> Router {
    let listener = bound_listener(SocketAddr::from(([127, 0, 0, 1], 0)));
    McpRouterBuilder::try_new(
        &listener,
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
    .unwrap()
    .build()
    .unwrap()
}

fn bound_listener(address: SocketAddr) -> tokio::net::TcpListener {
    let listener = std::net::TcpListener::bind(address).expect("test listener must bind");
    listener
        .set_nonblocking(true)
        .expect("test listener must become nonblocking");
    tokio::net::TcpListener::from_std(listener).expect("test listener requires a Tokio runtime")
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

#[tokio::test]
async fn unauthenticated_policy_rejects_an_actual_non_loopback_listener() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
    let error = McpRouterBuilder::try_new(
        &listener,
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
    .expect_err("unauthenticated public routers must prove a loopback-bound listener");

    assert_eq!(
        error,
        McpRouterBuildError::UnauthenticatedListenerNotLoopback
    );
}

#[tokio::test]
async fn builder_rejects_every_invalid_safe_root_with_typed_redacted_errors() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let parent = tempfile::tempdir().unwrap();
    let missing = parent.path().join("private-missing-root");
    let file = parent.path().join("private-file-root");
    std::fs::write(&file, "not-a-directory").unwrap();
    let symlink = parent.path().join("private-symlink-root");
    std::os::unix::fs::symlink(parent.path(), &symlink).unwrap();

    let invalid = [
        (vec![], SafeRootConfigurationError::EmptyConfiguration),
        (
            vec![parent.path().to_path_buf(); MAX_SAFE_ROOTS + 1],
            SafeRootConfigurationError::TooManyRoots,
        ),
        (vec![PathBuf::new()], SafeRootConfigurationError::EmptyPath),
        (
            vec![PathBuf::from("relative-root")],
            SafeRootConfigurationError::RelativePath,
        ),
        (
            vec![parent.path().join("child/../private-traversing-root")],
            SafeRootConfigurationError::Unresolved,
        ),
        (
            vec![missing.clone()],
            SafeRootConfigurationError::Unresolved,
        ),
        (vec![file.clone()], SafeRootConfigurationError::NotDirectory),
        (
            vec![symlink.clone()],
            SafeRootConfigurationError::SymbolicLink,
        ),
        (
            vec![PathBuf::from("/")],
            SafeRootConfigurationError::FilesystemRoot,
        ),
        (
            vec![parent.path().join(".termux-mcp-write-quarantine")],
            SafeRootConfigurationError::ReservedNamespace,
        ),
    ];

    for (roots, expected) in invalid {
        let error = McpRouterBuilder::try_new(
            &listener,
            McpAuthPolicy::static_bearer("expected-token").unwrap(),
            McpRequestLimits::from_seconds(1, 5, 1_024).unwrap(),
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            roots,
        )
        .expect_err("invalid roots must never produce a builder");
        assert_eq!(error, McpRouterBuildError::SafeRootConfiguration(expected));
        let diagnostic = format!("{error:?} {error}");
        assert!(!diagnostic.contains(missing.to_string_lossy().as_ref()));
        assert!(!diagnostic.contains(file.to_string_lossy().as_ref()));
        assert!(!diagnostic.contains(symlink.to_string_lossy().as_ref()));
        assert!(!diagnostic.contains(parent.path().to_string_lossy().as_ref()));
    }
}

#[tokio::test]
async fn builder_debug_output_redacts_credentials_and_safe_root_paths() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let builder = McpRouterBuilder::try_new(
        &listener,
        McpAuthPolicy::static_bearer("private-builder-token").unwrap(),
        McpRequestLimits::from_seconds(1, 5, 1_024).unwrap(),
        TransportSecurityPolicy::localhost(8000, false).unwrap(),
        vec![root.path().to_path_buf()],
    )
    .unwrap();

    let diagnostic = format!("{builder:?}");
    assert!(diagnostic.contains("<redacted>"));
    assert!(!diagnostic.contains("private-builder-token"));
    assert!(!diagnostic.contains(root.path().to_string_lossy().as_ref()));
}

#[cfg(not(feature = "android-battery-status"))]
#[tokio::test]
async fn builder_returns_a_typed_error_for_an_unavailable_battery_client() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let error = McpRouterBuilder::try_new(
        &listener,
        McpAuthPolicy::static_bearer("expected-token").unwrap(),
        McpRequestLimits::from_seconds(1, 5, 1_024).unwrap(),
        TransportSecurityPolicy::localhost(8000, false).unwrap(),
        vec![root.path().to_path_buf()],
    )
    .unwrap()
    .with_android_battery_status_enabled(true)
    .build()
    .expect_err("an uncompiled optional client must fail closed");

    assert_eq!(
        error,
        McpRouterBuildError::CapabilityUnavailable {
            capability: "android_battery_status"
        }
    );
}

#[cfg(not(feature = "android-volume-status"))]
#[tokio::test]
async fn builder_returns_a_typed_error_for_an_unavailable_volume_client() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let error = McpRouterBuilder::try_new(
        &listener,
        McpAuthPolicy::static_bearer("expected-token").unwrap(),
        McpRequestLimits::from_seconds(1, 5, 1_024).unwrap(),
        TransportSecurityPolicy::localhost(8000, false).unwrap(),
        vec![root.path().to_path_buf()],
    )
    .unwrap()
    .with_android_volume_status_enabled(true)
    .build()
    .expect_err("an uncompiled optional client must fail closed");

    assert_eq!(
        error,
        McpRouterBuildError::CapabilityUnavailable {
            capability: "android_volume_status"
        }
    );
}

#[tokio::test]
async fn builder_rejects_mutation_authorities_for_a_different_or_absent_principal() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let static_builder = || {
        McpRouterBuilder::try_new(
            &listener,
            McpAuthPolicy::static_bearer("expected-token").unwrap(),
            McpRequestLimits::from_seconds(1, 5, 1_024).unwrap(),
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            vec![root.path().to_path_buf()],
        )
        .unwrap()
    };
    let local_builder = || {
        McpRouterBuilder::try_new(
            &listener,
            McpAuthPolicy::unauthenticated_localhost_only(),
            McpRequestLimits::from_seconds(1, 5, 1_024).unwrap(),
            TransportSecurityPolicy::localhost(8000, false).unwrap(),
            vec![root.path().to_path_buf()],
        )
        .unwrap()
    };

    let create = CreateDirectoryGrantAuthority::from_hex_key(
        "test-key-1",
        TEST_CAPABILITY_KEY,
        "different-principal",
    )
    .unwrap();
    for error in [
        static_builder()
            .try_with_create_directory_authority(create.clone())
            .expect_err("create authority and bearer principal must match"),
        local_builder()
            .try_with_create_directory_authority(create)
            .expect_err("unauthenticated mode cannot accept a create authority"),
    ] {
        assert_eq!(
            error,
            McpRouterBuildError::AuthorityPrincipalMismatch {
                capability: "create_directory"
            }
        );
    }

    let copy = CopyFileGrantAuthority::from_hex_key(
        "test-key-1",
        TEST_CAPABILITY_KEY,
        "different-principal",
    )
    .unwrap();
    for error in [
        static_builder()
            .try_with_copy_file_authority(copy.clone())
            .expect_err("copy authority and bearer principal must match"),
        local_builder()
            .try_with_copy_file_authority(copy)
            .expect_err("unauthenticated mode cannot accept a copy authority"),
    ] {
        assert_eq!(
            error,
            McpRouterBuildError::AuthorityPrincipalMismatch {
                capability: "copy_file"
            }
        );
    }

    let write = WriteFileGrantAuthority::from_hex_key(
        "test-key-1",
        TEST_CAPABILITY_KEY,
        "different-principal",
    )
    .unwrap();
    for error in [
        static_builder()
            .try_with_write_file_authority(write.clone())
            .expect_err("write authority and bearer principal must match"),
        local_builder()
            .try_with_write_file_authority(write)
            .expect_err("unauthenticated mode cannot accept a write authority"),
    ] {
        assert_eq!(
            error,
            McpRouterBuildError::AuthorityPrincipalMismatch {
                capability: "write_file"
            }
        );
    }

    #[cfg(feature = "android-volume-control")]
    {
        let volume = AndroidVolumeGrantAuthority::from_hex_key(
            "test-key-1",
            TEST_CAPABILITY_KEY,
            "different-principal",
        )
        .unwrap();
        for error in [
            static_builder()
                .try_with_android_volume_control_authority(volume.clone())
                .expect_err("volume authority and bearer principal must match"),
            local_builder()
                .try_with_android_volume_control_authority(volume)
                .expect_err("unauthenticated mode cannot accept a volume authority"),
        ] {
            assert_eq!(
                error,
                McpRouterBuildError::AuthorityPrincipalMismatch {
                    capability: "android_volume_control"
                }
            );
        }
    }
}

#[tokio::test]
async fn unauthenticated_requests_cannot_reach_sessions_reads_grants_or_mutations() {
    let root = tempfile::tempdir().unwrap();
    let visible = root.path().join("visible.txt");
    let created = root.path().join("created-directory");
    let written = root.path().join("written.txt");
    let copied = root.path().join("copied.txt");
    std::fs::write(&visible, "private-visible-content").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let create_authority = CreateDirectoryGrantAuthority::from_hex_key(
        "test-key-1",
        TEST_CAPABILITY_KEY,
        "expected-token",
    )
    .unwrap();
    let copy_authority =
        CopyFileGrantAuthority::from_hex_key("test-key-1", TEST_CAPABILITY_KEY, "expected-token")
            .unwrap();
    let write_authority =
        WriteFileGrantAuthority::from_hex_key("test-key-1", TEST_CAPABILITY_KEY, "expected-token")
            .unwrap();
    let app = McpRouterBuilder::try_new(
        &listener,
        McpAuthPolicy::static_bearer("expected-token").unwrap(),
        McpRequestLimits::from_seconds(2, 5, 8 * 1_024).unwrap(),
        TransportSecurityPolicy::localhost(8000, false).unwrap(),
        vec![root.path().to_path_buf()],
    )
    .unwrap()
    .try_with_create_directory_authority(create_authority)
    .unwrap()
    .try_with_copy_file_authority(copy_authority)
    .unwrap()
    .try_with_write_file_authority(write_authority)
    .unwrap()
    .build()
    .unwrap();

    let fake_session = "00000000-0000-4000-8000-000000000000";
    let requests = [
        json!({"jsonrpc":"2.0","id":"discovery","method":"tools/list"}),
        json!({
            "jsonrpc":"2.0","id":"read","method":"tools/call",
            "params":{"name":"read_file","arguments":{"path":visible}}
        }),
        json!({
            "jsonrpc":"2.0","id":"create","method":"tools/call",
            "params":{"name":"create_directory","arguments":{"path":created,"dry_run":false}}
        }),
        json!({
            "jsonrpc":"2.0","id":"copy","method":"tools/call",
            "params":{"name":"copy_file","arguments":{"source_path":visible,"destination_path":copied,"dry_run":false}}
        }),
        json!({
            "jsonrpc":"2.0","id":"write","method":"tools/call",
            "params":{"name":"write_file","arguments":{"path":written,"content":"forbidden","dry_run":false}}
        }),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    ];

    for body in requests {
        let mut request = authenticated_request(body, None, Some(fake_session));
        request.headers_mut().insert(
            CREATE_DIRECTORY_GRANT_HEADER,
            header::HeaderValue::from_static("malformed-grant-must-not-be-parsed"),
        );
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let payload = response_json(response).await;
        assert_eq!(payload["error"], "unauthorized");
        assert!(!payload.to_string().contains("private-visible-content"));
        assert!(!payload.to_string().contains("capability_grant"));
    }

    for request in [
        Request::get("/mcp").body(Body::empty()).unwrap(),
        Request::delete("/mcp").body(Body::empty()).unwrap(),
    ] {
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    for attempt in 0..=64 {
        let response = app
            .clone()
            .oneshot(authenticated_request(
                json!({
                    "jsonrpc": "2.0",
                    "id": format!("blocked-capacity-{attempt}"),
                    "method": "initialize",
                    "params": {
                        "protocolVersion": MCP_PROTOCOL_VERSION,
                        "capabilities": {},
                        "clientInfo": {"name": "blocked", "version": "1.0.0"}
                    }
                }),
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    let authenticated_initialize = app
        .clone()
        .oneshot(authenticated_request(
            json!({
                "jsonrpc": "2.0",
                "id": "authenticated-after-blocked-capacity",
                "method": "initialize",
                "params": {
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {"name": "authorized", "version": "1.0.0"}
                }
            }),
            Some("Bearer expected-token"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(authenticated_initialize.status(), StatusCode::OK);
    assert!(authenticated_initialize
        .headers()
        .contains_key(MCP_SESSION_ID_HEADER));

    assert!(!created.exists());
    assert!(!copied.exists());
    assert!(!written.exists());
    assert_eq!(
        std::fs::read_to_string(visible).unwrap(),
        "private-visible-content"
    );
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
