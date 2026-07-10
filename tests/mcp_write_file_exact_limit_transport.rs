#![cfg(feature = "mcp-runtime")]

mod support;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use serde_json::json;
use support::{empty_test_file_tools, response_json, test_router};
use termux_mcp_server::write_policy::DEFAULT_MAX_WRITE_BYTES;
use tower::ServiceExt;

const EXPECTED_DRY_RUN_RESPONSE: &str = "DRY-RUN";

#[tokio::test]
async fn transport_write_file_allows_exact_default_limit_as_dry_run_preview() {
    let (root, file_tools) = empty_test_file_tools();
    let target = root.path().join("transport-exact-limit-dry-run.txt");
    let content = "x".repeat(DEFAULT_MAX_WRITE_BYTES);

    let response = test_router(file_tools)
        .oneshot(
            Request::post("/mcp")
                .header(header::HOST, "localhost:8000")
                .header(header::ORIGIN, "http://localhost:8000")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "jsonrpc": "2.0",
                        "id": "exact-limit-dry-run",
                        "method": "tools/call",
                        "params": {
                            "name": "write_file",
                            "arguments": {
                                "path": target.to_string_lossy(),
                                "content": content
                            }
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let payload = response_json(response).await;
    let result = payload.get("result").expect("response missing result");
    let content_text = result["content"][0]["text"]
        .as_str()
        .expect("response missing content text");
    let structured_content = result
        .get("structuredContent")
        .expect("response missing structuredContent");
    let expected_structured_content = json!({
        "dryRun": true,
        "bytes": DEFAULT_MAX_WRITE_BYTES,
        "message": EXPECTED_DRY_RUN_RESPONSE
    });
    let is_error = result.get("isError").and_then(|value| value.as_bool());

    assert_eq!(payload["id"], "exact-limit-dry-run");
    assert_eq!(content_text, EXPECTED_DRY_RUN_RESPONSE);
    assert_eq!(structured_content, &expected_structured_content);
    assert_eq!(is_error, Some(false));
    assert!(!target.exists());
}

#[tokio::test]
async fn transport_write_file_allows_exact_default_limit_with_explicit_mutation() {
    let (root, file_tools) = empty_test_file_tools();
    let target = root.path().join("transport-exact-limit-mutation.txt");
    let content = "x".repeat(DEFAULT_MAX_WRITE_BYTES);
    let expected_response = format!("Wrote {DEFAULT_MAX_WRITE_BYTES} bytes");

    let response = test_router(file_tools)
        .oneshot(
            Request::post("/mcp")
                .header(header::HOST, "localhost:8000")
                .header(header::ORIGIN, "http://localhost:8000")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "jsonrpc": "2.0",
                        "id": "exact-limit-mutation",
                        "method": "tools/call",
                        "params": {
                            "name": "write_file",
                            "arguments": {
                                "path": target.to_string_lossy(),
                                "content": content,
                                "dry_run": false
                            }
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let payload = response_json(response).await;
    let result = payload.get("result").expect("response missing result");
    let content_text = result["content"][0]["text"]
        .as_str()
        .expect("response missing content text");
    let structured_content = result
        .get("structuredContent")
        .expect("response missing structuredContent");
    let expected_structured_content = json!({
        "dryRun": false,
        "bytes": DEFAULT_MAX_WRITE_BYTES,
        "message": expected_response
    });
    let is_error = result.get("isError").and_then(|value| value.as_bool());

    assert_eq!(payload["id"], "exact-limit-mutation");
    assert_eq!(content_text, expected_response);
    assert_eq!(structured_content, &expected_structured_content);
    assert_eq!(is_error, Some(false));
    assert_eq!(
        tokio::fs::metadata(&target).await.unwrap().len(),
        DEFAULT_MAX_WRITE_BYTES as u64
    );
    let written_content = tokio::fs::read_to_string(&target).await.unwrap();
    assert_eq!(written_content, content);
}
