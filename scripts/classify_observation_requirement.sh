#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
umask 077

CLASSIFIER_VERSION=2

REPOSITORY_ROOT=''
SOURCE_COMMIT=''
CANDIDATE_COMMIT=''
EMULATED_REPORT=''
OUTPUT_REPORT=''

fail() {
  printf 'OBSERVATION_REQUIREMENT_RESULT=FAIL reason=%s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage: classify_observation_requirement.sh \
  --repository-root DIR \
  --source-commit SHA \
  --candidate-commit SHA \
  --emulated-report REPORT.json \
  --output REPORT.json

Classify a passing native ARM64 Termux candidate as either eligible for the
separate observation-inheritance verifier or as requiring a new direct
physical-device observation. Classification never grants release eligibility.
EOF
}

while (($#)); do
  case "$1" in
    --repository-root) (($# >= 2)) || fail missing_repository_root; REPOSITORY_ROOT="$2"; shift 2 ;;
    --source-commit) (($# >= 2)) || fail missing_source_commit; SOURCE_COMMIT="$2"; shift 2 ;;
    --candidate-commit) (($# >= 2)) || fail missing_candidate_commit; CANDIDATE_COMMIT="$2"; shift 2 ;;
    --emulated-report) (($# >= 2)) || fail missing_emulated_report; EMULATED_REPORT="$2"; shift 2 ;;
    --output) (($# >= 2)) || fail missing_output; OUTPUT_REPORT="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) fail unknown_argument ;;
  esac
done

for sha in "$SOURCE_COMMIT" "$CANDIDATE_COMMIT"; do
  [[ "$sha" =~ ^[0-9a-f]{40}$ ]] || fail commit_invalid
done
[[ "$REPOSITORY_ROOT" == /* && "$EMULATED_REPORT" == /* && "$OUTPUT_REPORT" == /* ]] || fail absolute_paths_required
[[ -e "$REPOSITORY_ROOT/.git" && ! -L "$REPOSITORY_ROOT/.git" ]] || fail repository_invalid
[[ -f "$EMULATED_REPORT" && ! -L "$EMULATED_REPORT" ]] || fail emulated_report_invalid
[[ ! -e "$OUTPUT_REPORT" && ! -L "$OUTPUT_REPORT" ]] || fail output_already_exists
[[ -d "$(dirname "$OUTPUT_REPORT")" && ! -L "$(dirname "$OUTPUT_REPORT")" ]] || fail output_parent_invalid

for command in awk chmod date dirname git install jq python3 rm sha256sum; do
  command -v "$command" >/dev/null 2>&1 || fail "required_command_missing_$command"
done

cd "$REPOSITORY_ROOT"
[[ "$(git rev-parse --is-inside-work-tree 2>/dev/null)" == true ]] || fail repository_invalid
git cat-file -e "$SOURCE_COMMIT^{commit}" 2>/dev/null || fail source_commit_missing
git cat-file -e "$CANDIDATE_COMMIT^{commit}" 2>/dev/null || fail candidate_commit_missing
git merge-base --is-ancestor "$SOURCE_COMMIT" "$CANDIDATE_COMMIT" || fail candidate_not_descended_from_source

EMULATED_REPORT_SHA="$(sha256sum "$EMULATED_REPORT" | awk '{print $1}')"
[[ "$EMULATED_REPORT_SHA" =~ ^[0-9a-f]{64}$ ]] || fail emulated_report_digest_invalid
jq -e \
  --arg candidate "$CANDIDATE_COMMIT" '
    .schemaVersion == 3
    and .gateVersion == "3"
    and .status == "pass"
    and .failureCode == null
    and .candidate.commit == $candidate
    and (.candidate.version | type == "string" and test("^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$"))
    and (.candidate.ciRunId | type == "string" and test("^[1-9][0-9]*$"))
    and (.candidate.securityRunId | type == "string" and test("^[1-9][0-9]*$"))
    and (.candidate.androidRunId | type == "string" and test("^[1-9][0-9]*$"))
    and .environment.executionMode == "official-termux-docker-native-arm64"
    and .environment.androidLinker == true
    and .runtimeValidation.status == "pass"
    and (.candidate.fullSuiteArtifact.sha256 | type == "string" and test("^[0-9a-f]{64}$"))
    and (.candidate.fullSuiteArtifact.manifestSha256 | type == "string" and test("^[0-9a-f]{64}$"))
    and .candidate.fullSuiteArtifact.artifactName == "termux-mcp-server-aarch64-linux-android-full-suite"
    and .candidate.fullSuiteArtifact.posture == "full-suite"
    and .candidate.fullSuiteArtifact.features == ["full-suite"]
    and .candidate.fullSuiteArtifact.fileName == "termux-mcp-server"
    and .aggregateValidation.status == "pass"
    and .aggregateValidation.defaultDisabled.toolCount == 17
    and .aggregateValidation.fullyEnabled.toolCount == 21
    and .aggregateValidation.directPhysicalObservationRequired == true
    and .stress.status == "pass"
    and .stress.samples >= 32
    and .stress.safeRootIdentityPinned == true
    and .stress.safeRootAncestorIdentityPinned == true
    and .stress.longObservationRequired == false
  ' "$EMULATED_REPORT" >/dev/null || fail emulated_report_contract_invalid

runtime_inputs_unchanged=true
if git diff --quiet "$SOURCE_COMMIT" "$CANDIDATE_COMMIT" -- \
  src \
  build.rs \
  .cargo \
  rust-toolchain.toml \
  scripts/cross_compile.sh \
  scripts/package_android_artifact.sh \
  scripts/termux_deploy.sh \
  scripts/termux_device_smoke.sh \
  scripts/termux_release_validate.sh \
  docs/release-evidence-schema-v1.json; then
  :
else
  diff_status=$?
  ((diff_status == 1)) || fail protected_input_comparison_failed
  runtime_inputs_unchanged=false
fi

cargo_inputs_unchanged=true
if python3 - "$REPOSITORY_ROOT" "$SOURCE_COMMIT" "$CANDIDATE_COMMIT" <<'PY'
import json
import pathlib
import subprocess
import sys
import tomllib

root = pathlib.Path(sys.argv[1])
source = sys.argv[2]
candidate = sys.argv[3]

def read_toml(ref: str, path: str):
    raw = subprocess.check_output(
        ["git", "-C", str(root), "show", f"{ref}:{path}"],
        text=True,
        stderr=subprocess.DEVNULL,
    )
    return tomllib.loads(raw)

def normalized_manifest(ref: str):
    value = read_toml(ref, "Cargo.toml")
    package = value.get("package")
    if not isinstance(package, dict) or "version" not in package:
        raise ValueError("root package version missing")
    package = dict(package)
    package.pop("version")
    value = dict(value)
    value["package"] = package
    return value

def normalized_lock(ref: str):
    value = read_toml(ref, "Cargo.lock")
    packages = value.get("package")
    if not isinstance(packages, list):
        raise ValueError("lockfile packages missing")
    matches = 0
    normalized = []
    for package in packages:
        package = dict(package)
        if package.get("name") == "termux-mcp-server":
            matches += 1
            package.pop("version", None)
        normalized.append(package)
    if matches != 1:
        raise ValueError("unexpected root package cardinality")
    value = dict(value)
    value["package"] = normalized
    return value

try:
    same = all(
        json.dumps(left, sort_keys=True, separators=(",", ":"))
        == json.dumps(right, sort_keys=True, separators=(",", ":"))
        for left, right in (
            (normalized_manifest(source), normalized_manifest(candidate)),
            (normalized_lock(source), normalized_lock(candidate)),
        )
    )
except Exception:
    raise SystemExit(2)

raise SystemExit(0 if same else 1)
PY
then
  :
else
  comparison_status=$?
  if ((comparison_status == 1)); then
    cargo_inputs_unchanged=false
  else
    fail cargo_input_comparison_failed
  fi
fi

# Aggregate v3 introduces a governed build input that the historical physical
# source did not observe. Preserve its digests in the report, but never route
# this release candidate through the legacy bridge.
full_suite_direct_observation_required=true
if [[ "$full_suite_direct_observation_required" == true ]]; then
  inheritance_candidate=false
  evidence_mode=physical_observation_required
  reason_code=full_suite_direct_physical_observation_required
  next_gate=direct_physical_device_observation
elif [[ "$runtime_inputs_unchanged" == true && "$cargo_inputs_unchanged" == true ]]; then
  inheritance_candidate=true
  evidence_mode=observation_inheritance_candidate
  reason_code=inheritance_verification_required
  next_gate=observation_inheritance_verification
elif [[ "$runtime_inputs_unchanged" == false && "$cargo_inputs_unchanged" == false ]]; then
  inheritance_candidate=false
  evidence_mode=physical_observation_required
  reason_code=runtime_and_build_inputs_changed
  next_gate=direct_physical_device_observation
elif [[ "$runtime_inputs_unchanged" == false ]]; then
  inheritance_candidate=false
  evidence_mode=physical_observation_required
  reason_code=runtime_or_deployment_inputs_changed
  next_gate=direct_physical_device_observation
else
  inheritance_candidate=false
  evidence_mode=physical_observation_required
  reason_code=cargo_or_dependency_inputs_changed
  next_gate=direct_physical_device_observation
fi

CREATED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
CANDIDATE_VERSION="$(jq -r .candidate.version "$EMULATED_REPORT")"
CI_RUN_ID="$(jq -r .candidate.ciRunId "$EMULATED_REPORT")"
SECURITY_RUN_ID="$(jq -r .candidate.securityRunId "$EMULATED_REPORT")"
ANDROID_RUN_ID="$(jq -r .candidate.androidRunId "$EMULATED_REPORT")"
IMAGE_DIGEST="$(jq -r .environment.imageDigest "$EMULATED_REPORT")"
SAMPLES="$(jq -r .stress.samples "$EMULATED_REPORT")"
FULL_SUITE_SHA="$(jq -r .candidate.fullSuiteArtifact.sha256 "$EMULATED_REPORT")"
FULL_SUITE_MANIFEST_SHA="$(jq -r .candidate.fullSuiteArtifact.manifestSha256 "$EMULATED_REPORT")"
[[ "$FULL_SUITE_SHA" =~ ^[0-9a-f]{64}$ && "$FULL_SUITE_MANIFEST_SHA" =~ ^[0-9a-f]{64}$ ]] || fail full_suite_digest_invalid

REPORT_NEXT="$(dirname "$OUTPUT_REPORT")/.observation-requirement-$$.json"
jq -n \
  --arg classifier_version "$CLASSIFIER_VERSION" \
  --arg created_at "$CREATED_AT" \
  --arg evidence_mode "$evidence_mode" \
  --arg reason_code "$reason_code" \
  --arg next_gate "$next_gate" \
  --arg source_commit "$SOURCE_COMMIT" \
  --arg candidate_commit "$CANDIDATE_COMMIT" \
  --arg candidate_version "$CANDIDATE_VERSION" \
  --arg ci_run_id "$CI_RUN_ID" \
  --arg security_run_id "$SECURITY_RUN_ID" \
  --arg android_run_id "$ANDROID_RUN_ID" \
  --arg full_suite_sha "$FULL_SUITE_SHA" \
  --arg full_suite_manifest_sha "$FULL_SUITE_MANIFEST_SHA" \
  --arg emulated_report_sha "$EMULATED_REPORT_SHA" \
  --arg image_digest "$IMAGE_DIGEST" \
  --argjson samples "$SAMPLES" \
  --argjson inheritance_candidate "$inheritance_candidate" \
  --argjson runtime_inputs_unchanged "$runtime_inputs_unchanged" \
  --argjson cargo_inputs_unchanged "$cargo_inputs_unchanged" \
  --argjson full_suite_direct_observation_required "$full_suite_direct_observation_required" '
  {
    schemaVersion: 2,
    classifierVersion: $classifier_version,
    status: "pass",
    failureCode: null,
    releaseQualificationEligible: false,
    createdAt: $created_at,
    evidenceMode: $evidence_mode,
    reasonCode: $reason_code,
    inheritanceCandidate: $inheritance_candidate,
    source: {commit: $source_commit},
    candidate: {
      commit: $candidate_commit,
      version: $candidate_version,
      ciRunId: $ci_run_id,
      securityRunId: $security_run_id,
      androidRunId: $android_run_id,
      fullSuiteArtifactSha256: $full_suite_sha,
      fullSuiteManifestSha256: $full_suite_manifest_sha
    },
    emulation: {
      reportSha256: $emulated_report_sha,
      executionMode: "official-termux-docker-native-arm64",
      imageDigest: $image_digest,
      status: "pass",
      samples: $samples
    },
    protectedInputComparison: {
      runtimeAndDeploymentInputsUnchanged: $runtime_inputs_unchanged,
      cargoAndDependencyInputsUnchangedExceptRootVersion: $cargo_inputs_unchanged
    },
    changedInputClasses: [
      if $runtime_inputs_unchanged then empty else "runtime_or_deployment" end,
      if $cargo_inputs_unchanged then empty else "cargo_or_dependency" end,
      if $full_suite_direct_observation_required then "full_suite_artifact" else empty end
    ],
    nextGate: $next_gate
  }' >"$REPORT_NEXT" || fail report_generation_failed
chmod 600 "$REPORT_NEXT" || fail report_mode_failed

jq -e '
  .schemaVersion == 2 and .classifierVersion == "2" and .status == "pass"
  and .failureCode == null and .releaseQualificationEligible == false
  and (.inheritanceCandidate | type == "boolean")
  and .emulation.status == "pass"
  and (.candidate.fullSuiteArtifactSha256 | test("^[0-9a-f]{64}$"))
  and (.candidate.fullSuiteManifestSha256 | test("^[0-9a-f]{64}$"))
  and (
    if .inheritanceCandidate then
      .evidenceMode == "observation_inheritance_candidate"
      and .reasonCode == "inheritance_verification_required"
      and .changedInputClasses == []
      and .nextGate == "observation_inheritance_verification"
    else
      .evidenceMode == "physical_observation_required"
      and (.changedInputClasses | length) >= 1
      and .nextGate == "direct_physical_device_observation"
    end
  )
' "$REPORT_NEXT" >/dev/null || fail generated_report_invalid

install -m 600 "$REPORT_NEXT" "$OUTPUT_REPORT" || fail report_publication_failed
rm -f -- "$REPORT_NEXT"

printf 'observation_requirement_report_sha256=%s\n' "$(sha256sum "$OUTPUT_REPORT" | awk '{print $1}')"
printf 'observation_requirement_report=%s\n' "$OUTPUT_REPORT"
printf 'observation_inheritance_candidate=%s\n' "$inheritance_candidate"
printf 'OBSERVATION_REQUIREMENT_RESULT=PASS\n'
