# Safe-rooted text search contract

`search_text` is a staged, read-only MCP filesystem capability compiled only with the existing `mcp-runtime` posture. It must not introduce a shell, regular expressions, glob expansion, caller-selected resource ceilings, or pathname re-resolution after descriptor validation.

## Request

The request schema is closed and contains only:

- `path`: absolute path beneath a configured safe root.
- `query`: non-empty literal UTF-8 text, bounded to 256 encoded bytes.
- `max_depth`: optional integer from 1 through 5; default 1.

Unknown fields are rejected. The query is matched case-sensitively as literal UTF-8 bytes. Empty queries, NUL bytes, relative paths, parent traversal, and paths outside configured safe roots fail closed.

## Filesystem traversal

Traversal starts from an open configured safe-root descriptor. Every descendant component is opened relative to its already-open parent with no-follow semantics. Symlinks and unsupported file types are skipped. Validated descendants are never reopened by pathname.

Blocking directory and file work runs through `spawn_blocking` so constrained Termux runtimes do not stall the async reactor.

## Fixed ceilings

The implementation owns these non-configurable limits:

- query bytes: 256
- traversal depth: 5
- directory entries examined: 4,096
- regular files examined: 1,024
- bytes read per file: 1 MiB
- aggregate bytes read: 16 MiB
- published matches: 1,024
- structured tool result: 256 KiB

Reaching any traversal, file, byte, match, or response ceiling returns the successfully collected deterministic prefix with `truncated: true`; it must not continue scanning after truncation is known.

## Result

Each match contains only:

- safe-rooted file path
- one-based line number
- one-based byte column

The response also publishes the fixed ceilings, scan counters, and `truncated`. It never echoes query text, matching line content, raw file bytes, or raw operating-system errors. Results are sorted by path, line, then byte column independently of filesystem enumeration order.

Invalid UTF-8 files, oversized files, unreadable entries, symlinks, sockets, devices, and other unsupported types are skipped using stable aggregate counters.

## Audit and metrics privacy

Audit records and metric labels contain only stable tool, outcome, and reason identifiers. They must never retain paths, query text, file content, match data, filenames, or raw operating-system errors.

Required stable outcomes include success, truncated, invalid-input, path-rejected, and internal-error. Skip counters remain aggregate and low-cardinality.

## Verification requirements

Before review readiness, the implementation must include focused coverage for:

- path traversal and symlink rejection at every directory level
- symlink-swap resistance using descriptor-relative access
- literal matching and one-based byte-column semantics for multibyte UTF-8
- deterministic ordering independent of directory enumeration order
- every fixed traversal, file, byte, match, and response ceiling
- invalid UTF-8, oversized, unreadable, and unsupported-file skipping
- bounded memory behavior and async-reactor isolation
- audit/metrics privacy and low-cardinality labels
- MCP schema closure, dispatch, response-size enforcement, and unknown-tool behavior
- native ARM64 Termux validation in the existing Android workflow posture

Release evidence for this runtime-changing capability remains ineligible until fresh physical-device qualification is captured under the repository release-evidence contract.
