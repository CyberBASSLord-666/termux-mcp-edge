#![cfg(feature = "mcp-runtime")]

mod support;

use axum::{
    body::{to_bytes, Body},
    http::{header, HeaderValue, Request, StatusCode},
    response::Response,
    Router,
};
use serde_json::{json, Value};
use support::{
    empty_test_file_tools, json_request, response_json, session_request, sse_test_router,
    TEST_STATIC_PRINCIPAL,
};
use termux_mcp_server::mcp_transport::{
    MAX_MCP_JSON_RPC_ID_BYTES, MAX_MCP_LAST_EVENT_ID_BYTES, MCP_LAST_EVENT_ID_HEADER,
    MCP_PROTOCOL_VERSION, MCP_PROTOCOL_VERSION_HEADER, MCP_SESSION_ID_HEADER,
};
use termux_mcp_server::tools::{
    MAX_BINARY_READ_RESPONSE_BYTES, MAX_TEXT_RANGE_BYTES, MAX_TEXT_RANGE_RESPONSE_BYTES,
};
use tower::ServiceExt;
use uuid::Uuid;

#[derive(Debug, PartialEq, Eq)]
struct SseEvent {
    id: String,
    data: String,
    retry: Option<u64>,
}

fn initialize_body() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": "initialize-sse",
        "method": "initialize",
        "params": {
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": "termux-mcp-edge-sse-tests",
                "version": "1.0.0"
            }
        }
    })
}

fn parse_sse(body: &[u8]) -> Vec<SseEvent> {
    let body = std::str::from_utf8(body).expect("SSE must be UTF-8");
    if body.is_empty() {
        return Vec::new();
    }
    assert!(!body.contains('\r'));
    assert!(body.ends_with("\n\n"));
    body.trim_end_matches("\n\n")
        .split("\n\n")
        .map(|frame| {
            let mut lines = frame.lines();
            let id = lines
                .next()
                .and_then(|line| line.strip_prefix("id: "))
                .expect("SSE frame missing id")
                .to_owned();
            let next = lines.next().expect("SSE frame missing data");
            let (retry, data_line) = if let Some(retry) = next.strip_prefix("retry: ") {
                (
                    Some(retry.parse().expect("SSE retry must be an integer")),
                    lines.next().expect("SSE frame missing data after retry"),
                )
            } else {
                (None, next)
            };
            let data = data_line
                .strip_prefix("data: ")
                .expect("SSE frame missing data")
                .to_owned();
            assert!(lines.next().is_none());
            SseEvent { id, data, retry }
        })
        .collect()
}

async fn sse_events(response: Response) -> Vec<SseEvent> {
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static(
            "text/event-stream; charset=utf-8"
        ))
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    parse_sse(&body)
}

async fn initialize_sse(router: &Router) -> (String, Vec<SseEvent>) {
    let response = router
        .clone()
        .oneshot(json_request(initialize_body()))
        .await
        .unwrap();
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&HeaderValue::from_static("no-store"))
    );
    let session_id = response
        .headers()
        .get(MCP_SESSION_ID_HEADER)
        .expect("initialize response missing session")
        .to_str()
        .unwrap()
        .to_owned();
    let events = sse_events(response).await;
    (session_id, events)
}

async fn initialize_active_sse(router: &Router) -> (String, Vec<SseEvent>) {
    let (session_id, events) = initialize_sse(router).await;
    let initialized = router
        .clone()
        .oneshot(session_request(
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            &session_id,
        ))
        .await
        .unwrap();
    assert_eq!(initialized.status(), StatusCode::ACCEPTED);
    assert!(to_bytes(initialized.into_body(), usize::MAX)
        .await
        .unwrap()
        .is_empty());
    (session_id, events)
}

async fn post_ping(router: &Router, session_id: &str, id: usize) -> Vec<SseEvent> {
    let response = router
        .clone()
        .oneshot(session_request(
            json!({"jsonrpc":"2.0","id":id,"method":"ping"}),
            session_id,
        ))
        .await
        .unwrap();
    sse_events(response).await
}

fn resume_request(session_id: &str, last_event_id: &str) -> Request<Body> {
    Request::get("/mcp")
        .header(header::HOST, "localhost:8000")
        .header(header::ORIGIN, "http://localhost:8000")
        .header(header::ACCEPT, "text/event-stream")
        .header(
            header::AUTHORIZATION,
            format!("Bearer {TEST_STATIC_PRINCIPAL}"),
        )
        .header(MCP_PROTOCOL_VERSION_HEADER, MCP_PROTOCOL_VERSION)
        .header(MCP_SESSION_ID_HEADER, session_id)
        .header(MCP_LAST_EVENT_ID_HEADER, last_event_id)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn opt_in_sse_primes_then_delivers_one_terminal_json_rpc_response() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = sse_test_router(file_tools);
    let (session_id, initialize_events) = initialize_active_sse(&router).await;

    assert_eq!(initialize_events.len(), 2);
    assert!(initialize_events[0].data.is_empty());
    assert_eq!(initialize_events[0].retry, Some(1_000));
    assert_eq!(initialize_events[1].retry, None);
    let (initialize_stream, initialize_sequence) =
        initialize_events[0].id.rsplit_once(':').unwrap();
    assert_eq!(initialize_sequence, "0");
    assert_eq!(
        Uuid::parse_str(initialize_stream).unwrap().to_string(),
        initialize_stream
    );
    assert_eq!(initialize_events[1].id, format!("{initialize_stream}:1"));
    assert_eq!(
        serde_json::from_str::<Value>(&initialize_events[1].data).unwrap()["id"],
        "initialize-sse"
    );

    let ping_events = post_ping(&router, &session_id, 7).await;
    assert_eq!(ping_events.len(), 2);
    assert!(ping_events[0].data.is_empty());
    assert_eq!(
        serde_json::from_str::<Value>(&ping_events[1].data).unwrap(),
        json!({"jsonrpc":"2.0","id":7,"result":{}})
    );
    assert_ne!(
        ping_events[0].id.rsplit_once(':').unwrap().0,
        initialize_stream
    );

    let runtime = router
        .oneshot(session_request(
            json!({
                "jsonrpc":"2.0",
                "id":"sse-runtime",
                "method":"tools/call",
                "params":{"name":"runtime_status","arguments":{}}
            }),
            &session_id,
        ))
        .await
        .unwrap();
    let runtime = sse_events(runtime).await;
    let runtime: Value = serde_json::from_str(&runtime[1].data).unwrap();
    let structured = &runtime["result"]["structuredContent"];
    assert_eq!(structured["serverSentEvents"], true);
    assert_eq!(
        structured["serverSentEventsMode"],
        "finite_request_response_with_origin_stream_replay"
    );
    assert_eq!(structured["jsonRpcIdMaxBytes"], MAX_MCP_JSON_RPC_ID_BYTES);
}

#[tokio::test]
async fn get_replays_only_events_after_the_exact_originating_cursor() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = sse_test_router(file_tools);
    let (session_id, _) = initialize_active_sse(&router).await;
    let ping_events = post_ping(&router, &session_id, 9).await;

    let replay = router
        .clone()
        .oneshot(resume_request(&session_id, &ping_events[0].id))
        .await
        .unwrap();
    assert_eq!(
        sse_events(replay).await,
        vec![SseEvent {
            id: ping_events[1].id.clone(),
            data: ping_events[1].data.clone(),
            retry: None,
        }]
    );

    let completed = router
        .clone()
        .oneshot(resume_request(&session_id, &ping_events[1].id))
        .await
        .unwrap();
    assert!(sse_events(completed).await.is_empty());

    let no_cursor = Request::get("/mcp")
        .header(header::HOST, "localhost:8000")
        .header(header::ORIGIN, "http://localhost:8000")
        .header(header::ACCEPT, "text/event-stream")
        .header(
            header::AUTHORIZATION,
            format!("Bearer {TEST_STATIC_PRINCIPAL}"),
        )
        .header(MCP_PROTOCOL_VERSION_HEADER, MCP_PROTOCOL_VERSION)
        .header(MCP_SESSION_ID_HEADER, &session_id)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        router.oneshot(no_cursor).await.unwrap().status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
}

#[tokio::test]
async fn replay_cursors_fail_closed_across_sessions_and_after_eviction() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = sse_test_router(file_tools);
    let (first_session, _) = initialize_active_sse(&router).await;
    let (second_session, _) = initialize_active_sse(&router).await;
    let first_events = post_ping(&router, &first_session, 1).await;

    let crossed = router
        .clone()
        .oneshot(resume_request(&second_session, &first_events[0].id))
        .await
        .unwrap();
    assert_eq!(crossed.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response_json(crossed).await["error"],
        "sse_cursor_not_found"
    );

    let mut oldest = Vec::new();
    for id in 10..19 {
        let events = post_ping(&router, &first_session, id).await;
        if oldest.is_empty() {
            oldest = events;
        }
    }
    let evicted = router
        .oneshot(resume_request(&first_session, &oldest[0].id))
        .await
        .unwrap();
    assert_eq!(evicted.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response_json(evicted).await["error"],
        "sse_cursor_not_found"
    );
}

#[tokio::test]
async fn malformed_duplicate_oversized_and_unknown_cursors_are_rejected() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = sse_test_router(file_tools);
    let (session_id, _) = initialize_active_sse(&router).await;

    let oversized = "x".repeat(MAX_MCP_LAST_EVENT_ID_BYTES + 1);
    for cursor in [
        "not-an-event",
        "00000000-0000-4000-8000-000000000000:01",
        oversized.as_str(),
    ] {
        let response = router
            .clone()
            .oneshot(resume_request(&session_id, cursor))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response_json(response).await["error"],
            "invalid_last_event_id"
        );
    }

    let unknown = format!("{}:0", Uuid::new_v4());
    let response = router
        .clone()
        .oneshot(resume_request(&session_id, &unknown))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response_json(response).await["error"],
        "sse_cursor_not_found"
    );

    let mut duplicate = resume_request(&session_id, &unknown);
    duplicate.headers_mut().append(
        MCP_LAST_EVENT_ID_HEADER,
        HeaderValue::from_static("00000000-0000-4000-8000-000000000000:0"),
    );
    let response = router.oneshot(duplicate).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(response).await["error"],
        "invalid_last_event_id"
    );
}

#[tokio::test]
async fn transport_and_session_validation_precede_sse_cursor_lookup() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = sse_test_router(file_tools);
    let (session_id, _) = initialize_active_sse(&router).await;
    let events = post_ping(&router, &session_id, 12).await;

    let mut rejected_origin = resume_request(&session_id, &events[0].id);
    rejected_origin.headers_mut().insert(
        header::ORIGIN,
        HeaderValue::from_static("https://attacker.invalid"),
    );
    let response = router.clone().oneshot(rejected_origin).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(response).await["error"],
        "transport_security_rejected"
    );

    let unknown_session = "00000000-0000-4000-8000-000000000000";
    let response = router
        .clone()
        .oneshot(resume_request(unknown_session, "not-an-event"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(response_json(response).await["error"], "session_not_found");

    let mut wrong_protocol = resume_request(&session_id, &events[0].id);
    wrong_protocol.headers_mut().insert(
        MCP_PROTOCOL_VERSION_HEADER,
        HeaderValue::from_static("2025-03-26"),
    );
    let response = router.oneshot(wrong_protocol).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(response).await["error"],
        "unsupported_protocol_version"
    );
}

#[tokio::test]
async fn large_json_rpc_responses_fall_back_to_json_without_unbounded_replay() {
    let (root, file_tools) = empty_test_file_tools();
    let large_path = root.path().join("large.txt");
    std::fs::write(&large_path, "a".repeat(128 * 1024 + 1)).unwrap();
    let router = sse_test_router(file_tools);
    let (session_id, _) = initialize_active_sse(&router).await;

    let response = router
        .clone()
        .oneshot(session_request(
            json!({
                "jsonrpc":"2.0",
                "id":"large-read",
                "method":"tools/call",
                "params":{"name":"read_file","arguments":{"path":large_path.to_string_lossy()}}
            }),
            &session_id,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response
        .headers()
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("application/json"));
    assert_eq!(response_json(response).await["id"], "large-read");
}

#[tokio::test]
async fn maximum_escaped_text_range_reaches_bounded_json_fallback() {
    let (root, file_tools) = empty_test_file_tools();
    let path = root.path().join("escaped-text-range.txt");
    std::fs::write(&path, vec![0_u8; MAX_TEXT_RANGE_BYTES]).unwrap();
    let router = sse_test_router(file_tools);
    let (session_id, _) = initialize_active_sse(&router).await;

    let response = router
        .oneshot(session_request(
            json!({
                "jsonrpc":"2.0",
                "id":"escaped-text-range",
                "method":"tools/call",
                "params":{
                    "name":"read_text_range",
                    "arguments":{
                        "path":path.to_string_lossy(),
                        "offset_bytes":0,
                        "max_bytes":MAX_TEXT_RANGE_BYTES
                    }
                }
            }),
            &session_id,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response
        .headers()
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("application/json"));
    let body = to_bytes(response.into_body(), MAX_TEXT_RANGE_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(body.len() > MAX_BINARY_READ_RESPONSE_BYTES);
    assert!(body.len() <= MAX_TEXT_RANGE_RESPONSE_BYTES);
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["id"], "escaped-text-range");
    let content = payload["result"]["structuredContent"]["content"]
        .as_str()
        .unwrap();
    assert_eq!(content, "\0".repeat(MAX_TEXT_RANGE_BYTES));
}

#[tokio::test]
async fn bounded_json_rpc_ids_never_fail_sse_collection_or_orphan_initialization() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = sse_test_router(file_tools);
    let maximum_id = "x".repeat(MAX_MCP_JSON_RPC_ID_BYTES - 2);
    let oversized_id = "x".repeat(MAX_MCP_JSON_RPC_ID_BYTES - 1);
    let escaped_oversized_id = "\0".repeat(MAX_MCP_JSON_RPC_ID_BYTES / 6 + 1);

    let mut oversized_initialize = initialize_body();
    oversized_initialize["id"] = json!(oversized_id.clone());
    let response = router
        .clone()
        .oneshot(json_request(oversized_initialize))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(response.headers().get(MCP_SESSION_ID_HEADER).is_none());
    let payload = response_json(response).await;
    assert_eq!(payload["id"], Value::Null);
    assert_eq!(payload["error"]["code"], -32001);

    let (session_id, _) = initialize_active_sse(&router).await;
    let maximum_ping = router
        .clone()
        .oneshot(session_request(
            json!({"jsonrpc":"2.0","id":maximum_id,"method":"ping"}),
            &session_id,
        ))
        .await
        .unwrap();
    assert_eq!(maximum_ping.status(), StatusCode::OK);
    assert!(maximum_ping
        .headers()
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("application/json"));
    assert_eq!(
        response_json(maximum_ping).await["id"]
            .as_str()
            .unwrap()
            .len(),
        MAX_MCP_JSON_RPC_ID_BYTES - 2
    );

    for body in [
        json!({"jsonrpc":"2.0","id":oversized_id.clone(),"method":"ping"}),
        json!({"jsonrpc":"2.0","id":escaped_oversized_id,"method":"ping"}),
        json!({"jsonrpc":"2.0","id":oversized_id.clone(),"method":"tools/list"}),
        json!({
            "jsonrpc":"2.0",
            "id":oversized_id,
            "method":"tools/call",
            "params":{"name":"runtime_status","arguments":{}}
        }),
    ] {
        let response = router
            .clone()
            .oneshot(session_request(body, &session_id))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(response
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("application/json"));
        let payload = response_json(response).await;
        assert_eq!(payload["id"], Value::Null);
        assert_eq!(payload["error"]["code"], -32001);
    }
}

#[tokio::test]
async fn notifications_stay_empty_and_delete_removes_replay_state() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = sse_test_router(file_tools);
    let (session_id, _) = initialize_active_sse(&router).await;
    let ping_events = post_ping(&router, &session_id, 3).await;

    let notification = router
        .clone()
        .oneshot(session_request(
            json!({"jsonrpc":"2.0","method":"notifications/example"}),
            &session_id,
        ))
        .await
        .unwrap();
    assert_eq!(notification.status(), StatusCode::ACCEPTED);
    assert!(notification.headers().get(header::CONTENT_TYPE).is_none());
    assert!(to_bytes(notification.into_body(), usize::MAX)
        .await
        .unwrap()
        .is_empty());

    let delete = Request::delete("/mcp")
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
        router.clone().oneshot(delete).await.unwrap().status(),
        StatusCode::NO_CONTENT
    );

    let replay = router
        .oneshot(resume_request(&session_id, &ping_events[0].id))
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::NOT_FOUND);
    assert_eq!(response_json(replay).await["error"], "session_not_found");
}
