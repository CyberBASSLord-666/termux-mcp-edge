# Contributing

This repository is intended to be operated as a security-sensitive Android edge MCP server. Treat every change as production-impacting.

## Development workflow

1. Create a focused branch from current `main`.
2. Keep changes independently reviewable and limited to one implementation or maintenance concern.
3. Run the core local validation matrix before opening a pull request. The checked-in [CI workflow](.github/workflows/ci.yml) is authoritative and adds dependency-input and runner-specific invariants:

The documentation contract uses Python 3 to verify relative Markdown links.

```bash
cargo metadata --locked --all-features --format-version 1 --no-deps >/dev/null
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo clippy --locked --workspace --all-targets --features mcp-runtime -- -D warnings
cargo clippy --locked --workspace --all-targets --features full-suite -- -D warnings
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets
cargo test --locked --workspace --all-targets --features mcp-runtime
cargo test --locked --workspace --all-targets --features full-suite
cargo test --locked --workspace --all-targets --all-features
cargo build --locked --release --features full-suite
bash tests/package_android_artifact_test.sh
bash tests/termux_deploy_test.sh
bash tests/termux_device_smoke_test.sh
bash tests/termux_emulated_gate_test.sh
bash tests/termux_release_validate_test.sh
bash tests/setup_named_tunnel_test.sh
bash tests/documentation_contract_test.sh
bash tests/cross_compile_contract_test.sh
bash tests/package_physical_qualification_test.sh
bash tests/stage_release_assets_test.sh
bash tests/release_staging_workflow_test.sh
```

4. Build the affected compile-time posture:

```bash
cargo build --release --locked
cargo build --release --locked --features mcp-runtime
cargo build --release --locked --features android-battery-status
cargo build --release --locked --features android-volume-status
cargo build --release --locked --features android-volume-control
cargo build --release --locked --features command-execution
cargo build --release --locked --features full-suite
```

5. For Android release validation, build every affected supported posture. The seven governed postures are six deliberately isolated least-privilege artifacts plus the named `full-suite` aggregate. A raw `--all-features` binary remains useful for host compatibility testing, but it is not the aggregate release contract and must not substitute for `full-suite`.

```bash
ANDROID_NDK_HOME=/path/to/android-ndk \
  BUILD_FEATURES='' \
  ./scripts/cross_compile.sh
ANDROID_NDK_HOME=/path/to/android-ndk \
  BUILD_FEATURES=mcp-runtime \
  ./scripts/cross_compile.sh
ANDROID_NDK_HOME=/path/to/android-ndk \
  BUILD_FEATURES=android-battery-status \
  ./scripts/cross_compile.sh
ANDROID_NDK_HOME=/path/to/android-ndk \
  BUILD_FEATURES=android-volume-status \
  ./scripts/cross_compile.sh
ANDROID_NDK_HOME=/path/to/android-ndk \
  BUILD_FEATURES=android-volume-control \
  ./scripts/cross_compile.sh
ANDROID_NDK_HOME=/path/to/android-ndk \
  BUILD_FEATURES=command-execution \
  ./scripts/cross_compile.sh
ANDROID_NDK_HOME=/path/to/android-ndk \
  BUILD_FEATURES=full-suite \
  ./scripts/cross_compile.sh
```

See [`docs/ANDROID_ARTIFACTS.md`](docs/ANDROID_ARTIFACTS.md) for artifact naming and release evidence. Release-staging changes must also preserve the exact-byte, evidence-lineage, read-only permission, and protected-environment contracts in [`docs/PUBLIC_RELEASE.md`](docs/PUBLIC_RELEASE.md); they must not add a candidate rebuild, tag, or GitHub Release path.

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
