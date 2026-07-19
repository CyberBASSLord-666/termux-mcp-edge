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
2. Opens the anchored root and resolves the existing parent one component at a time with no-follow descriptors.
3. Retains both the safe-root and mutation-parent descriptors through authorization, staging, publication, verification, cleanup, and durability sync.
4. Classifies the final name without following it:
   - absence selects **create**;
   - one regular file selects **replace** and retains a descriptor plus its type, device, inode, mode, and size snapshot;
   - a symlink, directory, FIFO, socket, device, or other special object is rejected.
5. Builds the authorization target from the anchored root identity, normalized root-relative components, exact content SHA-256, create-or-replace disposition, and mutating posture.

Create and replace are distinct authorization postures. A create grant cannot overwrite a file that appears before publication, and a replace grant cannot create a missing target.

## Preview behavior

Omitted `dry_run` and explicit `dry_run:true` perform the same validation and classification needed to describe the operation but do not require or consume a grant, create a staging file, publish content, or change the destination. Supplying an otherwise valid grant on a preview is permitted only in the exact `write_file` tool-call context and leaves that grant available for its later matching mutation. This is also the complete behavior of the public `FileSystemTools::write_file` API; it rejects explicit mutation.

## Authorized mutation sequence

For `dry_run:false`, the runtime acquires one process-wide mutex shared by every in-process `FileSystemTools` instance, then follows this order while retaining the mutex:

1. Revalidate the classified destination posture and any held replacement identity.
2. At one atomic request commit point immediately before grant consumption, transition from pending to either request-cancelled or worker-owned. A cancellation winner stops before grant consumption or mutation; a worker winner owns completion independently of later request cancellation.
3. Atomically validate and consume the exact grant immediately before the first filesystem mutation attempt. Consumption survives every later success, failure, timeout, or client cancellation.
4. Create one unpredictable same-parent staging name with exclusive no-follow creation, force mode `0600`, and capture its regular-file device/inode identity.
5. Write the exact bytes, sync the staging descriptor, then verify its held and named type, device, inode, mode, size, and SHA-256.
6. Revalidate the create-or-replace posture immediately before publication.
7. Publish atomically:
   - **create:** `RENAME_NOREPLACE`, followed by held/named identity, mode, and size verification;
   - **replace:** `RENAME_EXCHANGE`, followed by verification that the new staged identity owns the final name and the exact captured old identity owns the displaced name.
8. Sync the parent directory at the publication boundary, remove only the exact displaced regular-file identity for replacement, and sync the parent again after cleanup.

The mutex remains held through staging, publication, verification, any rollback, operation-owned cleanup, and the final parent sync. Consequently, two mutations prepared against the same old destination cannot race inside one server process: after the winner completes, the waiting operation fails its first destination revalidation before authorization or grant consumption. Its distinct, still-valid grant remains reusable with a newly prepared request.

In-process `write_file` namespace changes are serialized; detected namespace changes may cause a bounded private failure, but cannot turn create into overwrite, follow a symlink, publish a special object, or delete a name whose observed identity is not owned by the operation.

## Failure, cancellation, and cleanup

Cleanup is descriptor-relative and identity checked. A cleanup guard is armed only after the staging identity is captured. It may unlink only a named regular file whose device and inode still match that captured identity; a missing, exchanged, linked, or foreign object is preserved. Successful publication disarms staging cleanup only after the required durability boundary. The process-wide mutex makes this guarantee complete against every other in-process `write_file` mutation.

When replacement publication verification fails, rollback first requires the exact staged inode at the final name, then non-destructively exchanges it back with the displaced name regardless of the displaced object's identity. This restores a late foreign replacement instead of leaving authorized content published by a call that reports failure. Failures never make a consumed grant reusable.

The mutation runs in an owned blocking worker with a three-state pending/request-cancelled/worker-owned commit point immediately before grant consumption. If request cancellation wins, the worker performs no mutation and the grant remains reusable. If the worker wins, later request cancellation cannot abandon grant-consumed staging, verification, publication, rollback, cleanup, or durability work.

Against other in-process server writes, no success or failure path leaves an operation-owned staging file, and a foreign object placed at a former staging name is not removed.

An independent local OS process with direct write access to the same parent directory is outside the process mutex. Linux provides no conditional unlink-by-inode operation, so such a process can race the identity check and subsequent name-based `unlinkat`. Operators must give the server exclusive operational ownership of configured safe roots and must not run independent writers there; otherwise the foreign-object cleanup guarantee cannot be absolute across processes.

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
- process-wide serialization across separate `FileSystemTools` instances, including two pre-prepared same-target replacements where the stale loser fails before grant consumption and reuses that grant only after re-preparation;
- request cancellation on both sides of the pending/request-cancelled/worker-owned commit point;
- parent/final/staging exchange races, target mode/size/identity races, every post-consumption failure boundary, non-destructive exchange rollback despite a changed displaced identity, no staging residue, and in-process foreign-object preservation;
- oversized actual JSON-RPC ID preflight followed by successful reuse of the same unconsumed grant;
- private responses and aggregate audits;
- default and all-feature Rust suites, fixture parity, validator v8, device harness v8, every optional emulated posture, Android cross-builds, native official-Termux ARM64 execution, exact-head CI/Security, and direct physical observation when required by release classification.

## Non-goals

This tool does not authorize append, partial writes, binary argument encoding, permissions or ownership selection, symlink following, directory creation, recursive operations, deletion, rename, arbitrary host paths, or reuse of a grant for another request.
