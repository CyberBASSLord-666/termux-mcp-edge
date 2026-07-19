# Filesystem audit counter contract

This document defines the staged runtime contract for counting filesystem tool decisions without capturing sensitive caller data.

The contract applies to the existing staged filesystem tools:

- `create_directory`
- `copy_file`
- `hash_file`
- `list_directory`
- `path_metadata`
- `read_file`
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
- denied directory-listing requests, including invalid arguments and safe-root rejections
- allowed bounded metadata reads for one safe-rooted regular file or directory
- denied metadata requests, including invalid arguments, missing objects, unsupported types, safe-root rejections, response bounds, and internal failures
- allowed bounded safe-rooted file reads
- denied read requests, including invalid arguments, safe-root rejections, and byte-limit failures
- allowed bounded safe-rooted literal text searches
- denied search requests, including invalid arguments/query/depth, safe-root rejection, response bounds, and internal failures
- allowed `write_file` dry-run previews
- allowed explicit `write_file` mutations
- denied write requests, including invalid arguments, safe-root rejections, and byte-limit failures

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
| `hash_file` | `read_only` | `read_only` | `filesystem_read` |
| `list_directory` | `read_only` | `read_only` | `filesystem_read` |
| `path_metadata` | `read_only` | `read_only` | `filesystem_metadata` |
| `read_file` | `read_only` | `read_only` | `filesystem_read` |
| `search_text` | `read_only` | `read_only` | `filesystem_read` |
| `write_file` with dry-run preview | `dry_run` | `dry_run` | `filesystem_write` |
| `write_file` with explicit mutation | `mutating` | `mutating` | `filesystem_write` |

A directory or file mutation call is a dry-run preview unless `dry_run=false` resolves to an explicit mutation. Audit wiring must use the resolved mode, not merely the raw caller argument.

For `create_directory`, mutating mode is only the requested posture. It does not imply authorization: the dedicated runtime gate and exact request grant are checked separately. A denied grant records the mutating mode and one stable `capability_*` reason only; successful grant consumption adds no secret or caller-derived label.

## Stable reason-code guidance

Reason codes should describe policy outcomes, not caller values.

Recommended allowed reason codes:

- `safe_root_listing`
- `safe_root_metadata_read`
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
- `path_outside_safe_root`
- `read_byte_limit_exceeded`
- `write_byte_limit_exceeded`
- `filesystem_operation_failed`

The final runtime implementation may consolidate equivalent failures under fewer reason codes, but must keep codes stable, non-sensitive, and low-cardinality.

## Response-contract preservation

Audit counter wiring must not change existing JSON-RPC response shapes for `create_directory`, `copy_file`, `hash_file`, `list_directory`, `path_metadata`, `read_file`, `search_text`, or `write_file`.

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
3. `hash_file` records allowed and denied read-only decisions without retaining its path, filename, content, digest, byte count, file identity, partial state, or raw error.
4. `list_directory` records an allowed read-only filesystem event on successful safe-rooted listing.
5. `list_directory` records a denied read-only filesystem event for invalid arguments, invalid depth, safe-root rejection, and internal operation failure.
6. `read_file` records an allowed read-only filesystem event on successful bounded safe-rooted read.
7. `read_file` records a denied read-only filesystem event for invalid arguments, safe-root rejection, read byte-limit failure, and internal read failure.
8. `write_file` records an allowed dry-run filesystem event for successful dry-run previews.
9. `write_file` records an allowed mutating filesystem event for successful explicit writes.
10. `write_file` records denied filesystem events using the resolved dry-run or mutating mode for invalid arguments, write byte-limit failure, safe-root rejection, and internal write failure.
11. `path_metadata` records allowed and denied read-only decisions without retaining its path, filename, kind, size, timestamp, or raw error.
12. `search_text` records allowed and denied read-only decisions without retaining its path, query, content, or match locations.
13. Tests assert counter increments by stable tool and reason-code labels without asserting or storing raw paths/content/digests.

## Security invariant

Filesystem audit counter wiring is observability-only. It must not broaden the documented filesystem authority model, weaken safe-root checks, add shell access, or create any high-impact control surface.
