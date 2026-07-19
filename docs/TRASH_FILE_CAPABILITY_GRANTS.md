# `trash_file` capability grants

## Purpose and boundary

`trash_file` provides reversible removal of one bounded file from a configured filesystem safe root. Discovery and preview are part of the baseline `mcp-runtime` posture. Live mutation is disabled by default and requires all of the following:

- the `mcp-runtime` build posture;
- static bearer-token authentication;
- `MCP__FILE__TRASH_FILE_MUTATION_ENABLED=true`;
- one configured capability key id and 32-byte HMAC key;
- an initialized MCP session;
- explicit `dry_run:false`; and
- one fresh, single-use `MCP-Capability-Grant` issued locally by the exact server binary for the exact current file.

Omitted `dry_run` and explicit `dry_run:true` validate only. They do not create the recovery directory, move the target, require a grant, or consume one. A bearer token, active session, live gate, or `dry_run:false` is not authorization by itself.

This capability does not unlink data. A successful call atomically moves the exact file into a private recovery quarantine under its original parent. It authorizes no directory removal, recursive operation, overwrite, permanent purge, rename to a caller-selected destination, chmod, content change, shell, Android control, service control, or path outside the configured safe roots.

## Configuration

The private runtime environment must contain:

```dotenv
MCP__AUTH__STATIC_TOKEN=replace-with-a-strong-random-token
MCP__FILE__TRASH_FILE_MUTATION_ENABLED=true
MCP__CAPABILITY__KEY_ID=primary-1
MCP__CAPABILITY__HMAC_KEY_HEX=replace-with-64-lowercase-hex-characters
```

Protect the environment file with mode `0600`. The deployment manager and runtime both fail closed on a malformed boolean, missing static token, partial key pair, invalid key id, or invalid key. The key id is 1–32 lowercase ASCII letters, digits, hyphens, or underscores. The HMAC key is exactly 32 bytes encoded as 64 lowercase hexadecimal characters.

Restarting the process revokes outstanding grants and clears the bounded in-memory replay set. Rotate the key id and key together, restart, verify health and readiness, and discard every prior grant.

## Offline issuance and request

Issuance is not exposed through MCP. Use the exact deployed binary on the same device. The issuer opens, validates, and hashes the target independently; it never accepts caller-supplied identity or digest fields.

```bash
umask 077
GRANT_FILE="$(mktemp "$HOME/.termux-mcp-trash-grant.XXXXXX")"
chmod 600 "$GRANT_FILE"

MCP__CAPABILITY__CONFIG_FILE="$HOME/.config/termux-mcp-edge/runtime.env" \
MCP__CAPABILITY__SESSION_ID='replace-with-active-session-id' \
MCP__CAPABILITY__TRASH_FILE_TARGET='/data/data/com.termux/files/home/mcp-files/obsolete.bin' \
  "$HOME/.local/share/termux-mcp-edge/current/bin/termux-mcp-server" \
  --issue-trash-file-grant >"$GRANT_FILE"
```

The target must be an existing, no-follow, single-link regular file no larger than 1 MiB. The configured safe root and every traversed parent are opened with descriptor-relative no-follow confinement. Issuance fails if the path, object type, link count, size, identity, content, or safe-root posture is unacceptable.

Send the one line in `GRANT_FILE` only as the `MCP-Capability-Grant` header on the matching active-session request:

```json
{
  "jsonrpc": "2.0",
  "id": "trash-obsolete-file",
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

Remove the private grant file after the attempt:

```bash
rm -f -- "$GRANT_FILE"
```

Never place the grant, HMAC key, bearer token, session id, target path, file content, digest, or filesystem identity in command arguments, logs, tickets, screenshots, or release evidence. A grant sent with preview, discovery, initialization, a notification or response, GET, DELETE, or another tool is rejected as invalid capability context and is not consumed.

## Wire representation and exact binding

The header value has the fixed ASCII shape `v1.<key-id>.<payload-hex>.<signature-hex>`. Its payload is exactly 65 bytes (130 lowercase hexadecimal characters):

| Bytes | Value |
|---:|---|
| 16 | cryptographically random grant identifier |
| 1 | globally allocated `trash_file` family code `5` |
| 32 | keyed, domain-separated opaque operation binding |
| 8 | issued Unix seconds, big-endian |
| 8 | expiry Unix seconds, big-endian |

The outer HMAC-SHA-256 authenticates the version, key id, and payload. No raw principal, session, root, path, file identity, content digest, or recovery name is serialized in the payload.

The keyed operation binding covers:

- the random grant identifier;
- a domain-separated digest of the configured static bearer principal;
- the canonical active-session UUID;
- family code `5`, live-mutation posture, and recovery-retained posture;
- the lifetime-pinned safe-root device and inode;
- a domain-separated digest of every normalized root-relative target component;
- the exact target device, inode, size, high-resolution ctime, and link count of one;
- SHA-256 of the exact file bytes; and
- the fixed reversible-trash transaction posture.

The normal lifetime is 60 seconds. Validation permits at most five seconds of future clock skew and rejects a lifetime above 120 seconds. One process retains at most 4,096 unexpired consumed identifiers. Equivalent authorities with the same family, key id, key, and principal share replay, clock, and capacity state. That state is process-local: use one grant consumer for a key/principal domain or an external atomic one-use coordinator before horizontal multi-process operation.

A path, root, principal, session, identity, size, ctime, link-count, content, posture, family, version, key, signature, or time mismatch fails without consuming the grant. Successful validation consumes the identifier immediately before the atomic move. Consumption remains authoritative if any later synchronization, verification, response, timeout, or disconnect fails.

## Filesystem transaction

The runtime retains the selected safe-root descriptor, target-parent descriptor, and exact target-file descriptor from preparation through completion. The target is read and hashed under repeated identity checks. It is accepted only when the held descriptor and current parent entry identify the same single-link regular file of at most 1 MiB.

For a live request the permit-owned blocking worker:

1. preflights the complete path-free 16 KiB response before path access or grant consumption;
2. obtains one fail-fast shared filesystem-mutation worker permit;
3. prepares and hashes the target through its pinned root and no-follow parent descriptors;
4. obtains the poison-fail-closed process-wide filesystem publication lock;
5. preflights per-parent recovery capacity and takes a nonblocking advisory lock on the exact target;
6. revalidates root, parent, held target, named target, size, identity, and content digest;
7. opens or creates `.termux-mcp-trash-quarantine` under the held parent, requiring an exact mode-`0700` directory, and takes its nonblocking advisory lock;
8. rejects unknown names, non-regular entries, links, directories, malformed artifacts, more than 32 retained artifacts, or more than 32 MiB retained bytes;
9. resolves request cancellation against worker commit ownership, then validates and consumes the exact grant;
10. atomically moves the target to a fresh canonical `.termux-mcp-trash-artifact-<uuid>` name with `renameat2(..., RENAME_NOREPLACE)`;
11. verifies that the public target is absent and the retained entry is the exact original inode, mode, size, link count, and content; and
12. synchronizes the held parent and quarantine directories and revalidates recovery bounds before releasing the process lock.

The move is the commit point. There is no automatic rollback, deletion, truncation, chmod, or cleanup after it. A failure after commit can therefore return an error while the original file remains safely retained in quarantine and the grant remains consumed. This is deliberate: automatic pathname-based cleanup or rollback could damage an unrelated object during a same-UID namespace race.

The successful structured result contains only `dryRun`, `sizeBytes`, `recoveryArtifactRetained`, `maxFileBytes`, and `maxResponseBytes`. It never returns the target path, artifact name or path, content, digest, grant, session, root, or filesystem identity.

## Recovery and retention operations

The recovery namespace is intentionally invisible through every MCP filesystem tool, including direct reads, ranges, hashing, metadata, listing, discovery, and search. Its randomized artifacts contain no embedded original path. If recovery may be needed, the operator must retain an independent private change record identifying the intended target; do not weaken the MCP response to expose that mapping.

To restore or retire one retained artifact:

1. quiesce clients and stop the MCP service;
2. stop every other process running under the same Unix UID that can write the safe root;
3. locally inspect only the target parent's `.termux-mcp-trash-quarantine` directory and identify the exact artifact by trusted out-of-band change record, content, size, mode, and time context;
4. verify that the intended restore target is absent;
5. restore on the same filesystem with a no-clobber move, for example `mv -n -- "$ARTIFACT" "$TARGET"`, and verify the restored bytes and mode; or manually remove only the specifically reviewed artifact when retention is no longer required; and
6. verify recovery-directory ownership, mode `0700`, bounds, service health, and readiness before re-enabling mutation.

Do not use recursive removal, broad globs, or automated age-based deletion. Back up any recovery object that must survive device or filesystem loss before removing it. The 32-artifact/32-MiB ceiling is per target parent, not a global disk limit; capacity exhaustion returns a private bounded HTTP 507 error before grant consumption.

The advisory locks coordinate cooperating runtime instances only. A hostile or accidental process under the same Unix UID can ignore them, rename namespace entries, or exhaust storage. Production mutation roots must therefore be under the service's exclusive operational ownership while any live filesystem gate is enabled.

## Errors, audit, and status

Authorization failures return HTTP 403 with JSON-RPC code `-32003` and one stable reason such as `trash_file_mutation_disabled`, `capability_grant_missing`, `capability_grant_malformed`, `capability_grant_signature_invalid`, `capability_grant_binding_mismatch`, or `capability_grant_replayed`. Target replacement returns a stable conflict; capacity exhaustion returns HTTP 507; quarantine contention returns a stable conflict. Public errors never include private paths, content, artifact names, identities, host I/O details, or grant material.

`runtime_status` reports only bounded posture metadata: whether the gate and grant requirement are active, header name, TTL, binding posture, 1 MiB target ceiling, 16 KiB response ceiling, 32-artifact/32-MiB recovery limits, and the path-and-artifact-free response posture.

Aggregate audits retain only the tool, gate, dry-run/live mode, allowed/denied decision, and stable reason code. Successful mutation records `safe_root_file_trashed_recovery_retained`. Audits never retain caller arguments, paths, bytes, digests, grants, key material, principal fingerprints, sessions, JTIs, timestamps, artifact names, recovery usage, or filesystem identities.

## Production validation requirements

Before enabling live trash mutation, prove all of the following against the exact release artifact:

- disabled discovery constrains `dry_run` to `true`, and explicit mutation fails before path access;
- enabled discovery and status expose only the documented bounded posture;
- preview is nonmutating, creates no recovery state, and does not consume a valid grant;
- malformed, wrong-family, wrong-principal, wrong-session, wrong-root, wrong-path, changed-identity, changed-content, expired, future, invalid-signature, replayed, and concurrent-replay grants fail closed without private reflection;
- missing targets, roots, directories, symlinks, hard-linked files, special objects, outside paths, and files above 1 MiB are rejected;
- the exact 1 MiB boundary succeeds in preview and live operation;
- oversized caller-controlled response identifiers fail before path access or grant consumption;
- stale target, publication-lock wait cancellation, advisory-lock contention, and recovery-capacity failures precede grant consumption and permit an exact retry where appropriate;
- eight or more concurrent uses of one grant produce one move and one retained artifact;
- success retains the exact original inode, bytes, mode, size, and single-link identity under a mode-`0700` recovery directory while removing the public name;
- exact and mixed-case quarantine names plus symlink aliases remain hidden across every filesystem surface;
- unknown, symlinked, directory, hard-linked, and oversized recovery entries fail closed without cleanup;
- responses and audits remain path-, artifact-, content-, digest-, grant-, session-, principal-, and identity-free; and
- deployment preflight rejects a malformed gate or enabled trash mutation without static authentication and the complete capability key pair.
