# Validation

## Current Runtime Validation Scope

The default compiled runtime is an Axum HTTP health-check service. The optional `mcp-runtime` feature compiles the staged `/mcp` transport and its current limited tool surface.

Current staged MCP tools are `runtime_status`, `platform_info`, `android_status`, `project_service_status`, `list_directory`, `read_file`, and `write_file`. Later Android control, shell fallback, arbitrary command execution, process inventory, arbitrary service inspection, service mutation/control, and high-impact tools remain out of scope.

## Required Repository Gates

Run these from a Rust-enabled desktop or Termux environment with the Android build prerequisites installed:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo build --release
```

The GitHub CI workflow enforces format, Clippy, and tests. The Security workflow generates a lockfile and runs `cargo audit`.

## Dependency Update Validation

Dependency update PRs must remain separate from runtime behavior changes. Before merging a Cargo or GitHub Actions dependency update:

1. Confirm the PR diff is limited to dependency metadata, workflow pin updates, or generated lockfile changes.
2. Confirm exact-head CI succeeds for the dependency-update head SHA.
3. Confirm exact-head Security succeeds for the dependency-update head SHA.
4. Confirm the Security workflow output does not report unresolved advisories.
5. Avoid bundling dependency updates with MCP transport, browser-exposed routes, filesystem tools, system tools, or command-capable tool exposure.

If a dependency update is required to restore MCP transport or high-impact tools, keep it blocked until the related transport protections, authorization policy, and smoke tests are present in the same focused restoration stage or in already-merged prerequisite PRs.

## Runtime Smoke Test

After building or installing the binary, verify liveness:

```bash
curl -fsS http://127.0.0.1:8000/health
```

Expected response:

```text
ok
```

## Staged MCP Smoke Tests

When built with `--features mcp-runtime`, verify the transport using exact allowed `Host` and `Origin` headers:

```bash
curl -sS \
  -X POST \
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
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"project_service_status","arguments":{"service_name":"mcp_runtime"}}}' \
  http://127.0.0.1:8000/mcp
```

Expected behavior: the response is read-only, reports only the allowlisted project-owned logical runtime service, and does not expose process inventory, shell fallback, arbitrary service names, or control actions.

## Android Cross-Compilation

```bash
rustup target add aarch64-linux-android
ANDROID_NDK_HOME=/path/to/android-ndk ./scripts/cross_compile.sh
```

## MCP Runtime Gate

Do not mark the project as broadly MCP-runtime-ready until each enabled capability has proven:

1. Exact-head CI success.
2. Exact-head Security success when triggered, or documented acceptance of a docs-only/path-filtered non-run.
3. MCP tool discovery works.
4. Representative MCP tool calls work for the enabled surface.
5. Authentication and authorization behavior is documented and tested.
6. README, operations, and security docs match the implemented runtime.

## Current Known Limitation

The current runtime intentionally remains staged. It exposes selected low-risk and controlled MCP tools, but it does not expose Android platform control, shell fallback, arbitrary command execution, process inventory, arbitrary service inspection, service mutation/control, or high-impact controls. Restoring those surfaces is product work, not cleanup-only work.
