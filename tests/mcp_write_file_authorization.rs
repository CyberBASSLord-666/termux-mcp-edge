#![cfg(feature = "mcp-runtime")]

mod support;

use std::{
    os::unix::fs::PermissionsExt,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{body::to_bytes, http::StatusCode};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use support::{
    empty_test_file_tools, initialize_session, issue_write_file_grant, post_json_to_session,
    post_json_to_session_with_grant, response_json, test_router, write_file_authorized_test_router,
    TEST_CAPABILITY_KEY, TEST_STATIC_PRINCIPAL,
};
use termux_mcp_server::{
    tools::MAX_WRITE_FILE_RESPONSE_BYTES,
    write_file_grant::{
        WriteFileDisposition, WriteFileGrantAuthority, WRITE_FILE_GRANT_HEADER,
        WRITE_FILE_GRANT_TTL_SECONDS,
    },
    write_policy::DEFAULT_MAX_WRITE_BYTES,
};

type HmacSha256 = Hmac<Sha256>;

const DRY_RUN_SUMMARY: &str =
    "Validated one bounded safe-rooted UTF-8 file write without mutation.";
const MUTATION_SUMMARY: &str = "Wrote one bounded safe-rooted UTF-8 file with fixed mode 0600.";
const PAYLOAD_BYTES: usize = 65;
const ISSUED_OFFSET: usize = 49;
const EXPIRES_OFFSET: usize = 57;

fn write_call(id: impl Into<Value>, path: &str, content: &str, dry_run: bool) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.into(),
        "method": "tools/call",
        "params": {
            "name": "write_file",
            "arguments": {
                "path": path,
                "content": content,
                "dry_run": dry_run,
            },
        },
    })
}

fn runtime_status_call(id: impl Into<Value>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.into(),
        "method": "tools/call",
        "params": {"name": "runtime_status", "arguments": {}}
    })
}

fn corrupt_signature(token: &str) -> String {
    let mut corrupted = token.as_bytes().to_vec();
    let last = corrupted.last_mut().expect("grant is non-empty");
    *last = if *last == b'0' { b'1' } else { b'0' };
    String::from_utf8(corrupted).unwrap()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16).unwrap() as u8;
            let low = (pair[1] as char).to_digit(16).unwrap() as u8;
            (high << 4) | low
        })
        .collect()
}

fn encode_hex(value: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value {
        write!(&mut encoded, "{byte:02x}").unwrap();
    }
    encoded
}

fn resign_with_times(token: &str, issued: u64, expires: u64) -> String {
    let segments = token.split('.').collect::<Vec<_>>();
    assert_eq!(segments.len(), 4);
    let mut payload = decode_hex(segments[2]);
    assert_eq!(payload.len(), PAYLOAD_BYTES);
    payload[ISSUED_OFFSET..EXPIRES_OFFSET].copy_from_slice(&issued.to_be_bytes());
    payload[EXPIRES_OFFSET..PAYLOAD_BYTES].copy_from_slice(&expires.to_be_bytes());

    let payload = encode_hex(&payload);
    let signed = format!("{}.{}.{}", segments[0], segments[1], payload);
    let key = decode_hex(TEST_CAPABILITY_KEY);
    let mut mac = <HmacSha256 as KeyInit>::new_from_slice(&key).unwrap();
    mac.update(signed.as_bytes());
    format!("{signed}.{}", encode_hex(&mac.finalize().into_bytes()))
}

fn assert_capability_denial(body: &Value, reason: &str) {
    assert_eq!(body["error"]["code"], -32003);
    assert_eq!(body["error"]["data"]["reason"], reason);
}

fn assert_write_result(body: &Value, dry_run: bool, size: usize, disposition: &str) {
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
            "disposition": disposition,
            "mode": "0600",
            "maxFileBytes": DEFAULT_MAX_WRITE_BYTES,
            "maxResponseBytes": MAX_WRITE_FILE_RESPONSE_BYTES,
            "recoveryArtifactRetained": !dry_run && disposition == "replace",
        })
    );
}

#[tokio::test]
async fn disabled_gate_is_dry_run_only_and_denies_mutation_before_path_validation() {
    let (root, file_tools) = empty_test_file_tools();
    let target = root.path().join("disabled.txt");
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let discovery = post_json_to_session(
        router.clone(),
        &session_id,
        json!({"jsonrpc":"2.0","id":"tools","method":"tools/list"}),
    )
    .await;
    assert_eq!(discovery.status(), StatusCode::OK);
    let discovery = response_json(discovery).await;
    let write = discovery["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "write_file")
        .unwrap();
    assert_eq!(write["inputSchema"]["properties"]["dry_run"]["const"], true);
    assert_eq!(
        write["inputSchema"]["properties"]["content"]["x-maxBytes"],
        DEFAULT_MAX_WRITE_BYTES
    );
    assert!(write["description"]
        .as_str()
        .unwrap()
        .contains("mutation gate is disabled"));

    let denied = post_json_to_session(
        router.clone(),
        &session_id,
        write_call(
            "disabled",
            target.to_string_lossy().as_ref(),
            "must-not-write",
            false,
        ),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert_capability_denial(&response_json(denied).await, "write_file_mutation_disabled");
    assert!(!target.exists());

    let outside_root = tempfile::tempdir().unwrap();
    let outside = outside_root.path().join("outside.txt");
    let denied = post_json_to_session(
        router.clone(),
        &session_id,
        write_call(
            "disabled-outside",
            outside.to_string_lossy().as_ref(),
            "must-not-write",
            false,
        ),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert_capability_denial(&response_json(denied).await, "write_file_mutation_disabled");
    assert!(!outside.exists());

    let status = response_json(
        post_json_to_session(router, &session_id, runtime_status_call("status")).await,
    )
    .await;
    let runtime = &status["result"]["structuredContent"];
    assert_eq!(runtime["fileWriteMutationEnabled"], false);
    assert_eq!(runtime["fileWriteGrantRequired"], false);
    assert_eq!(runtime["fileWriteMode"], "dry_run_only_mutation_disabled");
}

#[tokio::test]
async fn enabled_gate_discovery_status_and_missing_grant_fail_closed() {
    let (root, file_tools) = empty_test_file_tools();
    let (router, _authority) = write_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let target = root.path().join("missing.txt");

    let discovery = response_json(
        post_json_to_session(
            router.clone(),
            &session_id,
            json!({"jsonrpc":"2.0","id":"tools","method":"tools/list"}),
        )
        .await,
    )
    .await;
    let write = discovery["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "write_file")
        .unwrap();
    assert!(write["inputSchema"]["properties"]["dry_run"]
        .get("const")
        .is_none());
    assert!(write["description"]
        .as_str()
        .unwrap()
        .contains("target/content/disposition-bound MCP-Capability-Grant"));

    let outside_root = tempfile::tempdir().unwrap();
    let outside_target = outside_root.path().join("outside.txt");
    let missing_parent_target = root.path().join("absent-parent/target.txt");
    let oversized_response_id = "x".repeat(MAX_WRITE_FILE_RESPONSE_BYTES + 1);
    for (index, (id, path)) in [
        (Value::from("missing"), target.as_path()),
        (Value::from("missing-outside"), outside_target.as_path()),
        (
            Value::from("missing-parent"),
            missing_parent_target.as_path(),
        ),
        (Value::from(oversized_response_id), target.as_path()),
    ]
    .into_iter()
    .enumerate()
    {
        let denied = post_json_to_session(
            router.clone(),
            &session_id,
            write_call(id, path.to_string_lossy().as_ref(), "content", false),
        )
        .await;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(denied.into_body(), MAX_WRITE_FILE_RESPONSE_BYTES + 1)
            .await
            .unwrap();
        assert!(body.len() <= MAX_WRITE_FILE_RESPONSE_BYTES);
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_capability_denial(&payload, "capability_grant_missing");
        if index == 3 {
            assert_eq!(payload["id"], Value::Null);
        }
        assert!(!path.exists());
    }

    let preview = post_json_to_session(
        router.clone(),
        &session_id,
        write_call(
            "grant-free-preview",
            target.to_string_lossy().as_ref(),
            "content",
            true,
        ),
    )
    .await;
    assert_eq!(preview.status(), StatusCode::OK);
    assert_write_result(&response_json(preview).await, true, 7, "create");
    assert!(!target.exists());

    let status = response_json(
        post_json_to_session(router, &session_id, runtime_status_call("status")).await,
    )
    .await;
    let runtime = &status["result"]["structuredContent"];
    assert_eq!(runtime["fileWriteMutationEnabled"], true);
    assert_eq!(runtime["fileWriteGrantRequired"], true);
    assert_eq!(
        runtime["fileWriteMode"],
        "dry_run_or_target_content_disposition_scoped_single_use_grant"
    );
    assert_eq!(runtime["fileWriteGrantHeader"], WRITE_FILE_GRANT_HEADER);
    assert_eq!(
        runtime["fileWriteGrantTtlSeconds"],
        WRITE_FILE_GRANT_TTL_SECONDS
    );
    assert_eq!(runtime["fileWriteMaxBytes"], DEFAULT_MAX_WRITE_BYTES);
    assert_eq!(
        runtime["fileWriteMaxResponseBytes"],
        MAX_WRITE_FILE_RESPONSE_BYTES
    );
    assert_eq!(
        runtime["auditCounters"]["by_tool"]["write_file"]["denied"],
        4
    );
    assert_eq!(
        runtime["auditCounters"]["by_reason_code"]["capability_grant_missing"]["denied"],
        4
    );
    assert_eq!(
        runtime["auditCounters"]["by_tool"]["write_file"]["allowed"],
        1
    );
}

#[tokio::test]
async fn malformed_signature_session_principal_path_and_content_bindings_are_enforced_privately() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let (router, authority) = write_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let other_authority = WriteFileGrantAuthority::from_hex_key(
        "test-key-1",
        TEST_CAPABILITY_KEY,
        "write-file-private-other-principal",
    )
    .unwrap();

    let targets = (0..6)
        .map(|index| root.path().join(format!("private-{index}.txt")))
        .collect::<Vec<_>>();
    let valid = issue_write_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        targets[1].to_string_lossy().as_ref(),
        b"private-content",
        WriteFileDisposition::Create,
    );
    let other_path = root.path().join("private-other-path.txt");
    let cases = [
        (
            "not-a-grant".to_owned(),
            "capability_grant_malformed",
            "private-content",
        ),
        (
            corrupt_signature(&valid),
            "capability_grant_signature_invalid",
            "private-content",
        ),
        (
            issue_write_file_grant(
                &authority,
                &issuer_tools,
                &uuid::Uuid::new_v4().to_string(),
                targets[2].to_string_lossy().as_ref(),
                b"private-content",
                WriteFileDisposition::Create,
            ),
            "capability_grant_binding_mismatch",
            "private-content",
        ),
        (
            issue_write_file_grant(
                &other_authority,
                &issuer_tools,
                &session_id,
                targets[3].to_string_lossy().as_ref(),
                b"private-content",
                WriteFileDisposition::Create,
            ),
            "capability_grant_binding_mismatch",
            "private-content",
        ),
        (
            issue_write_file_grant(
                &authority,
                &issuer_tools,
                &session_id,
                other_path.to_string_lossy().as_ref(),
                b"private-content",
                WriteFileDisposition::Create,
            ),
            "capability_grant_binding_mismatch",
            "private-content",
        ),
        (
            issue_write_file_grant(
                &authority,
                &issuer_tools,
                &session_id,
                targets[5].to_string_lossy().as_ref(),
                b"granted-private-content",
                WriteFileDisposition::Create,
            ),
            "capability_grant_binding_mismatch",
            "requested-private-content",
        ),
    ];

    let mut private_output = String::new();
    for (index, (grant, reason, content)) in cases.iter().enumerate() {
        let response = post_json_to_session_with_grant(
            router.clone(),
            &session_id,
            write_call(
                format!("private-{index}"),
                targets[index].to_string_lossy().as_ref(),
                content,
                false,
            ),
            grant,
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "case {index}");
        let body = response_json(response).await;
        assert_capability_denial(&body, reason);
        private_output.push_str(&body.to_string());
        assert!(!targets[index].exists());
    }

    let status = response_json(
        post_json_to_session(router, &session_id, runtime_status_call("audit")).await,
    )
    .await;
    let counters = &status["result"]["structuredContent"]["auditCounters"];
    assert_eq!(counters["by_tool"]["write_file"]["denied"], 6);
    assert_eq!(
        counters["by_reason_code"]["capability_grant_binding_mismatch"]["denied"],
        4
    );
    private_output.push_str(&status.to_string());
    for forbidden in [
        TEST_CAPABILITY_KEY,
        TEST_STATIC_PRINCIPAL,
        "write-file-private-other-principal",
        "private-other-path",
        "private-content",
        valid.as_str(),
    ] {
        assert!(!private_output.contains(forbidden));
    }
}

#[tokio::test]
async fn disposition_existing_identity_expiry_and_future_time_bindings_are_enforced() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let (router, authority) = write_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let disposition_target = root.path().join("disposition.txt");
    let disposition_grant = issue_write_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        disposition_target.to_string_lossy().as_ref(),
        b"new-content",
        WriteFileDisposition::Create,
    );
    std::fs::write(&disposition_target, "existing-content").unwrap();
    let denied = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        write_call(
            "disposition",
            disposition_target.to_string_lossy().as_ref(),
            "new-content",
            false,
        ),
        &disposition_grant,
    )
    .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert_capability_denial(
        &response_json(denied).await,
        "capability_grant_binding_mismatch",
    );
    assert_eq!(
        std::fs::read_to_string(&disposition_target).unwrap(),
        "existing-content"
    );

    let identity_target = root.path().join("identity.txt");
    let parked_target = root.path().join("identity-original.txt");
    std::fs::write(&identity_target, "identity-original").unwrap();
    let identity_grant = issue_write_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        identity_target.to_string_lossy().as_ref(),
        b"identity-authorized",
        WriteFileDisposition::Replace,
    );
    std::fs::rename(&identity_target, &parked_target).unwrap();
    std::fs::write(&identity_target, "identity-substitute").unwrap();
    let denied = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        write_call(
            "identity",
            identity_target.to_string_lossy().as_ref(),
            "identity-authorized",
            false,
        ),
        &identity_grant,
    )
    .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert_capability_denial(
        &response_json(denied).await,
        "capability_grant_binding_mismatch",
    );
    assert_eq!(
        std::fs::read_to_string(&identity_target).unwrap(),
        "identity-substitute"
    );
    assert_eq!(
        std::fs::read_to_string(&parked_target).unwrap(),
        "identity-original"
    );

    let current_time = now();
    for (name, issued, expires, reason) in [
        (
            "expired",
            current_time - 61,
            current_time - 1,
            "capability_grant_expired",
        ),
        (
            "future",
            current_time + 30,
            current_time + 30 + WRITE_FILE_GRANT_TTL_SECONDS,
            "capability_grant_future_issued",
        ),
    ] {
        let target = root.path().join(format!("{name}.txt"));
        let grant = issue_write_file_grant(
            &authority,
            &issuer_tools,
            &session_id,
            target.to_string_lossy().as_ref(),
            b"time-content",
            WriteFileDisposition::Create,
        );
        let grant = resign_with_times(&grant, issued, expires);
        let denied = post_json_to_session_with_grant(
            router.clone(),
            &session_id,
            write_call(
                name,
                target.to_string_lossy().as_ref(),
                "time-content",
                false,
            ),
            &grant,
        )
        .await;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN, "{name}");
        assert_capability_denial(&response_json(denied).await, reason);
        assert!(!target.exists());
    }
}

#[tokio::test]
async fn grants_are_single_use_for_replay_and_concurrent_replay() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let (router, authority) = write_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let replay_target = root.path().join("replay.txt");
    let replay_target_string = replay_target.to_string_lossy();
    let replay_binding = issuer_tools
        .write_file_grant_target(
            replay_target_string.as_ref(),
            b"replay-content",
            WriteFileDisposition::Create,
        )
        .unwrap();
    let replay_grant = authority.issue(&session_id, &replay_binding).unwrap();
    authority
        .consume(Some(&replay_grant), &session_id, &replay_binding)
        .unwrap();
    let replay = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        write_call(
            "replay",
            replay_target_string.as_ref(),
            "replay-content",
            false,
        ),
        &replay_grant,
    )
    .await;
    assert_eq!(replay.status(), StatusCode::FORBIDDEN);
    assert_capability_denial(&response_json(replay).await, "capability_grant_replayed");
    assert!(!replay_target.exists());

    let concurrent_target = root.path().join("concurrent.txt");
    let concurrent_binding = issuer_tools
        .write_file_grant_target(
            concurrent_target.to_string_lossy().as_ref(),
            b"concurrent-content",
            WriteFileDisposition::Create,
        )
        .unwrap();
    let concurrent_grant = Arc::new(authority.issue(&session_id, &concurrent_binding).unwrap());
    let barrier = Arc::new(tokio::sync::Barrier::new(9));
    let mut calls = tokio::task::JoinSet::new();
    for index in 0..8 {
        let router = router.clone();
        let session_id = session_id.clone();
        let target = concurrent_target.clone();
        let grant = Arc::clone(&concurrent_grant);
        let barrier = Arc::clone(&barrier);
        calls.spawn(async move {
            barrier.wait().await;
            let response = post_json_to_session_with_grant(
                router,
                &session_id,
                write_call(
                    format!("concurrent-{index}"),
                    target.to_string_lossy().as_ref(),
                    "concurrent-content",
                    false,
                ),
                &grant,
            )
            .await;
            let status = response.status();
            let body = response_json(response).await;
            (status, body)
        });
    }
    barrier.wait().await;
    let mut successes = 0;
    let mut denied = 0;
    let mut busy = 0;
    let mut capacity_denied = 0;
    while let Some(result) = calls.join_next().await {
        let (status, body) = result.unwrap();
        match status {
            StatusCode::OK => successes += 1,
            StatusCode::FORBIDDEN => {
                assert_eq!(body["error"]["code"], -32003);
                assert!(matches!(
                    body["error"]["data"]["reason"].as_str(),
                    Some("capability_grant_replayed" | "capability_grant_binding_mismatch")
                ));
                denied += 1;
            }
            StatusCode::CONFLICT => {
                assert_eq!(body["error"]["code"], -32006);
                busy += 1;
            }
            StatusCode::SERVICE_UNAVAILABLE => {
                assert_eq!(body["error"]["code"], -32007);
                assert_eq!(
                    body["error"]["message"],
                    "Filesystem mutation capacity unavailable"
                );
                capacity_denied += 1;
            }
            other => panic!("unexpected concurrent status {other}: {body}"),
        }
    }
    assert_eq!(successes, 1);
    assert_eq!(denied + busy + capacity_denied, 7);
    assert_eq!(
        authority
            .consume(
                Some(concurrent_grant.as_str()),
                &session_id,
                &concurrent_binding,
            )
            .unwrap_err(),
        termux_mcp_server::write_file_grant::WriteFileGrantError::Replayed
    );
    assert_eq!(
        std::fs::read_to_string(&concurrent_target).unwrap(),
        "concurrent-content"
    );
    assert_eq!(
        std::fs::metadata(&concurrent_target)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[tokio::test]
async fn dry_run_does_not_consume_create_grant_and_success_uses_fixed_mode() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let (router, authority) = write_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let target = root.path().join("create.txt");
    let target_path = target.to_string_lossy();
    let content = "authorized-create";
    let grant = issue_write_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        target_path.as_ref(),
        content.as_bytes(),
        WriteFileDisposition::Create,
    );

    let preview = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        write_call("preview", target_path.as_ref(), content, true),
        &grant,
    )
    .await;
    assert_eq!(preview.status(), StatusCode::OK);
    assert_write_result(&response_json(preview).await, true, content.len(), "create");
    assert!(!target.exists());

    let oversized = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        write_call(
            "x".repeat(MAX_WRITE_FILE_RESPONSE_BYTES + 1),
            target_path.as_ref(),
            content,
            false,
        ),
        &grant,
    )
    .await;
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let oversized_body = to_bytes(oversized.into_body(), MAX_WRITE_FILE_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(oversized_body.len() <= MAX_WRITE_FILE_RESPONSE_BYTES);
    let oversized_payload: Value = serde_json::from_slice(&oversized_body).unwrap();
    assert_eq!(oversized_payload["id"], Value::Null);
    assert_eq!(oversized_payload["error"]["code"], -32001);
    assert!(!target.exists());

    let created = post_json_to_session_with_grant(
        router,
        &session_id,
        write_call("create", target_path.as_ref(), content, false),
        &grant,
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    assert_write_result(
        &response_json(created).await,
        false,
        content.len(),
        "create",
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), content);
    assert_eq!(
        std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[tokio::test]
async fn replace_grant_atomically_replaces_exact_identity_with_fixed_mode_and_private_audit() {
    let (root, file_tools) = empty_test_file_tools();
    let issuer_tools = file_tools.clone();
    let (router, authority) = write_file_authorized_test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let target = root.path().join("replace.txt");
    std::fs::write(&target, "old-content").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
    let target_path = target.to_string_lossy();
    let content = "private-replacement-content";
    let grant = issue_write_file_grant(
        &authority,
        &issuer_tools,
        &session_id,
        target_path.as_ref(),
        content.as_bytes(),
        WriteFileDisposition::Replace,
    );

    let replaced = post_json_to_session_with_grant(
        router.clone(),
        &session_id,
        write_call("replace", target_path.as_ref(), content, false),
        &grant,
    )
    .await;
    assert_eq!(replaced.status(), StatusCode::OK);
    assert_write_result(
        &response_json(replaced).await,
        false,
        content.len(),
        "replace",
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), content);
    assert_eq!(
        std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let status = response_json(
        post_json_to_session(router, &session_id, runtime_status_call("audit")).await,
    )
    .await;
    let counters = &status["result"]["structuredContent"]["auditCounters"];
    assert_eq!(counters["denied_total"], 0);
    assert_eq!(counters["by_tool"]["write_file"]["allowed"], 1);
    assert_eq!(counters["by_tool"]["write_file"]["denied"], 0);
    assert_eq!(
        counters["by_reason_code"]["explicit_write_allowed"]["allowed"],
        1
    );
    let serialized = status.to_string();
    for forbidden in [
        target_path.as_ref(),
        content,
        grant.as_str(),
        TEST_CAPABILITY_KEY,
    ] {
        assert!(!serialized.contains(forbidden));
    }
}
