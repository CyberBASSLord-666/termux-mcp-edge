# Staged MCP write-tool transport gate

This note records the write-tool transport gate that was required before `write_file` became part of the staged MCP runtime. It is retained as historical validation context and must not be read as the current runtime contract.

## Historical baseline before the write-tool stage

Before the write-tool transport stage merged:

- `list_directory` was exposed through MCP with safe-root enforcement and bounded traversal depth.
- `read_file` was exposed through MCP with safe-root enforcement and a staged byte limit.
- `FileSystemTools::write_file` existed as an internal primitive, but omitted `dry_run` defaulted to `true`.
- No MCP runtime `write_file` tool was exposed yet.
- Android/platform tools, command execution, and high-impact tools remained unavailable.

## Current baseline after the write-tool stage

The current staged MCP runtime exposes `write_file` with safe-root enforcement, payload-size controls, and dry-run-by-default behavior. Omitted `dry_run` defaults to `true`; mutating writes require explicit `dry_run: false` and remain bounded by the configured filesystem safe roots.

Android platform control, shell fallback, arbitrary command execution, high-impact tools, and broad runtime-surface expansion remain unavailable unless a later independently validated stage adds them.

## Required transport constraints for the first write-capable runtime PR

The first transport PR that exposed `write_file` had to remain default-deny and dry-run-first:

1. `tools/list` may advertise `write_file` only with an input schema that makes write intent explicit.
2. `dry_run` must default to `true` if omitted at the transport boundary.
3. Mutating writes must require `dry_run: false` in the tool arguments.
4. Safe-root validation must remain mandatory for every write request.
5. The response for dry-run writes must be distinguishable from mutating writes.
6. Runtime status must continue to show Android/platform tools, command execution, and high-impact tools as disabled.
7. Tests must cover default dry-run, explicit opt-in mutation, path traversal rejection, discovery schema shape, and unknown-tool behavior.

## Non-goals for the write-tool transport stage

- No Android/platform tools.
- No shell or command execution.
- No broad SSE/runtime rewrite.
- No dependency expansion.
- No high-impact tools.
- No replacement of the existing transport security policy.

## Merge gate

A write-capable transport PR was mergeable only when its exact head SHA had passing validation and the diff was limited to the intended narrow transport/test surface. If CI or security validation failed, or the PR expanded beyond the write-tool gate, it had to remain blocked or be replaced with a smaller current-base stage.
