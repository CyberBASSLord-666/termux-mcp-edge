# Native ARM64 Termux Emulated Release Gate

## Purpose

The Android workflow executes all exact downloaded release-candidate postures in the official [`termux/termux-docker`](https://github.com/termux/termux-docker) environment on a native GitHub-hosted ARM64 runner. This closes the gap between cross-compilation and executable Android/Termux behavior without asking an operator to repeat long idle observation windows during ordinary feature development.

The gate uses the immutable image reference:

```text
termux/termux-docker:aarch64@sha256:926e5c08aebc6df89f1cb3d9558c3b56b6246e59305fcd707bdf68f2584493b3
```

The image supplies the Termux private-directory layout, Bionic runtime, Android linker, and package environment. The job itself runs on `ubuntu-24.04-arm`; it does not rely on x86 binary translation in CI.

## Exact-artifact coverage

The emulated job starts only after all seven governed Android postures complete: default, `mcp-runtime`, `android-battery-status`, `android-volume-status`, `android-volume-control`, `command-execution`, and `full-suite`. It downloads the bundles produced by the same workflow run and verifies:

- exact three-file bundle layout;
- checksum sidecars;
- repository, commit, workflow-run, version, posture, feature, target, digest, size, and ELF manifest fields;
- AArch64 Android executable identity and embedded version;
- default posture readiness and absence of `/mcp`;
- `mcp-runtime` authentication, Host/Origin ordering, initialization, notification semantics, protocol/session headers, the exact 17-tool allowlist, representative allowed and denied calls, successful safe-root startup and confined descendant operation from lifetime-pinned root identities, independently default-disabled directory/file-copy/file-trash/file-write mutation, exact-binary offline issuance against the same root-identity contract, missing/context/binding denial across those grants, create/copy/write replay denial, dry-run non-consumption, fixed-mode/no-replace directory creation and copy, exact source/destination/identity/digest copy binding, exact-target identity/content-bound reversible trash retention in a separate hidden mode-`0700` bounded quarantine, content/disposition-bound mode-`0600` file create/replace, exact copy/trash/write size and actual-ID response preflight, bounded descriptor-relative SHA-256 hashing, content-free descriptor-relative path metadata, bounded literal text-search execution, symlink/oversized/existing-destination denial, request/response bounds, and session deletion; trash replay and concurrent replay remain automated core/integration evidence rather than native-emulation claims;
- 256 additional high-frequency native samples covering stable PID, health, readiness, tool discovery, disabled high-impact gates, and complete session lifecycle.
- the battery artifact's exact manifest/digest/version/ELF posture, disabled-default discovery, enabled fixed-program invocation, zero arguments, cleared inherited environment, normalized field allowlist, sensitive-field redaction, prompt endless stdout/stderr rejection, isolated process-group termination, stdout/stderr pipe-holding descendant cleanup, caller-cancellation cleanup, stable API failure, audit-visible gate state, and continued absence of Android control, command execution, and high-impact tools.
- the volume artifact's exact manifest/digest/version/ELF posture, disabled-default discovery, fixed zero-argument `termux-volume` invocation, cleared inherited environment, exact six-stream parsing, canonical ordering, unknown-field rejection without reflection, prompt endless stdout/stderr rejection, the shared supervisor's process-group/pipe-holder/caller-cancellation cleanup, stable API failure, audit-visible gate state, and continued absence of volume mutation, Android control, command execution, and high-impact tools.
- the volume-control artifact's exact posture, incompatible-build rejection, disabled/enabled discovery and runtime truth, exact schema, preview non-consumption, grant binding/replay/header context, fixed two-argument execution, fresh maximum, verified mutation, rollback confirmed/unconfirmed, non-queueing concurrency, cancellation-independent recovery, supervisor cleanup, and private aggregate audits.
- the command artifact's exact manifest/digest/version/ELF posture plus the exact default artifact, default-build compile rejection, command-build runtime-disabled hiding, enabled exact profile/schema discovery, exact-name candidate-to-loaded-image device/inode attestation, `/proc/self/exe` spawning, descriptor-pinned non-root safe cwd, empty environment, null stdin, immutable 5-second/16 KiB/4 KiB maxima, rejected override fields and unknown profiles, stable audit counters, and continued absence of arbitrary commands, Android control, and high-impact tools. Its v3 runtime phase performs exactly 29 MCP requests, starts the server from `/`, replaces both executable and safe-root pathnames after initialization, and proves neither can redirect execution. A separate wrong-name phase requires typed construction failure before request serving and verifies that diagnostics disclose neither the bearer token nor filesystem paths.
- the full-suite artifact's distinct digest and manifest digest, exact `full-suite` posture/features, unchanged installed basename `termux-mcp-server`, exactly 17 tools with all optional runtime gates off, exactly 18 in each of four one-gate postures, and exactly 21 with all four enabled together. The aggregate probe completes the selected provider/profile call in every isolated posture, dispatches create/copy/trash/write while their mutation gates are disabled, verifies unchanged source/target/quarantine state, and proves that compile inclusion does not enable or couple constituent gates.

The canonical runtime validator remains authoritative for detailed protocol checks. The aggregate wrapper emits sanitized [`emulated-release-evidence-schema-v4.json`](emulated-release-evidence-schema-v4.json) evidence with `schemaVersion:4` and `gateVersion:"4"`. Schema v4 retains the lifetime-pinning, stress, exact full-suite identity, 17/18/21 truth table, and independent-gate proof from v3. It replaces the old release-policy assertion with an explicit claim boundary and closed covered/not-covered lists. The battery wrapper emits [`android-battery-emulated-evidence-schema-v3.json`](android-battery-emulated-evidence-schema-v3.json), the volume-status wrapper emits [`android-volume-emulated-evidence-schema-v2.json`](android-volume-emulated-evidence-schema-v2.json), the request-authorized volume-control wrapper emits [`android-volume-control-emulated-evidence-schema-v2.json`](android-volume-control-emulated-evidence-schema-v2.json), and the command wrapper emits [`command-emulated-evidence-schema-v3.json`](command-emulated-evidence-schema-v3.json). Older aggregate and specialized schemas remain immutable historical evidence and are not accepted as current qualification components.

Lifetime-pinning evidence is intentionally layered. Host regressions cover invalid-root rejection and perform root and ancestor rename/replacement, clone-sharing, redaction, and replacement-root grant-mismatch attacks. The exact native artifact gate independently exercises root and ancestor replacement plus successful startup, readiness, descriptor-relative calls, and grant flows in the Termux/Bionic environment.

## Automated release route

Native official-Termux execution is required for runtime-changing pull requests and exact-`main` candidates. Classifier v3 emits [`release-observation-requirement-schema-v3.json`](release-observation-requirement-schema-v3.json) with:

- `evidenceMode:"automated_release_qualification"`;
- `reasonCode:"automated_native_termux_evidence_required"`;
- `nextGate:"assemble_automated_release_qualification"`;
- `releaseQualificationEligible:false`; and
- the exact negative claim boundary.

The classifier binds the aggregate report, exact candidate identity, full-suite binary and manifest digests, workflow run IDs, image digest, and stress sample count. Runtime and dependency comparisons remain audit facts; they do not select a weaker evidence class. The classifier cannot grant release eligibility.

On a first-attempt exact-`main` push, Android runs the committed six-scenario isolated deployment/recovery gate and uploads seven frozen component reports without a release-eligibility conclusion. A separate read-only `workflow_run` qualifier revalidates the exact Android, CI, Security, artifact, and current-main identities before invoking `scripts/package_automated_qualification.sh`. Only that post-run assembler can emit `automated-qualification-v1.json` with `releaseEligible:true`.

Automated release qualification proves the exact artifacts under the digest-pinned official Termux userland on native ARM64, including deterministic Android-provider simulation and isolated deployment recovery. It does not certify physical-device, OEM, battery-aging, thermal-soak, radio, Doze, or Android-framework behavior. See [`AUTOMATED_RELEASE_QUALIFICATION.md`](AUTOMATED_RELEASE_QUALIFICATION.md).

## Historical physical evidence

The physical validator, device harness, observation-inheritance verifier, and their versioned schemas remain available for optional physical certification and historical audit. They are not accepted as `official_termux_native_automated_v1`, cannot be substituted into automated staging, and cannot add physical claims to an automated record.

The committed v0.5.1 report and bridge digests remain historical evidence for the surface they observed. No current ordinary release depends on inheriting or recreating its operator-entered observation duration.
