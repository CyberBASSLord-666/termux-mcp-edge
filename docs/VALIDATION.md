# Validation

## Current Runtime Validation Scope

The default compiled runtime is an Axum HTTP health/readiness service. The optional `mcp-runtime` feature compiles stable MCP 2025-11-25 Streamable HTTP handling at `/mcp` and its current limited tool surface.

The 17 baseline staged MCP tools are `runtime_status`, `platform_info`, `android_status`, `project_service_status`, `create_directory`, `copy_file`, `trash_file`, `find_paths`, `hash_file`, `list_directory`, `path_metadata`, `read_binary_file`, `read_binary_range`, `read_file`, `read_text_range`, `search_text`, and `write_file`. Separately built and runtime-enabled postures may add bounded read-only `android_battery_status`, `android_volume_status`, the fixed server-diagnostic `run_command_profile`, or preview-first request-authorized `set_android_volume`. Directory, file-copy, file-trash, and file-write previews are baseline, but their live mutations are independently default-disabled and each requires its own request-scoped grant. Android controls beyond exact-stream volume, shell fallback, arbitrary command execution, process inventory, arbitrary service inspection, service mutation/control, and unrelated high-impact tools remain out of scope for the live runtime.

The optional MCP transport enforces authentication before mobile-conscious concurrency, timeout, body-size, Host, Origin, JSON-RPC, discovery, and invocation handling. The named `full-suite` build composes every optional provider but keeps exactly 17 tools with its four optional runtime flags off and exactly 21 with all four on. Raw Cargo `--all-features` remains a separate development compatibility lane.

## Required Repository Gates

Run the same Rust gates enforced by `.github/workflows/ci.yml`:

```bash
cargo metadata --locked --all-features --format-version 1 --no-deps >/dev/null
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --features mcp-runtime -- -D warnings
cargo clippy --locked --workspace --all-targets --features full-suite -- -D warnings
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --features mcp-runtime
cargo test --locked --workspace --all-targets --features full-suite
cargo test --locked --workspace --all-targets --all-features
```

Build all supported compile-time postures when preparing a release candidate:

```bash
cargo build --release --locked
cargo build --release --locked --features mcp-runtime
cargo build --release --locked --features android-battery-status
cargo build --release --locked --features android-volume-status
cargo build --release --locked --features android-volume-control
cargo build --release --locked --features command-execution
cargo build --release --locked --features full-suite
```

Validate the exact release candidate on an AArch64 Termux device with the no-clone harness in [`DEVICE_PRODUCTION_GATE.md`](DEVICE_PRODUCTION_GATE.md). Its companion contract test runs in CI as `tests/termux_device_smoke_test.sh`; CI validates the harness interface and required coverage markers, while the actual run requires a real Termux/runit device.

Validate downloaded default, `mcp-runtime`, `android-volume-control`, and `full-suite` artifacts with [`RELEASE_CANDIDATE_VALIDATION.md`](RELEASE_CANDIDATE_VALIDATION.md). Validator v11 emits direct evidence schema v2, reconciles four exact manifests/digests, and proves the full-suite 17-disabled/21-enabled truth table without collapsing any runtime or request grant. CI runs the packaging and validator contract tests against deterministic posture fixtures and deployment-manager fixture mode.

The CI workflow first rejects a missing or stale committed dependency graph before any Cargo-aware formatting or cache action can repair it, verifies those steps leave both dependency inputs unchanged, and runs default, minimal `mcp-runtime`, and all-feature Clippy/tests with `--locked`. The Security workflow validates the same locked graph with `cargo audit` and fails on audit findings. The same exact-head workflow also runs pinned CodeQL `security-extended` analysis for Rust source and GitHub Actions workflow code, using buildless extraction and separate SARIF categories. A green CodeQL job proves that analysis and SARIF upload completed; code-scanning alerts must still be inspected and triaged rather than inferred absent from the workflow conclusion alone. CI and Security path filters cover every source and evidence input that can start Android validation, so its native-evidence job can resolve both required companion runs for the same commit. The native job reserves a monotonic, request-timeout-enforced 25-minute polling window inside its 50-minute job budget, which accommodates the 20-minute CodeQL job ceiling while preserving the prior native-validation allowance and requiring both exact-head companions to complete successfully.

## Secure Embedding Boundary Validation

The minimal `mcp-runtime` and all-feature suites must both exercise the single
public `McpRouterBuilder` path used by the package binary:

1. Construction receives an actually bound listener plus validated auth,
   request-limit, transport-security, and safe-root inputs. A wildcard listener
   is rejected for unauthenticated development, while request-time missing
   metadata, a non-loopback peer, and an actual served listener different from
   the listener validated by the builder are independently rejected.
2. Authentication remains the outer route layer. Unauthenticated oversized,
   malformed, wrong-origin, fake-session, discovery, read, malformed-grant,
   and mutation requests all return the same HTTP 401 boundary without session
   allocation, limit admission, parsing, disclosure, grant consumption, or
   filesystem change.
3. Invalid root classes return exact `McpRouterBuildError` variants without a
   configured path, token, descriptor, identity, or raw operating-system error.
   Minimal-feature tests request uncompiled battery and volume clients and
   require typed `CapabilityUnavailable` errors rather than panic or silent
   downgrade.
4. Mutation-authority setters reject a principal different from the selected
   static bearer and reject all authorities in unauthenticated mode. Matching
   principals still require the normal runtime gates and exact single-use
   grants.
5. Ordinary dependency and selected-workspace compile probes can use
   `McpRouterBuilder` but cannot import raw state, legacy router constructors,
   `McpTransportOptions`, `McpRouterProtection`, capability-authority bundles,
   binary command switches, raw command clients, or forged profiles.
6. Documentation examples compile, the package binary serves the exact
   listener supplied to the builder with opaque
   `ConnectInfo<McpConnectionInfo>` derived from each accepted stream, and all
   requested but unavailable optional clients fail startup.

The exact order under test is authentication; authenticated `Content-Length`,
concurrency, and timeout enforcement; streaming body limit and extraction;
Host/Origin; method/media and JSON-RPC; lifecycle/session; discovery; grant
context; tool dispatch; mutation. See [`EMBEDDING.md`](EMBEDDING.md).

## Safe-Root Lifetime-Pinning Validation

The all-feature suite must prove the safe-root authority boundary directly and deterministically:

1. Fallible construction rejects an empty set, more than 64 configured entries, empty/relative/traversing paths, filesystem root, missing objects, regular files, and a symlink in the root or any ancestor after the exact listener is bound but before runtime state, router construction, or request serving can proceed. Because the retained root uses a path descriptor, final-directory read/write/search permission is validated by the operation that needs it rather than overclaimed as a startup invariant.
2. Valid labels are normalized, sorted, and deduplicated deterministically. The input-entry ceiling is enforced before deduplication, and no best-effort canonicalization fallback is permitted.
3. Construction retains one no-follow directory descriptor and device/inode identity per distinct normalized root label; `FileSystemTools` clones share the same pins and operations duplicate and re-verify them instead of reopening pathnames. Lexical deduplication must not silently collapse different labels solely because bind-mount aliases share an identity.
4. Renaming or replacing a root, or renaming/replacing an ancestor, cannot redirect a running instance. Reads and mutations remain attached to the original pinned directory and leave replacement objects at the configured path untouched.
5. Create, copy, trash, and write grant issuers bind their independently pinned root identity. Runtime target preparation and consumption compare against the running pin, including an explicit regression where a grant issued after replacement fails to authorize the original pinned root.
6. Every descendant tool retains component-by-component no-follow traversal from the selected pin. Any fixed command-profile working directory derived from a filesystem root must duplicate that same pin rather than reopen the label.
7. Constructor errors, tool debug output, audit counters, and evidence contain no configured-root path, descriptor number, device/inode identity, or raw operating-system error. Public responses never expose descriptor numbers or root device/inode identity; successful tools may return only the contract-defined normalized request paths documented for that tool.

Host unit/integration tests provide the adversarial rename/replacement and redaction proof. Exact-artifact validators and native ARM64 official-Termux emulation provide normal startup, readiness, confined operation, grant, and deployment evidence. These deterministic gates are sufficient for development and merge validation; safe-root lifetime pinning does not require an arbitrary 60-minute idle monitoring run. Direct physical observation is reserved for a release-evidence classifier finding about battery, thermal, OEM process management, Android storage/mount behavior, or another device-only property that the focused gates cannot establish.

For `command-execution` changes, validation must run both ordinary path-dependency and two-member selected-workspace compile/API probes. Each first builds a valid consumer of the single public builder, then proves profile construction, resolved-handle access, raw execution-client access, removed authority symbols, the binary-only command switch, every legacy router constructor, and the former public option/authority bundle types are unreachable; the public builder remains command-disabled. Runtime tests must prove raw program/argv and every override field are rejected before spawn; wrong-name, symlink, non-regular, non-executable, and wrong-device/inode candidates return `McpRouterBuildError::CommandClientUnavailable` before the already-bound listener serves any request; the independently opened `/proc/self/exe` pins executable identity; no-follow cwd descriptors reject root aliases and survive pathname replacement; and maximum-plus-one timeout/stdout/stderr configurations fail before spawn. The native wrong-name phase must probe health while construction is in flight, require non-timeout process failure and the exact non-sensitive construction error, and reject token/path disclosure or a service-start log. The supervisor ceilings are exactly 5 seconds, 16 KiB stdout, and 4 KiB stderr independently of profile data. Output capacity may grow fallibly only for bytes actually read, never from a selected limit. Run default, minimal `mcp-runtime`, and all-feature Clippy/tests so the private execution surface is correct in every compile posture.

## Dependency Update Validation

Dependency update PRs must remain separate from runtime behavior changes. Before merging a Cargo or GitHub Actions dependency update:

1. Confirm the PR diff is limited to dependency metadata, workflow pin updates, or generated lockfile changes.
2. Confirm exact-head CI succeeds for the dependency-update head SHA.
3. Confirm exact-head Security succeeds for the dependency-update head SHA.
4. Confirm the Security workflow output does not report unresolved advisories.
5. Avoid bundling dependency updates with MCP transport, browser-exposed routes, filesystem tools, system tools, or command-capable tool exposure.

If a dependency update is required to restore a higher-risk surface, keep it blocked until the related transport protections, authorization policy, and smoke tests are present in the same focused restoration stage or in already-merged prerequisite PRs.

## Runtime Smoke Test

After building or installing the binary, verify liveness:

```bash
curl -fsS http://127.0.0.1:8000/health
```

Expected response:

```text
ok
```

Inspect readiness:

```bash
curl -fsS http://127.0.0.1:8000/ready | jq
```

The `/health` and `/ready` operational probes do not require bearer authentication. They must not return secrets, raw configuration, private paths, or tool output. When `mcp-runtime` is enabled, readiness should include only the active non-sensitive `mcp_request_limits` values.

## Staged MCP Smoke Tests

When built with `--features mcp-runtime`, load the configured token without printing it:

```bash
MCP_TEST_TOKEN="$(sed -n 's/^MCP__AUTH__STATIC_TOKEN=//p' "$HOME/.config/termux-mcp-edge/runtime.env")"
```

First prove unauthenticated discovery is rejected before request-limit accounting or JSON-RPC dispatch:

```bash
curl -i -sS \
  -X POST \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  --data '{"jsonrpc":"2.0","id":0,"method":"tools/list"}' \
  http://127.0.0.1:8000/mcp
```

Expected behavior: HTTP 401, `WWW-Authenticate: Bearer`, a non-sensitive `unauthorized` response, and no tool-discovery result.

Then initialize a bounded session using the exact allowed `Host` and `Origin` headers:

```bash
MCP_RESPONSE_HEADERS="$(mktemp)"
curl -sS -D "$MCP_RESPONSE_HEADERS" \
  -X POST \
  -H "Authorization: Bearer ${MCP_TEST_TOKEN}" \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  --data '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"termux-validation","version":"1.0.0"}}}' \
  http://127.0.0.1:8000/mcp | jq -e '.result.protocolVersion == "2025-11-25"'
MCP_SESSION_ID="$(awk 'tolower($1) == "mcp-session-id:" {sub(/^[^:]*:[[:space:]]*/, ""); sub(/\r$/, ""); print; exit}' "$MCP_RESPONSE_HEADERS")"
test -n "$MCP_SESSION_ID"
```

Complete initialization and confirm the notification receives HTTP 202 without a body:

```bash
test "$(curl -sS -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer ${MCP_TEST_TOKEN}" \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2025-11-25' \
  -H "MCP-Session-Id: ${MCP_SESSION_ID}" \
  --data '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  http://127.0.0.1:8000/mcp)" = 202
```

Verify authenticated discovery within that session:

```bash
curl -sS \
  -H "Authorization: Bearer ${MCP_TEST_TOKEN}" \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2025-11-25' \
  -H "MCP-Session-Id: ${MCP_SESSION_ID}" \
  --data '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  http://127.0.0.1:8000/mcp | jq -e '.result.tools | length == 17'
```

Confirm a normal `mcp-runtime` build returns exactly the 17 baseline tools. An optional build still returns 17 unless its corresponding runtime flag is true; then it returns 18 tools. An all-feature validation build returns 21 only when battery, volume-status, volume-control, and command runtime flags are all true. With `MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=false`, prove `create_directory` discovery constrains `dry_run` to `true` and explicit mutation is denied. With the gate, static authentication, and paired capability key enabled, exercise preview and explicit mode through an exact-binary locally issued grant. Prove missing/wrong-context/wrong-binding grants fail, dry run does not consume, one exact target succeeds at mode `0700`, and replay is denied. Independently, with `MCP__FILE__WRITE_MUTATION_ENABLED=false`, prove `write_file` discovery constrains `dry_run` to `true` and explicit mutation returns `write_file_mutation_disabled` before filesystem access. When enabled with static authentication and the capability key pair, prove exact-binary issuance from a private stable content file, all principal/session/root/target/content/disposition/existing-identity/time/signature/replay/state bindings, preview non-consumption, authorized create and replace, exact 1 MiB acceptance/one-byte-over denial, fixed target mode `0600`, and a content/path-free 16 KiB response including `recoveryArtifactRetained`. Prove create uses `NOREPLACE` and retains no artifact. Prove replacement rejects hard-linked or over-1-MiB prior targets, performs one irreversible `EXCHANGE`, and preserves the displaced prior inode/content in a hidden bounded recovery quarantine. Exercise quarantine capacity, malformed entry, namespace isolation, advisory-lock contention, post-commit failure, cancellation-independent completion, and retained grant consumption without claiming hostile same-UID rollback. For volume control, prove disabled discovery, closed schema, preview non-consumption, every exact binding and replay denial, fresh maximum enforcement, fixed two-argument execution, non-queueing concurrency, verified success, and rollback confirmed/unconfirmed without private reflection. Exercise `copy_file` with binary content in preview and explicit mode, prove fixed mode `0600`, exact bytes, absent-destination/no-replace behavior, one-byte-over rejection, content-private responses, and pre-mutation full-response bounding. Exercise `find_paths` with literal queries, every kind filter, default/exact depth, deterministic ordering, empty results, 8,192-entry/512-match/262,144-byte ceilings, no-follow/invalid-UTF-8 skips, oversized-ID response preflight, and content/query-private audits. Exercise `hash_file` with binary and boundary fixtures, prove exact SHA-256/size output, pre-read full-response bounding, one-byte-over rejection, no-follow descriptor confinement, and digest/path/content-private audits. Exercise `read_binary_file` with arbitrary bytes, empty/exact-limit/one-byte-over fixtures, canonical padded base64, no-follow identity confinement, max-plus-one runtime enforcement, pre-read full-response bounding, and path/host-metadata-private results and audits. Exercise `read_binary_range` with arbitrary slices, exact range/file ceilings, EOF and offset-past-EOF, no-follow identity confinement, detected size-change rejection, pre-read full-response bounding, and path/host-metadata-private results and audits. Exercise `read_text_range` with multi-byte code-point pagination, boundary deferral, midpoint/invalid-encoding rejection, exact range/file ceilings, descriptor confinement, size-change rejection, worst-case JSON escaping, response preflight, and private results/audits. Exercise `path_metadata` and literal `search_text` under their documented content-free ceilings. Also verify the default GET 405 and, separately, the enabled bounded SSE response/resumption posture below.

Copy validation is independent of create, trash, and write. With `MCP__FILE__COPY_FILE_MUTATION_ENABLED=false`, prove discovery constrains `dry_run` to `true` and explicit mutation returns `copy_file_mutation_disabled` before path access. When enabled, use the exact-binary issuer and prove principal/session/family/root/path/source-identity/size/SHA-256/destination/posture/time/signature/replay bindings, preview non-consumption, shared replay across reconstructed authorities, exact 1 MiB and plus-one behavior, lock-held stale source/destination denial before consumption, hidden staging, fixed mode `0600`, atomic no-replace, path/content-free 16 KiB responses, and private exactly-once terminal audits.

Trash validation is independently default-deny. With `MCP__FILE__TRASH_FILE_MUTATION_ENABLED=false`, prove discovery constrains `dry_run` to `true` and explicit mutation returns `trash_file_mutation_disabled` before path access. When enabled, use `--issue-trash-file-grant` and prove principal/session/family/root/path/target-device/inode/size/high-resolution-ctime/link-count/SHA-256/recovery-posture/time/signature/replay binding, preview non-consumption, exact 1 MiB and plus-one behavior, stale-target and special-file denial, oversized-ID response preflight, exact-inode atomic `NOREPLACE` retention, separate mode-`0700` quarantine bounds/isolation, cancellation-independent completion after ownership, path/content/artifact-private results, and exactly-once terminal audits. No MCP purge or restore may appear.

Use the exact candidate's offline issuer only after the session is initialized:

```bash
GRANT_FILE="$(mktemp)"
chmod 600 "$GRANT_FILE"
MCP__CAPABILITY__SESSION_ID="$MCP_SESSION_ID" \
MCP__CAPABILITY__CREATE_DIRECTORY_TARGET="$ABSENT_SAFE_ROOT_TARGET" \
MCP__CAPABILITY__CONFIG_FILE="$HOME/.config/termux-mcp-edge/runtime.env" \
  /absolute/path/to/termux-mcp-server \
  --issue-create-directory-grant >"$GRANT_FILE"
```

Send the single line from that private file only as `MCP-Capability-Grant` on the matching `tools/call` request, then remove the file. Do not put grant material in JSON, URLs, process arguments, logs, reports, screenshots, or tickets. The complete configuration, issuance, denial, rotation, and validation contract is [`CREATE_DIRECTORY_CAPABILITY_GRANTS.md`](CREATE_DIRECTORY_CAPABILITY_GRANTS.md).

Validate file-write issuance separately. Enable `MCP__FILE__WRITE_MUTATION_ENABLED=true` in the private mode-`0600` runtime configuration, keep the static token and key pair configured, and use the exact candidate binary. Put the exact UTF-8 request content in an absolute private no-follow regular file rather than an argument or environment value:

```bash
WRITE_GRANT_FILE="$(mktemp)"
WRITE_CONTENT_FILE="$(mktemp)"
chmod 600 "$WRITE_GRANT_FILE" "$WRITE_CONTENT_FILE"
# Populate WRITE_CONTENT_FILE with the exact intended UTF-8 bytes without tracing.

MCP__CAPABILITY__CONFIG_FILE="$HOME/.config/termux-mcp-edge/runtime.env" \
MCP__CAPABILITY__SESSION_ID="$MCP_SESSION_ID" \
MCP__CAPABILITY__WRITE_FILE_TARGET="$SAFE_ROOT_WRITE_TARGET" \
MCP__CAPABILITY__WRITE_FILE_CONTENT_FILE="$WRITE_CONTENT_FILE" \
MCP__CAPABILITY__WRITE_FILE_DISPOSITION=create \
  /absolute/path/to/the/exact/termux-mcp-server \
  --issue-write-file-grant >"$WRITE_GRANT_FILE"
```

Use `create` only for an absent target and `replace` only for the exact existing regular file classified during issuance. Send the grant as the single `MCP-Capability-Grant` header on the matching `write_file` `tools/call`, with exactly the same JSON `content` bytes and explicit `dry_run:false`, then securely remove the temporary files. Prove changes to content, target, disposition, or the replacement inode deny the request; preview does not consume; one matching operation succeeds; and replay is denied. See [`WRITE_FILE_CAPABILITY_GRANTS.md`](WRITE_FILE_CAPABILITY_GRANTS.md).

Validate copy issuance separately with the exact candidate and enabled private runtime configuration:

```bash
COPY_GRANT_FILE="$(mktemp)"
chmod 600 "$COPY_GRANT_FILE"
MCP__CAPABILITY__CONFIG_FILE="$HOME/.config/termux-mcp-edge/runtime.env" \
MCP__CAPABILITY__SESSION_ID="$MCP_SESSION_ID" \
MCP__CAPABILITY__COPY_FILE_SOURCE="$SAFE_ROOT_COPY_SOURCE" \
MCP__CAPABILITY__COPY_FILE_DESTINATION="$ABSENT_SAFE_ROOT_COPY_DESTINATION" \
  /absolute/path/to/the/exact/termux-mcp-server \
  --issue-copy-file-grant >"$COPY_GRANT_FILE"
```

The issuer must inspect and hash the source itself. Send the grant only on the exact live `copy_file` call, then remove it. A source or destination change must deny without consumption; a matching call must publish exact bytes at mode `0600` without returning either path; reuse must be replay-denied. See [`COPY_FILE_CAPABILITY_GRANTS.md`](COPY_FILE_CAPABILITY_GRANTS.md).

Validate reversible-trash issuance separately with the exact candidate and enabled private runtime configuration:

```bash
TRASH_GRANT_FILE="$(mktemp)"
chmod 600 "$TRASH_GRANT_FILE"
MCP__CAPABILITY__CONFIG_FILE="$HOME/.config/termux-mcp-edge/runtime.env" \
MCP__CAPABILITY__SESSION_ID="$MCP_SESSION_ID" \
MCP__CAPABILITY__TRASH_FILE_TARGET="$SAFE_ROOT_TRASH_TARGET" \
  /absolute/path/to/the/exact/termux-mcp-server \
  --issue-trash-file-grant >"$TRASH_GRANT_FILE"
```

The issuer must open and hash the exact single-link target itself. Send the grant only on the matching live `trash_file` call, then remove it. Preview must not consume or create a quarantine; a target identity/content change must deny without consumption; a matching call must retain the exact inode in the separate private recovery quarantine; and reuse must be replay-denied. See [`TRASH_FILE_CAPABILITY_GRANTS.md`](TRASH_FILE_CAPABILITY_GRANTS.md).

The header itself has an ordered fail-closed boundary: route authentication; Host/Origin; POST/media and exactly-one-bounded-ASCII-header validation; JSON-RPC envelope; session/protocol/lifecycle; `tools/call`; exact grant-aware tool context; closed tool schema; mutation gate; complete-response preflight; safe-root/payload/target classification; recovery-capacity and posture revalidation; grant binding and atomic consumption; then the first namespace mutation. Test duplicate, empty, non-ASCII, oversized, wrong-method, initialize, discovery, notification, response, and unrelated-tool contexts without reflecting the header.

Validate the project-owned service status tool with the current allowlisted service name:

```bash
curl -sS \
  -X POST \
  -H "Authorization: Bearer ${MCP_TEST_TOKEN}" \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2025-11-25' \
  -H "MCP-Session-Id: ${MCP_SESSION_ID}" \
  --data '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"project_service_status","arguments":{"service_name":"mcp_runtime"}}}' \
  http://127.0.0.1:8000/mcp
```

Expected behavior: the response is read-only, reports only the allowlisted project-owned logical runtime service, and does not expose process inventory, shell fallback, arbitrary service names, or control actions.

## MCP Request-Limit Validation

Default values are intentionally conservative for Termux:

- `MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS=4`
- `MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS=30`
- `MCP__TRANSPORT__MAX_BODY_BYTES=2097152`
- `MCP__TRANSPORT__SSE_ENABLED=false`

Validated ranges are concurrency `1–64`, timeout `1–300` seconds, and body size `1024–8388608` bytes. Prove startup fails for zero, negative/non-numeric, or above-range values.

### Oversized authenticated request

Temporarily set a small validated body ceiling, restart the service, and send a larger authenticated body:

```bash
export MCP__TRANSPORT__MAX_BODY_BYTES=1024
python - <<'PY' > /tmp/mcp-oversized.json
import json
print(json.dumps({
    "jsonrpc": "2.0",
    "id": 3,
    "method": "tools/call",
    "params": {
        "name": "write_file",
        "arguments": {
            "path": "/data/data/com.termux/files/home/mcp-files/oversized.txt",
            "content": "x" * 2048
        }
    }
}))
PY
curl -i -sS \
  -H "Authorization: Bearer ${MCP_TEST_TOKEN}" \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2025-11-25' \
  -H "MCP-Session-Id: ${MCP_SESSION_ID}" \
  --data-binary @/tmp/mcp-oversized.json \
  http://127.0.0.1:8000/mcp
rm -f /tmp/mcp-oversized.json
```

Expected behavior: HTTP 413, `mcp_request_body_too_large`, `Cache-Control: no-store`, and no reflected request content.

Repeat without the `Authorization` header. Expected behavior: HTTP 401 rather than 413, proving authentication remains the outer resource gate.

### Concurrency saturation

Set `MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS=1` in a controlled test deployment and issue two overlapping authenticated requests. The second request must fail fast with HTTP 503, `Retry-After: 1`, and `mcp_concurrency_limit_reached`; it must not queue indefinitely.

### Request timeout

Set `MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS` to a low validated value in a controlled test build with an intentionally delayed test handler. Expected behavior is HTTP 504 with `mcp_request_timeout` and `Cache-Control: no-store`.

The repository test suite covers timeout behavior without adding a production delay tool.

### Bounded SSE response and resumption

In a controlled authenticated deployment, set `MCP__TRANSPORT__SSE_ENABLED=true`, restart, initialize a fresh session, and confirm an eligible request returns `Content-Type: text/event-stream`. The body must contain exactly an empty `:0` primer with `retry: 1000` and one `:1` terminal JSON-RPC event on the same UUID stream. Resume from the primer with GET, the normal auth/Host/Origin/protocol/session headers, `Accept: text/event-stream`, and `Last-Event-ID: <primer-id>`; only the terminal event may be returned.

Also prove malformed, duplicate, and over-64-byte cursors return 400; a valid unknown cursor and another session's cursor return the same 404; the ninth retained response evicts the oldest stream; a response above 128 KiB stays JSON; and a maximum 256 KiB text range consisting of NUL bytes remains an HTTP 200 JSON response even though its escaped envelope exceeds the binary-read response ceiling. A canonical serialized JSON-RPC id of exactly 1 MiB must retain bounded JSON fallback, while one byte over must return HTTP 413 with a null id before initialization, session allocation, ping, discovery, runtime status, or tool effects. Notifications remain empty 202, and DELETE makes the prior session and replay unavailable. Restore `MCP__TRANSPORT__SSE_ENABLED=false` after the controlled validation.

### Filesystem mutation cancellation and recovery retention

Live `create_directory`, `copy_file`, `trash_file`, and `write_file` first require the one shared, fixed, fail-fast, non-queueing mutation-worker permit. The permit owns descriptor preparation and the complete blocking commit, so an HTTP timeout cannot release capacity while detached preparation continues. After preparation, the worker acquires one poison-fail-closed process-wide publication lock shared across every tool and router state, then revalidates the prepared absent target, exact copy source/destination, exact trash target identity/content, or exact write-replace identity. Only then does the atomic cancellation/worker-ownership guard decide commit: cancellation first, including while waiting for the process lock, consumes no grant and mutates nothing; stale revalidation likewise preserves the grant for fresh preparation. Worker ownership first makes completion cancellation-independent and consumes the grant immediately before the first namespace mutation. The process lock remains held through publication or retention verification and durability sync. `copy_file` and `write_file` create randomized mode-`0600` staging inodes inside the fixed per-parent `.termux-mcp-write-quarantine` and verify descriptor, name, content, and identity before publication. Copy and write-create publish with `NOREPLACE` and retain no artifact; write-replace performs one identity-verified irreversible `EXCHANGE`. Replace leaves the displaced prior inode/content and its existing metadata under the randomized write-quarantine name; no automatic unlink, truncation, chmod, or swap-back follows capture. `trash_file` instead moves the exact authorized inode with atomic `NOREPLACE` into an unpredictable name under the separate mode-`0700` `.termux-mcp-trash-quarantine`, verifies retained identity/content and public-name absence, and never exposes purge or restore. The write result reports `recoveryArtifactRetained:false` for preview/create and `true` for successful replace. Trash preview reports false and successful retention reports true. Copy, trash, and write results expose no private path, content, digest, grant, or internal name.

Unit and transport coverage must inject cancellation during preparation, publication-lock waiting, timeout, target/artifact exchange, sync failure, and post-commit failure around these boundaries. It must prove the permit remains occupied while detached preparation runs; the process lock serializes distinct `FileSystemTools` instances across create/copy/trash/write families; poison fails closed; stale losers fail before authorization and preserve grant reuse after fresh preparation; cancellation-before-commit preserves grant reuse and filesystem state; and worker-before-cancellation completes with retained consumption. It must also prove both quarantine namespaces are mode `0700`, inaccessible to all MCP filesystem operations, contain only their canonical regular artifact entries, and independently fail closed at 32 artifacts, 32 MiB, or nonblocking advisory-lock contention per parent. No uncertain name may trigger destructive cleanup, no incorrect inode may be reported as success, and a consumed JTI must remain replayed after every downstream failure. A post-commit write denial may leave the authorized new inode at the target with the displaced object quarantined; a post-commit trash denial may leave the exact authorized inode retained with its public name absent. Both are preservation states, not atomic rollback against a hostile same-UID writer.

Clear the temporary token variable and restore defaults after validation:

```bash
unset MCP_TEST_TOKEN
unset MCP_SESSION_ID
unset MCP__CAPABILITY__SESSION_ID
unset MCP__CAPABILITY__CREATE_DIRECTORY_TARGET
unset MCP__CAPABILITY__COPY_FILE_SOURCE
unset MCP__CAPABILITY__COPY_FILE_DESTINATION
unset MCP__CAPABILITY__TRASH_FILE_TARGET
unset MCP__CAPABILITY__WRITE_FILE_TARGET
unset MCP__CAPABILITY__WRITE_FILE_CONTENT_FILE
unset MCP__CAPABILITY__WRITE_FILE_DISPOSITION
rm -f "$MCP_RESPONSE_HEADERS"
rm -f "$GRANT_FILE" "$COPY_GRANT_FILE" "$TRASH_GRANT_FILE" "$WRITE_GRANT_FILE" "$WRITE_CONTENT_FILE"
unset MCP_RESPONSE_HEADERS
unset MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS
unset MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS
unset MCP__TRANSPORT__MAX_BODY_BYTES
unset MCP__TRANSPORT__SSE_ENABLED
```

Use [`operator-validation.md`](operator-validation.md) for representative allowed/denied calls, audit-counter checks, filesystem boundaries, Android status, and capability-token boundary validation.

## Android Cross-Compilation

```bash
rustup target add aarch64-linux-android
ANDROID_NDK_HOME=/path/to/android-ndk ./scripts/cross_compile.sh
BUILD_FEATURES=mcp-runtime \
  ANDROID_NDK_HOME=/path/to/android-ndk \
  ./scripts/cross_compile.sh
BUILD_FEATURES=android-battery-status \
  ANDROID_NDK_HOME=/path/to/android-ndk \
  ./scripts/cross_compile.sh
BUILD_FEATURES=android-volume-status \
  ANDROID_NDK_HOME=/path/to/android-ndk \
  ./scripts/cross_compile.sh
BUILD_FEATURES=command-execution \
  ANDROID_NDK_HOME=/path/to/android-ndk \
  ./scripts/cross_compile.sh
```

The `Android Cross Compile` workflow validates all seven governed postures on relevant pull requests and also supports manual dispatch and `v*` tag builds. Require default, `mcp-runtime`, `android-battery-status`, `android-volume-status`, `android-volume-control`, `command-execution`, and `full-suite` before treating a candidate run as complete. Verify commit, digest, Android AArch64 ELF identity, size, embedded version, and native-Termux evidence as described in [`ANDROID_ARTIFACTS.md`](ANDROID_ARTIFACTS.md). Aggregate schema/gate-v3 evidence must bind the full-suite digest and manifest to the exact 17/21 truth table. Because this aggregate changes the protected runtime/build surface, release qualification also requires a fresh device-harness-v11 physical observation; the v0.5.1 bridge is ineligible.

## MCP Runtime Gate

Do not mark the project as broadly MCP-runtime-ready until each enabled capability has proven:

1. Exact-head CI success.
2. Exact-head Security success when triggered, or documented acceptance of a path-filtered non-run when no dependency, lockfile, or Security workflow input changed.
3. Unauthenticated MCP discovery and invocation are rejected in static-token mode before resource-limit accounting.
4. Authenticated MCP tool discovery works.
5. Request concurrency, timeout, and body-size boundaries are validated.
6. Representative authenticated MCP tool calls work for the enabled surface.
7. Every tool handler enforces its advertised closed input schema with stable non-sensitive errors.
8. Authentication and authorization behavior is documented and tested.
9. Mutating filesystem cancellation cannot trigger destructive cleanup of an uncertain entry or permit consumed-grant replay; trash retains the exact authorized inode in its separate bounded quarantine, and write replacement preserves the displaced object in its write quarantine, including documented post-commit failure states.
10. README, operations, security, roadmap, and changelog documentation match the implemented runtime.
11. Android release artifacts are validated when producing a device build.

The exact-candidate release validator and Termux device-smoke gate must deterministically cover all four filesystem mutation gates, including reversible trash identity/content binding and retained recovery plus the write create/replace truth table, along with install/upgrade/rollback/uninstall recovery. Filesystem authority, grant authorization, and other deterministically testable development changes do not require a new arbitrary 60-minute monitoring window. Physical observation is required only when the release evidence classifier identifies battery, thermal, Android storage/mount, OEM process-management, or other device-only behavior not covered by exact-artifact host tests, emulated Termux, and focused native checks.

## Current Known Limitation

The transport implements stable MCP 2025-11-25 JSON and independently gated bounded-SSE postures, while tool authority intentionally remains staged. It exposes selected low-risk tools, separately gated bounded battery/volume telemetry, fixed server diagnostics, and one separately authorized exact-stream volume control. It does not expose broader Android control, shell fallback, arbitrary command execution, process inventory, arbitrary service inspection, service mutation/control, long-lived server queues, broadcast, or unrelated high-impact controls. Expanding those surfaces is separately threat-modeled product work.
