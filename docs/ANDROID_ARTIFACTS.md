# Android Validation Artifacts

The `Android Cross Compile` workflow validates the pinned Rust 1.88.0 toolchain and the `aarch64-linux-android` target against Android NDK r26d. Pull requests and pushes to `main` that change `src/**`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, the cross-compile script, the release-candidate validator, or the workflow itself trigger this validation; version-tag pushes also trigger it regardless of changed paths. Pull-request builds explicitly check out the pull-request head SHA; main/tag builds use the event SHA. This ensures that release evidence can be generated from artifacts rebuilt for the exact merged `main` commit instead of relying on pull-request artifacts.

The workflow builds six isolated feature postures:

- `termux-mcp-server-aarch64-linux-android-default` contains the default feature set. It provides the health/readiness runtime and does not include the MCP transport.
- `termux-mcp-server-aarch64-linux-android-mcp-runtime` is built with `--features mcp-runtime`. It contains the authenticated stable MCP 2025-11-25 transport and its 17-tool staged, bounded surface, including preview-first independently grant-gated reversible `trash_file`.
- `termux-mcp-server-aarch64-linux-android-android-battery-status` is built with `--features android-battery-status`. That feature includes `mcp-runtime`; the additional read-only battery tool remains hidden until its separate runtime flag is enabled.
- `termux-mcp-server-aarch64-linux-android-android-volume-status` is built with `--features android-volume-status`. That feature includes `mcp-runtime`; the additional read-only volume-status tool remains hidden until its separate runtime flag is enabled.
- `termux-mcp-server-aarch64-linux-android-android-volume-control` is built with `--features android-volume-control`. It includes the strict status provider required for fresh bounds checks, but exposes `set_android_volume` only with its independent runtime gate, static authentication, and request-grant key configuration.
- `termux-mcp-server-aarch64-linux-android-command-execution` is built with `--features command-execution`. That feature includes `mcp-runtime`; the additional fixed-profile diagnostic tool remains hidden until `MCP__COMMAND__ENABLED=true`.

Artifact names are part of the release evidence. Do not rename an artifact to a generic Android name or substitute one posture for another.

The workflow names above identify expiring validation bundles. Durable v0.6.0 GitHub Release assets, if publication is separately approved after final exact-main validation, must use:

- `termux-mcp-server-v0.6.0-aarch64-linux-android-default`;
- `termux-mcp-server-v0.6.0-aarch64-linux-android-mcp-runtime`;
- `termux-mcp-server-v0.6.0-aarch64-linux-android-android-battery-status` for a release that includes the optional battery posture.
- `termux-mcp-server-v0.6.0-aarch64-linux-android-android-volume-status` for a release that includes the optional volume posture.
- `termux-mcp-server-v0.6.0-aarch64-linux-android-android-volume-control` for a release that includes request-authorized volume control.
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

Android volume-control posture:

```bash
ANDROID_NDK_HOME=/path/to/android-ndk \
  BUILD_FEATURES=android-volume-control \
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
7. For the `mcp-runtime` posture, authenticated 17-tool discovery and representative allowed and denied tool calls, including four independent default-disabled filesystem mutations and exact-binary offline grant issuance. Artifact evidence must cover dry-run non-consumption, exact-target directory creation, binary copy, identity/content-bound reversible trash retention in its separate private bounded quarantine, content/disposition-bound mode-`0600` file create/replace and its replay denial, exact limits/response preflight, path metadata, and literal search. Trash replay and concurrent-replay denial are separately proven by automated core/integration tests, not attributed to the artifact or native-device gate.
8. For the battery posture, disabled-default discovery plus enabled fixed-path, zero-argument, cleared-environment, normalized-output, immediate endless-output termination, process-group/descendant/cancellation cleanup, provider-failure, audit, and no-device-control checks.
9. For the volume posture, disabled-default discovery plus enabled fixed-path, zero-argument, cleared-environment, exact six-stream normalization, canonical ordering, unknown-field rejection, immediate endless-output termination, process-group/descendant/cancellation cleanup, provider-failure, audit, and no-volume-mutation/device-control checks.
10. For the volume-control posture, incompatible-artifact compile rejection plus disabled/enabled discovery, exact closed schema, preview non-consumption, exact grant binding and replay behavior, fixed two-argument setter, fresh bounds, non-queueing concurrency, verification, rollback confirmed/unconfirmed, cancellation-independent recovery, bounded supervisor cleanup, and redacted audit checks.
11. For the command posture, default-artifact compile-gate rejection plus command-artifact disabled/enabled truth table, exact closed schema, exact-name candidate-to-loaded-image device/inode attestation, `/proc/self/exe` spawning, descriptor-pinned non-root safe cwd after pathname replacement, empty environment, null stdin, immutable 5-second/16 KiB stdout/4 KiB stderr maxima, override/unknown-profile rejection, audit counters, and proof that arbitrary commands and unrelated high-impact controls remain disabled. Require the strict v2 report's exactly 29 MCP requests plus its separate typed wrong-name construction-failure phase, pre-service rejection and redaction evidence, and complete candidate/artifact/environment checks.

All postures must satisfy the same artifact-integrity requirements. The `mcp-runtime`, battery, volume-status, volume-control, and command artifacts expose MCP authentication and transport; all must preserve Host/Origin validation, request limits, safe-root controls, and audit privacy. Each optional artifact adds only its documented bounded tool. The control posture adds only exact-stream request-authorized volume mutation; the command posture adds fixed server diagnostics but does not enable arbitrary command execution, shell fallback, arbitrary service mutation, or unrelated high-impact capabilities.

After downloading the default, `mcp-runtime`, and `android-volume-control` exact-head artifacts, validate them through [`RELEASE_CANDIDATE_VALIDATION.md`](RELEASE_CANDIDATE_VALIDATION.md). The offline validator requires the expected commit, version, workflow run IDs, artifact/manifest paths, and SHA-256 digests; reconciles all three manifests, proves the default artifact has no MCP route, exercises the `mcp-runtime` posture including reversible-trash authorization/recovery/privacy, and proves the control artifact's compile/runtime default-deny state without invoking the device setter. It emits schema-versioned evidence without retaining artifact paths, tokens, session IDs, bodies, or file contents.

The Android workflow validates the third artifact with [`termux_battery_emulated_gate.sh`](../scripts/termux_battery_emulated_gate.sh), the fourth with [`termux_volume_emulated_gate.sh`](../scripts/termux_volume_emulated_gate.sh), the fifth with [`termux_volume_control_emulated_gate.sh`](../scripts/termux_volume_control_emulated_gate.sh), and the sixth with [`termux_command_emulated_gate.sh`](../scripts/termux_command_emulated_gate.sh) inside the digest-pinned official Termux image on native ARM64. Control and command gates also consume incompatible artifacts to prove compile-time rejection. Their separate sanitized reports conform to the posture-specific versioned schemas. Passing feature-emulation reports are required development evidence but are never, by themselves, physical-device release qualification.
