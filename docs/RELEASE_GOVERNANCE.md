# Release and artifact governance

This document defines the authoritative release process for `termux-mcp-edge`. It is intentionally conservative because the project targets long-running Android/Termux deployments where a bad binary, ambiguous feature posture, or unrecoverable upgrade can strand the service.

## Authority and scope

`main` is the only release source of truth. A GitHub Release, tag, checksum, Android artifact, install instruction, or rollback instruction is authoritative only when it identifies the exact same commit from `main` and all required validation evidence exists for that commit.

Historical branches, workflow artifacts, local binaries, and pull-request artifacts are development evidence only. They must never be presented as production releases.

The repository uses one active implementation lane at a time. Release preparation must not create a competing feature branch while another implementation pull request is open.

## Version and tag contract

The package version in `Cargo.toml`, the current release entry in `CHANGELOG.md`, the Git tag, and the GitHub Release title must agree.

Release tags use `vMAJOR.MINOR.PATCH`, for example `v0.6.0`.

A release tag must:

- point directly to the validated `main` commit;
- be annotated or signed by an authorized maintainer;
- never be moved, recreated, or force-updated after publication;
- never be created from a pull-request head, merge queue ref, local-only commit, or stale branch;
- contain no build metadata or feature name that obscures the package version.

If an incorrect public tag or release exists, preserve the historical record and publish a corrected version. Do not rewrite released history.

## Required exact-head evidence

Before tagging or publishing, record the exact 40-character commit SHA and verify all required jobs against that SHA.

Required evidence:

1. The standard CI workflow succeeds, including formatting, Clippy, the complete test suite, and deployment shell tests.
2. The security workflow succeeds with no unresolved actionable advisory or policy failure.
3. Android AArch64 validation succeeds for both supported feature postures:
   - default feature set;
   - `mcp-runtime`.
4. Each Android job uploads the expected posture-specific binary artifact.
5. Artifact checksums are generated from the downloaded release candidates, not from unrelated local builds.
6. The release candidate version reported by the binary matches `Cargo.toml`.
7. Installation, upgrade, rollback, service restart, and operator smoke-test instructions are current for the candidate.
8. No unresolved blocking review thread, open merge-conflict state, or failed newer run exists for the same commit.

A successful run on an older SHA is not transferable. A rerun is acceptable only when GitHub still identifies the same exact commit and the failure was transient rather than code-dependent.

## Supported release artifacts

A release must publish clearly named artifacts for each supported Android posture:

- `termux-mcp-server-vMAJOR.MINOR.PATCH-aarch64-linux-android-default`
- `termux-mcp-server-vMAJOR.MINOR.PATCH-aarch64-linux-android-mcp-runtime`

Each binary must be accompanied by a SHA-256 checksum file. A combined `SHA256SUMS` file is recommended and must use exact artifact filenames.

Artifacts must not contain bearer tokens, environment files, safe-root paths from a maintainer machine, private keys, tunnel credentials, logs, or other deployment-specific state.

Workflow-retained artifacts are validation evidence and may expire. GitHub Release assets are the durable distribution channel. Documentation must not instruct operators to depend on an expiring workflow artifact.

## Reproducibility record

Every GitHub Release body must record:

- exact source commit SHA;
- Rust toolchain version;
- Android target triple;
- Android NDK version;
- feature posture for each artifact;
- exact CI, security, and Android workflow run links or run identifiers;
- SHA-256 checksums;
- known limitations and intentionally disabled high-impact capabilities;
- upgrade and rollback references.

The release process should be reproducible from the tagged source using the documented toolchain. A byte-for-byte reproducible build is preferred but is not claimed unless independently verified.

## Release procedure

1. Confirm there is no active competing implementation pull request.
2. Update `Cargo.toml` and `CHANGELOG.md` together in a focused release-preparation pull request when a version change is required.
3. Confirm README, security, operations, deployment, validation, and rollback documentation describe the actual candidate behavior.
4. Merge only through the normal protected-main process with expected-head SHA validation.
5. Wait for post-merge CI, security, and Android validation on the resulting `main` SHA.
6. Download both posture-specific Android bundles and verify artifact names, manifests, checksum sidecars, executable identity, size, and SHA-256 checksums.
7. Run the downloaded artifacts through the complete validator in [`RELEASE_CANDIDATE_VALIDATION.md`](RELEASE_CANDIDATE_VALIDATION.md), retain its schema-versioned sanitized JSON evidence, and run the source-build/device gate in [`DEVICE_PRODUCTION_GATE.md`](DEVICE_PRODUCTION_GATE.md).
8. Create the annotated or signed `vMAJOR.MINOR.PATCH` tag at the validated `main` SHA.
9. Publish the GitHub Release from that immutable tag and attach both binaries, manifests, and checksum sidecars.
10. Re-open the release page and independently verify every asset, checksum, link, version, and recorded SHA.

Do not publish a draft as final until every required artifact is attached and verified.

The downloaded-artifact report's `releaseEligible` field must be true before publication. That requires non-fixture preflight, runtime, and deployment phases plus an operator-supplied passing sustained observation of at least 60 minutes. The report is review evidence, not an automated authorization to tag or publish.

## Installation, upgrade, and rollback guarantees

Release notes must distinguish:

- a clean installation;
- an in-place upgrade;
- rollback to the immediately previous validated release;
- configuration incompatibilities or migrations;
- service-name or runit-path changes;
- default versus `mcp-runtime` feature posture.

Never claim rollback is automatic, atomic, or complete unless the deployed tooling and tests prove that behavior for the release. A release that changes service supervision, environment parsing, filesystem layout, authentication, transport policy, or safe-root behavior must include an explicit compatibility and recovery note.

## Security release rules

Do not publish a release when:

- authentication, Host/Origin enforcement, request ceilings, safe-root jailing, audit privacy, or cancellation cleanup is known to be weakened;
- an applicable dependency advisory is unresolved without a documented, reviewed exception;
- a workflow action is mutable or not pinned according to repository policy;
- release assets were produced by an untrusted fork or write-enabled workflow without explicit review;
- checksums were generated before the final assets were fixed;
- a high-impact capability is enabled without the documented compile-time/runtime gate, allowlist, bounds, audit behavior, and tests.

For a confidential vulnerability, prepare the fix and release through an appropriately private process. Public release notes should describe impact and remediation without exposing secrets or exploit-enabling operational data.

## Branch retention and deletion policy

Branch deletion is a separate, explicitly approved maintenance action. Before proposing deletion, classify each non-default branch as:

- merged and fully represented in `main`;
- superseded by a named pull request or commit;
- unique unreviewed work requiring preservation;
- abandoned experiment with no production value;
- active implementation lane.

The deletion proposal must list every branch, its head SHA, merge/supersession evidence, and disposition. Never delete a branch containing unique security, recovery, deployment, or release work until those commits are reviewed and accounted for.

No force push, history rewrite, tag deletion, release deletion, or artifact deletion is authorized by this policy alone.

## Release-blocking inconsistencies

The following are hard blockers until reconciled:

- package version, changelog version, tag, and release title disagree;
- a tag does not point to the recorded validated `main` SHA;
- only one Android feature posture has passed or been published;
- documentation names an artifact that does not exist;
- release assets lack checksums;
- the newest exact-head workflow result is failed, cancelled for a code-dependent reason, or missing;
- installation or rollback steps reference obsolete service paths;
- a release candidate contains undocumented behavior changes.

## Current repository posture

The v0.6.0 release-preparation lane reconciles the source package, lockfile, changelog, deployment examples, artifact names, and candidate record without creating a tag or GitHub Release. The historical `v0.1.0-baseline` tag and the validated exact-main v0.5.1 candidate are not retroactively declared production releases. Consequently, v0.6.0 has no authoritative previous public release: clean installation and uninstall are supported, while public rollback becomes available only after a later complete release is installed over v0.6.0.

The pre-metadata v0.5.1 exact-main evidence authorizes preparation work but is not transferable to the changed v0.6.0 commit. Before publication, the final merged v0.6.0 `main` SHA must independently complete CI, Security, both Android postures, downloaded-bundle validation, and the required physical sustained observation. See [`V0.6.0_RELEASE_CANDIDATE.md`](V0.6.0_RELEASE_CANDIDATE.md).
