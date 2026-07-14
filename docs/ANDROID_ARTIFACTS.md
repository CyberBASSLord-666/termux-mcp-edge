# Android Validation Artifacts

The `Android Cross Compile` workflow validates the pinned Rust 1.88.0 toolchain and the `aarch64-linux-android` target against Android NDK r26d. Pull requests and pushes to `main` that change `src/**`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, the cross-compile script, the release-candidate validator, or the workflow itself trigger this validation; version-tag pushes also trigger it regardless of changed paths. Pull-request builds explicitly check out the pull-request head SHA; main/tag builds use the event SHA. This ensures that release evidence can be generated from artifacts rebuilt for the exact merged `main` commit instead of relying on pull-request artifacts.

The workflow builds five isolated feature postures:

- `termux-mcp-server-aarch64-linux-android-default` contains the default feature set. It provides the health/readiness runtime and does not include the MCP transport.
- `termux-mcp-server-aarch64-linux-android-mcp-runtime` is built with `--features mcp-runtime`. It contains the authenticated stable MCP 2025-11-25 transport and its currently enabled staged, bounded tool surface.
- `termux-mcp-server-aarch64-linux-android-android-battery-status` is built with `--features android-battery-status`. That feature includes `mcp-runtime`; the additional read-only battery tool remains hidden until its separate runtime flag is enabled.
- `termux-mcp-server-aarch64-linux-android-android-volume-status` is built with `--features android-volume-status`. That feature includes `mcp-runtime`; the additional read-only volume-status tool remains hidden until its separate runtime flag is enabled.
- `termux-mcp-server-aarch64-linux-android-command-execution` is built with `--features command-execution`. That feature includes `mcp-runtime`; the additional fixed-profile diagnostic tool remains hidden until `MCP__COMMAND__ENABLED=true`.

Artifact names are part of the release evidence. Do not rename an artifact to a generic Android name or substitute one posture for another.

The workflow names above identify expiring validation bundles. Durable v0.6.0 GitHub Release assets, if publication is separately approved after final exact-main validation, must use:

- `termux-mcp-server-v0.6.0-aarch64-linux-android-default`;
- `termux-mcp-server-v0.6.0-aarch64-linux-android-mcp-runtime`;
- `termux-mcp-server-v0.6.0-aarch64-linux-android-android-battery-status` for a release that includes the optional battery posture.
- `termux-mcp-server-v0.6.0-aarch64-linux-android-android-volume-status` for a release that includes the optional volume posture.
- `termux-mcp-server-v0.6.0-aarch64-linux-android-command-execution` for a release that includes the optional fixed-command posture.

Each durable binary must be accompanied by its checksum/manifest material. A workflow bundle must not be presented as the durable release asset merely because its internal executable has the expected digest.

Each downloaded workflow artifact is a three-file bundle:

- `termux-mcp-server`: the posture-specific executable;
- `SHA256SUMS`: a checksum sidecar for the executable;
- `artifact-manifest.json`: schema-versioned repository, exact source SHA, workflow run ID, artifact name, posture/features, target, package version, digest, size, and ELF classification.

Keep all posture bundles in separate directories because their internal filenames intentionally match. GitHub artifact extraction may not retain executable mode; set the binary to mode `0700` before validation without changing its contents.

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

Android battery posture:

```bash
ANDROID_NDK_HOME=/path/to/android-ndk \
  BUILD_FEATURES=android-battery-status \
  ./scripts/cross_compile.sh
```

Android volume posture:

```bash
ANDROID_NDK_HOME=/path/to/android-ndk \
  BUILD_FEATURES=android-volume-status \
  ./scripts/cross_compile.sh
```

Fixed command posture:

```bash
ANDROID_NDK_HOME=/path/to/android-ndk \
  BUILD_FEATURES=command-execution \
  ./scripts/cross_compile.sh
```

Each command writes `target/aarch64-linux-android/release/termux-mcp-server`. Run them in separate worktrees or preserve each output before building another posture.

## Release evidence

Before treating an Android artifact as releasable, record:

1. The exact commit SHA.
2. The successful exact-head Android workflow run.
3. The artifact name, manifest, checksum sidecar, and intended feature posture.
4. The SHA-256 digest and AArch64 Android ELF identity.
5. The embedded `--version` output.
6. On-device `/health` and `/ready` results.
7. For the `mcp-runtime` posture, authenticated discovery and representative allowed and denied tool calls.
8. For the battery posture, disabled-default discovery plus enabled fixed-path, zero-argument, cleared-environment, normalized-output, immediate endless-output termination, process-group/descendant/cancellation cleanup, provider-failure, audit, and no-device-control checks.
9. For the volume posture, disabled-default discovery plus enabled fixed-path, zero-argument, cleared-environment, exact six-stream normalization, canonical ordering, unknown-field rejection, immediate endless-output termination, process-group/descendant/cancellation cleanup, provider-failure, audit, and no-volume-mutation/device-control checks.
10. For the command posture, default-artifact compile-gate rejection plus command-artifact disabled/enabled truth table, exact closed schema, fixed current executable and argv, safe-root cwd, empty environment, null stdin, bounded UTF-8 results, override/unknown-profile rejection, audit counters, and proof that arbitrary commands and high-impact controls remain disabled.

All postures must satisfy the same artifact-integrity requirements. The `mcp-runtime`, battery, volume, and command artifacts expose MCP authentication and transport; all must preserve Host/Origin validation, request limits, safe-root controls, and audit privacy. Each optional artifact adds only its documented read-only tool. The command posture adds fixed server diagnostics but does not enable arbitrary command execution, shell fallback, Android or volume control, arbitrary service mutation, or other high-impact capabilities.

After downloading the default and `mcp-runtime` exact-head artifacts, validate them through [`RELEASE_CANDIDATE_VALIDATION.md`](RELEASE_CANDIDATE_VALIDATION.md). The offline validator requires the expected commit, version, workflow run IDs, artifact/manifest paths, and SHA-256 digests; reconciles both manifests, proves the default artifact has no MCP route, exercises the `mcp-runtime` posture, and emits schema-versioned evidence without retaining artifact paths, tokens, session IDs, bodies, or file contents.

The Android workflow validates the third artifact with [`termux_battery_emulated_gate.sh`](../scripts/termux_battery_emulated_gate.sh), the fourth with [`termux_volume_emulated_gate.sh`](../scripts/termux_volume_emulated_gate.sh), and the fifth with [`termux_command_emulated_gate.sh`](../scripts/termux_command_emulated_gate.sh) inside the digest-pinned official Termux image on native ARM64. The command gate also consumes the exact default artifact to prove compile-time rejection. Their separate sanitized reports conform to [`android-battery-emulated-evidence-schema-v2.json`](android-battery-emulated-evidence-schema-v2.json), [`android-volume-emulated-evidence-schema-v1.json`](android-volume-emulated-evidence-schema-v1.json), and [`command-emulated-evidence-schema-v1.json`](command-emulated-evidence-schema-v1.json); battery v1 remains available only as the historical pre-supervisor contract. Passing feature-emulation reports are required development evidence but are never, by themselves, physical-device release qualification.
