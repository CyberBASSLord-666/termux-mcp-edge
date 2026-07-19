# MCP Runtime Roadmap

## Goal

Move from the health-check runtime to a broader MCP runtime for developers, advanced Termux operators, and power users without regressing security, CI, dependency posture, staged review discipline, or documentation accuracy.

This roadmap assumes informed operators who understand local automation risk. The project therefore uses explicit capability gates, opt-in configuration, allowlists, dry-run or preview behavior where useful, and audit coverage for higher-risk surfaces instead of permanently withholding advanced functionality.

## Current Baseline

`main` exposes the health-check runtime by default. The optional `mcp-runtime` feature exposes stable MCP 2025-11-25 Streamable HTTP handling at `/mcp`, exact transport security, bounded sessions, and twelve baseline tools: deterministic runtime/platform/Android/project-service metadata, preview-first and independently grant-gated single-directory creation, dry-run-first bounded binary file copy, streaming bounded SHA-256 file hashing, safe-rooted listing, descriptor-relative single-object metadata, bounded UTF-8 reads, bounded literal text search, and dry-run-first file writes.

The transport implements POST media negotiation, single request/notification/response classification, initialized gating, the subsequent-request protocol header, HTTP 202 notification/response semantics, and DELETE session termination. GET returns HTTP 405 as permitted when a server does not offer optional SSE. SSE, replay, and resumability are deliberately absent rather than partially implemented.

The staged runtime also includes in-memory non-sensitive audit counters for current tool decisions. Separate `android-battery-status` and `android-volume-status` compile features plus disabled-by-default runtime flags expose bounded read-only Termux:API telemetry. A separate `android-volume-control` posture now exposes only preview-first exact-stream mutation with static authentication, an exact single-use request grant, fixed execution, verification, and restoration. A separate `command-execution` feature and runtime flag expose only three fixed read-only diagnostics of the exact server binary. General high-impact capability-token primitives remain inert policy scaffolding. Broader Android control, shell fallback, arbitrary command execution, process inventory, arbitrary service inspection, service mutation/control, package management, network mutation, and unrelated high-impact actions remain unavailable until their own power-user capability gates land.

## Capability-Gate Philosophy

- Advanced capabilities are acceptable project goals when they are explicit, documented, and independently validated.
- Defaults stay narrow so accidental exposure is unlikely.
- Power-user expansion happens through opt-in configuration, feature gates, allowlists, bounded inputs/outputs, and audit events.
- Riskier tools should fail closed with clear structured errors rather than silently degrading into broad shell or platform access.
- A capability being disabled today means its runtime gate has not landed yet; it does not mean the capability is out of scope forever.

## Stage 1: Transport Request Validation

Add reusable `Host` and `Origin` validation primitives with unit coverage. No routes are exposed in this stage.

Status: complete.

Required gates:

- Exact-head CI success.
- Exact-head Security success.
- No new dependency surface.
- No runtime exposure.

## Stage 2: Minimal MCP Transport Shell

Introduce the smallest MCP transport runtime without filesystem, platform-control, command, or high-impact tools.

Status: complete.

Required gates:

- Exact-head CI success.
- Exact-head Security success.
- Dependency alerts clear after merge.
- `Host` and `Origin` validation enforced on browser-reachable transport routes.
- Bearer-token behavior preserved for non-local access paths.
- Smoke test proves transport liveness.

## Stage 3: Tool Discovery Contract

Expose a low-risk staged tool registry and prove tool discovery behavior.

Status: complete.

Required gates:

- Tool discovery smoke test.
- No filesystem write behavior in this stage.
- No platform automation behavior.
- No command execution behavior.

## Stage 4: Low-Risk Read-Only Tools

Add low-risk read-only tools with deterministic or tightly allowlisted output and tests.

Status: complete for `runtime_status`, `platform_info`, `android_status`, and `project_service_status`.

Required gates:

- Tool call smoke tests.
- Tool output schemas documented.
- No broad filesystem access from read-only metadata tools.
- No Android API calls, identifiers, shell fallback, or control behavior from Android status metadata.
- No arbitrary service inspection, process inventory, or service mutation/control from project service status metadata.

## Stage 5: Filesystem Tools

Restore filesystem capability with narrow safe roots, read/write separation, payload limits, explicit write controls, and non-sensitive audit counter coverage.

Status: exposed behind the staged runtime gate. The surface includes preview-first one-directory creation, dry-run-first one-file binary copy, bounded streaming SHA-256 file hashing, deterministic response-bounded listing, bounded single-object metadata, bounded UTF-8 reads, bounded literal text-location search, dry-run-first file writes, explicit crash-durable mutations, and non-sensitive audit counters. Directory mutation is separately default-disabled and requires a 60-second, principal/session/root/target-bound, single-use header grant issued offline by the exact binary. Live operations retain exact safe-root, source, and destination-parent descriptors through classification, bounded reads, staging, no-replace publication, cleanup, and durability sync. File copy is capped at 1 MiB, fixes destinations to mode `0600`, returns no content, and preflights its complete 16 KiB response before mutation. Hashing is capped at 16 MiB, retains one exact no-follow descriptor, returns only SHA-256 and bytes hashed, and preflights its complete 16 KiB response before reading. The response-bound work landed through #206, descriptor-relative race hardening through #200, bounded search through #240, bounded path metadata through #242, bounded directory creation through #244, bounded file copy through #247, directory request authorization through #248, and bounded hashing through #261.

Required gates:

- Safe-root traversal tests.
- Symlink escape tests.
- Default-disabled, missing/malformed/mismatched/expired/future/replay/concurrent grant tests plus dry-run non-consumption and explicit one-directory creation tests with fixed mode, existing-target denial, atomic no-replace publication, post-consumption failure semantics, and identity-checked cleanup.
- Dry-run and explicit binary file-copy tests with exact-limit enforcement, fixed mode, absent-destination/no-replace behavior, descriptor exchange resistance, response preflight, identity-safe cleanup, and content-private audit counters.
- Binary and empty-file SHA-256 tests with exact 16 MiB acceptance, one-byte-over rejection, descriptor exchange resistance, response preflight, runtime growth enforcement, and digest/path/content-private audit counters.
- Read-only directory listing test.
- Bounded path-metadata test with identifier and unsupported-type redaction.
- Bounded read-file test.
- Dry-run write test.
- Explicit mutation write test with safe-root and payload constraints.
- Oversize and exact-limit payload tests at direct-tool and MCP-transport boundaries.
- Audit counter coverage for allowed and denied filesystem decisions.
- Documentation of the filesystem audit counter contract in [`filesystem-audit-counter-contract.md`](filesystem-audit-counter-contract.md).
- Runtime audit counter documentation in [`runtime-audit-counters.md`](runtime-audit-counters.md).
- Documentation of operator assumptions through the staged [`operator-validation.md`](operator-validation.md) checklist.

## Protocol Completion Track: Stable MCP 2025-11-25

Implement the stable MCP 2025-11-25 lifecycle and Streamable HTTP contract without expanding tool authority in the same change.

Status: complete for the non-SSE Streamable HTTP posture. Optional server-initiated SSE, replay, and resumability remain unimplemented and must use a separate gate if later required.

Required gates:

- Initialization is the first client/server interaction and negotiated state gates normal operation.
- The single MCP endpoint implements required POST and GET behavior, including explicit JSON/SSE media negotiation and the specification-permitted GET 405 response when SSE is unavailable.
- Requests after initialization enforce the `MCP-Protocol-Version` header contract.
- Notification HTTP behavior and JSON-RPC no-response semantics conform to the stable transport.
- Session support uses cryptographically random UUIDs, bounded capacity, idle expiry, independent lifecycle state, and explicit DELETE termination.
- Cancellation notifications, timeout cleanup, shutdown/reset, 404 reinitialization, and multiple-client isolation are covered; there is no cross-request operation registry or SSE resumption state.
- Existing authentication, Host/Origin, resource, tool-authorization, and audit boundaries remain intact.
- Compatibility claims and operator validation cite the exact implemented protocol revision.

## Stage 6: Android Platform Tools

Restore Android platform tools only after explicit feature gates and operational documentation. Read-only `android_status` metadata is already complete and does not authorize this stage.

Status: read-only `android_battery_status` and `android_volume_status` are implemented behind independent compile-time and runtime gates. Preview-first `set_android_volume` is also implemented behind an independent compile gate, default-disabled runtime gate, static authentication, exact request grant, fixed execution, non-queueing mutation lane, fresh bounds, result verification, and confirmed recovery on failure. Broader Android/platform control is not started.

Required gates:

- Feature-gated compile path.
- Runtime disabled-by-default behavior.
- Tool-level smoke tests or documented manual validation.
- No shell fallback unless separately reviewed and authorized.
- Operator-facing documentation that clearly distinguishes read-only status from device-control actions.
- Capability and audit policy appropriate to each Android data/control family.

Battery telemetry satisfies this stage only for its read-only data family: fixed executable, no caller arguments, cleared environment, bounded normal operation/output, strict field normalization, disabled discovery, stable error codes, aggregate audit coverage, a cancellation-safe process-group supervisor with authoritative late-reap failure handling, and native ARM64 official-Termux cleanup validation. It does not satisfy or authorize any future Android control family.

Volume telemetry satisfies this stage only for zero-argument status: fixed `termux-volume` execution, no caller-selected stream or level, cleared environment, bounded output/time, an exact six-stream parser with canonical order, disabled discovery, stable errors, aggregate audit coverage, the shared hardened provider supervisor, and a dedicated native ARM64 evidence report. It does not authorize the upstream command's argument-taking mutation mode, audio routing, media control, or any broader Android control family.

Volume control satisfies this stage only for the exact six allowlisted streams and an integer level inside a fresh live maximum. Preview never invokes the setter. Mutation requires an exact single-use grant, one non-queueing permit, the fixed `termux-volume <stream> <level>` execution, post-set verification, and restoration to the captured prior level when setter or verification fails. It does not authorize audio routing, media control, arbitrary Termux:API calls, or broader Android control.

## Stage 7: Fixed Command Diagnostics and High-Impact Tooling

Add command execution or high-impact tooling only after separate authorization, audit/logging, and operator-consent policy is in place.

Status: the first fixed read-only server-diagnostic slice is implemented. Arbitrary, parameterized, mutating, and high-impact command surfaces are not started.

Completed prerequisites:

- Command-execution gate design and profile-review runbook.
- Separate compile-time feature and disabled-by-default runtime flag.
- Closed `run_command_profile` schema with three fixed current-executable profiles.
- Safe-root cwd, empty environment, null stdin, independent stream bounds, hard deadline, two non-queueing permits, process-group cleanup, and authoritative reaping.
- Stable non-sensitive failures and aggregate allowed/denied audit coverage.
- Exact-source fifth Android artifact with deterministic native ARM64 official-Termux evidence.
- High-impact controls threat model.
- Inert capability-token policy primitives with no token issuance, persistence, or live authorization surface.
- Backend-neutral audit event/counter primitives and capability-policy audit contract tests.
- Operator-facing validation and audit-counter documentation.

Required before any expansion beyond fixed diagnostics:

- Capability-token/confirmation model for high-impact actions.
- A separate gate for any new executable, placeholder, caller input, mutation, credential, or broad inspection authority.
- Separate focused validation PR for each tool family.
- Regression tests proving disabled-by-default behavior and no accidental MCP discovery.
- Rollback/cleanup behavior for mutating or long-running actions.
- Security review when dependencies, workflows, or security-relevant configuration change.

## Non-Goals

- Do not merge PRs that restore all runtime surfaces at once.
- Do not bundle dependency updates with unrelated behavior changes.
- Do not treat `project_service_status` as arbitrary service discovery or process inspection.
- Do not treat read-only Android/Termux status metadata as Android platform control.
- Do not treat fixed server diagnostics as arbitrary command execution or authorization for a broader executable surface.
- Do not treat inert capability-token policy as live high-impact authorization.
- Do not claim broad MCP production readiness without stable protocol conformance and tool smoke tests for each enabled surface.
