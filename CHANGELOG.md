# Changelog

## Unreleased — Reproducible Termux Deployment

- Added a dependency-light deployment manager for project-owned, versioned Termux installations.
- Added atomic install and upgrade activation with retained previous-release state and explicit rollback.
- Added automatic restoration of the previous release when candidate health or readiness validation fails.
- Added project-scoped runit service generation, persistent configuration separation, restrictive directory permissions, architecture checks, idempotency guards, dry-run support, and deterministic uninstall behavior.
- Added isolated shell tests covering install, duplicate refusal, upgrade, rollback, unsafe path/version rejection, secret non-disclosure, configuration preservation, and purge behavior.
- Added CI execution for deployment shell tests and path triggers for deployment scripts.
- Added a complete operator guide for install, upgrade, rollback, status, recovery, uninstall, cross-compiled artifacts, and on-device validation.

## 2026-07-10 — v0.5.1 Staged MCP Runtime and Audit Hardening

- Restored the optional `mcp-runtime` transport in independently validated stages while keeping the default build limited to operational health/readiness endpoints.
- Enforced configured bearer authentication on the complete `/mcp` route before request-limit accounting, Host/Origin validation, JSON-RPC parsing, tool discovery, or tool invocation; retained only explicit loopback-development bypass behavior.
- Added HTTP 401 authentication failures with `WWW-Authenticate: Bearer`, `Cache-Control: no-store`, bounded credential parsing, fixed-work token comparison, and token redaction in debug/error paths.
- Added mobile-conscious MCP request ceilings: four concurrent authenticated requests, a 30-second total request timeout, and a 2 MiB body limit by default, with validated operator override ranges.
- Added non-sensitive HTTP 413, 503, and 504 limit responses, fail-fast concurrency saturation, streaming body enforcement without double buffering, and active-limit readiness metadata.
- Added cancellation-safe same-directory temporary-file cleanup so timed-out or cancelled writes do not strand `.tmp` artifacts.
- Added exact `Host` and browser `Origin` validation before MCP request dispatch.
- Added staged discovery and tool-call coverage for `runtime_status`, `platform_info`, `android_status`, `project_service_status`, `list_directory`, `read_file`, and `write_file`.
- Kept Android platform control, shell fallback, arbitrary command execution, global process inspection, arbitrary service inspection, service mutation/control, and high-impact actions disabled.
- Added bounded safe-rooted directory listing and UTF-8 reads, default-dry-run writes, explicit safe-rooted mutation, atomic temporary-file replacement, and payload-size enforcement.
- Added traversal, symlink-boundary, oversize, exact-limit, dry-run, explicit-mutation, MCP-transport, request-limit, authentication-ordering, timeout, saturation, and temp-cleanup tests.
- Corrected JSON-RPC handling so syntactically valid requests missing `method` return `-32600 Invalid Request` while malformed JSON remains `-32700 Parse error`.
- Added backend-neutral audit events and in-memory aggregate counters for staged status and filesystem decisions without retaining paths, contents, secrets, tokens, environment values, or caller strings.
- Added inert fixed-allowlist command-policy primitives and inert high-impact capability-token policy primitives without process spawning or live authorization exposure.
- Added command-execution and high-impact-controls gate documentation, capability/audit contracts, and operator validation guidance.
- Hardened setup and cross-compilation scripts for strict error handling, validation, cleanup, and repeatable operation.
- Pinned GitHub Actions inputs to immutable commits, standardized deterministic runners, and bounded workflow execution with timeouts.
- Synchronized README, security, validation, contribution, roadmap, operations, and release documentation with the current staged runtime and CI contract.

## 2026-06-26 — Authentication Posture Hardening

- Changed startup authentication behavior to fail closed when `MCP__AUTH__STATIC_TOKEN` is missing.
- Rejected empty or whitespace-only `MCP__AUTH__STATIC_TOKEN` values at startup.
- Added explicit `MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=true` local-development opt-in, constrained to loopback bind addresses.
- Allowed `localhost` as an explicit loopback hostname for local-only unauthenticated development.
- Documented the unauthenticated-mode constraints in `README.md` and `docs/SECURITY.md`.

## 2026-06-26 — Automated Filesystem Safety Pass

- Reworked `FileSystemTools::sanitize` into a public, testable safe-root guard that rejects relative paths, NUL bytes, explicit parent-directory components, and paths escaping configured roots.
- Replaced recursive async directory traversal with bounded iterative breadth-first traversal to avoid Rust async recursion compile failures and reduce stack/future-size risk.
- Added list traversal bounds for maximum depth and maximum entries, plus metrics for truncated and unsafe skipped entries.
- Hardened atomic writes by staging temporary files in the destination directory and cleaning up failed temporary writes or renames.
- Added real integration tests for dry-run writes, atomic writes, reads, directory listing, and traversal rejection.
- Added property tests for safe-root path acceptance and rejection behavior.
- Added `proptest` and `tempfile` dev-dependencies and corrected crate repository metadata.
- Added `docs/VALIDATION.md` with validation commands and current run limitations.

## 2026-06-25 — Repository Baseline

- Created canonical Git repository layout for the Termux MCP Rust/Android edge server.
- Added repository hygiene files: `.gitignore`, `.gitattributes`, `.editorconfig`, license, contribution guide, GitHub Actions, Dependabot, and operational documentation.
- Preserved the uploaded Rust MCP server source tree as the initial baseline.
- Added validation and deployment instructions for desktop Rust validation and Android cross-compilation.
