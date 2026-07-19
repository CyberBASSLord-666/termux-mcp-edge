# Runtime audit counters

Termux MCP Edge exposes staged audit counters through the `runtime_status` tool in the `structuredContent.auditCounters` field. The counters are intentionally small, in-memory, and backend-neutral. They are designed to help advanced operators verify that capability gates are being exercised without turning the MCP runtime into a log store or observability backend.

## Current scope

The current staged runtime records aggregate decisions for the enabled surfaces that are wired into the audit counter path:

- `runtime_status`
- `platform_info`
- `android_status`
- `project_service_status`
- `create_directory`
- `copy_file`
- `find_paths`
- `hash_file`
- `list_directory`
- `path_metadata`
- `read_binary_file`
- `read_binary_range`
- `read_file`
- `read_text_range`
- `search_text`
- `write_file`

When a separately compiled and runtime-enabled optional posture is active, the same counter path also records `android_battery_status`, `android_volume_status`, `set_android_volume`, or `run_command_profile`. Disabled direct calls and provider/process failures are denied decisions; successful normalized reads, previews, verified volume mutations, or fixed diagnostics are allowed decisions. A detached volume mutation task owns its terminal audit guard, so waiter loss cannot omit or duplicate the verified/recovery outcome and task drop records one stable worker-failed denial. No raw Termux:API or command output is retained. Volume counters retain only stable gate/mode/decision/reason labels; command policy events may carry a numeric profile ordinal internally, but `AuditCounters` deliberately ignores all event metadata.

The counters are additive runtime metadata. They do not change the availability, authorization, output shape, or behavior of the staged tools. They are reset when the process restarts.

Filesystem tools remain governed by safe-root validation, bounded metadata/binary reads/text reads/search/discovery/copy/hash, and dry-run-by-default mutation. Directory, file-copy, and file-write mutations are independently default-disabled and request-grant gated. Their counters retain only stable decisions/reasons, never capability keys, grants, principal fingerprints, sessions, JTIs, target/content digests, create/replace disposition bindings, filesystem identities, timestamps, artifact names/counts/bytes, or replay state. A shared `capability_*` reason remains unambiguous when read with its `by_tool` bucket; the counter model never emits a grant fingerprint to correlate attempts. Copy audit counters are content-private and retain neither endpoint path nor copied bytes, identities, source metadata, request ids, or temporary names. Path-discovery counters retain neither starting/matched path, filename, query, kind, request ID, identity, nor raw error. Hash audit counters retain neither path, content, digest, size, identity, nor partial state. Whole-file, binary-range, and UTF-8-range read audit counters retain neither path, filename, offset, requested/returned size, raw/base64/text content, file size/identity, request ID, nor host error. All filesystem counters record only stable tool names and reason codes for allowed or denied decisions.

See [`filesystem-audit-counter-contract.md`](filesystem-audit-counter-contract.md) for the filesystem-specific counter contract and [`capability-token-evaluation-contract.md`](capability-token-evaluation-contract.md) for the future high-impact capability-token evaluation boundary.

## Counter shape

`auditCounters` contains deterministic aggregate counts:

```json
{
  "allowed_total": 6,
  "denied_total": 2,
  "by_tool": {
    "android_status": {
      "allowed": 1,
      "denied": 0
    },
    "list_directory": {
      "allowed": 1,
      "denied": 1
    },
    "project_service_status": {
      "allowed": 1,
      "denied": 1
    },
    "read_file": {
      "allowed": 1,
      "denied": 0
    },
    "runtime_status": {
      "allowed": 1,
      "denied": 0
    },
    "write_file": {
      "allowed": 1,
      "denied": 0
    }
  },
  "by_reason_code": {
    "allowlisted_project_service": {
      "allowed": 1,
      "denied": 0
    },
    "allowlisted_status_metadata": {
      "allowed": 1,
      "denied": 0
    },
    "dry_run_preview": {
      "allowed": 1,
      "denied": 0
    },
    "path_outside_safe_root": {
      "allowed": 0,
      "denied": 1
    },
    "safe_root_listing": {
      "allowed": 1,
      "denied": 0
    },
    "safe_root_read": {
      "allowed": 1,
      "denied": 0
    },
    "staged_runtime_metadata": {
      "allowed": 1,
      "denied": 0
    },
    "unsupported_service": {
      "allowed": 0,
      "denied": 1
    }
  }
}
```

Sparse maps are omitted when there are no recorded decisions. If the counter mutex is poisoned, `runtime_status` reports an unavailable audit snapshot instead of exposing partial internal state.

## Non-sensitive observability contract

Audit counters may store only stable labels and aggregate counts:

- tool names
- gate names, where represented by the event source
- reason codes
- allowed and denied totals

Audit counters must not store or serialize:

- raw filesystem paths
- file contents
- command output
- command arguments
- environment variable names or values
- hostnames, usernames, Android identifiers, or private device metadata
- secrets, bearer values, passwords, API keys, capability keys/grants, principal fingerprints, sessions, JTIs, target/content digests, create/replace disposition bindings, replacement/staging identities, artifact names/counts/bytes, grant timestamps, or replay state
- arbitrary caller-supplied strings

The `AuditCounters` implementation deliberately ignores event metadata so bounded metadata used in local policy tests cannot accidentally become a runtime telemetry payload.

## Reason-code expectations

Reason codes are stable, low-cardinality labels. They are suitable for assertions and coarse operational monitoring, but they are not a substitute for full request logging.

Current runtime/status/filesystem examples include:

- `staged_runtime_metadata`
- `read_only_platform_metadata`
- `arguments_not_supported`
- `allowlisted_status_metadata`
- `arguments_not_empty_or_not_object`
- `allowlisted_project_service`
- `missing_service_name`
- `invalid_service_arguments`
- `unsupported_service`
- `safe_root_listing`
- `safe_root_read`
- `safe_root_text_searched`
- `safe_root_paths_found`
- `safe_root_directory_created`
- `safe_root_file_copied`
- `safe_root_file_hashed`
- `safe_root_binary_read`
- `find_query_invalid`
- `filesystem_find_failed`
- `filesystem_copy_source_not_found`
- `filesystem_copy_parent_not_found`
- `filesystem_copy_same_path`
- `filesystem_copy_source_type_unsupported`
- `filesystem_copy_source_too_large`
- `copy_file_mutation_disabled`
- `filesystem_copy_source_changed`
- `filesystem_copy_destination_changed`
- `filesystem_copy_failed`
- `filesystem_binary_read_target_not_found`
- `filesystem_binary_read_type_unsupported`
- `filesystem_binary_read_size_limit_exceeded`
- `filesystem_binary_read_failed`
- `filesystem_parent_not_found`
- `filesystem_destination_exists`
- `filesystem_directory_create_failed`
- `create_directory_mutation_disabled`
- `write_file_mutation_disabled`
- `filesystem_write_target_changed`
- `filesystem_write_target_not_found`
- `filesystem_write_target_type_unsupported`
- `write_quarantine_capacity_exceeded`
- `filesystem_write_failed`
- `capability_grant_missing`
- `capability_grant_malformed`
- `capability_grant_version_unknown`
- `capability_grant_key_unknown`
- `capability_grant_signature_invalid`
- `capability_grant_expired`
- `capability_grant_future_issued`
- `capability_grant_lifetime_exceeded`
- `capability_grant_binding_mismatch`
- `capability_grant_replayed`
- `capability_clock_rollback`
- `capability_replay_capacity_exhausted`
- `capability_state_unavailable`
- `search_query_invalid`
- `filesystem_search_failed`
- `dry_run_preview`
- `explicit_write_allowed`
- `missing_path_argument`
- `invalid_filesystem_arguments`
- `invalid_list_depth`
- `path_outside_safe_root`
- `read_byte_limit_exceeded`
- `write_size_limit_exceeded`
- `response_size_limit_exceeded`
- `filesystem_operation_failed`
- `battery_status_read`
- `battery_feature_not_compiled`
- `battery_runtime_disabled`
- `battery_api_unavailable`
- `battery_api_spawn_failed`
- `battery_api_wait_failed`
- `battery_api_timeout`
- `battery_stdout_limit_exceeded`
- `battery_stderr_limit_exceeded`
- `battery_api_failed`
- `battery_output_invalid_utf8`
- `battery_output_invalid_json`
- `battery_output_invalid_field`
- `volume_status_read`
- `volume_feature_not_compiled`
- `volume_runtime_disabled`
- `volume_api_unavailable`
- `volume_api_spawn_failed`
- `volume_api_wait_failed`
- `volume_api_timeout`
- `volume_stdout_limit_exceeded`
- `volume_stderr_limit_exceeded`
- `volume_api_failed`
- `volume_output_invalid_utf8`
- `volume_output_invalid_json`
- `volume_output_invalid_field`
- `volume_control_arguments_invalid`
- `volume_control_feature_not_compiled`
- `volume_control_runtime_disabled`
- `volume_control_preview`
- `volume_control_mutation_verified`
- `volume_control_stream_invalid`
- `volume_control_level_out_of_range`
- `volume_control_concurrency_limit`
- `volume_control_set_failed_rollback_confirmed`
- `volume_control_set_failed_rollback_unconfirmed`
- `volume_control_verification_failed_rollback_confirmed`
- `volume_control_verification_failed_rollback_unconfirmed`
- `volume_control_worker_failed`
- the bounded shared `capability_*` authorization reasons
- `command_profile_execution_allowed`
- `command_feature_not_compiled`
- `command_runtime_disabled`
- `command_profile_missing_arguments`
- `command_profile_invalid_arguments`
- `command_profile_not_allowlisted`
- `command_safe_root_unavailable`
- `command_program_unavailable`
- `command_spawn_failed`
- `command_wait_failed`
- `command_timeout`
- `command_stdout_limit_exceeded`
- `command_stderr_limit_exceeded`
- `command_program_failed`
- `command_output_invalid_utf8`
- `command_concurrency_limit_exceeded`

Capability-token evaluation examples include:

- `capability_grant_allowed`
- `capability_grant_missing`
- `capability_grant_inactive`
- `capability_grant_expired`
- `capability_class_mismatch`
- `capability_scope_mismatch`
- `capability_confirmation_required`

New reason codes should be short, snake_case, and tied to a policy decision rather than a caller value.

### `write_file` counter ordering

The resolved mode is `dry_run` unless the closed schema explicitly supplies `dry_run:false`. A disabled live write records one denied `write_file`/`filesystem_write`/`mutating` decision with `write_file_mutation_disabled` before path access. Response-preflight, payload, safe-root, target-state, recovery-quarantine, and grant failures each record one stable denial and never consume or identify a grant unless exact authorization reached atomic consumption. Shared grant reasons (`capability_grant_missing`, binding, expiry, replay, clock, capacity, or state failures) are recorded only under the `write_file` tool bucket.

An allowed dry run is recorded only after bounded validation succeeds. An allowed mutating write is recorded only after the exact staged inode has been published and verified, required target-parent/quarantine synchronization has succeeded, and replacement recovery retention has been confirmed where applicable. Create retains no artifact; successful replace preserves the displaced prior inode/content and reports `recoveryArtifactRetained:true`. A post-consumption transaction failure remains a denial even though the JTI is no longer reusable, including a post-commit failure that leaves the authorized new target and displaced object quarantined. Request timeout or disconnect does not create a separate identity-bearing audit event; the cancellation-independent worker finishes with the same aggregate allowed/denied contract.

Release-validator and Termux device-smoke evidence must assert the disabled/enabled write posture, grant and replay reason buckets, authorized create/replace allowed counts, recovery-retention result, quarantine-capacity denial, failure counts, and absence of paths, content, digests, identities, grants, sessions, JTIs, or artifact names from serialized `runtime_status` and evidence reports.

## Expansion rules

Future audit expansion must remain staged and explicit:

1. Add or reuse backend-neutral audit primitives first.
2. Prefer counters over retained event logs unless a later design explicitly defines storage, retention, redaction, and operator controls.
3. Keep labels low-cardinality and stable.
4. Never record raw paths, file content, command output, environment values, private host details, or raw tokens.
5. Preserve existing MCP response contracts unless a focused PR intentionally changes them.
6. Keep high-impact controls disabled unless a separate capability gate, allowlist, confirmation, dry-run or preview model, structured failure mode, and audit contract are implemented.

## Relationship to higher-risk surfaces

Audit counters are not an authorization mechanism. They provide visibility into decisions made by staged gates. Exact-stream volume control is separately implemented behind explicit opt-in and request grants; broader Android control, arbitrary command execution, package/service/network mutation, and unrelated high-impact controls remain unavailable.

Originally added for #135; updated by #142.
