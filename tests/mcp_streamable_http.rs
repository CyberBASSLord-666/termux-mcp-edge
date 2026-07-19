#![cfg(feature = "mcp-runtime")]

mod support;

use axum::{
    body::{to_bytes, Body},
    http::{header, HeaderValue, Method, Request, StatusCode},
    response::Response,
    Router,
};
use serde_json::{json, Value};
use support::{
    empty_test_file_tools, initialize_session, json_request, response_json, session_request,
    test_router, TEST_STATIC_PRINCIPAL,
};
use termux_mcp_server::mcp_transport::{
    MCP_POST_ACCEPT, MCP_PROTOCOL_VERSION, MCP_PROTOCOL_VERSION_HEADER, MCP_SESSION_ID_HEADER,
};
use tower::ServiceExt;
use uuid::Uuid;

fn request_with_body(
    method: Method,
    body: impl Into<Body>,
    session_id: Option<&str>,
    protocol_version: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri("/mcp")
        .header(header::HOST, "localhost:8000")
        .header(header::ORIGIN, "http://localhost:8000")
        .header(
            header::AUTHORIZATION,
            format!("Bearer {TEST_STATIC_PRINCIPAL}"),
        )
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, MCP_POST_ACCEPT);
    if let Some(session_id) = session_id {
        builder = builder.header(MCP_SESSION_ID_HEADER, session_id);
    }
    if let Some(protocol_version) = protocol_version {
        builder = builder.header(MCP_PROTOCOL_VERSION_HEADER, protocol_version);
    }
    builder.body(body.into()).unwrap()
}

fn initialize_body(protocol_version: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": "initialize",
        "method": "initialize",
        "params": {
            "protocolVersion": protocol_version,
            "capabilities": {
                "roots": {"listChanged": true},
                "experimental": {"vendor.example/capability": {}}
            },
            "clientInfo": {
                "name": "streamable-http-tests",
                "title": "Streamable HTTP Tests",
                "version": "1.0.0",
                "description": "Protocol conformance client",
                "icons": [{
                    "src": "https://example.invalid/icon.png",
                    "mimeType": "image/png",
                    "sizes": ["48x48"],
                    "theme": "dark"
                }],
                "websiteUrl": "https://example.invalid"
            },
            "_meta": {"progressToken": "initialize-progress"}
        }
    })
}

async fn initialize_pending(router: &Router, protocol_version: &str) -> (Response, String) {
    let response = router
        .clone()
        .oneshot(json_request(initialize_body(protocol_version)))
        .await
        .unwrap();
    let session_id = response
        .headers()
        .get(MCP_SESSION_ID_HEADER)
        .expect("initialize response missing session")
        .to_str()
        .unwrap()
        .to_owned();
    (response, session_id)
}

async fn post_session(router: &Router, session_id: &str, body: Value) -> Response {
    router
        .clone()
        .oneshot(session_request(body, session_id))
        .await
        .unwrap()
}

async fn assert_empty_body(response: Response) {
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(body.is_empty());
}

#[tokio::test]
async fn initialize_negotiates_stable_version_and_returns_secure_session() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);

    for requested_version in [MCP_PROTOCOL_VERSION, "2024-11-05"] {
        let (response, session_id) = initialize_pending(&router, requested_version).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            Uuid::parse_str(&session_id).unwrap().to_string(),
            session_id
        );
        assert!(session_id.bytes().all(|byte| (0x21..=0x7e).contains(&byte)));
        assert!(response
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("application/json"));

        let payload = response_json(response).await;
        assert_eq!(payload["id"], "initialize");
        assert_eq!(payload["result"]["protocolVersion"], MCP_PROTOCOL_VERSION);
        assert_eq!(payload["result"]["serverInfo"]["name"], "termux-mcp-edge");
        assert_eq!(
            payload["result"]["capabilities"]["tools"]["listChanged"],
            false
        );
    }
}

#[tokio::test]
async fn initialize_schema_is_validated_before_session_creation() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let cases = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
        json!({
            "jsonrpc":"2.0","id":2,"method":"initialize",
            "params":{"protocolVersion":7,"capabilities":{},"clientInfo":{"name":"c","version":"1"}}
        }),
        json!({
            "jsonrpc":"2.0","id":3,"method":"initialize",
            "params":{"protocolVersion":MCP_PROTOCOL_VERSION,"capabilities":[],"clientInfo":{"name":"c","version":"1"}}
        }),
        json!({
            "jsonrpc":"2.0","id":4,"method":"initialize",
            "params":{"protocolVersion":MCP_PROTOCOL_VERSION,"capabilities":{},"clientInfo":{"name":"c"}}
        }),
        json!({
            "jsonrpc":"2.0","id":5,"method":"initialize",
            "params":{"protocolVersion":MCP_PROTOCOL_VERSION,"capabilities":{"roots":{"listChanged":"yes"}},"clientInfo":{"name":"c","version":"1"}}
        }),
        json!({
            "jsonrpc":"2.0","id":6,"method":"initialize",
            "params":{"protocolVersion":MCP_PROTOCOL_VERSION,"capabilities":{},"clientInfo":{"name":"c","version":"1","icons":[{"src":"x","theme":"unknown"}]}}
        }),
    ];

    for body in cases {
        let response = router.clone().oneshot(json_request(body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(response.headers().get(MCP_SESSION_ID_HEADER).is_none());
        let payload = response_json(response).await;
        assert_eq!(payload["error"]["code"], -32602);
    }
}

#[tokio::test]
async fn pending_session_allows_ping_but_gates_tools_until_initialized() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let (_, session_id) = initialize_pending(&router, MCP_PROTOCOL_VERSION).await;

    let blocked = post_session(
        &router,
        &session_id,
        json!({"jsonrpc":"2.0","id":"blocked","method":"tools/list"}),
    )
    .await;
    assert_eq!(blocked.status(), StatusCode::BAD_REQUEST);
    let blocked = response_json(blocked).await;
    assert_eq!(blocked["error"]["code"], -32000);
    assert_eq!(blocked["error"]["message"], "Server not initialized");

    let ping = post_session(
        &router,
        &session_id,
        json!({"jsonrpc":"2.0","id":"pending-ping","method":"ping"}),
    )
    .await;
    assert_eq!(ping.status(), StatusCode::OK);
    assert_eq!(response_json(ping).await["result"], json!({}));

    let initialized = post_session(
        &router,
        &session_id,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    assert_eq!(initialized.status(), StatusCode::ACCEPTED);
    assert_empty_body(initialized).await;

    let tools = post_session(
        &router,
        &session_id,
        json!({"jsonrpc":"2.0","id":"ready","method":"tools/list"}),
    )
    .await;
    assert_eq!(tools.status(), StatusCode::OK);
    assert!(response_json(tools).await["result"]["tools"].is_array());

    let duplicate_initialized = post_session(
        &router,
        &session_id,
        json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}}),
    )
    .await;
    assert_eq!(duplicate_initialized.status(), StatusCode::ACCEPTED);
    assert_empty_body(duplicate_initialized).await;
}

#[tokio::test]
async fn subsequent_requests_require_exact_protocol_and_session_headers() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let body = json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string();

    let missing_session =
        request_with_body(Method::POST, body.clone(), None, Some(MCP_PROTOCOL_VERSION));
    let response = router.clone().oneshot(missing_session).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(response_json(response).await["error"], "session_required");

    let missing_protocol = request_with_body(Method::POST, body.clone(), Some(&session_id), None);
    let response = router.clone().oneshot(missing_protocol).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(response).await["error"],
        "protocol_version_required"
    );

    let wrong_protocol = request_with_body(
        Method::POST,
        body.clone(),
        Some(&session_id),
        Some("2025-03-26"),
    );
    let response = router.clone().oneshot(wrong_protocol).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(response).await["error"],
        "unsupported_protocol_version"
    );

    let unknown_session = request_with_body(
        Method::POST,
        body.clone(),
        Some("00000000-0000-4000-8000-000000000000"),
        Some(MCP_PROTOCOL_VERSION),
    );
    let response = router.clone().oneshot(unknown_session).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(response_json(response).await["error"], "session_not_found");

    let mut duplicate_protocol = request_with_body(
        Method::POST,
        body.clone(),
        Some(&session_id),
        Some(MCP_PROTOCOL_VERSION),
    );
    duplicate_protocol.headers_mut().append(
        MCP_PROTOCOL_VERSION_HEADER,
        HeaderValue::from_static(MCP_PROTOCOL_VERSION),
    );
    let response = router.clone().oneshot(duplicate_protocol).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(response).await["error"],
        "invalid_protocol_version_header"
    );

    let mut duplicate_session = request_with_body(
        Method::POST,
        body,
        Some(&session_id),
        Some(MCP_PROTOCOL_VERSION),
    );
    duplicate_session.headers_mut().append(
        MCP_SESSION_ID_HEADER,
        HeaderValue::from_static("another-session"),
    );
    let response = router.oneshot(duplicate_session).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(response_json(response).await["error"], "session_not_found");
}

#[tokio::test]
async fn post_requires_json_content_type_and_both_explicit_accept_types() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let body = initialize_body(MCP_PROTOCOL_VERSION).to_string();

    for accept in [
        None,
        Some("*/*"),
        Some("application/json"),
        Some("text/event-stream"),
        Some("application/json, text/event-stream;q=0"),
    ] {
        let mut builder = Request::post("/mcp")
            .header(header::HOST, "localhost:8000")
            .header(header::ORIGIN, "http://localhost:8000")
            .header(
                header::AUTHORIZATION,
                format!("Bearer {TEST_STATIC_PRINCIPAL}"),
            )
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(accept) = accept {
            builder = builder.header(header::ACCEPT, accept);
        }
        let response = router
            .clone()
            .oneshot(builder.body(Body::from(body.clone())).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);
    }

    for content_type in [
        None,
        Some("text/plain"),
        Some("application/json-patch+json"),
    ] {
        let mut builder = Request::post("/mcp")
            .header(header::HOST, "localhost:8000")
            .header(header::ORIGIN, "http://localhost:8000")
            .header(
                header::AUTHORIZATION,
                format!("Bearer {TEST_STATIC_PRINCIPAL}"),
            )
            .header(header::ACCEPT, MCP_POST_ACCEPT);
        if let Some(content_type) = content_type {
            builder = builder.header(header::CONTENT_TYPE, content_type);
        }
        let response = router
            .clone()
            .oneshot(builder.body(Body::from(body.clone())).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    let accepted = Request::post("/mcp")
        .header(header::HOST, "localhost:8000")
        .header(header::ORIGIN, "http://localhost:8000")
        .header(
            header::AUTHORIZATION,
            format!("Bearer {TEST_STATIC_PRINCIPAL}"),
        )
        .header(header::CONTENT_TYPE, "Application/JSON; charset=utf-8")
        .header(
            header::ACCEPT,
            "Application/JSON; q=0.5, Text/Event-Stream; q=1",
        )
        .body(Body::from(body))
        .unwrap();
    assert_eq!(
        router.oneshot(accepted).await.unwrap().status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn notifications_and_client_responses_return_202_without_bodies() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    for body in [
        json!({"jsonrpc":"2.0","method":"notifications/unknown","params":{}}),
        json!({"jsonrpc":"2.0","id":"server-request","result":{}}),
        json!({"jsonrpc":"2.0","error":{"code":-32600,"message":"rejected"}}),
    ] {
        let response = post_session(&router, &session_id, body).await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        assert!(response.headers().get(header::CONTENT_TYPE).is_none());
        assert_empty_body(response).await;
    }

    for body in [
        json!([]),
        json!([{"jsonrpc":"2.0","id":1,"method":"ping"}]),
        json!({"jsonrpc":"2.0","id":1,"result":null}),
        json!({"jsonrpc":"2.0","id":1,"result":{},"error":{"code":-1,"message":"ambiguous"}}),
    ] {
        let response = post_session(&router, &session_id, body).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response_json(response).await["error"]["code"], -32600);
    }
}

#[tokio::test]
async fn get_explicitly_declines_sse_and_delete_terminates_the_session() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let get = Request::get("/mcp")
        .header(header::HOST, "localhost:8000")
        .header(header::ORIGIN, "http://localhost:8000")
        .header(
            header::AUTHORIZATION,
            format!("Bearer {TEST_STATIC_PRINCIPAL}"),
        )
        .header(header::ACCEPT, "text/event-stream")
        .header(MCP_PROTOCOL_VERSION_HEADER, MCP_PROTOCOL_VERSION)
        .header(MCP_SESSION_ID_HEADER, &session_id)
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(get).await.unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        response.headers().get(header::ALLOW),
        Some(&HeaderValue::from_static("POST, DELETE"))
    );
    assert!(response.headers().get(header::CONTENT_TYPE).is_none());

    let missing_accept = Request::get("/mcp")
        .header(header::HOST, "localhost:8000")
        .header(header::ORIGIN, "http://localhost:8000")
        .header(
            header::AUTHORIZATION,
            format!("Bearer {TEST_STATIC_PRINCIPAL}"),
        )
        .header(MCP_PROTOCOL_VERSION_HEADER, MCP_PROTOCOL_VERSION)
        .header(MCP_SESSION_ID_HEADER, &session_id)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        router
            .clone()
            .oneshot(missing_accept)
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_ACCEPTABLE
    );

    let delete = request_with_body(
        Method::DELETE,
        Body::empty(),
        Some(&session_id),
        Some(MCP_PROTOCOL_VERSION),
    );
    let response = router.clone().oneshot(delete).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_empty_body(response).await;

    let after_delete = request_with_body(
        Method::POST,
        json!({"jsonrpc":"2.0","id":1,"method":"ping"}).to_string(),
        Some(&session_id),
        Some(MCP_PROTOCOL_VERSION),
    );
    let response = router.oneshot(after_delete).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(response_json(response).await["error"], "session_not_found");
}

#[tokio::test]
async fn lifecycle_state_is_isolated_between_sessions() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let active = initialize_session(&router).await;
    let (_, pending) = initialize_pending(&router, MCP_PROTOCOL_VERSION).await;

    let active_response = post_session(
        &router,
        &active,
        json!({"jsonrpc":"2.0","id":"active","method":"tools/list"}),
    )
    .await;
    assert_eq!(active_response.status(), StatusCode::OK);

    let pending_response = post_session(
        &router,
        &pending,
        json!({"jsonrpc":"2.0","id":"pending","method":"tools/list"}),
    )
    .await;
    assert_eq!(pending_response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(pending_response).await["error"]["message"],
        "Server not initialized"
    );
}

#[tokio::test]
async fn transport_security_precedes_media_session_and_method_handling() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);

    for method in [Method::POST, Method::GET, Method::DELETE, Method::PUT] {
        let request = Request::builder()
            .method(method)
            .uri("/mcp")
            .header(header::HOST, "localhost:8000")
            .header(header::ORIGIN, "https://attacker.invalid")
            .header(
                header::AUTHORIZATION,
                format!("Bearer {TEST_STATIC_PRINCIPAL}"),
            )
            .body(Body::from("not-json"))
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response_json(response).await["error"],
            "transport_security_rejected"
        );
    }
}

#[tokio::test]
async fn unsupported_methods_are_bounded_and_advertise_the_endpoint_methods() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let request = Request::put("/mcp")
        .header(header::HOST, "localhost:8000")
        .header(header::ORIGIN, "http://localhost:8000")
        .header(
            header::AUTHORIZATION,
            format!("Bearer {TEST_STATIC_PRINCIPAL}"),
        )
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        response.headers().get(header::ALLOW),
        Some(&HeaderValue::from_static("POST, GET, DELETE"))
    );
    assert_empty_body(response).await;
}
