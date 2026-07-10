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

    assert_eq!(payload["id"], "exact-limit-dry-run");
    assert_eq!(
        payload["result"]["content"][0]["text"],
        EXPECTED_DRY_RUN_RESPONSE
    );
    assert_eq!(
        payload["result"]["structuredContent"],
        json!({
            "dryRun": true,
            "bytes": DEFAULT_MAX_WRITE_BYTES,
            "message": EXPECTED_DRY_RUN_RESPONSE
        })
    );
    assert_eq!(payload["result"]["isError"], false);
    assert!(!target.exists());
}
