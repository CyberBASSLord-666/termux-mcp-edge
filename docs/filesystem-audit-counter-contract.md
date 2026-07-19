# Filesystem audit counter contract

This document defines the staged runtime contract for counting filesystem tool decisions without capturing sensitive caller data.

The contract applies to the existing staged filesystem tools:

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

It builds on the backend-neutral audit helpers in `src/audit.rs`, including `filesystem_allowed_event`, `filesystem_denied_event`, `AuditMode`, and `AuditCounters`.

## Goals

Filesystem audit counter wiring should make operator-visible runtime counters useful while preserving the current staged security posture.

The runtime should count:

- allowed safe-rooted directory listings
- allowed directory-creation dry runs and explicit one-directory mutations
- denied directory creation, including invalid arguments, disabled mutation, stable grant authorization failures, safe-root rejection, missing parents, existing destinations, response bounds, and internal failures
- allowed bounded file-copy previews and explicit fixed-mode mutations
- denied file copy, including invalid arguments, safe-root rejection, missing source/parent, same path, existing destination, unsupported source, size/response bounds, and internal failures
- allowed bounded SHA-256 hashing of one safe-rooted regular file
- denied file hashing, including invalid arguments, safe-root rejection, missing or unsupported targets, size/response bounds, and internal failures
- allowed bounded content-free literal basename discovery
- denied path discovery, including invalid arguments/query/depth, safe-root rejection, response bounds, and internal failures
- denied directory-listing requests, including invalid arguments and safe-root rejections
- allowed bounded metadata reads for one safe-rooted regular file or directory
- denied metadata requests, including invalid arguments, missing objects, unsupported types, safe-root rejections, response bounds, and internal failures
- allowed bounded binary reads of one safe-rooted regular file
- denied binary reads, including invalid arguments, missing/unsupported targets, safe-root rejection, raw-byte/response limits, and internal failures
- allowed bounded binary range reads of one safe-rooted regular file
- denied binary range reads, including invalid arguments/ranges, missing/unsupported targets, safe-root rejection, file/response limits, concurrent size change, and internal failures
- allowed bounded safe-rooted file reads
- denied read requests, including invalid arguments, safe-root rejections, and byte-limit failures
- allowed bounded code-point-safe UTF-8 range reads of one safe-rooted regular file
- denied UTF-8 range reads, including invalid arguments/ranges/encoding, missing/unsupported targets, safe-root rejection, file/response limits, concurrent size change, and internal failures
- allowed bounded safe-rooted literal text searches
- denied search requests, including invalid arguments/query/depth, safe-root rejection, response bounds, and internal failures
- allowed `write_file` dry-run previews
- allowed exact-grant-authorized `write_file` create and replace mutations
- denied write requests, including invalid arguments, disabled mutation, stable grant authorization failures, safe-root/type/disposition rejection, byte/response limits, namespace changes, and internal failures

The runtime should continue exposing aggregate counters only through the additive `runtime_status.structuredContent.auditCounters` snapshot.

## Non-goals

This contract does not add or imply:

- arbitrary shell access
- arbitrary command execution
- global process listing
- arbitrary environment-variable exposure
- Android platform control
- high-impact controls
- filesystem access outside configured safe roots
- persistent audit storage
- raw event streaming to clients

## Data-minimization requirements

Filesystem audit events and counters must remain low-cardinality and non-sensitive.

Audit events and counters must not store:

- raw filesystem paths
- file contents
- caller-provided arbitrary strings
- command output
- environment values
- host identifiers
- user identifiers
- service-specific private metadata
- bearer or capability secrets, principal fingerprints, session identifiers, JTIs, target digests, grant timestamps, or replay-state contents

Counters may store only stable tool names and stable reason codes. Event metadata, when used by future sinks, must be limited to bounded numeric values such as byte limits or argument counts. The in-memory `AuditCounters` implementation intentionally ignores event metadata and records only aggregate totals by tool and reason code.

## Tool and mode mapping

| Tool | Allowed mode | Denied mode | Gate name |
| --- | --- | --- | --- |
| `create_directory` with dry-run preview | `dry_run` | `dry_run` | `filesystem_write` |
| `create_directory` with explicit mutation | `mutating` | `mutating` | `filesystem_write` |
| `copy_file` with dry-run preview | `dry_run` | `dry_run` | `filesystem_write` |
| `copy_file` with explicit mutation | `mutating` | `mutating` | `filesystem_write` |
| `find_paths` | `read_only` | `read_only` | `filesystem_read` |
| `hash_file` | `read_only` | `read_only` | `filesystem_read` |
| `list_directory` | `read_only` | `read_only` | `filesystem_read` |
| `path_metadata` | `read_only` | `read_only` | `filesystem_metadata` |
| `read_binary_file` | `read_only` | `read_only` | `filesystem_read` |
| `read_binary_range` | `read_only` | `read_only` | `filesystem_read` |
| `read_file` | `read_only` | `read_only` | `filesystem_read` |
| `read_text_range` | `read_only` | `read_only` | `filesystem_read` |
| `search_text` | `read_only` | `read_only` | `filesystem_read` |
| `write_file` with dry-run preview | `dry_run` | `dry_run` | `filesystem_write` |
| `write_file` with grant-authorized mutation intent | `mutating` | `mutating` | `filesystem_write` |

A directory, copy, or file-write call is a dry-run preview unless `dry_run=false` resolves to mutation intent. Audit wiring must use the resolved mode, not merely the raw caller argument; the mutating mode does not assert that a gate or grant authorized the request.

For `create_directory` and `write_file`, mutating mode is only the requested posture. It does not imply authorization: each dedicated runtime gate and exact request grant is checked separately. A denied grant records the mutating mode and one stable `capability_*` reason only; successful grant consumption adds no secret or caller-derived label. Write counters also exclude content digest, create/replace disposition, staging name, file identity/mode/size, and cleanup/rollback target.

## Stable reason-code guidance

Reason codes should describe policy outcomes, not caller values.

Recommended allowed reason codes:

- `safe_root_listing`
- `safe_root_metadata_read`
- `safe_root_binary_read`
- `safe_root_binary_range_read`
- `safe_root_text_range_read`
- `safe_root_paths_found`
- `safe_root_read`
- `safe_root_text_searched`
- `safe_root_directory_created`
- `safe_root_file_copied`
- `safe_root_file_hashed`
- `dry_run_preview`
- `explicit_write_allowed`

Recommended denied reason codes:

- `missing_path_argument`
- `invalid_filesystem_arguments`
- `invalid_list_depth`
- `search_query_invalid`
- `filesystem_path_not_found`
- `filesystem_path_type_unsupported`
- `filesystem_parent_not_found`
- `filesystem_destination_exists`
- `filesystem_directory_create_failed`
- `create_directory_mutation_disabled`
- stable `capability_*` authorization reasons defined by [`CREATE_DIRECTORY_CAPABILITY_GRANTS.md`](CREATE_DIRECTORY_CAPABILITY_GRANTS.md)
- `filesystem_copy_source_not_found`
- `filesystem_copy_parent_not_found`
- `filesystem_copy_same_path`
- `filesystem_copy_source_type_unsupported`
- `filesystem_copy_source_too_large`
- `filesystem_copy_failed`
- `filesystem_binary_read_target_not_found`
- `filesystem_binary_read_type_unsupported`
- `filesystem_binary_read_size_limit_exceeded`
- `filesystem_binary_read_failed`
- `filesystem_binary_range_target_not_found`
- `filesystem_binary_range_type_unsupported`
- `filesystem_binary_range_invalid`
- `filesystem_binary_range_file_too_large`
- `filesystem_binary_range_changed_during_read`
- `filesystem_binary_range_failed`
- `filesystem_text_range_target_not_found`
- `filesystem_text_range_type_unsupported`
- `filesystem_text_range_invalid`
- `filesystem_text_range_file_too_large`
- `filesystem_text_range_encoding_invalid`
- `filesystem_text_range_changed_during_read`
- `filesystem_text_range_failed`
- `find_query_invalid`
- `filesystem_find_failed`
- `path_outside_safe_root`
- `read_byte_limit_exceeded`
- `write_byte_limit_exceeded`
- `filesystem_operation_failed`

The final runtime implementation may consolidate equivalent failures under fewer reason codes, but must keep codes stable, non-sensitive, and low-cardinality.

## Response-contract preservation

Audit counter wiring must not change existing JSON-RPC response shapes for `create_directory`, `copy_file`, `find_paths`, `hash_file`, `list_directory`, `path_metadata`, `read_binary_file`, `read_binary_range`, `read_file`, `read_text_range`, `search_text`, or `write_file`.

In particular, runtime wiring must preserve:

- current success text for creation, directory listings, and writes
- current `structuredContent` payloads for every filesystem tool
- current JSON-RPC error codes for invalid params, payload-too-large, and internal errors
- current safe-root rejection message
- current default-dry-run directory and file mutation behavior

`runtime_status` may continue exposing the additive `auditCounters` snapshot already present in the staged runtime.

## Implementation checklist

A focused runtime wiring PR should verify all of the following:

1. `create_directory` records allowed dry-run and authorized mutating decisions and denied gate/grant/missing/existing/boundary/failure decisions without retaining keys, grants, principal/session/root/target bindings, replay state, paths, or temporary-name data.
2. `copy_file` records allowed preview and explicit-copy decisions plus every stable copy-specific denial without retaining paths, bytes, request ids, source metadata, or temporary names.
3. `find_paths` records allowed and denied read-only decisions without retaining its root, matched paths, filenames, query, kind, request ID, filesystem identities, or raw errors.
4. `hash_file` records allowed and denied read-only decisions without retaining its path, filename, content, digest, byte count, file identity, partial state, or raw error.
5. `list_directory` records an allowed read-only filesystem event on successful safe-rooted listing.
6. `list_directory` records a denied read-only filesystem event for invalid arguments, invalid depth, safe-root rejection, and internal operation failure.
7. `read_binary_file` records allowed and denied read-only decisions without retaining its path, filename, raw or encoded content, byte count, file identity, request ID, or raw error.
8. `read_binary_range` records allowed and denied read-only decisions without retaining its path, filename, offset, requested/returned length, raw or encoded content, file size/identity, request ID, or raw error.
9. `read_file` records an allowed read-only filesystem event on successful bounded safe-rooted read.
10. `read_file` records a denied read-only filesystem event for invalid arguments, safe-root rejection, read byte-limit failure, and internal read failure.
11. `read_text_range` records allowed and denied read-only decisions without retaining its path, filename, offset, requested/returned size, text content, file size/identity, request ID, or raw error.
12. `write_file` records an allowed dry-run filesystem event for successful previews without consuming a grant.
13. `write_file` records an allowed mutating filesystem event only after one exact grant-authorized create or replace completes its durability and cleanup contract.
14. `write_file` records exactly one denied filesystem event using the resolved dry-run or mutating mode for invalid arguments, disabled gate, grant failure, write/response byte limit, safe-root/type/disposition rejection, namespace change, and internal failure; it retains no grant, content, digest, path, response ID, target/staging identity, or raw error.
15. `path_metadata` records allowed and denied read-only decisions without retaining its path, filename, kind, size, timestamp, or raw error.
16. `search_text` records allowed and denied read-only decisions without retaining its path, query, content, or match locations.
17. Tests assert counter increments by stable tool and reason-code labels without asserting or storing raw paths/content/digests/base64/text data.

## Security invariant

Filesystem audit counter wiring is observability-only. It must not broaden the documented filesystem authority model, weaken safe-root checks, add shell access, or create any high-impact control surface.
