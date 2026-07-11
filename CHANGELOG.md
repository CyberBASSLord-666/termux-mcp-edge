# Changelog

## Unreleased

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
- Added fail-closed runit shutdown confirmation before activation, rollback, recovery, or uninstall; a failed or unconfirmed stop leaves existing state untouched and removes only the newly published, never-activated candidate release.
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
