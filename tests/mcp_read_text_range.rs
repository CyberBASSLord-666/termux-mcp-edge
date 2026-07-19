#![cfg(feature = "mcp-runtime")]

mod support;

use axum::{body::to_bytes, http::StatusCode};
use serde_json::{json, Value};
use support::{
    empty_test_file_tools, initialize_session, post_json_to_session, response_json, test_router,
};
use termux_mcp_server::{
    error::AppError,
    tools::{
        FileSystemTools, MAX_TEXT_RANGE_BYTES, MAX_TEXT_RANGE_FILE_BYTES,
        MAX_TEXT_RANGE_RESPONSE_BYTES, MIN_TEXT_RANGE_BYTES,
    },
};

fn range_call(id: Value, path: &str, offset_bytes: u64, max_bytes: usize) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "read_text_range",
            "arguments": {
                "path": path,
                "offset_bytes": offset_bytes,
                "max_bytes": max_bytes,
            },
        },
    })
}

#[tokio::test]
async fn discovery_advertises_one_closed_text_range_schema() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router,
        &session_id,
        json!({"jsonrpc":"2.0","id":"text-range-tools","method":"tools/list"}),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await.unwrap()).unwrap();
    let tools = payload["result"]["tools"].as_array().unwrap();
    assert_eq!(
        tools
            .iter()
            .filter(|tool| tool["name"] == "read_text_range")
            .count(),
        1
    );
    let schema = &tools
        .iter()
        .find(|tool| tool["name"] == "read_text_range")
        .unwrap()["inputSchema"];
    assert_eq!(schema["type"], "object");
    assert_eq!(
        schema["required"],
        json!(["path", "offset_bytes", "max_bytes"])
    );
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"].as_object().unwrap().len(), 3);
    assert_eq!(schema["properties"]["path"]["type"], "string");
    assert_eq!(schema["properties"]["offset_bytes"]["minimum"], 0);
    assert_eq!(
        schema["properties"]["offset_bytes"]["maximum"],
        MAX_TEXT_RANGE_FILE_BYTES
    );
    assert_eq!(
        schema["properties"]["max_bytes"]["minimum"],
        MIN_TEXT_RANGE_BYTES
    );
    assert_eq!(
        schema["properties"]["max_bytes"]["maximum"],
        MAX_TEXT_RANGE_BYTES
    );
}

#[tokio::test]
async fn text_range_pages_on_code_point_boundaries_without_path_metadata() {
    let (root, file_tools) = empty_test_file_tools();
    let path = root.path().join("private-range.txt");
    std::fs::write(&path, "aé🙂z").unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let first = post_json_to_session(
        router.clone(),
        &session_id,
        range_call(
            json!("first"),
            path.to_string_lossy().as_ref(),
            0,
            MIN_TEXT_RANGE_BYTES,
        ),
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let body = to_bytes(first.into_body(), MAX_TEXT_RANGE_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(body.len() <= MAX_TEXT_RANGE_RESPONSE_BYTES);
    let body_text = std::str::from_utf8(&body).unwrap();
    assert!(!body_text.contains("private-range.txt"));
    assert!(!body_text.contains(path.to_string_lossy().as_ref()));
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let first = &payload["result"]["structuredContent"];
    assert_eq!(first["content"], "aé");
    assert_eq!(first["offsetBytes"], 0);
    assert_eq!(first["nextOffsetBytes"], 3);
    assert_eq!(first["sizeBytes"], 3);
    assert_eq!(first["fileSizeBytes"], 8);
    assert_eq!(first["eof"], false);
    assert_eq!(first["maxReadBytes"], MAX_TEXT_RANGE_BYTES);
    assert_eq!(first["maxFileBytes"], MAX_TEXT_RANGE_FILE_BYTES);
    assert_eq!(first["maxResponseBytes"], MAX_TEXT_RANGE_RESPONSE_BYTES);
    assert_eq!(first.as_object().unwrap().len(), 9);

    let second = post_json_to_session(
        router.clone(),
        &session_id,
        range_call(
            json!("second"),
            path.to_string_lossy().as_ref(),
            3,
            MIN_TEXT_RANGE_BYTES,
        ),
    )
    .await;
    let payload: Value = serde_json::from_slice(
        &to_bytes(second.into_body(), MAX_TEXT_RANGE_RESPONSE_BYTES + 1)
            .await
            .unwrap(),
    )
    .unwrap();
    let second = &payload["result"]["structuredContent"];
    assert_eq!(second["content"], "🙂");
    assert_eq!(second["nextOffsetBytes"], 7);
    assert_eq!(second["eof"], false);

    let final_page = post_json_to_session(
        router.clone(),
        &session_id,
        range_call(
            json!("final"),
            path.to_string_lossy().as_ref(),
            7,
            MIN_TEXT_RANGE_BYTES,
        ),
    )
    .await;
    let payload: Value = serde_json::from_slice(
        &to_bytes(final_page.into_body(), MAX_TEXT_RANGE_RESPONSE_BYTES + 1)
            .await
            .unwrap(),
    )
    .unwrap();
    let final_page = &payload["result"]["structuredContent"];
    assert_eq!(final_page["content"], "z");
    assert_eq!(final_page["nextOffsetBytes"], 8);
    assert_eq!(final_page["eof"], true);

    let eof = post_json_to_session(
        router,
        &session_id,
        range_call(
            json!("eof"),
            path.to_string_lossy().as_ref(),
            8,
            MIN_TEXT_RANGE_BYTES,
        ),
    )
    .await;
    let payload: Value = serde_json::from_slice(
        &to_bytes(eof.into_body(), MAX_TEXT_RANGE_RESPONSE_BYTES + 1)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(payload["result"]["structuredContent"]["content"], "");
    assert_eq!(payload["result"]["structuredContent"]["eof"], true);
}

#[tokio::test]
async fn exact_content_and_sparse_file_limits_are_enforced() {
    use std::fs::File;

    let root = tempfile::tempdir().unwrap();
    let exact_content = root.path().join("exact-content.txt");
    let exact_file = root.path().join("exact-file.txt");
    let oversized_file = root.path().join("oversized-file.txt");
    std::fs::write(&exact_content, "x".repeat(MAX_TEXT_RANGE_BYTES)).unwrap();
    File::create(&exact_file)
        .unwrap()
        .set_len(MAX_TEXT_RANGE_FILE_BYTES as u64)
        .unwrap();
    File::create(&oversized_file)
        .unwrap()
        .set_len((MAX_TEXT_RANGE_FILE_BYTES + 1) as u64)
        .unwrap();
    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

    let exact = tools
        .read_text_range(
            exact_content.to_string_lossy().to_string(),
            0,
            MAX_TEXT_RANGE_BYTES,
        )
        .await
        .unwrap();
    assert_eq!(exact.content.len(), MAX_TEXT_RANGE_BYTES);
    assert_eq!(exact.size_bytes, MAX_TEXT_RANGE_BYTES);
    assert!(exact.eof);

    let eof = tools
        .read_text_range(
            exact_file.to_string_lossy().to_string(),
            MAX_TEXT_RANGE_FILE_BYTES as u64,
            MIN_TEXT_RANGE_BYTES,
        )
        .await
        .unwrap();
    assert!(eof.content.is_empty());
    assert!(eof.eof);

    assert!(matches!(
        tools
            .read_text_range(oversized_file.to_string_lossy().to_string(), 0, 4)
            .await,
        Err(AppError::FileTooLarge { size, max_size })
            if size == (MAX_TEXT_RANGE_FILE_BYTES + 1) as u64
                && max_size == MAX_TEXT_RANGE_FILE_BYTES as u64
    ));

    let router = test_router(tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router,
        &session_id,
        range_call(
            json!("exact-content"),
            exact_content.to_string_lossy().as_ref(),
            0,
            MAX_TEXT_RANGE_BYTES,
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), MAX_TEXT_RANGE_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(body.len() <= MAX_TEXT_RANGE_RESPONSE_BYTES);
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        payload["result"]["structuredContent"]["content"]
            .as_str()
            .unwrap()
            .len(),
        MAX_TEXT_RANGE_BYTES
    );
}

#[cfg(unix)]
#[tokio::test]
async fn text_range_rejects_missing_outside_symlinked_and_unsupported_targets() {
    use std::os::unix::{fs::symlink, net::UnixListener};

    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_file = outside.path().join("outside.txt");
    std::fs::write(&outside_file, b"outside-secret").unwrap();
    let link = root.path().join("link.txt");
    symlink(&outside_file, &link).unwrap();
    let socket = root.path().join("socket");
    let _listener = UnixListener::bind(&socket).unwrap();
    let linked_parent = root.path().join("linked-parent");
    symlink(outside.path(), &linked_parent).unwrap();
    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

    assert!(matches!(
        tools
            .read_text_range(
                root.path().join("missing").to_string_lossy().to_string(),
                0,
                MIN_TEXT_RANGE_BYTES,
            )
            .await,
        Err(AppError::PathNotFound)
    ));
    for target in [outside_file, link, linked_parent.join("outside.txt")] {
        assert!(matches!(
            tools
                .read_text_range(
                    target.to_string_lossy().to_string(),
                    0,
                    MIN_TEXT_RANGE_BYTES,
                )
                .await,
            Err(AppError::PathTraversal { .. })
        ));
    }
    for target in [root.path().to_path_buf(), socket] {
        assert!(matches!(
            tools
                .read_text_range(
                    target.to_string_lossy().to_string(),
                    0,
                    MIN_TEXT_RANGE_BYTES,
                )
                .await,
            Err(AppError::UnsupportedPathType) | Err(AppError::PathTraversal { .. })
        ));
    }
}

#[tokio::test]
async fn text_range_arguments_expansion_preflight_and_audits_are_bounded_and_private() {
    let (root, file_tools) = empty_test_file_tools();
    let path = root.path().join("private-text-range-target.txt");
    let invalid_utf8 = root.path().join("invalid-utf8.txt");
    let expanded = root.path().join("expanded.txt");
    std::fs::write(&path, "aé🙂z").unwrap();
    std::fs::write(&invalid_utf8, [b'a', 0xff]).unwrap();
    std::fs::write(&expanded, vec![0_u8; MAX_TEXT_RANGE_BYTES]).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    for (index, arguments) in [
        None,
        Some(json!({
            "path": path.to_string_lossy(),
            "offset_bytes": 0,
            "max_bytes": MIN_TEXT_RANGE_BYTES,
            "unexpected": true
        })),
        Some(json!({"path": false, "offset_bytes": 0, "max_bytes": 4})),
        Some(json!({"path": path.to_string_lossy(), "offset_bytes": -1, "max_bytes": 4})),
        Some(json!({"path": path.to_string_lossy(), "offset_bytes": 0, "max_bytes": -1})),
    ]
    .into_iter()
    .enumerate()
    {
        let params = match arguments {
            Some(arguments) => json!({"name":"read_text_range","arguments":arguments}),
            None => json!({"name":"read_text_range"}),
        };
        let response = post_json_to_session(
            router.clone(),
            &session_id,
            json!({
                "jsonrpc": "2.0",
                "id": format!("invalid-{index}"),
                "method": "tools/call",
                "params": params
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    for (index, (offset, max_bytes)) in [
        (0, MIN_TEXT_RANGE_BYTES - 1),
        (0, MAX_TEXT_RANGE_BYTES + 1),
        (MAX_TEXT_RANGE_FILE_BYTES as u64 + 1, MIN_TEXT_RANGE_BYTES),
        (9, MIN_TEXT_RANGE_BYTES),
        (2, MIN_TEXT_RANGE_BYTES),
    ]
    .into_iter()
    .enumerate()
    {
        let response = post_json_to_session(
            router.clone(),
            &session_id,
            range_call(
                json!(format!("bad-range-{index}")),
                path.to_string_lossy().as_ref(),
                offset,
                max_bytes,
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        if index >= 3 {
            let payload = response_json(response).await;
            assert_eq!(
                payload["error"]["data"],
                "Requested text range is not valid"
            );
        }
    }

    let invalid = post_json_to_session(
        router.clone(),
        &session_id,
        range_call(
            json!("invalid-utf8"),
            invalid_utf8.to_string_lossy().as_ref(),
            0,
            MIN_TEXT_RANGE_BYTES,
        ),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let allowed = post_json_to_session(
        router.clone(),
        &session_id,
        range_call(
            json!("allowed"),
            path.to_string_lossy().as_ref(),
            0,
            MIN_TEXT_RANGE_BYTES,
        ),
    )
    .await;
    assert_eq!(allowed.status(), StatusCode::OK);
    let denied = post_json_to_session(
        router.clone(),
        &session_id,
        range_call(
            json!("denied"),
            outside.path().to_string_lossy().as_ref(),
            0,
            MIN_TEXT_RANGE_BYTES,
        ),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::BAD_REQUEST);

    let expanded_response = post_json_to_session(
        router.clone(),
        &session_id,
        range_call(
            json!("expanded"),
            expanded.to_string_lossy().as_ref(),
            0,
            MAX_TEXT_RANGE_BYTES,
        ),
    )
    .await;
    assert_eq!(expanded_response.status(), StatusCode::OK);
    let expanded_body = to_bytes(
        expanded_response.into_body(),
        MAX_TEXT_RANGE_RESPONSE_BYTES + 1,
    )
    .await
    .unwrap();
    assert!(expanded_body.len() <= MAX_TEXT_RANGE_RESPONSE_BYTES);

    std::fs::remove_file(&path).unwrap();
    let oversized_id = "x".repeat(MAX_TEXT_RANGE_BYTES);
    let expected_id = json!(oversized_id.clone());
    for payload in [
        json!({
            "jsonrpc": "2.0",
            "id": oversized_id.clone(),
            "method": "tools/call",
            "params": {"name": "read_text_range"}
        }),
        range_call(
            json!(oversized_id),
            path.to_string_lossy().as_ref(),
            0,
            MIN_TEXT_RANGE_BYTES,
        ),
    ] {
        let response = post_json_to_session(router.clone(), &session_id, payload).await;
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = to_bytes(response.into_body(), MAX_TEXT_RANGE_RESPONSE_BYTES + 1)
            .await
            .unwrap();
        assert!(body.len() <= MAX_TEXT_RANGE_RESPONSE_BYTES);
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["id"], expected_id);
        assert_eq!(payload["error"]["code"], -32001);
    }

    let status = post_json_to_session(
        router,
        &session_id,
        json!({
            "jsonrpc": "2.0",
            "id": "status",
            "method": "tools/call",
            "params": {"name": "runtime_status", "arguments": {}}
        }),
    )
    .await;
    assert_eq!(status.status(), StatusCode::OK);
    let body = to_bytes(status.into_body(), 128 * 1024).await.unwrap();
    let text = std::str::from_utf8(&body).unwrap();
    for forbidden in [
        "private-text-range-target",
        "invalid-utf8.txt",
        path.to_string_lossy().as_ref(),
    ] {
        assert!(!text.contains(forbidden));
    }
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let runtime = &payload["result"]["structuredContent"];
    assert_eq!(runtime["textRangeReads"], true);
    assert_eq!(runtime["textRangeReadEncoding"], "utf-8");
    assert_eq!(runtime["textRangeReadMinBytes"], MIN_TEXT_RANGE_BYTES);
    assert_eq!(
        runtime["textRangeReadMaxFileBytes"],
        MAX_TEXT_RANGE_FILE_BYTES
    );
    assert_eq!(runtime["textRangeReadMaxBytes"], MAX_TEXT_RANGE_BYTES);
    assert_eq!(
        runtime["textRangeReadMaxResponseBytes"],
        MAX_TEXT_RANGE_RESPONSE_BYTES
    );
    let counters = &runtime["auditCounters"];
    assert_eq!(counters["by_tool"]["read_text_range"]["allowed"], 2);
    assert_eq!(counters["by_tool"]["read_text_range"]["denied"], 14);
    assert_eq!(
        counters["by_reason_code"]["safe_root_text_range_read"]["allowed"],
        2
    );
    assert_eq!(
        counters["by_reason_code"]["filesystem_text_range_invalid"]["denied"],
        5
    );
    assert_eq!(counters["by_reason_code"]["invalid_arguments"]["denied"], 4);
    assert_eq!(
        counters["by_reason_code"]["response_size_limit_exceeded"]["denied"],
        2
    );
}
