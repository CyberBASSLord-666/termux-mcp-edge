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
  --deployment-evidence FILE \
  --automated-qualification FILE \
  --runtime-archive termux-qualified-runtime-image-v1.tar.gz \
  --runtime-package-lock termux-runtime-package-lock-v1.json \
  --runtime-snapshot termux-runtime-snapshot-v1.json \
  --runtime-replay termux-runtime-snapshot-replay-v1.json \
  --license FILE \
  --repository OWNER/REPO \
  --commit SHA \
  --version VERSION \
  --ci-run-id ID \
  --security-run-id ID \
  --android-run-id ID \
  --qualification-run-id ID \
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
DEPLOYMENT_EVIDENCE=""
AUTOMATED_QUALIFICATION=""
RUNTIME_ARCHIVE=""
RUNTIME_PACKAGE_LOCK=""
RUNTIME_SNAPSHOT=""
RUNTIME_REPLAY=""
LICENSE_FILE=""
REPOSITORY=""
COMMIT=""
VERSION=""
CI_RUN_ID=""
SECURITY_RUN_ID=""
ANDROID_RUN_ID=""
QUALIFICATION_RUN_ID=""
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
    --deployment-evidence) (($# >= 2)) || { usage >&2; exit 2; }; DEPLOYMENT_EVIDENCE="$2"; shift 2 ;;
    --automated-qualification) (($# >= 2)) || { usage >&2; exit 2; }; AUTOMATED_QUALIFICATION="$2"; shift 2 ;;
    --runtime-archive) (($# >= 2)) || { usage >&2; exit 2; }; RUNTIME_ARCHIVE="$2"; shift 2 ;;
    --runtime-package-lock) (($# >= 2)) || { usage >&2; exit 2; }; RUNTIME_PACKAGE_LOCK="$2"; shift 2 ;;
    --runtime-snapshot) (($# >= 2)) || { usage >&2; exit 2; }; RUNTIME_SNAPSHOT="$2"; shift 2 ;;
    --runtime-replay) (($# >= 2)) || { usage >&2; exit 2; }; RUNTIME_REPLAY="$2"; shift 2 ;;
    --license) (($# >= 2)) || { usage >&2; exit 2; }; LICENSE_FILE="$2"; shift 2 ;;
    --repository) (($# >= 2)) || { usage >&2; exit 2; }; REPOSITORY="$2"; shift 2 ;;
    --commit) (($# >= 2)) || { usage >&2; exit 2; }; COMMIT="$2"; shift 2 ;;
    --version) (($# >= 2)) || { usage >&2; exit 2; }; VERSION="$2"; shift 2 ;;
    --ci-run-id) (($# >= 2)) || { usage >&2; exit 2; }; CI_RUN_ID="$2"; shift 2 ;;
    --security-run-id) (($# >= 2)) || { usage >&2; exit 2; }; SECURITY_RUN_ID="$2"; shift 2 ;;
    --android-run-id) (($# >= 2)) || { usage >&2; exit 2; }; ANDROID_RUN_ID="$2"; shift 2 ;;
    --qualification-run-id) (($# >= 2)) || { usage >&2; exit 2; }; QUALIFICATION_RUN_ID="$2"; shift 2 ;;
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
  local status=$?
  trap - EXIT INT TERM HUP
  if ((COMPLETED == 0)) \
    && [[ -n "$STAGING_DIR" && -n "$OUTPUT" && "$STAGING_DIR" == "$OUTPUT.staging.$$" ]]; then
    rm -rf -- "$STAGING_DIR" >/dev/null 2>&1 || status=1
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

for command_name in awk bash basename chmod cmp cp date dirname file find grep install jq mkdir mv python3 realpath rm sha256sum sort stat tar wc; do
  command -v "$command_name" >/dev/null 2>&1 || fail required_command_missing
done

required_values=(
  "$DEFAULT_DIR" "$MCP_RUNTIME_DIR" "$BATTERY_DIR" "$VOLUME_DIR"
  "$VOLUME_CONTROL_DIR" "$COMMAND_DIR" "$FULL_SUITE_DIR"
  "$EMULATED_EVIDENCE_DIR" "$DEPLOYMENT_EVIDENCE"
  "$AUTOMATED_QUALIFICATION" "$RUNTIME_ARCHIVE" "$RUNTIME_PACKAGE_LOCK"
  "$RUNTIME_SNAPSHOT" "$RUNTIME_REPLAY" "$LICENSE_FILE" "$REPOSITORY" "$COMMIT"
  "$VERSION" "$CI_RUN_ID" "$SECURITY_RUN_ID" "$ANDROID_RUN_ID"
  "$QUALIFICATION_RUN_ID" "$OUTPUT"
)
for required_value in "${required_values[@]}"; do
  [[ -n "$required_value" ]] || fail required_argument_missing
done
[[ "$(basename -- "$DEPLOYMENT_EVIDENCE")" == automated-native-deployment-v1.json ]] \
  || fail deployment_evidence_name_invalid
[[ "$(basename -- "$AUTOMATED_QUALIFICATION")" == automated-qualification-v1.json ]] \
  || fail automated_qualification_name_invalid
[[ "$(basename -- "$RUNTIME_ARCHIVE")" == termux-qualified-runtime-image-v1.tar.gz ]] \
  || fail runtime_archive_name_invalid
[[ "$(basename -- "$RUNTIME_PACKAGE_LOCK")" == termux-runtime-package-lock-v1.json ]] \
  || fail runtime_package_lock_name_invalid
[[ "$(basename -- "$RUNTIME_SNAPSHOT")" == termux-runtime-snapshot-v1.json ]] \
  || fail runtime_snapshot_name_invalid
[[ "$(basename -- "$RUNTIME_REPLAY")" == termux-runtime-snapshot-replay-v1.json ]] \
  || fail runtime_replay_name_invalid

[[ "$REPOSITORY" == "CyberBASSLord-666/termux-mcp-edge" ]] || fail repository_invalid
[[ "$COMMIT" =~ ^[0-9a-f]{40}$ ]] || fail commit_invalid
[[ "$VERSION" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] || fail version_invalid
[[ "$CI_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail ci_run_id_invalid
[[ "$SECURITY_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail security_run_id_invalid
[[ "$ANDROID_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail android_run_id_invalid
[[ "$QUALIFICATION_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail qualification_run_id_invalid
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
  termux-native-aggregate-evidence-v4.json
  termux-battery-emulated-evidence.json
  termux-volume-emulated-evidence.json
  termux-volume-control-emulated-evidence.json
  termux-command-emulated-evidence.json
  termux-observation-requirement-v3.json
)
runtime_source_names=(
  termux-qualified-runtime-image-v1.tar.gz
  termux-runtime-package-lock-v1.json
  termux-runtime-snapshot-v1.json
  termux-runtime-snapshot-replay-v1.json
)
runtime_sources=(
  "$RUNTIME_ARCHIVE"
  "$RUNTIME_PACKAGE_LOCK"
  "$RUNTIME_SNAPSHOT"
  "$RUNTIME_REPLAY"
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

reject_duplicate_json_keys() {
  local path="$1" error_code="$2"
  python3 - "$path" <<'PY' || fail "$error_code"
import json
import pathlib
import sys

def closed_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError("duplicate key")
        value[key] = item
    return value

def reject_constant(_value):
    raise ValueError("non-finite number")

with pathlib.Path(sys.argv[1]).open("r", encoding="utf-8") as source:
    json.load(
        source,
        object_pairs_hook=closed_object,
        parse_constant=reject_constant,
    )
PY
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
  "$PAYLOAD_DIR/evidence/runtime" "$WORK_DIR" "$SNAPSHOT_DIR" \
  "$SNAPSHOT_DIR/bundles" "$SNAPSHOT_DIR/emulated" "$SNAPSHOT_DIR/runtime" \
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

require_regular_file "$DEPLOYMENT_EVIDENCE" 1048576 deployment_evidence_invalid
require_regular_file "$AUTOMATED_QUALIFICATION" 1048576 automated_qualification_invalid
require_regular_file "$LICENSE_FILE" 1048576 license_invalid
cp -P -- "$DEPLOYMENT_EVIDENCE" "$SNAPSHOT_DIR/automated-native-deployment-v1.json" \
  || fail input_snapshot_failed
cp -P -- "$AUTOMATED_QUALIFICATION" "$SNAPSHOT_DIR/automated-qualification-v1.json" \
  || fail input_snapshot_failed
cp -P -- "$LICENSE_FILE" "$SNAPSHOT_DIR/LICENSE" || fail input_snapshot_failed
DEPLOYMENT_EVIDENCE="$SNAPSHOT_DIR/automated-native-deployment-v1.json"
AUTOMATED_QUALIFICATION="$SNAPSHOT_DIR/automated-qualification-v1.json"
LICENSE_FILE="$SNAPSHOT_DIR/LICENSE"

for index in "${!runtime_sources[@]}"; do
  runtime_source="${runtime_sources[$index]}"
  runtime_name="${runtime_source_names[$index]}"
  if [[ "$runtime_name" == termux-qualified-runtime-image-v1.tar.gz ]]; then
    runtime_limit=1610612736
  else
    runtime_limit=16777216
  fi
  require_regular_file "$runtime_source" "$runtime_limit" runtime_evidence_invalid
  cp -P -- "$runtime_source" "$SNAPSHOT_DIR/runtime/$runtime_name" \
    || fail input_snapshot_failed
  cmp -s -- "$runtime_source" "$SNAPSHOT_DIR/runtime/$runtime_name" \
    || fail input_snapshot_failed
  chmod 600 -- "$SNAPSHOT_DIR/runtime/$runtime_name" || fail input_snapshot_failed
  runtime_sources[$index]="$SNAPSHOT_DIR/runtime/$runtime_name"
done
RUNTIME_ARCHIVE="${runtime_sources[0]}"
RUNTIME_PACKAGE_LOCK="${runtime_sources[1]}"
RUNTIME_SNAPSHOT="${runtime_sources[2]}"
RUNTIME_REPLAY="${runtime_sources[3]}"

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
  reject_duplicate_json_keys "$evidence_path" emulated_evidence_json_invalid
  jq -e . "$evidence_path" >/dev/null 2>&1 || fail emulated_evidence_json_invalid
done
require_regular_file "$RUNTIME_ARCHIVE" 1610612736 runtime_archive_invalid
for runtime_json in "$RUNTIME_PACKAGE_LOCK" "$RUNTIME_SNAPSHOT" "$RUNTIME_REPLAY"; do
  require_regular_file "$runtime_json" 16777216 runtime_evidence_invalid
  reject_duplicate_json_keys "$runtime_json" runtime_evidence_json_invalid
  jq -e . "$runtime_json" >/dev/null 2>&1 || fail runtime_evidence_json_invalid
done

AGGREGATE_EVIDENCE="$EMULATED_EVIDENCE_DIR/termux-native-aggregate-evidence-v4.json"
BATTERY_EVIDENCE="$EMULATED_EVIDENCE_DIR/termux-battery-emulated-evidence.json"
VOLUME_EVIDENCE="$EMULATED_EVIDENCE_DIR/termux-volume-emulated-evidence.json"
VOLUME_CONTROL_EVIDENCE="$EMULATED_EVIDENCE_DIR/termux-volume-control-emulated-evidence.json"
COMMAND_EVIDENCE="$EMULATED_EVIDENCE_DIR/termux-command-emulated-evidence.json"
OBSERVATION_REQUIREMENT="$EMULATED_EVIDENCE_DIR/termux-observation-requirement-v3.json"
aggregate_sha="$(sha256sum -- "$AGGREGATE_EVIDENCE" | awk '{print $1}')" || fail aggregate_evidence_digest_failed

jq -e \
  --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg default_sha "${binary_sha[0]}" --argjson default_bytes "${binary_bytes[0]}" \
  --arg mcp_sha "${binary_sha[1]}" --argjson mcp_bytes "${binary_bytes[1]}" \
  --arg volume_control_sha "${binary_sha[4]}" --argjson volume_control_bytes "${binary_bytes[4]}" \
  --arg full_suite_sha "${binary_sha[6]}" --argjson full_suite_bytes "${binary_bytes[6]}" \
  --arg full_suite_manifest_sha "${manifest_sha[6]}" '
    (keys == ["aggregateValidation","candidate","claimBoundary","completedAt","coverage","environment","failureCode","gateVersion","releaseQualificationEligible","runtimeValidation","schemaVersion","startedAt","status","stress"])
    and .schemaVersion == 4
    and .gateVersion == "4"
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
    and (.environment | keys == ["androidLinker","architecture","executionMode","image","imageDigest","rootfsImageId","runtimeImageDigest"])
    and .environment.executionMode == "official-termux-docker-native-arm64"
    and (.environment.architecture == "aarch64" or .environment.architecture == "arm64")
    and .environment.image == "termux/termux-docker:aarch64"
    and (.environment.imageDigest | test("^sha256:[0-9a-f]{64}$"))
    and (.environment.rootfsImageId | test("^sha256:[0-9a-f]{64}$"))
    and (.environment.runtimeImageDigest | test("^sha256:[0-9a-f]{64}$"))
    and .environment.runtimeImageDigest != .environment.rootfsImageId
    and .environment.androidLinker == true
    and .claimBoundary == {
      physicalDeviceObserved:false,
      androidFrameworkObserved:false,
      sustainedPhysicalSoak:false,
      physicalCertification:"not_run"
    }
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
    and .aggregateValidation.automatedQualificationComponent == true
    and .stress.status == "pass"
    and .stress.servicePidStable == true
    and .stress.healthReadyStable == true
    and .stress.longObservationRequired == false
  ' "$AGGREGATE_EVIDENCE" >/dev/null 2>&1 || fail aggregate_evidence_mismatch

aggregate_environment_json="$(jq -cS '.environment' "$AGGREGATE_EVIDENCE")" \
  || fail aggregate_evidence_mismatch

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
    --arg related_sha "$related_sha" --argjson related_bytes "$related_bytes" \
    --argjson expected_environment "$aggregate_environment_json" '
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
      and .environment == $expected_environment
      and .validation.status == "pass"
      and .validation.requests >= 1
      and .validation.exactArtifact == true
      and ([.validation | to_entries[] | select(.value | type == "boolean") | select(.key != "longObservationRequired") | .value] | all(. == true))
      and (if (.validation | has("longObservationRequired")) then .validation.longObservationRequired == false else true end)
    ' "$path" >/dev/null 2>&1 || fail specialized_evidence_mismatch
}

verify_specialized_evidence "$BATTERY_EVIDENCE" battery 3 3 2
verify_specialized_evidence "$VOLUME_EVIDENCE" volume 2 2 3
verify_specialized_evidence "$VOLUME_CONTROL_EVIDENCE" volume-control 2 2 4 3
verify_specialized_evidence "$COMMAND_EVIDENCE" command 3 3 5 0

jq -e \
  --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg full_suite_sha "${binary_sha[6]}" --arg full_suite_manifest_sha "${manifest_sha[6]}" \
  --arg aggregate_sha "$aggregate_sha" '
    (keys == ["candidate","changedInputClasses","claimBoundary","classifierVersion","createdAt","emulation","evidenceMode","failureCode","inheritanceCandidate","nextGate","protectedInputComparison","reasonCode","releaseQualificationEligible","schemaVersion","source","status"])
    and .schemaVersion == 3
    and .classifierVersion == "3"
    and .status == "pass"
    and .failureCode == null
    and .releaseQualificationEligible == false
    and .evidenceMode == "automated_release_qualification"
    and .reasonCode == "automated_native_termux_evidence_required"
    and .inheritanceCandidate == false
    and .nextGate == "assemble_automated_release_qualification"
    and .claimBoundary == {
      physicalDeviceObserved:false,
      androidFrameworkObserved:false,
      sustainedPhysicalSoak:false,
      physicalCertification:"not_run"
    }
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
  ' "$OBSERVATION_REQUIREMENT" >/dev/null 2>&1 || fail observation_requirement_mismatch

require_regular_file "$DEPLOYMENT_EVIDENCE" 1048576 deployment_evidence_invalid
reject_duplicate_json_keys "$DEPLOYMENT_EVIDENCE" deployment_evidence_json_invalid
jq -e . "$DEPLOYMENT_EVIDENCE" >/dev/null 2>&1 || fail deployment_evidence_json_invalid
require_regular_file "$AUTOMATED_QUALIFICATION" 1048576 automated_qualification_invalid
reject_duplicate_json_keys "$AUTOMATED_QUALIFICATION" automated_qualification_json_invalid
jq -e . "$AUTOMATED_QUALIFICATION" >/dev/null 2>&1 || fail automated_qualification_json_invalid

: >"$WORK_DIR/expected-qualification-artifacts.jsonl"
for index in "${!postures[@]}"; do
  jq -cn \
    --arg posture "${postures[$index]}" \
    --argjson features "${features_json[$index]}" \
    --arg workflow_artifact_name "${artifact_names[$index]}" \
    --arg sha256 "${binary_sha[$index]}" \
    --argjson bytes "${binary_bytes[$index]}" \
    --arg manifest_sha256 "${manifest_sha[$index]}" '
      {
        posture:$posture,
        features:$features,
        workflowArtifactName:$workflow_artifact_name,
        sha256:$sha256,
        bytes:$bytes,
        manifestSha256:$manifest_sha256
      }
    ' >>"$WORK_DIR/expected-qualification-artifacts.jsonl" \
    || fail automated_qualification_artifact_record_failed
done
jq -s . "$WORK_DIR/expected-qualification-artifacts.jsonl" \
  >"$WORK_DIR/expected-qualification-artifacts.json" \
  || fail automated_qualification_artifact_record_failed

deployment_sha="$(sha256sum -- "$DEPLOYMENT_EVIDENCE" | awk '{print $1}')" \
  || fail deployment_evidence_digest_failed
deployment_bytes="$(stat -c '%s' -- "$DEPLOYMENT_EVIDENCE")" \
  || fail deployment_evidence_digest_failed
classifier_sha="$(sha256sum -- "$OBSERVATION_REQUIREMENT" | awk '{print $1}')" \
  || fail observation_requirement_digest_failed
classifier_bytes="$(stat -c '%s' -- "$OBSERVATION_REQUIREMENT")" \
  || fail observation_requirement_digest_failed
aggregate_bytes="$(stat -c '%s' -- "$AGGREGATE_EVIDENCE")" \
  || fail aggregate_evidence_digest_failed
script_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)" \
  || fail repository_root_resolution_failed
policy_path="$script_root/docs/release-qualification-policy-v1.json"
scenario_set_path="$script_root/docs/automated-native-deployment-scenarios-v1.json"
require_regular_file "$policy_path" 1048576 qualification_policy_invalid
require_regular_file "$scenario_set_path" 1048576 qualification_scenario_set_invalid
policy_sha="$(sha256sum -- "$policy_path" | awk '{print $1}')" \
  || fail qualification_policy_invalid
scenario_set_sha="$(sha256sum -- "$scenario_set_path" | awk '{print $1}')" \
  || fail qualification_scenario_set_invalid
rootfs_digest="$(jq -er '.environment.imageDigest' "$AGGREGATE_EVIDENCE")" \
  || fail aggregate_evidence_mismatch
rootfs_image_id="$(jq -er '.environment.rootfsImageId' "$AGGREGATE_EVIDENCE")" \
  || fail aggregate_evidence_mismatch
runtime_image_digest="$(jq -er '.environment.runtimeImageDigest' "$AGGREGATE_EVIDENCE")" \
  || fail aggregate_evidence_mismatch
runtime_archive_sha="$(sha256sum -- "$RUNTIME_ARCHIVE" | awk '{print $1}')" \
  || fail runtime_evidence_digest_failed
runtime_archive_bytes="$(stat -c '%s' -- "$RUNTIME_ARCHIVE")" \
  || fail runtime_evidence_digest_failed
runtime_package_lock_sha="$(sha256sum -- "$RUNTIME_PACKAGE_LOCK" | awk '{print $1}')" \
  || fail runtime_evidence_digest_failed
runtime_package_lock_bytes="$(stat -c '%s' -- "$RUNTIME_PACKAGE_LOCK")" \
  || fail runtime_evidence_digest_failed
runtime_snapshot_sha="$(sha256sum -- "$RUNTIME_SNAPSHOT" | awk '{print $1}')" \
  || fail runtime_evidence_digest_failed
runtime_snapshot_bytes="$(stat -c '%s' -- "$RUNTIME_SNAPSHOT")" \
  || fail runtime_evidence_digest_failed
runtime_replay_sha="$(sha256sum -- "$RUNTIME_REPLAY" | awk '{print $1}')" \
  || fail runtime_evidence_digest_failed
runtime_replay_bytes="$(stat -c '%s' -- "$RUNTIME_REPLAY")" \
  || fail runtime_evidence_digest_failed

: >"$WORK_DIR/expected-specialized-evidence.jsonl"
for specialized_path in \
  "$BATTERY_EVIDENCE" "$VOLUME_EVIDENCE" "$VOLUME_CONTROL_EVIDENCE" "$COMMAND_EVIDENCE"
do
  specialized_name="$(basename -- "$specialized_path")"
  specialized_sha="$(sha256sum -- "$specialized_path" | awk '{print $1}')" \
    || fail specialized_evidence_digest_failed
  specialized_bytes="$(stat -c '%s' -- "$specialized_path")" \
    || fail specialized_evidence_digest_failed
  jq -cn --arg file_name "$specialized_name" --arg sha256 "$specialized_sha" \
    --argjson bytes "$specialized_bytes" \
    '{fileName:$file_name,sha256:$sha256,bytes:$bytes}' \
    >>"$WORK_DIR/expected-specialized-evidence.jsonl" \
    || fail specialized_evidence_digest_failed
done
jq -s . "$WORK_DIR/expected-specialized-evidence.jsonl" \
  >"$WORK_DIR/expected-specialized-evidence.json" \
  || fail specialized_evidence_digest_failed

jq -e \
  --arg repository "$REPOSITORY" --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg qualification_run "$QUALIFICATION_RUN_ID" \
  --arg aggregate_sha "$aggregate_sha" --argjson aggregate_bytes "$aggregate_bytes" \
  --arg deployment_sha "$deployment_sha" --argjson deployment_bytes "$deployment_bytes" \
  --arg classifier_sha "$classifier_sha" --argjson classifier_bytes "$classifier_bytes" \
  --arg policy_sha "$policy_sha" --arg scenario_set_sha "$scenario_set_sha" \
  --arg rootfs_digest "$rootfs_digest" --arg rootfs_image_id "$rootfs_image_id" \
  --arg runtime_image_digest "$runtime_image_digest" \
  --arg runtime_archive_sha "$runtime_archive_sha" \
  --argjson runtime_archive_bytes "$runtime_archive_bytes" \
  --arg runtime_package_lock_sha "$runtime_package_lock_sha" \
  --argjson runtime_package_lock_bytes "$runtime_package_lock_bytes" \
  --arg runtime_snapshot_sha "$runtime_snapshot_sha" \
  --argjson runtime_snapshot_bytes "$runtime_snapshot_bytes" \
  --arg runtime_replay_sha "$runtime_replay_sha" \
  --argjson runtime_replay_bytes "$runtime_replay_bytes" \
  --slurpfile artifacts "$WORK_DIR/expected-qualification-artifacts.json" \
  --slurpfile specialized "$WORK_DIR/expected-specialized-evidence.json" '
    (keys == ["artifacts","claimBoundary","commit","envelopeVersion","environment","evidence","failureCode","gates","policy","qualificationClass","qualificationRun","releaseEligible","repository","retainedRuntime","scenarioSet","schemaVersion","status","version","workflowRuns"])
    and .schemaVersion == 1
    and .envelopeVersion == "1"
    and .status == "pass"
    and .failureCode == null
    and .releaseEligible == true
    and .qualificationClass == "official_termux_native_automated_v1"
    and .repository == $repository
    and .commit == $commit
    and .version == $version
    and .claimBoundary == {
      physicalDeviceObserved:false,
      androidFrameworkObserved:false,
      sustainedPhysicalSoak:false,
      physicalCertification:"not_run"
    }
    and (.workflowRuns | keys == ["android","ci","security"])
    and .workflowRuns.ci == {
      runId:$ci,attempt:1,event:"push",ref:"refs/heads/main",
      headCommit:$commit,conclusion:"success"
    }
    and .workflowRuns.security == {
      runId:$security,attempt:1,event:"push",ref:"refs/heads/main",
      headCommit:$commit,conclusion:"success"
    }
    and .workflowRuns.android == {
      runId:$android,attempt:1,event:"push",ref:"refs/heads/main",
      headCommit:$commit,conclusion:"success"
    }
    and .qualificationRun == {
      runId:$qualification_run,
      attempt:1,
      event:"workflow_run",
      sourceWorkflow:"Android Cross Compile",
      sourceRunId:$android
    }
    and .environment.executionMode == "official-termux-docker-native-arm64"
    and .environment.architecture == "aarch64"
    and (.environment | keys == ["androidRuntime","architecture","executionMode","rootfsUserland","runtimeImageDigest"])
    and .environment.rootfsUserland.image == "termux/termux-docker:aarch64"
    and .environment.rootfsUserland.digest == $rootfs_digest
    and .environment.rootfsUserland.imageId == $rootfs_image_id
    and .environment.rootfsUserland.prefix == "/data/data/com.termux/files/usr"
    and .environment.runtimeImageDigest == $runtime_image_digest
    and .environment.runtimeImageDigest != .environment.rootfsUserland.imageId
    and .environment.androidRuntime.abi == "android-bionic"
    and .environment.androidRuntime.linkerPath == "/system/bin/linker64"
    and (.environment.androidRuntime.linkerSha256 | test("^[0-9a-f]{64}$"))
    and (.environment.androidRuntime.linkerBytes | type == "number" and floor == . and . >= 1)
    and .environment.androidRuntime.linkerIdentity == "aarch64-android-bionic-elf"
    and (.retainedRuntime | keys == [
      "androidLinker",
      "archive",
      "base",
      "claimBoundary",
      "installedPackages",
      "packageLock",
      "rebuildReproducibilityClaim",
      "replay",
      "runtimeImageId",
      "snapshot",
      "verification"
    ])
    and .retainedRuntime.runtimeImageId == $runtime_image_digest
    and .retainedRuntime.base == {
      image:"termux/termux-docker:aarch64",
      digest:$rootfs_digest,
      imageId:$rootfs_image_id
    }
    and .retainedRuntime.archive == {
      fileName:"termux-qualified-runtime-image-v1.tar.gz",
      sha256:$runtime_archive_sha,
      bytes:$runtime_archive_bytes
    }
    and .retainedRuntime.packageLock == {
      fileName:"termux-runtime-package-lock-v1.json",
      sha256:$runtime_package_lock_sha,
      bytes:$runtime_package_lock_bytes
    }
    and .retainedRuntime.snapshot == {
      fileName:"termux-runtime-snapshot-v1.json",
      sha256:$runtime_snapshot_sha,
      bytes:$runtime_snapshot_bytes
    }
    and .retainedRuntime.replay == {
      fileName:"termux-runtime-snapshot-replay-v1.json",
      sha256:$runtime_replay_sha,
      bytes:$runtime_replay_bytes
    }
    and (.retainedRuntime.installedPackages | keys == ["count","sha256"])
    and (.retainedRuntime.installedPackages.sha256 | test("^[0-9a-f]{64}$"))
    and (.retainedRuntime.installedPackages.count
      | type == "number" and floor == . and . >= 1 and . <= 4096)
    and (.retainedRuntime.androidLinker | keys == ["bytes","path","sha256"])
    and .retainedRuntime.androidLinker.path == .environment.androidRuntime.linkerPath
    and .retainedRuntime.androidLinker.sha256 == .environment.androidRuntime.linkerSha256
    and .retainedRuntime.androidLinker.bytes == .environment.androidRuntime.linkerBytes
    and .retainedRuntime.verification == {
      archiveDigestVerified:true,
      singleImageArchive:true,
      loadedImageIdVerified:true,
      platformVerified:true,
      rootfsLayersVerified:true,
      packageLockVerified:true,
      packageInputBytesVerified:true,
      repositoryIndexBytesVerified:true,
      installedPackageInventoryVerified:true,
      requiredRuntimeCommandsVerified:true,
      androidLinkerVerified:true,
      runtimeNetworkAccess:false
    }
    and .retainedRuntime.claimBoundary == .claimBoundary
    and .retainedRuntime.rebuildReproducibilityClaim == false
    and .artifacts == $artifacts[0]
    and .evidence == {
      aggregate:{
        fileName:"termux-native-aggregate-evidence-v4.json",
        sha256:$aggregate_sha,
        bytes:$aggregate_bytes
      },
      deployment:{
        fileName:"automated-native-deployment-v1.json",
        sha256:$deployment_sha,
        bytes:$deployment_bytes
      },
      classifier:{
        fileName:"termux-observation-requirement-v3.json",
        sha256:$classifier_sha,
        bytes:$classifier_bytes
      },
      specialized:$specialized[0]
    }
    and .gates == {
      firstAttemptMainWorkflows:"pass",
      artifactLineage:"pass",
      officialTermuxNativeRuntime:"pass",
      aggregateComposition:"pass",
      specializedProviderBoundaries:"pass",
      isolatedDeploymentRecovery:"pass",
      automatedReleaseClassification:"pass"
    }
    and (.policy | keys == ["fileName","sha256"])
    and .policy.fileName == "release-qualification-policy-v1.json"
    and .policy.sha256 == $policy_sha
    and (.scenarioSet | keys == ["fileName","scenarioCount","scenarioIds","scenarioSetVersion","schemaVersion","sha256"])
    and .scenarioSet.fileName == "automated-native-deployment-scenarios-v1.json"
    and .scenarioSet.schemaVersion == 1
    and .scenarioSet.scenarioSetVersion == "1"
    and .scenarioSet.scenarioCount == 6
    and .scenarioSet.scenarioIds == [
      "isolated_fresh_deploy",
      "failed_upgrade_recovery",
      "supervised_restart",
      "rollback_recovery",
      "uninstall",
      "bounded_cleanup"
    ]
    and .scenarioSet.sha256 == $scenario_set_sha
  ' "$AUTOMATED_QUALIFICATION" >/dev/null 2>&1 \
  || fail automated_qualification_mismatch

# Replay the canonical qualification assembler against the private snapshots
# and require byte-for-byte equality with the supplied envelope. This imports
# the core validator's strict duplicate-key, schema, deployment, nested
# environment, and cross-document identity checks without trusting a caller to
# have invoked it correctly.
packager_path="$script_root/scripts/package_automated_qualification.sh"
require_regular_file "$packager_path" 2097152 qualification_packager_invalid
recomputed_parent="$WORK_DIR/recomputed-qualification"
mkdir -m 700 -- "$recomputed_parent" || fail automated_qualification_mismatch
recomputed_qualification="$recomputed_parent/automated-qualification-v1.json"
if ! bash "$packager_path" \
  --policy "$policy_path" \
  --scenario-set "$scenario_set_path" \
  --aggregate-evidence "$AGGREGATE_EVIDENCE" \
  --deployment-evidence "$DEPLOYMENT_EVIDENCE" \
  --classifier-evidence "$OBSERVATION_REQUIREMENT" \
  --battery-evidence "$BATTERY_EVIDENCE" \
  --volume-evidence "$VOLUME_EVIDENCE" \
  --volume-control-evidence "$VOLUME_CONTROL_EVIDENCE" \
  --command-evidence "$COMMAND_EVIDENCE" \
  --runtime-archive "$RUNTIME_ARCHIVE" \
  --runtime-package-lock "$RUNTIME_PACKAGE_LOCK" \
  --runtime-snapshot "$RUNTIME_SNAPSHOT" \
  --runtime-replay "$RUNTIME_REPLAY" \
  --default-dir "${bundle_dirs[0]}" \
  --mcp-runtime-dir "${bundle_dirs[1]}" \
  --battery-dir "${bundle_dirs[2]}" \
  --volume-dir "${bundle_dirs[3]}" \
  --volume-control-dir "${bundle_dirs[4]}" \
  --command-dir "${bundle_dirs[5]}" \
  --full-suite-dir "${bundle_dirs[6]}" \
  --qualification-run-id "$QUALIFICATION_RUN_ID" \
  --output "$recomputed_qualification" \
  >"$WORK_DIR/recomputed-qualification.stdout" \
  2>"$WORK_DIR/recomputed-qualification.stderr"
then
  fail automated_qualification_mismatch
fi
cmp -s -- "$recomputed_qualification" "$AUTOMATED_QUALIFICATION" \
  || fail automated_qualification_mismatch

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

copy_evidence "$AGGREGATE_EVIDENCE" \
  evidence/termux-native-aggregate-evidence-v4.json "$WORK_DIR/aggregate-record.json"
copy_evidence "$DEPLOYMENT_EVIDENCE" \
  evidence/automated-native-deployment-v1.json "$WORK_DIR/deployment-record.json"
copy_evidence "$OBSERVATION_REQUIREMENT" \
  evidence/termux-observation-requirement-v3.json "$WORK_DIR/classifier-record.json"
copy_evidence "$AUTOMATED_QUALIFICATION" \
  evidence/automated-qualification-v1.json "$WORK_DIR/qualification-record.json"
copy_evidence "$RUNTIME_ARCHIVE" \
  evidence/runtime/termux-qualified-runtime-image-v1.tar.gz \
  "$WORK_DIR/runtime-archive-record.json"
copy_evidence "$RUNTIME_PACKAGE_LOCK" \
  evidence/runtime/termux-runtime-package-lock-v1.json \
  "$WORK_DIR/runtime-package-lock-record.json"
copy_evidence "$RUNTIME_SNAPSHOT" \
  evidence/runtime/termux-runtime-snapshot-v1.json \
  "$WORK_DIR/runtime-snapshot-record.json"
copy_evidence "$RUNTIME_REPLAY" \
  evidence/runtime/termux-runtime-snapshot-replay-v1.json \
  "$WORK_DIR/runtime-replay-record.json"

specialized_sources=(
  "$BATTERY_EVIDENCE"
  "$VOLUME_EVIDENCE"
  "$VOLUME_CONTROL_EVIDENCE"
  "$COMMAND_EVIDENCE"
)
specialized_destinations=(
  evidence/termux-battery-emulated-evidence.json
  evidence/termux-volume-emulated-evidence.json
  evidence/termux-volume-control-emulated-evidence.json
  evidence/termux-command-emulated-evidence.json
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
  --arg qualification_run "$QUALIFICATION_RUN_ID" \
  --slurpfile artifacts "$WORK_DIR/artifact-records.jsonl" \
  --slurpfile aggregate "$WORK_DIR/aggregate-record.json" \
  --slurpfile deployment "$WORK_DIR/deployment-record.json" \
  --slurpfile classifier "$WORK_DIR/classifier-record.json" \
  --slurpfile qualification "$WORK_DIR/qualification-record.json" \
  --slurpfile runtime_archive "$WORK_DIR/runtime-archive-record.json" \
  --slurpfile runtime_package_lock "$WORK_DIR/runtime-package-lock-record.json" \
  --slurpfile runtime_snapshot "$WORK_DIR/runtime-snapshot-record.json" \
  --slurpfile runtime_replay "$WORK_DIR/runtime-replay-record.json" \
  --slurpfile specialized "$WORK_DIR/specialized-evidence-records.jsonl" \
  --slurpfile license "$WORK_DIR/license-record.json" '
    {
      schemaVersion: 2,
      publicationState: "staged_not_released",
      releaseEligible: false,
      qualificationClass: "official_termux_native_automated_v1",
      claimBoundary: {
        physicalDeviceObserved: false,
        androidFrameworkObserved: false,
        sustainedPhysicalSoak: false,
        physicalCertification: "not_run"
      },
      repository: $repository,
      commit: $commit,
      version: $version,
      target: "aarch64-linux-android",
      workflowRuns: {
        ci:$ci,
        security:$security,
        android:$android,
        qualification:$qualification_run
      },
      checksums: {algorithm:"sha256", combinedFileName:"SHA256SUMS"},
      license: $license[0],
      evidence: {
        qualification: $qualification[0],
        aggregate: $aggregate[0],
        deployment: $deployment[0],
        classifier: $classifier[0],
        runtime: {
          archive: $runtime_archive[0],
          packageLock: $runtime_package_lock[0],
          snapshot: $runtime_snapshot[0],
          replay: $runtime_replay[0]
        },
        specialized: $specialized
      },
      artifacts: $artifacts
    }
  ' >"$PAYLOAD_DIR/release-staging-manifest-v2.json" || fail staging_manifest_write_failed

chmod 644 "$PAYLOAD_DIR/SHA256SUMS" "$PAYLOAD_DIR"/*.sha256 \
  "$PAYLOAD_DIR/release-staging-manifest-v2.json" || fail metadata_mode_failed

jq -e \
  --arg repository "$REPOSITORY" --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg qualification "$QUALIFICATION_RUN_ID" \
  --arg runtime_archive_sha "$runtime_archive_sha" \
  --argjson runtime_archive_bytes "$runtime_archive_bytes" \
  --arg runtime_package_lock_sha "$runtime_package_lock_sha" \
  --argjson runtime_package_lock_bytes "$runtime_package_lock_bytes" \
  --arg runtime_snapshot_sha "$runtime_snapshot_sha" \
  --argjson runtime_snapshot_bytes "$runtime_snapshot_bytes" \
  --arg runtime_replay_sha "$runtime_replay_sha" \
  --argjson runtime_replay_bytes "$runtime_replay_bytes" '
    (keys == ["artifacts","checksums","claimBoundary","commit","evidence","license","publicationState","qualificationClass","releaseEligible","repository","schemaVersion","target","version","workflowRuns"])
    and .schemaVersion == 2
    and .publicationState == "staged_not_released"
    and .releaseEligible == false
    and .qualificationClass == "official_termux_native_automated_v1"
    and .claimBoundary == {
      physicalDeviceObserved:false,
      androidFrameworkObserved:false,
      sustainedPhysicalSoak:false,
      physicalCertification:"not_run"
    }
    and .repository == $repository
    and .commit == $commit
    and .version == $version
    and .target == "aarch64-linux-android"
    and .workflowRuns == {
      ci:$ci,
      security:$security,
      android:$android,
      qualification:$qualification
    }
    and .checksums == {algorithm:"sha256", combinedFileName:"SHA256SUMS"}
    and (.artifacts | length == 7)
    and ([.artifacts[].posture] == ["default","mcp-runtime","android-battery-status","android-volume-status","android-volume-control","command-execution","full-suite"])
    and (.evidence | keys == ["aggregate","classifier","deployment","qualification","runtime","specialized"])
    and (.evidence.runtime | keys == ["archive","packageLock","replay","snapshot"])
    and .evidence.runtime.archive == {
      fileName:"evidence/runtime/termux-qualified-runtime-image-v1.tar.gz",
      sha256:$runtime_archive_sha,
      bytes:$runtime_archive_bytes
    }
    and .evidence.runtime.packageLock == {
      fileName:"evidence/runtime/termux-runtime-package-lock-v1.json",
      sha256:$runtime_package_lock_sha,
      bytes:$runtime_package_lock_bytes
    }
    and .evidence.runtime.snapshot == {
      fileName:"evidence/runtime/termux-runtime-snapshot-v1.json",
      sha256:$runtime_snapshot_sha,
      bytes:$runtime_snapshot_bytes
    }
    and .evidence.runtime.replay == {
      fileName:"evidence/runtime/termux-runtime-snapshot-replay-v1.json",
      sha256:$runtime_replay_sha,
      bytes:$runtime_replay_bytes
    }
    and (.evidence.specialized | length == 4)
  ' "$PAYLOAD_DIR/release-staging-manifest-v2.json" >/dev/null 2>&1 \
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
archive_size="$(stat -c '%s' -- "$TEMP_TAR")" || fail archive_size_invalid
[[ "$archive_size" =~ ^[0-9]+$ ]] || fail archive_size_invalid
((archive_size > 0 && archive_size <= 2147483647)) || fail archive_size_invalid
archive_sha="$(sha256sum -- "$TEMP_TAR" | awk '{print $1}')" || fail archive_digest_failed
[[ "$archive_sha" =~ ^[0-9a-f]{64}$ ]] || fail archive_digest_failed
if ! EXPECTED_ARCHIVE_SHA256="$archive_sha" EXPECTED_ARCHIVE_SIZE="$archive_size" \
  python3 - "$TEMP_TAR" "$OUTPUT" <<'PY'
import ctypes
import hashlib
import os
import stat
import sys

source, output = sys.argv[1:]
expected_sha256 = os.environ["EXPECTED_ARCHIVE_SHA256"]
expected_size = int(os.environ["EXPECTED_ARCHIVE_SIZE"])
source_fd = -1
output_parent_fd = -1
try:
    source_fd = os.open(
        source,
        os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW,
    )
    source_stat = os.fstat(source_fd)
    if (
        not stat.S_ISREG(source_stat.st_mode)
        or stat.S_IMODE(source_stat.st_mode) != 0o600
        or source_stat.st_uid != os.getuid()
        or source_stat.st_size != expected_size
    ):
        raise OSError
    digest = hashlib.sha256()
    while True:
        chunk = os.read(source_fd, 1024 * 1024)
        if not chunk:
            break
        digest.update(chunk)
    if digest.hexdigest() != expected_sha256:
        raise OSError

    output_parent, output_name = os.path.split(output)
    output_parent_fd = os.open(
        output_parent,
        os.O_RDONLY | os.O_CLOEXEC | os.O_DIRECTORY | os.O_NOFOLLOW,
    )
    parent_stat = os.fstat(output_parent_fd)
    if not stat.S_ISDIR(parent_stat.st_mode):
        raise OSError
    libc = ctypes.CDLL(None, use_errno=True)
    libc.linkat.argtypes = [
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_int,
    ]
    libc.linkat.restype = ctypes.c_int
    proc_fd_path = f"/proc/self/fd/{source_fd}".encode()
    if libc.linkat(
        -100,
        proc_fd_path,
        output_parent_fd,
        os.fsencode(output_name),
        0x400,
    ) != 0:
        raise OSError(ctypes.get_errno(), "linkat")
except OSError:
    raise SystemExit(1)
finally:
    if output_parent_fd >= 0:
        os.close(output_parent_fd)
    if source_fd >= 0:
        os.close(source_fd)
PY
then
  fail archive_publication_failed
fi
# The no-replace hard-link commit is the final fallible publication operation.
# Never unlink the public output name during failure cleanup: an EEXIST result
# may belong to a concurrent owner, including a hard link to the private tar.
COMPLETED=1
rm -rf -- "$STAGING_DIR" >/dev/null 2>&1 || true
STAGING_DIR=""
printf '[release-stage] publicationState=staged_not_released releaseEligible=false archiveSha256=%s\n' "$archive_sha"
