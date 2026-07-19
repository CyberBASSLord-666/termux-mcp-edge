#![cfg(feature = "mcp-runtime")]

mod support;

use std::{os::unix::fs::PermissionsExt, sync::Arc};

use axum::{
    body::{to_bytes, Body},
    http::{header, HeaderValue, Method, Request, StatusCode},
};
use serde_json::{json, Value};
use support::{
    copy_file_authorized_test_router, empty_test_file_tools, initialize_session,
    issue_copy_file_grant, post_json_to_session, post_json_to_session_with_grant, response_json,
    session_request, TEST_CAPABILITY_KEY, TEST_STATIC_PRINCIPAL,
};
use termux_mcp_server::{
    copy_file_grant::{
        CopyFileGrantAuthority, COPY_FILE_GRANT_HEADER, COPY_FILE_GRANT_TTL_SECONDS,
        MAX_COPY_FILE_GRANT_HEADER_BYTES,
    },
    mcp_transport::{
        MAX_MCP_JSON_RPC_ID_BYTES, MCP_POST_ACCEPT, MCP_PROTOCOL_VERSION,
        MCP_PROTOCOL_VERSION_HEADER, MCP_SESSION_ID_HEADER,
    },
    tools::{COPY_FILE_MODE, MAX_COPY_FILE_BYTES, MAX_COPY_FILE_RESPONSE_BYTES},
    write_file_grant::{WriteFileDisposition, WriteFileGrantAuthority},
};
use tower::ServiceExt;

const DRY_RUN_SUMMARY: &str = "Validated one bounded safe-rooted file copy without mutation.";
const MUTATION_SUMMARY: &str = "Copied one bounded safe-rooted file with fixed mode 0600.";

fn copy_call(
    id: impl Into<Value>,
    source_path: &str,
    destination_path: &str,
    dry_run: Option<bool>,
) -> Value {
    let mut arguments = json!({
        "source_path": source_path,
        "destination_path": destination_path,
    });
    if let Some(dry_run) = dry_run {
        arguments["dry_run"] = json!(dry_run);
    }
    json!({
        "jsonrpc": "2.0",
        "id": id.into(),
        "method": "tools/call",
        "params": {"name": "copy_file", "arguments": arguments},
    })
}

fn runtime_status_call(id: impl Into<Value>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.into(),
        "method": "tools/call",
        "params": {"name": "runtime_status", "arguments": {}},
    })
}

fn corrupt_signature(token: &str) -> String {
    let mut corrupted = token.as_bytes().to_vec();
    let last = corrupted.last_mut().expect("grant is non-empty");
    *last = if *last == b'0' { b'1' } else { b'0' };
    String::from_utf8(corrupted).unwrap()
}

fn assert_capability_denial(body: &Value, reason: &str) {
    assert_eq!(body["error"]["code"], -32003);
    assert_eq!(body["error"]["data"]["reason"], reason);
}

fn assert_path_free_copy_result(body: &Value, dry_run: bool, size: usize) {
    let result = &body["result"];
    assert_eq!(result["isError"], false);
    assert_eq!(
        result["content"][0]["text"],
        if dry_run {
            DRY_RUN_SUMMARY
        } else {
            MUTATION_SUMMARY
        }
    );
    assert_eq!(
        result["structuredContent"],
        json!({
            "dryRun": dry_run,
            "sizeBytes": size,
            "mode": "0600",
            "maxFileBytes": MAX_COPY_FILE_BYTES,
            "maxResponseBytes": MAX_COPY_FILE_RESPONSE_BYTES,
        })
    );
    let serialized = result.to_string();
    assert!(!serialized.contains("sourcePath"));
    assert!(!serialized.contains("destinationPath"));
}

#[tokio::test]
async fn enabled_discovery_status_and_missing_grant_are_exact_and_fail_closed() {
    let (_root, file_tools) = empty_test_file_tools();
    let (router, _authority) = copy_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let discovery = response_json(
        post_json_to_session(
            router.clone(),
            &session_id,
            json!({"jsonrpc":"2.0","id":"tools","method":"tools/list"}),
        )
        .await,
    )
    .await;
    let copy = discovery["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "copy_file")
        .unwrap();
    assert_eq!(copy["inputSchema"]["type"], "object");
    assert_eq!(
        copy["inputSchema"]["required"],
        json!(["source_path", "destination_path"])
    );
    assert_eq!(copy["inputSchema"]["additionalProperties"], false);
    assert_eq!(
        copy["inputSchema"]["properties"].as_object().unwrap().len(),
        3
    );
    assert!(copy["inputSchema"]["properties"]["dry_run"]
        .get("const")
        .is_none());
    assert!(copy["description"]
        .as_str()
        .unwrap()
        .contains("source-identity/content/destination-bound"));

    let outside = tempfile::tempdir().unwrap();
    let destination = outside.path().join("must-not-exist.bin");
    let denied = post_json_to_session(
        router.clone(),
        &session_id,
        copy_call(
            "missing",
            outside.path().join("unreadable").to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
            Some(false),
        ),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert_capability_denial(&response_json(denied).await, "capability_grant_missing");
    assert!(!destination.exists());

    let status = response_json(
        post_json_to_session(router, &session_id, runtime_status_call("status")).await,
    )
    .await;
    let runtime = &status["result"]["structuredContent"];
    assert_eq!(runtime["copyFileMutationEnabled"], true);
    assert_eq!(runtime["copyFileGrantRequired"], true);
    assert_eq!(runtime["copyFileGrantHeader"], COPY_FILE_GRANT_HEADER);
    assert_eq!(
        runtime["copyFileGrantTtlSeconds"],
        COPY_FILE_GRANT_TTL_SECONDS
    );
    assert_eq!(
        runtime["copyFileMode"],
        "dry_run_or_source_content_destination_scoped_single_use_grant"
    );
    assert_eq!(
        runtime["copyFileGrantBinding"],
        "source_root_path_identity_size_sha256_destination_root_path_absent_no_replace"
    );
    assert_eq!(runtime["copyFileMaxBytes"], MAX_COPY_FILE_BYTES);
    assert_eq!(
        runtime["copyFileMaxResponseBytes"],
        MAX_COPY_FILE_RESPONSE_BYTES
    );
    assert_eq!(
        runtime["copyFileResponsePosture"],
        "path_free_bounded_metadata_only"
    );
}

#[tokio::test]
async fn exact_grant_survives_preview_smuggling_and_response_preflight_then_copies_once() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let source = root.path().join("private-source.bin");
    let destination = root.path().join("private-destination.bin");
    let content = b"private-authorized-copy\0\xff";
    std::fs::write(&source, content).unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o777)).unwrap();
    let (router, authority) = copy_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let grant = issue_copy_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        source.to_string_lossy().as_ref(),
        destination.to_string_lossy().as_ref(),
    );

    let smuggled_preview = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        copy_call(
            "smuggled-preview",
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
            Some(true),
        ),
        &grant,
    )
    .await;
    assert_eq!(smuggled_preview.status(), StatusCode::BAD_REQUEST);
    assert!(!response_json(smuggled_preview)
        .await
        .to_string()
        .contains(&grant));
    assert!(!destination.exists());

    let preview = post_json_to_session(
        router.clone(),
        &session_id,
        copy_call(
            "preview",
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
            None,
        ),
    )
    .await;
    assert_eq!(preview.status(), StatusCode::OK);
    assert_path_free_copy_result(&response_json(preview).await, true, content.len());
    assert!(!destination.exists());

    let oversized_id = "x".repeat(MAX_COPY_FILE_RESPONSE_BYTES);
    assert!(oversized_id.len() < MAX_MCP_JSON_RPC_ID_BYTES);
    let preflight = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        copy_call(
            oversized_id,
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
            Some(false),
        ),
        &grant,
    )
    .await;
    assert_eq!(preflight.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = to_bytes(preflight.into_body(), MAX_COPY_FILE_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(body.len() <= MAX_COPY_FILE_RESPONSE_BYTES);
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["id"], Value::Null);
    assert!(!destination.exists());

    let copied = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        copy_call(
            "copy",
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
            Some(false),
        ),
        &grant,
    )
    .await;
    assert_eq!(copied.status(), StatusCode::OK);
    let copied = response_json(copied).await;
    assert_path_free_copy_result(&copied, false, content.len());
    assert_eq!(std::fs::read(&destination).unwrap(), content);
    assert_eq!(
        std::fs::metadata(&destination)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        COPY_FILE_MODE
    );

    std::fs::remove_file(&destination).unwrap();
    let replay = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        copy_call(
            "replay",
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
            Some(false),
        ),
        &grant,
    )
    .await;
    assert_eq!(replay.status(), StatusCode::FORBIDDEN);
    assert_capability_denial(&response_json(replay).await, "capability_grant_replayed");
    assert!(!destination.exists());

    let status = response_json(
        post_json_to_session(router, &session_id, runtime_status_call("audit")).await,
    )
    .await;
    let serialized = status.to_string();
    for forbidden in [
        TEST_CAPABILITY_KEY,
        TEST_STATIC_PRINCIPAL,
        grant.as_str(),
        source.to_string_lossy().as_ref(),
        destination.to_string_lossy().as_ref(),
        "private-authorized-copy",
    ] {
        assert!(!serialized.contains(forbidden));
    }
    let counters = &status["result"]["structuredContent"]["auditCounters"];
    assert_eq!(counters["by_tool"]["copy_file"]["allowed"], 2);
    assert_eq!(
        counters["by_reason_code"]["safe_root_file_copied"]["allowed"],
        1
    );
    assert_eq!(
        counters["by_reason_code"]["capability_grant_replayed"]["denied"],
        1
    );
}

#[tokio::test]
async fn malformed_signature_session_principal_path_root_and_family_bindings_are_private() {
    let first_root = tempfile::tempdir().unwrap();
    let second_root = tempfile::tempdir().unwrap();
    let source = first_root.path().join("private-source.txt");
    let other_source = first_root.path().join("private-other-source.txt");
    let destination = first_root.path().join("private-destination.txt");
    let other_destination = first_root.path().join("private-other-destination.txt");
    let other_root_destination = second_root.path().join("private-destination.txt");
    std::fs::write(&source, "same-size-private-a").unwrap();
    std::fs::write(&other_source, "same-size-private-b").unwrap();
    let file_tools = termux_mcp_server::tools::FileSystemTools::try_new(vec![
        first_root.path().to_path_buf(),
        second_root.path().to_path_buf(),
    ])
    .unwrap();
    let issuer_tools = file_tools.clone();
    let (router, authority) = copy_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let other_session_id = initialize_session(&router).await;
    let exact_target = issuer_tools
        .copy_file_grant_target(
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
        )
        .unwrap();

    let malformed = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        copy_call(
            "malformed",
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
            Some(false),
        ),
        "not-a-grant",
    )
    .await;
    assert_eq!(malformed.status(), StatusCode::FORBIDDEN);
    assert_capability_denial(
        &response_json(malformed).await,
        "capability_grant_malformed",
    );

    let valid = authority.issue(&session_id, &exact_target).unwrap();
    let invalid_signature = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        copy_call(
            "signature",
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
            Some(false),
        ),
        &corrupt_signature(&valid),
    )
    .await;
    assert_eq!(invalid_signature.status(), StatusCode::FORBIDDEN);
    assert_capability_denial(
        &response_json(invalid_signature).await,
        "capability_grant_signature_invalid",
    );

    let wrong_session_grant = authority.issue(&session_id, &exact_target).unwrap();
    let wrong_session = post_json_to_session_with_grant(
        router.clone(),
        &other_session_id,
        copy_call(
            "session",
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
            Some(false),
        ),
        &wrong_session_grant,
    )
    .await;
    assert_eq!(wrong_session.status(), StatusCode::FORBIDDEN);
    assert_capability_denial(
        &response_json(wrong_session).await,
        "capability_grant_binding_mismatch",
    );

    let other_principal = CopyFileGrantAuthority::from_hex_key(
        "test-copy-key-1",
        TEST_CAPABILITY_KEY,
        "private-other-copy-principal",
    )
    .unwrap();
    let other_principal_grant = other_principal.issue(&session_id, &exact_target).unwrap();
    let wrong_principal = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        copy_call(
            "principal",
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
            Some(false),
        ),
        &other_principal_grant,
    )
    .await;
    assert_eq!(wrong_principal.status(), StatusCode::FORBIDDEN);
    assert_capability_denial(
        &response_json(wrong_principal).await,
        "capability_grant_binding_mismatch",
    );

    for (label, requested_source, requested_destination) in [
        ("source", other_source.as_path(), destination.as_path()),
        ("destination", source.as_path(), other_destination.as_path()),
        ("root", source.as_path(), other_root_destination.as_path()),
    ] {
        let grant = authority.issue(&session_id, &exact_target).unwrap();
        let denied = post_json_to_session_with_grant(
            router.clone(),
            &session_id,
            copy_call(
                label,
                requested_source.to_string_lossy().as_ref(),
                requested_destination.to_string_lossy().as_ref(),
                Some(false),
            ),
            &grant,
        )
        .await;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN, "{label}");
        assert_capability_denial(
            &response_json(denied).await,
            "capability_grant_binding_mismatch",
        );
        authority
            .consume(Some(&grant), &session_id, &exact_target)
            .expect("a mismatched request must not consume the exact grant");
    }

    let write_authority = WriteFileGrantAuthority::from_hex_key(
        "test-copy-key-1",
        TEST_CAPABILITY_KEY,
        TEST_STATIC_PRINCIPAL,
    )
    .unwrap();
    let write_target = issuer_tools
        .write_file_grant_target(
            other_destination.to_string_lossy().as_ref(),
            b"private-family-content",
            WriteFileDisposition::Create,
        )
        .unwrap();
    let wrong_family_grant = write_authority.issue(&session_id, &write_target).unwrap();
    let wrong_family = post_json_to_session_with_grant(
        router,
        &session_id,
        copy_call(
            "family",
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
            Some(false),
        ),
        &wrong_family_grant,
    )
    .await;
    assert_eq!(wrong_family.status(), StatusCode::FORBIDDEN);
    let wrong_family = response_json(wrong_family).await;
    assert_capability_denial(&wrong_family, "capability_grant_binding_mismatch");

    let reflected = wrong_family.to_string();
    for private in [
        TEST_CAPABILITY_KEY,
        TEST_STATIC_PRINCIPAL,
        "private-other-copy-principal",
        "private-family-content",
        source.to_string_lossy().as_ref(),
        destination.to_string_lossy().as_ref(),
        wrong_family_grant.as_str(),
    ] {
        assert!(!reflected.contains(private));
    }
    assert!(!destination.exists());
}

#[tokio::test]
async fn stale_source_and_destination_fail_before_consumption() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let source = root.path().join("source.txt");
    let source_parked = root.path().join("source-original.txt");
    let destination = root.path().join("destination.txt");
    std::fs::write(&source, "source-original").unwrap();
    let (router, authority) = copy_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let source_target = issuer_tools
        .copy_file_grant_target(
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
        )
        .unwrap();
    let source_grant = authority.issue(&session_id, &source_target).unwrap();
    std::fs::rename(&source, &source_parked).unwrap();
    std::fs::write(&source, "source-substitute").unwrap();
    let stale_source = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        copy_call(
            "stale-source",
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
            Some(false),
        ),
        &source_grant,
    )
    .await;
    assert_eq!(stale_source.status(), StatusCode::FORBIDDEN);
    assert_capability_denial(
        &response_json(stale_source).await,
        "capability_grant_binding_mismatch",
    );
    authority
        .consume(Some(&source_grant), &session_id, &source_target)
        .expect("stale source rejection must precede grant consumption");
    assert!(!destination.exists());

    std::fs::remove_file(&source).unwrap();
    std::fs::rename(&source_parked, &source).unwrap();
    let destination_target = issuer_tools
        .copy_file_grant_target(
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
        )
        .unwrap();
    let destination_grant = authority.issue(&session_id, &destination_target).unwrap();
    std::fs::write(&destination, "destination-racer").unwrap();
    let stale_destination = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        copy_call(
            "stale-destination",
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
            Some(false),
        ),
        &destination_grant,
    )
    .await;
    assert_eq!(stale_destination.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        std::fs::read_to_string(&destination).unwrap(),
        "destination-racer"
    );

    std::fs::remove_file(&destination).unwrap();
    let retry = post_json_to_session_with_grant(
        router,
        &session_id,
        copy_call(
            "destination-retry",
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
            Some(false),
        ),
        &destination_grant,
    )
    .await;
    assert_eq!(retry.status(), StatusCode::OK);
    assert_eq!(
        std::fs::read_to_string(&destination).unwrap(),
        "source-original"
    );
}

#[tokio::test]
async fn capability_header_rejects_wrong_context_duplicate_oversized_non_ascii_and_argument_smuggling(
) {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let source = root.path().join("source.txt");
    let destination = root.path().join("destination.txt");
    std::fs::write(&source, "header-context-content").unwrap();
    let (router, authority) = copy_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let grant = issue_copy_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        source.to_string_lossy().as_ref(),
        destination.to_string_lossy().as_ref(),
    );

    let wrong_tool = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        runtime_status_call("wrong-tool"),
        &grant,
    )
    .await;
    assert_eq!(wrong_tool.status(), StatusCode::BAD_REQUEST);

    let wrong_method = Request::builder()
        .method(Method::GET)
        .uri("/mcp")
        .header(header::HOST, "localhost:8000")
        .header(header::ORIGIN, "http://localhost:8000")
        .header(header::ACCEPT, "text/event-stream")
        .header(MCP_PROTOCOL_VERSION_HEADER, MCP_PROTOCOL_VERSION)
        .header(MCP_SESSION_ID_HEADER, &session_id)
        .header(COPY_FILE_GRANT_HEADER, &grant)
        .body(Body::empty())
        .unwrap();
    let wrong_method = router.clone().oneshot(wrong_method).await.unwrap();
    assert_eq!(wrong_method.status(), StatusCode::BAD_REQUEST);

    let smuggled = post_json_to_session(
        router.clone(),
        &session_id,
        json!({
            "jsonrpc":"2.0",
            "id":"argument-smuggling",
            "method":"tools/call",
            "params":{
                "name":"copy_file",
                "arguments":{
                    "source_path":source.to_string_lossy(),
                    "destination_path":destination.to_string_lossy(),
                    "dry_run":false,
                    "capability_grant":grant,
                }
            }
        }),
    )
    .await;
    assert_eq!(smuggled.status(), StatusCode::BAD_REQUEST);
    assert!(!response_json(smuggled).await.to_string().contains(&grant));

    let mut duplicate = session_request(
        copy_call(
            "duplicate",
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
            Some(false),
        ),
        &session_id,
    );
    duplicate.headers_mut().append(
        COPY_FILE_GRANT_HEADER,
        HeaderValue::from_str(&grant).unwrap(),
    );
    duplicate.headers_mut().append(
        COPY_FILE_GRANT_HEADER,
        HeaderValue::from_str(&grant).unwrap(),
    );
    let duplicate = router.clone().oneshot(duplicate).await.unwrap();
    assert_eq!(duplicate.status(), StatusCode::BAD_REQUEST);

    for (label, value) in [
        (
            "oversized",
            HeaderValue::from_str(&"a".repeat(MAX_COPY_FILE_GRANT_HEADER_BYTES + 1)).unwrap(),
        ),
        (
            "non-ascii",
            HeaderValue::from_bytes(&[0xff, 0xfe]).expect("opaque header bytes are representable"),
        ),
    ] {
        let mut request = session_request(
            copy_call(
                label,
                source.to_string_lossy().as_ref(),
                destination.to_string_lossy().as_ref(),
                Some(false),
            ),
            &session_id,
        );
        request.headers_mut().insert(COPY_FILE_GRANT_HEADER, value);
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{label}");
        let body = response_json(response).await;
        assert_eq!(body["error"], "invalid_capability_grant_header");
        assert!(!body.to_string().contains(&grant));
    }
    assert!(!destination.exists());

    let allowed = post_json_to_session_with_grant(
        router,
        &session_id,
        copy_call(
            "allowed",
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
            Some(false),
        ),
        &grant,
    )
    .await;
    assert_eq!(allowed.status(), StatusCode::OK);
    assert_eq!(
        std::fs::read_to_string(destination).unwrap(),
        "header-context-content"
    );
}

#[tokio::test]
async fn replay_is_shared_by_independently_constructed_router_authorities() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let source = root.path().join("source.txt");
    let destination = root.path().join("destination.txt");
    std::fs::write(&source, "shared-replay-content").unwrap();
    let (router, router_authority) = copy_file_authorized_test_router(file_tools);
    let independent_authority = CopyFileGrantAuthority::from_hex_key(
        "test-copy-key-1",
        TEST_CAPABILITY_KEY,
        TEST_STATIC_PRINCIPAL,
    )
    .unwrap();
    let session_id = initialize_session(&router).await;
    let target = issuer_tools
        .copy_file_grant_target(
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
        )
        .unwrap();
    let grant = independent_authority.issue(&session_id, &target).unwrap();
    independent_authority
        .consume(Some(&grant), &session_id, &target)
        .unwrap();

    let replay = post_json_to_session_with_grant(
        router,
        &session_id,
        copy_call(
            "router-replay",
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
            Some(false),
        ),
        &grant,
    )
    .await;
    assert_eq!(replay.status(), StatusCode::FORBIDDEN);
    assert_capability_denial(&response_json(replay).await, "capability_grant_replayed");
    assert!(!destination.exists());
    assert_eq!(
        router_authority
            .consume(Some(&grant), &session_id, &target)
            .unwrap_err()
            .reason_code(),
        "capability_grant_replayed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_same_destination_has_exactly_one_success() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let source = root.path().join("source.txt");
    let destination = root.path().join("destination.txt");
    std::fs::write(&source, "concurrent-copy-content").unwrap();
    let (router, authority) = copy_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let target = issuer_tools
        .copy_file_grant_target(
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
        )
        .unwrap();
    let grant = Arc::new(authority.issue(&session_id, &target).unwrap());
    let barrier = Arc::new(tokio::sync::Barrier::new(9));
    let mut calls = tokio::task::JoinSet::new();
    for index in 0..8 {
        let router = router.clone();
        let session_id = session_id.clone();
        let source = source.clone();
        let destination = destination.clone();
        let grant = Arc::clone(&grant);
        let barrier = Arc::clone(&barrier);
        calls.spawn(async move {
            barrier.wait().await;
            let response = post_json_to_session_with_grant(
                router,
                &session_id,
                copy_call(
                    format!("concurrent-{index}"),
                    source.to_string_lossy().as_ref(),
                    destination.to_string_lossy().as_ref(),
                    Some(false),
                ),
                &grant,
            )
            .await;
            (response.status(), response_json(response).await)
        });
    }
    barrier.wait().await;
    let mut successes = 0;
    while let Some(result) = calls.join_next().await {
        let (status, body) = result.unwrap();
        match status {
            StatusCode::OK => successes += 1,
            StatusCode::BAD_REQUEST | StatusCode::FORBIDDEN | StatusCode::SERVICE_UNAVAILABLE => {
                assert!(body.get("error").is_some());
            }
            other => panic!("unexpected concurrent status {other}: {body}"),
        }
    }
    assert_eq!(successes, 1);
    assert_eq!(
        std::fs::read_to_string(&destination).unwrap(),
        "concurrent-copy-content"
    );
    assert_eq!(
        authority
            .consume(Some(grant.as_str()), &session_id, &target)
            .unwrap_err()
            .reason_code(),
        "capability_grant_replayed"
    );
}

#[tokio::test]
async fn exact_one_mib_copy_succeeds_and_plus_one_preflight_preserves_another_grant() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let exact_source = root.path().join("exact.bin");
    let exact_destination = root.path().join("exact-copy.bin");
    std::fs::write(&exact_source, vec![0x5a; MAX_COPY_FILE_BYTES]).unwrap();
    let small_source = root.path().join("small.bin");
    let small_destination = root.path().join("small-copy.bin");
    std::fs::write(&small_source, b"small-copy").unwrap();
    let oversized_source = root.path().join("oversized.bin");
    std::fs::write(&oversized_source, vec![0x7a; MAX_COPY_FILE_BYTES + 1]).unwrap();
    let (router, authority) = copy_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let exact_grant = issue_copy_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        exact_source.to_string_lossy().as_ref(),
        exact_destination.to_string_lossy().as_ref(),
    );
    let exact = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        copy_call(
            "exact",
            exact_source.to_string_lossy().as_ref(),
            exact_destination.to_string_lossy().as_ref(),
            Some(false),
        ),
        &exact_grant,
    )
    .await;
    assert_eq!(exact.status(), StatusCode::OK);
    assert_path_free_copy_result(&response_json(exact).await, false, MAX_COPY_FILE_BYTES);
    assert_eq!(
        std::fs::metadata(&exact_destination).unwrap().len(),
        1_048_576
    );

    let small_grant = issue_copy_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        small_source.to_string_lossy().as_ref(),
        small_destination.to_string_lossy().as_ref(),
    );
    let plus_one = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        copy_call(
            "plus-one",
            oversized_source.to_string_lossy().as_ref(),
            small_destination.to_string_lossy().as_ref(),
            Some(false),
        ),
        &small_grant,
    )
    .await;
    assert_eq!(plus_one.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(!small_destination.exists());

    let retry = post_json_to_session_with_grant(
        router,
        &session_id,
        copy_call(
            "small-retry",
            small_source.to_string_lossy().as_ref(),
            small_destination.to_string_lossy().as_ref(),
            Some(false),
        ),
        &small_grant,
    )
    .await;
    assert_eq!(retry.status(), StatusCode::OK);
    assert_eq!(std::fs::read(&small_destination).unwrap(), b"small-copy");
}

#[test]
fn public_wire_contract_constants_remain_bounded() {
    assert_eq!(COPY_FILE_GRANT_HEADER, "mcp-capability-grant");
    assert_eq!(MAX_COPY_FILE_GRANT_HEADER_BYTES, 384);
    assert_eq!(COPY_FILE_GRANT_TTL_SECONDS, 60);
    assert!(MAX_COPY_FILE_RESPONSE_BYTES < MAX_MCP_JSON_RPC_ID_BYTES);
    assert_eq!(MCP_POST_ACCEPT, "application/json, text/event-stream");
}

#[allow(dead_code)]
fn _request_shape_compile_guard(session_id: &str, grant: &str) -> Request<Body> {
    Request::post("/mcp")
        .header(header::HOST, "localhost:8000")
        .header(header::ORIGIN, "http://localhost:8000")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, MCP_POST_ACCEPT)
        .header(MCP_PROTOCOL_VERSION_HEADER, MCP_PROTOCOL_VERSION)
        .header(MCP_SESSION_ID_HEADER, session_id)
        .header(COPY_FILE_GRANT_HEADER, grant)
        .body(Body::empty())
        .unwrap()
}
