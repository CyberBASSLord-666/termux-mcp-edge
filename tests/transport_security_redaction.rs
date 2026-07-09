#![cfg(feature = "mcp-runtime")]

mod support;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use support::{response_json, test_file_tools, test_router};
use tower::ServiceExt;

#[tokio::test]
async fn missing_host_response_uses_stable_reason_code() {
    let (_root, file_tools) = test_file_tools();
    let response = test_router(file_tools)
        .oneshot(
            Request::post("/mcp")
                .header(header::ORIGIN, "http://localhost:8000")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("not-json"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let payload = response_json(response).await;
    assert_eq!(payload["error"], "transport_security_rejected");
    assert_eq!(payload["message"], "missing_host");
}

#[tokio::test]
async fn required_origin_response_uses_stable_reason_code() {
    let (_root, file_tools) = test_file_tools();
    let response = test_router(file_tools)
        .oneshot(
            Request::post("/mcp")
                .header(header::HOST, "localhost:8000")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("not-json"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let payload = response_json(response).await;
    assert_eq!(payload["error"], "transport_security_rejected");
    assert_eq!(payload["message"], "origin_required");
}

#[tokio::test]
async fn disallowed_host_response_uses_stable_reason_code() {
    let (_root, file_tools) = test_file_tools();
    let response = test_router(file_tools)
        .oneshot(
            Request::post("/mcp")
                .header(header::HOST, "attacker.example:8000")
                .header(header::ORIGIN, "http://localhost:8000")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("not-json"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let payload = response_json(response).await;
    assert_eq!(payload["error"], "transport_security_rejected");
    assert_eq!(payload["message"], "host_not_allowed");

    let serialized = payload.to_string();
    assert!(!serialized.contains("attacker.example"));
}

#[tokio::test]
async fn disallowed_origin_response_uses_stable_reason_code() {
    let (_root, file_tools) = test_file_tools();
    let response = test_router(file_tools)
        .oneshot(
            Request::post("/mcp")
                .header(header::HOST, "localhost:8000")
                .header(header::ORIGIN, "https://attacker.example")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("not-json"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let payload = response_json(response).await;
    assert_eq!(payload["error"], "transport_security_rejected");
    assert_eq!(payload["message"], "origin_not_allowed");

    let serialized = payload.to_string();
    assert!(!serialized.contains("attacker.example"));
}

#[tokio::test]
async fn malformed_origin_response_uses_stable_reason_code() {
    let (_root, file_tools) = test_file_tools();
    let response = test_router(file_tools)
        .oneshot(
            Request::post("/mcp")
                .header(header::HOST, "localhost:8000")
                .header(header::ORIGIN, "https://identity@localhost:8000")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("not-json"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let payload = response_json(response).await;
    assert_eq!(payload["error"], "transport_security_rejected");
    assert_eq!(payload["message"], "invalid_origin");

    let serialized = payload.to_string();
    assert!(!serialized.contains("identity@localhost"));
}
