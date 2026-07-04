# MCP Runtime Roadmap

## Goal

Move from the conservative health-check runtime to a broader MCP runtime without regressing security, CI, dependency posture, staged review discipline, or documentation accuracy.

## Current Baseline

`main` exposes the health-check runtime by default. The optional `mcp-runtime` feature is being restored in narrow stages. The current staged transport validates exact `Host` and browser `Origin` values before handling `/mcp`, supports `initialize`, exposes `tools/list`, and adds deterministic read-only `runtime_status`, non-sensitive read-only `platform_info`, read-only allowlisted `android_status`, safe-rooted directory listing, bounded safe-rooted UTF-8 file reads, default-dry-run safe-rooted file writes, and read-only allowlisted `project_service_status` for project-owned logical service state. Android platform control, shell fallback, command execution, process inventory, arbitrary service inspection, service mutation/control, and high-impact actions remain unavailable.

## Stage 1: Transport Request Validation

Add reusable `Host` and `Origin` validation primitives with unit coverage. No routes are exposed in this stage.

Status: complete.

Required gates:

- Exact-head CI success.
- Exact-head Security success.
- No new dependency surface.
- No runtime exposure.

## Stage 2: Minimal MCP Transport Shell

Introduce the smallest MCP transport runtime without filesystem, platform, command, or high-impact tools.

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

Restore filesystem capability with narrow safe roots, read/write separation, payload limits, and explicit write controls.

Status: in progress. Current substages expose safe-rooted directory listing, bounded safe-rooted UTF-8 file reads, and default-dry-run safe-rooted file writes. The write surface remains constrained by safe-root and payload-size validation and requires explicit `dry_run:false` for mutation.

Required gates:

- Safe-root traversal tests.
- Symlink escape tests.
- Read-only directory listing test.
- Bounded read-file test.
- Dry-run write test.
- Explicit mutation write test with safe-root and payload constraints.
- Documentation of operator assumptions.

## Stage 6: Android Platform Tools

Restore Android platform tools only after explicit feature gates and operational documentation. Read-only `android_status` metadata is already complete and does not authorize this stage.

Status: not started for control-oriented Android/platform tools.

Required gates:

- Feature-gated compile path.
- Runtime disabled-by-default behavior.
- Tool-level smoke tests or documented manual validation.
- No shell fallback unless separately reviewed and authorized.

## Stage 7: Command Execution and High-Impact Tooling

Add command execution or high-impact tooling only after separate authorization, audit/logging, and operator-consent policy is in place.

Status: not started.

Required gates:

- Feature-gated compile path.
- Explicit operator opt-in.
- Audit/logging assumptions documented.
- Separate validation PR.
- Regression tests proving disabled-by-default behavior.

## Non-Goals

- Do not merge PRs that restore all runtime surfaces at once.
- Do not bundle dependency updates with unrelated behavior changes.
- Do not treat `project_service_status` as arbitrary service discovery or process inspection.
- Do not treat read-only Android/Termux status metadata as Android platform control.
- Do not claim broad MCP production readiness without transport and tool smoke tests for each enabled surface.
