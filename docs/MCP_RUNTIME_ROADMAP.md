# MCP Runtime Roadmap

## Goal

Move from the health-check runtime to a broader MCP runtime for developers, advanced Termux operators, and power users without regressing security, CI, dependency posture, staged review discipline, or documentation accuracy.

This roadmap assumes informed operators who understand local automation risk. The project therefore uses explicit capability gates, opt-in configuration, allowlists, dry-run or preview behavior where useful, and audit coverage for higher-risk surfaces instead of permanently withholding advanced functionality.

## Current Baseline

`main` exposes the health-check runtime by default. The optional `mcp-runtime` feature exposes stable MCP 2025-11-25 Streamable HTTP handling at `/mcp`, validates exact `Host` and browser `Origin` values before protocol handling, negotiates initialize state, scopes lifecycle to bounded UUID sessions, and exposes `tools/list` plus deterministic read-only `runtime_status`, non-sensitive read-only `platform_info`, read-only allowlisted `android_status`, safe-rooted directory listing, bounded safe-rooted UTF-8 file reads, default-dry-run safe-rooted file writes, and read-only allowlisted `project_service_status` for project-owned logical service state.

The transport implements POST media negotiation, single request/notification/response classification, initialized gating, the subsequent-request protocol header, HTTP 202 notification/response semantics, and DELETE session termination. GET returns HTTP 405 as permitted when a server does not offer optional SSE. SSE, replay, and resumability are deliberately absent rather than partially implemented.

The staged runtime also includes in-memory non-sensitive audit counters for current tool decisions. Command-policy and high-impact capability-token modules are inert policy scaffolding only. Android platform control, shell fallback, live command execution, process inventory, arbitrary service inspection, service mutation/control, package management, network mutation, and high-impact actions remain unavailable until their own power-user capability gates land.

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

Status: exposed behind the staged runtime gate. The surface includes deterministic response-bounded directory listing, bounded UTF-8 reads, default-dry-run writes, explicit crash-durable writes, and non-sensitive audit counters. Live operations are anchored to opened safe-root descriptors, reject symlink components, and retain the same descriptor through enumeration, read, temporary-file creation, rename, cleanup, and parent sync. The response-bound work landed through #206 and descriptor-relative race hardening landed through #200.

Required gates:

- Safe-root traversal tests.
- Symlink escape tests.
- Read-only directory listing test.
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

Status: not started for control-oriented Android/platform tools.

Required gates:

- Feature-gated compile path.
- Runtime disabled-by-default behavior.
- Tool-level smoke tests or documented manual validation.
- No shell fallback unless separately reviewed and authorized.
- Operator-facing documentation that clearly distinguishes read-only status from device-control actions.
- Capability and audit policy appropriate to each Android data/control family.

## Stage 7: Command Execution and High-Impact Tooling

Add command execution or high-impact tooling only after separate authorization, audit/logging, and operator-consent policy is in place.

Status: design and inert policy scaffolding complete; live runtime execution and high-impact tool exposure are not started.

Completed prerequisites:

- Command-execution gate design.
- Fixed allowlist and bounded command-policy primitives with no process spawning.
- High-impact controls threat model.
- Inert capability-token policy primitives with no token issuance, persistence, or live authorization surface.
- Backend-neutral audit event/counter primitives and capability-policy audit contract tests.
- Operator-facing validation and audit-counter documentation.

Required before live implementation:

- Dedicated compile-time feature gate.
- Runtime disabled-by-default configuration.
- Explicit operator opt-in.
- Fixed allowlisted command shapes; no arbitrary shell string execution.
- Bounded timeout, argv, stdout, stderr, working-directory, and environment policy.
- Capability-token/confirmation model for high-impact actions.
- Audit event integration for every allowed and denied invocation.
- Separate focused validation PR for each tool family.
- Regression tests proving disabled-by-default behavior and no accidental MCP discovery.
- Rollback/cleanup behavior for mutating or long-running actions.
- Security review when dependencies, workflows, or security-relevant configuration change.

## Non-Goals

- Do not merge PRs that restore all runtime surfaces at once.
- Do not bundle dependency updates with unrelated behavior changes.
- Do not treat `project_service_status` as arbitrary service discovery or process inspection.
- Do not treat read-only Android/Termux status metadata as Android platform control.
- Do not treat inert command/capability policy modules as live execution or authorization.
- Do not claim broad MCP production readiness without stable protocol conformance and tool smoke tests for each enabled surface.
