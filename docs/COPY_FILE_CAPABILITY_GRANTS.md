# `copy_file` capability grants

Live `copy_file` is a narrowly scoped Class 2 mutation. It requires all of the following independently:

1. an `mcp-runtime` build;
2. static-token authentication;
3. `MCP__FILE__COPY_FILE_MUTATION_ENABLED=true`;
4. one valid capability key identifier and 32-byte HMAC key; and
5. one fresh, single-use `MCP-Capability-Grant` issued by the exact server binary for the exact source and absent destination.

Omitted `dry_run` and explicit `dry_run:true` remain previews. They never require or consume a grant and never publish a destination. Explicit `dry_run:false`, a bearer token, and an active MCP session are not substitutes for the copy grant. Enabling create, write, or Android-volume mutation does not enable copy.

## Runtime configuration

```bash
MCP__AUTH__STATIC_TOKEN=replace-with-a-strong-private-token
MCP__FILE__COPY_FILE_MUTATION_ENABLED=true
MCP__CAPABILITY__KEY_ID=primary-1
MCP__CAPABILITY__HMAC_KEY_HEX=replace-with-64-lowercase-hex-characters
```

The copy gate defaults to `false`. Enabling it without the `mcp-runtime` feature, valid static authentication, or the complete capability key pair fails startup. Partial or malformed capability key configuration fails even while the gate is disabled so an operator cannot unknowingly retain an unusable signing posture.

When disabled, discovery keeps `copy_file` available for preview but constrains `dry_run` to `true`. An explicit mutation is denied with `copy_file_mutation_disabled` before source or destination inspection. When enabled, discovery describes the request grant and `runtime_status` reports only the stable gate, mode, public header name, 60-second lifetime, and fixed file/response limits.

## Exact-binary offline issuance

Grant issuance is a local CLI operation, never an MCP tool:

```bash
umask 077
GRANT_FILE="$(mktemp "$HOME/.cache/termux-mcp-copy-grant.XXXXXX")"

MCP__CAPABILITY__CONFIG_FILE="$HOME/.config/termux-mcp-edge/runtime.env" \
MCP__CAPABILITY__SESSION_ID="$MCP_SESSION_ID" \
MCP__CAPABILITY__COPY_FILE_SOURCE="$ABSOLUTE_SOURCE" \
MCP__CAPABILITY__COPY_FILE_DESTINATION="$ABSOLUTE_ABSENT_DESTINATION" \
  termux-mcp-server --issue-copy-file-grant >"$GRANT_FILE"
```

`MCP__CAPABILITY__CONFIG_FILE` uses the same bounded literal configuration loader as the other issuers. It must identify an absolute, owner-private, final-component-no-follow regular file of at most 64 KiB. The loader accepts one allowlisted literal `NAME=value` record per setting and never evaluates shell syntax. Omitting it is supported only for isolated validator and development processes that supply the exact configuration directly in their environment.

The issuer independently constructs validated filesystem tools from the deployed safe-root configuration. Fallible construction lifetime-pins both selected roots; source and destination preparation duplicate those descriptors and bind their device/inode identities without reopening configured pathnames. It opens and holds the source without following links, requires a single-link regular file at or below 1 MiB, reads the exact bytes, captures the high-resolution identity, computes SHA-256 itself, validates the absent destination and existing parent, and derives the exact grant target. Runtime consumption compares both bindings with the service's own lifetime pins. If either configured pathname was replaced after service startup, a grant issued against the replacement cannot authorize the original root. The issuer and service need not share descriptor numbers; both must identify the same directory objects. There is no caller-supplied size, identity, or digest. Grant output is one line on standard output; errors are stable and do not reflect either path, content, key, principal, session, digest, descriptor, or filesystem identity.

Send the grant only in its dedicated header:

```http
MCP-Capability-Grant: v1.<key-id>.<opaque-payload>.<mac>
```

Remove the grant file immediately after the request. Never place grants or keys in JSON, URLs, command-line arguments, logs, screenshots, tickets, or release evidence.

## Exact authorization binding

The fixed 65-byte signed payload contains only:

- a random 128-bit grant identifier;
- globally allocated capability code `4`;
- one keyed, domain-separated 32-byte operation binding;
- issuance seconds; and
- expiry seconds.

The payload does not serialize the private binding inputs. The operation binding covers:

- the keyed static authenticated principal;
- the canonical active MCP session UUID;
- copy capability code `4` and mutating/no-replace posture;
- source safe-root device and inode;
- a domain-separated, length-prefixed digest of normalized source components;
- source device, inode, exact size, high-resolution change time, and one-link count;
- SHA-256 of the exact copied bytes;
- destination safe-root device and inode;
- a separately domain-separated, length-prefixed digest of normalized destination components;
- fixed absent-destination/no-replace disposition;
- signed format version and key identifier; and
- random grant identifier, issuance, and expiry.

Directory code `1`, write code `2`, volume code `3`, and copy code `4` come from one internal wire registry. All ordered cross-family uses are rejected without consuming the source grant.

The normal lifetime is 60 seconds. Validation permits at most five seconds of future clock skew and rejects zero or greater-than-120-second lifetimes. One process retains at most 4,096 unexpired consumed copy identifiers. Equivalent independently constructed copy authorities with the same key identifier, HMAC key, and static principal share replay, last-observed-clock, and capacity state through the bounded process-global registry. Other families, keys, key identifiers, and principals remain isolated. Full, poisoned, unavailable, capacity-mismatched, or namespace-exhausted state fails closed.

This replay guarantee does not cross an operating-system process boundary. Run one grant-consuming process per capability-key/principal domain, or add an external atomic one-use coordinator before deploying multiple consumers. Restart clears the process-local state; rotate the key during restart when outstanding grants must be invalidated immediately.

## Validation and commit order

For `dry_run:false`, the runtime performs this order:

1. authenticate the HTTP request;
2. validate Host/Origin, method, media types, body and concurrency limits, JSON-RPC, protocol version, and active session;
3. accept exactly one bounded ASCII capability header only for `tools/call` → `copy_file`;
4. validate the closed copy schema and the independently enabled runtime gate;
5. preflight the complete 16 KiB success response with the caller's real JSON-RPC id;
6. acquire the single fail-fast filesystem-mutation worker permit, or deny without preparation or consumption;
7. inside the permit-owned blocking worker, anchor both roots, hold source/root/parent and destination/root/parent descriptors, read and hash the bounded source, require the destination absent, and build the exact grant target;
8. acquire the poison-fail-closed process-wide publication lock shared by create, copy, and write;
9. revalidate both root identities, the held and named source identity/content/size/SHA-256, the destination parent identity and absence, and hidden staging capacity;
10. atomically resolve request cancellation against worker ownership. A cancellation winner stops without consuming the grant or mutating the filesystem;
11. validate and atomically consume the exact grant; and
12. retain the process lock while staging, publishing, verifying, synchronizing, and performing identity-safe cleanup, independently of later request cancellation or disconnect.

Missing, malformed, wrong-context, stale, mismatched, oversized-response, worker-capacity, poisoned-lock, cancellation-before-commit, and preview requests do not consume a valid grant. Once step 11 succeeds, the grant remains consumed after every later staging, publication, verification, sync, cleanup, response, timeout, or disconnect outcome. Concurrent replay through equivalent authorities permits exactly one consumer.

## Filesystem transaction

The runtime copies the already-read, grant-bound bytes; it does not reread an untrusted pathname after authorization. Staging occurs inside the destination parent's reserved `.termux-mcp-write-quarantine` directory. That namespace must be mode `0700`, is rejected and hidden by every MCP filesystem surface, and uses the existing bounded nonblocking advisory lock. The runtime:

1. creates one unpredictable regular staging object exclusively at mode `0600`;
2. writes the exact bounded bytes and synchronizes the held descriptor;
3. verifies its held and named type, one-link identity, mode, and size;
4. publishes from the hidden quarantine to the held destination parent with atomic `NOREPLACE`;
5. verifies the final name and held descriptor are the exact staged inode under the fixed contract;
6. synchronizes the destination parent and quarantine; and
7. verifies the quarantine bounds before reporting success.

An existing destination always wins and is never overwritten. Cleanup captures the staging identity before it can remove anything, follows the authoritative staging or published parent descriptor, stats without following links, and unlinks only the exact captured single regular-file identity. Foreign files, links, directories, FIFOs, sockets, and substituted identities are preserved. A same-UID external writer can ignore the advisory lock or race namespace operations and force a bounded denial; production mutation safe roots therefore require exclusive operational ownership by this service while any live filesystem mutation gate is enabled.

Success returns only `dryRun`, `sizeBytes`, fixed `mode`, and fixed file/response limits. It returns no source path, destination path, content, digest, identity, grant, or staging name.

## Stable denials and audit privacy

Authorization failures use HTTP 403, JSON-RPC `-32003`, and one stable reason. Header shape or wrong-context use is rejected before tool authorization without reflecting the header.

| Condition | Stable reason |
| --- | --- |
| Gate disabled | `copy_file_mutation_disabled` |
| Grant absent | `capability_grant_missing` |
| Malformed or oversized | `capability_grant_malformed` |
| Unknown version or key | `capability_grant_version_unknown` / `capability_grant_key_unknown` |
| Invalid MAC | `capability_grant_signature_invalid` |
| Expired or future-issued | `capability_grant_expired` / `capability_grant_future_issued` |
| Invalid lifetime | `capability_grant_lifetime_exceeded` |
| Principal/session/source/destination/content/posture mismatch | `capability_grant_binding_mismatch` |
| Already consumed | `capability_grant_replayed` |
| Clock moved backward | `capability_clock_rollback` |
| Replay capacity exhausted | `capability_replay_capacity_exhausted` |
| Replay state unavailable | `capability_state_unavailable` |

Filesystem and scheduling failures use the stable copy, safe-root, response-limit, mutation-worker, cancellation, and publication reasons documented by the audit contract. Responses and aggregate counters retain no path, content, digest, identity, principal fingerprint, session, JTI, key, grant, timestamp, temporary name, or host error. The detached worker owns exactly one terminal audit result after it starts; waiter timeout or disconnect cannot erase or duplicate that outcome.

## Public Rust API boundary

`FileSystemTools::copy_file` is preview-only. Passing `Some(false)` returns `CopyMutationAuthorizationRequired` before path inspection. Live preparation and execution are crate-private and reachable only through the grant-aware transport worker. Public target construction and grant issuance can inspect a confined target but do not expose a mutation primitive. This prevents a downstream embedding from routing around the runtime gate, session binding, response preflight, cancellation boundary, grant consumption, or audit ownership.

## Required evidence

Default and all-feature validation must cover:

- disabled/enabled startup truth tables, discovery, and runtime status;
- exact-binary issuance without private reflection;
- every principal, session, family, root, path, source identity, size, digest, destination, posture, time, version, key, and signature mismatch;
- sequential and concurrent replay across independently constructed equivalent authorities;
- preview non-consumption and wrong-context/duplicate/oversized/non-ASCII/smuggled header rejection;
- exact 1 MiB acceptance and plus-one denial;
- source or destination changes before lock-held commit without consumption;
- two grants racing one destination across independent tool instances;
- cancellation before and after worker ownership;
- hidden mode-`0700` staging, fixed mode `0600`, exact bytes, atomic no-replace, verification, sync, and empty-success cleanup;
- identity-safe preservation of foreign regular, symlink, directory, FIFO, socket, and substituted objects;
- actual-id 16 KiB response preflight before consumption;
- content/path/digest/grant-free responses, logs, counters, fixtures, and evidence; and
- default and all-feature format, Clippy, tests, Security, Android cross-builds, emulated native gates, and physical native Termux validation of the exact candidate.
