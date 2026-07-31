# Public release staging and publication

This guide separates six things that are easy to confuse:

1. **Workflow bundles** are the seven three-file Android posture artifacts produced by one exact successful `Android Cross Compile` run. They are qualification inputs, not public downloads.
2. **Automated component and runtime evidence** are two additional Android artifacts: the seven-file aggregate/provider/classifier/deployment component set and the three-file retained-runtime snapshot containing the runtime archive, package lock, and snapshot record. No component grants release eligibility alone.
3. **Automated qualification** is one twelve-member artifact: the seven unchanged components, four retained-runtime records (archive, package lock, snapshot, and offline replay), and the closed `official_termux_native_automated_v1` envelope emitted only by the separate first-attempt post-run qualifier.
4. **A release stage** is a deterministic tar containing byte-for-byte copies of the seven qualified workflow binaries plus the governed runtime evidence under `evidence/runtime/`. It is still not a tag or GitHub Release.
5. **A draft Release** is a pre-created empty GitHub Release for one pre-existing protected annotated tag. Attaching verified assets does not make the draft an installation source or publication authority.
6. **A public Release** becomes the durable distribution channel only after independent byte verification, a separate protected final approval, publication, an `immutable: true` response, and successful public re-download proof.

## Current boundary

The staging and publication lanes are separate workflows with separate permissions and approvals. Staging has no `contents: write` permission and no tag, Release, package, deployment, or OIDC permission. A staged tar says `publicationState: "staged_not_released"` and `releaseEligible: false` even though the automated qualification inside it independently says `releaseEligible:true`. Publication can consume that exact tar but cannot rebuild a candidate, restage different bytes, or change the staging record.

The repository is public, and GitHub Actions artifacts are available to signed-in people with repository read access. The staged tar and the three non-secret scalar staging inputs are therefore **not confidential storage**. Draft assets are access-restricted while the Release is a draft, but they must still be treated as non-confidential because the same release-intended bytes already exist in the public-repository staging artifact. Published Release assets are public. Never put the raw harness report, credentials, personal data, private device paths, or other secrets in a workflow input, stage, draft, or public asset.

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
| `release-final` | Reverify and publish the already verified draft | `RELEASE_FINAL_PROTECTED=final-publication-reviewer-main-only-immutable-v1` and `RELEASE_FINAL_EXCLUSIVE_MUTATION_FREEZE=exclusive-release-main-policy-tag-writers-paused-v1` | `RELEASE_FINAL_POLICY_READ_TOKEN` |

Each environment must require a trusted reviewer who is not the workflow initiator, prevent self-review, allow only `main`, disable administrator bypass, and expose only its own guard and policy credential. The two eligible-reviewer sets must be disjoint; a person eligible to approve asset attachment must not be eligible to approve final publication. For this personal-account repository, that requires the owner plus at least two additional trusted collaborators. If those distinct reviewers are unavailable, publication stops rather than weakening either boundary.

Organization and repository variable scopes must not define `RELEASE_PRODUCTION_PROTECTED`, `RELEASE_FINAL_PROTECTED`, or `RELEASE_FINAL_EXCLUSIVE_MUTATION_FREEZE`; each guard exists only in its named environment. Organization, repository, and other environment secret scopes must not define either policy-token name. `RELEASE_PRODUCTION_POLICY_READ_TOKEN` and `RELEASE_FINAL_POLICY_READ_TOKEN` must be separate fine-grained credentials limited to this repository's **Administration: read** permission. They are used only for a bounded authenticated `GET` of the immutable-releases policy; they have no Contents, Actions, Workflows, Packages, Deployments, or identity-token write authority and are never used to create, update, upload, publish, or delete a Release.

GitHub's Release update API has no compare-and-swap precondition spanning the final asset listing and the irreversible publish PATCH. Final approval therefore asserts an exclusive mutation freeze: from approval until the PATCH response is resolved, every other human, app, token, and workflow with Release-write authority is paused; repository-settings writers may not change immutable-release policy; tag and ruleset writers may not move, delete, recreate, or weaken protection for the candidate tag; `main` is not advanced; and no CI, Security, Android, qualification, staging, or publication rerun is started. Repository workflows enforce that only this publication workflow declares `contents: write`; administrators must also suspend or exclude external contents, settings, ruleset, and tag writers for this bounded window. The workflow's repeated API reads are latest-observed checks, not an atomic repository-wide lock.

An administrator must also complete and independently review all of the following before the publication workflow is dispatched:

- enable immutable releases for this repository; the setting applies only to future publications;
- create an active tag ruleset for `v*` that blocks update, force-update, and deletion, restricts creation to authorized maintainers, and gives GitHub Actions no bypass;
- have an authorized maintainer create the exact `vMAJOR.MINOR.PATCH` **annotated tag** at the qualified commit outside the workflow; and
- create one empty draft GitHub Release for that existing tag with the exact version title, a blank body, `draft: true`, `prerelease: false`, and zero assets.

The publication workflow never creates a tag or Release. This prevents the Releases API from implicitly manufacturing a lightweight tag when a requested tag is missing. The attach and final jobs independently use their environment-scoped Administration-read credentials to require the immutable-releases policy to be enabled before either write boundary proceeds. The ordinary `GITHUB_TOKEN` receives `contents: write` only inside those two protected jobs.

## Qualification inputs

Use one first-attempt successful `Android Cross Compile` push run from the exact current `main` commit. The run retains exactly nine artifacts for 30 days:

- the seven governed posture bundles listed in [Android validation artifacts](ANDROID_ARTIFACTS.md); and
- `termux-mcp-native-qualification-components`, containing exactly the seven frozen component reports; and
- `termux-mcp-qualified-runtime-snapshot`, containing exactly the retained runtime archive, package lock, and snapshot record.

That successful Android run triggers the read-only `Automated Release Qualification` workflow. The qualifier must itself be a successful first attempt and emits exactly one artifact, `termux-mcp-native-qualification-evidence`, containing twelve members: the seven unchanged components, the runtime archive, package lock, snapshot, newly generated offline replay record, and `automated-qualification-v1.json`. Protected staging discovers this qualifier from its exact Android-run ID and source-commit run title; no qualifier ID is a dispatch input.

The workflow rejects expired, missing, duplicate, extra, pull-request, tag, fork, stale, rerun, incomplete, failed, or substituted Android and qualifier inputs. It revalidates the current `main` ref; exact source SHA; all four CI, Security, Android, and qualifier run IDs; both workflow paths and event types; artifact IDs and server digests; manifest/checksum/size/target/ELF posture; and evidence lineage before and after environment approval.

The exact qualifier artifact must contain the aggregate v4 report, four specialized provider reports, classifier v3, the six-scenario native deployment report, all four retained-runtime members, and `automated-qualification-v1.json`. The qualification envelope must conform to [`release-automated-qualification-schema-v1.json`](release-automated-qualification-schema-v1.json), bind its own qualifier run ID, the source Android run, the committed [`release-qualification-policy-v1.json`](release-qualification-policy-v1.json), and deployment scenario set by digest, and carry this exact boundary:

```json
{
  "physicalDeviceObserved": false,
  "androidFrameworkObserved": false,
  "sustainedPhysicalSoak": false,
  "physicalCertification": "not_run"
}
```

Every automated record states `physicalDeviceObserved:false`, `androidFrameworkObserved:false`, `sustainedPhysicalSoak:false`, `physicalCertification:"not_run"`, and `rebuildReproducibilityClaim:false` where the runtime contract applies. There is no evidence bundle to encode and no physical-evidence workflow input. There is also no qualification-class or qualifier-run workflow input. Staging downloads the seven Android bundles, the retained-runtime snapshot, and the separately produced automated qualification by their exact Actions artifact IDs. `package_physical_qualification.sh` remains available for a separately named optional physical-certification tier; its output cannot be submitted to the automated route.

Automated release qualification proves the exact artifacts under the digest-pinned official Termux userland on native ARM64, including deterministic Android-provider simulation and isolated deployment recovery. It does not certify physical-device, OEM, battery-aging, thermal-soak, radio, Doze, or Android-framework behavior.

Dispatch `Stage Release Assets` from `main` with:

- `expected_commit`: the lowercase 40-character exact current `main` SHA;
- `version`: the exact `Cargo.toml` version;
- `android_run_id`: the qualifying first-attempt Android run.

Preflight performs the complete verification without producing a retained artifact. The protected `stage` job repeats every check after approval, performs a final current-`main` check, and then uploads one raw deterministic tar.

## Staged payload

For v0.7.0 the tar is named `termux-mcp-server-v0.7.0-release-stage-<sha12>.tar`. It contains:

- seven byte-identical binaries under the final `termux-mcp-server-v0.7.0-aarch64-linux-android-<posture>` names;
- one checksum sidecar per binary and a combined `SHA256SUMS`;
- the unchanged workflow manifests under unambiguous names;
- the automated qualification plus its aggregate, specialized, classifier, and deployment evidence;
- the retained runtime archive, package lock, snapshot, and offline replay record under `evidence/runtime/`;
- `LICENSE`; and
- [`release-staging-manifest-v2.json`](release-staging-manifest-schema-v2.json), validated by its closed schema.

The staging manifest binds the exact source plus CI, Security, Android, and qualifier workflow run IDs, qualification class and negative claim boundary, every source and staged digest, all four retained-runtime records, every preserved manifest/evidence digest, and the deterministic member inventory. Renaming never changes binary bytes. Any assembler mismatch before upload leaves no local staging tar. The final step also requires the raw-upload server digest to equal the locally computed tar digest. The staged Actions artifact is retained for 30 days. Because this read-only workflow intentionally cannot delete Actions artifacts, a failure after upload can leave an **unqualified** artifact until an administrator deletes it or retention expires; only a successful workflow summary with matching IDs and digests identifies a qualified stage.

## Fixed public asset set

The v0.7.0 draft must begin empty and, after the protected attachment job, contain exactly sixteen assets:

1. the seven versioned Android binaries listed in [Android validation artifacts](ANDROID_ARTIFACTS.md);
2. the seven matching `<binary-name>.sha256` sidecars;
3. `SHA256SUMS`; and
4. the unchanged raw `termux-mcp-server-v0.7.0-release-stage-<sha12>.tar` downloaded from the exact staging Actions artifact.

The first fifteen files are byte-for-byte members extracted from that tar. The sixteenth is the tar itself, byte-for-byte unchanged. The closed `release-staging-manifest-v2.json`, workflow manifests, LICENSE, sanitized evidence, and all four retained-runtime members remain only inside the raw tar and are not separate GitHub Release assets. A different count, filename, byte length, digest, upload state, or duplicate name fails the draft.

`scripts/prepare_release_publication_assets.sh` validates this projection in a private bundle and exposes the complete `assets/` directory plus `release-publication-receipt-v1.json` with one atomic no-replace directory rename. Bundle-directory existence is the completion marker: an incomplete run exposes neither sibling, and a competing bundle is never replaced or deleted. The receipt is verification state, not a seventeenth Release asset; it remains private and must not be uploaded.

GitHub automatically offers tag-derived source ZIP and tar archives. Those generated downloads are not members of the sixteen-asset contract, are not Android binaries, and are not covered by the publisher's asset digest or `verify-asset` proof. The v0.7.0 contract also makes no separate SBOM or third-party-notice asset claim. Adding any new durable asset requires a separately reviewed staging and publication contract change; it must not be appended during a live release.

## Publication dispatch

Dispatch `Publish Immutable Release` from [`.github/workflows/publish-release.yml`](../.github/workflows/publish-release.yml) only from `main`. Supply the exact current-main commit, package version, annotated tag-object SHA, staging artifact ID, raw staging-tar SHA-256, and pre-created empty draft Release ID. The workflow derives the exact `v<version>` tag name and resolves the artifact's owning staging run from those identities. These identifiers and digests are non-secret. Do not pass a token, release body, binary, evidence document, path, or encoded archive through workflow inputs.

Preflight is read-only. It requires the dispatch workflow/ref/SHA and current `main` to identify the same commit; validates the version-derived protected annotated tag and supplied tag-object SHA; resolves the exact successful first-attempt staging run from the supplied artifact ID; requires the one named unexpired staging artifact by both ID and server digest; downloads that raw tar by ID with digest mismatch as an error; and validates the complete staging tar, manifest, provenance, evidence lineage, member allowlist, and fixed sixteen-asset projection. It also requires the supplied Release ID to be the one `draft: true`, `prerelease: false`, exact-tag draft with the exact version title, blank body, and zero assets. An existing published Release, another draft for the tag, a lightweight/moved tag, or any pre-existing asset is a hard failure.

## Operator worksheet (v0.7.0)

Use this worksheet for one unchanged candidate. Record values only from GitHub or the exact checked-out commit; never guess an ID or digest, and never paste a credential into a workflow input.

| Checkpoint | Exact value to record |
| --- | --- |
| Candidate | `expected_commit=<40-character current main SHA>` |
| Candidate | `version=0.7.0` |
| Candidate | `android_run_id=<successful first-attempt Android push run ID>` |
| Stage result | `staging_run_id=<successful first-attempt staging run ID>` |
| Stage result | `staged_artifact_id=<ID from the successful staging summary>` |
| Stage result | `staged_artifact_sha256=<raw tar SHA-256 from the same summary>` |
| Protected tag | `expected_tag_object_sha=<annotated tag object SHA>` |
| Empty draft | `draft_release_id=<numeric ID of the one exact-tag empty draft>` |
| Public proof | `release_id=<same Release ID>` |
| Public proof | `release_url=<immutable public Release URL>` |
| Public proof | `asset_count=16` and `immutable=true` |

Follow these steps in order:

1. Open **Actions → Stage Release Assets → Run workflow**, select `main`, and enter only `expected_commit`, `version`, and `android_run_id`. Approve `release-qualification` only after checking its actual environment protections. Wait for a terminal first-attempt success. Record `staging_run_id` from the numeric `/actions/runs/<id>` segment of that successful run's URL, then copy `staged_artifact_id` and `staged_artifact_sha256` from the same run's summary.
2. Verify immutable Releases, the active no-bypass `v*` tag ruleset, both publication environments, their disjoint reviewers, and the documented environment-only guards and credentials. Stop if any control is missing.
3. After staging succeeds, an authorized maintainer may create and push the annotated tag without force:

   ```bash
   set -euo pipefail

   RELEASE_REPO=CyberBASSLord-666/termux-mcp-edge
   RELEASE_REMOTE_URL="https://github.com/$RELEASE_REPO.git"
   RELEASE_VERSION=0.7.0
   RELEASE_COMMIT=PASTE_40_CHARACTER_EXPECTED_COMMIT_HERE
   RELEASE_TAG="v$RELEASE_VERSION"

   test "${#RELEASE_COMMIT}" -eq 40
   case "$RELEASE_COMMIT" in
     *[!0-9a-f]*) printf 'stop: expected_commit is not lowercase hexadecimal\n' >&2; exit 1 ;;
   esac
   git fetch --no-tags "$RELEASE_REMOTE_URL" main
   test "$(git rev-parse FETCH_HEAD)" = "$RELEASE_COMMIT"
   if git show-ref --verify --quiet "refs/tags/$RELEASE_TAG"; then
     printf 'stop: local tag already exists: %s\n' "$RELEASE_TAG" >&2
     exit 1
   fi
   REMOTE_TAG_MATCHES="$(
     git ls-remote --refs "$RELEASE_REMOTE_URL" "refs/tags/$RELEASE_TAG"
   )"
   if test -n "$REMOTE_TAG_MATCHES"; then
     printf 'stop: remote tag already exists: %s\n' "$RELEASE_TAG" >&2
     exit 1
   fi
   git tag -a "$RELEASE_TAG" "$RELEASE_COMMIT" -m "$RELEASE_TAG"
   TAG_OBJECT_SHA="$(git rev-parse "$RELEASE_TAG^{tag}")"
   git push "$RELEASE_REMOTE_URL" \
     "refs/tags/$RELEASE_TAG:refs/tags/$RELEASE_TAG"
   REMOTE_TAG_OBJECT_SHA="$(
     git ls-remote --refs "$RELEASE_REMOTE_URL" "refs/tags/$RELEASE_TAG" |
       awk 'NR == 1 { sha = $1 } END { if (NR != 1) exit 1; print sha }'
   )"
   test "$REMOTE_TAG_OBJECT_SHA" = "$TAG_OBJECT_SHA"
   printf '%s\n' "$TAG_OBJECT_SHA"
   ```

   Copy the final command's 40-character result as `expected_tag_object_sha`. A pre-existing tag, a lightweight tag, a different target, or any need for `--force` is a stop condition.
4. Open **Releases → Draft a new release**, choose the existing `v0.7.0` tag, set the title to exactly `v0.7.0`, leave the body completely blank, leave prerelease disabled, attach nothing, and save as a draft. There must be exactly one draft for the tag and it must have zero assets.
5. Capture the numeric draft ID with an authenticated GitHub CLI plus `jq`, or obtain the same ID from an independently authenticated paginated Releases API read:

   ```bash
   set -euo pipefail

   RELEASE_REPO=CyberBASSLord-666/termux-mcp-edge
   RELEASE_VERSION=0.7.0

   DRAFT_RELEASE_ID="$(
     gh api --paginate --slurp "repos/$RELEASE_REPO/releases?per_page=100" |
       jq -er \
         --arg release_tag "v$RELEASE_VERSION" \
         --arg release_name "v$RELEASE_VERSION" '
           add
           | [ .[] | select(.tag_name == $release_tag) ] as $matches
           | if ($matches | length) != 1 then
               error("expected exactly one Release for the tag")
             elif (
               $matches[0].draft == true and
               $matches[0].prerelease == false and
               $matches[0].name == $release_name and
               (($matches[0].body // "") == "") and
               (($matches[0].assets | length) == 0)
             ) then
               $matches[0].id
             else
               error("exact-tag Release is not the required empty draft")
             end
         '
   )"
   test "$(printf '%s\n' "$DRAFT_RELEASE_ID" | sed '/^$/d' | wc -l | tr -d ' ')" = 1
   case "$DRAFT_RELEASE_ID" in
     ''|*[!0-9]*) printf 'stop: draft Release ID is not one numeric value\n' >&2; exit 1 ;;
   esac
   printf '%s\n' "$DRAFT_RELEASE_ID"
   ```

   The paginated response is flattened before filtering: exactly one Release with the exact tag must exist, and that sole object must be the conforming empty draft. Exactly one numeric line is allowed. Record it as `draft_release_id`; zero or multiple tag matches, or one nonconforming match, is a stop condition.
6. Open **Actions → Publish Immutable Release → Run workflow**, select `main`, and enter the six recorded values: `expected_commit`, `version`, `expected_tag_object_sha`, `staged_artifact_id`, `staged_artifact_sha256`, and `draft_release_id`.
7. The `release-production` reviewer sees the successful read-only preflight and the pending protected job's workflow definition; the attachment job's repeated checks execute only after approval. The independent verification record does not exist yet. The disjoint `release-final` reviewer acts only after the fresh read-only verification job has retained that record and rendered its summary.
8. Record success only when the final read-only job reports the same Release ID, `immutable: true`, exactly sixteen governed assets, and successful public byte-for-byte re-download. Copy that run link and the public Release URL into the closure record for issues #301 and #310.

## Publication state machine

The protected workflow has one-way states and does not skip or combine them:

1. **Public, non-confidential stage.** The staging workflow has already emitted one exact raw tar. It remains `staged_not_released` and is not an installation source.
2. **Pre-created empty draft.** An authorized maintainer creates the protected annotated tag and empty draft outside the workflow. Neither object alone authorizes installation.
3. **Protected attachment.** The `release-production` job waits for its environment reviewer, requires `RELEASE_PRODUCTION_PROTECTED=asset-attachment-reviewer-main-only-v1`, verifies the immutable-release policy with `RELEASE_PRODUCTION_POLICY_READ_TOKEN`, and repeats every source, tag, run, artifact, tar, manifest, draft, and zero-asset check. Only then does its job-local `contents: write` token bind the deterministic, provenance-derived release body and attach the fixed sixteen assets. GitHub provides no atomic precondition across each validating read and the following body PATCH or asset POST. Concurrent mutation can therefore only cause validation or attachment to fail and leave an unpublished, non-resumable partial draft; the workflow never automatically deletes or repairs it. It cannot create or publish a Release, create or move a tag, change the title or prerelease state, delete or replace an asset, or rebuild anything. It retains a closed attachment record with all sixteen server-assigned identities and presents the record digests in the job summary.
4. **Independent byte verification.** A fresh read-only job takes only the recorded draft Release ID and expected identities. It lists exactly sixteen uploaded assets, rejects `starter` or non-uploaded state, binds every asset ID/name/size/server digest, downloads every asset afresh by ID, and compares every byte to a separately downloaded exact staging tar. It re-runs `SHA256SUMS`, all seven sidecars, the closed staging-manifest checks, and the raw-tar digest. This job has no release-write permission. It retains the closed JSON verification record for 30 days and renders its run, Release, source, stage, record hashes, and sixteen asset identities in a reviewer-readable job summary.
5. **Separate final approval and mutation freeze.** Only after independent verification and record retention succeed may the `release-final` job wait for its disjoint reviewer. The reviewer must first establish the bounded exclusive-writer freeze above. After approval the job downloads that exact current-run verification artifact by server-assigned ID, requires the recorded Actions digest and file SHA-256, and semantically reproduces the record from the current draft before and immediately before mutation. It requires both final-environment guards, verifies the immutable-release policy with `RELEASE_FINAL_POLICY_READ_TOKEN`, and repeats current-main, tag, stage, asset-ID, server-digest, fresh-download, checksum, and byte-equality checks. Its exact PATCH changes `draft` from `true` to `false`, reasserts the already-verified `prerelease: false` state, and explicitly requests this Release as latest; it makes no tag, asset, title, or body mutation.
6. **Immutable public proof.** Publication success is not the PATCH response alone. A fresh public read-back must report the exact Release ID/tag/commit, `draft: false`, `prerelease: false`, and `immutable: true`. A final read-only proof then downloads all sixteen assets through their public URLs without the policy credential, verifies the exact allowlist and every byte/digest against the retained identities, and records the immutable Release URL, identity, asset count, and successful public proof in its summary.

GitHub makes a tag and attached assets immutable only when an immutable Release is published. Before step 5 the ruleset protects the annotated tag, but documentation must not call it a GitHub-immutable tag. The environments provide separate approval checkpoints; the configured disjoint reviewer sets provide the human separation.

## Publication records

The Release body is bound before upload and contains only deterministic facts already available at that boundary: source and annotated-tag identity, staging/CI/Security/Android run identities, staging artifact and tar digest, toolchain/target/NDK versions, the expected sixteen names/sizes/SHA-256 values, operational limitations, and deployment/governance links. It intentionally does not claim server-assigned Release asset IDs, later approval identities, or a future immutable/public result.

The separate workflow record covers those later facts. The protected attachment and independent verification jobs each retain a closed JSON identity record for 30 days. Those records bind the publication workflow run, Release ID, stage identity, release-body/expected-asset-set digests, and every server-assigned asset ID, name, size, state, content type, API/download URL, and server SHA-256 digest. Their job summaries bind the record file SHA-256 and Actions artifact ID/server digest. The `release-production` reviewer uses the linked run's successful read-only preflight and the pending protected job's workflow definition; the protected attachment checks execute only after approval. The independent verification record does not exist until after that approval and attachment. Its exact workflow-run link and summary are then review context for the disjoint `release-final` decision. Because both protected jobs intentionally set `deployment: false`, they create no GitHub Deployment record; the workflow therefore does not guess or copy reviewer identities into a pre-approval record. Use the linked run's environment-review UI, together with any applicable GitHub audit-log evidence, when human-review attribution must be preserved outside the retained artifacts. The final job must consume the exact retained verification record, and the post-verification summary records the immutable public Release identity after the public byte proof succeeds.

These workflow records contain only public release provenance, not credentials or private device data. They supplement the deterministic Release body; they are not seventeenth Release assets and are not described as permanent once their documented retention expires.

## Draft recovery and stop conditions

Creating a draft, attaching assets, and publishing are not one atomic operation. The workflow therefore fails closed as follows:

- an attachment error, including a concurrent change between a validating GET and a body PATCH or asset POST, may leave a partial draft; that draft is unpublished and non-resumable, and the workflow never auto-deletes or repairs a draft, asset, tag, or staging artifact;
- a verification mismatch leaves the draft unpublished and blocks the final job;
- a denied, rejected, or expired final approval leaves the verified draft unpublished;
- every GitHub workflow rerun is rejected by the first-attempt guard; after inspecting and explicitly retiring or cleaning any partial draft back to the documented empty state, an administrator must start a fresh reviewed dispatch; and
- an ambiguous publish response is resolved by reading the exact Release ID before any retry. If it is public, no publish mutation is repeated; only immutable/public proof continues.

Once GitHub reports the Release immutable, its tag and assets are not repaired, replaced, or deleted by automation. A post-publication proof failure is a release incident and requires a corrected later version while preserving the historical record. Neither a partial draft nor a fully verified draft may be described as public, immutable, installable, or released.

## Publication prerequisites

The checked-in workflow and documentation do not satisfy administrator-only publication controls by themselves. A publication dispatch must remain blocked unless all of the following are true for one unchanged current-main commit:

- exact-main first-attempt CI, Security, Android/native validation, automated qualification, and the protected stage all pass;
- the `release-qualification`, `release-production`, and `release-final` environments exist with their documented guards, branch rules, bypass posture, and reviewer separation;
- the final reviewer can establish the documented exclusive Release/main/settings/tag/ruleset/workflow mutation freeze for the complete final-publication window;
- the two separate environment-only Administration-read policy credentials exist and immutable releases are enabled;
- the active `v*` tag ruleset exists and the exact annotated version tag is protected at the qualified commit;
- the exact-tag draft is pre-created with the exact version title, a blank body, and zero assets; and
- attachment, independent draft verification, separate final approval, publication, `immutable: true`, and public sixteen-asset re-download proof all complete without a waived assertion.

Only the immutable GitHub Release whose public sixteen-asset proof passed is installable. A tag, draft, workflow bundle, or stage is never a substitute.
