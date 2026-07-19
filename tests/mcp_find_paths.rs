#![cfg(feature = "mcp-runtime")]

mod support;

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::symlink;
use std::os::unix::net::UnixListener;

use axum::{body::to_bytes, http::StatusCode};
use serde_json::{json, Value};
use support::{empty_test_file_tools, initialize_session, post_json_to_session, test_router};
use termux_mcp_server::{
    error::AppError,
    tools::{
        FileSystemTools, FindPathFilter, FindPathKind, MAX_FIND_DEPTH, MAX_FIND_ENTRIES,
        MAX_FIND_MATCHES, MAX_FIND_QUERY_BYTES, MAX_FIND_RESPONSE_BYTES, MIN_FIND_DEPTH,
    },
};

fn find_call(id: Value, arguments: Option<Value>) -> Value {
    let params = match arguments {
        Some(arguments) => json!({"name": "find_paths", "arguments": arguments}),
        None => json!({"name": "find_paths"}),
    };
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": params,
    })
}

#[tokio::test]
async fn discovery_advertises_one_closed_literal_path_schema() {
    let (_root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;
    let response = post_json_to_session(
        router,
        &session_id,
        json!({"jsonrpc":"2.0","id":"find-tools","method":"tools/list"}),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await.unwrap()).unwrap();
    let tools = payload["result"]["tools"].as_array().unwrap();
    assert_eq!(
        tools
            .iter()
            .filter(|tool| tool["name"] == "find_paths")
            .count(),
        1
    );
    let schema = &tools
        .iter()
        .find(|tool| tool["name"] == "find_paths")
        .unwrap()["inputSchema"];
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["required"], json!(["path", "query"]));
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"].as_object().unwrap().len(), 4);
    assert_eq!(schema["properties"]["query"]["minLength"], 1);
    assert_eq!(
        schema["properties"]["query"]["maxLength"],
        MAX_FIND_QUERY_BYTES
    );
    assert_eq!(
        schema["properties"]["query"]["x-maxBytes"],
        MAX_FIND_QUERY_BYTES
    );
    assert_eq!(
        schema["properties"]["kind"]["enum"],
        json!(["any", "regular_file", "directory"])
    );
    assert_eq!(
        schema["properties"]["max_depth"]["minimum"],
        MIN_FIND_DEPTH
    );
    assert_eq!(
        schema["properties"]["max_depth"]["maximum"],
        MAX_FIND_DEPTH
    );
}

#[tokio::test]
async fn find_paths_is_literal_content_private_ordered_and_kind_filtered() {
    let (root, file_tools) = empty_test_file_tools();
    let nested = root.path().join("nested");
    let matching_directory = root.path().join("z-match-directory");
    std::fs::create_dir(&nested).unwrap();
    std::fs::create_dir(&matching_directory).unwrap();
    std::fs::write(root.path().join("a-match-file.txt"), "private-content-a").unwrap();
    std::fs::write(nested.join("deep-match.log"), "private-content-b").unwrap();
    std::fs::write(root.path().join("unrelated.txt"), "match-in-content-only").unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let shallow = post_json_to_session(
        router.clone(),
        &session_id,
        find_call(
            json!("shallow"),
            Some(json!({
                "path": root.path().to_string_lossy(),
                "query": "match",
                "max_depth": 1
            })),
        ),
    )
    .await;
    assert_eq!(shallow.status(), StatusCode::OK);
    let body = to_bytes(shallow.into_body(), MAX_FIND_RESPONSE_BYTES + 1)
        .await
        .unwrap();
    assert!(body.len() <= MAX_FIND_RESPONSE_BYTES);
    let body_text = std::str::from_utf8(&body).unwrap();
    assert!(!body_text.contains("private-content-a"));
    assert!(!body_text.contains("private-content-b"));
    assert!(!body_text.contains("match-in-content-only"));
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let result = &payload["result"]["structuredContent"];
    assert_eq!(result["truncated"], false);
    assert_eq!(result["queryBytes"], 5);
    assert_eq!(result["kindFilter"], "any");
    assert_eq!(result["maxDepth"], 1);
    assert_eq!(result["maxEntries"], MAX_FIND_ENTRIES);
    assert_eq!(result["maxMatches"], MAX_FIND_MATCHES);
    assert_eq!(result["maxResponseBytes"], MAX_FIND_RESPONSE_BYTES);
    assert_eq!(result["matches"].as_array().unwrap().len(), 2);
    assert_eq!(result["matches"][0]["kind"], "regular_file");
    assert!(result["matches"][0]["path"]
        .as_str()
        .unwrap()
        .ends_with("a-match-file.txt"));
    assert_eq!(result["matches"][1]["kind"], "directory");
    assert!(result["matches"][1]["path"]
        .as_str()
        .unwrap()
        .ends_with("z-match-directory"));

    let files = post_json_to_session(
        router.clone(),
        &session_id,
        find_call(
            json!("files"),
            Some(json!({
                "path": root.path().to_string_lossy(),
                "query": "match",
                "kind": "regular_file"
            })),
        ),
    )
    .await;
    assert_eq!(files.status(), StatusCode::OK);
    let payload: Value = serde_json::from_slice(
        &to_bytes(files.into_body(), MAX_FIND_RESPONSE_BYTES + 1)
            .await
            .unwrap(),
    )
    .unwrap();
    let result = &payload["result"]["structuredContent"];
    assert_eq!(result["kindFilter"], "regular_file");
    assert_eq!(result["maxDepth"], MAX_FIND_DEPTH);
    assert_eq!(result["matches"].as_array().unwrap().len(), 2);
    assert!(result["matches"].as_array().unwrap().iter().all(|entry| {
        entry["kind"] == "regular_file" && entry["path"].as_str().unwrap().contains("match")
    }));

    let directories = post_json_to_session(
        router.clone(),
        &session_id,
        find_call(
            json!("directories"),
            Some(json!({
                "path": root.path().to_string_lossy(),
                "query": "match",
                "kind": "directory"
            })),
        ),
    )
    .await;
    assert_eq!(directories.status(), StatusCode::OK);
    let payload: Value = serde_json::from_slice(
        &to_bytes(directories.into_body(), MAX_FIND_RESPONSE_BYTES + 1)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        payload["result"]["structuredContent"]["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let none = post_json_to_session(
        router,
        &session_id,
        find_call(
            json!("none"),
            Some(json!({
                "path": root.path().to_string_lossy(),
                "query": "absent-literal"
            })),
        ),
    )
    .await;
    assert_eq!(none.status(), StatusCode::OK);
    let payload: Value = serde_json::from_slice(
        &to_bytes(none.into_body(), MAX_FIND_RESPONSE_BYTES + 1)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        payload["result"]["structuredContent"]["matches"],
        json!([])
    );
}

#[tokio::test]
async fn find_paths_skips_unsafe_and_invalid_names_and_rejects_linked_parents() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("valid-needle.txt"), "private").unwrap();
    std::fs::write(outside.path().join("outside-needle.txt"), "outside").unwrap();
    symlink(
        outside.path().join("outside-needle.txt"),
        root.path().join("symlink-needle"),
    )
    .unwrap();
    let _socket = UnixListener::bind(root.path().join("socket-needle")).unwrap();
    let invalid_name = OsString::from_vec(vec![b'i', b'n', b'v', 0xff, b'n']);
    std::fs::write(root.path().join(&invalid_name), "invalid-name-private").unwrap();
    symlink(outside.path(), root.path().join("linked-parent")).unwrap();
    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

    let result = tools
        .find_paths(
            root.path().to_string_lossy().to_string(),
            "needle".to_owned(),
            FindPathFilter::Any,
            None,
        )
        .await
        .unwrap();
    assert_eq!(result.matches.len(), 1);
    assert_eq!(result.matches[0].kind, FindPathKind::RegularFile);
    assert!(result.matches[0].path.ends_with("valid-needle.txt"));
    assert_eq!(result.skipped_invalid_utf8_entries, 1);
    assert_eq!(result.skipped_unsafe_entries, 3);

    let linked_parent = tools
        .find_paths(
            root.path()
                .join("linked-parent")
                .to_string_lossy()
                .to_string(),
            "outside".to_owned(),
            FindPathFilter::Any,
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(linked_parent, AppError::PathTraversal { .. }));

    let outside_error = tools
        .find_paths(
            outside.path().to_string_lossy().to_string(),
            "outside".to_owned(),
            FindPathFilter::Any,
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(outside_error, AppError::PathTraversal { .. }));
}

#[tokio::test]
async fn find_paths_enforces_match_entry_and_complete_response_bounds() {
    let match_root = tempfile::tempdir().unwrap();
    for index in 0..=MAX_FIND_MATCHES {
        std::fs::write(
            match_root.path().join(format!("needle-{index:04}.txt")),
            "private",
        )
        .unwrap();
    }
    let tools = FileSystemTools::new(vec![match_root.path().to_path_buf()]);
    let result = tools
        .find_paths(
            match_root.path().to_string_lossy().to_string(),
            "needle".to_owned(),
            FindPathFilter::Any,
            None,
        )
        .await
        .unwrap();
    assert_eq!(result.matches.len(), MAX_FIND_MATCHES);
    assert!(result.truncated);
    assert!(result
        .matches
        .windows(2)
        .all(|pair| pair[0].path < pair[1].path));

    let entry_root = tempfile::tempdir().unwrap();
    for index in 0..=MAX_FIND_ENTRIES {
        std::fs::write(
            entry_root.path().join(format!("entry-{index:05}.txt")),
            "private",
        )
        .unwrap();
    }
    let tools = FileSystemTools::new(vec![entry_root.path().to_path_buf()]);
    let result = tools
        .find_paths(
            entry_root.path().to_string_lossy().to_string(),
            "never-matches".to_owned(),
            FindPathFilter::Any,
            None,
        )
        .await
        .unwrap();
    assert_eq!(result.entries_examined, MAX_FIND_ENTRIES);
    assert!(result.truncated);

    let response_root = tempfile::tempdir().unwrap();
    let mut nested = response_root.path().to_path_buf();
    for depth in 0..4 {
        nested = nested.join(format!("{}-{depth}", "d".repeat(220)));
        std::fs::create_dir(&nested).unwrap();
    }
    for index in 0..MAX_FIND_MATCHES {
        std::fs::write(
            nested.join(format!("needle-{}-{index:04}", "f".repeat(210))),
            "private",
        )
        .unwrap();
    }
    let tools = FileSystemTools::new(vec![response_root.path().to_path_buf()]);
    let result = tools
        .find_paths(
            response_root.path().to_string_lossy().to_string(),
            "needle".to_owned(),
            FindPathFilter::Any,
            None,
        )
        .await
        .unwrap();
    assert!(result.truncated);
    assert!(result.matches.len() < MAX_FIND_MATCHES);
    assert!(serde_json::to_vec(&result).unwrap().len() <= MAX_FIND_RESPONSE_BYTES - 1_024);
}

#[tokio::test]
async fn find_paths_preflights_arguments_and_response_then_audits_private_reasons() {
    let (root, file_tools) = empty_test_file_tools();
    let path = root.path().join("private-needle.txt");
    std::fs::write(&path, "private-content-marker").unwrap();
    let outside = tempfile::tempdir().unwrap();
    let router = test_router(file_tools);
    let session_id = initialize_session(&router).await;

    let invalid = [
        None,
        Some(json!({"path": root.path().to_string_lossy(), "query": ""})),
        Some(json!({
            "path": root.path().to_string_lossy(),
            "query": "q".repeat(MAX_FIND_QUERY_BYTES + 1)
        })),
        Some(json!({
            "path": root.path().to_string_lossy(),
            "query": "😀".repeat((MAX_FIND_QUERY_BYTES / 4) + 1)
        })),
        Some(json!({"path": root.path().to_string_lossy(), "query": "two\nlines"})),
        Some(json!({"path": root.path().to_string_lossy(), "query": "a/b"})),
        Some(json!({
            "path": root.path().to_string_lossy(),
            "query": "needle",
            "kind": "symlink"
        })),
        Some(json!({
            "path": root.path().to_string_lossy(),
            "query": "needle",
            "max_depth": MIN_FIND_DEPTH - 1
        })),
        Some(json!({
            "path": root.path().to_string_lossy(),
            "query": "needle",
            "max_depth": MAX_FIND_DEPTH + 1
        })),
        Some(json!({
            "path": root.path().to_string_lossy(),
            "query": "needle",
            "glob": true
        })),
    ];
    for (index, arguments) in invalid.into_iter().enumerate() {
        let response = post_json_to_session(
            router.clone(),
            &session_id,
            find_call(json!(format!("invalid-{index}")), arguments),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "case {index}");
    }

    let allowed = post_json_to_session(
        router.clone(),
        &session_id,
        find_call(
            json!("allowed"),
            Some(json!({
                "path": root.path().to_string_lossy(),
                "query": "needle"
            })),
        ),
    )
    .await;
    assert_eq!(allowed.status(), StatusCode::OK);
    let denied = post_json_to_session(
        router.clone(),
        &session_id,
        find_call(
            json!("denied"),
            Some(json!({
                "path": outside.path().to_string_lossy(),
                "query": "private-query"
            })),
        ),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::BAD_REQUEST);

    std::fs::remove_file(&path).unwrap();
    let oversized_id = "x".repeat(MAX_FIND_RESPONSE_BYTES);
    for arguments in [
        None,
        Some(json!({
            "path": path.to_string_lossy(),
            "query": "needle"
        })),
    ] {
        let response = post_json_to_session(
            router.clone(),
            &session_id,
            find_call(json!(oversized_id.clone()), arguments),
        )
        .await;
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let payload: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), MAX_FIND_RESPONSE_BYTES + 1)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["id"], Value::Null);
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
        "private-content-marker",
        "private-query",
        path.to_string_lossy().as_ref(),
        outside.path().to_string_lossy().as_ref(),
    ] {
        assert!(!text.contains(forbidden));
    }
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let runtime = &payload["result"]["structuredContent"];
    assert_eq!(runtime["pathDiscovery"], true);
    assert_eq!(
        runtime["pathDiscoveryMatchMode"],
        "case_sensitive_literal_basename"
    );
    assert_eq!(runtime["pathDiscoveryMaxDepth"], MAX_FIND_DEPTH);
    assert_eq!(runtime["pathDiscoveryMaxEntries"], MAX_FIND_ENTRIES);
    assert_eq!(runtime["pathDiscoveryMaxMatches"], MAX_FIND_MATCHES);
    assert_eq!(runtime["pathDiscoveryMaxQueryBytes"], MAX_FIND_QUERY_BYTES);
    assert_eq!(
        runtime["pathDiscoveryMaxResponseBytes"],
        MAX_FIND_RESPONSE_BYTES
    );
    let counters = &runtime["auditCounters"];
    assert_eq!(counters["by_tool"]["find_paths"]["allowed"], 1);
    assert_eq!(counters["by_tool"]["find_paths"]["denied"], 13);
    assert_eq!(
        counters["by_reason_code"]["safe_root_paths_found"]["allowed"],
        1
    );
    assert_eq!(
        counters["by_reason_code"]["find_query_invalid"]["denied"],
        5
    );
    assert_eq!(counters["by_reason_code"]["invalid_arguments"]["denied"], 2);
    assert_eq!(
        counters["by_reason_code"]["invalid_max_depth"]["denied"],
        2
    );
    assert_eq!(
        counters["by_reason_code"]["response_size_limit_exceeded"]["denied"],
        2
    );
    assert_eq!(counters["by_reason_code"]["safe_root_rejected"]["denied"], 1);
    assert_eq!(counters["by_reason_code"]["missing_arguments"]["denied"], 1);
}
