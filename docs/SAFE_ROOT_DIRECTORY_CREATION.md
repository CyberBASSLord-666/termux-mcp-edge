# Safe-rooted directory creation

`create_directory` is a narrowly scoped MCP filesystem capability for creating exactly one project-owned directory beneath an already configured filesystem safe root.

## Availability

- Compiled only with the existing `mcp-runtime` posture.
- Hidden from `tools/list` when the MCP runtime is unavailable.
- Uses a closed input schema containing:
  - required absolute `path`;
  - optional `dryRun` boolean, defaulting to `true`.
- Mutation requires the caller to send `dryRun: false` explicitly.

## Fixed behavior

- Creates one directory only.
- Does not create missing parents.
- Does not recurse.
- Does not replace or modify an existing object.
- Does not accept caller-selected mode, ownership, timeout, root, or durability behavior.
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
- Audit events may retain only stable tool, outcome/reason, and dry-run-versus-mutating labels.
- Paths, path fragments, raw OS errors, inode/device identifiers, ownership, and permission internals must never enter audit labels or logs.

## Required regression coverage

The implementation is incomplete until tests prove:

- omitted or true `dryRun` leaves the target absent;
- explicit false creates exactly one directory with mode `0700`;
- existing file/directory, missing parent, outside-root, relative, traversal, NUL, root-target, symlink-final, and symlink-parent denials;
- parent and final-object exchange resistance under deterministic race hooks;
- rollback after post-create verification failure and parent-sync failure;
- cleanup-failure precedence;
- closed MCP schema and stable allowed/denied responses;
- exact tool allowlists and aggregate audit privacy;
- release-validator, device-smoke, and native official-Termux ARM64 execution evidence.

## Non-goals

Recursive creation, deletion, rename, movement, chmod, ownership changes, shell execution, arbitrary command execution, and broad Android shared-storage authority remain unavailable.