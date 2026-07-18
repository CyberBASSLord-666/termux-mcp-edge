# Native ARM64 Termux Emulated Release Gate

## Purpose

The Android workflow executes all exact downloaded release-candidate postures in the official [`termux/termux-docker`](https://github.com/termux/termux-docker) environment on a native GitHub-hosted ARM64 runner. This closes the gap between cross-compilation and executable Android/Termux behavior without asking an operator to repeat long idle observation windows during ordinary feature development.

The gate uses the immutable image reference:

```text
termux/termux-docker:aarch64@sha256:926e5c08aebc6df89f1cb3d9558c3b56b6246e59305fcd707bdf68f2584493b3
```

The image supplies the Termux private-directory layout, Bionic runtime, Android linker, and package environment. The job itself runs on `ubuntu-24.04-arm`; it does not rely on x86 binary translation in CI.

## Exact-artifact coverage

The emulated job starts only after the default, `mcp-runtime`, `android-battery-status`, `android-volume-status`, `android-volume-control`, and `command-execution` Android build postures complete. It downloads the bundles produced by the same workflow run and verifies:

- exact three-file bundle layout;
- checksum sidecars;
- repository, commit, workflow-run, version, posture, feature, target, digest, size, and ELF manifest fields;
- AArch64 Android executable identity and embedded version;
- default posture readiness and absence of `/mcp`;
- `mcp-runtime` authentication, Host/Origin ordering, initialization, notification semantics, protocol/session headers, exact tool allowlist, representative allowed and denied calls, safe-root confinement, default-disabled directory mutation plus exact-binary offline issuance, missing/binding/replay denial, dry-run non-consumption, fixed-mode/no-replace directory creation and binary file copy, content-free descriptor-relative path metadata, bounded literal text-search execution, symlink/oversized/existing-destination denial, request/response bounds, and session deletion;
- 256 additional high-frequency native samples covering stable PID, health, readiness, tool discovery, disabled high-impact gates, and complete session lifecycle.
- the battery artifact's exact manifest/digest/version/ELF posture, disabled-default discovery, enabled fixed-program invocation, zero arguments, cleared inherited environment, normalized field allowlist, sensitive-field redaction, prompt endless stdout/stderr rejection, isolated process-group termination, stdout/stderr pipe-holding descendant cleanup, caller-cancellation cleanup, stable API failure, audit-visible gate state, and continued absence of Android control, command execution, and high-impact tools.
- the volume artifact's exact manifest/digest/version/ELF posture, disabled-default discovery, fixed zero-argument `termux-volume` invocation, cleared inherited environment, exact six-stream parsing, canonical ordering, unknown-field rejection without reflection, prompt endless stdout/stderr rejection, the shared supervisor's process-group/pipe-holder/caller-cancellation cleanup, stable API failure, audit-visible gate state, and continued absence of volume mutation, Android control, command execution, and high-impact tools.
- the volume-control artifact's exact posture, incompatible-build rejection, disabled/enabled discovery and runtime truth, exact schema, preview non-consumption, grant binding/replay/header context, fixed two-argument execution, fresh maximum, verified mutation, rollback confirmed/unconfirmed, non-queueing concurrency, cancellation-independent recovery, supervisor cleanup, and private aggregate audits.
- the command artifact's exact manifest/digest/version/ELF posture plus the exact default artifact, default-build compile rejection, command-build runtime-disabled hiding, enabled exact profile/schema discovery, fixed current-executable argv, safe-root cwd, empty environment, null stdin, bounded UTF-8 responses, rejected override fields and unknown profiles, stable audit counters, and continued absence of arbitrary commands, Android control, and high-impact tools.

The canonical runtime validator remains authoritative for detailed protocol checks. The baseline wrapper emits a sanitized report conforming to [`emulated-release-evidence-schema-v1.json`](emulated-release-evidence-schema-v1.json), the battery wrapper emits [`android-battery-emulated-evidence-schema-v2.json`](android-battery-emulated-evidence-schema-v2.json), the volume wrapper emits [`android-volume-emulated-evidence-schema-v1.json`](android-volume-emulated-evidence-schema-v1.json), and the command wrapper emits [`command-emulated-evidence-schema-v1.json`](command-emulated-evidence-schema-v1.json). Battery schema v2 adds explicit process-supervisor and cancellation attestations; battery v1 is retained for historical evidence only. The command gate is deterministic and explicitly records that no long observation is required for its boundary validation. No feature-emulation report sets the canonical validator's direct-observation `releaseEligible` field.

## Development qualification versus release observation

Native Termux emulation is a required CI check for runtime-changing pull requests. A runtime change is not a CI failure merely because an older physical-device observation cannot be reused. After emulation passes, [`classify_observation_requirement.sh`](../scripts/classify_observation_requirement.sh) emits a sanitized report conforming to [`release-observation-requirement-schema-v1.json`](release-observation-requirement-schema-v1.json):

- `observation_inheritance_candidate` means protected runtime/deployment and normalized Cargo inputs still match the physical source. The separate inheritance verifier must still prove ancestry, exact artifact digests, and every other release condition before release eligibility can become true.
- `physical_observation_required` means runtime/deployment or Cargo/dependency inputs changed. Development CI passes when the exact artifacts and all automated gates pass, but `releaseQualificationEligible` remains false and a later release process must obtain new direct physical evidence.

The classifier itself always sets `releaseQualificationEligible:false`. It cannot waive, synthesize, or inherit a physical result.

## Physical-observation inheritance

An emulator cannot establish battery, thermal, OEM process-management, device audio-policy, or mobile-radio behavior. Those properties may be inherited from an already completed physical observation only when [`verify_observation_inheritance.sh`](../scripts/verify_observation_inheritance.sh) proves every condition below:

1. The source report is sanitized, schema-valid, non-fixture, fully passing, physically observed for at least 60 minutes, and identified by an expected SHA-256 digest.
2. The source commit is an ancestor of a previously qualified bridge commit, and the candidate is a descendant of that bridge.
3. `src/**`, `build.rs`, `.cargo/**`, the Rust toolchain pin, cross-compilation script, artifact packager, deployment manager, device gate, canonical validator, and direct-evidence schema are unchanged from the physically observed source.
4. `Cargo.toml` and `Cargo.lock` are structurally identical after removing only the root package version.
5. Exact candidate default and `mcp-runtime` binary digests match the independently qualified bridge digests.
6. Exact-head CI and Security pass, every candidate Android build posture passes, the default and `mcp-runtime` digests match the qualified bridge, and the native ARM64 official-Termux emulation report passes.

The verifier emits a sanitized report conforming to [`release-observation-inheritance-schema-v1.json`](release-observation-inheritance-schema-v1.json). `releaseQualificationEligible: true` means the combined direct physical source evidence and exact candidate evidence satisfy this narrow inheritance route. It is review evidence, not permission to tag or publish.

## Stop conditions

Observation inheritance is forbidden when any of these changes:

- runtime source or enabled feature surface;
- any dependency, dependency feature, build profile, or Rust toolchain;
- authentication, Host/Origin, request/session/resource, filesystem, audit, or shutdown behavior;
- deployment, service supervision, configuration parsing, upgrade, recovery, rollback, or uninstall logic;
- either exact bridge artifact digest;
- the required Termux image digest or native ARM64 execution posture.

Such a candidate requires a new direct physical observation before release qualification, but does not require a user to run a 60-minute observation merely to validate or merge a development PR. A failed or missing emulator result still fails CI and blocks both classification routes.

## v0.6.0 inheritance source

The committed source report [`release-evidence/v0.5.1-physical-fe5f7b80.json`](release-evidence/v0.5.1-physical-fe5f7b80.json) records the completed Galaxy/Termux physical qualification at `fe5f7b80a8ff13c2e39951d16f37b2e37a10a36b`. Its SHA-256 digest is:

```text
677796015065eb193ac78b2dd200de64efccb95a226837a4545c85021cb9283c
```

The v0.6.0 bridge commit is `a97e7cf2734ca3c997abc4e7d2aebaaa9fa856b9`, with independently downloaded bridge digests:

- default: `8fb1e89d942e5f925359eb22ea3321d6025baa83ee1da60fe58f1c62fe60dce1`;
- `mcp-runtime`: `e4c68590c02c2861b18392d7fae2b7542f6610e4a52615aee76f66c06cc7358e`.

The final v0.6.0 commit may use this route only if its rebuilt binaries retain those exact digests and the verifier passes without an exception or waived check.
