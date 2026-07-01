# Validation

## Current Runtime Validation Scope

The current compiled runtime is an Axum HTTP health-check service. MCP transport and MCP tool endpoints are not compiled into the current release line.

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

## Android Cross-Compilation

```bash
rustup target add aarch64-linux-android
ANDROID_NDK_HOME=/path/to/android-ndk ./scripts/cross_compile.sh
```

## MCP Transport Restoration Gate

Do not mark the project as MCP-runtime-ready until a future PR restores transport integration and proves:

1. Exact-head CI success.
2. Exact-head Security success.
3. MCP tool discovery works.
4. At least one MCP tool call works.
5. Authentication and authorization behavior is documented and tested.
6. README, operations, and security docs match the implemented runtime.

## Current Known Limitation

The current runtime intentionally does not expose MCP transport or MCP tools. This is a documented safety posture after removing vulnerable and incompatible dependency surfaces. Restoring transport is product work, not a cleanup-only change.
