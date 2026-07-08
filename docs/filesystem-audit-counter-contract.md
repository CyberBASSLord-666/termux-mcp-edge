# Filesystem audit counter contract

This document defines the staged runtime contract for counting filesystem tool decisions without capturing sensitive caller data.

The contract applies to the existing staged filesystem tools:

- `list_directory`
- `read_file`
- `write_file`

It builds on the backend-neutral audit helpers in `src/audit.rs`, including `filesystem_allowed_event`, `filesystem_denied_event`, `AuditMode`, and `AuditCounters`.

## Goals

Filesystem audit counter wiring should make operator-visible runtime counters useful while preserving the current staged security posture.

The runtime should count:

- allowed safe-rooted directory listings
- denied directory-listing requests, including invalid arguments and safe-root rejections
- allowed bounded safe-rooted file reads
- denied read requests, including invalid arguments, safe-root rejections, and byte-limit failures
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

Counters may store only stable tool names and stable reason codes. Event metadata, when used by future sinks, must be limited to bounded numeric values such as byte limits or argument counts. The in-memory `AuditCounters` implementation intentionally ignores event metadata and records only aggregate totals by tool and reason code.

## Tool and mode mapping

| Tool | Allowed mode | Denied mode | Gate name |
| --- | --- | --- | --- |
| `list_directory` | `read_only` | `read_only` | `filesystem_safe_root` |
| `read_file` | `read_only` | `read_only` | `filesystem_safe_root` |
| `write_file` with dry-run preview | `dry_run` | `dry_run` | `filesystem_write` |
| `write_file` with explicit mutation | `mutating` | `mutating` | `filesystem_write` |

A write call is a dry-run preview unless the existing write policy resolves `dry_run=false` to an explicit mutation. Audit wiring must use the resolved mode, not merely the raw caller argument.

## Stable reason-code guidance

Reason codes should describe policy outcomes, not caller values.

Recommended allowed reason codes:

- `safe_root_listing`
- `safe_root_read`
- `dry_run_preview`
- `explicit_write_allowed`

Recommended denied reason codes:

- `missing_path_argument`
- `invalid_filesystem_arguments`
- `invalid_list_depth`
- `path_outside_safe_root`
- `read_byte_limit_exceeded`
- `write_byte_limit_exceeded`
- `filesystem_operation_failed`

The final runtime implementation may consolidate equivalent failures under fewer reason codes, but must keep codes stable, non-sensitive, and low-cardinality.

## Response-contract preservation

Audit counter wiring must not change existing JSON-RPC response shapes for `list_directory`, `read_file`, or `write_file`.

In particular, runtime wiring must preserve:

- current success text for directory listings and writes
- current `structuredContent` payloads for all three tools
- current JSON-RPC error codes for invalid params, payload-too-large, and internal errors
- current safe-root rejection message
- current default-dry-run write behavior

`runtime_status` may continue exposing the additive `auditCounters` snapshot already present in the staged runtime.

## Implementation checklist

A focused runtime wiring PR should verify all of the following:

1. `list_directory` records an allowed read-only filesystem event on successful safe-rooted listing.
2. `list_directory` records a denied read-only filesystem event for invalid arguments, invalid depth, safe-root rejection, and internal operation failure.
3. `read_file` records an allowed read-only filesystem event on successful bounded safe-rooted read.
4. `read_file` records a denied read-only filesystem event for invalid arguments, safe-root rejection, read byte-limit failure, and internal read failure.
5. `write_file` records an allowed dry-run filesystem event for successful dry-run previews.
6. `write_file` records an allowed mutating filesystem event for successful explicit writes.
7. `write_file` records denied filesystem events using the resolved dry-run or mutating mode for invalid arguments, write byte-limit failure, safe-root rejection, and internal write failure.
8. Tests assert counter increments by stable tool and reason-code labels without asserting or storing raw paths/content.

## Security invariant

Filesystem audit counter wiring is observability-only. It must not broaden the filesystem authority model, weaken safe-root checks, add new tools, expose new arguments, add shell access, or create any high-impact control surface.
