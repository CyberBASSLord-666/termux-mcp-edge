#![cfg(feature = "mcp-runtime")]

mod support;

use axum::{
    body::Body,
    http::{header, HeaderName, HeaderValue, Request, StatusCode},
    response::Response,
};
use support::{response_json, test_file_tools, test_router, TEST_STATIC_PRINCIPAL};
use tower::ServiceExt;

async fn duplicate_transport_header_response(
    duplicate_name: HeaderName,
    values: [&'static str; 2],
    authenticated: bool,
) -> Response {
    let (_root, file_tools) = test_file_tools();
    let mut request = Request::post("/mcp")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("not-json"))
        .unwrap();
    let headers = request.headers_mut();

    if duplicate_name == header::HOST {
        for value in values {
            headers.append(header::HOST, HeaderValue::from_static(value));
        }
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://localhost:8000"),
        );
    } else {
        headers.insert(header::HOST, HeaderValue::from_static("localhost:8000"));
        for value in values {
            headers.append(header::ORIGIN, HeaderValue::from_static(value));
        }
    }
    if authenticated {
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::try_from(format!("Bearer {TEST_STATIC_PRINCIPAL}")).unwrap(),
        );
    }

    test_router(file_tools).oneshot(request).await.unwrap()
}

async fn non_text_transport_header_response(header_name: HeaderName) -> Response {
    let (_root, file_tools) = test_file_tools();
    let mut request = Request::post("/mcp")
        .header(
            header::AUTHORIZATION,
            format!("Bearer {TEST_STATIC_PRINCIPAL}"),
        )
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::HOST, "localhost:8000")
        .header(header::ORIGIN, "http://localhost:8000")
        .body(Body::from("not-json"))
        .unwrap();
    request
        .headers_mut()
        .insert(header_name, HeaderValue::from_bytes(&[0x80]).unwrap());

    test_router(file_tools).oneshot(request).await.unwrap()
}

#[tokio::test]
async fn missing_host_response_uses_stable_reason_code() {
    let (_root, file_tools) = test_file_tools();
    let response = test_router(file_tools)
        .oneshot(
            Request::post("/mcp")
                .header(header::HOST, "")
                .header(
                    header::AUTHORIZATION,
                    format!("Bearer {TEST_STATIC_PRINCIPAL}"),
                )
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
                .header(
                    header::AUTHORIZATION,
                    format!("Bearer {TEST_STATIC_PRINCIPAL}"),
                )
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
                .header(
                    header::AUTHORIZATION,
                    format!("Bearer {TEST_STATIC_PRINCIPAL}"),
                )
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
                .header(
                    header::AUTHORIZATION,
                    format!("Bearer {TEST_STATIC_PRINCIPAL}"),
                )
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
                .header(
                    header::AUTHORIZATION,
                    format!("Bearer {TEST_STATIC_PRINCIPAL}"),
                )
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

#[tokio::test]
async fn duplicate_host_headers_are_rejected_before_body_parsing_without_reflection() {
    for values in [
        ["localhost:8000", "localhost:8000"],
        ["localhost:8000", "attacker.example:8000"],
        ["attacker.example:8000", "localhost:8000"],
    ] {
        let response = duplicate_transport_header_response(header::HOST, values, true).await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let payload = response_json(response).await;
        assert_eq!(payload["error"], "transport_security_rejected");
        assert_eq!(payload["message"], "invalid_host_header");
        assert!(!payload.to_string().contains("attacker.example"));
    }
}

#[tokio::test]
async fn duplicate_origin_headers_are_rejected_before_body_parsing_without_reflection() {
    for values in [
        ["http://localhost:8000", "http://localhost:8000"],
        ["http://localhost:8000", "https://attacker.example"],
        ["https://attacker.example", "http://localhost:8000"],
    ] {
        let response = duplicate_transport_header_response(header::ORIGIN, values, true).await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let payload = response_json(response).await;
        assert_eq!(payload["error"], "transport_security_rejected");
        assert_eq!(payload["message"], "invalid_origin_header");
        assert!(!payload.to_string().contains("attacker.example"));
    }
}

#[tokio::test]
async fn non_text_transport_headers_are_rejected_before_body_parsing_without_reflection() {
    for (header_name, expected_reason) in [
        (header::HOST, "invalid_host_header"),
        (header::ORIGIN, "invalid_origin_header"),
    ] {
        let response = non_text_transport_header_response(header_name).await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let payload = response_json(response).await;
        assert_eq!(payload["error"], "transport_security_rejected");
        assert_eq!(payload["message"], expected_reason);
        assert!(!payload.to_string().contains("\\u0080"));
    }
}

#[tokio::test]
async fn authentication_remains_outermost_for_duplicate_transport_headers() {
    for duplicate_name in [header::HOST, header::ORIGIN] {
        let response = duplicate_transport_header_response(
            duplicate_name,
            ["localhost:8000", "attacker.example:8000"],
            false,
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let payload = response_json(response).await;
        assert_eq!(payload["error"], "unauthorized");
        assert!(!payload.to_string().contains("invalid_"));
    }
}
