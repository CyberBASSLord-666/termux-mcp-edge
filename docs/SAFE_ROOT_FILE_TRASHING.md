# Safe-root reversible file trashing

`trash_file` validates or reversibly removes one bounded safe-rooted regular file without exposing raw deletion. Preview is the default. Explicit mutation moves the exact file into a private per-parent recovery quarantine only after the independent runtime gate, static authentication, active MCP session, and exact request grant authorize it. The complete issuance and replay contract is [`TRASH_FILE_CAPABILITY_GRANTS.md`](TRASH_FILE_CAPABILITY_GRANTS.md).

## Closed request schema

| Field | Type | Required | Contract |
| --- | --- | --- | --- |
| `path` | string | yes | Absolute path to one single-link regular file inside a configured safe root. |
| `dry_run` | boolean | no | Defaults to `true`; explicit `false` requests separately gated grant-authorized retention. |

Unknown fields, missing or incorrectly typed arguments, relative paths, NUL bytes, parent traversal, the safe-root directory itself, reserved quarantine components, symbolic links, directories, FIFOs, sockets, devices, missing targets, multiply linked files, and files above the fixed limit are rejected.

## Fixed limits and result

- maximum target size: 1,048,576 bytes;
- accepted content: arbitrary bytes, including empty and non-UTF-8 files;
- recovery directory mode: exactly `0700`;
- maximum retained artifacts per target parent: 32;
- maximum retained bytes per target parent: 33,554,432;
- complete JSON-RPC success response: at most 16,384 bytes.

The exact successful structured result is:

```json
{
  "dryRun": false,
  "sizeBytes": 123,
  "recoveryArtifactRetained": true,
  "maxFileBytes": 1048576,
  "maxResponseBytes": 16384
}
```

Preview reports `recoveryArtifactRetained:false`. Neither posture returns the target path, file content, digest, identity, quarantine path, or recovery name. The transport preflights the full success envelope with the real caller-controlled request id before path access, worker admission, or grant consumption.

## Descriptor-confined preparation

The operation invokes no shell, subprocess, platform trash utility, archive tool, or external provider.

1. Select the longest matching configured safe root and duplicate its lifetime-pinned descriptor.
2. Traverse normalized descendant components descriptor-relatively with no-follow semantics.
3. Retain the root and target-parent descriptors and observe the final component without following it.
4. Require a regular file, exact size at or below 1 MiB, and link count one.
5. Open the target with `O_NOFOLLOW`, `O_NONBLOCK`, and close-on-exec.
6. Verify held and named device, inode, type, size, high-resolution ctime, and link count agree.
7. Read at most the limit plus one byte from the held descriptor, compute SHA-256, and reverify held and named identity after the read.
8. Build the opaque grant target from the pinned root identity, normalized components, exact file identity, and content digest.

Preview ends after this validation and does not create the quarantine. Public Rust callers cannot cross into mutation.

## Atomic retention transaction

Inside the single fail-fast mutation worker, and under the required single-consuming-process ownership model, the live path acquires the process-wide publication lock shared by create, copy, trash, and write. It then:

1. opens and nonblockingly locks an existing `.termux-mcp-trash-quarantine`, or records that it is absent without creating it;
2. validates fixed mode, held/named directory identity, canonical contents, and remaining capacity;
3. takes a nonblocking exclusive advisory lock on the held target;
4. revalidates root, parent, target identity, exact bytes, and SHA-256;
5. resolves cancellation against worker ownership and atomically consumes the exact grant;
6. creates and synchronizes an absent fixed quarantine only after consumption;
7. chooses a randomized canonical UUID recovery name and verifies it is absent;
8. renames from the held parent into the held quarantine with `RENAME_NOREPLACE`;
9. verifies the public name is absent and the retained name plus held descriptor identify the exact authorized inode;
10. rereads the retained bytes and verifies exact size and SHA-256;
11. synchronizes the target parent and quarantine directories and revalidates bounds.

The rename is the removal commit point. It never overwrites a recovery name. After it succeeds, no cleanup path unlinks, truncates, swaps back, or changes the retained object. A post-commit error can therefore return failure while still retaining the exact recovery inode; the grant remains consumed. Cancellation after worker ownership likewise cannot turn into rollback or destructive cleanup.

## Private namespace

Both `.termux-mcp-trash-quarantine` and mixed-case variants are reserved. The runtime rejects configured roots containing that component and refuses direct traversal. Recursive listing, basename discovery, metadata, UTF-8 and binary reads, ranged reads, hashing, text search, copy source/destination, write target, create target, trash target, and every grant-target helper skip or reject it. The runtime also rejects directory aliases that resolve to the named quarantine identity.

Each retained entry must:

- use `.termux-mcp-trash-artifact-` followed by a canonical lowercase hyphenated UUID;
- be a no-follow regular file;
- have exactly one link; and
- remain at or below 1 MiB.

Any unknown or malformed entry, link, directory, FIFO, socket, oversized object, wrong directory mode, capacity violation, or lock contention fails closed. The runtime never attempts cleanup of such an object.

## Durability and recovery

A successful result requires synchronization of both the held source parent and held quarantine after verified rename. These are the operation's directory durability boundaries. Device or filesystem failure still requires independent backup where stronger recovery is needed.

There is no network or MCP restore/purge capability. Operators must stop the service and all same-UID writers before inspecting recovery material. Because artifacts contain no original-path mapping, restore or deletion must identify one exact artifact through a trusted out-of-band change record and a separate reviewed local procedure. Verify the intended public target is absent, use a same-filesystem no-clobber move, and verify restored bytes and mode. Copy recovery material that must survive device or filesystem loss to independent protected storage before retirement. Recursive removal, broad globbing, age-only cleanup, and maintenance while the runtime is active are outside this contract.

## Stable audit surface

Allowed reasons are `dry_run_preview` and `safe_root_file_trashed_recovery_retained`. Denials use low-cardinality safe-root, missing/type/size/changed-target, mutation-disabled, `capability_*`, worker-capacity, cancellation, quarantine-full/busy, response-limit, and generic trash-failure reasons. Audit events and aggregate counters contain no path, content, digest, grant, principal, session, JTI, key, timestamp, device/inode, artifact name, retained count, retained bytes, or operating-system error.

## Deliberate non-capabilities

`trash_file` does not provide unlink, purge, restore, recursive removal, directory removal, globbing, multiple targets, caller-selected recovery names, caller-selected destinations, overwrite, rename outside the private quarantine, secure erasure, chmod, ownership changes, link following, Android media deletion, remote storage deletion, retention scheduling, or automatic cleanup. Each broader operation requires its own independently gated and reviewed contract.

## Release evidence

Release, device, and native-artifact validation must prove exact schema and disabled discovery, grant issuance and binding denial, preview non-consumption, preflight preservation, exact-limit behavior, randomized no-replace publication, restrictive permissions, namespace isolation, recovery preservation, bounded private responses/audits, deployment dependencies, Android cross-builds, and native Termux execution for the exact commit. Automated core/integration tests separately prove trash replay and concurrent-replay denial, target and parent races, two-grant single-target serialization, cancellation boundaries, capacity/malformed-entry handling, and the broader adversarial matrix; those are not all attributed to the artifact gates.
