# Request-Authorized Android Volume Control

`set_android_volume` is a narrowly bounded Android mutation for one documented
audio stream and one exact integer level. It is preview-first, hidden by
default, and independent from the read-only `android_volume_status` runtime
gate.

## Authority boundary

The tool exists only when all of these conditions hold:

1. the binary was built with `--features android-volume-control`;
2. `MCP__ANDROID__VOLUME_CONTROL_ENABLED=true` passes startup validation;
3. static-token authentication is configured; and
4. `MCP__CAPABILITY__KEY_ID` and `MCP__CAPABILITY__HMAC_KEY_HEX` form a valid
   capability key pair.

The feature includes the strict read-only volume-status provider needed for
fresh bounds checks. It does not implicitly enable discovery of
`android_volume_status`; that tool still has its own runtime flag.

The control boundary is fixed:

- executable: the same absolute Termux:API `termux-volume` program used by the
  status provider;
- setter argv: exactly `<stream> <level>`;
- streams: `alarm`, `call`, `music`, `notification`, `ring`, and `system`;
- level: an integer from zero through the fresh `max_volume` for that stream;
- cwd: `/`;
- environment: empty;
- stdin: null;
- timeout and output ceilings: fixed by the binary;
- concurrency: one non-queueing mutation permit.

There is no shell, `PATH` lookup, caller-selected executable, argv extension,
environment, stdin, cwd, timeout, output limit, or command fallback.

## Configuration

Build the dedicated posture:

```bash
cargo build --release --features android-volume-control
```

Use a private mode-`0600` `runtime.env` and static-token authentication:

```dotenv
MCP__AUTH__STATIC_TOKEN=replace-with-a-strong-random-token
MCP__ANDROID__VOLUME_CONTROL_ENABLED=true
MCP__CAPABILITY__KEY_ID=primary-1
MCP__CAPABILITY__HMAC_KEY_HEX=replace-with-64-lowercase-hex-characters
```

The server fails startup when the runtime gate is enabled in an incompatible
binary, authentication is not static-token based, either key field is absent,
or either field is malformed. With the flag absent or `false`, the tool is not
advertised and direct calls fail without invoking Termux:API.

## Preview

The closed input schema is:

```json
{
  "stream": "music",
  "level": 8,
  "dry_run": true
}
```

`dry_run` defaults to `true`. Preview performs a fresh strict six-stream status
read, validates `0..=max_volume`, and returns only the selected stream, captured
current level, requested level, live maximum, `dryRun:true`, `changed:false`,
`verified:false`, outcome `preview`, and rollback classification
`not_required`. It neither requires nor consumes a grant and never invokes the
setter.

## Exact-binary grant issuance

Live mutation additionally needs one 60-second, single-use
`MCP-Capability-Grant`. Issue it locally with the exact deployed binary after
initializing the target MCP session:

```bash
umask 077
GRANT_FILE="$(mktemp)"
MCP__CAPABILITY__CONFIG_FILE="$HOME/.config/termux-mcp-edge/runtime.env" \
MCP__CAPABILITY__SESSION_ID="$MCP_SESSION_ID" \
MCP__CAPABILITY__VOLUME_STREAM=music \
MCP__CAPABILITY__VOLUME_LEVEL=8 \
  "$HOME/.local/share/termux-mcp-edge/current/termux-mcp-server" \
  --issue-android-volume-grant >"$GRANT_FILE"
chmod 600 "$GRANT_FILE"
```

The offline issuer uses the deployed literal configuration parser: it does not
evaluate shell syntax, follow the final config-file component, accept a public
config mode, or print private inputs. The grant binds a keyed digest of the
static principal, canonical session UUID, volume-control capability, exact
stream, exact level, mutating posture, key ID, random identifier, issue time,
and expiry.

Send the private file's one line only in the matching request header:

```http
MCP-Capability-Grant: <single private grant line>
```

```json
{
  "jsonrpc": "2.0",
  "id": "set-volume",
  "method": "tools/call",
  "params": {
    "name": "set_android_volume",
    "arguments": {"stream":"music","level":8,"dry_run":false}
  }
}
```

Remove the grant file immediately after the request. Never place grants or
keys in JSON, URLs, process arguments, logs, audit labels, screenshots,
tickets, or release evidence.

## Mutation and recovery order

For `dry_run:false`, the server:

1. rejects conflicting work rather than waiting for the single mutation lane;
2. performs a fresh strict status read and validates the requested live bound;
3. validates the grant against the authenticated principal, active session,
   capability, exact stream, exact level, posture, time, signature, and replay
   state;
4. atomically consumes the grant immediately before the first setter attempt;
5. executes only fixed `termux-volume <stream> <level>`;
6. reads strict status again and requires the requested level; and
7. on setter or verification failure, attempts to set the captured prior level
   and confirms it with another strict status read.

Once step 4 succeeds, the grant remains consumed for success, provider failure,
verification failure, rollback success, rollback failure, response loss, or
caller cancellation. Execution, verification, and recovery move to an owned
task after consumption, so cancellation of the HTTP request cannot cancel the
recovery sequence. Every provider subprocess remains under the bounded,
process-group-aware supervisor.

A verified success returns `outcome:"mutation_verified"`, the captured and
requested levels, live maximum, whether the numerical value changed, and
`rollback:"not_required"`. Failures return stable redacted reason codes:

| Class | Reason code |
|---|---|
| Invalid level | `volume_control_level_out_of_range` |
| Conflicting mutation | `volume_control_concurrency_limit` |
| Setter failed, prior level confirmed | `volume_control_set_failed_rollback_confirmed` |
| Setter failed, restoration unconfirmed | `volume_control_set_failed_rollback_unconfirmed` |
| Verification failed, prior level confirmed | `volume_control_verification_failed_rollback_confirmed` |
| Verification failed, restoration unconfirmed | `volume_control_verification_failed_rollback_unconfirmed` |
| Detached worker failed | `volume_control_worker_failed` |

Strict status-provider failures retain the documented `volume_*` codes from
[`ANDROID_VOLUME_STATUS.md`](ANDROID_VOLUME_STATUS.md). Authorization denials
use the shared stable `capability_*` reason family, including missing,
malformed, wrong binding, expired, future-issued, invalid signature, unknown
key/version, clock rollback, bounded replay-state exhaustion, and replay.
Responses never contain raw provider output, stderr, tokens, grants, keys,
principal/session identifiers, or caller-supplied strings.

## Audit and operational truth

`runtime_status` reports compile state, enabled state, the stable mode, the
public grant-header name, 60-second TTL, and whether bounded Android device
control/high-impact tooling is active. `tools/list` advertises the control tool
only in the enabled posture; grant issuance is never an MCP tool.

Aggregate in-memory counters record only the stable tool, gate, dry-run or
mutating mode, allowed/denied decision, and reason code. Preview, verified
mutation, authorization denial, concurrency rejection, and both rollback
classifications are therefore measurable without recording arguments or
credentials.

## Validation

Release evidence must prove both compile postures and both runtime-gate states,
closed discovery/schema behavior, static-token/key requirements, preview
non-consumption, every binding/time/signature/replay denial, concurrent replay,
header-context rejection, exact argv/cwd/environment/stdin, fresh maximum
validation, non-queueing concurrency, success verification, rollback confirmed
and unconfirmed, timeout/output/process-group/cancellation cleanup, audit
redaction, exact artifact provenance, and native ARM64 official-Termux
execution. Emulation is deterministic development evidence; physical-device
audio-policy or OEM behavior requires separate device evidence.
