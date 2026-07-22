# Public release staging and publication

This guide separates six things that are easy to confuse:

1. **Workflow bundles** are the seven three-file Android artifacts produced by one exact successful `Android Cross Compile` run. They are qualification inputs, not public downloads.
2. **Automated evidence** is the native ARM64 Termux evidence from that same run. It proves the exact downloaded workflow binaries behaved as specified, but it does not replace a real-device observation.
3. **Physical evidence** is validator-v11/schema-v2 evidence plus a harness-v11 report from the same immutable commit. The harness records a separate locked on-device build digest; it is not claimed byte-identical to the workflow-built full-suite binary.
4. **A release stage** is a deterministic tar containing byte-for-byte copies of the seven qualified workflow binaries under their final versioned names. It is still not a tag or GitHub Release.
5. **A draft Release** is a pre-created empty GitHub Release for one pre-existing protected annotated tag. Attaching verified assets does not make the draft an installation source or publication authority.
6. **A public Release** becomes the durable distribution channel only after independent byte verification, a separate protected final approval, publication, an `immutable: true` response, and successful public re-download proof.

## Current boundary

The staging and publication lanes are separate workflows with separate permissions and approvals. Staging has no `contents: write` permission and no tag, Release, package, deployment, or OIDC permission. A staged tar says `publicationState: "staged_not_released"` and `releaseEligible: false` even though the physical validator report inside it must independently say `releaseEligible: true`. Publication can consume that exact tar but cannot rebuild a candidate, restage different bytes, or change the staging record.

The repository is public, and GitHub Actions artifacts are available to signed-in people with repository read access. The staged tar and both staging dispatch files are therefore **not confidential storage**. Draft assets are access-restricted while the Release is a draft, but they must still be treated as non-confidential because the same release-intended bytes already exist in the public-repository staging artifact. Published Release assets are public. Never put the raw harness report, credentials, personal data, private device paths, or other secrets in a workflow input, stage, draft, or public asset.

No operator should install from a workflow bundle, staged tar, or draft Release as though it were a durable public release. Public installation begins only after the protected final job publishes the independently verified draft, GitHub reports `immutable: true`, and the public proof job re-downloads all sixteen governed assets and verifies their exact bytes. Before publication the annotated tag is protected but is not yet made immutable by GitHub's immutable-release control.

## One-time staging administrator setup

Before running the staging workflow, an administrator must create the `release-qualification` environment in repository settings and configure all of the following:

- at least one trusted required reviewer who is not the workflow initiator;
- prevent self-review;
- `main` as the only allowed branch;
- administrator bypass disabled;
- no environment secrets;
- environment variable `RELEASE_QUALIFICATION_PROTECTED=required-reviewer-main-only-v1`.

The organization and repository variable scopes must **not** define `RELEASE_QUALIFICATION_PROTECTED`; only the `release-qualification` environment may define it. GitHub falls back to broader-scope variables when an environment-scoped variable is absent, so a same-named organization or repository variable would defeat the missing-environment guard.

Do this before the workflow is ever dispatched. GitHub otherwise creates a referenced missing environment without protection rules. The owner must confirm the actual eligible-reviewer list in repository Settings. For this personal-account repository, add at least one second trusted collaborator who can review the initiator; a team becomes an option only if the repository is later transferred to an organization. Do not weaken prevent-self-review to work around this prerequisite.

The checked-in workflow cannot inspect or create these repository settings. A reviewer must verify them in the GitHub UI before approving a job.

## One-time publication administrator setup

Publication requires two additional pre-created environments with disjoint eligible-reviewer sets:

| Environment | Purpose | Required environment-only guard | Required environment-only policy credential |
| --- | --- | --- | --- |
| `release-production` | Attach the fixed asset set to the exact empty draft | `RELEASE_PRODUCTION_PROTECTED=asset-attachment-reviewer-main-only-v1` | `RELEASE_PRODUCTION_POLICY_READ_TOKEN` |
| `release-final` | Reverify and publish the already verified draft | `RELEASE_FINAL_PROTECTED=final-publication-reviewer-main-only-immutable-v1` | `RELEASE_FINAL_POLICY_READ_TOKEN` |

Each environment must require a trusted reviewer who is not the workflow initiator, prevent self-review, allow only `main`, disable administrator bypass, and expose only its own guard and policy credential. The two eligible-reviewer sets must be disjoint; a person eligible to approve asset attachment must not be eligible to approve final publication. For this personal-account repository, that requires the owner plus at least two additional trusted collaborators. If those distinct reviewers are unavailable, publication stops rather than weakening either boundary.

Organization and repository variable scopes must not define `RELEASE_PRODUCTION_PROTECTED` or `RELEASE_FINAL_PROTECTED`; each guard exists only in its named environment. Organization, repository, and other environment secret scopes must not define either policy-token name. `RELEASE_PRODUCTION_POLICY_READ_TOKEN` and `RELEASE_FINAL_POLICY_READ_TOKEN` must be separate fine-grained credentials limited to this repository's **Administration: read** permission. They are used only for a bounded authenticated `GET` of the immutable-releases policy; they have no Contents, Actions, Workflows, Packages, Deployments, or identity-token write authority and are never used to create, update, upload, publish, or delete a Release.

An administrator must also complete and independently review all of the following before the publication workflow is dispatched:

- enable immutable releases for this repository; the setting applies only to future publications;
- create an active tag ruleset for `v*` that blocks update, force-update, and deletion, restricts creation to authorized maintainers, and gives GitHub Actions no bypass;
- have an authorized maintainer create the exact `vMAJOR.MINOR.PATCH` **annotated tag** at the qualified commit outside the workflow; and
- create one empty draft GitHub Release for that existing tag with the exact version title, a blank body, `draft: true`, `prerelease: false`, and zero assets.

The publication workflow never creates a tag or Release. This prevents the Releases API from implicitly manufacturing a lightweight tag when a requested tag is missing. The attach and final jobs independently use their environment-scoped Administration-read credentials to require the immutable-releases policy to be enabled before either write boundary proceeds. The ordinary `GITHUB_TOKEN` receives `contents: write` only inside those two protected jobs.

## Qualification inputs

Use one first-attempt successful `Android Cross Compile` push run from the exact current `main` commit. Its artifacts are retained for 30 days and must consist of exactly:

- the seven governed posture bundles listed in [Android validation artifacts](ANDROID_ARTIFACTS.md); and
- `termux-mcp-emulated-evidence`.

The workflow rejects expired, missing, duplicate, extra, pull-request, tag, fork, stale, rerun, incomplete, or failed inputs. It revalidates the current `main` ref, source SHA, run identity, manifest, checksum, size, target, ELF identity, feature posture, aggregate evidence, and companion CI/Security runs before and after environment approval.

Run the downloaded-artifact validator and physical device harness exactly as described in [release-candidate validation](RELEASE_CANDIDATE_VALIDATION.md) and the [device production gate](DEVICE_PRODUCTION_GATE.md). Then use `scripts/package_physical_qualification.sh --help` to create the [`physical-qualification-v1.json` closed-schema envelope](release-physical-qualification-schema-v1.json). The packager:

- requires validator v11, schema v2, non-fixture status, every phase passing, AArch64, at least 60 stable minutes, and `releaseEligible: true`;
- requires the harness-v11 final PASS and cleanup contract;
- binds the CI, Security, and Android run IDs;
- records SHA-256 for the sanitized validator report and private raw harness report;
- records the workflow full-suite digest and separate native-device full-suite digest without equating them; and
- emits no device paths, identifiers, command output, bearer material, or raw harness content.

Keep the raw harness report private. The reviewer compares the private report to the envelope's digest out of band:

```bash
test "$(sha256sum /absolute/path/device-harness-v11.txt | awk '{print $1}')" = \
  "$(jq -r .rawHarnessReportSha256 /absolute/path/dispatch-v1/physical-qualification-v1.json)"
```

The packager requires canonical absolute mode-`0600` report paths and a new absolute output directory beneath an existing mode-`0700` parent. A typical device-side preparation is:

```bash
EVIDENCE_PARENT="$HOME/.local/share/termux-mcp-release-evidence"
install -d -m 700 "$EVIDENCE_PARENT"
chmod 600 /absolute/path/release-validator-v11.json /absolute/path/device-harness-v11.txt
scripts/package_physical_qualification.sh \
  --validator-report /absolute/path/release-validator-v11.json \
  --harness-report /absolute/path/device-harness-v11.txt \
  --output-dir "$EVIDENCE_PARENT/dispatch-v1"
```

On failure the command removes only its private unpublished staging directory. On success the output contains exactly the unchanged validator report and the sanitized envelope; it never copies the raw harness report.

## Dispatch bundle

Create a gzip-compressed tar containing exactly these two regular files at its root:

- `release-validator-v11.json`
- `physical-qualification-v1.json`

Encode it as single-line base64 and record the SHA-256 of the compressed bytes. The encoded value must be at most 60,000 characters. Do not truncate or split an oversized bundle. The current lane cannot accept one; stop and add a separately reviewed intake path before staging.

```bash
PHYSICAL_ARCHIVE="$EVIDENCE_PARENT/physical-qualification-v1.tar.gz"
tar --format=gnu --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner \
  -C "$EVIDENCE_PARENT/dispatch-v1" \
  -cf - physical-qualification-v1.json release-validator-v11.json \
  | gzip -n -9 >"$PHYSICAL_ARCHIVE"
PHYSICAL_BUNDLE_SHA256="$(sha256sum "$PHYSICAL_ARCHIVE" | awk '{print $1}')"
PHYSICAL_BUNDLE_GZIP_BASE64="$(base64 -w0 "$PHYSICAL_ARCHIVE")"
(( ${#PHYSICAL_BUNDLE_GZIP_BASE64} <= 60000 ))
```

Dispatch `Stage Release Assets` from `main` with:

- `expected_commit`: the lowercase 40-character exact current `main` SHA;
- `version`: the exact `Cargo.toml` version;
- `android_run_id`: the qualifying first-attempt Android run;
- `physical_bundle_sha256`: the compressed bundle digest; and
- `physical_bundle_gzip_base64`: the single-line encoded bundle.

The input is sanitized but the workflow still masks it and never prints it. Masking reduces accidental log disclosure; it does not make a workflow-dispatch input confidential. Preflight performs the complete verification without producing a retained artifact. The protected `stage` job repeats every check after approval, performs a final current-`main` check, and then uploads one raw deterministic tar.

## Staged payload

For v0.6.0 the tar is named `termux-mcp-server-v0.6.0-release-stage-<sha12>.tar`. It contains:

- seven byte-identical binaries under the final `termux-mcp-server-v0.6.0-aarch64-linux-android-<posture>` names;
- one checksum sidecar per binary and a combined `SHA256SUMS`;
- the unchanged workflow manifests under unambiguous names;
- sanitized validator, native-emulation, classifier, and physical-qualification evidence;
- `LICENSE`; and
- [`release-staging-manifest-v1.json`](release-staging-manifest-schema-v1.json), validated by its closed schema.

The staging manifest binds the exact source and workflow run IDs, every source and staged digest, every preserved manifest/evidence digest, and the deterministic member inventory. Renaming never changes binary bytes. Any assembler mismatch before upload leaves no local staging tar. The final step also requires the raw-upload server digest to equal the locally computed tar digest. The staged Actions artifact is retained for 30 days. Because this read-only workflow intentionally cannot delete Actions artifacts, a failure after upload can leave an **unqualified** artifact until an administrator deletes it or retention expires; only a successful workflow summary with matching IDs and digests identifies a qualified stage.

## Fixed public asset set

The v0.6.0 draft must begin empty and, after the protected attachment job, contain exactly sixteen assets:

1. the seven versioned Android binaries listed in [Android validation artifacts](ANDROID_ARTIFACTS.md);
2. the seven matching `<binary-name>.sha256` sidecars;
3. `SHA256SUMS`; and
4. the unchanged raw `termux-mcp-server-v0.6.0-release-stage-<sha12>.tar` downloaded from the exact staging Actions artifact.

The first fifteen files are byte-for-byte members extracted from that tar. The sixteenth is the tar itself, byte-for-byte unchanged. The closed `release-staging-manifest-v1.json`, workflow manifests, LICENSE, and sanitized evidence remain inside the raw tar and are not separate GitHub Release assets. A different count, filename, byte length, digest, upload state, or duplicate name fails the draft.

`scripts/prepare_release_publication_assets.sh` validates this projection and emits a private `release-publication-receipt-v1.json` for workflow comparison. The receipt is verification state, not a seventeenth Release asset, and must not be uploaded.

GitHub automatically offers tag-derived source ZIP and tar archives. Those generated downloads are not members of the sixteen-asset contract, are not Android binaries, and are not covered by the publisher's asset digest or `verify-asset` proof. The v0.6.0 contract also makes no separate SBOM or third-party-notice asset claim. Adding any new durable asset requires a separately reviewed staging and publication contract change; it must not be appended during a live release.

## Publication dispatch

Dispatch `Publish Immutable Release` from [`.github/workflows/publish-release.yml`](../.github/workflows/publish-release.yml) only from `main`. Supply the exact current-main commit, package version, annotated tag-object SHA, staging artifact ID, raw staging-tar SHA-256, and pre-created empty draft Release ID. The workflow derives the exact `v<version>` tag name and resolves the artifact's owning staging run from those identities. These identifiers and digests are non-secret. Do not pass a token, release body, binary, evidence document, path, or encoded archive through workflow inputs.

Preflight is read-only. It requires the dispatch workflow/ref/SHA and current `main` to identify the same commit; validates the version-derived protected annotated tag and supplied tag-object SHA; resolves the exact successful first-attempt staging run from the supplied artifact ID; requires the one named unexpired staging artifact by both ID and server digest; downloads that raw tar by ID with digest mismatch as an error; and validates the complete staging tar, manifest, provenance, evidence lineage, member allowlist, and fixed sixteen-asset projection. It also requires the supplied Release ID to be the one `draft: true`, `prerelease: false`, exact-tag draft with the exact version title, blank body, and zero assets. An existing published Release, another draft for the tag, a lightweight/moved tag, or any pre-existing asset is a hard failure.

## Publication state machine

The protected workflow has one-way states and does not skip or combine them:

1. **Public, non-confidential stage.** The staging workflow has already emitted one exact raw tar. It remains `staged_not_released` and is not an installation source.
2. **Pre-created empty draft.** An authorized maintainer creates the protected annotated tag and empty draft outside the workflow. Neither object alone authorizes installation.
3. **Protected attachment.** The `release-production` job waits for its environment reviewer, requires `RELEASE_PRODUCTION_PROTECTED=asset-attachment-reviewer-main-only-v1`, verifies the immutable-release policy with `RELEASE_PRODUCTION_POLICY_READ_TOKEN`, and repeats every source, tag, run, artifact, tar, manifest, draft, and zero-asset check. Only then does its job-local `contents: write` token bind the deterministic, provenance-derived release body and attach the fixed sixteen assets. It cannot create or publish a Release, create or move a tag, change the title or prerelease state, delete or replace an asset, or rebuild anything. It retains a closed attachment record with all sixteen server-assigned identities and presents the record digests in the job summary.
4. **Independent byte verification.** A fresh read-only job takes only the recorded draft Release ID and expected identities. It lists exactly sixteen uploaded assets, rejects `starter` or non-uploaded state, binds every asset ID/name/size/server digest, downloads every asset afresh by ID, and compares every byte to a separately downloaded exact staging tar. It re-runs `SHA256SUMS`, all seven sidecars, the closed staging-manifest checks, and the raw-tar digest. This job has no release-write permission. It retains the closed JSON verification record for 30 days and renders its run, Release, source, stage, record hashes, and sixteen asset identities in a reviewer-readable job summary.
5. **Separate final approval.** Only after independent verification and record retention succeed may the `release-final` job wait for its disjoint reviewer. After approval it downloads that exact current-run verification artifact by server-assigned ID, requires the recorded Actions digest and file SHA-256, and semantically reproduces the record from the current draft before and immediately before mutation. It also requires `RELEASE_FINAL_PROTECTED=final-publication-reviewer-main-only-immutable-v1`, verifies the immutable-release policy with `RELEASE_FINAL_POLICY_READ_TOKEN`, and repeats current-main, tag, stage, asset-ID, server-digest, fresh-download, checksum, and byte-equality checks. Its exact PATCH changes `draft` from `true` to `false`, reasserts the already-verified `prerelease: false` state, and explicitly requests this Release as latest; it makes no tag, asset, title, or body mutation.
6. **Immutable public proof.** Publication success is not the PATCH response alone. A fresh public read-back must report the exact Release ID/tag/commit, `draft: false`, `prerelease: false`, and `immutable: true`. A final read-only proof then downloads all sixteen assets through their public URLs without the policy credential, verifies the exact allowlist and every byte/digest against the retained identities, and records the immutable Release URL, identity, asset count, and successful public proof in its summary.

GitHub makes a tag and attached assets immutable only when an immutable Release is published. Before step 5 the ruleset protects the annotated tag, but documentation must not call it a GitHub-immutable tag. The environments provide separate approval checkpoints; the configured disjoint reviewer sets provide the human separation.

## Publication records

The Release body is bound before upload and contains only deterministic facts already available at that boundary: source and annotated-tag identity, staging/CI/Security/Android run identities, staging artifact and tar digest, toolchain/target/NDK versions, the expected sixteen names/sizes/SHA-256 values, operational limitations, and deployment/governance links. It intentionally does not claim server-assigned Release asset IDs, later approval identities, or a future immutable/public result.

The separate workflow record covers those later facts. The protected attachment and independent verification jobs each retain a closed JSON identity record for 30 days. Those records bind the publication workflow run, Release ID, stage identity, release-body/expected-asset-set digests, and every server-assigned asset ID, name, size, state, content type, API/download URL, and server SHA-256 digest. Their job summaries bind the record file SHA-256 and Actions artifact ID/server digest. The verification record's exact workflow-run link is the review context for the `release-production` and `release-final` protected-job decisions. Because both jobs intentionally set `deployment: false`, they create no GitHub Deployment record; the workflow therefore does not guess or copy reviewer identities into a pre-approval record. Use the linked run's environment-review UI, together with any applicable GitHub audit-log evidence, when human-review attribution must be preserved outside the retained artifacts. The final job must consume the exact retained verification record, and the post-verification summary records the immutable public Release identity after the public byte proof succeeds.

These workflow records contain only public release provenance, not credentials or private device data. They supplement the deterministic Release body; they are not seventeenth Release assets and are not described as permanent once their documented retention expires.

## Draft recovery and stop conditions

Creating a draft, attaching assets, and publishing are not one atomic operation. The workflow therefore fails closed as follows:

- an attachment error may leave a partial draft, but the workflow never auto-deletes a draft, asset, tag, or staging artifact;
- a verification mismatch leaves the draft unpublished and blocks the final job;
- a denied, rejected, or expired final approval leaves the verified draft unpublished;
- every GitHub workflow rerun is rejected by the first-attempt guard; after inspecting and explicitly retiring or cleaning any partial draft back to the documented empty state, an administrator must start a fresh reviewed dispatch; and
- an ambiguous publish response is resolved by reading the exact Release ID before any retry. If it is public, no publish mutation is repeated; only immutable/public proof continues.

Once GitHub reports the Release immutable, its tag and assets are not repaired, replaced, or deleted by automation. A post-publication proof failure is a release incident and requires a corrected later version while preserving the historical record. Neither a partial draft nor a fully verified draft may be described as public, immutable, installable, or released.

## Publication prerequisites

The checked-in workflow and documentation do not satisfy administrator-only or physical-device gates by themselves. A publication dispatch must remain blocked unless all of the following are true for one unchanged current-main commit:

- exact-main first-attempt CI, Security, Android/native validation, fresh validator-v11/schema-v2 physical qualification, and the protected stage all pass;
- the `release-qualification`, `release-production`, and `release-final` environments exist with their documented guards, branch rules, bypass posture, and reviewer separation;
- the two separate environment-only Administration-read policy credentials exist and immutable releases are enabled;
- the active `v*` tag ruleset exists and the exact annotated version tag is protected at the qualified commit;
- the exact-tag draft is pre-created with the exact version title, a blank body, and zero assets; and
- attachment, independent draft verification, separate final approval, publication, `immutable: true`, and public sixteen-asset re-download proof all complete without a waived assertion.

Only the immutable GitHub Release whose public sixteen-asset proof passed is installable. A tag, draft, workflow bundle, or stage is never a substitute.
