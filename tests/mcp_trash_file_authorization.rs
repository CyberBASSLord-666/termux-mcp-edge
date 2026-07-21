#![cfg(feature = "mcp-runtime")]

mod support;

use std::{
    os::unix::fs::{MetadataExt, PermissionsExt},
    sync::Arc,
};

use axum::{
    body::{to_bytes, Body},
    http::{header, HeaderValue, Method, Request, StatusCode},
};
use serde_json::{json, Value};
use support::{
    create_fifo, empty_test_file_tools, initialize_session, issue_trash_file_grant,
    post_json_to_session, post_json_to_session_with_grant, response_json, session_request,
    trash_file_authorized_test_router, TEST_CAPABILITY_KEY, TEST_STATIC_PRINCIPAL,
};
use termux_mcp_server::{
    mcp_transport::{
        MAX_MCP_JSON_RPC_ID_BYTES, MCP_POST_ACCEPT, MCP_PROTOCOL_VERSION,
        MCP_PROTOCOL_VERSION_HEADER, MCP_SESSION_ID_HEADER,
    },
    tools::{
        FileSystemTools, MAX_TRASH_FILE_BYTES, MAX_TRASH_FILE_QUARANTINE_ARTIFACTS,
        MAX_TRASH_FILE_QUARANTINE_BYTES, MAX_TRASH_FILE_RESPONSE_BYTES,
    },
    trash_file_grant::{
        TrashFileGrantAuthority, TrashFileGrantError, MAX_TRASH_FILE_GRANT_HEADER_BYTES,
        TRASH_FILE_GRANT_HEADER, TRASH_FILE_GRANT_TTL_SECONDS,
    },
    write_file_grant::{WriteFileDisposition, WriteFileGrantAuthority},
};
use tower::ServiceExt;

const TRASH_QUARANTINE: &str = ".termux-mcp-trash-quarantine";
const TRASH_ARTIFACT_PREFIX: &str = ".termux-mcp-trash-artifact-";
const DRY_RUN_SUMMARY: &str =
    "Validated one bounded safe-rooted file for reversible trashing without mutation.";
const MUTATION_SUMMARY: &str =
    "Moved one bounded safe-rooted file into the private recovery quarantine.";

fn trash_call(id: impl Into<Value>, path: &str, dry_run: Option<bool>) -> Value {
    let mut arguments = json!({"path": path});
    if let Some(dry_run) = dry_run {
        arguments["dry_run"] = json!(dry_run);
    }
    json!({
        "jsonrpc": "2.0",
        "id": id.into(),
        "method": "tools/call",
        "params": {"name": "trash_file", "arguments": arguments},
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

fn assert_path_free_trash_result(body: &Value, dry_run: bool, size: usize) {
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
            "recoveryArtifactRetained": !dry_run,
            "maxFileBytes": MAX_TRASH_FILE_BYTES,
            "maxResponseBytes": MAX_TRASH_FILE_RESPONSE_BYTES,
        })
    );
    let serialized = result.to_string();
    for forbidden in [
        "path",
        "artifactName",
        "sha256",
        TRASH_QUARANTINE,
        TRASH_ARTIFACT_PREFIX,
    ] {
        assert!(!serialized.contains(forbidden));
    }
}

#[tokio::test]
async fn enabled_discovery_status_and_missing_grant_are_exact_and_fail_closed() {
    let (_root, file_tools) = empty_test_file_tools();
    let (router, _authority) = trash_file_authorized_test_router(file_tools);
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
    let trash = discovery["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "trash_file")
        .unwrap();
    assert_eq!(trash["inputSchema"]["type"], "object");
    assert_eq!(trash["inputSchema"]["required"], json!(["path"]));
    assert_eq!(trash["inputSchema"]["additionalProperties"], false);
    assert_eq!(
        trash["inputSchema"]["properties"]
            .as_object()
            .unwrap()
            .len(),
        2
    );
    assert!(trash["inputSchema"]["properties"]["dry_run"]
        .get("const")
        .is_none());
    assert!(trash["description"]
        .as_str()
        .unwrap()
        .contains("identity/content-bound"));

    let outside = tempfile::tempdir().unwrap();
    let denied = post_json_to_session(
        router.clone(),
        &session_id,
        trash_call(
            "missing",
            outside
                .path()
                .join("private-missing")
                .to_string_lossy()
                .as_ref(),
            Some(false),
        ),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert_capability_denial(&response_json(denied).await, "capability_grant_missing");

    let status = response_json(
        post_json_to_session(router, &session_id, runtime_status_call("status")).await,
    )
    .await;
    let runtime = &status["result"]["structuredContent"];
    assert_eq!(runtime["trashFileMutationEnabled"], true);
    assert_eq!(runtime["trashFileGrantRequired"], true);
    assert_eq!(runtime["trashFileGrantHeader"], TRASH_FILE_GRANT_HEADER);
    assert_eq!(
        runtime["trashFileGrantTtlSeconds"],
        TRASH_FILE_GRANT_TTL_SECONDS
    );
    assert_eq!(
        runtime["trashFileMode"],
        "dry_run_or_identity_content_scoped_single_use_grant_with_recovery_retained"
    );
    assert_eq!(
        runtime["trashFileGrantBinding"],
        "root_path_single_link_identity_size_ctime_sha256_recovery_retained"
    );
    assert_eq!(runtime["trashFileMaxBytes"], MAX_TRASH_FILE_BYTES);
    assert_eq!(
        runtime["trashFileMaxResponseBytes"],
        MAX_TRASH_FILE_RESPONSE_BYTES
    );
    assert_eq!(
        runtime["trashFileQuarantineMaxArtifacts"],
        MAX_TRASH_FILE_QUARANTINE_ARTIFACTS
    );
    assert_eq!(
        runtime["trashFileQuarantineMaxBytes"],
        MAX_TRASH_FILE_QUARANTINE_BYTES
    );
    assert_eq!(
        runtime["trashFileResponsePosture"],
        "path_and_artifact_free_bounded_metadata_only"
    );
}

#[tokio::test]
async fn exact_grant_survives_preview_and_preflight_then_retains_one_private_artifact() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let target = root.path().join("private-target.bin");
    let content = b"private-authorized-trash\0\xff";
    std::fs::write(&target, content).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
    let target_before = std::fs::metadata(&target).unwrap();
    let (router, authority) = trash_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let grant = issue_trash_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        target.to_string_lossy().as_ref(),
    );

    let smuggled_preview = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        trash_call(
            "smuggled-preview",
            target.to_string_lossy().as_ref(),
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
    assert!(target.exists());

    let preview = post_json_to_session(
        router.clone(),
        &session_id,
        trash_call("preview", target.to_string_lossy().as_ref(), None),
    )
    .await;
    assert_eq!(preview.status(), StatusCode::OK);
    assert_path_free_trash_result(&response_json(preview).await, true, content.len());
    assert!(target.exists());
    assert!(!root.path().join(TRASH_QUARANTINE).exists());

    let oversized_id = "x".repeat(MAX_TRASH_FILE_RESPONSE_BYTES);
    assert!(oversized_id.len() < MAX_MCP_JSON_RPC_ID_BYTES);
    let preflight = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        trash_call(oversized_id, target.to_string_lossy().as_ref(), Some(false)),
        &grant,
    )
    .await;
    assert_eq!(preflight.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = to_bytes(preflight.into_body(), MAX_TRASH_FILE_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(body.len() <= MAX_TRASH_FILE_RESPONSE_BYTES);
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["id"], Value::Null);
    assert!(target.exists());
    assert!(!root.path().join(TRASH_QUARANTINE).exists());

    let trashed = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        trash_call("trash", target.to_string_lossy().as_ref(), Some(false)),
        &grant,
    )
    .await;
    assert_eq!(trashed.status(), StatusCode::OK);
    assert_path_free_trash_result(&response_json(trashed).await, false, content.len());
    assert!(!target.exists());

    let quarantine = root.path().join(TRASH_QUARANTINE);
    let quarantine_metadata = std::fs::symlink_metadata(&quarantine).unwrap();
    assert!(quarantine_metadata.is_dir());
    assert_eq!(quarantine_metadata.permissions().mode() & 0o777, 0o700);
    let artifacts = std::fs::read_dir(&quarantine)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(artifacts.len(), 1);
    let artifact = artifacts[0].path();
    let name = artifact.file_name().unwrap().to_string_lossy();
    let uuid = name.strip_prefix(TRASH_ARTIFACT_PREFIX).unwrap();
    assert_eq!(
        uuid::Uuid::parse_str(uuid)
            .unwrap()
            .hyphenated()
            .to_string(),
        uuid
    );
    assert_eq!(std::fs::read(&artifact).unwrap(), content);
    let retained = std::fs::metadata(&artifact).unwrap();
    assert_eq!(retained.dev(), target_before.dev());
    assert_eq!(retained.ino(), target_before.ino());
    assert_eq!(retained.nlink(), 1);
    assert_eq!(retained.permissions().mode() & 0o777, 0o640);

    let replay = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        trash_call("replay", target.to_string_lossy().as_ref(), Some(false)),
        &grant,
    )
    .await;
    assert_eq!(replay.status(), StatusCode::BAD_REQUEST);
    assert!(!response_json(replay).await.to_string().contains(&grant));

    let status = response_json(
        post_json_to_session(router, &session_id, runtime_status_call("audit")).await,
    )
    .await;
    let serialized = status.to_string();
    for forbidden in [
        TEST_CAPABILITY_KEY,
        TEST_STATIC_PRINCIPAL,
        grant.as_str(),
        target.to_string_lossy().as_ref(),
        artifact.to_string_lossy().as_ref(),
        "private-authorized-trash",
    ] {
        assert!(!serialized.contains(forbidden));
    }
    let counters = &status["result"]["structuredContent"]["auditCounters"];
    assert_eq!(counters["by_tool"]["trash_file"]["allowed"], 2);
    assert_eq!(
        counters["by_reason_code"]["safe_root_file_trashed_recovery_retained"]["allowed"],
        1
    );
}

#[tokio::test]
async fn malformed_signature_session_principal_path_root_content_and_family_bindings_are_private() {
    let first_root = tempfile::tempdir().unwrap();
    let second_root = tempfile::tempdir().unwrap();
    let target = first_root.path().join("exact.bin");
    let other_path = first_root.path().join("other.bin");
    let other_root_path = second_root.path().join("other-root.bin");
    let write_target = first_root.path().join("write-family-target.bin");
    std::fs::write(&target, "exact-private-content").unwrap();
    std::fs::write(&other_path, "other-private-content").unwrap();
    std::fs::write(&other_root_path, "other-root-private-content").unwrap();
    let file_tools = FileSystemTools::try_new(vec![
        first_root.path().to_path_buf(),
        second_root.path().to_path_buf(),
    ])
    .unwrap();
    let issuer_tools = file_tools.clone();
    let (router, authority) = trash_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let other_session_id = initialize_session(&router).await;
    let exact_target = issuer_tools
        .trash_file_grant_target(target.to_string_lossy().as_ref())
        .unwrap();
    let valid = authority.issue(&session_id, &exact_target).unwrap();

    for (label, grant, expected_reason) in [
        (
            "malformed",
            "v1.too.short".to_owned(),
            "capability_grant_malformed",
        ),
        (
            "signature",
            corrupt_signature(&valid),
            "capability_grant_signature_invalid",
        ),
    ] {
        let denied = post_json_to_session_with_grant(
            router.clone(),
            &session_id,
            trash_call(label, target.to_string_lossy().as_ref(), Some(false)),
            &grant,
        )
        .await;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN, "{label}");
        assert_capability_denial(&response_json(denied).await, expected_reason);
    }
    assert!(!first_root.path().join(TRASH_QUARANTINE).exists());

    let wrong_session_grant = authority.issue(&session_id, &exact_target).unwrap();
    let wrong_session = post_json_to_session_with_grant(
        router.clone(),
        &other_session_id,
        trash_call("session", target.to_string_lossy().as_ref(), Some(false)),
        &wrong_session_grant,
    )
    .await;
    assert_eq!(wrong_session.status(), StatusCode::FORBIDDEN);
    assert_capability_denial(
        &response_json(wrong_session).await,
        "capability_grant_binding_mismatch",
    );

    let other_principal = TrashFileGrantAuthority::from_hex_key(
        "test-trash-key-1",
        TEST_CAPABILITY_KEY,
        "private-other-trash-principal",
    )
    .unwrap();
    let other_principal_grant = other_principal.issue(&session_id, &exact_target).unwrap();
    let wrong_principal = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        trash_call("principal", target.to_string_lossy().as_ref(), Some(false)),
        &other_principal_grant,
    )
    .await;
    assert_eq!(wrong_principal.status(), StatusCode::FORBIDDEN);
    assert_capability_denial(
        &response_json(wrong_principal).await,
        "capability_grant_binding_mismatch",
    );

    for (label, requested) in [("path", &other_path), ("root", &other_root_path)] {
        let grant = authority.issue(&session_id, &exact_target).unwrap();
        let denied = post_json_to_session_with_grant(
            router.clone(),
            &session_id,
            trash_call(label, requested.to_string_lossy().as_ref(), Some(false)),
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
            .expect("binding mismatch must not consume the exact grant");
    }

    let stale_grant = authority.issue(&session_id, &exact_target).unwrap();
    let parked = first_root.path().join("parked-original.bin");
    std::fs::rename(&target, &parked).unwrap();
    std::fs::write(&target, "changed-private-content").unwrap();
    let stale = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        trash_call(
            "content-identity",
            target.to_string_lossy().as_ref(),
            Some(false),
        ),
        &stale_grant,
    )
    .await;
    assert_eq!(stale.status(), StatusCode::FORBIDDEN);
    assert_capability_denial(
        &response_json(stale).await,
        "capability_grant_binding_mismatch",
    );
    authority
        .consume(Some(&stale_grant), &session_id, &exact_target)
        .expect("stale target must be rejected before grant consumption");
    assert!(!first_root.path().join(TRASH_QUARANTINE).exists());

    let write_authority = WriteFileGrantAuthority::from_hex_key(
        "test-trash-key-1",
        TEST_CAPABILITY_KEY,
        TEST_STATIC_PRINCIPAL,
    )
    .unwrap();
    let write_binding = issuer_tools
        .write_file_grant_target(
            write_target.to_string_lossy().as_ref(),
            b"private-write-family-content",
            WriteFileDisposition::Create,
        )
        .unwrap();
    let wrong_family_grant = write_authority.issue(&session_id, &write_binding).unwrap();
    std::fs::remove_file(&target).unwrap();
    std::fs::rename(&parked, &target).unwrap();
    let wrong_family = post_json_to_session_with_grant(
        router,
        &session_id,
        trash_call("family", target.to_string_lossy().as_ref(), Some(false)),
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
        "private-other-trash-principal",
        "private-write-family-content",
        target.to_string_lossy().as_ref(),
        wrong_family_grant.as_str(),
    ] {
        assert!(!reflected.contains(private));
    }
    assert!(target.exists());
    assert!(!first_root.path().join(TRASH_QUARANTINE).exists());
}

#[tokio::test]
async fn exact_limit_succeeds_and_plus_one_or_unsupported_targets_preserve_other_grants() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let exact = root.path().join("exact.bin");
    let oversized = root.path().join("oversized.bin");
    let small = root.path().join("small.bin");
    let linked = root.path().join("linked.bin");
    let linked_alias = root.path().join("linked-alias.bin");
    let directory = root.path().join("directory");
    let fifo = root.path().join("fifo");
    let symlink_path = root.path().join("symlink.bin");
    let missing = root.path().join("missing.bin");
    std::fs::write(&exact, vec![0x5a; MAX_TRASH_FILE_BYTES]).unwrap();
    std::fs::write(&oversized, vec![0x6a; MAX_TRASH_FILE_BYTES + 1]).unwrap();
    std::fs::write(&small, b"small-trash").unwrap();
    std::fs::write(&linked, b"linked-content").unwrap();
    std::fs::hard_link(&linked, &linked_alias).unwrap();
    std::fs::create_dir(&directory).unwrap();
    create_fifo(&fifo);
    std::os::unix::fs::symlink(&small, &symlink_path).unwrap();
    let (router, authority) = trash_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let exact_grant = issue_trash_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        exact.to_string_lossy().as_ref(),
    );
    let exact_response = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        trash_call("exact", exact.to_string_lossy().as_ref(), Some(false)),
        &exact_grant,
    )
    .await;
    assert_eq!(exact_response.status(), StatusCode::OK);
    assert_path_free_trash_result(
        &response_json(exact_response).await,
        false,
        MAX_TRASH_FILE_BYTES,
    );
    assert!(!exact.exists());

    let small_target = issuer_tools
        .trash_file_grant_target(small.to_string_lossy().as_ref())
        .unwrap();
    let reusable = authority.issue(&session_id, &small_target).unwrap();
    let cases = [
        ("oversized", &oversized, StatusCode::PAYLOAD_TOO_LARGE),
        ("hardlink", &linked, StatusCode::BAD_REQUEST),
        ("directory", &directory, StatusCode::BAD_REQUEST),
        ("fifo", &fifo, StatusCode::BAD_REQUEST),
        ("symlink", &symlink_path, StatusCode::BAD_REQUEST),
        ("missing", &missing, StatusCode::BAD_REQUEST),
    ];
    for (label, path, expected) in cases {
        let response = post_json_to_session_with_grant(
            router.clone(),
            &session_id,
            trash_call(label, path.to_string_lossy().as_ref(), Some(false)),
            &reusable,
        )
        .await;
        assert_eq!(response.status(), expected, "{label}");
        let body = response_json(response).await.to_string();
        for private in [path.to_string_lossy().as_ref(), reusable.as_str()] {
            assert!(!body.contains(private), "{label}");
        }
    }
    assert!(small.exists());
    let retry = post_json_to_session_with_grant(
        router,
        &session_id,
        trash_call("retry", small.to_string_lossy().as_ref(), Some(false)),
        &reusable,
    )
    .await;
    assert_eq!(retry.status(), StatusCode::OK);
    assert!(!small.exists());
    assert_eq!(std::fs::read_to_string(&linked).unwrap(), "linked-content");
    assert_eq!(
        std::fs::read_to_string(&linked_alias).unwrap(),
        "linked-content"
    );
}

#[tokio::test]
async fn capability_header_rejects_wrong_context_duplicate_oversized_non_ascii_and_arguments() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let target = root.path().join("target.bin");
    std::fs::write(&target, "header-context-content").unwrap();
    let (router, authority) = trash_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let grant = issue_trash_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        target.to_string_lossy().as_ref(),
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
        .header(
            header::AUTHORIZATION,
            format!("Bearer {TEST_STATIC_PRINCIPAL}"),
        )
        .header(MCP_PROTOCOL_VERSION_HEADER, MCP_PROTOCOL_VERSION)
        .header(MCP_SESSION_ID_HEADER, &session_id)
        .header(TRASH_FILE_GRANT_HEADER, &grant)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        router.clone().oneshot(wrong_method).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );

    let smuggled = post_json_to_session(
        router.clone(),
        &session_id,
        json!({
            "jsonrpc":"2.0",
            "id":"argument-smuggling",
            "method":"tools/call",
            "params":{
                "name":"trash_file",
                "arguments":{
                    "path":target.to_string_lossy(),
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
        trash_call("duplicate", target.to_string_lossy().as_ref(), Some(false)),
        &session_id,
    );
    duplicate.headers_mut().append(
        TRASH_FILE_GRANT_HEADER,
        HeaderValue::from_str(&grant).unwrap(),
    );
    duplicate.headers_mut().append(
        TRASH_FILE_GRANT_HEADER,
        HeaderValue::from_str(&grant).unwrap(),
    );
    assert_eq!(
        router.clone().oneshot(duplicate).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );

    for (label, value) in [
        (
            "oversized",
            HeaderValue::from_str(&"a".repeat(MAX_TRASH_FILE_GRANT_HEADER_BYTES + 1)).unwrap(),
        ),
        (
            "non-ascii",
            HeaderValue::from_bytes(&[0xff, 0xfe]).expect("opaque bytes are representable"),
        ),
    ] {
        let mut request = session_request(
            trash_call(label, target.to_string_lossy().as_ref(), Some(false)),
            &session_id,
        );
        request.headers_mut().insert(TRASH_FILE_GRANT_HEADER, value);
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{label}");
        let body = response_json(response).await;
        assert_eq!(body["error"], "invalid_capability_grant_header");
        assert!(!body.to_string().contains(&grant));
    }
    assert!(target.exists());

    let allowed = post_json_to_session_with_grant(
        router,
        &session_id,
        trash_call("allowed", target.to_string_lossy().as_ref(), Some(false)),
        &grant,
    )
    .await;
    assert_eq!(allowed.status(), StatusCode::OK);
    assert!(!target.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_distinct_grants_racing_one_target_consume_exactly_one() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let target = root.path().join("raced.bin");
    std::fs::write(&target, "raced-private-content").unwrap();
    let (router, authority) = trash_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let binding = issuer_tools
        .trash_file_grant_target(target.to_string_lossy().as_ref())
        .unwrap();
    let grants = Arc::new([
        authority.issue(&session_id, &binding).unwrap(),
        authority.issue(&session_id, &binding).unwrap(),
    ]);
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let mut calls = tokio::task::JoinSet::new();
    for index in 0..2 {
        let router = router.clone();
        let session_id = session_id.clone();
        let target = target.clone();
        let grants = Arc::clone(&grants);
        let barrier = Arc::clone(&barrier);
        calls.spawn(async move {
            barrier.wait().await;
            let response = post_json_to_session_with_grant(
                router,
                &session_id,
                trash_call(
                    format!("race-{index}"),
                    target.to_string_lossy().as_ref(),
                    Some(false),
                ),
                &grants[index],
            )
            .await;
            (index, response.status(), response_json(response).await)
        });
    }
    barrier.wait().await;
    let mut winner = None;
    while let Some(result) = calls.join_next().await {
        let (index, status, body) = result.unwrap();
        match status {
            StatusCode::OK => winner = Some(index),
            StatusCode::BAD_REQUEST | StatusCode::CONFLICT | StatusCode::SERVICE_UNAVAILABLE => {
                assert!(body.get("error").is_some());
            }
            other => panic!("unexpected race status {other}: {body}"),
        }
    }
    let winner = winner.expect("one grant must win the exact target");
    let loser = 1 - winner;
    assert!(!target.exists());
    assert_eq!(
        std::fs::read_dir(root.path().join(TRASH_QUARANTINE))
            .unwrap()
            .count(),
        1
    );
    assert_eq!(
        authority
            .consume(Some(&grants[winner]), &session_id, &binding)
            .unwrap_err(),
        TrashFileGrantError::Replayed
    );
    authority
        .consume(Some(&grants[loser]), &session_id, &binding)
        .expect("the losing race must preserve its distinct grant");
}

#[tokio::test]
async fn replay_is_shared_by_independently_constructed_authorities() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let target = root.path().join("shared-replay.bin");
    std::fs::write(&target, "shared-replay-content").unwrap();
    let (router, router_authority) = trash_file_authorized_test_router(file_tools);
    let independent = TrashFileGrantAuthority::from_hex_key(
        "test-trash-key-1",
        TEST_CAPABILITY_KEY,
        TEST_STATIC_PRINCIPAL,
    )
    .unwrap();
    let session_id = initialize_session(&router).await;
    let binding = issuer_tools
        .trash_file_grant_target(target.to_string_lossy().as_ref())
        .unwrap();
    let grant = independent.issue(&session_id, &binding).unwrap();
    independent
        .consume(Some(&grant), &session_id, &binding)
        .unwrap();

    let replay = post_json_to_session_with_grant(
        router,
        &session_id,
        trash_call(
            "router-replay",
            target.to_string_lossy().as_ref(),
            Some(false),
        ),
        &grant,
    )
    .await;
    assert_eq!(replay.status(), StatusCode::FORBIDDEN);
    assert_capability_denial(&response_json(replay).await, "capability_grant_replayed");
    assert!(target.exists());
    assert_eq!(
        router_authority
            .consume(Some(&grant), &session_id, &binding)
            .unwrap_err(),
        TrashFileGrantError::Replayed
    );
}

#[test]
fn public_wire_contract_constants_remain_bounded() {
    assert_eq!(TRASH_FILE_GRANT_HEADER, "mcp-capability-grant");
    assert_eq!(MAX_TRASH_FILE_GRANT_HEADER_BYTES, 384);
    assert_eq!(TRASH_FILE_GRANT_TTL_SECONDS, 60);
    assert_eq!(MAX_TRASH_FILE_BYTES, 1_048_576);
    assert_eq!(MAX_TRASH_FILE_QUARANTINE_ARTIFACTS, 32);
    assert_eq!(MAX_TRASH_FILE_QUARANTINE_BYTES, 33_554_432);
    assert!(std::hint::black_box(MAX_TRASH_FILE_RESPONSE_BYTES) < MAX_MCP_JSON_RPC_ID_BYTES);
    assert_eq!(MCP_POST_ACCEPT, "application/json, text/event-stream");
}

#[allow(dead_code)]
fn _request_shape_compile_guard(session_id: &str, grant: &str) -> Request<Body> {
    Request::post("/mcp")
        .header(header::HOST, "localhost:8000")
        .header(header::ORIGIN, "http://localhost:8000")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, MCP_POST_ACCEPT)
        .header(
            header::AUTHORIZATION,
            format!("Bearer {TEST_STATIC_PRINCIPAL}"),
        )
        .header(MCP_PROTOCOL_VERSION_HEADER, MCP_PROTOCOL_VERSION)
        .header(MCP_SESSION_ID_HEADER, session_id)
        .header(TRASH_FILE_GRANT_HEADER, grant)
        .body(Body::empty())
        .unwrap()
}
