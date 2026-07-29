#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
umask 077

CLASSIFIER_VERSION=3

REPOSITORY_ROOT=''
SOURCE_COMMIT=''
CANDIDATE_COMMIT=''
EMULATED_REPORT=''
OUTPUT_REPORT=''
REPORT_NEXT=''

fail() {
  printf 'OBSERVATION_REQUIREMENT_RESULT=FAIL reason=%s\n' "$*" >&2
  exit 1
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM HUP
  [[ -z "$REPORT_NEXT" ]] || rm -f -- "$REPORT_NEXT" >/dev/null 2>&1 || status=1
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

usage() {
  cat <<'EOF'
Usage: classify_observation_requirement.sh \
  --repository-root DIR \
  --source-commit SHA \
  --candidate-commit SHA \
  --emulated-report REPORT.json \
  --output REPORT.json

Route a passing native ARM64 official-Termux candidate to the closed automated
release-qualification assembler. Classification never grants release
eligibility and never claims physical-device or Android-framework observation.
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
OUTPUT_PARENT="$(dirname "$OUTPUT_REPORT")"
[[ -d "$OUTPUT_PARENT" && ! -L "$OUTPUT_PARENT" ]] || fail output_parent_invalid
[[ "$(realpath -e "$OUTPUT_PARENT")" == "$OUTPUT_PARENT" ]] || fail output_parent_invalid
[[ "$(stat -c %a "$OUTPUT_PARENT")" == 700 ]] || fail output_parent_invalid

for command in awk chmod date dirname git jq ln mktemp python3 realpath rm sha256sum stat; do
  command -v "$command" >/dev/null 2>&1 || fail "required_command_missing_$command"
done
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)" \
  || fail commit_helper_invalid
COMMIT_HELPER="$SCRIPT_DIR/commit_verified_file.py"
[[ -f "$COMMIT_HELPER" && ! -L "$COMMIT_HELPER" ]] || fail commit_helper_invalid

python3 - "$EMULATED_REPORT" <<'PY' || fail emulated_report_json_invalid
import json
import pathlib
import sys

def closed_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate key: {key}")
        result[key] = value
    return result

def reject_constant(value):
    raise ValueError(f"non-finite number: {value}")

path = pathlib.Path(sys.argv[1])
with path.open("r", encoding="utf-8") as source:
    json.load(
        source,
        object_pairs_hook=closed_object,
        parse_constant=reject_constant,
    )
PY

cd "$REPOSITORY_ROOT"
[[ "$(git rev-parse --is-inside-work-tree 2>/dev/null)" == true ]] || fail repository_invalid
git cat-file -e "$SOURCE_COMMIT^{commit}" 2>/dev/null || fail source_commit_missing
git cat-file -e "$CANDIDATE_COMMIT^{commit}" 2>/dev/null || fail candidate_commit_missing
git merge-base --is-ancestor "$SOURCE_COMMIT" "$CANDIDATE_COMMIT" || fail candidate_not_descended_from_source

EMULATED_REPORT_SHA="$(sha256sum "$EMULATED_REPORT" | awk '{print $1}')"
[[ "$EMULATED_REPORT_SHA" =~ ^[0-9a-f]{64}$ ]] || fail emulated_report_digest_invalid
jq -e \
  --arg candidate "$CANDIDATE_COMMIT" '
    (keys == ["aggregateValidation","candidate","claimBoundary","completedAt","coverage","environment","failureCode","gateVersion","releaseQualificationEligible","runtimeValidation","schemaVersion","startedAt","status","stress"])
    and .schemaVersion == 4
    and .gateVersion == "4"
    and .status == "pass"
    and .failureCode == null
    and .releaseQualificationEligible == false
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
    and .aggregateValidation.automatedQualificationComponent == true
    and .claimBoundary == {
      physicalDeviceObserved: false,
      androidFrameworkObserved: false,
      sustainedPhysicalSoak: false,
      physicalCertification: "not_run"
    }
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

# Source comparisons remain useful audit facts, but they no longer decide
# whether a candidate must fabricate or inherit physical observation. Every
# exact native-Termux candidate is routed to the same closed automated
# assembler, which independently validates all current evidence.
inheritance_candidate=false
evidence_mode=automated_release_qualification
reason_code=automated_native_termux_evidence_required
next_gate=assemble_automated_release_qualification

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

REPORT_NEXT="$(mktemp "$OUTPUT_PARENT/.observation-requirement.XXXXXX")" \
  || fail report_generation_failed
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
  --argjson cargo_inputs_unchanged "$cargo_inputs_unchanged" '
  {
    schemaVersion: 3,
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
    claimBoundary: {
      physicalDeviceObserved: false,
      androidFrameworkObserved: false,
      sustainedPhysicalSoak: false,
      physicalCertification: "not_run"
    },
    protectedInputComparison: {
      runtimeAndDeploymentInputsUnchanged: $runtime_inputs_unchanged,
      cargoAndDependencyInputsUnchangedExceptRootVersion: $cargo_inputs_unchanged
    },
    changedInputClasses: [
      if $runtime_inputs_unchanged then empty else "runtime_or_deployment" end,
      if $cargo_inputs_unchanged then empty else "cargo_or_dependency" end
    ],
    nextGate: $next_gate
  }' >"$REPORT_NEXT" || fail report_generation_failed
chmod 600 "$REPORT_NEXT" || fail report_mode_failed

jq -e '
  .schemaVersion == 3 and .classifierVersion == "3" and .status == "pass"
  and .failureCode == null and .releaseQualificationEligible == false
  and .inheritanceCandidate == false
  and .emulation.status == "pass"
  and (.candidate.fullSuiteArtifactSha256 | test("^[0-9a-f]{64}$"))
  and (.candidate.fullSuiteManifestSha256 | test("^[0-9a-f]{64}$"))
  and .evidenceMode == "automated_release_qualification"
  and .reasonCode == "automated_native_termux_evidence_required"
  and .nextGate == "assemble_automated_release_qualification"
  and (.protectedInputComparison |
    keys == [
      "cargoAndDependencyInputsUnchangedExceptRootVersion",
      "runtimeAndDeploymentInputsUnchanged"
    ]
    and (.runtimeAndDeploymentInputsUnchanged | type == "boolean")
    and (.cargoAndDependencyInputsUnchangedExceptRootVersion | type == "boolean")
  )
  and .changedInputClasses == [
    if .protectedInputComparison.runtimeAndDeploymentInputsUnchanged
    then empty else "runtime_or_deployment" end,
    if .protectedInputComparison.cargoAndDependencyInputsUnchangedExceptRootVersion
    then empty else "cargo_or_dependency" end
  ]
  and .claimBoundary == {
    physicalDeviceObserved: false,
    androidFrameworkObserved: false,
    sustainedPhysicalSoak: false,
    physicalCertification: "not_run"
  }
' "$REPORT_NEXT" >/dev/null || fail generated_report_invalid

REPORT_SHA="$(sha256sum -- "$REPORT_NEXT" | awk '{print $1}')" \
  || fail report_publication_failed
if ! python3 "$COMMIT_HELPER" \
  --source "$REPORT_NEXT" \
  --destination "$OUTPUT_REPORT" \
  --sha256 "$REPORT_SHA" \
  --mode 600
then
  fail output_already_exists
fi
rm -f -- "$REPORT_NEXT" >/dev/null 2>&1 || true
REPORT_NEXT=''

printf 'observation_requirement_report_sha256=%s\n' "$REPORT_SHA"
printf 'observation_requirement_report=%s\n' "$OUTPUT_REPORT"
printf 'observation_route=%s\n' "$evidence_mode"
printf 'OBSERVATION_REQUIREMENT_RESULT=PASS\n'
