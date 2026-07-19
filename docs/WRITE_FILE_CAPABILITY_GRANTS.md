# `write_file` capability grants

## Purpose

`write_file` is always available as a bounded UTF-8 validation preview in the staged MCP runtime. A live write is a separate Class 2 authority. It is disabled by default and requires all of the following:

- the `mcp-runtime` build posture;
- static bearer-token authentication;
- `MCP__FILE__WRITE_MUTATION_ENABLED=true`;
- one configured capability key id and 32-byte HMAC key;
- an active MCP session;
- explicit `dry_run:false`; and
- one fresh, single-use `MCP-Capability-Grant` issued offline by the exact server binary for the exact target, content, and create-or-replace disposition.

Omitted `dry_run` and explicit `dry_run:true` never mutate, never require a grant, and never consume one. A valid transport bearer token, active session, or `dry_run:false` is not a substitute for the request grant.

This gate authorizes no deletion, rename, chmod, directory mutation, binary upload, shell, Android control, service control, package operation, process action, network action, or path outside the configured safe roots.

## Configuration

The deployed private runtime environment must contain:

```dotenv
MCP__AUTH__STATIC_TOKEN=replace-with-a-strong-random-token
MCP__FILE__WRITE_MUTATION_ENABLED=true
MCP__CAPABILITY__KEY_ID=primary-1
MCP__CAPABILITY__HMAC_KEY_HEX=replace-with-64-lowercase-hex-characters
```

Protect the environment file with mode `0600`. The key id is a bounded lowercase identifier. The HMAC key is exactly 32 bytes encoded as 64 lowercase hexadecimal characters. A partial, malformed, or incompatible configuration fails startup. Disabling mutation does not make malformed capability-key configuration acceptable.

Restarting the process revokes every outstanding grant and clears the bounded in-memory replay set. Rotate the key id and key together, restart the service, confirm readiness, and discard every grant issued under the prior key.

## Offline issuance

Issuance is deliberately unavailable over MCP. Use the exact deployed binary on the device. Put the exact UTF-8 content in a private regular file rather than a command argument or environment value, then select the intended target disposition:

```bash
umask 077
GRANT_FILE="$(mktemp "$HOME/.termux-mcp-write-grant.XXXXXX")"
CONTENT_FILE="$(mktemp "$HOME/.termux-mcp-write-content.XXXXXX")"
chmod 600 "$GRANT_FILE" "$CONTENT_FILE"

# Write the exact intended UTF-8 bytes to CONTENT_FILE without shell tracing.

MCP__CAPABILITY__CONFIG_FILE="$HOME/.config/termux-mcp-edge/runtime.env" \
MCP__CAPABILITY__SESSION_ID='replace-with-active-session-id' \
MCP__CAPABILITY__WRITE_FILE_TARGET='/data/data/com.termux/files/home/mcp-files/output.txt' \
MCP__CAPABILITY__WRITE_FILE_CONTENT_FILE="$CONTENT_FILE" \
MCP__CAPABILITY__WRITE_FILE_DISPOSITION='create' \
  "$HOME/.local/share/termux-mcp-edge/current/bin/termux-mcp-server" \
  --issue-write-file-grant >"$GRANT_FILE"
```

Use `create` only when the final path is absent. Use `replace` only when the final path is an existing regular file. Issuance validates the same anchored safe-root, no-follow parent, target kind, bounded payload, and normalized target contract used by the runtime. The private content file must be a bounded, stable, no-follow regular file containing valid UTF-8. Its bytes are hashed into the grant binding; they are never serialized into the grant.

Send the single line in `GRANT_FILE` only as the `MCP-Capability-Grant` header on the matching `tools/call` request. The JSON `content` must have exactly the same UTF-8 bytes used during issuance:

```json
{
  "jsonrpc": "2.0",
  "id": "write-output",
  "method": "tools/call",
  "params": {
    "name": "write_file",
    "arguments": {
      "path": "/data/data/com.termux/files/home/mcp-files/output.txt",
      "content": "exact intended content",
      "dry_run": false
    }
  }
}
```

Remove the private grant and content files after the attempt:

```bash
rm -f -- "$GRANT_FILE" "$CONTENT_FILE"
```

Never paste grants, keys, bearer tokens, content, or private target paths into tickets, logs, command arguments, screenshots, or retained reports.

## Wire representation and privacy

The `MCP-Capability-Grant` value has the fixed ASCII shape
`v1.<key-id>.<payload-hex>.<signature-hex>`. The payload is exactly 64 bytes,
encoded as exactly 128 lowercase hexadecimal characters. It contains only:

| Bytes | Value |
|---:|---|
| 16 | fresh random grant identifier |
| 32 | keyed, domain-separated HMAC-SHA-256 operation binding |
| 8 | issued Unix seconds, big-endian |
| 8 | expiry Unix seconds, big-endian |

The outer 32-byte HMAC-SHA-256 signature authenticates the version, key id, and
payload. The payload is intentionally opaque: the operation binding is not a
serialized concatenation or digest of the bound request fields. In particular,
the wire payload never serializes the raw principal or its binding digest,
canonical session, root or target, content digest, disposition, replacement
device/inode/size/high-resolution ctime/link-count identity, or capability and
posture codes. The issued and expiry timestamps above are the only time values
on the wire; replacement ctime is only operation-binding input. The random ID
makes two grants for the same request unlinkable from their payload bindings.

## Exact binding

The keyed operation binding conceptually binds all of these values; this is
authorization input, not a claim about their serialization in the payload:

- the fresh random 128-bit grant identifier;
- a domain-separated digest of the configured static bearer principal;
- the canonical 128-bit MCP session UUID;
- the globally allocated `write_file` capability code `3`;
- the anchored safe-root device and inode;
- a domain-separated digest of the normalized root-relative target components;
- the SHA-256 digest of the exact UTF-8 content bytes;
- exact `create` or `replace` disposition;
- the mutating posture;
- for `replace`, an exact existing regular-file identity: device, inode, size,
  ctime seconds, ctime nanoseconds, and link count (which must be one).

The code is allocated by the same internal registry that assigns directory
creation `1`, Android volume `2`, and reserves file copy `4`. Pairwise
uniqueness is an invariant test and callers cannot supply or alter the family
identifier.

Version and key id are covered by the outer signature; issuance and expiry are
validated from the signed payload. For `create`, the replacement-identity
position in the binding has a fixed absent encoding, so create and replace
cannot cross-authorize.

The normal lifetime is 60 seconds. Validation allows at most five seconds of future clock skew and rejects lifetimes above 120 seconds. One process retains at most 4,096 unexpired consumed grant identifiers. Expired replay entries are pruned before a new valid grant is recorded; a full or poisoned replay state fails closed.

The runtime first acquires the single shared non-queueing filesystem-mutation worker permit; exhaustion returns private HTTP 503 / JSON-RPC `-32007` before preparation, grant consumption, or mutation. Inside the permit-owned blocking worker it validates safe-root and parent descriptors, target disposition and identity, content size, recovery-quarantine capacity, and grant binding. After descriptor preparation, one atomic guard resolves request cancellation against worker commit ownership. A cancellation winner consumes no grant and changes no filesystem state. A worker winner atomically consumes the grant immediately before publication work and continues independently of later timeout or disconnect; consumption remains authoritative if staging, synchronization, publication, verification, response delivery, or any later step fails.

## Filesystem transaction

The runtime holds the anchored safe-root and destination-parent descriptors throughout the operation. It never re-resolves an authorized absolute pathname for mutation. Every target parent has one reserved `.termux-mcp-write-quarantine` namespace. That directory is opened without following links, must be mode `0700`, and is invisible and inaccessible through every MCP filesystem operation.

For a live write it:

1. classifies the exact final entry without following links; replacement accepts only a single-link regular file of at most 1 MiB and captures its exact identity;
2. rejects a full, malformed, contended, or unavailable recovery quarantine before authorization, resolves cancellation against worker ownership, then consumes the exact request grant immediately before the first publication mutation;
3. opens or creates the fixed private quarantine, takes its nonblocking advisory lock, and rechecks its contents and capacity;
4. creates one randomized `.termux-mcp-write-artifact-*` regular staging entry at mode `0600`, writes the bounded bytes, synchronizes it, and verifies its held and named identity, type, size, and mode;
5. for `create`, publishes that entry with atomic `NOREPLACE`, leaving no retained recovery artifact;
6. for `replace`, revalidates the bound target and performs one irreversible `EXCHANGE`; the authorized new inode becomes the target and the displaced prior inode/content remains under the randomized quarantine name;
7. verifies that the final entry is the exact staged file with the expected type, identity, size, and mode; and
8. synchronizes the held target parent and quarantine directories and revalidates the quarantine bounds.

The quarantine retains at most 32 regular artifacts and 32 MiB per target parent. These are per-parent limits, not a global disk bound. Unknown names, links, directories, special objects, capacity exhaustion, or nonblocking lock contention fail closed. The advisory lock serializes cooperating runtime writers only; a process under the same Unix UID can ignore it, alter names, or cause denial of service.

Replacement has one commit point: the `EXCHANGE`. There is no automatic rollback after it and no later unlink, truncation, or metadata mutation of the captured object. The staging entry is created at mode `0600`, but after exchange the recovery artifact is the displaced prior inode and keeps its prior mode and other metadata. This avoids mutating the live target before commit or altering captured recovery metadata afterward; confidentiality therefore depends on the enclosing mode-`0700` quarantine. A failure after that commit can leave the authorized new inode at the public target while the displaced object remains quarantined; the request still fails and its grant remains consumed. This preservation rule avoids deleting an unrelated object during a hostile same-UID namespace race, but POSIX pathname operations cannot provide an inode-conditional rollback against such a peer.

Successful target-parent and quarantine synchronization is the crash-durability boundary. A successful replacement returns `recoveryArtifactRetained:true`; create and preview return `false`. The artifact is recovery material, not an automatically managed rollback version. Operators should still use preview, exact content review, the disposition-bound grant, and an external backup for independent recovery.

## Recovery-artifact maintenance

Do not remove quarantine entries while the runtime or another same-UID writer may be active. To reclaim capacity:

1. quiesce clients, stop the MCP service, and stop other writers running under the same Unix UID;
2. inspect the target parent's `.termux-mcp-write-quarantine` directory locally and identify the specific retained artifact to preserve or remove;
3. manually remove only the selected artifact, without broad globs or recursive deletion; and
4. restart the service and confirm health and readiness before re-enabling writes.

Recovery artifacts contain prior file content and are protected only by the quarantine's Unix ownership and permissions. Back up any artifact that must survive device or filesystem failure before deleting it.

## Errors and audit privacy

Authorization failures return HTTP 403 with JSON-RPC code `-32003` and one stable reason. Filesystem contract failures use stable bounded categories. Responses do not echo the grant, key, principal binding, session, target digest, content digest, content, artifact name, replacement device/inode/size/ctime/link count, private path, or host error.

Representative authorization reasons include:

| Condition | Reason |
|---|---|
| Mutation gate disabled | `write_file_mutation_disabled` |
| Grant absent | `capability_grant_missing` |
| Malformed grant | `capability_grant_malformed` |
| Unknown version or key | `capability_grant_version_unknown` / `capability_grant_key_unknown` |
| Invalid signature | `capability_grant_signature_invalid` |
| Expired, future, or excessive lifetime | `capability_grant_expired` / `capability_grant_future_issued` / `capability_grant_lifetime_exceeded` |
| Principal, session, root, target, content, disposition, or posture mismatch | `capability_grant_binding_mismatch` |
| Reuse | `capability_grant_replayed` |
| Clock rollback | `capability_clock_rollback` |
| Replay capacity exhausted | `capability_replay_capacity_exhausted` |
| Replay state unavailable | `capability_state_unavailable` |

A full, contended, malformed, or unavailable recovery quarantine is a filesystem denial, not a grant authorization result. Capacity exhaustion uses a stable bounded HTTP 507 response and does not disclose the parent, artifact names, retained bytes, or counts.

In-memory aggregate audit counters retain only the stable tool name, gate, dry-run/mutating mode, allowed/denied decision, and stable reason code. They never retain raw paths, content, byte digests, grants, keys, principal fingerprints, sessions, JTIs, timestamps, artifact names, retained counts/bytes, or filesystem identities.

## Validation requirements

Before enabling mutation in production, validate:

- disabled discovery advertises dry-run only and rejects explicit mutation before path access;
- enabled discovery documents the header grant and still defaults to preview;
- every malformed, stale, mismatched, replayed, and concurrent-replay grant fails closed;
- preview does not consume a valid grant;
- `create` and `replace` are distinct and cannot authorize each other;
- content changes cannot reuse a grant;
- the exact 1 MiB UTF-8 payload succeeds and one byte above fails;
- final symlinks, symlinked parents, directories, FIFOs, missing parents, and outside paths are rejected;
- target and staging-name exchanges cannot publish or delete a foreign object;
- success always produces the exact requested bytes at fixed mode `0600`;
- create returns `recoveryArtifactRetained:false`, retains no artifact, and cannot replace an existing entry;
- replacement returns `recoveryArtifactRetained:true` and preserves the displaced prior inode/content in the reserved quarantine;
- the quarantine is mode `0700`, hidden from all MCP filesystem operations, and rejects unknown entries, special objects, lock contention, more than 32 artifacts, or more than 32 MiB per target parent;
- a replacement target with multiple hard links or a size above 1 MiB is rejected;
- oversized caller-controlled response identifiers fail before grant consumption or staging;
- cancellation before worker commit consumes no grant and changes nothing; cancellation after worker ownership never triggers destructive artifact cleanup or grant replay, and tests account for the possible authorized-new-target plus retained-old-artifact state; and
- audit/status responses remain free of all caller, credential, grant, digest, and filesystem-identity material.
