# Validation

## Current Runtime Validation Scope

The default compiled runtime is an Axum HTTP health/readiness service. The optional `mcp-runtime` feature compiles the staged `/mcp` transport and its current limited tool surface.

Current staged MCP tools are `runtime_status`, `platform_info`, `android_status`, `project_service_status`, `list_directory`, `read_file`, and `write_file`. Android control, shell fallback, arbitrary command execution, process inventory, arbitrary service inspection, service mutation/control, and high-impact tools remain out of scope for the live runtime.

The optional MCP transport enforces authentication before mobile-conscious concurrency, timeout, body-size, Host, Origin, JSON-RPC, discovery, and invocation handling.

## Required Repository Gates

Run the same Rust gates enforced by `.github/workflows/ci.yml`:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```

Build both supported compile-time postures when preparing a release candidate:

```bash
cargo build --release
cargo build --release --features mcp-runtime
```

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
  --data '{"jsonrpc":"2.0","id":0,"method":"tools/list"}' \
  http://127.0.0.1:8000/mcp
```

Expected behavior: HTTP 401, `WWW-Authenticate: Bearer`, a non-sensitive `unauthorized` response, and no tool-discovery result.

Then verify authenticated discovery using the exact allowed `Host` and `Origin` headers:

```bash
curl -sS \
  -X POST \
  -H "Authorization: Bearer ${MCP_TEST_TOKEN}" \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  http://127.0.0.1:8000/mcp
```

Confirm discovery returns exactly the staged tools expected for the current release line: `runtime_status`, `platform_info`, `android_status`, `project_service_status`, `list_directory`, `read_file`, and `write_file`.

Validate the project-owned service status tool with the current allowlisted service name:

```bash
curl -sS \
  -X POST \
  -H "Authorization: Bearer ${MCP_TEST_TOKEN}" \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"project_service_status","arguments":{"service_name":"mcp_runtime"}}}' \
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
```

The `Android Cross Compile` workflow validates both postures on relevant pull requests and also supports manual dispatch and `v*` tag builds. Require the posture-specific `termux-mcp-server-aarch64-linux-android-default` and `termux-mcp-server-aarch64-linux-android-mcp-runtime` artifacts before treating a release run as complete. Verify their commit, digest, Android AArch64 ELF identity, size, and embedded version as described in [`ANDROID_ARTIFACTS.md`](ANDROID_ARTIFACTS.md).

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

The current runtime intentionally remains staged. It exposes selected low-risk and controlled MCP tools, but it does not expose Android platform control, shell fallback, arbitrary command execution, process inventory, arbitrary service inspection, service mutation/control, or high-impact controls. Restoring those surfaces is product work, not cleanup-only work.
