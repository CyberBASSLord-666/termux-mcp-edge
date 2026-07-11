# Changelog

## Unreleased

- Hardened named Cloudflare Tunnel setup with required explicit arguments, strict DNS-label validation, exact authenticated JSON tunnel discovery, explicit `--create` authorization, zero-call dry-run, non-overwriting DNS behavior, private cleanup, and hermetic fake-`cloudflared` regression coverage.
- Made security- and network-relevant environment loading uniformly fail closed: only absent variables use defaults, present non-Unicode values are rejected without reflecting values, safe-root lists preserve exact entries and reject empties, and listener port `0` is rejected across runtime, transport helpers, and deployment validation.
- Added `scripts/termux_deploy.sh` as the canonical manager for project-owned, versioned Termux releases and the fixed `mcp_runtime` runit service.
- Added distinct install and upgrade modes, atomic `current`/`previous` activation, explicit rollback, status, configuration-preserving uninstall, and explicit configuration purge.
- Added strict root placement beneath `HOME` and `PREFIX`, path-segment validation, non-overlapping deployment/configuration roots, validated loopback probe URLs, and fixed service ownership boundaries.
- Added artifact validation for regular executable state, non-symlink input, bounded size, SHA-256, ELF architecture, and exact embedded package version.
- Added exact dependency-free binary `--version` and `--help` behavior with integration coverage for extra, unknown, and non-UTF-8 arguments.
- Added a deployment lock with live-owner rejection, stale-lock recovery, restrictive lock metadata, and cleanup traps for locks, staging directories, and temporary links.
- Added private `runtime.env` validation and a literal allowlisted `NAME=value` loader that does not evaluate configuration as program text.
- Added fail-closed authentication-posture checks for deployed configuration and rejected group/world-accessible, symlinked, malformed, or non-allowlisted configuration.
- Added exact deployment-state snapshots so a failed candidate restores prior `current`/`previous` links, removes the failed release, restarts the prior runtime, and verifies recovery.
- Added rollback recovery that restores and re-probes the original active release when the selected rollback target is unhealthy.
- Added invalid-link rejection for release targets that escape the project releases root or point to incomplete releases.
- Added dry-run validation without release, service, link, or lock mutation.
- Added fail-closed runit shutdown confirmation before activation, rollback, recovery, or uninstall; a failed or unconfirmed pre-activation stop leaves existing state untouched, removes only the newly published never-activated candidate, and permits a clean same-version retry.
- Added atomic run-file publication, pre-activation `down` gating, and complete staged service-directory publication so runit cannot observe a partial or prematurely startable service.
- Added service-state snapshots and interruption recovery for the canonical directory, `run` file, `down` marker, modes, and exact release links; failed first installation now removes all newly introduced service state.
- Expanded deployment tests for stop failure, uninstall preservation, atomic run-file cleanup, pre-start gating, failed initial-service cleanup, failed upgrade recovery, and failed rollback recovery.
- Added the atomic runit transition and operator validation contract in `docs/RUNIT_SERVICE_TRANSITIONS.md`.
- Added CI deployment tests covering verification failures, operation-mode enforcement, initial-install cleanup, failed-upgrade recovery, failed-rollback recovery, active/stale locks, unsafe roots, invalid links, literal configuration handling, secret non-disclosure, uninstall preservation, and explicit purge.
- Added canonical deployment, upgrade, rollback, recovery, validation, and on-device production-gate documentation.
- Updated CI path filters and validation to include deployment scripts and shell tests.
- Hardened the design-only command policy with explicit timeout/output lower bounds, bounded argv and environment-name cardinality, deterministic denial precedence, non-sensitive reason codes, and boundary regression coverage.
- Centralized Host and Origin authority normalization across startup configuration and request validation, rejecting ASCII whitespace/control characters, wildcard/userinfo/URL delimiters, malformed DNS/IPv4/bracketed-IPv6 forms, ambiguous colons, and invalid ports while preserving case-insensitive exact allowlist matching.
- Pinned CI and Android validation to Rust 1.88.0, verified the active toolchain and Android target, and made Android AArch64 validation build and publish separately named default and `mcp-runtime` feature postures for Rust source and toolchain changes.
- Enforced every advertised MCP tool input schema at runtime, centralized omitted-or-empty no-argument handling, replaced serde-derived public errors with stable bounded responses, and corrected write-payload limit mapping.
- Migrated `/mcp` to the stable MCP 2025-11-25 Streamable HTTP contract with initialize negotiation, pending/active lifecycle gating, strict POST media handling, protocol-version headers, compliant HTTP 202 notification/response handling, and explicit non-SSE GET 405 behavior.
- Added bounded UUID session management with 64-session capacity, 30-minute idle expiry, per-session isolation, DELETE termination, restart/reconnect semantics, and no retained client initialize metadata.
- Extended strict JSON-RPC classification to stable single-message client responses, rejected batch arrays and non-object MCP params, preserved authentication/Host/Origin/request-limit ordering, and added end-to-end transport conformance coverage.

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
