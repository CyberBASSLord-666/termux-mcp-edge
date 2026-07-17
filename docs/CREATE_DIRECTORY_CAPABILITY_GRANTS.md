# `create_directory` request-capability grants

Explicit directory creation is protected by two independent controls in addition to authenticated MCP transport and safe-root confinement:

1. the default-disabled runtime gate `MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=true`; and
2. one short-lived, request-scoped, single-use grant in the `MCP-Capability-Grant` HTTP header.

`dry_run` omitted or set to `true` remains a validation-only operation and does not require or consume a grant. `dry_run:false` is insufficient by itself. There is no network tool for issuing grants, and grant material is never accepted in tool arguments, URLs, JSON-RPC bodies, responses, logs, or audit labels.

## Runtime configuration

The mutation posture requires an `mcp-runtime` build, static bearer authentication, and all three settings below:

```dotenv
MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=true
MCP__CAPABILITY__KEY_ID=primary-1
MCP__CAPABILITY__HMAC_KEY_HEX=replace-with-64-lowercase-hex-characters
```

The key identifier must match `^[a-z0-9_-]{1,32}$`. The HMAC key must be exactly 32 random bytes encoded as 64 lowercase hexadecimal characters. To generate one locally without printing the bearer token:

```bash
umask 077
dd if=/dev/urandom bs=32 count=1 status=none | sha256sum | awk '{print $1}'
```

Keep the resulting key only in the owner-readable `runtime.env`. Startup and deployment validation fail closed for a missing pair, malformed value, duplicate configuration entry, localhost-only unauthenticated posture, or an enabled mutation gate in a binary without `mcp-runtime`.

When the gate is disabled, `tools/list` retains `create_directory` for preview but constrains `dry_run` to `true`; direct mutation returns HTTP 403 with `create_directory_mutation_disabled`. When enabled, discovery removes that constraint and states that a request grant is required. `runtime_status` exposes only the boolean posture, public header name, public 60-second lifetime, and stable mode—not key or grant material.

## Issuing one grant

Initialize and activate an authenticated MCP session first. Then run the exact server binary locally with the session and absent target supplied through dedicated environment variables:

```bash
GRANT_FILE="$(mktemp "$HOME/.termux-mcp-create-grant.XXXXXX")"
chmod 600 "$GRANT_FILE"

MCP__CAPABILITY__SESSION_ID="$MCP_SESSION_ID" \
MCP__CAPABILITY__CREATE_DIRECTORY_TARGET="$ABSOLUTE_ABSENT_TARGET" \
  "$HOME/.local/share/termux-mcp-edge/current/termux-mcp-server" \
  --issue-create-directory-grant >"$GRANT_FILE"
```

The issuer loads the same static principal, safe roots, mutation gate, key identifier, and HMAC key as the service. It anchors the configured roots, resolves the exact existing parent without following links, rejects an existing or out-of-root target, and writes exactly one grant line to standard output. Errors are generic and do not echo the target, session, key, or bearer token.

Submit the mutating request once:

```bash
curl --fail-with-body --silent --show-error \
  -H "Authorization: Bearer $MCP_TEST_TOKEN" \
  -H "MCP-Session-Id: $MCP_SESSION_ID" \
  -H 'MCP-Protocol-Version: 2025-11-25' \
  -H "MCP-Capability-Grant: $(<"$GRANT_FILE")" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  --data-binary "$(jq -cn --arg path "$ABSOLUTE_ABSENT_TARGET" \
    '{jsonrpc:"2.0",id:"create",method:"tools/call",params:{name:"create_directory",arguments:{path:$path,dry_run:false}}}')" \
  http://127.0.0.1:8000/mcp

rm -f -- "$GRANT_FILE"
unset MCP__CAPABILITY__SESSION_ID MCP__CAPABILITY__CREATE_DIRECTORY_TARGET
```

Do not paste grants into tickets, terminal transcripts, command history, process arguments, or retained logs. The release validator and device gate stage them in mode-`0600` temporary files and remove them with isolated state.

## Cryptographic binding

The fixed-shape `v1.<kid>.<payload>.<mac>` grant uses HMAC-SHA-256. The authenticated payload binds all of:

- a random 128-bit JTI;
- the SHA-256 fingerprint of the configured static bearer principal;
- the canonical MCP session UUID;
- the `filesystem.create-directory` capability identifier;
- the selected safe root's device and inode identity;
- a domain-separated, length-prefixed SHA-256 digest of normalized root-relative target components;
- the mutating posture;
- issuance and expiry seconds;
- the signed version and key identifier.

The normal lifetime is 60 seconds. The validator rejects zero or greater-than-120-second lifetimes, issuance more than 5 seconds in the future, expiry at the current second, an unknown version or key, and any signature or binding mismatch. One process retains at most 4,096 unexpired consumed JTIs. A full replay set fails closed; expired entries are pruned before a new valid grant is recorded.

## Validation and consumption order

For mutation, the runtime performs this order:

1. authenticate the HTTP request;
2. validate Host/Origin, media types, body limits, JSON-RPC, protocol version, and active session;
3. accept exactly one bounded ASCII capability header only for `tools/call` → `create_directory`;
4. validate the closed tool schema and preflight the complete 16 KiB response;
5. resolve safe-root confinement, open and hold the exact parent descriptor, prove the final target is absent, and compute the target binding;
6. verify and atomically consume the JTI under the replay lock;
7. immediately attempt the first filesystem mutation using the held descriptor.

A target mismatch, malformed grant, or wrong request context does not consume a valid grant. A dry run does not consume it. Once step 6 succeeds, the grant remains consumed even if directory staging, verification, sync, publication, response serialization, or cleanup later fails. Concurrent replay permits at most one mutation attempt.

## Stable denials

Authorization failures return HTTP 403, JSON-RPC code `-32003`, and only a stable reason. Header shape or wrong-context failures return HTTP 400 without reflecting the header.

| Condition | Stable reason |
|---|---|
| Gate disabled | `create_directory_mutation_disabled` |
| Grant absent | `capability_grant_missing` |
| Malformed or oversized | `capability_grant_malformed` |
| Unknown format version | `capability_grant_version_unknown` |
| Unknown key identifier | `capability_grant_key_unknown` |
| Invalid MAC | `capability_grant_signature_invalid` |
| Expired | `capability_grant_expired` |
| Issued too far in the future | `capability_grant_future_issued` |
| Invalid or excessive lifetime | `capability_grant_lifetime_exceeded` |
| Principal/session/root/target/posture mismatch | `capability_grant_binding_mismatch` |
| JTI already consumed | `capability_grant_replayed` |
| Wall clock moved backward | `capability_clock_rollback` |
| Bounded replay set full | `capability_replay_capacity_exhausted` |
| Replay state unavailable | `capability_state_unavailable` |

Audit counters retain only the stable reason, tool, gate, dry-run/mutating mode, decision, and count. They never retain grants, keys, bearer fingerprints, session identifiers, paths, target digests, JTIs, timestamps, or host errors.

## Rotation and restart

Only one key identifier is active in a process. Rotate by replacing both capability key settings atomically and restarting the service; all grants signed under the old key then fail as unknown-key grants. Changing the static bearer token also changes the principal binding. A restart clears the in-memory replay set, but pre-restart grants still expire within their short lifetime; operators that require immediate invalidation should rotate the key during restart.

The grant is deliberately narrower than a general capability-token framework. It authorizes only one already-confined, absent directory target and does not authorize copy, write, delete, rename, permissions, recursive creation, shell, service, package, process, network, or Android control.
