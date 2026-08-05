# Android Validation Artifacts

The `Android Cross Compile` workflow validates the pinned Rust 1.88.0 toolchain and the `aarch64-linux-android` target against Android NDK r26d. Every artifact build and its package-version metadata query use the committed `Cargo.lock` and fail instead of resolving a different graph; packaging also verifies that `Cargo.toml` and `Cargo.lock` remain unchanged. Pull requests and pushes to `main` that change release inputs trigger this validation. A version tag does not start a rebuild: public staging must preserve the already qualified exact-main bytes instead of manufacturing a second candidate. Pull-request builds explicitly check out the pull-request head SHA and main builds use the event SHA.

The workflow builds seven governed feature postures. The first six preserve least-privilege deployment choices; the seventh is the explicit aggregate:

- `termux-mcp-server-aarch64-linux-android-default` contains the default feature set. It provides the health/readiness runtime and does not include the MCP transport.
- `termux-mcp-server-aarch64-linux-android-mcp-runtime` is built with `--features mcp-runtime`. It contains the authenticated stable MCP 2025-11-25 transport and its 17-tool staged, bounded surface, including preview-first independently grant-gated reversible `trash_file`.
- `termux-mcp-server-aarch64-linux-android-android-battery-status` is built with `--features android-battery-status`. That feature includes `mcp-runtime`; the additional read-only battery tool remains hidden until its separate runtime flag is enabled.
- `termux-mcp-server-aarch64-linux-android-android-volume-status` is built with `--features android-volume-status`. That feature includes `mcp-runtime`; the additional read-only volume-status tool remains hidden until its separate runtime flag is enabled.
- `termux-mcp-server-aarch64-linux-android-android-volume-control` is built with `--features android-volume-control`. It includes the strict status provider required for fresh bounds checks, but exposes `set_android_volume` only with its independent runtime gate, static authentication, and request-grant key configuration.
- `termux-mcp-server-aarch64-linux-android-command-execution` is built with `--features command-execution`. That feature includes `mcp-runtime`; the additional fixed-profile diagnostic tool remains hidden until `MCP__COMMAND__ENABLED=true`.
- `termux-mcp-server-aarch64-linux-android-full-suite` is built with `--features full-suite`. It composes the four governed optional release providers in one named artifact, but all four optional runtime gates remain independent. It exposes exactly 17 tools with those gates off and exactly 21 only when all four are enabled.

Artifact names are part of the release evidence. Do not rename an artifact to a generic Android name or substitute one posture for another. In particular, a raw Cargo `--all-features` build is a development compatibility lane, not the governed `full-suite` bundle.

The isolated `android-rish` feature is not an eighth governed artifact and is not a `full-suite` constituent. CI compiles, lints, and tests its default-disabled exact-token diagnostic foundation, but the official Termux container explicitly lacks an Android framework and cannot exercise Shizuku Binder permission or lifecycle. Adding a release-capable rish artifact requires a typed lifecycle-authoritative bridge, a new reviewed evidence-schema and inventory revision, and exact physical AArch64 Android/Termux qualification together; existing physical v1 evidence and the existing seven-bundle, nine-workflow-artifact, twelve-qualification-member, and sixteen-public-asset contracts must not be reinterpreted as that evidence. See [`SHIZUKU_RISH_CONTROL_PLANE.md`](SHIZUKU_RISH_CONTROL_PLANE.md).

The workflow names above identify expiring validation bundles. Durable v0.7.0 GitHub Release assets, if publication is separately approved after final exact-main validation, must use:

- `termux-mcp-server-v0.7.0-aarch64-linux-android-default`;
- `termux-mcp-server-v0.7.0-aarch64-linux-android-mcp-runtime`;
- `termux-mcp-server-v0.7.0-aarch64-linux-android-android-battery-status` for a release that includes the optional battery posture.
- `termux-mcp-server-v0.7.0-aarch64-linux-android-android-volume-status` for a release that includes the optional volume posture.
- `termux-mcp-server-v0.7.0-aarch64-linux-android-android-volume-control` for a release that includes request-authorized volume control.
- `termux-mcp-server-v0.7.0-aarch64-linux-android-command-execution` for a release that includes the optional fixed-command posture.
- `termux-mcp-server-v0.7.0-aarch64-linux-android-full-suite` for the governed aggregate posture.

Each durable binary must be accompanied by its matching `.sha256` sidecar. The closed public inventory is exactly the seven binaries, seven sidecars, combined `SHA256SUMS`, and unchanged raw deterministic staging tar. The tar retains all seven workflow manifests, the release-staging manifest, LICENSE, sanitized component evidence, and the retained runtime archive/package lock/snapshot/offline replay under `evidence/runtime/`. Those members are not separate Release assets, and GitHub's generated source archives are not governed Android assets. A workflow bundle must not be presented as the durable release asset merely because its internal executable has the expected digest.

The Android run emits exactly nine artifacts: seven posture bundles, the seven-report native-Termux component artifact, and the three-file retained-runtime snapshot artifact. All nine use the same 30-day retention lane so the separate completed-run qualifier and protected staging review can finish against the same immutable inputs. They remain expiring validation inputs, not a distribution channel. Neither the resulting stage nor a draft Release is an installation source. [`PUBLIC_RELEASE.md`](PUBLIC_RELEASE.md) defines the exact-byte staging, fixed sixteen-asset publication, and immutable public-proof boundaries.

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

Full-suite posture:

```bash
ANDROID_NDK_HOME=/path/to/android-ndk \
  BUILD_FEATURES=full-suite \
  ./scripts/cross_compile.sh
```

Each command writes `target/aarch64-linux-android/release/termux-mcp-server`. The wrapper always supplies Cargo's `--locked` option; a missing or stale lockfile is an error. Run the commands in separate worktrees or preserve each output before building another posture.

## Release evidence

Before treating an Android artifact as releasable, record:

1. The exact commit SHA.
2. The successful exact-head Android workflow run.
3. The artifact name, manifest, checksum sidecar, and intended feature posture.
4. The SHA-256 digest and AArch64 Android ELF identity.
5. The embedded `--version` output.
6. Native official-Termux `/health` and `/ready` results for ordinary automated qualification; record on-device results only when the separate optional physical-certification tier is requested.
7. For the `mcp-runtime` posture, authenticated 17-tool discovery and representative allowed and denied tool calls, including four independent default-disabled filesystem mutations and exact-binary offline grant issuance. Artifact evidence must cover dry-run non-consumption, exact-target directory creation, binary copy, identity/content-bound reversible trash retention in its separate private bounded quarantine, content/disposition-bound mode-`0600` file create/replace and its replay denial, exact limits/response preflight, path metadata, and literal search. Trash replay and concurrent-replay denial are separately proven by automated core/integration tests, not attributed to the artifact or native-device gate.
8. For the battery posture, disabled-default discovery plus enabled fixed-path, zero-argument, cleared-environment, normalized-output, immediate endless-output termination, process-group/descendant/cancellation cleanup, provider-failure, audit, and no-device-control checks.
9. For the volume posture, disabled-default discovery plus enabled fixed-path, zero-argument, cleared-environment, exact six-stream normalization, canonical ordering, unknown-field rejection, immediate endless-output termination, process-group/descendant/cancellation cleanup, provider-failure, audit, and no-volume-mutation/device-control checks.
10. For the volume-control posture, incompatible-artifact compile rejection plus disabled/enabled discovery, exact closed schema, preview non-consumption, exact grant binding and replay behavior, fixed two-argument setter, fresh bounds, non-queueing concurrency, verification, rollback confirmed/unconfirmed, cancellation-independent recovery, bounded supervisor cleanup, and redacted audit checks.
11. For the command posture, default-artifact compile-gate rejection plus command-artifact disabled/enabled truth table, exact closed schema, exact-name candidate-to-loaded-image device/inode attestation, `/proc/self/exe` spawning, descriptor-pinned non-root safe cwd after pathname replacement, empty environment, null stdin, immutable 5-second/16 KiB stdout/4 KiB stderr maxima, override/unknown-profile rejection, audit counters, and proof that arbitrary commands and unrelated high-impact controls remain disabled. Require the strict v3 report's exactly 29 MCP requests plus its separate typed wrong-name construction-failure phase, pre-service rejection and redaction evidence, complete candidate/artifact/environment checks, and separate pinned base-userland and derived runtime-image identities.
12. For the full-suite posture, reconcile its distinct digest and manifest; prove the 17-tool default-disabled, four isolated 18-tool, and 21-tool fully enabled truth table; and complete the selected battery, volume status, volume control, or fixed diagnostic call in every isolated posture. Dispatch all four filesystem mutations while disabled and prove source, target, destination, and quarantine state remain unchanged. Live filesystem and volume mutations must remain separately default-disabled and require their own exact-operation grants.

All postures must satisfy the same artifact-integrity requirements. The `mcp-runtime`, battery, volume-status, volume-control, command, and full-suite artifacts expose MCP authentication and transport; all must preserve Host/Origin validation, request limits, safe-root controls, and audit privacy. Each least-privilege optional artifact adds only its documented bounded tool. The full suite composes those tools without adding authority or collapsing their gates.

After downloading the default, `mcp-runtime`, `android-volume-control`, and `full-suite` exact-head artifacts, validate them through [`RELEASE_CANDIDATE_VALIDATION.md`](RELEASE_CANDIDATE_VALIDATION.md). Canonical validator v11 requires the expected commit, version, workflow run IDs, artifact/manifest paths, and SHA-256 digests; reconciles all four manifests; and emits sanitized direct evidence schema v2 without retaining artifact paths, tokens, session IDs, bodies, or file contents.

The Android workflow validates the battery, volume-status, volume-control, command, and full-suite contracts inside the digest-pinned official Termux userland on native ARM64. Aggregate evidence schema/gate v4 binds the full-suite binary and manifest digests to its 17/18/21 runtime truth table and explicit covered/not-covered claims. Android freezes and uploads seven component reports plus the content-addressed runtime archive, exact package lock, and snapshot, but cannot grant release eligibility. A separate first-attempt post-run qualifier revalidates the exact Android, CI, Security, and artifact identities, performs the no-network replay, and emits one twelve-member qualification artifact: seven components, four retained-runtime members, and the `official_termux_native_automated_v1` envelope.

Automated release qualification proves the exact artifacts under the digest-pinned official Termux userland on native ARM64, including deterministic Android-provider simulation and isolated deployment recovery. It does not certify physical-device, OEM, battery-aging, thermal-soak, radio, Doze, or Android-framework behavior. It records `physicalDeviceObserved:false`, `androidFrameworkObserved:false`, `sustainedPhysicalSoak:false`, `physicalCertification:"not_run"`, and `rebuildReproducibilityClaim:false`; the retained runtime does not claim future rebuild reproducibility.
