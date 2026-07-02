# MCP Runtime Roadmap

## Goal

Move from the current conservative health‑check runtime to a full MCP runtime without regressing security, CI, dependency posture, or documentation accuracy.

## Current Baseline

`main` exposes the health‑check runtime by default. The optional `mcp-runtime` feature is being restored in narrow stages. The current staged transport shell validates exact `Host` and browser `Origin` values before handling `/mcp`, supports `initialize`, exposes `tools/list`, and adds deterministic read‑only `runtime_status` plus safe‑rooted directory listing and file I/O with optional dry‑run, as well as a basic platform information tool. Android platform access, command execution, and high‑impact actions remain unavailable.

## Stage 1: Transport Request Validation

Add reusable `Host` and `Origin` validation primitives with unit coverage. No routes are exposed in this stage.

Status: complete.

Required gates:

- Exact‑head CI success.
- Exact‑head Security success.
- No new dependency surface.
- No runtime exposure.

## Stage 2: Minimal MCP Transport Shell

Introduce the smallest MCP transport runtime without filesystem, platform, or high‑impact tools.

Status: complete.

Required gates:

- Exact‑head CI success.
- Exact‑head Security success.
- Dependency alerts clear after merge.
- `Host` and `Origin` validation enforced on browser‑reachable transport routes.
- Bearer‑token behavior preserved for non‑local access paths.
- Smoke test proves transport liveness.

## Stage 3: Tool Discovery Contract

Expose an empty or low‑risk tool registry and prove tool discovery behavior.

Status: complete.

Required gates:

- Tool discovery smoke test.
- No filesystem write behavior.
- No platform automation behavior.
- No command execution behavior.

## Stage 4: First Low‑Risk Read‑Only Tool

Add one low‑risk read‑only tool with deterministic output and tests.

Status: complete.

Required gates:

- Tool call smoke test.
- Tool output schema documented.
- No broad filesystem or platform access.

## Stage 5: Filesystem Tools

Restore filesystem capability with narrow safe roots, read/write separation, and explicit write controls.

Status: **complete**. This stage introduces safe‑rooted `read_file` and `write_file` tools in addition to the existing `list_directory`. Reads always validate the safe‑root boundary and return file contents. Writes default to dry‑run, requiring an explicit flag to persist changes. Symlink escapes are prevented and only the configured safe roots are allowed.

Required gates:

- Safe‑root traversal tests.
- Symlink escape tests.
- Read/write behaviour tests.
- Dry‑run write test.
- Documentation of operator assumptions.

## Stage 6: Android Platform Tools

Restore Android platform tools only after explicit feature gates and operational documentation.

Status: **in progress**. The first substage exposes a read‑only `platform_info` tool that returns basic host OS/arch/family information using standard library constants. No Android APIs are called and no device sensors or high‑impact operations are enabled.

Required gates:

- Feature‑gated compile path.
- Runtime disabled‑by‑default behaviour for any future platform APIs.
- Tool‑level smoke test for `platform_info`.
- Documentation of platform tool assumptions.

## Stage 7: High‑Impact Tooling

Add high‑impact tooling only after separate authorization and operator‑consent policy is in place.

Status: not started.

Required gates:

- Feature‑gated compile path.
- Explicit operator opt‑in.
- Audit/logging assumptions documented.
- Separate validation PR.

## Non‑Goals

- Do not merge PRs that restore all runtime surfaces at once.
- Do not bundle dependency updates with unrelated behaviour changes.
- Do not claim MCP production readiness without transport and tool smoke tests.
