# Safe-Root File Write Contract

## Scope

`write_file` previews or performs one bounded UTF-8 file publication below one configured filesystem safe root. It is not an arbitrary file-descriptor, append, patch, chmod, ownership, metadata-preservation, symlink, or special-file API.

The baseline `mcp-runtime` registry always contains the tool. Mutation is independently default-disabled and is never authorized by bearer authentication or `dry_run:false` alone.

The public library entry point `FileSystemTools::write_file` is preview-only. Omitted `dry_run` and `Some(true)` validate and classify; `Some(false)` returns an authorization-required error without mutation. Live publication is reachable only through the crate-private prepared operation used by the MCP transport, so an embedding caller cannot bypass request-scoped authorization.

## Request and response bounds

| Field | Type | Required | Contract |
|---|---|---:|---|
| `path` | string | yes | Absolute lexical descendant of one configured safe root. |
| `content` | string | yes | Exact UTF-8 bytes to publish; maximum 1,048,576 bytes. |
| `dry_run` | boolean | no | Defaults to `true`. Explicit `false` selects a mutation request but does not authorize it. |

The complete success response, including the caller's actual JSON-RPC ID, is capped at 16,384 bytes. The runtime constructs and serializes that exact success envelope before grant consumption, staging-file creation, or any other mutation. An oversized response therefore cannot consume a grant or leave a temporary file.

Successful publication always has mode `0600`. Existing permissions, ownership metadata, timestamps, extended attributes, hard-link relationships, and sparse layout are not preserved.

## Gate and discovery state

`MCP__FILE__WRITE_MUTATION_ENABLED` defaults to `false`.

- When disabled, discovery keeps `write_file` visible for preview, constrains `dry_run` to `true`, runtime status reports `fileWriteMutationEnabled=false`, and live dispatch returns a private authorization denial without touching the filesystem.
- When enabled, startup additionally requires static-token authentication and the complete capability key pair. Discovery removes the `dry_run:true` constraint, but every live call still needs one exact request-scoped grant.
- A binary without `mcp-runtime` rejects the enabled flag. A disabled gate never becomes enabled merely because another filesystem or Android capability uses the same signing key.

See [Write-file capability grants](WRITE_FILE_CAPABILITY_GRANTS.md) for configuration and issuance.

## Descriptor-safe classification

After argument and complete-response preflight, the runtime:

1. Lexically anchors `path` to the most specific configured safe root and rejects relative paths, NUL bytes, parent traversal, the safe-root path itself, and paths outside every root.
2. Duplicates and identity-verifies the selected root's lifetime pin, then resolves the existing parent one component at a time with no-follow descriptors.
3. Retains the mutation-parent descriptor through authorization, private-quarantine staging, publication, verification, and durability sync. Replacement also retains a no-follow descriptor for the classified target. The operation may release its root duplicate after parent resolution, while the shared lifetime pin remains authoritative for later calls.
4. Classifies the final name without following it:
   - absence selects **create**;
   - one single-link regular file of at most 1 MiB selects **replace** and retains a descriptor plus its device, inode, size, high-resolution ctime, and link-count identity;
   - a symlink, directory, FIFO, socket, device, or other special object is rejected.
5. Builds the authorization target from the anchored root identity, normalized root-relative components, exact content SHA-256, create-or-replace disposition, and mutating posture.

Create and replace are distinct authorization postures. A create grant cannot overwrite a file that appears before publication, and a replace grant cannot create a missing target.

## Preview behavior

Omitted `dry_run` and explicit `dry_run:true` perform the same validation and classification needed to describe the operation but do not require or consume a grant, create a staging file, publish content, or change the destination. Supplying an otherwise valid grant on a preview is permitted only in the exact `write_file` tool-call context and leaves that grant available for its later matching mutation. This is also the complete behavior of the public `FileSystemTools::write_file` API; it rejects explicit mutation.

## Authorized mutation sequence

For `dry_run:false`, the transport first tries to acquire the one shared, non-queueing filesystem-mutation worker permit. This permit is shared with other authorized filesystem mutation families. If another worker owns it, the request fails immediately with a private capacity response before descriptor preparation, grant consumption, or filesystem mutation. The runtime does not maintain a queue of waiting writes.

Inside the permit-owned blocking worker, the runtime follows this order:

1. Prepare and classify the target while retaining the mutation-parent descriptor and, for replacement, the exact existing-target identity and descriptor.
2. Acquire the one poison-fail-closed process-wide filesystem-publication lock shared by every `FileSystemTools` instance and by `create_directory`, `copy_file`, and `write_file`. A worker may wait here only after it owns the fail-fast permit. The lock remains held through every later step.
3. Perform a read-only quarantine-capacity preflight. If the fixed `.termux-mcp-write-quarantine` child exists, open it without following links, require a mode-`0700` directory, acquire its nonblocking advisory lock, and reject malformed contents, contention, more than 32 artifacts, or insufficient remaining capacity within the 32 MiB bound. This preflight descriptor and lock are released before authorization.
4. Under the process lock, revalidate the exact prepared posture: create still requires an absent final name; replace requires the same held and named device, inode, size, high-resolution ctime, and single-link identity.
5. Resolve the atomic request-cancellation/worker-ownership commit guard and, only for a worker winner, atomically validate and consume the exact grant. Cancellation while preparing or waiting for the process lock wins without consuming the grant or changing the filesystem. Grant consumption survives every later success or failure.
6. Open or create the quarantine. Creating the directory, when absent, is the first mutation attempt. Revalidate its type and mode, reacquire and retain its nonblocking lock, and recheck capacity so a change after preflight fails closed.
7. Create one unpredictable `.termux-mcp-write-artifact-*` regular file inside the held quarantine directory with exclusive no-follow creation and mode `0600`. Write the exact bytes, sync it, and verify its held and named type, identity, mode, and size.
8. Revalidate the create-or-replace posture immediately before publication as defense in depth against non-cooperating external namespace changes.
9. Publish atomically:
   - **create:** move the staged inode to the final name with `RENAME_NOREPLACE`; no recovery artifact remains;
   - **replace:** perform one `RENAME_EXCHANGE`; the authorized staged inode becomes the final target and the displaced prior inode remains under the randomized quarantine name.
10. Verify the exact final identity, mode, and size, sync the held target parent and quarantine directories, and revalidate the quarantine bounds before releasing the process lock.

The process-wide lock serializes every cooperating in-process `create_directory`, `copy_file`, and `write_file` publication across distinct `FileSystemTools` instances, including those owned by distinct router states. It is a correctness boundary, not a fairness queue: the per-state worker permit remains fail-fast and bounds blocking work before this lock is reached. Poison is never recovered; later publication attempts fail closed before commit claim or grant consumption. The retained quarantine advisory lock separately coordinates the parent-local recovery namespace with cooperating external processes. Its preflight lock is deliberately not held across authorization, so every relevant condition is checked again under the retained lock. Therefore a post-consumption create, open, quarantine-lock, or defense-in-depth revalidation failure consumes the grant without publishing content. An independent process under the same Unix UID can ignore the advisory lock and race namespace operations; such interference can force a bounded denial or a documented post-commit failure. Production mutation safe roots therefore require exclusive operational ownership by this service: do not run independent writers against a configured root while live `create_directory`, `copy_file`, or `write_file` gates are enabled.

## Failure, cancellation, and retained recovery

Replacement has one irreversible commit point: `RENAME_EXCHANGE`. After that exchange the runtime never automatically rolls back, unlinks, truncates, renames, or deliberately changes the displaced object's content, mode, ownership, or extended attributes. The prior inode and content remain in the mode-`0700` quarantine. Its mode, ownership, and extended attributes are not deliberately modified, but the rename operation can update ctime, and other filesystem-managed metadata is not guaranteed to remain unchanged. A successful replacement reports `recoveryArtifactRetained:true`; create and preview report `false`.

If verification or directory synchronization fails after exchange, the request reports failure and the grant remains consumed. The authorized new inode may remain at the public target and the displaced prior inode remains quarantined. This is an explicit preservation rule: POSIX pathname operations cannot provide an inode-conditional rollback or cleanup that is safe against a hostile same-UID namespace peer.

Before worker ownership—including while the blocking worker waits for the process publication lock—cancellation consumes no grant and changes no state. A stale prepared operation that loses lock-held posture revalidation also fails before grant consumption, so the same unconsumed grant remains usable after a fresh matching preparation. After the worker wins the commit guard, later timeout, disconnect, or task cancellation does not detach the permit-owned blocking operation. It continues while retaining both permit and process lock through bounded publication, verification, and durability work without destructive post-capture cleanup.

Recovery artifacts are operator-managed material, not an automatic version history. Do not remove them while the runtime or another same-UID writer is active. To reclaim capacity, stop and quiesce the service and other writers, inspect the exact quarantine entry locally, remove only the selected entry without broad globs or recursive deletion, then restart and verify health and readiness. Back up any retained content that must survive device or filesystem loss.

## Stable private failures and audit behavior

The transport uses bounded JSON-RPC errors without reflecting the target path, content, content digest, safe-root identity, principal, session, grant, key ID, JTI, timestamps, or staging name.

Authorization denials use HTTP 403 / JSON-RPC `-32003` with stable low-cardinality reasons such as missing, malformed, unknown-version/key, invalid-signature, expired, future-issued, lifetime, binding, replay, clock, capacity, or state failure. Invalid path, posture, or target type uses bounded argument errors; payload or response ceilings use bounded size errors; internal I/O failures use one private filesystem-write reason.

Exactly one aggregate filesystem audit decision is recorded per dispatched tool call. Audit counters may retain only tool, gate, resolved dry-run-or-mutating mode, allowed/denied count, and stable reason code.

## Required validation

Release evidence must cover:

- disabled-default discovery, runtime status, and inert explicit mutation;
- enabled discovery and exact static-auth/key startup requirements;
- omitted and explicit preview, including grant non-consumption;
- missing, malformed, wrong-context, wrong-principal, wrong-session, wrong-root, wrong-target, wrong-content, wrong-disposition, wrong-posture, expired, future, lifetime, version, key, signature, replay, concurrent-replay, capacity, and clock failures;
- exact 1 MiB acceptance and 1 MiB plus one byte rejection;
- fixed `0600` create and replace with exact content;
- create/replace mismatch, create no-replace, missing parent, root target, outside root, linked parent, final symlink, directory, FIFO, and other special-object rejection;
- fail-fast shared-worker capacity across mutation families, including denial before preparation or grant consumption and owned-worker completion after request cancellation;
- process-wide serialization across distinct tool instances, poison failure, lock-held absent/exact-identity posture revalidation, stale-loser grant reuse, and request cancellation while waiting on either side of the pending/request-cancelled/worker-owned commit point;
- quarantine mode, naming, visibility, capacity, lock contention, and malformed-entry denial; create `NOREPLACE`; irreversible replace `EXCHANGE`; exact retained displaced identity, content, and mode; and every post-consumption failure state without automatic rollback or destructive cleanup;
- oversized actual JSON-RPC ID preflight followed by successful reuse of the same unconsumed grant;
- private responses and aggregate audits;
- default and all-feature Rust suites, fixture parity, validator v8, device harness v8, every optional emulated posture, Android cross-builds, native official-Termux ARM64 execution, exact-head CI/Security, and direct physical observation when required by release classification.

## Non-goals

This tool does not authorize append, partial writes, binary argument encoding, permissions or ownership selection, symlink following, directory creation, recursive operations, deletion, rename, arbitrary host paths, or reuse of a grant for another request.
