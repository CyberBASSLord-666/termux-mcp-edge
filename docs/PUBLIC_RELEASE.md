# Public release staging

This guide separates four things that are easy to confuse:

1. **Workflow bundles** are the seven three-file Android artifacts produced by one exact successful `Android Cross Compile` run. They are qualification inputs, not public downloads.
2. **Automated evidence** is the native ARM64 Termux evidence from that same run. It proves the exact downloaded workflow binaries behaved as specified, but it does not replace a real-device observation.
3. **Physical evidence** is validator-v11/schema-v2 evidence plus a harness-v11 report from the same immutable commit. The harness records a separate locked on-device build digest; it is not claimed byte-identical to the workflow-built full-suite binary.
4. **A release stage** is a deterministic tar containing byte-for-byte copies of the seven qualified workflow binaries under their final versioned names. It is still not a tag or GitHub Release.

## Current boundary

The repository can prepare a protected pre-release stage only. Staging has no `contents: write` permission and no tag, Release, package, deployment, or OIDC permission. A staged tar says `publicationState: "staged_not_released"` and `releaseEligible: false` even though the physical validator report inside it must independently say `releaseEligible: true`.

The repository is public, and GitHub Actions artifacts are available to signed-in people with repository read access. The staged tar and both dispatch files are therefore **not confidential storage**. They contain only release-intended binaries and sanitized, path-free evidence. Never put the raw harness report, credentials, personal data, private device paths, or other secrets in a workflow input or staged artifact.

No operator should install from a workflow bundle or staged tar as though it were a durable public release. Public installation begins only after a separately protected publication step attaches the exact staged bytes to a verified immutable tag and re-downloads every asset for checksum verification.

## One-time administrator setup

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

## What remains before publication

A staged artifact is intentionally temporary and is not a durable GitHub Release. It is not confidential in this public repository. Public publication additionally requires a pre-existing annotated or signed version tag at the staged commit, a separate `release-production` approval boundary, exact staged-artifact ID and server-digest verification, immutable-release policy verification, asset upload followed by server-side re-download, and an independent final publication approval. A future publisher must consume the staged tar; it must never rebuild the binaries or let the Release API create a missing tag implicitly.
