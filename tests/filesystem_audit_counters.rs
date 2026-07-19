use termux_mcp_server::audit::{
    filesystem_allowed_event, filesystem_denied_event, AuditCounters, AuditMode,
};

#[test]
fn filesystem_audit_events_increment_aggregate_counters_without_sensitive_values() {
    let mut counters = AuditCounters::default();

    let allowed_list = filesystem_allowed_event(
        1_725_000_000,
        "list_directory",
        "filesystem_read",
        AuditMode::ReadOnly,
        "safe_root_listed",
    );
    let allowed_dry_run = filesystem_allowed_event(
        1_725_000_001,
        "write_file",
        "filesystem_write",
        AuditMode::DryRun,
        "dry_run_preview",
    );
    let allowed_search = filesystem_allowed_event(
        1_725_000_002,
        "search_text",
        "filesystem_read",
        AuditMode::ReadOnly,
        "safe_root_text_searched",
    );
    let allowed_metadata = filesystem_allowed_event(
        1_725_000_003,
        "path_metadata",
        "filesystem_metadata",
        AuditMode::ReadOnly,
        "safe_root_metadata_read",
    );
    let allowed_create = filesystem_allowed_event(
        1_725_000_004,
        "create_directory",
        "filesystem_write",
        AuditMode::Mutating,
        "safe_root_directory_created",
    );
    let allowed_copy = filesystem_allowed_event(
        1_725_000_005,
        "copy_file",
        "filesystem_write",
        AuditMode::Mutating,
        "safe_root_file_copied",
    );
    let allowed_hash = filesystem_allowed_event(
        1_725_000_006,
        "hash_file",
        "filesystem_read",
        AuditMode::ReadOnly,
        "safe_root_file_hashed",
    );
    let allowed_find = filesystem_allowed_event(
        1_725_000_007,
        "find_paths",
        "filesystem_read",
        AuditMode::ReadOnly,
        "safe_root_paths_found",
    );
    let allowed_binary_read = filesystem_allowed_event(
        1_725_000_008,
        "read_binary_file",
        "filesystem_read",
        AuditMode::ReadOnly,
        "safe_root_binary_read",
    );
    let allowed_binary_range = filesystem_allowed_event(
        1_725_000_009,
        "read_binary_range",
        "filesystem_read",
        AuditMode::ReadOnly,
        "safe_root_binary_range_read",
    );
    let denied_read = filesystem_denied_event(
        1_725_000_010,
        "read_file",
        "filesystem_read",
        AuditMode::ReadOnly,
        "safe_root_rejected",
    );

    counters.record_event(&allowed_list);
    counters.record_event(&allowed_dry_run);
    counters.record_event(&allowed_search);
    counters.record_event(&allowed_metadata);
    counters.record_event(&allowed_create);
    counters.record_event(&allowed_copy);
    counters.record_event(&allowed_hash);
    counters.record_event(&allowed_find);
    counters.record_event(&allowed_binary_read);
    counters.record_event(&allowed_binary_range);
    counters.record_event(&denied_read);

    assert_eq!(counters.allowed_total, 10);
    assert_eq!(counters.denied_total, 1);
    assert_eq!(counters.total(), 11);
    assert_eq!(counters.by_tool["list_directory"].allowed, 1);
    assert_eq!(counters.by_tool["write_file"].allowed, 1);
    assert_eq!(counters.by_tool["read_file"].denied, 1);
    assert_eq!(counters.by_tool["search_text"].allowed, 1);
    assert_eq!(counters.by_tool["path_metadata"].allowed, 1);
    assert_eq!(counters.by_tool["create_directory"].allowed, 1);
    assert_eq!(counters.by_tool["copy_file"].allowed, 1);
    assert_eq!(counters.by_tool["hash_file"].allowed, 1);
    assert_eq!(counters.by_tool["find_paths"].allowed, 1);
    assert_eq!(counters.by_tool["read_binary_file"].allowed, 1);
    assert_eq!(counters.by_tool["read_binary_range"].allowed, 1);
    assert_eq!(counters.by_reason_code["safe_root_listed"].allowed, 1);
    assert_eq!(counters.by_reason_code["dry_run_preview"].allowed, 1);
    assert_eq!(counters.by_reason_code["safe_root_rejected"].denied, 1);
    assert_eq!(
        counters.by_reason_code["safe_root_text_searched"].allowed,
        1
    );
    assert_eq!(
        counters.by_reason_code["safe_root_metadata_read"].allowed,
        1
    );
    assert_eq!(
        counters.by_reason_code["safe_root_directory_created"].allowed,
        1
    );
    assert_eq!(counters.by_reason_code["safe_root_file_copied"].allowed, 1);
    assert_eq!(counters.by_reason_code["safe_root_file_hashed"].allowed, 1);
    assert_eq!(counters.by_reason_code["safe_root_paths_found"].allowed, 1);
    assert_eq!(counters.by_reason_code["safe_root_binary_read"].allowed, 1);
    assert_eq!(
        counters.by_reason_code["safe_root_binary_range_read"].allowed,
        1
    );

    let serialized = serde_json::to_string(&counters)
        .expect("filesystem audit counters should serialize deterministically")
        .to_ascii_lowercase();

    for forbidden in [
        "/data/", "bearer", "content", "password", "secret", "token", "/home/",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "filesystem audit counters must not expose sensitive token: {forbidden}"
        );
    }
}

#[test]
fn filesystem_mutating_write_decision_is_counted_without_payload_metadata() {
    let mut counters = AuditCounters::default();
    let event = filesystem_allowed_event(
        1_725_000_003,
        "write_file",
        "filesystem_write",
        AuditMode::Mutating,
        "explicit_write_allowed",
    );

    counters.record_event(&event);

    assert_eq!(counters.allowed_total, 1);
    assert_eq!(counters.denied_total, 0);
    assert_eq!(counters.by_tool["write_file"].allowed, 1);
    assert_eq!(counters.by_reason_code["explicit_write_allowed"].allowed, 1);
    assert!(event.metadata.is_empty());
}
