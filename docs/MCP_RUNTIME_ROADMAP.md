# MCP Runtime Roadmap

## Goal

Move from the current conservative health-check runtime to a full MCP runtime without regressing security, CI, dependency posture, or documentation accuracy.

## Current Baseline

`main` exposes the health-check runtime by default. The optional `mcp-runtime` feature is being restored in narrow stages. The current staged transport shell validates exact `Host` and browser `Origin` values before handling `/mcp`, supports `initialize`, exposes `tools/list`, and adds deterministic read-only `runtime_status`, non-sensitive read-only `platform_info`, read-only allowlisted `android_status`, safe-rooted read-only directory listing, bounded safe-rooted UTF-8 file reads, and safe-rooted `write_file` with dry-run-by-default behavior. Mutating writes require explicit `"dry_run": false`. Project-owned `service_status` primitives are present as data-only code for the allowlisted `mcp_runtime` service, but they are not yet exposed through MCP. Android platform APIs/control tools, process inspection, command execution, and high-impact actions remain unavailable.

## Stage 1: Transport Request Validation

Add reusable `Host` and `Origin` validation primitives with unit coverage. No routes are exposed in this stage.

Status: complete.

Required gates:

- Exact-head CI success.
- Exact-head Security success.
- No new dependency surface.
- No runtime exposure.

## Stage 2: Minimal MCP Transport Shell

Introduce the smallest MCP transport runtime without filesystem, platform, or high-impact tools.

Status: complete.

Required gates:

- Exact-head CI success.
- Exact-head Security success.
- Dependency alerts clear after merge.
- `Host` and `Origin` validation enforced on browser-reachable transport routes.
- Bearer-token behavior preserved for non-local access paths.
- Smoke test proves transport liveness.

## Stage 3: Tool Discovery Contract

Expose an empty or low-risk tool registry and prove tool discovery behavior.

Status: complete.

Required gates:

- Tool discovery smoke test.
- No filesystem write behavior.
- No platform automation behavior.
- No command execution behavior.

## Stage 4: First Low-Risk Read-Only Tool

Add one low-risk read-only tool with deterministic output and tests.

Status: complete.

Required gates:

- Tool call smoke test.
- Tool output schema documented.
- No broad filesystem or platform access.

## Stage 5: Filesystem Tools

Restore filesystem capability with narrow safe roots, read/write separation, and explicit write controls.

Status: complete for the current staged filesystem surface. The current substage exposes safe-rooted read-only directory listing, bounded safe-rooted UTF-8 file reads, and safe-rooted `write_file` that defaults to dry-run. Mutating writes require explicit `"dry_run": false`, remain safe-root constrained, and use non-sensitive audit decision primitives for staged write policy decisions.

Required gates:

- Safe-root traversal tests.
- Symlink escape tests.
- Read-only directory listing test.
- Bounded read-file test.
- Dry-run write test before any write-capable tool is exposed.
- Documentation of operator assumptions.
- Non-sensitive audit decision coverage before later mutating or command-capable stages expand.

## Stage 6: Android Platform Tools

Restore Android platform tools only after explicit feature gates and operational documentation.

Status: primitive baseline in progress. The current `android_status` MCP tool exposes only read-only allowlisted Android/Termux status metadata. It does not use Android APIs, perform Android control actions, enable shell fallback, execute commands, or expose high-impact controls.

Required gates:

- Feature-gated compile path.
- Runtime disabled-by-default behavior.
- Tool-level smoke tests or documented manual validation.
- No command execution or high-impact controls bundled into the same PR.
- Explicit operator documentation before any Android platform API/control surface is exposed.

## Stage 6a: Project-Owned Service Status

Add project-owned service-status primitives only after Android/platform status remains constrained.

Status: primitive baseline in progress. `service_status` currently models the allowlisted `mcp_runtime` logical service only. It is read-only, project-owned, data-only code and is not exposed as an MCP transport tool in this stage.

Required gates:

- Fixed in-repo service allowlist.
- No arbitrary process enumeration.
- No PID, command-line, environment, port, package, or filesystem inventory exposure.
- No service start, stop, restart, kill, shell, or mutation behavior.
- Separate transport-exposure PR before any MCP `service_status` tool is advertised.

## Stage 7: High-Impact Tooling

Add high-impact tooling only after separate authorization and operator-consent policy is in place.

Status: not started.

Required gates:

- Feature-gated compile path.
- Explicit operator opt-in.
- Audit/logging assumptions documented.
- Separate validation PR.

## Non-Goals

- Do not merge PRs that restore all runtime surfaces at once.
- Do not bundle dependency updates with unrelated behavior changes.
- Do not claim MCP production readiness without transport and tool smoke tests.
