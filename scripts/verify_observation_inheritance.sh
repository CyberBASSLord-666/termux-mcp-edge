#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
umask 077

VERIFIER_VERSION=1

REPOSITORY_ROOT=''
SOURCE_COMMIT=''
SOURCE_REPORT=''
SOURCE_REPORT_SHA=''
CANDIDATE_COMMIT=''
EMULATED_REPORT=''
BRIDGE_COMMIT=''
BRIDGE_DEFAULT_SHA=''
BRIDGE_MCP_SHA=''
OUTPUT_REPORT=''

fail() {
  printf 'OBSERVATION_INHERITANCE_RESULT=FAIL reason=%s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage: verify_observation_inheritance.sh \
  --repository-root DIR \
  --source-commit SHA \
  --source-report REPORT.json \
  --source-report-sha256 SHA256 \
  --candidate-commit SHA \
  --emulated-report REPORT.json \
  --bridge-commit SHA \
  --bridge-default-sha256 SHA256 \
  --bridge-mcp-sha256 SHA256 \
  --output REPORT.json

Observation inheritance is intentionally narrow. It succeeds only when the
candidate descends from a physically observed source, runtime/deployment/build
inputs remain equivalent after normalizing the root package version, and the
exact candidate artifacts match the independently qualified bridge digests.
EOF
}

while (($#)); do
  case "$1" in
    --repository-root) (($# >= 2)) || fail missing_repository_root; REPOSITORY_ROOT="$2"; shift 2 ;;
    --source-commit) (($# >= 2)) || fail missing_source_commit; SOURCE_COMMIT="$2"; shift 2 ;;
    --source-report) (($# >= 2)) || fail missing_source_report; SOURCE_REPORT="$2"; shift 2 ;;
    --source-report-sha256) (($# >= 2)) || fail missing_source_report_sha; SOURCE_REPORT_SHA="$2"; shift 2 ;;
    --candidate-commit) (($# >= 2)) || fail missing_candidate_commit; CANDIDATE_COMMIT="$2"; shift 2 ;;
    --emulated-report) (($# >= 2)) || fail missing_emulated_report; EMULATED_REPORT="$2"; shift 2 ;;
    --bridge-commit) (($# >= 2)) || fail missing_bridge_commit; BRIDGE_COMMIT="$2"; shift 2 ;;
    --bridge-default-sha256) (($# >= 2)) || fail missing_bridge_default_sha; BRIDGE_DEFAULT_SHA="$2"; shift 2 ;;
    --bridge-mcp-sha256) (($# >= 2)) || fail missing_bridge_mcp_sha; BRIDGE_MCP_SHA="$2"; shift 2 ;;
    --output) (($# >= 2)) || fail missing_output; OUTPUT_REPORT="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) fail unknown_argument ;;
  esac
done

for sha in "$SOURCE_COMMIT" "$CANDIDATE_COMMIT" "$BRIDGE_COMMIT"; do
  [[ "$sha" =~ ^[0-9a-f]{40}$ ]] || fail commit_invalid
done
for sha in "$SOURCE_REPORT_SHA" "$BRIDGE_DEFAULT_SHA" "$BRIDGE_MCP_SHA"; do
  [[ "$sha" =~ ^[0-9a-f]{64}$ ]] || fail digest_invalid
done
[[ "$BRIDGE_DEFAULT_SHA" != "$BRIDGE_MCP_SHA" ]] || fail bridge_postures_not_distinct
[[ "$REPOSITORY_ROOT" == /* && "$SOURCE_REPORT" == /* && "$EMULATED_REPORT" == /* && "$OUTPUT_REPORT" == /* ]] || fail absolute_paths_required
[[ -e "$REPOSITORY_ROOT/.git" && ! -L "$REPOSITORY_ROOT/.git" ]] || fail repository_invalid
[[ -f "$SOURCE_REPORT" && ! -L "$SOURCE_REPORT" ]] || fail source_report_invalid
[[ -f "$EMULATED_REPORT" && ! -L "$EMULATED_REPORT" ]] || fail emulated_report_invalid
[[ ! -e "$OUTPUT_REPORT" && ! -L "$OUTPUT_REPORT" ]] || fail output_already_exists

for command in git jq python3 sha256sum stat; do
  command -v "$command" >/dev/null 2>&1 || fail "required_command_missing_$command"
done

cd "$REPOSITORY_ROOT"
[[ "$(git rev-parse --is-inside-work-tree 2>/dev/null)" == true ]] || fail repository_invalid
git cat-file -e "$SOURCE_COMMIT^{commit}" 2>/dev/null || fail source_commit_missing
git cat-file -e "$CANDIDATE_COMMIT^{commit}" 2>/dev/null || fail candidate_commit_missing
git cat-file -e "$BRIDGE_COMMIT^{commit}" 2>/dev/null || fail bridge_commit_missing
git merge-base --is-ancestor "$SOURCE_COMMIT" "$BRIDGE_COMMIT" || fail bridge_not_descended_from_source
git merge-base --is-ancestor "$BRIDGE_COMMIT" "$CANDIDATE_COMMIT" || fail candidate_not_descended_from_bridge

[[ "$(sha256sum "$SOURCE_REPORT" | awk '{print $1}')" == "$SOURCE_REPORT_SHA" ]] || fail source_report_digest_mismatch
jq -e --arg source "$SOURCE_COMMIT" '
  .schemaVersion == 1
  and .status == "pass"
  and .failureCode == null
  and .releaseEligible == true
  and .repository.commit == $source
  and .environment.architecture == "aarch64"
  and .environment.fixtureMode == false
  and .requestedPhase == "all"
  and .phases == {preflight:"pass",runtime:"pass",deployment:"pass"}
  and .sustainedObservation.operatorSupplied == true
  and .sustainedObservation.status == "pass"
  and .sustainedObservation.minutes >= 60
  and .sustainedObservation.reasonCode == "stable"
  and .sustainedObservation.minimumMinutes == 60
' "$SOURCE_REPORT" >/dev/null || fail source_report_contract_invalid

EMULATED_REPORT_SHA="$(sha256sum "$EMULATED_REPORT" | awk '{print $1}')"
[[ "$EMULATED_REPORT_SHA" =~ ^[0-9a-f]{64}$ ]] || fail emulated_report_digest_invalid
jq -e \
  --arg candidate "$CANDIDATE_COMMIT" \
  --arg default_sha "$BRIDGE_DEFAULT_SHA" \
  --arg mcp_sha "$BRIDGE_MCP_SHA" '
    .schemaVersion == 1
    and .gateVersion == "1"
    and .status == "pass"
    and .failureCode == null
    and .candidate.commit == $candidate
    and .candidate.defaultArtifact.sha256 == $default_sha
    and .candidate.mcpRuntimeArtifact.sha256 == $mcp_sha
    and .environment.executionMode == "official-termux-docker-native-arm64"
    and .environment.androidLinker == true
    and .runtimeValidation.status == "pass"
    and .stress.status == "pass"
    and .stress.samples >= 32
    and .stress.servicePidStable == true
    and .stress.healthReadyStable == true
    and .stress.sessionLifecycle == true
    and .stress.exactToolAllowlist == true
    and .stress.highImpactDisabled == true
  ' "$EMULATED_REPORT" >/dev/null || fail emulated_report_contract_invalid

git diff --quiet "$SOURCE_COMMIT" "$CANDIDATE_COMMIT" -- \
  src \
  build.rs \
  .cargo \
  rust-toolchain.toml \
  scripts/cross_compile.sh \
  scripts/package_android_artifact.sh \
  scripts/termux_deploy.sh \
  scripts/termux_device_smoke.sh \
  scripts/termux_release_validate.sh \
  docs/release-evidence-schema-v1.json || fail runtime_or_deployment_inputs_changed

python3 - "$REPOSITORY_ROOT" "$SOURCE_COMMIT" "$CANDIDATE_COMMIT" <<'PY'
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
    )
    return tomllib.loads(raw)

def normalized_manifest(ref: str):
    value = read_toml(ref, "Cargo.toml")
    package = value.get("package")
    if not isinstance(package, dict) or "version" not in package:
        raise SystemExit("root package version missing")
    package = dict(package)
    package.pop("version")
    value = dict(value)
    value["package"] = package
    return value

def normalized_lock(ref: str):
    value = read_toml(ref, "Cargo.lock")
    packages = value.get("package")
    if not isinstance(packages, list):
        raise SystemExit("lockfile packages missing")
    matches = 0
    normalized = []
    for package in packages:
        package = dict(package)
        if package.get("name") == "termux-mcp-server":
            matches += 1
            package.pop("version", None)
        normalized.append(package)
    if matches != 1:
        raise SystemExit("unexpected root package cardinality")
    value = dict(value)
    value["package"] = normalized
    return value

for label, left, right in (
    ("Cargo.toml", normalized_manifest(source), normalized_manifest(candidate)),
    ("Cargo.lock", normalized_lock(source), normalized_lock(candidate)),
):
    if json.dumps(left, sort_keys=True, separators=(",", ":")) != json.dumps(
        right, sort_keys=True, separators=(",", ":")
    ):
        raise SystemExit(f"{label} changed beyond root package version")
PY

SOURCE_VERSION="$(jq -r .repository.version "$SOURCE_REPORT")"
CANDIDATE_VERSION="$(jq -r .candidate.version "$EMULATED_REPORT")"
[[ "$SOURCE_VERSION" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] || fail source_version_invalid
[[ "$CANDIDATE_VERSION" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] || fail candidate_version_invalid
[[ "$SOURCE_VERSION" != "$CANDIDATE_VERSION" ]] || fail versions_not_distinct

SOURCE_MINUTES="$(jq -r .sustainedObservation.minutes "$SOURCE_REPORT")"
SOURCE_REASON="$(jq -r .sustainedObservation.reasonCode "$SOURCE_REPORT")"
CI_RUN_ID="$(jq -r .candidate.ciRunId "$EMULATED_REPORT")"
SECURITY_RUN_ID="$(jq -r .candidate.securityRunId "$EMULATED_REPORT")"
ANDROID_RUN_ID="$(jq -r .candidate.androidRunId "$EMULATED_REPORT")"
DEFAULT_BYTES="$(jq -r .candidate.defaultArtifact.bytes "$EMULATED_REPORT")"
MCP_BYTES="$(jq -r .candidate.mcpRuntimeArtifact.bytes "$EMULATED_REPORT")"
IMAGE_DIGEST="$(jq -r .environment.imageDigest "$EMULATED_REPORT")"
SAMPLES="$(jq -r .stress.samples "$EMULATED_REPORT")"
REQUESTS="$(jq -r .stress.requests "$EMULATED_REPORT")"
CREATED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

REPORT_NEXT="$(dirname "$OUTPUT_REPORT")/.observation-inheritance-$$.json"
jq -n \
  --arg verifier_version "$VERIFIER_VERSION" \
  --arg created_at "$CREATED_AT" \
  --arg source_commit "$SOURCE_COMMIT" \
  --arg source_version "$SOURCE_VERSION" \
  --arg source_report_sha "$SOURCE_REPORT_SHA" \
  --argjson source_minutes "$SOURCE_MINUTES" \
  --arg source_reason "$SOURCE_REASON" \
  --arg bridge_commit "$BRIDGE_COMMIT" \
  --arg candidate_commit "$CANDIDATE_COMMIT" \
  --arg candidate_version "$CANDIDATE_VERSION" \
  --arg ci_run_id "$CI_RUN_ID" \
  --arg security_run_id "$SECURITY_RUN_ID" \
  --arg android_run_id "$ANDROID_RUN_ID" \
  --arg default_sha "$BRIDGE_DEFAULT_SHA" \
  --argjson default_bytes "$DEFAULT_BYTES" \
  --arg mcp_sha "$BRIDGE_MCP_SHA" \
  --argjson mcp_bytes "$MCP_BYTES" \
  --arg emulated_report_sha "$EMULATED_REPORT_SHA" \
  --arg image_digest "$IMAGE_DIGEST" \
  --argjson samples "$SAMPLES" \
  --argjson requests "$REQUESTS" '
  {
    schemaVersion: 1,
    verifierVersion: $verifier_version,
    status: "pass",
    failureCode: null,
    releaseQualificationEligible: true,
    createdAt: $created_at,
    evidenceMode: "inherited_physical_observation",
    sourceObservation: {
      commit: $source_commit,
      version: $source_version,
      reportSha256: $source_report_sha,
      operatorSupplied: true,
      physicalDevice: true,
      status: "pass",
      minutes: $source_minutes,
      reasonCode: $source_reason
    },
    bridge: {
      commit: $bridge_commit,
      defaultArtifactSha256: $default_sha,
      mcpRuntimeArtifactSha256: $mcp_sha
    },
    candidate: {
      commit: $candidate_commit,
      version: $candidate_version,
      ciRunId: $ci_run_id,
      securityRunId: $security_run_id,
      androidRunId: $android_run_id,
      defaultArtifact: {sha256: $default_sha, bytes: $default_bytes},
      mcpRuntimeArtifact: {sha256: $mcp_sha, bytes: $mcp_bytes},
      emulatedReportSha256: $emulated_report_sha
    },
    equivalence: {
      runtimeSourceUnchanged: true,
      dependencyGraphUnchanged: true,
      buildInputsUnchangedExceptRootVersion: true,
      deploymentLogicUnchanged: true,
      candidateArtifactsMatchBridge: true,
      officialTermuxNativeArm64GatePassed: true,
      termuxImageDigest: $image_digest,
      stressSamples: $samples,
      stressRequests: $requests
    }
  }' >"$REPORT_NEXT"
chmod 600 "$REPORT_NEXT"

jq -e '
  .schemaVersion == 1 and .verifierVersion == "1" and .status == "pass"
  and .releaseQualificationEligible == true
  and .evidenceMode == "inherited_physical_observation"
  and .sourceObservation.physicalDevice == true
  and .sourceObservation.status == "pass"
  and .sourceObservation.minutes >= 60
  and .equivalence.runtimeSourceUnchanged == true
  and .equivalence.dependencyGraphUnchanged == true
  and .equivalence.buildInputsUnchangedExceptRootVersion == true
  and .equivalence.deploymentLogicUnchanged == true
  and .equivalence.candidateArtifactsMatchBridge == true
  and .equivalence.officialTermuxNativeArm64GatePassed == true
' "$REPORT_NEXT" >/dev/null || fail generated_report_invalid

install -m 600 "$REPORT_NEXT" "$OUTPUT_REPORT" || fail report_publication_failed
rm -f -- "$REPORT_NEXT"

printf 'observation_inheritance_report_sha256=%s\n' "$(sha256sum "$OUTPUT_REPORT" | awk '{print $1}')"
printf 'observation_inheritance_report=%s\n' "$OUTPUT_REPORT"
printf 'OBSERVATION_INHERITANCE_RESULT=PASS\n'
