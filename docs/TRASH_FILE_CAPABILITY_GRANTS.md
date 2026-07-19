# `trash_file` capability grants

Live `trash_file` is a narrowly bounded Class 2 mutation that moves one exact regular file into private recovery storage. It is not unlink, recursive deletion, caller-directed rename, purge, or restore. Live mutation requires every boundary below:

1. an `mcp-runtime` build;
2. static bearer authentication;
3. `MCP__FILE__TRASH_FILE_MUTATION_ENABLED=true`;
4. one valid capability key identifier and 32-byte HMAC key;
5. an active canonical MCP session;
6. explicit `dry_run:false`; and
7. one fresh single-use grant issued by the exact deployed binary for the exact file.

Omitted `dry_run` and explicit `dry_run:true` are fully validated previews. They neither mutate nor consume a grant. Enabling create, copy, write, or Android volume mutation does not enable trashing, and grants for those families cannot authorize it.

## Runtime configuration

```dotenv
MCP__AUTH__STATIC_TOKEN=replace-with-a-strong-private-token
MCP__FILE__TRASH_FILE_MUTATION_ENABLED=true
MCP__CAPABILITY__KEY_ID=primary-1
MCP__CAPABILITY__HMAC_KEY_HEX=replace-with-64-lowercase-hex-characters
```

The trash gate defaults to `false`. Enabling it without `mcp-runtime`, static authentication, or both capability-key settings fails startup. A partial or malformed key configuration fails closed even while mutation is disabled. Protect the runtime environment file with mode `0600`. The key identifier is 1–32 lowercase ASCII letters, digits, hyphens, or underscores; the HMAC key is exactly 32 bytes encoded as 64 lowercase hexadecimal characters. The deployment validator applies the same dependency checks before installation, upgrade, or rollback. Rotate the key identifier and key together, restart, verify health/readiness, and discard all earlier grants when immediate revocation is required.

When disabled, discovery retains preview but constrains `dry_run` to `true`; explicit mutation returns `trash_file_mutation_disabled` before path access. When enabled, discovery removes that constraint and documents the exact identity/content-bound grant. `runtime_status` exposes only stable posture, header, TTL, limits, recovery policy, and aggregate audit counters.

## Exact-binary offline issuance

Issuance is local CLI functionality and is never registered as an MCP tool:

```bash
umask 077
GRANT_FILE="$(mktemp "$HOME/.cache/termux-mcp-trash-grant.XXXXXX")"
chmod 600 "$GRANT_FILE"

MCP__CAPABILITY__CONFIG_FILE="$HOME/.config/termux-mcp-edge/runtime.env" \
MCP__CAPABILITY__SESSION_ID="$MCP_SESSION_ID" \
MCP__CAPABILITY__TRASH_FILE_TARGET="$ABSOLUTE_TARGET" \
  "$HOME/.local/share/termux-mcp-edge/current/bin/termux-mcp-server" \
  --issue-trash-file-grant >"$GRANT_FILE"
```

The configuration file must be absolute, owner-private, no larger than 64 KiB, and a final-component-no-follow regular file. Its bounded literal parser accepts only allowlisted `NAME=value` records and never evaluates shell syntax.

The issuer independently constructs the safe-root authority from deployed configuration. It lifetime-pins the selected root, traverses without following links, opens and holds the final file, requires a single-link regular file no larger than 1 MiB, reads the exact bytes, captures device, inode, size, high-resolution ctime, and link count, and computes SHA-256 itself. Callers cannot supply identity or digest fields. If a configured root pathname was replaced after server startup, a newly issued grant cannot authorize the server's earlier pinned root object.

Send the one-line grant only as the header on the exact live call:

```http
MCP-Capability-Grant: v1.<key-id>.<opaque-payload>.<mac>
```

```json
{
  "jsonrpc": "2.0",
  "id": "trash-one-file",
  "method": "tools/call",
  "params": {
    "name": "trash_file",
    "arguments": {
      "path": "/data/data/com.termux/files/home/mcp-files/obsolete.bin",
      "dry_run": false
    }
  }
}
```

Remove the grant file after the attempt with `rm -f -- "$GRANT_FILE"`. Never place the grant, key, bearer token, session, target, identity, or digest in JSON arguments, URLs, process arguments, logs, screenshots, tickets, or retained evidence. A grant attached to preview, discovery, initialization, a notification or client response, GET, DELETE, or another tool is rejected as invalid capability context without consumption.

## Opaque wire format and exact binding

The signed payload is exactly 65 bytes encoded as 130 lowercase hexadecimal characters:

| Bytes | Value |
| ---: | --- |
| 16 | random grant identifier |
| 1 | globally allocated `trash_file` family code `5` |
| 32 | keyed domain-separated operation binding |
| 8 | issuance Unix seconds, big-endian |
| 8 | expiry Unix seconds, big-endian |

The outer HMAC-SHA-256 authenticates the version, key identifier, and payload. The operation binding covers, without serializing:

- the keyed static principal;
- canonical active-session UUID;
- trash family `5`, mutating posture, and fixed recovery-retained posture;
- lifetime-pinned root device and inode;
- a domain-separated length-prefixed digest of normalized relative path components;
- target device, inode, exact size, ctime seconds, ctime nanoseconds, and link count `1`;
- SHA-256 of the exact file bytes;
- random grant identifier, issuance, and expiry.

Directory `1`, write `2`, volume `3`, copy `4`, and trash `5` are allocated in one internal registry. Every cross-family use is rejected without consuming the source grant.

The normal lifetime is 60 seconds. Validation permits at most five seconds of future skew and rejects zero or greater-than-120-second lifetimes. One process retains at most 4,096 unexpired consumed trash identifiers. Equivalent authorities with the same family, key identifier, HMAC key, and principal share replay, clock-rollback, and capacity state through the bounded process-global registry. Different families and authority domains remain isolated. Full, poisoned, unavailable, capacity-mismatched, or namespace-exhausted state fails closed.

Replay coordination is process-local. Run one consuming process per effective authority or add an external atomic one-use coordinator. A restart clears process state; rotate the key during restart when all outstanding grants must be revoked.

## Validation, cancellation, and commit order

For explicit mutation, the runtime performs this order within the required single-consuming-process, exclusively owned mutation root deployment:

1. authenticate the HTTP request and validate the exact served listener boundary;
2. validate Host/Origin, method, media types, body and concurrency limits, JSON-RPC, protocol version, and active session;
3. accept exactly one bounded ASCII capability header only on `tools/call` for live `trash_file`;
4. validate the closed schema and independent gate;
5. preflight the complete 16 KiB response with the caller's real JSON-RPC id;
6. acquire the single fail-fast filesystem-mutation worker permit;
7. anchor the lifetime-pinned root and prepare held root, parent, and target descriptors without following links;
8. validate regular-file type, one-link identity, exact size, high-resolution ctime, and SHA-256;
9. acquire the poison-fail-closed publication lock shared by create, copy, trash, and write;
10. inspect and nonblockingly lock an existing trash quarantine, or classify an absent quarantine without creating it; reserve its fixed capacity;
11. nonblockingly lock and revalidate the held and named target identity/content;
12. atomically resolve request cancellation against worker ownership and consume the exact grant;
13. only after successful consumption, create an absent private quarantine if needed, allocate an unpredictable absent recovery name, and atomically rename the target with `NOREPLACE`;
14. verify held, removed-name, retained-name, identity, bytes, and bounds; synchronize the held target parent and quarantine directories.

Every pre-consumption step is read-only with respect to filesystem namespace and content. In that supported ownership model, missing, malformed, stale, mismatched, preview, oversized-response, capacity, lock-contention, worker-capacity, and cancellation-before-ownership outcomes consume no valid grant and do not create an empty quarantine. Once ownership and grant consumption succeed, later timeout or disconnect cannot cancel the worker. Consumption remains final after every post-consumption result, and any successfully moved file remains retained; no cleanup path unlinks a recovery or foreign object. Multiple independent consuming processes do not share the publication mutex or replay registry and are outside this guarantee.

## Recovery transaction

The quarantine is the fixed `.termux-mcp-trash-quarantine` directory under the target's held parent. It is opened with no-follow semantics, must be mode `0700`, and is hidden by name and directory identity across every MCP filesystem surface. It admits at most 32 canonical regular artifacts and 32 MiB per parent. Every retained entry must use a lowercase canonical UUID name of the form `.termux-mcp-trash-artifact-<uuid>`, be no larger than 1 MiB, and have one link.

The target is moved with descriptor-relative `renameat2(..., RENAME_NOREPLACE)` into a randomized absent name. The operation does not copy, truncate, unlink, chmod, recurse, follow links, or resolve the absolute path again. The exact inode and its existing metadata move into the private directory. Success verifies the still-held descriptor and retained name refer to the bound inode, rereads and hashes the retained bytes, confirms the public target name is absent, synchronizes both directories, and revalidates quarantine bounds.

Unknown names, symlinks, directories, FIFOs, sockets, oversized files, hard-linked artifacts, mode drift, capacity exhaustion, and advisory-lock contention fail closed without cleanup. A same-UID peer can ignore advisory locks or race POSIX namespace operations; production mutation roots therefore require exclusive operational ownership while any live filesystem mutation gate is enabled.

There is intentionally no MCP purge or restore operation. Recovery maintenance is local and manual:

1. quiesce clients and stop the service and other same-UID writers;
2. inspect the specific parent quarantine locally and identify the exact artifact using a trusted out-of-band change record plus content, size, mode, and time context—the artifact itself contains no original-path mapping;
3. verify the intended public target is absent, restore with a same-filesystem no-clobber move, and verify the restored bytes and mode, or copy required recovery material to independent protected storage before retirement;
4. remove only a specifically verified artifact—never an age-only rule, broad glob, or recursive directory; and
5. restart and verify readiness before re-enabling mutation.

## Stable denials and privacy

Authorization failures use HTTP 403, JSON-RPC `-32003`, and a stable reason:

| Condition | Stable reason |
| --- | --- |
| Gate disabled | `trash_file_mutation_disabled` |
| Grant absent | `capability_grant_missing` |
| Malformed or oversized | `capability_grant_malformed` |
| Unknown version or key | `capability_grant_version_unknown` / `capability_grant_key_unknown` |
| Invalid signature | `capability_grant_signature_invalid` |
| Expired, future, or excessive lifetime | `capability_grant_expired` / `capability_grant_future_issued` / `capability_grant_lifetime_exceeded` |
| Principal/session/root/path/identity/content/posture mismatch | `capability_grant_binding_mismatch` |
| Reuse | `capability_grant_replayed` |
| Clock rollback | `capability_clock_rollback` |
| Replay capacity exhausted | `capability_replay_capacity_exhausted` |
| Replay state unavailable | `capability_state_unavailable` |

Filesystem outcomes use bounded stable categories for missing/unsupported/oversized/changed targets, safe-root rejection, busy or full recovery storage, response size, worker capacity, cancellation, and internal failure. Recovery capacity returns HTTP 507 before consumption; quarantine contention and changed-target failures return HTTP 409. Results report only `dryRun`, `sizeBytes`, `recoveryArtifactRetained`, and fixed limits. Responses, errors, debug output, status, audits, fixtures, and evidence never contain path, content, digest, grant, key, principal fingerprint, session, JTI, timestamps, artifact name, target/quarantine identity, counts, or host errors.

## Public Rust boundary

`FileSystemTools::trash_file` is preview-only. Passing `Some(false)` returns `TrashMutationAuthorizationRequired`. Prepared mutation types and authorization-aware execution are crate-private; `TrashFileGrantTarget` is public only as an opaque issuer input with private construction fields. The sole public `McpRouterBuilder` accepts a trash authority only when it binds the exact configured static transport principal.

## Required evidence

Automated core/integration validation must cover every header, family, principal, session, root, path, identity, content, posture, time, signature, replay, concurrent-replay, rollback, and capacity denial; replacement, hard-link, symlink, directory, FIFO, disappearance, and two-grants/one-target races; cancellation on both sides of ownership; quarantine shape, capacity, namespace, durability, and foreign-entry preservation; bounded private responses/audits; and public API opacity. Release, device, and native-artifact validation directly covers default-disabled posture, enabled status, exact-binary issuance, binding denial, preview non-consumption, preflight preservation, exact 1 MiB and plus-one behavior, successful recovery retention, private evidence, deployment dependencies, exact-head CI/Security, Android builds, and native ARM64 Termux execution. It does not independently exercise reuse of a consumed trash grant; replay and concurrent replay are supplied by the automated core/integration evidence.
