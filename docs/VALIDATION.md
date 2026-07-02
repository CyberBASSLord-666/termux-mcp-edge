# Validation

## Current automated pass

This pass was performed in a Rust-enabled container on the current feature branch.

### Checks completed

- Verified the authenticated HTTP/SSE MCP transport compiles.
- Verified bearer-token helper tests, filesystem integration tests, mock client tests, and path-sanitization property tests pass.
- Ran clippy with all targets and all features with warnings denied.
- Performed a focused security review against `main` covering authentication, session lifecycle, filesystem containment, subprocess execution, temporary-file handling, and denial-of-service limits.

### Validation commands to run locally

Run these from a Rust-enabled desktop or Termux environment with the Android build prerequisites installed:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo build --release
```

For Android cross-compilation:

```bash
rustup target add aarch64-linux-android
ANDROID_NDK_HOME=/path/to/android-ndk ./scripts/cross_compile.sh
```

### Validation not completed in this run

Android cross-compilation was not executed because this container does not provide an Android NDK path. Install the Android NDK and run the cross-compilation command above before publishing an Android release artifact.

`cargo-audit` and `cargo-deny` are recommended for release gates, but those Cargo subcommands may need to be installed separately in local or CI environments.
