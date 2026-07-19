#![cfg(feature = "mcp-runtime")]

mod support;

use axum::http::StatusCode;
use serde_json::json;
use support::{
    empty_test_file_tools, initialize_session, issue_write_file_grant, post_json_to_session,
    post_json_to_session_with_grant, response_json, test_router, write_file_authorized_test_router,
};
use termux_mcp_server::{
    tools::MAX_WRITE_FILE_RESPONSE_BYTES, write_file_grant::WriteFileDisposition,
    write_policy::DEFAULT_MAX_WRITE_BYTES,
};

const EXPECTED_DRY_RUN_RESPONSE: &str =
    "Validated one bounded safe-rooted UTF-8 file write without mutation.";
const EXPECTED_MUTATION_RESPONSE: &str =
    "Wrote one bounded safe-rooted UTF-8 file with fixed mode 0600.";

#[tokio::test]
async fn transport_write_file_allows_exact_default_limit_as_dry_run_preview() {
    let (root, file_tools) = empty_test_file_tools();
    let target = root.path().join("transport-exact-limit-dry-run.txt");
    let content = "x".repeat(DEFAULT_MAX_WRITE_BYTES);

    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router,
        &session_id,
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
        }),
    )
    .await;

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
        "sizeBytes": DEFAULT_MAX_WRITE_BYTES,
        "disposition": "create",
        "mode": "0600",
        "maxFileBytes": DEFAULT_MAX_WRITE_BYTES,
        "maxResponseBytes": MAX_WRITE_FILE_RESPONSE_BYTES,
        "recoveryArtifactRetained": false,
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

    let issuer_tools = file_tools.clone();
    let (router, authority) = write_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let grant = issue_write_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        target.to_string_lossy().as_ref(),
        content.as_bytes(),
        WriteFileDisposition::Create,
    );
    let response = post_json_to_session_with_grant(
        router,
        &session_id,
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
        }),
        &grant,
    )
    .await;

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
        "sizeBytes": DEFAULT_MAX_WRITE_BYTES,
        "disposition": "create",
        "mode": "0600",
        "maxFileBytes": DEFAULT_MAX_WRITE_BYTES,
        "maxResponseBytes": MAX_WRITE_FILE_RESPONSE_BYTES,
        "recoveryArtifactRetained": false,
    });
    let is_error = result.get("isError").and_then(|value| value.as_bool());

    assert_eq!(payload["id"], "exact-limit-mutation");
    assert_eq!(content_text, EXPECTED_MUTATION_RESPONSE);
    assert_eq!(structured_content, &expected_structured_content);
    assert_eq!(is_error, Some(false));
    assert_eq!(
        tokio::fs::metadata(&target).await.unwrap().len(),
        DEFAULT_MAX_WRITE_BYTES as u64
    );
    let written_content = tokio::fs::read_to_string(&target).await.unwrap();
    assert_eq!(written_content, content);
}
