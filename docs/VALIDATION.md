# Validation

## Current Runtime Validation Scope

The default compiled runtime is an Axum HTTP health-check service. The optional `mcp-runtime` feature compiles the staged `/mcp` transport and its current limited tool surface.

Current staged MCP tools are `runtime_status`, `platform_info`, `android_status`, `project_service_status`, `list_directory`, `read_file`, and `write_file`. Android control, shell fallback, arbitrary command execution, process inventory, arbitrary service inspection, service mutation/control, and high-impact tools remain out of scope for the live runtime.

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

The `/health` and `/ready` operational probes do not require bearer authentication. They must not return secrets, raw configuration, private paths, or tool output.

## Staged MCP Smoke Tests

When built with `--features mcp-runtime`, load the configured token without printing it:

```bash
MCP_TEST_TOKEN="$(cat "$HOME/.termux_mcp_token")"
```

First prove unauthenticated discovery is rejected before JSON-RPC dispatch:

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

Clear the temporary shell variable after validation:

```bash
unset MCP_TEST_TOKEN
```

Use [`operator-validation.md`](operator-validation.md) for representative allowed/denied calls, audit-counter checks, filesystem boundaries, Android status, and capability-token boundary validation.

## Android Cross-Compilation

```bash
rustup target add aarch64-linux-android
ANDROID_NDK_HOME=/path/to/android-ndk ./scripts/cross_compile.sh
```

The `Android Cross Compile` workflow also supports manual dispatch and `v*` tag builds. Verify the uploaded artifact exists and contains `termux-mcp-server` before treating the run as release evidence.

## MCP Runtime Gate

Do not mark the project as broadly MCP-runtime-ready until each enabled capability has proven:

1. Exact-head CI success.
2. Exact-head Security success when triggered, or documented acceptance of a path-filtered non-run when no dependency, lockfile, or Security workflow input changed.
3. Unauthenticated MCP discovery and invocation are rejected in static-token mode.
4. Authenticated MCP tool discovery works.
5. Representative authenticated MCP tool calls work for the enabled surface.
6. Authentication and authorization behavior is documented and tested.
7. README, operations, security, roadmap, and changelog documentation match the implemented runtime.
8. Android release artifacts are validated when producing a device build.

## Current Known Limitation

The current runtime intentionally remains staged. It exposes selected low-risk and controlled MCP tools, but it does not expose Android platform control, shell fallback, arbitrary command execution, process inventory, arbitrary service inspection, service mutation/control, or high-impact controls. Restoring those surfaces is product work, not cleanup-only work.
