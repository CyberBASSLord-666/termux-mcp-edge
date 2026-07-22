#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077

usage() {
  cat <<'EOF'
Usage: stage_release_assets.sh \
  --default-dir DIR \
  --mcp-runtime-dir DIR \
  --android-battery-status-dir DIR \
  --android-volume-status-dir DIR \
  --android-volume-control-dir DIR \
  --command-execution-dir DIR \
  --full-suite-dir DIR \
  --emulated-evidence-dir DIR \
  --validator-evidence FILE \
  --physical-qualification FILE \
  --license FILE \
  --repository OWNER/REPO \
  --commit SHA \
  --version VERSION \
  --ci-run-id ID \
  --security-run-id ID \
  --android-run-id ID \
  --output termux-mcp-server-vVERSION-release-stage-SHA12.tar

This command only validates and stages already-built, already-qualified bytes.
It never compiles, tags, publishes, or calls a network service.
EOF
}

DEFAULT_DIR=""
MCP_RUNTIME_DIR=""
BATTERY_DIR=""
VOLUME_DIR=""
VOLUME_CONTROL_DIR=""
COMMAND_DIR=""
FULL_SUITE_DIR=""
EMULATED_EVIDENCE_DIR=""
VALIDATOR_EVIDENCE=""
PHYSICAL_QUALIFICATION=""
LICENSE_FILE=""
REPOSITORY=""
COMMIT=""
VERSION=""
CI_RUN_ID=""
SECURITY_RUN_ID=""
ANDROID_RUN_ID=""
OUTPUT=""
STAGING_DIR=""
COMPLETED=0

while (($# > 0)); do
  case "$1" in
    --default-dir) (($# >= 2)) || { usage >&2; exit 2; }; DEFAULT_DIR="$2"; shift 2 ;;
    --mcp-runtime-dir) (($# >= 2)) || { usage >&2; exit 2; }; MCP_RUNTIME_DIR="$2"; shift 2 ;;
    --android-battery-status-dir) (($# >= 2)) || { usage >&2; exit 2; }; BATTERY_DIR="$2"; shift 2 ;;
    --android-volume-status-dir) (($# >= 2)) || { usage >&2; exit 2; }; VOLUME_DIR="$2"; shift 2 ;;
    --android-volume-control-dir) (($# >= 2)) || { usage >&2; exit 2; }; VOLUME_CONTROL_DIR="$2"; shift 2 ;;
    --command-execution-dir) (($# >= 2)) || { usage >&2; exit 2; }; COMMAND_DIR="$2"; shift 2 ;;
    --full-suite-dir) (($# >= 2)) || { usage >&2; exit 2; }; FULL_SUITE_DIR="$2"; shift 2 ;;
    --emulated-evidence-dir) (($# >= 2)) || { usage >&2; exit 2; }; EMULATED_EVIDENCE_DIR="$2"; shift 2 ;;
    --validator-evidence) (($# >= 2)) || { usage >&2; exit 2; }; VALIDATOR_EVIDENCE="$2"; shift 2 ;;
    --physical-qualification) (($# >= 2)) || { usage >&2; exit 2; }; PHYSICAL_QUALIFICATION="$2"; shift 2 ;;
    --license) (($# >= 2)) || { usage >&2; exit 2; }; LICENSE_FILE="$2"; shift 2 ;;
    --repository) (($# >= 2)) || { usage >&2; exit 2; }; REPOSITORY="$2"; shift 2 ;;
    --commit) (($# >= 2)) || { usage >&2; exit 2; }; COMMIT="$2"; shift 2 ;;
    --version) (($# >= 2)) || { usage >&2; exit 2; }; VERSION="$2"; shift 2 ;;
    --ci-run-id) (($# >= 2)) || { usage >&2; exit 2; }; CI_RUN_ID="$2"; shift 2 ;;
    --security-run-id) (($# >= 2)) || { usage >&2; exit 2; }; SECURITY_RUN_ID="$2"; shift 2 ;;
    --android-run-id) (($# >= 2)) || { usage >&2; exit 2; }; ANDROID_RUN_ID="$2"; shift 2 ;;
    --output) (($# >= 2)) || { usage >&2; exit 2; }; OUTPUT="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done

fail() {
  printf '[release-stage] ERROR: %s\n' "$1" >&2
  exit 1
}

cleanup() {
  if ((COMPLETED == 0)) \
    && [[ -n "$STAGING_DIR" && -n "$OUTPUT" && "$STAGING_DIR" == "$OUTPUT.staging.$$" ]]; then
    rm -rf -- "$STAGING_DIR" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

for command_name in awk basename chmod cmp cp date dirname file find grep install jq mkdir mv realpath rm sha256sum sort stat tar wc; do
  command -v "$command_name" >/dev/null 2>&1 || fail required_command_missing
done

required_values=(
  "$DEFAULT_DIR" "$MCP_RUNTIME_DIR" "$BATTERY_DIR" "$VOLUME_DIR"
  "$VOLUME_CONTROL_DIR" "$COMMAND_DIR" "$FULL_SUITE_DIR"
  "$EMULATED_EVIDENCE_DIR" "$VALIDATOR_EVIDENCE"
  "$PHYSICAL_QUALIFICATION" "$LICENSE_FILE" "$REPOSITORY" "$COMMIT"
  "$VERSION" "$CI_RUN_ID" "$SECURITY_RUN_ID" "$ANDROID_RUN_ID" "$OUTPUT"
)
for required_value in "${required_values[@]}"; do
  [[ -n "$required_value" ]] || fail required_argument_missing
done

[[ "$REPOSITORY" == "CyberBASSLord-666/termux-mcp-edge" ]] || fail repository_invalid
[[ "$COMMIT" =~ ^[0-9a-f]{40}$ ]] || fail commit_invalid
[[ "$VERSION" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] || fail version_invalid
[[ "$CI_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail ci_run_id_invalid
[[ "$SECURITY_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail security_run_id_invalid
[[ "$ANDROID_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail android_run_id_invalid
[[ -n "$OUTPUT" && "$OUTPUT" != / && ! -e "$OUTPUT" && ! -L "$OUTPUT" ]] || fail output_invalid
expected_output_name="termux-mcp-server-v${VERSION}-release-stage-${COMMIT:0:12}.tar"
[[ "$(basename -- "$OUTPUT")" == "$expected_output_name" ]] || fail output_name_invalid
output_parent="$(dirname -- "$OUTPUT")"
[[ -d "$output_parent" && ! -L "$output_parent" ]] || fail output_parent_invalid

postures=(
  default
  mcp-runtime
  android-battery-status
  android-volume-status
  android-volume-control
  command-execution
  full-suite
)
bundle_dirs=(
  "$DEFAULT_DIR"
  "$MCP_RUNTIME_DIR"
  "$BATTERY_DIR"
  "$VOLUME_DIR"
  "$VOLUME_CONTROL_DIR"
  "$COMMAND_DIR"
  "$FULL_SUITE_DIR"
)
artifact_names=(
  termux-mcp-server-aarch64-linux-android-default
  termux-mcp-server-aarch64-linux-android-mcp-runtime
  termux-mcp-server-aarch64-linux-android-android-battery-status
  termux-mcp-server-aarch64-linux-android-android-volume-status
  termux-mcp-server-aarch64-linux-android-android-volume-control
  termux-mcp-server-aarch64-linux-android-command-execution
  termux-mcp-server-aarch64-linux-android-full-suite
)
features_json=(
  '[]'
  '["mcp-runtime"]'
  '["android-battery-status"]'
  '["android-volume-status"]'
  '["android-volume-control"]'
  '["command-execution"]'
  '["full-suite"]'
)
evidence_source_names=(
  termux-emulated-evidence.json
  termux-battery-emulated-evidence.json
  termux-volume-emulated-evidence.json
  termux-volume-control-emulated-evidence.json
  termux-command-emulated-evidence.json
  termux-observation-requirement.json
)
binary_sha=()
binary_bytes=()
manifest_sha=()

require_regular_file() {
  local path="$1" max_bytes="$2" error_code="$3" size
  [[ -f "$path" && ! -L "$path" ]] || fail "$error_code"
  size="$(stat -c '%s' -- "$path" 2>/dev/null)" || fail "$error_code"
  [[ "$size" =~ ^[0-9]+$ ]] || fail "$error_code"
  ((size > 0 && size <= max_bytes)) || fail "$error_code"
}

output_abs="$(realpath -m -- "$OUTPUT")" || fail output_resolution_failed
input_directories=("${bundle_dirs[@]}" "$EMULATED_EVIDENCE_DIR")
for input_directory in "${input_directories[@]}"; do
  [[ -d "$input_directory" && ! -L "$input_directory" ]] || fail input_directory_invalid
  input_abs="$(realpath -- "$input_directory")" || fail input_directory_resolution_failed
  case "$output_abs" in
    "$input_abs"|"$input_abs"/*) fail output_overlaps_input ;;
  esac
done

# Snapshot every caller-controlled input before validation. All subsequent
# checks and copies operate only on these private snapshots, so a later source
# replacement cannot change the validated release stage.
STAGING_DIR="$OUTPUT.staging.$$"
[[ ! -e "$STAGING_DIR" && ! -L "$STAGING_DIR" ]] || fail staging_directory_exists
PAYLOAD_DIR="$STAGING_DIR/payload"
WORK_DIR="$STAGING_DIR/work"
SNAPSHOT_DIR="$STAGING_DIR/input"
mkdir -m 700 -- "$STAGING_DIR" "$PAYLOAD_DIR" "$PAYLOAD_DIR/evidence" \
  "$WORK_DIR" "$SNAPSHOT_DIR" "$SNAPSHOT_DIR/bundles" "$SNAPSHOT_DIR/emulated" \
  || fail staging_directory_create_failed

source_bundle_dirs=("${bundle_dirs[@]}")
expected_bundle_entries=$'SHA256SUMS\nartifact-manifest.json\ntermux-mcp-server'
for index in "${!source_bundle_dirs[@]}"; do
  source_root="${source_bundle_dirs[$index]}"
  snapshot_root="$SNAPSHOT_DIR/bundles/${postures[$index]}"
  actual_entries="$(find "$source_root" -mindepth 1 -maxdepth 1 -printf '%f\n' 2>/dev/null | sort)" \
    || fail bundle_enumeration_failed
  [[ "$actual_entries" == "$expected_bundle_entries" ]] || fail bundle_members_invalid
  mkdir -m 700 -- "$snapshot_root" || fail input_snapshot_failed
  require_regular_file "$source_root/termux-mcp-server" 67108864 bundle_binary_invalid
  require_regular_file "$source_root/SHA256SUMS" 256 bundle_checksum_invalid
  require_regular_file "$source_root/artifact-manifest.json" 65536 bundle_manifest_invalid
  cp -P -- "$source_root/termux-mcp-server" "$snapshot_root/termux-mcp-server" \
    || fail input_snapshot_failed
  cp -P -- "$source_root/SHA256SUMS" "$snapshot_root/SHA256SUMS" \
    || fail input_snapshot_failed
  cp -P -- "$source_root/artifact-manifest.json" "$snapshot_root/artifact-manifest.json" \
    || fail input_snapshot_failed
  bundle_dirs[$index]="$snapshot_root"
done

expected_evidence_entries="$(printf '%s\n' "${evidence_source_names[@]}" | sort)"
actual_evidence_entries="$(find "$EMULATED_EVIDENCE_DIR" -mindepth 1 -maxdepth 1 -printf '%f\n' 2>/dev/null | sort)" \
  || fail emulated_evidence_enumeration_failed
[[ "$actual_evidence_entries" == "$expected_evidence_entries" ]] \
  || fail emulated_evidence_members_invalid
for evidence_name in "${evidence_source_names[@]}"; do
  source_evidence="$EMULATED_EVIDENCE_DIR/$evidence_name"
  require_regular_file "$source_evidence" 1048576 emulated_evidence_file_invalid
  cp -P -- "$source_evidence" "$SNAPSHOT_DIR/emulated/$evidence_name" \
    || fail input_snapshot_failed
done
EMULATED_EVIDENCE_DIR="$SNAPSHOT_DIR/emulated"

require_regular_file "$VALIDATOR_EVIDENCE" 1048576 validator_evidence_invalid
require_regular_file "$PHYSICAL_QUALIFICATION" 65536 physical_qualification_invalid
require_regular_file "$LICENSE_FILE" 1048576 license_invalid
cp -P -- "$VALIDATOR_EVIDENCE" "$SNAPSHOT_DIR/release-validator-v11.json" \
  || fail input_snapshot_failed
cp -P -- "$PHYSICAL_QUALIFICATION" "$SNAPSHOT_DIR/physical-qualification-v1.json" \
  || fail input_snapshot_failed
cp -P -- "$LICENSE_FILE" "$SNAPSHOT_DIR/LICENSE" || fail input_snapshot_failed
VALIDATOR_EVIDENCE="$SNAPSHOT_DIR/release-validator-v11.json"
PHYSICAL_QUALIFICATION="$SNAPSHOT_DIR/physical-qualification-v1.json"
LICENSE_FILE="$SNAPSHOT_DIR/LICENSE"

verify_bundle() {
  local index="$1" root posture artifact_name features
  local actual_entries expected_entries binary checksum manifest
  local bytes digest identity checksum_line checksum_lines manifest_digest
  root="${bundle_dirs[$index]}"
  posture="${postures[$index]}"
  artifact_name="${artifact_names[$index]}"
  features="${features_json[$index]}"

  expected_entries=$'SHA256SUMS\nartifact-manifest.json\ntermux-mcp-server'
  actual_entries="$(find "$root" -mindepth 1 -maxdepth 1 -printf '%f\n' 2>/dev/null | sort)" \
    || fail bundle_enumeration_failed
  [[ "$actual_entries" == "$expected_entries" ]] || fail bundle_members_invalid

  binary="$root/termux-mcp-server"
  checksum="$root/SHA256SUMS"
  manifest="$root/artifact-manifest.json"
  require_regular_file "$binary" 67108864 bundle_binary_invalid
  require_regular_file "$checksum" 256 bundle_checksum_invalid
  require_regular_file "$manifest" 65536 bundle_manifest_invalid

  bytes="$(stat -c '%s' -- "$binary" 2>/dev/null)" || fail bundle_binary_stat_failed
  digest="$(sha256sum -- "$binary" 2>/dev/null | awk '{print $1}')" || fail bundle_binary_digest_failed
  [[ "$digest" =~ ^[0-9a-f]{64}$ ]] || fail bundle_binary_digest_failed
  identity="$(file -b -- "$binary" 2>/dev/null)" || fail bundle_binary_identity_failed
  [[ "$identity" == *ELF* && "$identity" == *"ARM aarch64"* ]] || fail bundle_binary_architecture_mismatch
  [[ "$identity" == *Android* || "$identity" == *"/system/bin/linker64"* ]] \
    || fail bundle_binary_android_identity_missing

  checksum_lines="$(awk 'END {print NR}' "$checksum")" || fail bundle_checksum_invalid
  [[ "$checksum_lines" == 1 ]] || fail bundle_checksum_invalid
  checksum_line="$(<"$checksum")"
  [[ "$checksum_line" == "$digest  termux-mcp-server" ]] || fail bundle_checksum_mismatch
  (cd "$root" && sha256sum -c SHA256SUMS >/dev/null 2>&1) || fail bundle_checksum_mismatch

  jq -e \
    --arg repository "$REPOSITORY" \
    --arg commit "$COMMIT" \
    --arg run_id "$ANDROID_RUN_ID" \
    --arg artifact_name "$artifact_name" \
    --arg posture "$posture" \
    --arg version "$VERSION" \
    --arg sha "$digest" \
    --argjson bytes "$bytes" \
    --argjson features "$features" '
      (keys == ["artifactName","bytes","commit","createdAt","elf","features","fileName","posture","repository","schemaVersion","sha256","target","version","workflowRunId"])
      and .schemaVersion == 1
      and .repository == $repository
      and .commit == $commit
      and .workflowRunId == $run_id
      and .artifactName == $artifact_name
      and .posture == $posture
      and .features == $features
      and .target == "aarch64-linux-android"
      and .fileName == "termux-mcp-server"
      and .version == $version
      and .sha256 == $sha
      and .bytes == $bytes
      and .elf == "aarch64-android-elf"
      and (.createdAt | type == "string" and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"))
    ' "$manifest" >/dev/null 2>&1 || fail bundle_manifest_mismatch

  manifest_digest="$(sha256sum -- "$manifest" | awk '{print $1}')" || fail bundle_manifest_digest_failed
  [[ "$manifest_digest" =~ ^[0-9a-f]{64}$ ]] || fail bundle_manifest_digest_failed
  binary_sha[$index]="$digest"
  binary_bytes[$index]="$bytes"
  manifest_sha[$index]="$manifest_digest"
}

for index in "${!postures[@]}"; do
  verify_bundle "$index"
done
unique_binary_digests="$(printf '%s\n' "${binary_sha[@]}" | sort -u | awk 'END {print NR}')"
[[ "$unique_binary_digests" == 7 ]] || fail bundle_posture_digests_not_distinct

expected_evidence_entries="$(printf '%s\n' "${evidence_source_names[@]}" | sort)"
actual_evidence_entries="$(find "$EMULATED_EVIDENCE_DIR" -mindepth 1 -maxdepth 1 -printf '%f\n' 2>/dev/null | sort)" \
  || fail emulated_evidence_enumeration_failed
[[ "$actual_evidence_entries" == "$expected_evidence_entries" ]] || fail emulated_evidence_members_invalid
for evidence_name in "${evidence_source_names[@]}"; do
  evidence_path="$EMULATED_EVIDENCE_DIR/$evidence_name"
  require_regular_file "$evidence_path" 1048576 emulated_evidence_file_invalid
  jq -e . "$evidence_path" >/dev/null 2>&1 || fail emulated_evidence_json_invalid
done

AGGREGATE_EVIDENCE="$EMULATED_EVIDENCE_DIR/termux-emulated-evidence.json"
BATTERY_EVIDENCE="$EMULATED_EVIDENCE_DIR/termux-battery-emulated-evidence.json"
VOLUME_EVIDENCE="$EMULATED_EVIDENCE_DIR/termux-volume-emulated-evidence.json"
VOLUME_CONTROL_EVIDENCE="$EMULATED_EVIDENCE_DIR/termux-volume-control-emulated-evidence.json"
COMMAND_EVIDENCE="$EMULATED_EVIDENCE_DIR/termux-command-emulated-evidence.json"
OBSERVATION_REQUIREMENT="$EMULATED_EVIDENCE_DIR/termux-observation-requirement.json"
aggregate_sha="$(sha256sum -- "$AGGREGATE_EVIDENCE" | awk '{print $1}')" || fail aggregate_evidence_digest_failed

jq -e \
  --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg default_sha "${binary_sha[0]}" --argjson default_bytes "${binary_bytes[0]}" \
  --arg mcp_sha "${binary_sha[1]}" --argjson mcp_bytes "${binary_bytes[1]}" \
  --arg volume_control_sha "${binary_sha[4]}" --argjson volume_control_bytes "${binary_bytes[4]}" \
  --arg full_suite_sha "${binary_sha[6]}" --argjson full_suite_bytes "${binary_bytes[6]}" \
  --arg full_suite_manifest_sha "${manifest_sha[6]}" '
    (keys == ["aggregateValidation","candidate","completedAt","environment","failureCode","gateVersion","releaseQualificationEligible","runtimeValidation","schemaVersion","startedAt","status","stress"])
    and .schemaVersion == 3
    and .gateVersion == "3"
    and .status == "pass"
    and .failureCode == null
    and .releaseQualificationEligible == false
    and (.candidate | keys == ["androidRunId","androidVolumeControlArtifact","ciRunId","commit","defaultArtifact","fullSuiteArtifact","mcpRuntimeArtifact","securityRunId","version"])
    and .candidate.commit == $commit
    and .candidate.version == $version
    and .candidate.ciRunId == $ci
    and .candidate.securityRunId == $security
    and .candidate.androidRunId == $android
    and .candidate.defaultArtifact == {sha256:$default_sha, bytes:$default_bytes}
    and .candidate.mcpRuntimeArtifact == {sha256:$mcp_sha, bytes:$mcp_bytes}
    and .candidate.androidVolumeControlArtifact == {sha256:$volume_control_sha, bytes:$volume_control_bytes}
    and .candidate.fullSuiteArtifact.sha256 == $full_suite_sha
    and .candidate.fullSuiteArtifact.bytes == $full_suite_bytes
    and .candidate.fullSuiteArtifact.manifestSha256 == $full_suite_manifest_sha
    and .candidate.fullSuiteArtifact.artifactName == "termux-mcp-server-aarch64-linux-android-full-suite"
    and .candidate.fullSuiteArtifact.posture == "full-suite"
    and .candidate.fullSuiteArtifact.features == ["full-suite"]
    and .candidate.fullSuiteArtifact.fileName == "termux-mcp-server"
    and .environment.executionMode == "official-termux-docker-native-arm64"
    and (.environment.architecture == "aarch64" or .environment.architecture == "arm64")
    and .environment.androidLinker == true
    and .runtimeValidation.status == "pass"
    and .runtimeValidation.phases.preflight == "pass"
    and .runtimeValidation.phases.runtime == "pass"
    and .aggregateValidation.status == "pass"
    and .aggregateValidation.requests >= 14
    and .aggregateValidation.defaultDisabled.toolCount == 17
    and .aggregateValidation.defaultDisabled.exactToolOrder == true
    and .aggregateValidation.defaultDisabled.optionalFeaturesCompiled == true
    and .aggregateValidation.defaultDisabled.optionalToolsHidden == true
    and .aggregateValidation.defaultDisabled.runtimeFlagsOmitted == true
    and .aggregateValidation.fullyEnabled.toolCount == 21
    and .aggregateValidation.fullyEnabled.exactToolOrder == true
    and .aggregateValidation.fullyEnabled.allOptionalToolsExposed == true
    and .aggregateValidation.fullyEnabled.providerSuccesses == true
    and .aggregateValidation.fullyEnabled.volumePreviewNoMutation == true
    and .aggregateValidation.fullyEnabled.volumeGrantIsolation == true
    and .aggregateValidation.fullyEnabled.commandExecutableIdentityPinned == true
    and .aggregateValidation.independentRuntimeGates == true
    and .aggregateValidation.filesystemMutationsDisabled == true
    and .aggregateValidation.boundedCleanup == true
    and .aggregateValidation.directPhysicalObservationRequired == true
    and .stress.status == "pass"
    and .stress.servicePidStable == true
    and .stress.healthReadyStable == true
    and .stress.longObservationRequired == false
  ' "$AGGREGATE_EVIDENCE" >/dev/null 2>&1 || fail aggregate_evidence_mismatch

verify_specialized_evidence() {
  local path="$1" mode="$2" schema_version="$3" gate_version="$4"
  local artifact_index="$5" related_index="${6:--1}"
  local artifact_sha_value="${binary_sha[$artifact_index]}"
  local artifact_bytes_value="${binary_bytes[$artifact_index]}"
  local related_sha="" related_bytes=0
  if ((related_index >= 0)); then
    related_sha="${binary_sha[$related_index]}"
    related_bytes="${binary_bytes[$related_index]}"
  fi
  jq -e \
    --arg mode "$mode" --argjson schema "$schema_version" --arg gate "$gate_version" \
    --arg commit "$COMMIT" --arg version "$VERSION" \
    --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
    --arg artifact_sha "$artifact_sha_value" --argjson artifact_bytes "$artifact_bytes_value" \
    --arg related_sha "$related_sha" --argjson related_bytes "$related_bytes" '
      (keys == ["candidate","completedAt","environment","failureCode","gateVersion","releaseQualificationEligible","schemaVersion","startedAt","status","validation"])
      and .schemaVersion == $schema
      and .gateVersion == $gate
      and .status == "pass"
      and .failureCode == null
      and .releaseQualificationEligible == false
      and .candidate.commit == $commit
      and .candidate.version == $version
      and .candidate.ciRunId == $ci
      and .candidate.securityRunId == $security
      and .candidate.androidRunId == $android
      and .candidate.artifact == {sha256:$artifact_sha, bytes:$artifact_bytes}
      and (if $mode == "volume-control" then
             (.candidate | keys == ["androidRunId","artifact","ciRunId","commit","incompatibleArtifact","securityRunId","version"])
             and .candidate.incompatibleArtifact == {sha256:$related_sha, bytes:$related_bytes}
           elif $mode == "command" then
             (.candidate | keys == ["androidRunId","artifact","ciRunId","commit","defaultArtifact","securityRunId","version"])
             and .candidate.defaultArtifact == {sha256:$related_sha, bytes:$related_bytes}
           else
             (.candidate | keys == ["androidRunId","artifact","ciRunId","commit","securityRunId","version"])
           end)
      and .environment.executionMode == "official-termux-docker-native-arm64"
      and (.environment.architecture == "aarch64" or .environment.architecture == "arm64")
      and .validation.status == "pass"
      and .validation.requests >= 1
      and .validation.exactArtifact == true
      and ([.validation | to_entries[] | select(.value | type == "boolean") | select(.key != "longObservationRequired") | .value] | all(. == true))
      and (if (.validation | has("longObservationRequired")) then .validation.longObservationRequired == false else true end)
    ' "$path" >/dev/null 2>&1 || fail specialized_evidence_mismatch
}

verify_specialized_evidence "$BATTERY_EVIDENCE" battery 2 2 2
verify_specialized_evidence "$VOLUME_EVIDENCE" volume 1 1 3
verify_specialized_evidence "$VOLUME_CONTROL_EVIDENCE" volume-control 1 1 4 3
verify_specialized_evidence "$COMMAND_EVIDENCE" command 2 2 5 0

jq -e \
  --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg full_suite_sha "${binary_sha[6]}" --arg full_suite_manifest_sha "${manifest_sha[6]}" \
  --arg aggregate_sha "$aggregate_sha" '
    (keys == ["candidate","changedInputClasses","classifierVersion","createdAt","emulation","evidenceMode","failureCode","inheritanceCandidate","nextGate","protectedInputComparison","reasonCode","releaseQualificationEligible","schemaVersion","source","status"])
    and .schemaVersion == 2
    and .classifierVersion == "2"
    and .status == "pass"
    and .failureCode == null
    and .releaseQualificationEligible == false
    and .evidenceMode == "physical_observation_required"
    and .reasonCode == "full_suite_direct_physical_observation_required"
    and .inheritanceCandidate == false
    and .nextGate == "direct_physical_device_observation"
    and (.candidate | keys == ["androidRunId","ciRunId","commit","fullSuiteArtifactSha256","fullSuiteManifestSha256","securityRunId","version"])
    and .candidate.commit == $commit
    and .candidate.version == $version
    and .candidate.ciRunId == $ci
    and .candidate.securityRunId == $security
    and .candidate.androidRunId == $android
    and .candidate.fullSuiteArtifactSha256 == $full_suite_sha
    and .candidate.fullSuiteManifestSha256 == $full_suite_manifest_sha
    and .emulation.reportSha256 == $aggregate_sha
    and .emulation.status == "pass"
    and .emulation.executionMode == "official-termux-docker-native-arm64"
    and (.changedInputClasses | index("full_suite_artifact") != null)
  ' "$OBSERVATION_REQUIREMENT" >/dev/null 2>&1 || fail observation_requirement_mismatch

require_regular_file "$VALIDATOR_EVIDENCE" 1048576 validator_evidence_invalid
jq -e . "$VALIDATOR_EVIDENCE" >/dev/null 2>&1 || fail validator_evidence_json_invalid
validator_sha="$(sha256sum -- "$VALIDATOR_EVIDENCE" | awk '{print $1}')" || fail validator_evidence_digest_failed
jq -e \
  --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg default_sha "${binary_sha[0]}" --argjson default_bytes "${binary_bytes[0]}" \
  --arg mcp_sha "${binary_sha[1]}" --argjson mcp_bytes "${binary_bytes[1]}" \
  --arg volume_control_sha "${binary_sha[4]}" --argjson volume_control_bytes "${binary_bytes[4]}" \
  --arg full_suite_sha "${binary_sha[6]}" --argjson full_suite_bytes "${binary_bytes[6]}" '
    def artifact:
      type == "object"
      and (keys == ["bytes","elf","sha256","version"])
      and (.sha256 | type == "string" and test("^[0-9a-f]{64}$"))
      and (.bytes | type == "number" and floor == . and . >= 1 and . <= 67108864)
      and (.version | type == "string" and test("^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$"))
      and .elf == "aarch64-android-elf";
    def passed($code):
      any(.results[]; .code == $code and .outcome == "pass");
    (keys == ["artifacts","completedAt","deploymentCandidate","environment","failureCode","phases","releaseEligible","repository","requestedPhase","results","schemaVersion","startedAt","status","sustainedObservation","validatorVersion"])
    and .schemaVersion == 2
    and .validatorVersion == "11"
    and .status == "pass"
    and .failureCode == null
    and .releaseEligible == true
    and (.startedAt | type == "string" and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"))
    and (.completedAt | type == "string" and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"))
    and .requestedPhase == "all"
    and (.repository | keys == ["androidRunId","ciRunId","commit","securityRunId","version"])
    and .repository.commit == $commit
    and .repository.version == $version
    and .repository.ciRunId == $ci
    and .repository.securityRunId == $security
    and .repository.androidRunId == $android
    and (.environment | type == "object" and keys == ["architecture","fixtureMode","tools"])
    and (.environment.architecture == "aarch64" or .environment.architecture == "arm64")
    and .environment.fixtureMode == false
    and (.environment.tools | type == "object" and keys == ["bash","curl","file","jq"])
    and all(.environment.tools[];
      type == "string" and length >= 1 and length <= 256
      and (contains("\u0000") | not)
      and (contains("\n") | not)
      and (contains("\r") | not))
    and (.environment.tools.jq | length <= 64)
    and .phases == {preflight:"pass", runtime:"pass", deployment:"pass"}
    and .deploymentCandidate == {posture:"full-suite", productionAction:null}
    and (.artifacts | keys == ["androidVolumeControl","baseline","default","fullSuite","mcpRuntime"])
    and .artifacts.default == {sha256:$default_sha, bytes:$default_bytes, version:$version, elf:"aarch64-android-elf"}
    and .artifacts.mcpRuntime == {sha256:$mcp_sha, bytes:$mcp_bytes, version:$version, elf:"aarch64-android-elf"}
    and .artifacts.androidVolumeControl == {sha256:$volume_control_sha, bytes:$volume_control_bytes, version:$version, elf:"aarch64-android-elf"}
    and .artifacts.fullSuite == {sha256:$full_suite_sha, bytes:$full_suite_bytes, version:$version, elf:"aarch64-android-elf"}
    and (.artifacts.baseline | artifact)
    and .artifacts.baseline.version != $version
    and ([
      .artifacts.default.sha256,
      .artifacts.mcpRuntime.sha256,
      .artifacts.androidVolumeControl.sha256,
      .artifacts.fullSuite.sha256,
      .artifacts.baseline.sha256
    ] | unique | length == 5)
    and (.results | type == "array" and length >= 1 and length <= 256)
    and all(.results[];
      type == "object"
      and (keys == ["check","code","outcome","phase"])
      and (.phase == "preflight" or .phase == "runtime" or .phase == "deployment")
      and (.check | type == "string" and length <= 96 and test("^[a-z0-9_]+$"))
      and (.outcome == "pass" or .outcome == "fail" or .outcome == "info")
      and (.code | type == "string" and length <= 96 and test("^[a-z0-9_]+$")))
    and all(.results[]; .outcome != "fail")
    and passed("full_suite_default_disabled_17_tool_posture_verified")
    and passed("full_suite_battery_runtime_gate_independence_verified")
    and passed("full_suite_volume_status_runtime_gate_independence_verified")
    and passed("full_suite_volume_control_runtime_gate_independence_verified")
    and passed("full_suite_command_runtime_gate_independence_verified")
    and passed("full_suite_enabled_21_tool_posture_verified")
    and passed("full_suite_optional_provider_success_verified")
    and passed("full_suite_volume_preview_and_grant_boundary_verified")
    and passed("full_suite_command_basename_and_profile_verified")
    and passed("full_suite_filesystem_mutations_independently_disabled")
    and passed("full_suite_deployment_candidate_selected")
    and (.sustainedObservation | type == "object" and keys == ["minimumMinutes","minutes","operatorSupplied","reasonCode","status"])
    and .sustainedObservation.operatorSupplied == true
    and .sustainedObservation.status == "pass"
    and (.sustainedObservation.minutes | type == "number" and floor == . and . >= 60 and . <= 10080)
    and .sustainedObservation.minimumMinutes == 60
    and .sustainedObservation.reasonCode == "stable"
  ' "$VALIDATOR_EVIDENCE" >/dev/null 2>&1 || fail validator_evidence_mismatch

validator_started_at="$(jq -r '.startedAt' "$VALIDATOR_EVIDENCE")" \
  || fail validator_evidence_mismatch
validator_completed_at="$(jq -r '.completedAt' "$VALIDATOR_EVIDENCE")" \
  || fail validator_evidence_mismatch
validator_started_epoch="$(date -u -d "$validator_started_at" '+%s' 2>/dev/null)" \
  || fail validator_evidence_mismatch
validator_completed_epoch="$(date -u -d "$validator_completed_at" '+%s' 2>/dev/null)" \
  || fail validator_evidence_mismatch
[[ "$(date -u -d "@$validator_started_epoch" '+%Y-%m-%dT%H:%M:%SZ' 2>/dev/null)" == "$validator_started_at" ]] \
  || fail validator_evidence_mismatch
[[ "$(date -u -d "@$validator_completed_epoch" '+%Y-%m-%dT%H:%M:%SZ' 2>/dev/null)" == "$validator_completed_at" ]] \
  || fail validator_evidence_mismatch
((validator_completed_epoch >= validator_started_epoch)) || fail validator_evidence_mismatch

require_regular_file "$PHYSICAL_QUALIFICATION" 65536 physical_qualification_invalid
jq -e . "$PHYSICAL_QUALIFICATION" >/dev/null 2>&1 || fail physical_qualification_json_invalid
jq -e \
  --arg repository "$REPOSITORY" --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg validator_sha "$validator_sha" --arg full_suite_sha "${binary_sha[6]}" '
    (keys == ["androidRunId","architecture","ciRunId","cleanupConfirmed","commit","envelopeVersion","failureCode","harnessPassed","harnessVersion","nativeFullSuiteSha256","rawHarnessReportSha256","releaseEligible","repository","schemaVersion","securityRunId","status","validatorReportSha256","validatorVersion","version","workflowFullSuiteSha256"])
    and .schemaVersion == 1
    and .envelopeVersion == "1"
    and .status == "pass"
    and .failureCode == null
    and .releaseEligible == true
    and .repository == $repository
    and .commit == $commit
    and .version == $version
    and .ciRunId == $ci
    and .securityRunId == $security
    and .androidRunId == $android
    and .validatorVersion == "11"
    and .harnessVersion == "11"
    and .architecture == "aarch64"
    and .validatorReportSha256 == $validator_sha
    and (.rawHarnessReportSha256 | type == "string" and test("^[0-9a-f]{64}$"))
    and .workflowFullSuiteSha256 == $full_suite_sha
    and (.nativeFullSuiteSha256 | type == "string" and test("^[0-9a-f]{64}$"))
    and .harnessPassed == true
    and .cleanupConfirmed == true
  ' "$PHYSICAL_QUALIFICATION" >/dev/null 2>&1 || fail physical_qualification_mismatch

require_regular_file "$LICENSE_FILE" 1048576 license_invalid

: >"$WORK_DIR/artifact-records.jsonl"
: >"$WORK_DIR/specialized-evidence-records.jsonl"
: >"$PAYLOAD_DIR/SHA256SUMS"

for index in "${!postures[@]}"; do
  posture="${postures[$index]}"
  release_name="termux-mcp-server-v${VERSION}-aarch64-linux-android-${posture}"
  checksum_name="$release_name.sha256"
  workflow_manifest_name="$release_name.workflow-manifest.json"
  install -m 755 -- "${bundle_dirs[$index]}/termux-mcp-server" "$PAYLOAD_DIR/$release_name" \
    || fail binary_copy_failed
  cmp -s -- "${bundle_dirs[$index]}/termux-mcp-server" "$PAYLOAD_DIR/$release_name" \
    || fail binary_copy_mismatch
  [[ "$(sha256sum -- "$PAYLOAD_DIR/$release_name" | awk '{print $1}')" == "${binary_sha[$index]}" ]] \
    || fail binary_copy_digest_mismatch
  printf '%s  %s\n' "${binary_sha[$index]}" "$release_name" >"$PAYLOAD_DIR/$checksum_name" \
    || fail checksum_write_failed
  printf '%s  %s\n' "${binary_sha[$index]}" "$release_name" >>"$PAYLOAD_DIR/SHA256SUMS" \
    || fail checksum_write_failed
  install -m 644 -- "${bundle_dirs[$index]}/artifact-manifest.json" "$PAYLOAD_DIR/$workflow_manifest_name" \
    || fail manifest_copy_failed
  cmp -s -- "${bundle_dirs[$index]}/artifact-manifest.json" "$PAYLOAD_DIR/$workflow_manifest_name" \
    || fail manifest_copy_mismatch
  jq -cn \
    --arg posture "$posture" \
    --argjson features "${features_json[$index]}" \
    --arg workflow_artifact_name "${artifact_names[$index]}" \
    --arg workflow_manifest_file_name "$workflow_manifest_name" \
    --arg workflow_manifest_sha256 "${manifest_sha[$index]}" \
    --arg release_file_name "$release_name" \
    --arg checksum_file_name "$checksum_name" \
    --arg sha256 "${binary_sha[$index]}" \
    --argjson bytes "${binary_bytes[$index]}" '
      {
        posture: $posture,
        features: $features,
        workflowArtifactName: $workflow_artifact_name,
        workflowFileName: "termux-mcp-server",
        workflowManifestFileName: $workflow_manifest_file_name,
        workflowManifestSha256: $workflow_manifest_sha256,
        releaseFileName: $release_file_name,
        checksumFileName: $checksum_file_name,
        sha256: $sha256,
        bytes: $bytes,
        elf: "aarch64-android-elf"
      }
    ' >>"$WORK_DIR/artifact-records.jsonl" || fail manifest_record_write_failed
done

copy_evidence() {
  local source="$1" destination_name="$2" record_file="$3"
  local destination="$PAYLOAD_DIR/$destination_name" digest bytes
  install -m 644 -- "$source" "$destination" || fail evidence_copy_failed
  cmp -s -- "$source" "$destination" || fail evidence_copy_mismatch
  digest="$(sha256sum -- "$destination" | awk '{print $1}')" || fail evidence_digest_failed
  bytes="$(stat -c '%s' -- "$destination")" || fail evidence_stat_failed
  jq -cn --arg file_name "$destination_name" --arg sha256 "$digest" --argjson bytes "$bytes" \
    '{fileName:$file_name, sha256:$sha256, bytes:$bytes}' >"$record_file" \
    || fail evidence_record_write_failed
}

copy_evidence "$AGGREGATE_EVIDENCE" evidence/emulated-release-v3.json "$WORK_DIR/aggregate-record.json"
copy_evidence "$VALIDATOR_EVIDENCE" evidence/release-validator-v11.json "$WORK_DIR/validator-record.json"
copy_evidence "$PHYSICAL_QUALIFICATION" evidence/physical-qualification-v1.json "$WORK_DIR/physical-record.json"

specialized_sources=(
  "$BATTERY_EVIDENCE"
  "$VOLUME_EVIDENCE"
  "$VOLUME_CONTROL_EVIDENCE"
  "$COMMAND_EVIDENCE"
  "$OBSERVATION_REQUIREMENT"
)
specialized_destinations=(
  evidence/android-battery-emulated-v2.json
  evidence/android-volume-emulated-v1.json
  evidence/android-volume-control-emulated-v1.json
  evidence/command-emulated-v2.json
  evidence/release-observation-requirement-v2.json
)
for index in "${!specialized_sources[@]}"; do
  copy_evidence "${specialized_sources[$index]}" "${specialized_destinations[$index]}" \
    "$WORK_DIR/specialized-record.json"
  jq -c . "$WORK_DIR/specialized-record.json" >>"$WORK_DIR/specialized-evidence-records.jsonl" \
    || fail evidence_record_write_failed
done

install -m 644 -- "$LICENSE_FILE" "$PAYLOAD_DIR/LICENSE" || fail license_copy_failed
cmp -s -- "$LICENSE_FILE" "$PAYLOAD_DIR/LICENSE" || fail license_copy_mismatch
license_sha="$(sha256sum -- "$PAYLOAD_DIR/LICENSE" | awk '{print $1}')" || fail license_digest_failed
license_bytes="$(stat -c '%s' -- "$PAYLOAD_DIR/LICENSE")" || fail license_stat_failed
jq -cn --arg sha256 "$license_sha" --argjson bytes "$license_bytes" \
  '{fileName:"LICENSE", sha256:$sha256, bytes:$bytes}' >"$WORK_DIR/license-record.json" \
  || fail license_record_write_failed

jq -S -n \
  --arg repository "$REPOSITORY" --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --slurpfile artifacts "$WORK_DIR/artifact-records.jsonl" \
  --slurpfile aggregate "$WORK_DIR/aggregate-record.json" \
  --slurpfile validator "$WORK_DIR/validator-record.json" \
  --slurpfile physical "$WORK_DIR/physical-record.json" \
  --slurpfile specialized "$WORK_DIR/specialized-evidence-records.jsonl" \
  --slurpfile license "$WORK_DIR/license-record.json" '
    {
      schemaVersion: 1,
      publicationState: "staged_not_released",
      releaseEligible: false,
      repository: $repository,
      commit: $commit,
      version: $version,
      target: "aarch64-linux-android",
      workflowRuns: {ci:$ci, security:$security, android:$android},
      checksums: {algorithm:"sha256", combinedFileName:"SHA256SUMS"},
      license: $license[0],
      evidence: {
        aggregate: $aggregate[0],
        validator: $validator[0],
        physicalQualification: $physical[0],
        specialized: $specialized
      },
      artifacts: $artifacts
    }
  ' >"$PAYLOAD_DIR/release-staging-manifest-v1.json" || fail staging_manifest_write_failed

chmod 644 "$PAYLOAD_DIR/SHA256SUMS" "$PAYLOAD_DIR"/*.sha256 \
  "$PAYLOAD_DIR/release-staging-manifest-v1.json" || fail metadata_mode_failed

jq -e \
  --arg repository "$REPOSITORY" --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" '
    (keys == ["artifacts","checksums","commit","evidence","license","publicationState","releaseEligible","repository","schemaVersion","target","version","workflowRuns"])
    and .schemaVersion == 1
    and .publicationState == "staged_not_released"
    and .releaseEligible == false
    and .repository == $repository
    and .commit == $commit
    and .version == $version
    and .target == "aarch64-linux-android"
    and .workflowRuns == {ci:$ci, security:$security, android:$android}
    and .checksums == {algorithm:"sha256", combinedFileName:"SHA256SUMS"}
    and (.artifacts | length == 7)
    and ([.artifacts[].posture] == ["default","mcp-runtime","android-battery-status","android-volume-status","android-volume-control","command-execution","full-suite"])
    and (.evidence.specialized | length == 5)
  ' "$PAYLOAD_DIR/release-staging-manifest-v1.json" >/dev/null 2>&1 \
  || fail staging_manifest_verification_failed

(cd "$PAYLOAD_DIR" && sha256sum -c SHA256SUMS >/dev/null 2>&1) || fail combined_checksum_verification_failed
for index in "${!postures[@]}"; do
  release_name="termux-mcp-server-v${VERSION}-aarch64-linux-android-${postures[$index]}"
  (cd "$PAYLOAD_DIR" && sha256sum -c "$release_name.sha256" >/dev/null 2>&1) \
    || fail per_file_checksum_verification_failed
done

find "$PAYLOAD_DIR" -type d -exec chmod 755 {} + || fail directory_mode_failed
if find "$PAYLOAD_DIR" -type l -print -quit | grep -q .; then
  fail staged_link_detected
fi
if find "$PAYLOAD_DIR" ! -type f ! -type d -print -quit | grep -q .; then
  fail staged_special_file_detected
fi

TEMP_TAR="$STAGING_DIR/$expected_output_name"
tar --format=gnu --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner \
  --mode='u+rwX,go+rX,go-w' -C "$PAYLOAD_DIR" -cf "$TEMP_TAR" . \
  || fail deterministic_archive_failed
chmod 600 "$TEMP_TAR" || fail archive_mode_failed
archive_sha="$(sha256sum -- "$TEMP_TAR" | awk '{print $1}')" || fail archive_digest_failed
[[ "$archive_sha" =~ ^[0-9a-f]{64}$ ]] || fail archive_digest_failed
mv -T --no-clobber -- "$TEMP_TAR" "$OUTPUT" || fail archive_publication_failed
[[ ! -e "$TEMP_TAR" && ! -L "$TEMP_TAR" ]] || fail archive_publication_conflict
[[ -f "$OUTPUT" && ! -L "$OUTPUT" ]] || fail archive_publication_failed
[[ "$(sha256sum -- "$OUTPUT" | awk '{print $1}')" == "$archive_sha" ]] \
  || fail archive_publication_digest_mismatch
rm -rf -- "$STAGING_DIR" >/dev/null 2>&1 || true
STAGING_DIR=""
COMPLETED=1
printf '[release-stage] publicationState=staged_not_released releaseEligible=false archiveSha256=%s\n' "$archive_sha"
