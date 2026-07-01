# MCP Runtime Roadmap

## Goal

Move from the current conservative health-check runtime to a full MCP runtime without regressing security, CI, dependency posture, or documentation accuracy.

## Current Baseline

`main` currently exposes the health-check runtime only. MCP transport and MCP tools are not compiled into the release line.

## Stage 1: Transport Request Validation

Add reusable `Host` and `Origin` validation primitives with unit coverage. No routes are exposed in this stage.

Required gates:

- Exact-head CI success.
- Exact-head Security success.
- No new dependency surface.
- No runtime exposure.

## Stage 2: Minimal MCP Transport Shell

Introduce the smallest MCP transport runtime without filesystem, platform, or high-impact tools.

Required gates:

- Exact-head CI success.
- Exact-head Security success.
- Dependency alerts clear after merge.
- Host and Origin validation enforced on browser-reachable transport routes.
- Bearer-token behavior preserved for non-local access paths.
- Smoke test proves transport liveness.

## Stage 3: Tool Discovery Contract

Expose an empty or low-risk tool registry and prove tool discovery behavior.

Required gates:

- Tool discovery smoke test.
- No filesystem write behavior.
- No platform automation behavior.
- No command execution behavior.

## Stage 4: First Low-Risk Read-Only Tool

Add one low-risk read-only tool with deterministic output and tests.

Required gates:

- Tool call smoke test.
- Tool output schema documented.
- No broad filesystem or platform access.

## Stage 5: Filesystem Tools

Restore filesystem capability with narrow safe roots, read/write separation, and explicit write controls.

Required gates:

- Safe-root traversal tests.
- Symlink escape tests.
- Read-only mode test.
- Dry-run write test.
- Documentation of operator assumptions.

## Stage 6: Android Platform Tools

Restore Android platform tools only after explicit feature gates and operational documentation.

Required gates:

- Feature-gated compile path.
- Runtime disabled-by-default behavior.
- Tool-level smoke tests or documented manual validation.

## Stage 7: High-Impact Tooling

Add high-impact tooling only after separate authorization and operator-consent policy is in place.

Required gates:

- Feature-gated compile path.
- Explicit operator opt-in.
- Audit/logging assumptions documented.
- Separate validation PR.

## Non-Goals

- Do not merge PRs that restore all runtime surfaces at once.
- Do not bundle dependency updates with unrelated behavior changes.
- Do not claim MCP production readiness without transport and tool smoke tests.
