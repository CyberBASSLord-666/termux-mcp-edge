# MCP write_file dry-run transport gate

This stage defines the minimum gate for exposing `write_file` through the MCP transport without enabling default mutation.

## Scope allowed

- Add `write_file` to MCP tool discovery only after the transport rejects unsafe write requests deterministically.
- Require `path` and `content` arguments.
- Treat omitted `dry_run` as dry-run.
- Permit `dry_run: true` as dry-run.
- Reject `dry_run: false` at the MCP transport boundary until a later explicitly approved mutating stage.
- Reuse the existing safe-root filesystem implementation and write-policy primitives.

## Scope not allowed

- No default mutating writes.
- No Android/platform tools.
- No shell or command execution.
- No high-impact tools.
- No broad SSE/session transport expansion.
- No dependency expansion.
- No workflow broadening.

## Validation required for the next transport-wiring PR

Before a follow-up implementation PR may merge, it must satisfy all of the following checks:

- Exact-head CI success for the implementation PR head SHA.
- Security is not required unless dependency, lockfile, or Security workflow inputs change.
- Diff remains narrow and current-base, meaning the PR branch is rebased on current `main` or is otherwise mergeable without bringing stale base changes forward.
- Tests cover tool discovery, dry-run response behavior, safe-root rejection, payload-limit rejection, and explicit-mutating-write rejection.

## Next implementation target

The next implementation PR may wire a transport-level `write_file` tool only in dry-run mode. Actual mutation remains blocked until a separate stage adds authorization policy, operator intent confirmation, and regression coverage for explicit mutation controls.
