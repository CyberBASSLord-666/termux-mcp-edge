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
    empty_test_file_tools, initialize_session, post_json_to_session, response_json,
    stateless_sse_test_router, stateless_test_router, test_file_tools, test_router,
    TEST_STATIC_PRINCIPAL,
};
use termux_mcp_server::{
    create_directory_grant::CREATE_DIRECTORY_GRANT_HEADER,
    mcp_transport::{
        MCP_LAST_EVENT_ID_HEADER, MCP_METHOD_HEADER, MCP_NAME_HEADER, MCP_POST_ACCEPT,
        MCP_PROTOCOL_VERSION, MCP_PROTOCOL_VERSION_HEADER, MCP_SESSION_ID_HEADER,
        MCP_STATELESS_PROTOCOL_VERSION,
    },
};
use tower::ServiceExt;

const HEADER_MISMATCH: i64 = -32020;
const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

fn request_meta(version: &str) -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": version,
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {
            "name": "termux-mcp-edge-stateless-tests",
            "version": "1.0.0"
        }
    })
}

fn modern_body(id: Value, method: &str, fields: Value) -> Value {
    let mut params = fields
        .as_object()
        .cloned()
        .expect("test request fields must be an object");
    params.insert(
        "_meta".to_owned(),
        request_meta(MCP_STATELESS_PROTOCOL_VERSION),
    );
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    })
}

fn modern_request(body: Value, method: &str, name: Option<&str>) -> Request<Body> {
    request_for_version(body, MCP_STATELESS_PROTOCOL_VERSION, method, name)
}

fn request_for_version(
    body: Value,
    version: &str,
    method: &str,
    name: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::post("/mcp")
        .header(header::HOST, "localhost:8000")
        .header(header::ORIGIN, "http://localhost:8000")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, MCP_POST_ACCEPT)
        .header(
            header::AUTHORIZATION,
            format!("Bearer {TEST_STATIC_PRINCIPAL}"),
        )
        .header(MCP_PROTOCOL_VERSION_HEADER, version)
        .header(MCP_METHOD_HEADER, method);
    if let Some(name) = name {
        builder = builder.header(MCP_NAME_HEADER, name);
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

fn modern_method_request(method: Method) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri("/mcp")
        .header(header::HOST, "localhost:8000")
        .header(header::ORIGIN, "http://localhost:8000")
        .header(header::ACCEPT, MCP_POST_ACCEPT)
        .header(
            header::AUTHORIZATION,
            format!("Bearer {TEST_STATIC_PRINCIPAL}"),
        )
        .header(MCP_PROTOCOL_VERSION_HEADER, MCP_STATELESS_PROTOCOL_VERSION)
        .body(Body::empty())
        .unwrap()
}

async fn send(router: &Router, request: Request<Body>) -> Response {
    router.clone().oneshot(request).await.unwrap()
}

async fn assert_rpc_error(response: Response, status: StatusCode, id: Value, code: i64) -> Value {
    assert_eq!(response.status(), status);
    assert!(response.headers().get(MCP_SESSION_ID_HEADER).is_none());
    let payload = response_json(response).await;
    assert_eq!(payload["jsonrpc"], "2.0");
    assert_eq!(payload["id"], id);
    assert_eq!(payload["error"]["code"], code);
    payload
}

fn assert_modern_result_envelope(payload: &Value, id: &str) {
    assert_eq!(payload["jsonrpc"], "2.0");
    assert_eq!(payload["id"], id);
    assert_eq!(payload["result"]["resultType"], "complete");
    assert_eq!(
        payload["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "termux-mcp-edge"
    );
    assert!(
        payload["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["version"]
            .as_str()
            .is_some_and(|version| !version.is_empty())
    );
}

fn assert_json_not_session_response(response: &Response) {
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get(MCP_SESSION_ID_HEADER).is_none());
    assert!(response.headers().get(MCP_LAST_EVENT_ID_HEADER).is_none());
    assert!(response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/json")));
}

#[tokio::test]
async fn stateless_protocol_is_default_off_and_explicitly_opted_in() {
    let body = modern_body(json!("discover-default-off"), "server/discover", json!({}));
    let (_root, file_tools) = empty_test_file_tools();
    let response = send(
        &test_router(file_tools),
        modern_request(body, "server/discover", None),
    )
    .await;
    let payload = assert_rpc_error(
        response,
        StatusCode::BAD_REQUEST,
        json!("discover-default-off"),
        UNSUPPORTED_PROTOCOL_VERSION,
    )
    .await;
    assert_eq!(
        payload["error"]["data"]["requested"],
        MCP_STATELESS_PROTOCOL_VERSION
    );
    assert_eq!(
        payload["error"]["data"]["supported"],
        json!([MCP_PROTOCOL_VERSION])
    );

    let body = modern_body(json!("discover-opted-in"), "server/discover", json!({}));
    let (_root, file_tools) = empty_test_file_tools();
    let response = send(
        &stateless_test_router(file_tools),
        modern_request(body, "server/discover", None),
    )
    .await;
    assert_json_not_session_response(&response);
}

#[tokio::test]
async fn discovery_has_the_complete_stateless_capability_and_cache_contract() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = stateless_test_router(file_tools);
    let response = send(
        &router,
        modern_request(
            modern_body(json!("discover"), "server/discover", json!({})),
            "server/discover",
            None,
        ),
    )
    .await;
    assert_json_not_session_response(&response);
    let payload = response_json(response).await;
    assert_modern_result_envelope(&payload, "discover");
    assert_eq!(
        payload["result"]["supportedVersions"],
        json!([MCP_STATELESS_PROTOCOL_VERSION, MCP_PROTOCOL_VERSION])
    );
    assert!(payload["result"]["capabilities"]["tools"].is_object());
    assert_eq!(
        payload["result"]["capabilities"]["tools"]["listChanged"],
        false
    );
    assert_eq!(payload["result"]["ttlMs"], 0);
    assert_eq!(payload["result"]["cacheScope"], "private");
}

#[tokio::test]
async fn stateless_post_requires_both_supported_response_media_types() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = stateless_test_router(file_tools);
    let mut request = modern_request(
        modern_body(json!("accept"), "server/discover", json!({})),
        "server/discover",
        None,
    );
    request
        .headers_mut()
        .insert(header::ACCEPT, HeaderValue::from_static("application/json"));

    let response = send(&router, request).await;
    assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);
    assert!(response.headers().get(MCP_SESSION_ID_HEADER).is_none());

    let mut request = modern_request(
        modern_body(json!("accept-q-zero"), "server/discover", json!({})),
        "server/discover",
        None,
    );
    request.headers_mut().insert(
        header::ACCEPT,
        HeaderValue::from_static("application/json, text/event-stream;q=0"),
    );
    assert_eq!(
        send(&router, request).await.status(),
        StatusCode::NOT_ACCEPTABLE
    );
}

#[tokio::test]
async fn stateless_list_read_and_preview_need_no_lifecycle_or_session() {
    let (root, file_tools) = test_file_tools();
    let router = stateless_test_router(file_tools);

    let listed = send(
        &router,
        modern_request(
            modern_body(json!("list"), "tools/list", json!({})),
            "tools/list",
            None,
        ),
    )
    .await;
    assert_json_not_session_response(&listed);
    let listed = response_json(listed).await;
    assert_modern_result_envelope(&listed, "list");
    assert_eq!(listed["result"]["ttlMs"], 0);
    assert_eq!(listed["result"]["cacheScope"], "private");
    let tools = listed["result"]["tools"]
        .as_array()
        .expect("stateless tools/list must return tools");
    assert!(tools.iter().any(|tool| tool["name"] == "read_file"));
    assert!(tools.iter().any(|tool| tool["name"] == "create_directory"));
    for mutation_name in ["create_directory", "copy_file", "trash_file", "write_file"] {
        let tool = tools
            .iter()
            .find(|tool| tool["name"] == mutation_name)
            .unwrap_or_else(|| panic!("missing stateless preview tool: {mutation_name}"));
        assert_eq!(
            tool["inputSchema"]["properties"]["dry_run"]["const"], true,
            "stateless mutation schema must be preview-only: {mutation_name}"
        );
    }

    let visible = root.path().join("visible.txt");
    let read = send(
        &router,
        modern_request(
            modern_body(
                json!("read"),
                "tools/call",
                json!({
                    "name": "read_file",
                    "arguments": {"path": visible.to_string_lossy()}
                }),
            ),
            "tools/call",
            Some("read_file"),
        ),
    )
    .await;
    assert_json_not_session_response(&read);
    let read = response_json(read).await;
    assert_modern_result_envelope(&read, "read");
    assert_eq!(read["result"]["isError"], false);
    assert!(read["result"].to_string().contains("safe content"));

    let preview_target = root.path().join("stateless-preview");
    let preview = send(
        &router,
        modern_request(
            modern_body(
                json!("preview"),
                "tools/call",
                json!({
                    "name": "create_directory",
                    "arguments": {
                        "path": preview_target.to_string_lossy(),
                        "dry_run": true
                    }
                }),
            ),
            "tools/call",
            Some("create_directory"),
        ),
    )
    .await;
    assert_json_not_session_response(&preview);
    let preview = response_json(preview).await;
    assert_modern_result_envelope(&preview, "preview");
    assert_eq!(preview["result"]["isError"], false);
    assert_eq!(preview["result"]["structuredContent"]["dryRun"], true);
    assert!(!preview_target.exists());
}

#[tokio::test]
async fn every_stateless_request_requires_exact_protocol_metadata() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = stateless_test_router(file_tools);
    let cases = [
        (
            "missing-params",
            json!({"jsonrpc":"2.0","id":"missing-params","method":"server/discover"}),
        ),
        (
            "missing-meta",
            json!({
                "jsonrpc":"2.0",
                "id":"missing-meta",
                "method":"server/discover",
                "params":{}
            }),
        ),
        (
            "missing-version",
            json!({
                "jsonrpc":"2.0",
                "id":"missing-version",
                "method":"server/discover",
                "params":{"_meta":{"io.modelcontextprotocol/clientCapabilities":{}}}
            }),
        ),
        (
            "missing-capabilities",
            json!({
                "jsonrpc":"2.0",
                "id":"missing-capabilities",
                "method":"server/discover",
                "params":{"_meta":{
                    "io.modelcontextprotocol/protocolVersion":MCP_STATELESS_PROTOCOL_VERSION
                }}
            }),
        ),
        (
            "invalid-capabilities",
            json!({
                "jsonrpc":"2.0",
                "id":"invalid-capabilities",
                "method":"server/discover",
                "params":{"_meta":{
                    "io.modelcontextprotocol/protocolVersion":MCP_STATELESS_PROTOCOL_VERSION,
                    "io.modelcontextprotocol/clientCapabilities":{"roots":true}
                }}
            }),
        ),
        (
            "invalid-progress-token",
            json!({
                "jsonrpc":"2.0",
                "id":"invalid-progress-token",
                "method":"server/discover",
                "params":{"_meta":{
                    "io.modelcontextprotocol/protocolVersion":MCP_STATELESS_PROTOCOL_VERSION,
                    "io.modelcontextprotocol/clientCapabilities":{},
                    "progressToken":true
                }}
            }),
        ),
        (
            "invalid-log-level",
            json!({
                "jsonrpc":"2.0",
                "id":"invalid-log-level",
                "method":"server/discover",
                "params":{"_meta":{
                    "io.modelcontextprotocol/protocolVersion":MCP_STATELESS_PROTOCOL_VERSION,
                    "io.modelcontextprotocol/clientCapabilities":{},
                    "io.modelcontextprotocol/logLevel":"verbose"
                }}
            }),
        ),
    ];

    for (id, body) in cases {
        let response = send(&router, modern_request(body, "server/discover", None)).await;
        assert_rpc_error(response, StatusCode::BAD_REQUEST, json!(id), -32602).await;
    }

    let fractional_progress = json!({
        "jsonrpc":"2.0",
        "id":"fractional-progress",
        "method":"server/discover",
        "params":{"_meta":{
            "io.modelcontextprotocol/protocolVersion":MCP_STATELESS_PROTOCOL_VERSION,
            "io.modelcontextprotocol/clientCapabilities":{},
            "progressToken":1.5
        }}
    });
    let response = send(
        &router,
        modern_request(fractional_progress, "server/discover", None),
    )
    .await;
    assert_json_not_session_response(&response);
    assert_modern_result_envelope(&response_json(response).await, "fractional-progress");
}

#[tokio::test]
async fn routing_headers_are_single_exact_mirrors_of_the_json_body() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = stateless_test_router(file_tools);

    let body = modern_body(json!("method-routing"), "tools/list", json!({}));
    let mut missing_method = modern_request(body.clone(), "tools/list", None);
    missing_method.headers_mut().remove(MCP_METHOD_HEADER);
    assert_rpc_error(
        send(&router, missing_method).await,
        StatusCode::BAD_REQUEST,
        json!("method-routing"),
        HEADER_MISMATCH,
    )
    .await;

    let mut missing_all_modern_headers = modern_request(body.clone(), "server/discover", None);
    missing_all_modern_headers
        .headers_mut()
        .remove(MCP_PROTOCOL_VERSION_HEADER);
    missing_all_modern_headers
        .headers_mut()
        .remove(MCP_METHOD_HEADER);
    assert_rpc_error(
        send(&router, missing_all_modern_headers).await,
        StatusCode::BAD_REQUEST,
        json!("method-routing"),
        HEADER_MISMATCH,
    )
    .await;

    let mismatched_method = modern_request(body.clone(), "server/discover", None);
    assert_rpc_error(
        send(&router, mismatched_method).await,
        StatusCode::BAD_REQUEST,
        json!("method-routing"),
        HEADER_MISMATCH,
    )
    .await;

    let mut duplicate_method = modern_request(body, "tools/list", None);
    duplicate_method
        .headers_mut()
        .append(MCP_METHOD_HEADER, HeaderValue::from_static("tools/list"));
    assert_rpc_error(
        send(&router, duplicate_method).await,
        StatusCode::BAD_REQUEST,
        json!("method-routing"),
        HEADER_MISMATCH,
    )
    .await;

    let call = modern_body(
        json!("name-routing"),
        "tools/call",
        json!({"name":"runtime_status","arguments":{}}),
    );
    let missing_name = modern_request(call.clone(), "tools/call", None);
    assert_rpc_error(
        send(&router, missing_name).await,
        StatusCode::BAD_REQUEST,
        json!("name-routing"),
        HEADER_MISMATCH,
    )
    .await;

    let mismatched_name = modern_request(call.clone(), "tools/call", Some("read_file"));
    assert_rpc_error(
        send(&router, mismatched_name).await,
        StatusCode::BAD_REQUEST,
        json!("name-routing"),
        HEADER_MISMATCH,
    )
    .await;

    let mut duplicate_name = modern_request(call.clone(), "tools/call", Some("runtime_status"));
    duplicate_name
        .headers_mut()
        .append(MCP_NAME_HEADER, HeaderValue::from_static("runtime_status"));
    assert_rpc_error(
        send(&router, duplicate_name).await,
        StatusCode::BAD_REQUEST,
        json!("name-routing"),
        HEADER_MISMATCH,
    )
    .await;

    let mut duplicate_version = modern_request(call, "tools/call", Some("runtime_status"));
    duplicate_version.headers_mut().append(
        MCP_PROTOCOL_VERSION_HEADER,
        HeaderValue::from_static(MCP_STATELESS_PROTOCOL_VERSION),
    );
    assert_rpc_error(
        send(&router, duplicate_version).await,
        StatusCode::BAD_REQUEST,
        json!("name-routing"),
        HEADER_MISMATCH,
    )
    .await;
}

#[tokio::test]
async fn body_header_version_mismatch_and_unsupported_versions_are_distinct() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = stateless_test_router(file_tools);

    let mismatched = modern_body(json!("version-mismatch"), "server/discover", json!({}));
    let response = send(
        &router,
        request_for_version(mismatched, MCP_PROTOCOL_VERSION, "server/discover", None),
    )
    .await;
    assert_rpc_error(
        response,
        StatusCode::BAD_REQUEST,
        json!("version-mismatch"),
        HEADER_MISMATCH,
    )
    .await;

    let unsupported = "2099-01-01";
    let body = json!({
        "jsonrpc":"2.0",
        "id":"unsupported",
        "method":"server/discover",
        "params":{"_meta":request_meta(unsupported)}
    });
    let response = send(
        &router,
        request_for_version(body, unsupported, "server/discover", None),
    )
    .await;
    let payload = assert_rpc_error(
        response,
        StatusCode::BAD_REQUEST,
        json!("unsupported"),
        UNSUPPORTED_PROTOCOL_VERSION,
    )
    .await;
    assert_eq!(payload["error"]["data"]["requested"], unsupported);
    assert_eq!(
        payload["error"]["data"]["supported"],
        json!([MCP_STATELESS_PROTOCOL_VERSION, MCP_PROTOCOL_VERSION])
    );

    let malformed_unknown = Request::post("/mcp")
        .header(header::HOST, "localhost:8000")
        .header(header::ORIGIN, "http://localhost:8000")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, MCP_POST_ACCEPT)
        .header(
            header::AUTHORIZATION,
            format!("Bearer {TEST_STATIC_PRINCIPAL}"),
        )
        .header(MCP_PROTOCOL_VERSION_HEADER, unsupported)
        .header(MCP_METHOD_HEADER, "server/discover")
        .body(Body::from(
            json!({
                "jsonrpc":"2.0",
                "id":"malformed-unknown",
                "method":"server/discover"
            })
            .to_string(),
        ))
        .unwrap();
    assert_rpc_error(
        send(&router, malformed_unknown).await,
        StatusCode::BAD_REQUEST,
        json!("malformed-unknown"),
        -32602,
    )
    .await;
}

#[tokio::test]
async fn named_unsupported_primitives_validate_routing_before_method_lookup() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = stateless_test_router(file_tools);

    let resource = modern_body(
        json!("resource"),
        "resources/read",
        json!({"uri":"file:///projects/example/config.json"}),
    );
    let response = send(
        &router,
        modern_request(
            resource.clone(),
            "resources/read",
            Some("file:///projects/example/config.json"),
        ),
    )
    .await;
    assert_rpc_error(response, StatusCode::NOT_FOUND, json!("resource"), -32601).await;

    let response = send(&router, modern_request(resource, "resources/read", None)).await;
    assert_rpc_error(
        response,
        StatusCode::BAD_REQUEST,
        json!("resource"),
        HEADER_MISMATCH,
    )
    .await;

    let prompt = modern_body(
        json!("prompt"),
        "prompts/get",
        json!({"name":"review prompt"}),
    );
    let response = send(
        &router,
        modern_request(prompt, "prompts/get", Some("review prompt")),
    )
    .await;
    assert_rpc_error(response, StatusCode::NOT_FOUND, json!("prompt"), -32601).await;
}

#[tokio::test]
async fn mcp_name_plain_ascii_and_canonical_base64_forms_are_supported() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = stateless_test_router(file_tools);

    let plain = modern_body(
        json!("plain-name"),
        "tools/call",
        json!({"name":"read file","arguments":{}}),
    );
    assert_rpc_error(
        send(
            &router,
            modern_request(plain, "tools/call", Some("read file")),
        )
        .await,
        StatusCode::BAD_REQUEST,
        json!("plain-name"),
        -32602,
    )
    .await;

    let encoded = modern_body(
        json!("encoded-name"),
        "tools/call",
        json!({"name":"réad_file","arguments":{}}),
    );
    assert_rpc_error(
        send(
            &router,
            modern_request(
                encoded.clone(),
                "tools/call",
                Some("=?base64?csOpYWRfZmlsZQ==?="),
            ),
        )
        .await,
        StatusCode::BAD_REQUEST,
        json!("encoded-name"),
        -32602,
    )
    .await;

    let malformed = modern_request(encoded, "tools/call", Some("=?base64?%%%?="));
    assert_rpc_error(
        send(&router, malformed).await,
        StatusCode::BAD_REQUEST,
        json!("encoded-name"),
        HEADER_MISMATCH,
    )
    .await;

    let safe_encoded = modern_body(
        json!("safe-encoded"),
        "tools/call",
        json!({"name":"runtime_status","arguments":{}}),
    );
    let response = send(
        &router,
        modern_request(
            safe_encoded,
            "tools/call",
            Some("=?base64?cnVudGltZV9zdGF0dXM=?="),
        ),
    )
    .await;
    assert_json_not_session_response(&response);
    let payload = response_json(response).await;
    assert_modern_result_envelope(&payload, "safe-encoded");
    assert_eq!(
        payload["result"]["structuredContent"]["currentRequestProtocol"],
        MCP_STATELESS_PROTOCOL_VERSION
    );
    assert_eq!(
        payload["result"]["structuredContent"]["currentRequestSessionManagement"],
        "none"
    );
    assert_eq!(
        payload["result"]["structuredContent"]["currentRequestMutationMode"],
        "read_and_preview_only"
    );
}

#[tokio::test]
async fn stateless_post_ignores_legacy_session_and_replay_headers() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = stateless_test_router(file_tools);
    let mut request = modern_request(
        modern_body(json!("ignored-legacy"), "tools/list", json!({})),
        "tools/list",
        None,
    );
    request.headers_mut().append(
        MCP_SESSION_ID_HEADER,
        HeaderValue::from_static("not-a-session"),
    );
    request.headers_mut().append(
        MCP_SESSION_ID_HEADER,
        HeaderValue::from_static("also-not-a-session"),
    );
    request.headers_mut().append(
        MCP_LAST_EVENT_ID_HEADER,
        HeaderValue::from_static("legacy:7"),
    );
    request.headers_mut().append(
        MCP_LAST_EVENT_ID_HEADER,
        HeaderValue::from_static("legacy:8"),
    );

    let response = send(&router, request).await;
    assert_json_not_session_response(&response);
    let payload = response_json(response).await;
    assert_modern_result_envelope(&payload, "ignored-legacy");
}

#[tokio::test]
async fn stateless_get_delete_and_legacy_sse_replay_are_never_selected() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = stateless_sse_test_router(file_tools);

    for method in [Method::GET, Method::DELETE] {
        let response = send(&router, modern_method_request(method)).await;
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            response.headers().get(header::ALLOW),
            Some(&HeaderValue::from_static("POST"))
        );
        assert!(response.headers().get(MCP_SESSION_ID_HEADER).is_none());
        assert!(response.headers().get(header::CONTENT_TYPE).is_none());
        assert!(to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .is_empty());
    }

    let response = send(
        &router,
        modern_request(
            modern_body(json!("json-not-sse"), "tools/list", json!({})),
            "tools/list",
            None,
        ),
    )
    .await;
    assert_json_not_session_response(&response);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(!bytes.starts_with(b"id:"));
    let payload: Value = serde_json::from_slice(&bytes).unwrap();
    assert_modern_result_envelope(&payload, "json-not-sse");
}

#[tokio::test]
async fn removed_lifecycle_ping_notifications_and_client_responses_are_rejected() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = stateless_test_router(file_tools);

    for (id, method, fields) in [
        ("ping", "ping", json!({})),
        (
            "initialize",
            "initialize",
            json!({
                "protocolVersion": MCP_STATELESS_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name":"removed","version":"1.0.0"}
            }),
        ),
    ] {
        let response = send(
            &router,
            modern_request(modern_body(json!(id), method, fields), method, None),
        )
        .await;
        assert_rpc_error(response, StatusCode::NOT_FOUND, json!(id), -32601).await;
    }

    let notification = json!({
        "jsonrpc":"2.0",
        "method":"notifications/initialized",
        "params":{"_meta":request_meta(MCP_STATELESS_PROTOCOL_VERSION)}
    });
    let response = send(
        &router,
        modern_request(notification, "notifications/initialized", None),
    )
    .await;
    assert!(response.status().is_client_error());
    assert_ne!(response.status(), StatusCode::NOT_FOUND);
    assert!(response.headers().get(MCP_SESSION_ID_HEADER).is_none());

    let response_body = json!({"jsonrpc":"2.0","id":"client-response","result":{}});
    let request = Request::post("/mcp")
        .header(header::HOST, "localhost:8000")
        .header(header::ORIGIN, "http://localhost:8000")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, MCP_POST_ACCEPT)
        .header(
            header::AUTHORIZATION,
            format!("Bearer {TEST_STATIC_PRINCIPAL}"),
        )
        .header(MCP_PROTOCOL_VERSION_HEADER, MCP_STATELESS_PROTOCOL_VERSION)
        .body(Body::from(response_body.to_string()))
        .unwrap();
    let response = send(&router, request).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(response.headers().get(MCP_SESSION_ID_HEADER).is_none());
}

#[tokio::test]
async fn stateless_grants_and_explicit_mutation_are_fail_closed_and_private() {
    let (root, file_tools) = empty_test_file_tools();
    let router = stateless_test_router(file_tools);
    let target = root.path().join("must-not-be-created");
    let grant = "v1.secret-stateless-grant-must-not-be-reflected";

    let mut granted_preview = modern_request(
        modern_body(
            json!("grant"),
            "tools/call",
            json!({
                "name":"create_directory",
                "arguments":{
                    "path":target.to_string_lossy(),
                    "dry_run":true
                }
            }),
        ),
        "tools/call",
        Some("create_directory"),
    );
    granted_preview.headers_mut().insert(
        CREATE_DIRECTORY_GRANT_HEADER,
        HeaderValue::from_static(grant),
    );
    let response = send(&router, granted_preview).await;
    let payload = assert_rpc_error(response, StatusCode::BAD_REQUEST, json!("grant"), -32602).await;
    assert!(!payload.to_string().contains(grant));
    assert!(!target.exists());

    let mutation = modern_request(
        modern_body(
            json!("mutation"),
            "tools/call",
            json!({
                "name":"create_directory",
                "arguments":{
                    "path":target.to_string_lossy(),
                    "dry_run":false
                }
            }),
        ),
        "tools/call",
        Some("create_directory"),
    );
    let response = send(&router, mutation).await;
    assert_rpc_error(response, StatusCode::BAD_REQUEST, json!("mutation"), -32602).await;
    assert!(!target.exists());
}

#[tokio::test]
async fn stateless_opt_in_preserves_the_legacy_session_contract() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = stateless_test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let tools = post_json_to_session(
        router.clone(),
        &session_id,
        json!({"jsonrpc":"2.0","id":"legacy-list","method":"tools/list"}),
    )
    .await;
    assert_eq!(tools.status(), StatusCode::OK);
    let tools = response_json(tools).await;
    assert!(tools["result"]["tools"].is_array());
    assert!(tools["result"].get("resultType").is_none());

    let ping = post_json_to_session(
        router,
        &session_id,
        json!({"jsonrpc":"2.0","id":"legacy-ping","method":"ping"}),
    )
    .await;
    assert_eq!(ping.status(), StatusCode::OK);
    assert_eq!(
        response_json(ping).await,
        json!({"jsonrpc":"2.0","id":"legacy-ping","result":{}})
    );
}
