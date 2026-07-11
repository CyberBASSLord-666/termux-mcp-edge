#![cfg(feature = "mcp-runtime")]

mod support;

use axum::{http::StatusCode, response::Response, Router};
use serde_json::{json, Map, Value};
use support::{
    empty_test_file_tools, initialize_session, post_json_to_session, post_json_with_empty_root,
    response_json, test_router,
};
use termux_mcp_server::write_policy::DEFAULT_MAX_WRITE_BYTES;

const NO_ARGUMENT_TOOLS: [&str; 3] = ["runtime_status", "platform_info", "android_status"];
const TOOL_CALL_PARAMS_INVALID: &str = "tools/call params do not match the required schema.";
const TOOL_ARGUMENTS_INVALID: &str = "Tool arguments do not match the advertised input schema.";

fn tool_call(id: impl Into<Value>, name: &str, arguments: Option<Value>) -> Value {
    let mut params = Map::new();
    params.insert("name".to_owned(), json!(name));
    if let Some(arguments) = arguments {
        params.insert("arguments".to_owned(), arguments);
    }

    json!({
        "jsonrpc": "2.0",
        "id": id.into(),
        "method": "tools/call",
        "params": params,
    })
}

async fn post_to_router(router: Router, request_body: Value) -> Response {
    let session_id = initialize_session(&router).await;
    post_json_to_session(router, &session_id, request_body).await
}

async fn assert_invalid_params(response: Response, expected_id: &Value, expected_data: &str) {
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = response_json(response).await;
    assert_eq!(&payload["id"], expected_id);
    assert_eq!(payload["error"]["code"], -32602);
    assert_eq!(payload["error"]["message"], "Invalid params");
    assert_eq!(payload["error"]["data"], expected_data);
}

#[tokio::test]
async fn tools_call_envelope_is_closed_and_returns_one_bounded_error() {
    let sensitive = "/data/data/com.termux/files/home/private/runtime.env";
    let omitted_id = json!("closed-envelope-omitted");
    let omitted_response = post_json_with_empty_root(json!({
        "jsonrpc": "2.0",
        "id": omitted_id.clone(),
        "method": "tools/call"
    }))
    .await;
    assert_invalid_params(omitted_response, &omitted_id, TOOL_CALL_PARAMS_INVALID).await;

    let array_id = json!("closed-envelope-array");
    let array_response = post_json_with_empty_root(json!({
        "jsonrpc": "2.0",
        "id": array_id.clone(),
        "method": "tools/call",
        "params": []
    }))
    .await;
    assert_eq!(array_response.status(), StatusCode::BAD_REQUEST);
    let array_payload = response_json(array_response).await;
    assert_eq!(array_payload["id"], array_id);
    assert_eq!(array_payload["error"]["code"], -32600);
    assert_eq!(array_payload["error"]["message"], "Invalid Request");

    let cases = [
        json!({}),
        json!({"name": 7}),
        json!({"name": "runtime_status", "unexpected": sensitive}),
        json!({
            "name": "runtime_status",
            "arguments": {},
            "extra": {"secret": sensitive}
        }),
    ];

    for (index, params) in cases.into_iter().enumerate() {
        let id = json!(format!("closed-envelope-{index}"));
        let response = post_json_with_empty_root(json!({
            "jsonrpc": "2.0",
            "id": id.clone(),
            "method": "tools/call",
            "params": params,
        }))
        .await;

        assert_invalid_params(response, &id, TOOL_CALL_PARAMS_INVALID).await;
    }
}

#[tokio::test]
async fn argument_bearing_tools_reject_omitted_arguments_with_bounded_errors() {
    let cases = [
        (
            "project_service_status",
            "project_service_status requires a service_name argument.",
        ),
        ("list_directory", "list_directory requires a path argument."),
        ("read_file", "read_file requires a path argument."),
        (
            "write_file",
            "write_file requires path and content arguments.",
        ),
    ];

    for (tool_name, expected_data) in cases {
        let id = json!(format!("{tool_name}-omitted"));
        let response = post_json_with_empty_root(tool_call(id.clone(), tool_name, None)).await;
        assert_invalid_params(response, &id, expected_data).await;
    }
}

#[tokio::test]
async fn no_argument_tools_accept_only_omitted_or_empty_object() {
    for tool_name in NO_ARGUMENT_TOOLS {
        for (label, arguments) in [("omitted", None), ("empty-object", Some(json!({})))] {
            let response = post_json_with_empty_root(tool_call(
                format!("{tool_name}-{label}"),
                tool_name,
                arguments,
            ))
            .await;

            assert_eq!(response.status(), StatusCode::OK, "{tool_name}: {label}");
        }

        let rejected = [
            ("null", Value::Null),
            ("boolean", json!(false)),
            ("number", json!(7)),
            ("string", json!("value")),
            ("empty-array", json!([])),
            ("nested-array", json!([{"private": "value"}])),
            ("non-empty-object", json!({"unexpected": true})),
            ("nested-object", json!({"nested": {"private": "value"}})),
        ];

        for (label, arguments) in rejected {
            let id = json!(format!("{tool_name}-{label}"));
            let response =
                post_json_with_empty_root(tool_call(id.clone(), tool_name, Some(arguments))).await;

            assert_invalid_params(response, &id, TOOL_ARGUMENTS_INVALID).await;
        }
    }
}

#[tokio::test]
async fn argument_bearing_tools_accept_their_minimal_and_full_schemas() {
    let (root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let source = root.path().join("source.txt");
    let dry_run_target = root.path().join("dry-run-target.txt");
    let mutation_target = root.path().join("mutation-target.txt");
    tokio::fs::write(&source, "safe content").await.unwrap();

    let valid_calls = [
        (
            "service-minimal-and-full",
            "project_service_status",
            json!({"service_name": "mcp_runtime"}),
        ),
        (
            "list-minimal",
            "list_directory",
            json!({"path": root.path().to_string_lossy()}),
        ),
        (
            "list-full",
            "list_directory",
            json!({"path": root.path().to_string_lossy(), "max_depth": 5}),
        ),
        (
            "read-minimal-and-full",
            "read_file",
            json!({"path": source.to_string_lossy()}),
        ),
        (
            "write-minimal",
            "write_file",
            json!({
                "path": dry_run_target.to_string_lossy(),
                "content": "preview only"
            }),
        ),
        (
            "write-full",
            "write_file",
            json!({
                "path": mutation_target.to_string_lossy(),
                "content": "explicit mutation",
                "dry_run": false
            }),
        ),
    ];

    for (id, tool_name, arguments) in valid_calls {
        let response =
            post_to_router(router.clone(), tool_call(id, tool_name, Some(arguments))).await;
        assert_eq!(response.status(), StatusCode::OK, "valid call failed: {id}");
    }

    assert!(!dry_run_target.exists());
    assert_eq!(
        tokio::fs::read_to_string(&mutation_target).await.unwrap(),
        "explicit mutation"
    );
}

#[tokio::test]
async fn every_advertised_tool_rejects_unknown_argument_fields() {
    let (root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let source = root.path().join("source.txt");
    let target = root.path().join("must-not-change.txt");
    tokio::fs::write(&source, "safe content").await.unwrap();
    tokio::fs::write(&target, "original").await.unwrap();

    let cases = [
        ("runtime_status", json!({"unexpected": true})),
        ("platform_info", json!({"unexpected": true})),
        ("android_status", json!({"unexpected": true})),
        (
            "project_service_status",
            json!({"service_name": "mcp_runtime", "unexpected": true}),
        ),
        (
            "list_directory",
            json!({"path": root.path().to_string_lossy(), "unexpected": true}),
        ),
        (
            "read_file",
            json!({"path": source.to_string_lossy(), "unexpected": true}),
        ),
        (
            "write_file",
            json!({
                "path": target.to_string_lossy(),
                "content": "changed",
                "dry_run": false,
                "unexpected": true
            }),
        ),
    ];

    for (index, (tool_name, arguments)) in cases.into_iter().enumerate() {
        let id = json!(format!("unknown-field-{index}"));
        let response = post_to_router(
            router.clone(),
            tool_call(id.clone(), tool_name, Some(arguments)),
        )
        .await;
        assert_invalid_params(response, &id, TOOL_ARGUMENTS_INVALID).await;
    }

    assert_eq!(
        tokio::fs::read_to_string(&target).await.unwrap(),
        "original"
    );
}

#[tokio::test]
async fn argument_bearing_tools_reject_invalid_json_classes_and_field_types() {
    let (root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let source = root.path().join("source.txt");
    tokio::fs::write(&source, "safe content").await.unwrap();

    for tool_name in [
        "project_service_status",
        "list_directory",
        "read_file",
        "write_file",
    ] {
        for (label, arguments) in [
            ("null", Value::Null),
            ("scalar", json!(true)),
            ("array", json!([{"nested": "value"}])),
            ("empty-object", json!({})),
        ] {
            let id = json!(format!("{tool_name}-{label}"));
            let response = post_to_router(
                router.clone(),
                tool_call(id.clone(), tool_name, Some(arguments)),
            )
            .await;
            assert_invalid_params(response, &id, TOOL_ARGUMENTS_INVALID).await;
        }
    }

    let wrong_field_types = [
        ("project_service_status", json!({"service_name": 7})),
        (
            "project_service_status",
            json!({"service_name": "private-unlisted-service"}),
        ),
        ("list_directory", json!({"path": false})),
        (
            "list_directory",
            json!({"path": root.path().to_string_lossy(), "max_depth": "5"}),
        ),
        ("read_file", json!({"path": [source.to_string_lossy()]})),
        (
            "write_file",
            json!({"path": 7, "content": "content", "dry_run": false}),
        ),
        (
            "write_file",
            json!({"path": root.path().join("target.txt").to_string_lossy(), "content": false}),
        ),
        (
            "write_file",
            json!({
                "path": root.path().join("target.txt").to_string_lossy(),
                "content": "content",
                "dry_run": "false"
            }),
        ),
    ];

    for (index, (tool_name, arguments)) in wrong_field_types.into_iter().enumerate() {
        let id = json!(format!("wrong-field-type-{index}"));
        let response = post_to_router(
            router.clone(),
            tool_call(id.clone(), tool_name, Some(arguments)),
        )
        .await;
        assert_invalid_params(response, &id, TOOL_ARGUMENTS_INVALID).await;
    }
}

#[tokio::test]
async fn rejected_write_arguments_never_create_or_modify_files() {
    let (root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let existing = root.path().join("existing.txt");
    let missing = root.path().join("missing.txt");
    tokio::fs::write(&existing, "original").await.unwrap();

    let invalid_writes = [
        json!({
            "path": existing.to_string_lossy(),
            "content": "changed",
            "dry_run": false,
            "unknown": true
        }),
        json!({
            "path": existing.to_string_lossy(),
            "content": "changed",
            "dry_run": "false"
        }),
        json!({"path": existing.to_string_lossy(), "dry_run": false}),
        json!({
            "path": missing.to_string_lossy(),
            "content": "created",
            "dry_run": false,
            "unknown": {"nested": true}
        }),
    ];

    for (index, arguments) in invalid_writes.into_iter().enumerate() {
        let id = json!(format!("rejected-write-{index}"));
        let response = post_to_router(
            router.clone(),
            tool_call(id.clone(), "write_file", Some(arguments)),
        )
        .await;
        assert_invalid_params(response, &id, TOOL_ARGUMENTS_INVALID).await;
    }

    assert_eq!(
        tokio::fs::read_to_string(&existing).await.unwrap(),
        "original"
    );
    assert!(!missing.exists());
}

#[tokio::test]
async fn oversized_write_uses_the_write_specific_bounded_error_mapping() {
    let (root, file_tools) = empty_test_file_tools();
    let router = test_router(file_tools);
    let target = root.path().join("oversized.txt");
    let oversized = "x".repeat(DEFAULT_MAX_WRITE_BYTES + 1);

    let response = post_to_router(
        router.clone(),
        tool_call(
            "oversized-write",
            "write_file",
            Some(json!({
                "path": target.to_string_lossy(),
                "content": oversized,
                "dry_run": false
            })),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let payload = response_json(response).await;
    assert_eq!(payload["id"], "oversized-write");
    assert_eq!(payload["error"]["code"], -32001);
    assert_eq!(payload["error"]["message"], "Payload too large");
    assert_eq!(
        payload["error"]["data"],
        "File content exceeds the staged write_file byte limit."
    );
    assert!(!target.exists());

    let status_response = post_to_router(
        router,
        tool_call("audit-after-oversized-write", "runtime_status", None),
    )
    .await;
    assert_eq!(status_response.status(), StatusCode::OK);
    let status = response_json(status_response).await;
    assert_eq!(
        status["result"]["structuredContent"]["auditCounters"]["by_reason_code"]
            ["write_size_limit_exceeded"]["denied"],
        1
    );
}
