#!/usr/bin/env python3
from pathlib import Path

PATH = Path("src/mcp_transport.rs")
text = PATH.read_text()


def replace_once(old: str, new: str) -> None:
    global text
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"expected exactly one match, found {count}: {old[:120]!r}")
    text = text.replace(old, new, 1)


replace_once(
    'const FILESYSTEM_CREATE_FAILED: &str = "filesystem_directory_create_failed";\n',
    'const FILESYSTEM_CREATE_FAILED: &str = "filesystem_directory_create_failed";\n'
    'const FILESYSTEM_CREATE_MUTATION_DISABLED: &str =\n'
    '    "directory_mutation_authorization_unavailable";\n',
)

replace_once(
    '    sessions: McpSessionStore,\n    android_battery_status_enabled: bool,\n',
    '    sessions: McpSessionStore,\n'
    '    create_directory_mutation_enabled: bool,\n'
    '    android_battery_status_enabled: bool,\n',
)

needle = '            sessions: McpSessionStore::new(),\n            android_'
count = text.count(needle)
if count < 1:
    raise SystemExit("no McpTransportState constructor initializers found")
text = text.replace(
    needle,
    '            sessions: McpSessionStore::new(),\n'
    '            create_directory_mutation_enabled: false,\n'
    '            android_',
)

replace_once(
    '''        CREATE_DIRECTORY_TOOL => {
            handle_create_directory_call(
                id,
                call.arguments.into_value(),
                &state.file_tools,
                &state.audit_counters,
            )
            .await
        }
''',
    '''        CREATE_DIRECTORY_TOOL => {
            handle_create_directory_call_with_gate(
                id,
                call.arguments.into_value(),
                &state.file_tools,
                &state.audit_counters,
                state.create_directory_mutation_enabled,
            )
            .await
        }
''',
)

replace_once(
    '''#[rustfmt::skip]
async fn handle_create_directory_call(
    id: Option<Value>,
    arguments: Option<Value>,
    file_tools: &FileSystemTools,
    audit_counters: &SharedAuditCounters,
) -> Response {
''',
    '''#[rustfmt::skip]
async fn handle_create_directory_call_with_gate(
    id: Option<Value>,
    arguments: Option<Value>,
    file_tools: &FileSystemTools,
    audit_counters: &SharedAuditCounters,
    mutation_authorized: bool,
) -> Response {
''',
)

replace_once(
    '''        return bounded_payload_too_large(
            id,
            "Directory creation response exceeds the staged response byte limit.",
            MAX_CREATE_DIRECTORY_RESPONSE_BYTES,
        );
    }
    match file_tools.create_directory(args.path, Some(dry_run)).await {
''',
    '''        return bounded_payload_too_large(
            id,
            "Directory creation response exceeds the staged response byte limit.",
            MAX_CREATE_DIRECTORY_RESPONSE_BYTES,
        );
    }
    if !dry_run && !mutation_authorized {
        record_filesystem_denied(
            audit_counters,
            CREATE_DIRECTORY_TOOL,
            FILESYSTEM_WRITE_GATE,
            AuditMode::Mutating,
            FILESYSTEM_CREATE_MUTATION_DISABLED,
        );
        return tool_error_result(
            id,
            CREATE_DIRECTORY_TOOL,
            "filesystem_directory_create_unauthorized",
            FILESYSTEM_CREATE_MUTATION_DISABLED,
        );
    }
    match file_tools.create_directory(args.path, Some(dry_run)).await {
''',
)

replace_once(
    '"description": "Create one safe-rooted directory with fixed mode 0700. Defaults to dry-run; mutation requires explicit dry_run=false.",',
    '"description": "Validate one safe-rooted directory creation in dry-run mode. Live mutation is unavailable until the request-scoped authorization gate is implemented.",',
)
replace_once(
    '"description": "Defaults to true. Set explicitly to false to create exactly one directory.",',
    '"description": "Defaults to true. Explicit false requests are rejected while the mutation authorization gate is closed.",',
)
replace_once(
    'filesystem=create-directory-list-metadata-read-search-and-dry-run-write-file',
    'filesystem=create-directory-dry-run-only-list-metadata-read-search-and-dry-run-write-file',
)
replace_once(
    '"filesystemToolMode": "create_directory_list_directory_path_metadata_read_file_search_text_and_default_dry_run_write_file",',
    '"filesystemToolMode": "create_directory_dry_run_only_list_directory_path_metadata_read_file_search_text_and_default_dry_run_write_file",\n'
    '                    "createDirectoryMutation": false,\n'
    '                    "createDirectoryMutationMode": "authorization_gate_closed",',
)

insert_after = '''    fn filesystem_write_audit_mode_and_reason_follow_dry_run_state() {
        assert_eq!(filesystem_write_mode(true), AuditMode::DryRun);
        assert_eq!(filesystem_write_mode(false), AuditMode::Mutating);
        assert_eq!(filesystem_write_allowed_reason(true), FILESYSTEM_DRY_RUN_ALLOWED);
        assert_eq!(filesystem_write_allowed_reason(false), FILESYSTEM_WRITE_ALLOWED);
    }
'''
new_test = insert_after + '''

    #[tokio::test]
    async fn create_directory_mutation_fails_closed_without_authorization_gate() {
        use axum::body::to_bytes;

        let safe_root = tempfile::tempdir().unwrap();
        let destination = safe_root.path().join("must-not-exist");
        let file_tools = FileSystemTools::new(vec![safe_root.path().to_path_buf()]);
        let counters = Arc::new(Mutex::new(AuditCounters::default()));

        let response = handle_create_directory_call_with_gate(
            Some(json!("closed-gate")),
            Some(json!({
                "path": destination.to_string_lossy(),
                "dry_run": false,
            })),
            &file_tools,
            &counters,
            false,
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), MAX_CREATE_DIRECTORY_RESPONSE_BYTES)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["result"]["isError"], true);
        assert_eq!(
            payload["result"]["structuredContent"]["reasonCode"],
            FILESYSTEM_CREATE_MUTATION_DISABLED
        );
        assert!(!destination.exists());

        let snapshot = counters.lock().unwrap().clone();
        assert_eq!(snapshot.denied_total, 1);
        assert_eq!(
            snapshot.by_reason_code[FILESYSTEM_CREATE_MUTATION_DISABLED].denied,
            1
        );
    }
'''
replace_once(insert_after, new_test)

PATH.write_text(text)
