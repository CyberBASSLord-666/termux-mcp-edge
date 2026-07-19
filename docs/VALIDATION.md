# Validation

## Current Runtime Validation Scope

The default compiled runtime is an Axum HTTP health/readiness service. The optional `mcp-runtime` feature compiles stable MCP 2025-11-25 Streamable HTTP handling at `/mcp` and its current limited tool surface.

The baseline staged MCP tools are `runtime_status`, `platform_info`, `android_status`, `project_service_status`, `create_directory`, `copy_file`, `hash_file`, `list_directory`, `path_metadata`, `read_file`, `search_text`, and `write_file`. Separately built and runtime-enabled postures may add bounded read-only `android_battery_status`, `android_volume_status`, the fixed server-diagnostic `run_command_profile`, or preview-first request-authorized `set_android_volume`. Directory preview is baseline, but directory mutation is independently default-disabled and requires one request-scoped grant. Android controls beyond exact-stream volume, shell fallback, arbitrary command execution, process inventory, arbitrary service inspection, service mutation/control, and unrelated high-impact tools remain out of scope for the live runtime.

The optional MCP transport enforces authentication before mobile-conscious concurrency, timeout, body-size, Host, Origin, JSON-RPC, discovery, and invocation handling.

## Required Repository Gates

Run the same Rust gates enforced by `.github/workflows/ci.yml`:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```

Build all supported compile-time postures when preparing a release candidate:

```bash
cargo build --release
cargo build --release --features mcp-runtime
cargo build --release --features android-battery-status
cargo build --release --features android-volume-status
cargo build --release --features android-volume-control
cargo build --release --features command-execution
```

Validate the exact release candidate on an AArch64 Termux device with the no-clone harness in [`DEVICE_PRODUCTION_GATE.md`](DEVICE_PRODUCTION_GATE.md). Its companion contract test runs in CI as `tests/termux_device_smoke_test.sh`; CI validates the harness interface and required coverage markers, while the actual run requires a real Termux/runit device.

Validate downloaded default, `mcp-runtime`, and `android-volume-control` artifacts with [`RELEASE_CANDIDATE_VALIDATION.md`](RELEASE_CANDIDATE_VALIDATION.md). CI runs `tests/package_android_artifact_test.sh` for exact-source manifest/checksum bundle construction and `tests/termux_release_validate_test.sh` against deterministic default/MCP/control HTTP fixtures and deployment-manager fixture mode. Coverage includes preflight success, three-way provenance/digest/architecture/symlink/metadata failures, artifact-change detection, wrong feature posture, the volume-control compile/default-disabled truth table without device mutation, confirmation gates, transport/response/safe-root contracts, failed upgrade/rollback recovery, interruption cleanup, redaction, and the versioned JSON evidence contract.

The CI workflow enforces format, Clippy, and all-feature tests. The Security workflow validates the locked dependency graph with `cargo audit` and fails on audit findings.

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
  http://127.0.0.1:8000/mcp | jq -e '.result.tools | length == 12'
```

Confirm a normal `mcp-runtime` build returns exactly the twelve baseline tools. An optional build still returns twelve unless its corresponding runtime flag is true; then its one additional tool is thirteenth. An all-feature validation build returns sixteen only when battery, volume-status, volume-control, and command runtime flags are all true. With `MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=false`, prove `create_directory` discovery constrains `dry_run` to `true` and explicit mutation is denied. With the gate, static authentication, and paired capability key enabled, exercise preview and explicit mode through an exact-binary locally issued grant. Prove missing/wrong-context/wrong-binding grants fail, dry run does not consume, one exact target succeeds at mode `0700`, and replay is denied. For volume control, prove disabled discovery, closed schema, preview non-consumption, every exact binding and replay denial, fresh maximum enforcement, fixed two-argument execution, non-queueing concurrency, verified success, and rollback confirmed/unconfirmed without private reflection. Exercise `copy_file` with binary content in preview and explicit mode, prove fixed mode `0600`, exact bytes, absent-destination/no-replace behavior, one-byte-over rejection, content-private responses, and pre-mutation full-response bounding. Exercise `hash_file` with binary and boundary fixtures, prove exact SHA-256/size output, pre-read full-response bounding, one-byte-over rejection, no-follow descriptor confinement, and digest/path/content-private audits. Exercise `path_metadata` and literal `search_text` under their documented content-free ceilings. Also verify a GET with `Accept: text/event-stream` plus the active protocol/session headers returns HTTP 405; this is the documented non-SSE Streamable HTTP posture.

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

### Write cancellation cleanup

Explicit mutation continues to use a same-directory temporary file and atomic rename. The temporary path is protected by a drop cleanup guard. Unit coverage must prove an armed guard removes the temp file and a disarmed guard preserves a successfully committed file. After a forced timeout/cancellation test, no `.*.tmp` artifact should remain in the safe root.

Clear the temporary token variable and restore defaults after validation:

```bash
unset MCP_TEST_TOKEN
unset MCP_SESSION_ID
unset MCP__CAPABILITY__SESSION_ID
unset MCP__CAPABILITY__CREATE_DIRECTORY_TARGET
rm -f "$MCP_RESPONSE_HEADERS"
unset MCP_RESPONSE_HEADERS
unset MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS
unset MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS
unset MCP__TRANSPORT__MAX_BODY_BYTES
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

The `Android Cross Compile` workflow validates all six postures on relevant pull requests and also supports manual dispatch and `v*` tag builds. Require the posture-specific default, `mcp-runtime`, `android-battery-status`, `android-volume-status`, `android-volume-control`, and `command-execution` artifacts before treating a release run that publishes the optional features as complete. Verify their commit, digest, Android AArch64 ELF identity, size, embedded version, and native-Termux evidence as described in [`ANDROID_ARTIFACTS.md`](ANDROID_ARTIFACTS.md). Provider and control evidence requires prompt endless-output rejection plus process-group, pipe-holder, client-cancellation, and bounded-supervisor cleanup attestations; control additionally proves exact authorization, verification, and recovery. Command evidence proves default-artifact compile rejection, runtime-disabled hiding, exact profiles/schema, boundary isolation, override rejection, and audit counters without a long observation. Host regressions separately force cleanup-reserve exhaustion on timeout, both output-limit paths, and caller cancellation, requiring the stable wait failure to override the primary result only after direct-child reaping.

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
9. Mutating filesystem cancellation does not strand temporary files.
10. README, operations, security, roadmap, and changelog documentation match the implemented runtime.
11. Android release artifacts are validated when producing a device build.

## Current Known Limitation

The transport implements the stable non-SSE MCP 2025-11-25 posture, while tool authority intentionally remains staged. It exposes selected low-risk tools, separately gated bounded battery/volume telemetry, fixed server diagnostics, and one separately authorized exact-stream volume control. It does not expose broader Android control, shell fallback, arbitrary command execution, process inventory, arbitrary service inspection, service mutation/control, or unrelated high-impact controls. Expanding those surfaces is separately threat-modeled product work.
