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
- be an annotated tag created by an authorized maintainer outside the publication workflow;
- be protected by an active `v*` tag ruleset that blocks update, force-update, and deletion and gives GitHub Actions no bypass;
- never be moved, recreated, or force-updated after publication;
- never be created from a pull-request head, merge queue ref, local-only commit, or stale branch;
- contain no build metadata or feature name that obscures the package version.

If an incorrect public tag or release exists, preserve the historical record and publish a corrected version. Do not rewrite released history.

## Required exact-head evidence

Before tagging or publishing, record the exact 40-character commit SHA and verify all required jobs against that SHA.

Required evidence:

1. The standard CI workflow succeeds, including locked dependency-graph preflight, unchanged `Cargo.toml`/`Cargo.lock` assertions around Cargo-aware setup, formatting, locked Clippy, the complete locked test suite, and deployment shell tests.
2. The security workflow succeeds with no unresolved actionable advisory or policy failure.
3. Android AArch64 validation succeeds for all supported feature postures:
   - default feature set;
   - `mcp-runtime`;
   - `android-battery-status`;
   - `android-volume-status`;
   - `android-volume-control`;
   - `command-execution`;
   - `full-suite`.
4. Each Android job uploads the expected posture-specific binary artifact.
5. Artifact checksums are generated from the downloaded release candidates, not from unrelated local builds.
6. The release candidate version reported by the binary matches `Cargo.toml`.
7. Installation, upgrade, rollback, service restart, and operator smoke-test instructions are current for the candidate; filesystem authorization changes include exact disabled/enabled gate, grant-binding, boundary, race, cleanup, and private-evidence coverage.
8. No unresolved blocking review thread, open merge-conflict state, or failed newer run exists for the same commit.

A successful run on an older SHA is not transferable. A same-SHA rerun may help diagnose a transient development or merge failure, but it never qualifies release staging or publication. Those lanes accept only first-attempt successful CI, Security, and Android push runs for the exact candidate.

## Supported release artifacts

A release must publish clearly named artifacts for each supported Android posture:

- `termux-mcp-server-vMAJOR.MINOR.PATCH-aarch64-linux-android-default`
- `termux-mcp-server-vMAJOR.MINOR.PATCH-aarch64-linux-android-mcp-runtime`
- `termux-mcp-server-vMAJOR.MINOR.PATCH-aarch64-linux-android-android-battery-status`
- `termux-mcp-server-vMAJOR.MINOR.PATCH-aarch64-linux-android-android-volume-status`
- `termux-mcp-server-vMAJOR.MINOR.PATCH-aarch64-linux-android-android-volume-control`
- `termux-mcp-server-vMAJOR.MINOR.PATCH-aarch64-linux-android-command-execution`
- `termux-mcp-server-vMAJOR.MINOR.PATCH-aarch64-linux-android-full-suite`

The durable v0.6.0 Release has a closed sixteen-asset inventory: these seven binaries, seven matching `<binary-name>.sha256` files, one combined `SHA256SUMS`, and the unchanged raw deterministic staging tar. The tar contains the closed staging manifest, workflow manifests, LICENSE, and sanitized evidence; those members are not uploaded as separate assets. No extra, duplicate, missing, `starter`, renamed, zero-byte, or unexpected-state asset is permitted.

Artifacts must not contain bearer tokens, environment files, safe-root paths from a maintainer machine, private keys, tunnel credentials, logs, or other deployment-specific state.

Workflow-retained artifacts are validation evidence and may expire. GitHub Release assets are the durable distribution channel. Documentation must not instruct operators to depend on an expiring workflow artifact.

The Android workflow retains the seven bundles and native-emulation evidence for 30 days and does not rebuild on a version-tag push. A release stage must copy the qualified exact-main bytes without modification; a tag-triggered or local rebuild is a different, unqualified candidate.

## Protected staging boundary

[`PUBLIC_RELEASE.md`](PUBLIC_RELEASE.md) defines the mandatory protected staging lane. Its manual workflow has only `actions: read` and `contents: read`, requires exact current-main source and first-attempt upstream run identity, consumes a sanitized physical-qualification envelope, and repeats every check after approval by the pre-created `release-qualification` environment. Its temporary Actions artifact is not confidential in this public repository.

The stage contains final filenames and checksums but is not public authority. Its closed manifest must say `publicationState: "staged_not_released"` and `releaseEligible: false`. No staging code may create a tag, draft Release, public Release, package, deployment, or rebuilt binary.

## Protected publication boundary

[`PUBLIC_RELEASE.md`](PUBLIC_RELEASE.md) also defines the separate publication lane. It consumes an exact staging artifact ID and raw-tar/server digest, resolves the owning staging run, requires a pre-existing protected annotated tag by its exact tag-object SHA, and accepts only a pre-created exact-tag draft with zero assets. The stage remains public-repository, non-confidential evidence; a draft is not an installation source or release authority.

Asset attachment and final publication are distinct protected write jobs. `release-production` permits only attachment of the fixed sixteen assets after its reviewer and guard approve. A fresh read-only job then downloads all sixteen assets by server-assigned ID, verifies them byte-for-byte against a separate exact-stage download, retains a closed identity record, and exposes a reviewer-readable summary. Only that passing record can unlock the disjoint `release-final` reviewer boundary. The final job downloads that exact same-run record, verifies its Actions and file digests, semantically reproduces it from current server state, and then sends the closed publication PATCH: `draft:false`, the already-required `prerelease:false`, and `make_latest:"true"`. It does not mutate the tag, title, body, or asset set.

Each write job has job-local `contents: write`; preflight, independent verification, and public proof remain read-only. The two protected environments use disjoint reviewers and distinct environment-only Administration-read credentials solely to verify that repository release immutability is enabled. The ordinary workflow token never receives Administration permission, and the policy credentials never upload, mutate, publish, or delete repository content.

GitHub release immutability begins at publication, not tag or draft creation. Before publication, the ruleset protects the annotated tag. After publication, the public read-back must say `immutable: true`, and an unauthenticated public proof must re-download the exact sixteen assets and reproduce every recorded digest. GitHub-generated source ZIP/tar archives are tag-derived convenience downloads, not governed assets or Android release binaries.

## Reproducibility and publication records

The deterministic GitHub Release body is bound before any asset upload and records only facts already available at that boundary:

- exact source commit SHA;
- Rust toolchain version;
- Android target triple;
- Android NDK version;
- feature posture for each artifact;
- exact CI, security, and Android workflow run links or run identifiers;
- exact staging workflow run ID, staging artifact ID, and staging artifact server digest;
- annotated tag object identity;
- each of the sixteen expected Release asset names, sizes, and SHA-256 digests;
- SHA-256 checksums;
- known limitations and intentionally disabled high-impact capabilities;
- upgrade and rollback references.

Server-assigned and post-upload facts belong to the separate workflow publication record, not to the pre-upload Release body. The attachment and independent-verification jobs retain closed JSON records for 30 days that identify the publication run, draft Release ID, stage, record hashes, and all sixteen server asset IDs/names/sizes/states/content types/URLs/digests. The independent-verification summary exposes those identities before the final environment review, and the final job must download and reproduce that exact record before publication. The exact linked workflow run is the review context for both protected-job decisions. Because the jobs intentionally set `deployment: false`, no GitHub Deployment history is created and the retained records do not claim reviewer identities; use the run's environment-review UI and any applicable GitHub audit-log evidence when human-review attribution must be archived. The final public-proof summary records the `immutable: true` response and successful public re-download. Values are recorded only after GitHub returns them; guessed IDs, URLs, checksums, approval identities, or immutable state are forbidden.

The release process should be reproducible from the tagged source using the documented toolchain. A byte-for-byte reproducible build is preferred but is not claimed unless independently verified.

Every product and Android artifact build must use the committed `Cargo.lock`; a stale or missing lock is a release failure, never an instruction to regenerate dependencies in place. The device gate's synthetic older-version rollback fixture may change only the root package-version field in both `Cargo.toml` and `Cargo.lock`, must build that graph with `--locked`, and must restore a clean exact-head tree before any candidate build.

## Release procedure

1. Confirm there is no active competing implementation pull request.
2. Update `Cargo.toml` and `CHANGELOG.md` together in a focused release-preparation pull request when a version change is required.
3. Confirm README, security, operations, deployment, validation, and rollback documentation describe the actual candidate behavior.
4. Merge only through the normal protected-main process with expected-head SHA validation.
5. Wait for post-merge CI, security, and Android validation on the resulting `main` SHA.
6. Download all seven posture-specific Android bundles and verify artifact names, manifests, checksum sidecars, executable identity, size, and SHA-256 checksums.
7. Run the default, `mcp-runtime`, `android-volume-control`, and `full-suite` bundles through validator v11 in [`RELEASE_CANDIDATE_VALIDATION.md`](RELEASE_CANDIDATE_VALIDATION.md). Retain direct schema-v2 evidence, aggregate schema/gate-v3 evidence that binds the exact workflow full-suite digest/manifest, 17/18/21 truth table, independent runtime gates, and all four disabled filesystem dispatches, plus every posture's exact-source native ARM64 official-Termux evidence. For v0.6.0, complete a fresh device-harness-v11 physical observation of the same immutable commit and retain its separately recorded on-device native-build digest; do not assert byte equality between different toolchain builds.
8. Package the sanitized physical-qualification envelope while retaining the raw harness report privately by its digest.
9. Run protected release staging from the exact qualifying Android run. Verify the deterministic tar, closed staging manifest, exact source-to-staged binary digest equality, and recorded Actions artifact ID/server digest.
10. Have an administrator verify repository release immutability, the active no-bypass `v*` tag ruleset, and the disjoint `release-production`/`release-final` environment protections and policy credentials.
11. Have an authorized maintainer create the annotated `vMAJOR.MINOR.PATCH` tag at the staged `main` SHA outside the workflow. At this point the tag is protected by the ruleset but is not yet made immutable by a published GitHub Release.
12. Pre-create one **empty draft** GitHub Release for that existing tag with the exact title and a blank body, `draft: true`, `prerelease: false`, and zero assets. Never allow the Release API to create a missing tag implicitly.
13. Dispatch the publisher with the exact source commit, version, annotated tag-object SHA, staging artifact ID/raw-tar digest, and draft Release ID. Read-only preflight must resolve the owning staging run and validate every identity plus the closed sixteen-asset projection.
14. After `release-production` approval, bind the deterministic provenance-derived release body and attach only the seven binaries, seven sidecars, `SHA256SUMS`, and unchanged raw staging tar. This job must not create or publish a Release, create or move a tag, change the title or prerelease state, or delete or replace any draft asset.
15. On a fresh read-only runner, re-download every draft asset by ID and independently verify the exact count, names, states, sizes, server digests, checksums, staging manifest, and byte equality against a separate exact-stage download.
16. Obtain the disjoint `release-final` approval only after step 15 retains its closed record and reviewer-readable summary. Reverify current main, tag, policy, draft, stage, and every asset after approval; download and semantically reproduce the exact same-run verification record; then publish the already verified draft with only `draft:false`, an idempotent `prerelease:false`, and the explicit `make_latest:"true"` request. Do not change its tag, title, body, or assets.
17. Require the public Release response to say `immutable: true`, then publicly re-download and hash all sixteen assets and record their immutable identities.

Do not publish a draft as final until every required artifact is attached and independently verified. A staged artifact, protected tag, empty or populated draft, physical report, or `releaseEligible: true` field is evidence only and never substitutes for the final approval and immutable public proof.

Attachment and publication are not an atomic transaction. A failed attachment or verification leaves the draft unpublished; the workflow never automatically deletes a draft, asset, tag, staging artifact, or public Release. Every GitHub workflow rerun is rejected by the first-attempt guard. An administrator must inspect and explicitly retire the partial candidate or return it to the documented empty-draft state, then start a fresh reviewed dispatch. If publication returns an ambiguous response, query the exact Release ID before retrying any mutation. Once a Release is public and immutable, a failed public proof is handled as a release incident and corrected with a later version rather than rewriting history.

Before publication, an applicable evidence route must pass:

- **Direct route:** the downloaded-artifact report's `releaseEligible` field is true after non-fixture preflight, runtime, deployment, and an operator-supplied passing physical observation of at least 60 minutes.
- **Inherited route:** an earlier direct physical report remains applicable only after the exact candidate passes native ARM64 official-Termux emulation and `verify_observation_inheritance.sh` proves unchanged runtime source, dependencies, build inputs except the root version, deployment logic, and exact bridge artifact digests. Its report must set `releaseQualificationEligible: true` without a waived assertion.

Both are review evidence, not automated authorization to tag or publish. An emulator alone never replaces device-specific battery, thermal, OEM process-management, or radio evidence.

The v0.6.0 full-suite change is not eligible for the inherited route: it adds a feature composition, a seventh artifact, a new aggregate runtime truth table, and a new digest identity that the historical v0.5.1 report did not observe. Its exact-main direct schema-v2 report must say `releaseEligible:true` after a fresh physical AArch64 observation.

## Installation, upgrade, and rollback guarantees

Release notes must distinguish:

- a clean installation;
- an in-place upgrade;
- rollback to the immediately previous validated release;
- configuration incompatibilities or migrations;
- service-name or runit-path changes;
- default, `mcp-runtime`, battery, volume-status, volume-control, fixed-command, and `full-suite` feature postures. Raw Cargo `--all-features` is development coverage, not a supported release asset.

Never claim rollback is automatic, atomic, or complete unless the deployed tooling and tests prove that behavior for the release. A release that changes service supervision, environment parsing, filesystem layout, authentication, transport policy, or safe-root behavior must include an explicit compatibility and recovery note.

## Security release rules

Do not publish a release when:

- authentication, Host/Origin enforcement, request ceilings, safe-root jailing, audit privacy, or cancellation cleanup is known to be weakened;
- an applicable dependency advisory is unresolved without a documented, reviewed exception;
- a workflow action is mutable or not pinned according to repository policy;
- release assets were produced by an untrusted fork or write-enabled workflow without explicit review;
- repository release immutability cannot be verified as enabled through the environment-only Administration-read policy credential;
- the protected tag is lightweight, moved, deletable, unruled, or was created by the publication workflow;
- the `release-production` and `release-final` environment reviewer sets overlap, either environment permits self-review or administrator bypass, or a broader-scope guard/policy credential can substitute for the environment value;
- the draft was not pre-created empty for the exact annotated tag, contains an extra asset, or is already public;
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
- any supported Android feature posture has not passed or has not been published;
- documentation names an artifact that does not exist;
- release assets lack checksums;
- the sixteen-asset allowlist, independent draft download, final revalidation, `immutable: true` response, or public re-download proof is missing or mismatched;
- the newest exact-head workflow result is failed, cancelled for a code-dependent reason, or missing;
- installation or rollback steps reference obsolete service paths;
- a release candidate contains undocumented behavior changes.

## v0.6.0 release authority

The v0.6.0 release-preparation lane reconciles the source package, lockfile, changelog, deployment examples, artifact names, candidate record, staging boundary, and protected publication state machine without itself creating a tag or GitHub Release. The historical `v0.1.0-baseline` tag and the validated exact-main v0.5.1 candidate are not retroactively declared production releases. Consequently, v0.6.0 has no authoritative previous public release: clean installation and uninstall are supported, while public rollback becomes available only after a later complete release is installed over v0.6.0.

The pre-metadata v0.5.1 exact-main evidence is historical and cannot qualify the full-suite v0.6.0 candidate. Authorization of a `v0.6.0` GitHub Release requires the final merged v0.6.0 `main` SHA to independently complete CI, Security, all seven Android postures, downloaded-bundle validation, aggregate schema/gate-v3 native ARM64 official-Termux evidence, a fresh harness-v11 physical observation whose validator-v11/schema-v2 report says `releaseEligible:true`, protected staging, the two disjoint publication approvals, independent draft verification, and immutable public sixteen-asset proof. See [`V0.6.0_RELEASE_CANDIDATE.md`](V0.6.0_RELEASE_CANDIDATE.md).
