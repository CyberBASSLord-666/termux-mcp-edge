# Staged MCP write-tool transport gate

This note records the required gate for the next filesystem stage before any write-capable MCP runtime surface is exposed.

## Current baseline

- `list_directory` is exposed through MCP with safe-root enforcement and bounded traversal depth.
- `read_file` is exposed through MCP with safe-root enforcement and a staged byte limit.
- `FileSystemTools::write_file` exists as an internal primitive, but omitted `dry_run` defaults to `true`.
- No MCP runtime `write_file` tool is exposed yet.
- Android/platform tools, command execution, and high-impact tools remain unavailable.

## Required transport constraints for the first write-capable runtime PR

The first transport PR that exposes `write_file` must remain default-deny and dry-run-first:

1. `tools/list` may advertise `write_file` only with an input schema that makes write intent explicit.
2. `dry_run` must default to `true` if omitted at the transport boundary.
3. Mutating writes must require `dry_run: false` in the tool arguments.
4. Safe-root validation must remain mandatory for every write request.
5. The response for dry-run writes must be distinguishable from mutating writes.
6. Runtime status must continue to show Android/platform tools, command execution, and high-impact tools as disabled.
7. Tests must cover default dry-run, explicit opt-in mutation, path traversal rejection, discovery schema shape, and unknown-tool behavior.

## Non-goals for the next transport stage

- No Android/platform tools.
- No shell or command execution.
- No broad SSE/runtime rewrite.
- No dependency expansion.
- No high-impact tools.
- No replacement of the existing transport security policy.

## Merge gate

A write-capable transport PR is mergeable only when its exact head SHA has passing validation and the diff is limited to the intended narrow transport/test surface. If CI or security validation fails, or the PR expands beyond the write-tool gate, it must remain blocked or be replaced with a smaller current-base stage.
