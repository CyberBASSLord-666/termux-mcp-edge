# Write-File Capability Grants

## Security boundary

Static bearer authentication proves who may enter the MCP transport. It does not authorize file mutation.

Live `write_file` requires all of the following:

1. a binary built with `mcp-runtime`;
2. `MCP__FILE__WRITE_MUTATION_ENABLED=true`;
3. non-empty static-token authentication;
4. one valid capability key ID and 32-byte HMAC key;
5. one active canonical MCP session;
6. explicit `dry_run:false`; and
7. exactly one matching `MCP-Capability-Grant` header.

The gate defaults to `false`. Partial, malformed, non-Unicode, or compile-incompatible configuration fails before the listener starts.

## Private configuration

```dotenv
MCP__AUTH__STATIC_TOKEN=replace-with-a-strong-private-token
MCP__FILE__WRITE_MUTATION_ENABLED=true
MCP__CAPABILITY__KEY_ID=primary-1
MCP__CAPABILITY__HMAC_KEY_HEX=replace-with-64-lowercase-hex-characters
```

The key ID is 1–32 lowercase ASCII letters, digits, `-`, or `_`. The HMAC key is exactly 64 lowercase hexadecimal characters. The same private key configuration may support other capability families, but distinct signed capability codes and operation digests prevent cross-authorization.

Keep deployed `runtime.env` mode `0600`. Do not source it in a shell. The offline issuer can read it through the project's bounded, literal, no-follow configuration parser.

## Exact-binary offline issuance

The exact deployed candidate issues the grant; the running HTTP service never exposes an issuer endpoint.

The operator supplies:

```text
MCP__CAPABILITY__CONFIG_FILE=/absolute/private/runtime.env
MCP__CAPABILITY__SESSION_ID=<canonical active session UUID>
MCP__CAPABILITY__WRITE_FILE_TARGET=<absolute safe-rooted target>
MCP__CAPABILITY__WRITE_FILE_CONTENT_SHA256=<64 lowercase hex characters>
```

Then invokes:

```bash
termux-mcp-server --issue-write-file-grant
```

The command writes exactly one bounded ASCII grant line to stdout and uses stable non-reflective stderr on failure. Redirect stdout directly to a private mode-`0600` file. Never place a grant in shell history, command arguments, URLs, JSON tool arguments, logs, tickets, screenshots, or evidence JSON.

The issuer anchors the configured safe roots, validates the target without mutation, and infers whether the exact current posture is create or replace. The caller cannot request a more permissive disposition.

## Cryptographic binding

Each versioned HMAC-SHA-256 grant contains fixed-shape binary fields and is bound to:

- a random 128-bit single-use grant ID;
- a keyed digest of the configured static principal;
- the canonical 128-bit MCP session UUID;
- the write-file capability code;
- anchored safe-root device and inode;
- a domain-separated digest of every length-delimited normalized root-relative path component;
- the exact lowercase-SHA-256 content digest supplied to the issuer;
- the inferred create-or-replace publication disposition;
- the mutating posture;
- issue and expiry times;
- the configured key ID and token version.

The exact target path, content, digest, principal, session string, and root identity do not appear in plaintext in the token. A grant is bearer-sensitive even though its fields are fixed-shape and digested.

## Lifetime and single-use behavior

The exact binary issues a 60-second grant. The runtime rejects expired, excessively future-issued, zero/excessive-lifetime, unknown-version, unknown-key, invalid-signature, and noncanonical-session tokens.

Replay state is in-memory, concurrency safe, bounded, and fail closed. Exactly one concurrent consumer can win. Expired replay entries are pruned, capacity exhaustion denies new consumption, clock rollback denies consumption, and poisoned/unavailable state never authorizes mutation.

Restarting the service clears replay memory but does not make an old token valid beyond its signed expiry, binding, current target disposition, active session, and key posture. Rotate the key after suspected disclosure.

## Request order

For a live call, the runtime:

1. authenticates the bearer token;
2. enforces request limits and exact transport headers;
3. validates one bounded ASCII grant header and the JSON-RPC tool-call context;
4. parses arguments and confirms the write gate is enabled;
5. serializes the complete success response with the caller's actual JSON-RPC ID under the 16 KiB ceiling;
6. validates the 1 MiB payload ceiling and descriptor-safe target classification;
7. revalidates the create-or-replace destination posture;
8. verifies and atomically consumes the grant immediately before exclusive staging-file creation; and
9. completes staging, publication, durability, verification, rollback, and identity-safe cleanup in an owned worker.

Failure after step 8 leaves the grant consumed. Failure before step 8 leaves it unconsumed.

Dry-run calls never consume a grant and never stage or publish. A grant supplied to an exact `write_file` preview remains usable for the later matching live call. A grant header on initialization, lifecycle messages, discovery, another tool, notifications, responses, GET, or DELETE is rejected as invalid capability context.

## Binding consequences

- A different bearer principal, session, safe root, target, content byte, disposition, or mutating posture fails with the same private binding denial.
- A create grant cannot replace a file that appears after issuance; no-replace publication fails and the grant remains consumed once staging was attempted.
- A replace grant cannot create a target that disappears.
- A directory-creation or Android-volume grant cannot authorize `write_file`, even when the key ID and HMAC key are shared.
- A successful or failed consumed grant cannot be retried. Issue a new grant after revalidating the current target and content.

## Stable denial reasons

Authorization failures use HTTP 403 and JSON-RPC `-32003` with stable reasons:

| Condition | Reason |
|---|---|
| Gate has no authority | `write_file_mutation_disabled` |
| Header absent | `capability_grant_missing` |
| Shape or encoding invalid | `capability_grant_malformed` |
| Token/key version unavailable | `capability_grant_version_unknown` / `capability_grant_key_unknown` |
| MAC invalid | `capability_grant_signature_invalid` |
| Time invalid | `capability_grant_expired`, `capability_grant_future_issued`, or `capability_grant_lifetime_exceeded` |
| Any principal/session/root/target/content/disposition/posture mismatch | `capability_grant_binding_mismatch` |
| Grant ID already consumed | `capability_grant_replayed` |
| Clock, replay capacity, or state unsafe | `capability_clock_rollback`, `capability_replay_capacity_exhausted`, or `capability_state_unavailable` |

Header duplication, non-ASCII bytes, oversize, or a forbidden request context is rejected before capability verification with a bounded transport error. None of these responses identify which private binding differed.

## Rotation and recovery

1. Generate a new random 32-byte key offline.
2. Choose a new valid key ID.
3. atomically replace the private deployed configuration;
4. restart and verify readiness plus disabled/enabled runtime status;
5. issue only new-key grants from the exact active binary; and
6. securely remove obsolete grant files and old configuration copies after rollback requirements expire.

If a mutation returns a post-consumption failure, inspect only the safe-rooted target and private service logs, verify no operation-owned staging name remains, and issue a fresh grant only after confirming the intended current create-or-replace posture and exact content digest. Never reuse or paste the failed grant for diagnosis.

## Audit and evidence privacy

Runtime counters and release evidence may contain only low-cardinality tool, gate, mode, decision, and reason labels. They must never contain the target path, content, content digest, grant, HMAC key, key-derived principal digest, session, JTI, issue/expiry time, root identity, staging name, hostname, username, or raw error.

The full filesystem transaction and acceptance matrix are defined in [Safe-root file writes](SAFE_ROOT_FILE_WRITES.md).
