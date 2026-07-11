# Contributing

This repository is intended to be operated as a security-sensitive Android edge MCP server. Treat every change as production-impacting.

## Development workflow

1. Create a focused branch from current `main`.
2. Keep changes independently reviewable and limited to one implementation or maintenance concern.
3. Run the same local validation gates enforced by CI before opening a pull request:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```

4. Build the affected compile-time posture:

```bash
cargo build --release
cargo build --release --features mcp-runtime
```

5. For Android release validation, build both supported postures:

```bash
ANDROID_NDK_HOME=/path/to/android-ndk \
  BUILD_FEATURES='' \
  ./scripts/cross_compile.sh
ANDROID_NDK_HOME=/path/to/android-ndk \
  BUILD_FEATURES=mcp-runtime \
  ./scripts/cross_compile.sh
```

See [`docs/ANDROID_ARTIFACTS.md`](docs/ANDROID_ARTIFACTS.md) for artifact naming and release evidence.

6. Record the exact head SHA and the applicable CI, Android, and Security results before merge.
7. Do not merge stale, behind-base, cancelled, failing, broadened, or unreviewed work.

## Pull request scope

- Keep dependency, lockfile, and workflow maintenance separate from runtime behavior changes.
- Prefer one active implementation PR at a time.
- Update tests when behavior or a boundary changes.
- Update operator-facing documentation when a runtime surface, configuration key, security assumption, or release procedure changes.
- Preserve existing MCP response contracts unless the PR explicitly documents and tests an intentional change.

## Security expectations

- Do not commit secrets, tokens, tunnel credentials, certificates, private keys, or device-specific configuration.
- New tools must declare their risk profile and minimum required scope.
- Any tool that mutates local files, launches commands, interacts with Android automation, or accesses the network must be disabled by default or protected by explicit scope checks.
- Path-taking code must canonicalize or safely resolve paths and enforce configured safe roots.
- Network-taking code must reject localhost, link-local, private-address, and metadata-service targets unless explicitly and narrowly allowed.
- Command-capable and high-impact surfaces require dedicated compile-time/runtime gates, fixed allowlists, bounded inputs/outputs, audit coverage, tests, and operator documentation.
- Audit counters must retain stable low-cardinality labels only, never secrets or raw caller values.

## Documentation expectations

Every behavioral or security-posture change should update the relevant project-control documentation, including one or more of:

- `README.md`
- `docs/SECURITY.md`
- `docs/OPERATIONS.md`
- `docs/VALIDATION.md`
- `docs/MCP_RUNTIME_ROADMAP.md`
- `CHANGELOG.md`

Documentation-only PRs should still identify their source-of-truth implementation and explain why path-filtered CI/Security non-runs are acceptable.
