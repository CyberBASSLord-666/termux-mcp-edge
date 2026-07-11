# Android Validation Artifacts

The `Android Cross Compile` workflow validates the pinned Rust 1.88.0 toolchain and the `aarch64-linux-android` target against Android NDK r26d. Pull requests that change `src/**`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, the cross-compile script, or the workflow itself trigger this validation.

The workflow builds two isolated feature postures:

- `termux-mcp-server-aarch64-linux-android-default` contains the default feature set. It provides the health/readiness runtime and does not include the staged MCP transport.
- `termux-mcp-server-aarch64-linux-android-mcp-runtime` is built with `--features mcp-runtime`. It contains the authenticated staged MCP transport and its currently enabled bounded tool surface.

Artifact names are part of the release evidence. Do not rename either artifact to a generic Android name or substitute one posture for the other.

## Local reproduction

Default posture:

```bash
ANDROID_NDK_HOME=/path/to/android-ndk \
  BUILD_FEATURES='' \
  ./scripts/cross_compile.sh
```

MCP runtime posture:

```bash
ANDROID_NDK_HOME=/path/to/android-ndk \
  BUILD_FEATURES=mcp-runtime \
  ./scripts/cross_compile.sh
```

Each command writes `target/aarch64-linux-android/release/termux-mcp-server`. Run them in separate worktrees or copy and rename the first output before building the second posture.

## Release evidence

Before treating an Android artifact as releasable, record:

1. The exact commit SHA.
2. The successful exact-head Android workflow run.
3. The artifact name and intended feature posture.
4. The SHA-256 digest and AArch64 Android ELF identity.
5. The embedded `--version` output.
6. On-device `/health` and `/ready` results.
7. For the `mcp-runtime` posture, authenticated discovery and representative allowed and denied tool calls.

Both postures must satisfy the same startup-authentication and deployment requirements. The `mcp-runtime` artifact must additionally preserve Host/Origin validation, request limits, safe-root controls, and audit privacy. It does not enable Android control, arbitrary command execution, shell fallback, arbitrary service mutation, or other high-impact capabilities unless those surfaces are separately implemented behind explicit gates and validated.
