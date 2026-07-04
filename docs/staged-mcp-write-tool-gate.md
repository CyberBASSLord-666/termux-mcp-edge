# Staged MCP write-tool transport gate

This note records the required gate for the current filesystem stage and the continuing constraints for later write-capable or higher-impact MCP runtime surfaces.

## Current baseline

- `runtime_status` is exposed through MCP with deterministic read-only runtime metadata.
- `platform_info` is exposed through MCP with non-sensitive read-only platform metadata.
- `android_status` is exposed through MCP with read-only allowlisted Android/Termux status metadata only.
- `list_directory` is exposed through MCP with safe-root enforcement and bounded traversal depth.
- `read_file` is exposed through MCP with safe-root enforcement and a staged byte limit.
- `write_file` is exposed through MCP, but omitted `dry_run` defaults to `true`.
- Mutating writes require explicit `"dry_run": false` and remain safe-root constrained.
- Android platform API/control tools, shell fallback, command execution, and high-impact controls remain unavailable.

## Required transport constraints for the current write-capable runtime surface

The current `write_file` transport surface must remain default-deny and dry-run-first:

1. `tools/list` may advertise `write_file` only with an input schema that makes write intent explicit.
2. `dry_run` must default to `true` if omitted at the transport boundary.
3. Mutating writes must require `dry_run: false` in the tool arguments.
4. Safe-root validation must remain mandatory for every write request.
5. The response for dry-run writes must be distinguishable from mutating writes.
6. Runtime status must continue to show Android platform API/control tools, command execution, and high-impact tools as disabled.
7. Tests must cover default dry-run, explicit opt-in mutation, path traversal rejection, discovery schema shape, and unknown-tool behavior.

## Android status constraints

The current `android_status` tool is status-only and must not be treated as Android platform control readiness:

1. It may expose only allowlisted read-only status metadata.
2. It must not use Android APIs or shell fallback.
3. It must not perform Android platform control actions.
4. It must not expose device identifiers, package lists, process listings, environment variables, usernames, hostnames, secrets, or broad filesystem state.
5. It must continue to report command execution and high-impact controls as disabled.

## Non-goals for the next runtime stages

- No shell or command execution.
- No broad SSE/runtime rewrite.
- No dependency expansion.
- No high-impact tools.
- No replacement of the existing transport security policy.
- No Android platform API/control surface without a separate current-base PR, explicit feature gate, tests, and operational documentation.

## Merge gate

A runtime-surface PR is mergeable only when its exact head SHA has passing validation and the diff is limited to the intended narrow transport/test surface. If CI or security validation fails, or the PR expands beyond the staged gate, it must remain blocked or be replaced with a smaller current-base stage.
