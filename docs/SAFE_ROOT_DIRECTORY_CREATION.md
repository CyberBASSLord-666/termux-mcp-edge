# Safe-rooted directory creation

`create_directory` is a narrowly scoped MCP filesystem capability for creating exactly one project-owned directory beneath an already configured filesystem safe root.

## Availability and authorization

- Compiled only with the existing `mcp-runtime` posture.
- Hidden from `tools/list` unless the MCP runtime is available **and** the dedicated directory-creation runtime capability is enabled.
- The dedicated runtime capability must default to disabled, parse fail-closed, and be independent of read-only filesystem tools and broader command/high-impact capabilities.
- Every mutating request must carry an explicit, request-scoped capability grant for `filesystem.create-directory`; authentication and runtime enablement alone are insufficient.
- Uses a closed input schema containing:
  - required absolute `path`;
  - optional `dryRun` boolean, defaulting to `true`.
- Mutation requires all of the following: authenticated request, enabled dedicated runtime capability, valid request-scoped capability grant, and explicit `dryRun: false`.
- Missing, malformed, expired, mismatched, or replayed grants fail closed with stable redacted errors.
- Dry-run requests may validate policy and confinement but must not create, remove, rename, chmod, chown, or synchronize any target object.

## Fixed behavior

- Creates one directory only.
- Does not create missing parents.
- Does not recurse.
- Does not replace or modify an existing object.
- Does not accept caller-selected mode, ownership, timeout, root, durability behavior, or resource ceilings.
- Uses fixed mode `0700`.
- Returns only the normalized safe-rooted path, `dryRun`, fixed mode, and the fixed response ceiling.

## Descriptor-relative confinement

The implementation must reuse the established safe-root anchoring and component-by-component no-follow traversal. The exact parent directory is opened by descriptor and retained through mutation, verification, rollback, and synchronization.

The final creation operation must use `mkdirat` against that retained parent descriptor. Pathname re-resolution after authorization is prohibited.

The operation must reject, with stable redacted errors:

- the safe root itself;
- relative paths;
- traversal and NUL input;
- paths outside configured roots;
- missing parents;
- symlink parents or final components;
- existing files or directories;
- unsupported final object types.

## Verification, durability, and rollback

After `mkdirat` succeeds, the implementation must:

1. Open or inspect the exact created entry relative to the retained parent descriptor with no-follow semantics.
2. Verify it is a directory with mode `0700`.
3. Synchronize the exact parent directory before reporting success.

If verification or parent synchronization fails, cleanup must remove only the newly created empty directory through the same retained parent descriptor. Cleanup failure must be surfaced as the authoritative terminal failure; the operation must not report success while durability or rollback is uncertain.

Blocking filesystem work must execute outside the async runtime.

## Limits and privacy

- The full serialized MCP response is subject to a fixed ceiling.
- Audit events may retain only stable tool, outcome/reason, dry-run-versus-mutating, runtime-gate, and capability-grant result labels.
- Paths, path fragments, raw grants, grant identifiers, raw OS errors, inode/device identifiers, ownership, and permission internals must never enter audit labels or logs.
- Authorization denial, confinement denial, durability failure, rollback success, and rollback failure must be distinguishable through stable aggregate counters without exposing caller or filesystem data.

## Required regression coverage

The implementation is incomplete until tests prove:

- omitted or true `dryRun` leaves the target absent;
- explicit false still fails when the dedicated runtime capability is disabled;
- explicit false still fails without a valid request-scoped `filesystem.create-directory` grant;
- malformed, expired, mismatched, and replayed grants fail closed and leave the target absent;
- a valid grant cannot authorize another tool, path scope, or second request;
- explicit false with all authorization conditions satisfied creates exactly one directory with mode `0700`;
- existing file/directory, missing parent, outside-root, relative, traversal, NUL, root-target, symlink-final, and symlink-parent denials;
- parent and final-object exchange resistance under deterministic race hooks;
- rollback after post-create verification failure and parent-sync failure;
- cleanup-failure precedence;
- closed MCP schema and stable allowed/denied responses;
- exact baseline and enabled tool allowlists, including hidden discovery while the dedicated gate is disabled;
- aggregate audit privacy and grant-secret redaction;
- release-validator, device-smoke, and native official-Termux ARM64 execution evidence.

## Non-goals

Recursive creation, deletion, rename, movement, chmod, ownership changes, shell execution, arbitrary command execution, and broad Android shared-storage authority remain unavailable.
