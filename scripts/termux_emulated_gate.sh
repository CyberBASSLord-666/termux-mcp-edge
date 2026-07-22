#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
umask 077

GATE_VERSION=3
EXPECTED_IMAGE='termux/termux-docker:aarch64'
DEFAULT_SAMPLES=256
MAX_SAMPLES=4096
DEFAULT_PORT=18766

DEFAULT_DIR=''
MCP_DIR=''
VOLUME_CONTROL_DIR=''
FULL_SUITE_DIR=''
EXPECTED_COMMIT=''
EXPECTED_VERSION=''
CI_RUN_ID=''
SECURITY_RUN_ID=''
ANDROID_RUN_ID=''
OUTPUT_REPORT=''
SAMPLES="$DEFAULT_SAMPLES"
PORT="$DEFAULT_PORT"

STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
WORK_ROOT=''
SERVER_PID=''
SESSION_ID=''
REQUEST_COUNT=0
MCP_STATUS=''
BATTERY_PROGRAM=''
VOLUME_PROGRAM=''
BATTERY_PROGRAM_CREATED=false
VOLUME_PROGRAM_CREATED=false

log() { printf '[termux-emulated] %s\n' "$*"; }
fail() {
  printf 'TERMUX_MCP_EMULATED_RESULT=FAIL reason=%s\n' "$*" >&2
  exit 1
}

terminate_server_pid_bounded() {
  local pid="${1:-}" attempt state=''
  [[ "$pid" =~ ^[1-9][0-9]*$ ]] || return 0

  kill -TERM "$pid" >/dev/null 2>&1 || true
  for ((attempt = 0; attempt < 50; attempt++)); do
    if ! kill -0 "$pid" >/dev/null 2>&1; then
      wait "$pid" 2>/dev/null || true
      return 0
    fi
    if [[ -r "/proc/$pid/stat" ]]; then
      state="$(awk '{print $3}' "/proc/$pid/stat" 2>/dev/null || true)"
      if [[ "$state" == Z ]]; then
        wait "$pid" 2>/dev/null || true
        return 0
      fi
    fi
    sleep 0.1
  done

  kill -KILL "$pid" >/dev/null 2>&1 || true
  for ((attempt = 0; attempt < 20; attempt++)); do
    if ! kill -0 "$pid" >/dev/null 2>&1; then
      wait "$pid" 2>/dev/null || true
      return 0
    fi
    if [[ -r "/proc/$pid/stat" ]]; then
      state="$(awk '{print $3}' "/proc/$pid/stat" 2>/dev/null || true)"
      if [[ "$state" == Z ]]; then
        wait "$pid" 2>/dev/null || true
        return 0
      fi
    fi
    sleep 0.1
  done
  return 1
}

usage() {
  cat <<'EOF'
Usage: termux_emulated_gate.sh \
  --default-dir DIR \
  --mcp-dir DIR \
  --volume-control-dir DIR \
  --full-suite-dir DIR \
  --expected-commit SHA \
  --expected-version VERSION \
  --ci-run-id ID \
  --security-run-id ID \
  --android-run-id ID \
  --output REPORT.json \
  [--samples COUNT] \
  [--port PORT]

This gate must run natively on AArch64 inside the pinned official Termux
Docker environment. It validates exact workflow bundles through the canonical
runtime validator, then performs a bounded high-frequency MCP stability pass.
EOF
}

cleanup() {
  local status=$?
  trap - EXIT
  trap '' INT TERM HUP
  if [[ -n "$SERVER_PID" ]] && kill -0 "$SERVER_PID" >/dev/null 2>&1; then
    terminate_server_pid_bounded "$SERVER_PID" || status=1
  fi
  SERVER_PID=''
  unset MCP_TOKEN SESSION_ID 2>/dev/null || true
  if [[ "$BATTERY_PROGRAM_CREATED" == true ]]; then
    rm -f -- "$BATTERY_PROGRAM" >/dev/null 2>&1 || status=1
  fi
  if [[ "$VOLUME_PROGRAM_CREATED" == true ]]; then
    rm -f -- "$VOLUME_PROGRAM" >/dev/null 2>&1 || status=1
  fi
  [[ -z "$WORK_ROOT" ]] || rm -rf -- "$WORK_ROOT" >/dev/null 2>&1 || status=1
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

while (($#)); do
  case "$1" in
    --default-dir)
      (($# >= 2)) || fail missing_default_dir
      DEFAULT_DIR="$2"
      shift 2
      ;;
    --mcp-dir)
      (($# >= 2)) || fail missing_mcp_dir
      MCP_DIR="$2"
      shift 2
      ;;
    --volume-control-dir)
      (($# >= 2)) || fail missing_volume_control_dir
      VOLUME_CONTROL_DIR="$2"
      shift 2
      ;;
    --full-suite-dir)
      (($# >= 2)) || fail missing_full_suite_dir
      FULL_SUITE_DIR="$2"
      shift 2
      ;;
    --expected-commit)
      (($# >= 2)) || fail missing_expected_commit
      EXPECTED_COMMIT="$2"
      shift 2
      ;;
    --expected-version)
      (($# >= 2)) || fail missing_expected_version
      EXPECTED_VERSION="$2"
      shift 2
      ;;
    --ci-run-id)
      (($# >= 2)) || fail missing_ci_run_id
      CI_RUN_ID="$2"
      shift 2
      ;;
    --security-run-id)
      (($# >= 2)) || fail missing_security_run_id
      SECURITY_RUN_ID="$2"
      shift 2
      ;;
    --android-run-id)
      (($# >= 2)) || fail missing_android_run_id
      ANDROID_RUN_ID="$2"
      shift 2
      ;;
    --output)
      (($# >= 2)) || fail missing_output
      OUTPUT_REPORT="$2"
      shift 2
      ;;
    --samples)
      (($# >= 2)) || fail missing_samples
      SAMPLES="$2"
      shift 2
      ;;
    --port)
      (($# >= 2)) || fail missing_port
      PORT="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *) fail unknown_argument ;;
  esac
done

[[ "$EXPECTED_COMMIT" =~ ^[0-9a-f]{40}$ ]] || fail expected_commit_invalid
[[ "$EXPECTED_VERSION" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] || fail expected_version_invalid
[[ "$CI_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail ci_run_id_invalid
[[ "$SECURITY_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail security_run_id_invalid
[[ "$ANDROID_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail android_run_id_invalid
[[ "$SAMPLES" =~ ^[0-9]+$ ]] || fail samples_invalid
((SAMPLES >= 32 && SAMPLES <= MAX_SAMPLES)) || fail samples_out_of_range
[[ "$PORT" =~ ^[0-9]+$ ]] || fail port_invalid
((PORT >= 1024 && PORT <= 65535)) || fail port_invalid
[[ "$DEFAULT_DIR" == /* && "$MCP_DIR" == /* && "$VOLUME_CONTROL_DIR" == /* && "$FULL_SUITE_DIR" == /* && "$OUTPUT_REPORT" == /* ]] || fail absolute_paths_required

[[ "${TERMUX_MCP_EMULATED_ENVIRONMENT:-}" == official-termux-docker-native-arm64 ]] || fail environment_attestation_missing
IMAGE_DIGEST="${TERMUX_MCP_TERMUX_IMAGE_DIGEST:-}"
[[ "$IMAGE_DIGEST" =~ ^sha256:[0-9a-f]{64}$ ]] || fail image_digest_invalid
[[ "$(uname -m)" == aarch64 || "$(uname -m)" == arm64 ]] || fail architecture_not_arm64
[[ "${PREFIX:-}" == /data/data/com.termux/files/usr ]] || fail termux_prefix_invalid
[[ "${HOME:-}" == /data/data/com.termux/files/home ]] || fail termux_home_invalid
[[ -x /system/bin/linker64 ]] || fail android_linker_missing

for command in awk bash cat chmod curl date dd dirname env file find grep install jq kill mkdir mktemp mv readlink realpath rm sed seq sha256sum sleep stat timeout wc; do
  command -v "$command" >/dev/null 2>&1 || fail "required_command_missing_$command"
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VALIDATOR="$SCRIPT_DIR/termux_release_validate.sh"
DEPLOY_SCRIPT="$SCRIPT_DIR/termux_deploy.sh"
[[ -f "$VALIDATOR" && ! -L "$VALIDATOR" ]] || fail validator_invalid
[[ -f "$DEPLOY_SCRIPT" && ! -L "$DEPLOY_SCRIPT" ]] || fail deploy_script_invalid
bash -n "$VALIDATOR"
bash -n "$DEPLOY_SCRIPT"

DEFAULT_ARTIFACT="$DEFAULT_DIR/termux-mcp-server"
DEFAULT_MANIFEST="$DEFAULT_DIR/artifact-manifest.json"
DEFAULT_CHECKSUMS="$DEFAULT_DIR/SHA256SUMS"
MCP_ARTIFACT="$MCP_DIR/termux-mcp-server"
MCP_MANIFEST="$MCP_DIR/artifact-manifest.json"
MCP_CHECKSUMS="$MCP_DIR/SHA256SUMS"
VOLUME_CONTROL_ARTIFACT="$VOLUME_CONTROL_DIR/termux-mcp-server"
VOLUME_CONTROL_MANIFEST="$VOLUME_CONTROL_DIR/artifact-manifest.json"
VOLUME_CONTROL_CHECKSUMS="$VOLUME_CONTROL_DIR/SHA256SUMS"
FULL_SUITE_ARTIFACT="$FULL_SUITE_DIR/termux-mcp-server"
FULL_SUITE_MANIFEST="$FULL_SUITE_DIR/artifact-manifest.json"
FULL_SUITE_CHECKSUMS="$FULL_SUITE_DIR/SHA256SUMS"

for path in \
  "$DEFAULT_ARTIFACT" "$DEFAULT_MANIFEST" "$DEFAULT_CHECKSUMS" \
  "$MCP_ARTIFACT" "$MCP_MANIFEST" "$MCP_CHECKSUMS" \
  "$VOLUME_CONTROL_ARTIFACT" "$VOLUME_CONTROL_MANIFEST" "$VOLUME_CONTROL_CHECKSUMS" \
  "$FULL_SUITE_ARTIFACT" "$FULL_SUITE_MANIFEST" "$FULL_SUITE_CHECKSUMS"; do
  [[ -f "$path" && ! -L "$path" ]] || fail artifact_bundle_member_invalid
done
[[ -x "$DEFAULT_ARTIFACT" && -x "$MCP_ARTIFACT" && -x "$VOLUME_CONTROL_ARTIFACT" && -x "$FULL_SUITE_ARTIFACT" ]] || fail artifact_binary_not_executable
[[ "$(find "$DEFAULT_DIR" -mindepth 1 -maxdepth 1 -type f | wc -l)" == 3 ]] || fail default_bundle_member_count_invalid
[[ "$(find "$MCP_DIR" -mindepth 1 -maxdepth 1 -type f | wc -l)" == 3 ]] || fail mcp_bundle_member_count_invalid
[[ "$(find "$VOLUME_CONTROL_DIR" -mindepth 1 -maxdepth 1 -type f | wc -l)" == 3 ]] || fail volume_control_bundle_member_count_invalid
[[ "$(find "$FULL_SUITE_DIR" -mindepth 1 -maxdepth 1 -type f | wc -l)" == 3 ]] || fail full_suite_bundle_member_count_invalid
(cd "$DEFAULT_DIR" && sha256sum -c SHA256SUMS >/dev/null) || fail default_checksum_invalid
(cd "$MCP_DIR" && sha256sum -c SHA256SUMS >/dev/null) || fail mcp_checksum_invalid
(cd "$VOLUME_CONTROL_DIR" && sha256sum -c SHA256SUMS >/dev/null) || fail volume_control_checksum_invalid
(cd "$FULL_SUITE_DIR" && sha256sum -c SHA256SUMS >/dev/null) || fail full_suite_checksum_invalid

validate_manifest() {
  local manifest="$1" artifact_name="$2" posture="$3" expected_features="$4"
  jq -e \
    --arg commit "$EXPECTED_COMMIT" \
    --arg version "$EXPECTED_VERSION" \
    --arg run_id "$ANDROID_RUN_ID" \
    --arg artifact_name "$artifact_name" \
    --arg posture "$posture" \
    --argjson features "$expected_features" '
      (keys == ["artifactName","bytes","commit","createdAt","elf","features","fileName","posture","repository","schemaVersion","sha256","target","version","workflowRunId"])
      and .schemaVersion == 1
      and .repository == "CyberBASSLord-666/termux-mcp-edge"
      and .commit == $commit
      and .workflowRunId == $run_id
      and .artifactName == $artifact_name
      and .posture == $posture
      and .features == $features
      and .target == "aarch64-linux-android"
      and .fileName == "termux-mcp-server"
      and .version == $version
      and .elf == "aarch64-android-elf"
      and (.sha256 | test("^[0-9a-f]{64}$"))
      and (.bytes >= 1 and .bytes <= 67108864)
    ' "$manifest" >/dev/null
}

validate_manifest "$DEFAULT_MANIFEST" termux-mcp-server-aarch64-linux-android-default default '[]' || fail default_manifest_invalid
validate_manifest "$MCP_MANIFEST" termux-mcp-server-aarch64-linux-android-mcp-runtime mcp-runtime '["mcp-runtime"]' || fail mcp_manifest_invalid
validate_manifest "$VOLUME_CONTROL_MANIFEST" termux-mcp-server-aarch64-linux-android-android-volume-control android-volume-control '["android-volume-control"]' || fail volume_control_manifest_invalid
validate_manifest "$FULL_SUITE_MANIFEST" termux-mcp-server-aarch64-linux-android-full-suite full-suite '["full-suite"]' || fail full_suite_manifest_invalid

DEFAULT_SHA="$(jq -r .sha256 "$DEFAULT_MANIFEST")"
MCP_SHA="$(jq -r .sha256 "$MCP_MANIFEST")"
VOLUME_CONTROL_SHA="$(jq -r .sha256 "$VOLUME_CONTROL_MANIFEST")"
FULL_SUITE_SHA="$(jq -r .sha256 "$FULL_SUITE_MANIFEST")"
FULL_SUITE_MANIFEST_SHA="$(sha256sum "$FULL_SUITE_MANIFEST" | awk '{print $1}')"
DEFAULT_BYTES="$(jq -r .bytes "$DEFAULT_MANIFEST")"
MCP_BYTES="$(jq -r .bytes "$MCP_MANIFEST")"
VOLUME_CONTROL_BYTES="$(jq -r .bytes "$VOLUME_CONTROL_MANIFEST")"
FULL_SUITE_BYTES="$(jq -r .bytes "$FULL_SUITE_MANIFEST")"
[[ "$DEFAULT_SHA" != "$MCP_SHA" && "$DEFAULT_SHA" != "$VOLUME_CONTROL_SHA" && "$DEFAULT_SHA" != "$FULL_SUITE_SHA" && "$MCP_SHA" != "$VOLUME_CONTROL_SHA" && "$MCP_SHA" != "$FULL_SUITE_SHA" && "$VOLUME_CONTROL_SHA" != "$FULL_SUITE_SHA" ]] || fail artifact_postures_not_distinct
[[ "$(sha256sum "$DEFAULT_ARTIFACT" | awk '{print $1}')" == "$DEFAULT_SHA" ]] || fail default_digest_mismatch
[[ "$(sha256sum "$MCP_ARTIFACT" | awk '{print $1}')" == "$MCP_SHA" ]] || fail mcp_digest_mismatch
[[ "$(sha256sum "$VOLUME_CONTROL_ARTIFACT" | awk '{print $1}')" == "$VOLUME_CONTROL_SHA" ]] || fail volume_control_digest_mismatch
[[ "$(sha256sum "$FULL_SUITE_ARTIFACT" | awk '{print $1}')" == "$FULL_SUITE_SHA" ]] || fail full_suite_digest_mismatch
[[ "$(stat -c %s "$DEFAULT_ARTIFACT")" == "$DEFAULT_BYTES" ]] || fail default_size_mismatch
[[ "$(stat -c %s "$MCP_ARTIFACT")" == "$MCP_BYTES" ]] || fail mcp_size_mismatch
[[ "$(stat -c %s "$VOLUME_CONTROL_ARTIFACT")" == "$VOLUME_CONTROL_BYTES" ]] || fail volume_control_size_mismatch
[[ "$(stat -c %s "$FULL_SUITE_ARTIFACT")" == "$FULL_SUITE_BYTES" ]] || fail full_suite_size_mismatch
[[ "$(timeout -k 2 5 "$DEFAULT_ARTIFACT" --version)" == "termux-mcp-server $EXPECTED_VERSION" ]] || fail default_version_mismatch
[[ "$(timeout -k 2 5 "$MCP_ARTIFACT" --version)" == "termux-mcp-server $EXPECTED_VERSION" ]] || fail mcp_version_mismatch
[[ "$(timeout -k 2 5 "$VOLUME_CONTROL_ARTIFACT" --version)" == "termux-mcp-server $EXPECTED_VERSION" ]] || fail volume_control_version_mismatch
[[ "$(timeout -k 2 5 "$FULL_SUITE_ARTIFACT" --version)" == "termux-mcp-server $EXPECTED_VERSION" ]] || fail full_suite_version_mismatch
FULL_SUITE_IDENTITY="$(file -b "$FULL_SUITE_ARTIFACT")" || fail full_suite_identity_failed
[[ "$FULL_SUITE_IDENTITY" == *ELF* && "$FULL_SUITE_IDENTITY" == *"ARM aarch64"* ]] || fail full_suite_architecture_mismatch
[[ "$FULL_SUITE_IDENTITY" == *Android* || "$FULL_SUITE_IDENTITY" == *"/system/bin/linker64"* ]] || fail full_suite_android_identity_missing

OUTPUT_PARENT="$(dirname "$OUTPUT_REPORT")"
[[ -d "$OUTPUT_PARENT" && ! -L "$OUTPUT_PARENT" ]] || fail output_parent_invalid
[[ "$(realpath -e "$OUTPUT_PARENT")" == "$OUTPUT_PARENT" ]] || fail output_parent_not_canonical
[[ "$(stat -c %a "$OUTPUT_PARENT")" == 700 ]] || fail output_parent_not_private
[[ ! -e "$OUTPUT_REPORT" && ! -L "$OUTPUT_REPORT" ]] || fail output_already_exists

WORK_ROOT="$(mktemp -d "$HOME/.termux-mcp-emulated-gate.XXXXXX")" || fail work_root_create_failed
chmod 700 "$WORK_ROOT"
SAFE_PARENT="$WORK_ROOT/safe-parent"
SAFE_ROOT="$SAFE_PARENT/safe-root"
TOKEN_FILE="$WORK_ROOT/token"
CONFIG_FILE="$WORK_ROOT/release-validator.env"
RUNTIME_REPORT="$WORK_ROOT/runtime-evidence.json"
STRESS_LOG="$WORK_ROOT/stress-server.log"
BODY_FILE="$WORK_ROOT/body.json"
HEADER_FILE="$WORK_ROOT/headers.txt"
REQUEST_FILE="$WORK_ROOT/request.json"
mkdir -m 700 "$SAFE_PARENT"
mkdir -m 700 "$SAFE_ROOT"
printf '%s' emulated-visible >"$SAFE_ROOT/visible.txt"
chmod 600 "$SAFE_ROOT/visible.txt"

MCP_TOKEN="$(dd if=/dev/urandom bs=32 count=1 status=none | sha256sum | awk '{print $1}')"
[[ "$MCP_TOKEN" =~ ^[0-9a-f]{64}$ ]] || fail token_generation_failed
printf '%s' "$MCP_TOKEN" >"$TOKEN_FILE"
chmod 600 "$TOKEN_FILE"

# The canonical validator and the aggregate gate both exercise the exact
# full-suite artifact. Install fixed-path, sanitized Termux:API fixtures before
# validation so every provider runs through its production process boundary.
VOLUME_STATE="$WORK_ROOT/full-suite-music-level"
VOLUME_CALLS="$WORK_ROOT/full-suite-volume-calls"
printf '5\n' >"$VOLUME_STATE"
chmod 600 "$VOLUME_STATE"
BATTERY_PROGRAM="$PREFIX/bin/termux-battery-status"
VOLUME_PROGRAM="$PREFIX/bin/termux-volume"
[[ "$BATTERY_PROGRAM" == /data/data/com.termux/files/usr/bin/termux-battery-status ]] || fail aggregate_battery_program_path_invalid
[[ "$VOLUME_PROGRAM" == /data/data/com.termux/files/usr/bin/termux-volume ]] || fail aggregate_volume_program_path_invalid
[[ ! -e "$BATTERY_PROGRAM" && ! -L "$BATTERY_PROGRAM" ]] || fail aggregate_battery_program_already_present
[[ ! -e "$VOLUME_PROGRAM" && ! -L "$VOLUME_PROGRAM" ]] || fail aggregate_volume_program_already_present
BATTERY_PROGRAM_CREATED=true
VOLUME_PROGRAM_CREATED=true

cat >"$WORK_ROOT/full-suite-battery.next" <<'EOF'
#!/data/data/com.termux/files/usr/bin/bash
set -euo pipefail
[[ "$#" -eq 0 ]]
[[ "$PWD" == / ]]
[[ "$(/data/data/com.termux/files/usr/bin/readlink /proc/self/fd/0)" == /dev/null ]]
[[ "$(/data/data/com.termux/files/usr/bin/env | /data/data/com.termux/files/usr/bin/awk '/^MCP__/{count++} END{print count+0}')" == 0 ]]
printf '%s' '{"present":true,"health":"GOOD","plugged":"PLUGGED_USB","status":"CHARGING","temperature":30.5,"voltage":4200,"current":123000,"current_average":120000,"percentage":88,"level":88,"scale":100,"charge_counter":4000000,"energy":16000000,"cycle":200}'
EOF
chmod 700 "$WORK_ROOT/full-suite-battery.next"
install -m 700 "$WORK_ROOT/full-suite-battery.next" "$BATTERY_PROGRAM"

cat >"$WORK_ROOT/full-suite-volume.next" <<EOF
#!/data/data/com.termux/files/usr/bin/bash
set -euo pipefail
state='$VOLUME_STATE'
calls='$VOLUME_CALLS'
[[ "\$PWD" == / ]]
[[ "\$(/data/data/com.termux/files/usr/bin/readlink /proc/self/fd/0)" == /dev/null ]]
[[ "\$(/data/data/com.termux/files/usr/bin/env | /data/data/com.termux/files/usr/bin/awk '/^MCP__/{count++} END{print count+0}')" == 0 ]]
if ((\$# == 0)); then
  IFS= read -r music <"\$state"
  printf '[{"stream":"alarm","volume":4,"max_volume":7},{"stream":"call","volume":1,"max_volume":5},{"stream":"music","volume":%s,"max_volume":15},{"stream":"notification","volume":3,"max_volume":7},{"stream":"ring","volume":6,"max_volume":7},{"stream":"system","volume":2,"max_volume":7}]' "\$music"
  exit 0
fi
[[ \$# -eq 2 && "\$1" == music && "\$2" =~ ^[0-9]+$ ]]
printf '%s|%s\n' "\$1" "\$2" >>"\$calls"
printf '%s\n' "\$2" >"\$state"
EOF
chmod 700 "$WORK_ROOT/full-suite-volume.next"
install -m 700 "$WORK_ROOT/full-suite-volume.next" "$VOLUME_PROGRAM"

cat >"$CONFIG_FILE" <<EOF
EXPECTED_COMMIT=$EXPECTED_COMMIT
EXPECTED_VERSION=$EXPECTED_VERSION
DEFAULT_ARTIFACT=$DEFAULT_ARTIFACT
DEFAULT_SHA256=$DEFAULT_SHA
DEFAULT_MANIFEST=$DEFAULT_MANIFEST
MCP_ARTIFACT=$MCP_ARTIFACT
MCP_SHA256=$MCP_SHA
MCP_MANIFEST=$MCP_MANIFEST
VOLUME_CONTROL_ARTIFACT=$VOLUME_CONTROL_ARTIFACT
VOLUME_CONTROL_SHA256=$VOLUME_CONTROL_SHA
VOLUME_CONTROL_MANIFEST=$VOLUME_CONTROL_MANIFEST
FULL_SUITE_ARTIFACT=$FULL_SUITE_ARTIFACT
FULL_SUITE_SHA256=$FULL_SUITE_SHA
FULL_SUITE_MANIFEST=$FULL_SUITE_MANIFEST
AUTH_TOKEN_FILE=$TOKEN_FILE
SAFE_ROOT=$SAFE_ROOT
BIND_HOST=127.0.0.1
PORT=$PORT
DEPLOY_SCRIPT=$DEPLOY_SCRIPT
CI_RUN_ID=$CI_RUN_ID
SECURITY_RUN_ID=$SECURITY_RUN_ID
ANDROID_RUN_ID=$ANDROID_RUN_ID
SUSTAINED_OBSERVATION_STATUS=not_run
SUSTAINED_OBSERVATION_MINUTES=0
SUSTAINED_OBSERVATION_REASON_CODE=not_observed
EOF
chmod 600 "$CONFIG_FILE"

log 'running canonical exact-artifact runtime validator'
bash "$VALIDATOR" \
  --config "$CONFIG_FILE" \
  --report "$RUNTIME_REPORT" \
  --phase runtime \
  --confirm-runtime-mutation

jq -e \
  --arg commit "$EXPECTED_COMMIT" \
  --arg version "$EXPECTED_VERSION" \
  --arg ci "$CI_RUN_ID" \
  --arg security "$SECURITY_RUN_ID" \
  --arg android "$ANDROID_RUN_ID" '
    .status == "pass"
    and .failureCode == null
    and .validatorVersion == "11"
    and .releaseEligible == false
    and .repository == {commit:$commit,version:$version,ciRunId:$ci,securityRunId:$security,androidRunId:$android}
    and .phases == {preflight:"pass",runtime:"pass",deployment:"not_run"}
    and .sustainedObservation.status == "not_run"
    and ([.results[].code] | index("default_posture_verified") != null)
    and ([.results[].code] | index("mcp_posture_verified") != null)
    and ([.results[].code] | index("volume_control_posture_verified") != null)
    and ([.results[].code] | index("volume_control_hidden_while_disabled") != null)
    and ([.results[].code] | index("volume_control_disabled_call_rejected") != null)
    and ([.results[].code] | index("exact_tool_allowlist") != null)
    and ([.results[].code] | index("request_scoped_single_use_grant_enforced") != null)
    and ([.results[].code] | index("request_scoped_single_use_copy_grant_enforced") != null)
    and ([.results[].code] | index("source_content_destination_binding_enforced") != null)
    and ([.results[].code] | index("exact_binary_copy_verified") != null)
    and ([.results[].code] | index("copy_file_boundary_denials_verified") != null)
    and ([.results[].code] | index("copy_file_private_audit_verified") != null)
    and ([.results[].code] | index("copy_file_disabled_posture_verified") != null)
    and ([.results[].code] | index("safe_root_file_trash_verified") != null)
    and ([.results[].code] | index("request_scoped_trash_grant_enforced") != null)
    and ([.results[].code] | index("trash_identity_content_binding_enforced") != null)
    and ([.results[].code] | index("bounded_trash_file_response_preflight_verified") != null)
    and ([.results[].code] | index("exact_trash_file_byte_limit_verified") != null)
    and ([.results[].code] | index("trash_recovery_quarantine_verified") != null)
    and ([.results[].code] | index("trash_file_private_audit_verified") != null)
    and ([.results[].code] | index("trash_file_disabled_posture_verified") != null)
    and ([.results[].code] | index("expanded_body_posture_verified") != null)
    and ([.results[].code] | index("safe_root_file_create_replace_verified") != null)
    and ([.results[].code] | index("request_scoped_single_use_write_grant_enforced") != null)
    and ([.results[].code] | index("exact_write_file_byte_limit_verified") != null)
    and ([.results[].code] | index("bounded_write_file_response_preflight_verified") != null)
    and ([.results[].code] | index("symlink_escape_rejected") != null)
    and ([.results[].code] | index("authentication_precedes_body_limit") != null)
    and ([.results[].code] | index("full_suite_default_disabled_17_tool_posture_verified") != null)
    and ([.results[].code] | index("full_suite_enabled_21_tool_posture_verified") != null)
    and ([.results[].code] | index("full_suite_optional_provider_success_verified") != null)
    and ([.results[].code] | index("full_suite_volume_preview_and_grant_boundary_verified") != null)
    and ([.results[].code] | index("full_suite_command_basename_and_profile_verified") != null)
    and ([.results[].code] | index("full_suite_filesystem_mutations_independently_disabled") != null)
  ' "$RUNTIME_REPORT" >/dev/null || fail runtime_report_invalid
RUNTIME_REPORT_SHA="$(sha256sum "$RUNTIME_REPORT" | awk '{print $1}')"
RUNTIME_RESULT_COUNT="$(jq '.results | length' "$RUNTIME_REPORT")"

curl_local() {
  command curl --disable --proto '=http' --noproxy '*' --connect-timeout 2 --max-time 10 "$@"
}

start_stress_server() {
  MCP__AUTH__STATIC_TOKEN="$MCP_TOKEN" \
  MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=false \
  MCP__SERVER__HOST=127.0.0.1 \
  MCP__SERVER__PORT="$PORT" \
  MCP__TRANSPORT__ALLOWED_HOSTS="localhost:$PORT,127.0.0.1:$PORT" \
  MCP__TRANSPORT__ALLOWED_ORIGINS="http://localhost:$PORT,http://127.0.0.1:$PORT" \
  MCP__TRANSPORT__ALLOW_MISSING_ORIGIN=false \
  MCP__TRANSPORT__SSE_ENABLED=false \
  MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS=4 \
  MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS=30 \
  MCP__TRANSPORT__MAX_BODY_BYTES=1024 \
  MCP__FILE__SAFE_ROOTS="$SAFE_ROOT" \
  MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=false \
  MCP__FILE__COPY_FILE_MUTATION_ENABLED=false \
  MCP__FILE__TRASH_FILE_MUTATION_ENABLED=false \
  MCP__FILE__WRITE_MUTATION_ENABLED=false \
  RUST_LOG=termux_mcp_server=info \
    "$MCP_ARTIFACT" >"$STRESS_LOG" 2>&1 &
  SERVER_PID=$!
  local attempt
  for attempt in $(seq 1 100); do
    kill -0 "$SERVER_PID" >/dev/null 2>&1 || fail stress_server_exited
    if [[ "$(curl_local --silent --max-time 2 "http://127.0.0.1:$PORT/health" 2>/dev/null || true)" == ok ]]; then
      return 0
    fi
    sleep 0.1
  done
  fail "stress_server_not_ready_after_${attempt}_attempts"
}

post_mcp() {
  local payload="$1" session="${2:-}" grant="${3:-}"
  printf '%s' "$payload" >"$REQUEST_FILE"
  local -a headers=(
    -H "Authorization: Bearer $MCP_TOKEN"
    -H "Host: localhost:$PORT"
    -H "Origin: http://localhost:$PORT"
    -H 'Content-Type: application/json'
    -H 'Accept: application/json, text/event-stream'
  )
  if [[ -n "$session" ]]; then
    headers+=( -H "MCP-Session-Id: $session" -H 'MCP-Protocol-Version: 2025-11-25' )
  fi
  if [[ -n "$grant" ]]; then
    headers+=( -H "MCP-Capability-Grant: $grant" )
  fi
  MCP_STATUS="$(curl_local --silent --show-error --output "$BODY_FILE" --write-out '%{http_code}' \
    "${headers[@]}" --data-binary "@$REQUEST_FILE" "http://127.0.0.1:$PORT/mcp")"
  REQUEST_COUNT=$((REQUEST_COUNT + 1))
}

log "running $SAMPLES-sample native ARM64 stress pass"
start_stress_server
STRESS_PID="$SERVER_PID"

printf '%s' '{"jsonrpc":"2.0","id":"initialize","method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"termux-emulated-gate","version":"1"}}}' >"$REQUEST_FILE"
status="$(curl_local --silent --show-error --dump-header "$HEADER_FILE" --output "$BODY_FILE" --write-out '%{http_code}' \
  -H "Authorization: Bearer $MCP_TOKEN" \
  -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
  -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  --data-binary "@$REQUEST_FILE" "http://127.0.0.1:$PORT/mcp")"
REQUEST_COUNT=$((REQUEST_COUNT + 1))
[[ "$status" == 200 ]] || fail stress_initialize_status_invalid
jq -e --arg version "$EXPECTED_VERSION" '.result.protocolVersion == "2025-11-25" and .result.serverInfo.version == $version' "$BODY_FILE" >/dev/null || fail stress_initialize_body_invalid
SESSION_ID="$(awk 'tolower($1) == "mcp-session-id:" {sub(/^[^:]*:[[:space:]]*/, ""); sub(/\r$/, ""); print; exit}' "$HEADER_FILE")"
[[ "$SESSION_ID" =~ ^[A-Za-z0-9-]{1,128}$ ]] || fail stress_session_invalid

post_mcp '{"jsonrpc":"2.0","method":"notifications/initialized"}' "$SESSION_ID"
[[ "$MCP_STATUS" == 202 && ! -s "$BODY_FILE" ]] || fail stress_initialized_notification_invalid

# Prove that every operation continues from the descriptor pinned at startup,
# even after both the configured root pathname and one of its ancestors are
# renamed and replaced with attacker-controlled directories.
PINNED_ROOT="$SAFE_PARENT/original-safe-root"
mv -- "$SAFE_ROOT" "$PINNED_ROOT"
mkdir -m 700 "$SAFE_ROOT"
printf '%s' root-replacement >"$SAFE_ROOT/visible.txt"
chmod 600 "$SAFE_ROOT/visible.txt"
post_mcp "$(jq -cn --arg path "$SAFE_ROOT/visible.txt" '{jsonrpc:"2.0",id:"root-identity",method:"tools/call",params:{name:"read_file",arguments:{path:$path}}}')" "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail stress_root_identity_status_invalid
jq -e '.result.structuredContent.content == "emulated-visible"' "$BODY_FILE" >/dev/null || fail stress_root_identity_redirected
[[ "$(cat "$SAFE_ROOT/visible.txt")" == root-replacement ]] || fail stress_root_replacement_fixture_invalid
[[ "$(cat "$PINNED_ROOT/visible.txt")" == emulated-visible ]] || fail stress_pinned_root_fixture_invalid

PINNED_PARENT="$WORK_ROOT/original-safe-parent"
mv -- "$SAFE_PARENT" "$PINNED_PARENT"
mkdir -m 700 "$SAFE_PARENT"
mkdir -m 700 "$SAFE_ROOT"
printf '%s' ancestor-replacement >"$SAFE_ROOT/visible.txt"
chmod 600 "$SAFE_ROOT/visible.txt"
post_mcp "$(jq -cn --arg path "$SAFE_ROOT/visible.txt" '{jsonrpc:"2.0",id:"ancestor-identity",method:"tools/call",params:{name:"read_file",arguments:{path:$path}}}')" "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail stress_ancestor_identity_status_invalid
jq -e '.result.structuredContent.content == "emulated-visible"' "$BODY_FILE" >/dev/null || fail stress_ancestor_identity_redirected
[[ "$(cat "$SAFE_ROOT/visible.txt")" == ancestor-replacement ]] || fail stress_ancestor_replacement_fixture_invalid
[[ "$(cat "$PINNED_PARENT/original-safe-root/visible.txt")" == emulated-visible ]] || fail stress_pinned_ancestor_fixture_invalid

for sample in $(seq 1 "$SAMPLES"); do
  kill -0 "$STRESS_PID" >/dev/null 2>&1 || fail stress_pid_lost
  [[ "$SERVER_PID" == "$STRESS_PID" ]] || fail stress_pid_changed
  [[ "$(curl_local --fail --silent "http://127.0.0.1:$PORT/health")" == ok ]] || fail stress_health_invalid
  REQUEST_COUNT=$((REQUEST_COUNT + 1))
  curl_local --fail --silent "http://127.0.0.1:$PORT/ready" >"$BODY_FILE"
  REQUEST_COUNT=$((REQUEST_COUNT + 1))
  jq -e --arg version "$EXPECTED_VERSION" '
    .status == "ready" and .version == $version and .mcp_runtime_enabled == true
    and .safe_root_count == 1 and .auth_posture == "static_token"
  ' "$BODY_FILE" >/dev/null || fail stress_readiness_invalid

  post_mcp '{"jsonrpc":"2.0","id":"tools","method":"tools/list"}' "$SESSION_ID"
  [[ "$MCP_STATUS" == 200 ]] || fail stress_tools_status_invalid
  jq -e '[.result.tools[].name] == ["runtime_status","platform_info","android_status","project_service_status","create_directory","copy_file","trash_file","find_paths","hash_file","list_directory","path_metadata","read_binary_file","read_binary_range","read_file","read_text_range","search_text","write_file"]' "$BODY_FILE" >/dev/null || fail stress_tool_allowlist_invalid
  jq -e '
    .result.tools
    | map(select(.name == "create_directory"))[0] as $tool
    | $tool.inputSchema.properties.dry_run.const == true
      and ($tool.description | contains("mutation gate is disabled"))
  ' "$BODY_FILE" >/dev/null || fail stress_create_directory_disabled_posture_invalid
  jq -e '
    .result.tools
    | map(select(.name == "copy_file"))[0] as $tool
    | $tool.inputSchema.properties.dry_run.const == true
      and ($tool.description | contains("copy mutation gate is disabled"))
  ' "$BODY_FILE" >/dev/null || fail stress_copy_file_disabled_posture_invalid
  jq -e '
    .result.tools
    | map(select(.name == "trash_file"))[0] as $tool
    | $tool.inputSchema.type == "object"
      and ($tool.inputSchema.properties | keys) == ["dry_run","path"]
      and $tool.inputSchema.properties.path.type == "string"
      and $tool.inputSchema.properties.dry_run.type == "boolean"
      and $tool.inputSchema.properties.dry_run.const == true
      and $tool.inputSchema.required == ["path"]
      and $tool.inputSchema.additionalProperties == false
      and ($tool.description | contains("dedicated trash mutation gate is disabled"))
  ' "$BODY_FILE" >/dev/null || fail stress_trash_file_disabled_posture_invalid
  jq -e '
    .result.tools
    | map(select(.name == "write_file"))[0] as $tool
    | $tool.inputSchema.properties.dry_run.const == true
      and ($tool.description | contains("mutation gate is disabled"))
  ' "$BODY_FILE" >/dev/null || fail stress_write_file_disabled_posture_invalid

  if ((sample % 16 == 0)); then
    post_mcp '{"jsonrpc":"2.0","id":"runtime","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}' "$SESSION_ID"
    [[ "$MCP_STATUS" == 200 ]] || fail stress_runtime_status_invalid
    jq -e '
      .result.structuredContent.commandExecution == false
      and .result.structuredContent.androidPlatformTools == false
      and .result.structuredContent.androidBatteryStatusCompiled == false
      and .result.structuredContent.androidBatteryStatusEnabled == false
      and .result.structuredContent.androidVolumeStatusCompiled == false
      and .result.structuredContent.androidVolumeStatusEnabled == false
      and .result.structuredContent.androidDeviceControl == false
      and .result.structuredContent.createDirectoryMutationEnabled == false
      and .result.structuredContent.createDirectoryGrantRequired == false
      and .result.structuredContent.createDirectoryMutationMode == "dry_run_only_mutation_disabled"
      and .result.structuredContent.copyFileMutationEnabled == false
      and .result.structuredContent.copyFileMode == "dry_run_only_mutation_disabled"
      and .result.structuredContent.copyFileGrantRequired == false
      and .result.structuredContent.copyFileGrantHeader == "mcp-capability-grant"
      and .result.structuredContent.copyFileGrantTtlSeconds == 60
      and .result.structuredContent.copyFileGrantBinding == "source_root_path_identity_size_sha256_destination_root_path_absent_no_replace"
      and .result.structuredContent.copyFileMaxBytes == 1048576
      and .result.structuredContent.copyFileMaxResponseBytes == 16384
      and .result.structuredContent.copyFileResponsePosture == "path_free_bounded_metadata_only"
      and .result.structuredContent.trashFileMutationEnabled == false
      and .result.structuredContent.trashFileMode == "dry_run_only_mutation_disabled"
      and .result.structuredContent.trashFileGrantRequired == false
      and .result.structuredContent.trashFileGrantHeader == "mcp-capability-grant"
      and .result.structuredContent.trashFileGrantTtlSeconds == 60
      and .result.structuredContent.trashFileGrantBinding == "root_path_single_link_identity_size_ctime_sha256_recovery_retained"
      and .result.structuredContent.trashFileMaxBytes == 1048576
      and .result.structuredContent.trashFileMaxResponseBytes == 16384
      and .result.structuredContent.trashFileQuarantineMaxArtifacts == 32
      and .result.structuredContent.trashFileQuarantineMaxBytes == 33554432
      and .result.structuredContent.trashFileResponsePosture == "path_and_artifact_free_bounded_metadata_only"
      and .result.structuredContent.fileWrites == true
      and .result.structuredContent.fileWriteMutationEnabled == false
      and .result.structuredContent.fileWriteGrantRequired == false
      and .result.structuredContent.fileWriteMode == "dry_run_only_mutation_disabled"
      and .result.structuredContent.fileWriteGrantHeader == "mcp-capability-grant"
      and .result.structuredContent.fileWriteGrantTtlSeconds == 60
      and .result.structuredContent.fileWriteMaxBytes == 1048576
      and .result.structuredContent.fileWriteMaxResponseBytes == 16384
      and .result.structuredContent.highImpactTools == false
    ' "$BODY_FILE" >/dev/null || fail stress_high_impact_gate_invalid
  fi

  if ((sample == 1)); then
    post_mcp "$(jq -cn --arg source "$SAFE_ROOT/visible.txt" --arg destination "$SAFE_ROOT/copy-disabled.txt" '{jsonrpc:"2.0",id:"copy-disabled",method:"tools/call",params:{name:"copy_file",arguments:{source_path:$source,destination_path:$destination,dry_run:false}}}')" "$SESSION_ID"
    [[ "$MCP_STATUS" == 403 ]] || fail stress_copy_file_disabled_status_invalid
    jq -e '.error.code == -32003 and .error.data.reason == "copy_file_mutation_disabled"' "$BODY_FILE" >/dev/null || fail stress_copy_file_disabled_contract_invalid
    [[ ! -e "$SAFE_ROOT/copy-disabled.txt" && ! -L "$SAFE_ROOT/copy-disabled.txt" ]] || fail stress_copy_file_disabled_replacement_mutated
    [[ ! -e "$PINNED_PARENT/original-safe-root/copy-disabled.txt" && ! -L "$PINNED_PARENT/original-safe-root/copy-disabled.txt" ]] || fail stress_copy_file_disabled_pinned_root_mutated

    post_mcp "$(jq -cn --arg path "$SAFE_ROOT/visible.txt" '{jsonrpc:"2.0",id:"trash-disabled",method:"tools/call",params:{name:"trash_file",arguments:{path:$path,dry_run:false}}}')" "$SESSION_ID"
    [[ "$MCP_STATUS" == 403 ]] || fail stress_trash_file_disabled_status_invalid
    jq -e '.error.code == -32003 and .error.data.reason == "trash_file_mutation_disabled"' "$BODY_FILE" >/dev/null || fail stress_trash_file_disabled_contract_invalid
    [[ -f "$SAFE_ROOT/visible.txt" && "$(cat "$SAFE_ROOT/visible.txt")" == ancestor-replacement ]] || fail stress_trash_file_disabled_replacement_target_mutated
    [[ -f "$PINNED_PARENT/original-safe-root/visible.txt" && "$(cat "$PINNED_PARENT/original-safe-root/visible.txt")" == emulated-visible ]] || fail stress_trash_file_disabled_pinned_target_mutated
    [[ ! -e "$SAFE_ROOT/.termux-mcp-trash-quarantine" && ! -L "$SAFE_ROOT/.termux-mcp-trash-quarantine" ]] || fail stress_trash_file_disabled_replacement_quarantine_mutated
    [[ ! -e "$PINNED_PARENT/original-safe-root/.termux-mcp-trash-quarantine" && ! -L "$PINNED_PARENT/original-safe-root/.termux-mcp-trash-quarantine" ]] || fail stress_trash_file_disabled_pinned_quarantine_mutated

    post_mcp "$(jq -cn --arg path "$SAFE_ROOT/write-disabled.txt" '{jsonrpc:"2.0",id:"write-disabled",method:"tools/call",params:{name:"write_file",arguments:{path:$path,content:"inert",dry_run:false}}}')" "$SESSION_ID"
    [[ "$MCP_STATUS" == 403 ]] || fail stress_write_file_disabled_status_invalid
    jq -e '.error.code == -32003 and .error.data.reason == "write_file_mutation_disabled"' "$BODY_FILE" >/dev/null || fail stress_write_file_disabled_contract_invalid
    [[ ! -e "$SAFE_ROOT/write-disabled.txt" && ! -L "$SAFE_ROOT/write-disabled.txt" ]] || fail stress_write_file_disabled_replacement_mutated
    [[ ! -e "$PINNED_PARENT/original-safe-root/write-disabled.txt" && ! -L "$PINNED_PARENT/original-safe-root/write-disabled.txt" ]] || fail stress_write_file_disabled_pinned_root_mutated

    post_mcp '{"jsonrpc":"2.0","id":"battery-uncompiled","method":"tools/call","params":{"name":"android_battery_status","arguments":{}}}' "$SESSION_ID"
    [[ "$MCP_STATUS" == 200 ]] || fail stress_battery_uncompiled_status_invalid
    jq -e '
      .result.isError == true
      and .result.structuredContent.error == "android_battery_status_unavailable"
      and .result.structuredContent.reasonCode == "battery_feature_not_compiled"
    ' "$BODY_FILE" >/dev/null || fail stress_battery_uncompiled_contract_invalid

    post_mcp '{"jsonrpc":"2.0","id":"volume-uncompiled","method":"tools/call","params":{"name":"android_volume_status","arguments":{}}}' "$SESSION_ID"
    [[ "$MCP_STATUS" == 200 ]] || fail stress_volume_uncompiled_status_invalid
    jq -e '
      .result.isError == true
      and .result.structuredContent.error == "android_volume_status_unavailable"
      and .result.structuredContent.reasonCode == "volume_feature_not_compiled"
    ' "$BODY_FILE" >/dev/null || fail stress_volume_uncompiled_contract_invalid
  fi
done

status="$(curl_local --silent --show-error --output "$BODY_FILE" --write-out '%{http_code}' -X DELETE \
  -H "Authorization: Bearer $MCP_TOKEN" -H "MCP-Session-Id: $SESSION_ID" \
  -H 'MCP-Protocol-Version: 2025-11-25' \
  -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
  "http://127.0.0.1:$PORT/mcp")"
REQUEST_COUNT=$((REQUEST_COUNT + 1))
[[ "$status" == 204 && ! -s "$BODY_FILE" ]] || fail stress_session_delete_invalid
unset SESSION_ID

terminate_server_pid_bounded "$SERVER_PID" || fail stress_shutdown_not_bounded
SERVER_PID=''
STRESS_REQUEST_COUNT="$REQUEST_COUNT"

log 'validating aggregate full-suite runtime truth table'
AGGREGATE_REQUEST_START="$REQUEST_COUNT"
AGGREGATE_CLEANUPS=0
AGGREGATE_SERVER_LOG="$WORK_ROOT/full-suite-server.log"
AGGREGATE_CAPABILITY_CONFIG="$WORK_ROOT/full-suite-runtime.env"
AGGREGATE_GRANT_FILE="$WORK_ROOT/full-suite-volume-grant"
FULL_SUITE_EXECUTABLE_DIR="$WORK_ROOT/full-suite-executable"
FULL_SUITE_EXECUTABLE="$FULL_SUITE_EXECUTABLE_DIR/termux-mcp-server"
FULL_SUITE_REPLACEMENT_MARKER="$WORK_ROOT/full-suite-replacement-ran"
mkdir -m 700 "$FULL_SUITE_EXECUTABLE_DIR"
install -m 700 "$FULL_SUITE_ARTIFACT" "$FULL_SUITE_EXECUTABLE"
[[ "$(sha256sum "$FULL_SUITE_EXECUTABLE" | awk '{print $1}')" == "$FULL_SUITE_SHA" ]] || fail full_suite_execution_copy_digest_mismatch

AGGREGATE_CREATE_TARGET="$SAFE_ROOT/full-suite-create-disabled"
AGGREGATE_COPY_SOURCE="$SAFE_ROOT/full-suite-copy-source.txt"
AGGREGATE_COPY_DESTINATION="$SAFE_ROOT/full-suite-copy-disabled.txt"
AGGREGATE_TRASH_TARGET="$SAFE_ROOT/full-suite-trash-target.txt"
AGGREGATE_TRASH_QUARANTINE="$SAFE_ROOT/.termux-mcp-trash-quarantine"
AGGREGATE_WRITE_DESTINATION="$SAFE_ROOT/full-suite-write-disabled.txt"
for path in \
  "$AGGREGATE_CREATE_TARGET" "$AGGREGATE_COPY_SOURCE" "$AGGREGATE_COPY_DESTINATION" \
  "$AGGREGATE_TRASH_TARGET" "$AGGREGATE_TRASH_QUARANTINE" "$AGGREGATE_WRITE_DESTINATION"; do
  [[ ! -e "$path" && ! -L "$path" ]] || fail aggregate_filesystem_fixture_path_occupied
done
printf '%s' aggregate-copy-source >"$AGGREGATE_COPY_SOURCE"
printf '%s' aggregate-trash-target >"$AGGREGATE_TRASH_TARGET"
chmod 600 "$AGGREGATE_COPY_SOURCE" "$AGGREGATE_TRASH_TARGET"
AGGREGATE_COPY_SOURCE_SHA="$(sha256sum "$AGGREGATE_COPY_SOURCE" | awk '{print $1}')"
AGGREGATE_COPY_SOURCE_STATE="$(stat -c '%d:%i:%s:%f:%y:%z' "$AGGREGATE_COPY_SOURCE")"
AGGREGATE_TRASH_TARGET_SHA="$(sha256sum "$AGGREGATE_TRASH_TARGET" | awk '{print $1}')"
AGGREGATE_TRASH_TARGET_STATE="$(stat -c '%d:%i:%s:%f:%y:%z' "$AGGREGATE_TRASH_TARGET")"

CAPABILITY_KEY="$(dd if=/dev/urandom bs=32 count=1 status=none | sha256sum | awk '{print $1}')"
[[ "$CAPABILITY_KEY" =~ ^[0-9a-f]{64}$ ]] || fail aggregate_capability_key_generation_failed
cat >"$AGGREGATE_CAPABILITY_CONFIG" <<EOF
MCP__AUTH__STATIC_TOKEN=$MCP_TOKEN
MCP__CAPABILITY__KEY_ID=native-full-suite-1
MCP__CAPABILITY__HMAC_KEY_HEX=$CAPABILITY_KEY
MCP__ANDROID__BATTERY_STATUS_ENABLED=true
MCP__ANDROID__VOLUME_STATUS_ENABLED=true
MCP__ANDROID__VOLUME_CONTROL_ENABLED=true
MCP__COMMAND__ENABLED=true
MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=false
MCP__FILE__COPY_FILE_MUTATION_ENABLED=false
MCP__FILE__TRASH_FILE_MUTATION_ENABLED=false
MCP__FILE__WRITE_MUTATION_ENABLED=false
MCP__FILE__SAFE_ROOTS=$SAFE_ROOT
EOF
chmod 600 "$AGGREGATE_CAPABILITY_CONFIG"

start_aggregate_server() {
  local posture="$1"
  local -a cleared_runtime_environment=(
    -u MCP__ANDROID__BATTERY_STATUS_ENABLED
    -u MCP__ANDROID__VOLUME_STATUS_ENABLED
    -u MCP__ANDROID__VOLUME_CONTROL_ENABLED
    -u MCP__COMMAND__ENABLED
  )
  local -a posture_environment=()
  [[ "$(sha256sum "$FULL_SUITE_EXECUTABLE" | awk '{print $1}')" == "$FULL_SUITE_SHA" ]] || fail aggregate_execution_copy_digest_mismatch
  case "$posture" in
    default-disabled) ;;
    battery-only)
      posture_environment=(MCP__ANDROID__BATTERY_STATUS_ENABLED=true)
      ;;
    volume-status-only)
      posture_environment=(MCP__ANDROID__VOLUME_STATUS_ENABLED=true)
      ;;
    volume-control-only)
      posture_environment=(MCP__ANDROID__VOLUME_CONTROL_ENABLED=true)
      ;;
    command-only)
      posture_environment=(MCP__COMMAND__ENABLED=true)
      ;;
    fully-enabled)
      posture_environment=(
        MCP__ANDROID__BATTERY_STATUS_ENABLED=true
        MCP__ANDROID__VOLUME_STATUS_ENABLED=true
        MCP__ANDROID__VOLUME_CONTROL_ENABLED=true
        MCP__COMMAND__ENABLED=true
      )
      ;;
    *) fail aggregate_posture_invalid ;;
  esac
  env "${cleared_runtime_environment[@]}" "${posture_environment[@]}" \
    MCP__AUTH__STATIC_TOKEN="$MCP_TOKEN" \
    MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=false \
    MCP__CAPABILITY__KEY_ID=native-full-suite-1 \
    MCP__CAPABILITY__HMAC_KEY_HEX="$CAPABILITY_KEY" \
    MCP__SERVER__HOST=127.0.0.1 \
    MCP__SERVER__PORT="$PORT" \
    MCP__TRANSPORT__ALLOWED_HOSTS="localhost:$PORT,127.0.0.1:$PORT" \
    MCP__TRANSPORT__ALLOWED_ORIGINS="http://localhost:$PORT,http://127.0.0.1:$PORT" \
    MCP__TRANSPORT__ALLOW_MISSING_ORIGIN=false \
    MCP__TRANSPORT__SSE_ENABLED=false \
    MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS=4 \
    MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS=30 \
    MCP__TRANSPORT__MAX_BODY_BYTES=32768 \
    MCP__FILE__SAFE_ROOTS="$SAFE_ROOT" \
    MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=false \
    MCP__FILE__COPY_FILE_MUTATION_ENABLED=false \
    MCP__FILE__TRASH_FILE_MUTATION_ENABLED=false \
    MCP__FILE__WRITE_MUTATION_ENABLED=false \
    RUST_LOG=termux_mcp_server=info \
      "$FULL_SUITE_EXECUTABLE" >"$AGGREGATE_SERVER_LOG" 2>&1 &
  SERVER_PID=$!
  local attempt
  for attempt in $(seq 1 100); do
    kill -0 "$SERVER_PID" >/dev/null 2>&1 || fail aggregate_server_exited
    if [[ "$(curl_local --silent --max-time 2 "http://127.0.0.1:$PORT/health" 2>/dev/null || true)" == ok ]]; then
      return 0
    fi
    sleep 0.1
  done
  fail "aggregate_server_not_ready_after_${attempt}_attempts"
}

initialize_aggregate_session() {
  printf '%s' '{"jsonrpc":"2.0","id":"initialize","method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"termux-full-suite-emulated-gate","version":"1"}}}' >"$REQUEST_FILE"
  MCP_STATUS="$(curl_local --silent --show-error --dump-header "$HEADER_FILE" --output "$BODY_FILE" --write-out '%{http_code}' \
    -H "Authorization: Bearer $MCP_TOKEN" \
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
    --data-binary "@$REQUEST_FILE" "http://127.0.0.1:$PORT/mcp")"
  REQUEST_COUNT=$((REQUEST_COUNT + 1))
  [[ "$MCP_STATUS" == 200 ]] || fail aggregate_initialize_status_invalid
  jq -e --arg version "$EXPECTED_VERSION" '.result.protocolVersion == "2025-11-25" and .result.serverInfo.version == $version' "$BODY_FILE" >/dev/null || fail aggregate_initialize_body_invalid
  SESSION_ID="$(awk 'tolower($1) == "mcp-session-id:" {sub(/^[^:]*:[[:space:]]*/, ""); sub(/\r$/, ""); print; exit}' "$HEADER_FILE")"
  [[ "$SESSION_ID" =~ ^[A-Za-z0-9-]{1,128}$ ]] || fail aggregate_session_invalid
  post_mcp '{"jsonrpc":"2.0","method":"notifications/initialized"}' "$SESSION_ID"
  [[ "$MCP_STATUS" == 202 && ! -s "$BODY_FILE" ]] || fail aggregate_initialized_notification_invalid
}

close_aggregate_session() {
  local status
  status="$(curl_local --silent --show-error --output "$BODY_FILE" --write-out '%{http_code}' -X DELETE \
    -H "Authorization: Bearer $MCP_TOKEN" -H "MCP-Session-Id: $SESSION_ID" \
    -H 'MCP-Protocol-Version: 2025-11-25' \
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
    "http://127.0.0.1:$PORT/mcp")"
  REQUEST_COUNT=$((REQUEST_COUNT + 1))
  [[ "$status" == 204 && ! -s "$BODY_FILE" ]] || fail aggregate_session_delete_invalid
  SESSION_ID=''
}

stop_aggregate_server() {
  terminate_server_pid_bounded "$SERVER_PID" || fail aggregate_shutdown_not_bounded
  SERVER_PID=''
  AGGREGATE_CLEANUPS=$((AGGREGATE_CLEANUPS + 1))
}

validate_single_optional_gate_posture() {
  local posture="$1" selected_tool=''
  local battery_enabled=false volume_status_enabled=false volume_control_enabled=false command_enabled=false
  case "$posture" in
    battery-only)
      selected_tool=android_battery_status
      battery_enabled=true
      ;;
    volume-status-only)
      selected_tool=android_volume_status
      volume_status_enabled=true
      ;;
    volume-control-only)
      selected_tool=set_android_volume
      volume_control_enabled=true
      ;;
    command-only)
      selected_tool=run_command_profile
      command_enabled=true
      ;;
    *) fail aggregate_single_gate_posture_invalid ;;
  esac

  start_aggregate_server "$posture"
  initialize_aggregate_session
  post_mcp "$(jq -cn --arg id "full-suite-$posture-tools" '{jsonrpc:"2.0",id:$id,method:"tools/list"}')" "$SESSION_ID"
  [[ "$MCP_STATUS" == 200 ]] || fail "aggregate_${posture}_tools_status_invalid"
  jq -e --arg selected_tool "$selected_tool" '
    ["runtime_status","platform_info","android_status","project_service_status","create_directory","copy_file","trash_file","find_paths","hash_file","list_directory","path_metadata","read_binary_file","read_binary_range","read_file","read_text_range","search_text","write_file"] as $base_tools
    | [.result.tools[].name] == ($base_tools + [$selected_tool])
      and (.result.tools | length) == 18
  ' "$BODY_FILE" >/dev/null || fail "aggregate_${posture}_tool_allowlist_invalid"

  post_mcp "$(jq -cn --arg id "full-suite-$posture-runtime" '{jsonrpc:"2.0",id:$id,method:"tools/call",params:{name:"runtime_status",arguments:{}}}')" "$SESSION_ID"
  [[ "$MCP_STATUS" == 200 ]] || fail "aggregate_${posture}_runtime_status_invalid"
  jq -e \
    --arg selected_tool "$selected_tool" \
    --argjson battery_enabled "$battery_enabled" \
    --argjson volume_status_enabled "$volume_status_enabled" \
    --argjson volume_control_enabled "$volume_control_enabled" \
    --argjson command_enabled "$command_enabled" '
      ["runtime_status","platform_info","android_status","project_service_status","create_directory","copy_file","trash_file","find_paths","hash_file","list_directory","path_metadata","read_binary_file","read_binary_range","read_file","read_text_range","search_text","write_file"] as $base_tools
      | .result.structuredContent as $status
      | $status.availableTools == ($base_tools + [$selected_tool])
        and ($status.availableTools | length) == 18
        and $status.androidBatteryStatusCompiled == true
        and $status.androidBatteryStatusEnabled == $battery_enabled
        and $status.androidVolumeStatusCompiled == true
        and $status.androidVolumeStatusEnabled == $volume_status_enabled
        and $status.androidVolumeControlCompiled == true
        and $status.androidVolumeControlEnabled == $volume_control_enabled
        and $status.androidVolumeGrantRequired == $volume_control_enabled
        and $status.androidDeviceControl == $volume_control_enabled
        and $status.commandExecutionCompiled == true
        and $status.commandExecution == $command_enabled
        and $status.arbitraryCommandExecution == false
        and $status.androidPlatformTools == ($battery_enabled or $volume_status_enabled or $volume_control_enabled)
        and $status.highImpactTools == $volume_control_enabled
        and $status.createDirectoryMutationEnabled == false
        and $status.copyFileMutationEnabled == false
        and $status.trashFileMutationEnabled == false
        and $status.fileWriteMutationEnabled == false
    ' "$BODY_FILE" >/dev/null || fail "aggregate_${posture}_runtime_contract_invalid"

  case "$posture" in
    battery-only)
      post_mcp '{"jsonrpc":"2.0","id":"full-suite-battery-only-success","method":"tools/call","params":{"name":"android_battery_status","arguments":{}}}' "$SESSION_ID"
      [[ "$MCP_STATUS" == 200 ]] || fail aggregate_battery_only_success_status_invalid
      jq -e '.result.isError == false and .result.structuredContent.percentage == 88 and .result.structuredContent.cycle_count == 200' "$BODY_FILE" >/dev/null \
        || fail aggregate_battery_only_success_contract_invalid
      ;;
    volume-status-only)
      post_mcp '{"jsonrpc":"2.0","id":"full-suite-volume-status-only-success","method":"tools/call","params":{"name":"android_volume_status","arguments":{}}}' "$SESSION_ID"
      [[ "$MCP_STATUS" == 200 ]] || fail aggregate_volume_status_only_success_status_invalid
      jq -e '.result.isError == false and [.result.structuredContent.streams[].stream] == ["alarm","call","music","notification","ring","system"] and .result.structuredContent.streams[2] == {stream:"music",volume:5,maxVolume:15}' "$BODY_FILE" >/dev/null \
        || fail aggregate_volume_status_only_success_contract_invalid
      ;;
    volume-control-only)
      [[ "$(cat "$VOLUME_STATE")" == 5 && ! -e "$VOLUME_CALLS" ]] || fail aggregate_volume_control_only_fixture_invalid
      post_mcp '{"jsonrpc":"2.0","id":"full-suite-volume-control-only-success","method":"tools/call","params":{"name":"set_android_volume","arguments":{"stream":"music","level":9,"dry_run":true}}}' "$SESSION_ID"
      [[ "$MCP_STATUS" == 200 ]] || fail aggregate_volume_control_only_success_status_invalid
      jq -e '.result.isError == false and .result.structuredContent == {stream:"music",previousLevel:5,requestedLevel:9,maxVolume:15,dryRun:true,changed:false,verified:false,outcome:"preview",rollback:"not_required"}' "$BODY_FILE" >/dev/null \
        || fail aggregate_volume_control_only_success_contract_invalid
      [[ "$(cat "$VOLUME_STATE")" == 5 && ! -e "$VOLUME_CALLS" ]] || fail aggregate_volume_control_only_success_mutated
      ;;
    command-only)
      post_mcp '{"jsonrpc":"2.0","id":"full-suite-command-only-success","method":"tools/call","params":{"name":"run_command_profile","arguments":{"profile":"server_version"}}}' "$SESSION_ID"
      [[ "$MCP_STATUS" == 200 ]] || fail aggregate_command_only_success_status_invalid
      jq -e --arg version "$EXPECTED_VERSION" '.result.isError == false and .result.structuredContent.profile == "server_version" and .result.structuredContent.exitCode == 0 and .result.structuredContent.stdout == ("termux-mcp-server " + $version + "\n")' "$BODY_FILE" >/dev/null \
        || fail aggregate_command_only_success_contract_invalid
      [[ ! -e "$FULL_SUITE_REPLACEMENT_MARKER" ]] || fail aggregate_command_only_executable_replacement_ran
      ;;
  esac
  close_aggregate_session
  stop_aggregate_server
}

start_aggregate_server default-disabled
initialize_aggregate_session
post_mcp '{"jsonrpc":"2.0","id":"full-suite-default-tools","method":"tools/list"}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail aggregate_default_tools_status_invalid
jq -e '
  [.result.tools[].name] == ["runtime_status","platform_info","android_status","project_service_status","create_directory","copy_file","trash_file","find_paths","hash_file","list_directory","path_metadata","read_binary_file","read_binary_range","read_file","read_text_range","search_text","write_file"]
' "$BODY_FILE" >/dev/null || fail aggregate_default_tool_allowlist_invalid
post_mcp '{"jsonrpc":"2.0","id":"full-suite-default-runtime","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail aggregate_default_runtime_status_invalid
jq -e '
  .result.structuredContent.androidBatteryStatusCompiled == true
  and .result.structuredContent.androidBatteryStatusEnabled == false
  and .result.structuredContent.androidVolumeStatusCompiled == true
  and .result.structuredContent.androidVolumeStatusEnabled == false
  and .result.structuredContent.androidVolumeControlCompiled == true
  and .result.structuredContent.androidVolumeControlEnabled == false
  and .result.structuredContent.commandExecutionCompiled == true
  and .result.structuredContent.commandExecution == false
  and .result.structuredContent.androidVolumeGrantRequired == false
  and .result.structuredContent.createDirectoryMutationEnabled == false
  and .result.structuredContent.copyFileMutationEnabled == false
  and .result.structuredContent.trashFileMutationEnabled == false
  and .result.structuredContent.fileWriteMutationEnabled == false
  and .result.structuredContent.highImpactTools == false
' "$BODY_FILE" >/dev/null || fail aggregate_default_runtime_contract_invalid
close_aggregate_session
stop_aggregate_server

for aggregate_posture in battery-only volume-status-only volume-control-only command-only; do
  validate_single_optional_gate_posture "$aggregate_posture"
done
unset aggregate_posture

start_aggregate_server fully-enabled
initialize_aggregate_session
mv -- "$FULL_SUITE_EXECUTABLE_DIR" "$WORK_ROOT/full-suite-loaded-executable"
mkdir -m 700 "$FULL_SUITE_EXECUTABLE_DIR"
cat >"$FULL_SUITE_EXECUTABLE" <<EOF
#!/data/data/com.termux/files/usr/bin/bash
set -euo pipefail
: >'$FULL_SUITE_REPLACEMENT_MARKER'
printf '%s\n' 'termux-mcp-server replacement-must-not-run'
EOF
chmod 700 "$FULL_SUITE_EXECUTABLE"
post_mcp '{"jsonrpc":"2.0","id":"full-suite-enabled-tools","method":"tools/list"}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail aggregate_enabled_tools_status_invalid
jq -e '
  [.result.tools[].name] == ["runtime_status","platform_info","android_status","project_service_status","create_directory","copy_file","trash_file","find_paths","hash_file","list_directory","path_metadata","read_binary_file","read_binary_range","read_file","read_text_range","search_text","write_file","android_battery_status","android_volume_status","set_android_volume","run_command_profile"]
' "$BODY_FILE" >/dev/null || fail aggregate_enabled_tool_allowlist_invalid
post_mcp '{"jsonrpc":"2.0","id":"full-suite-enabled-runtime","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail aggregate_enabled_runtime_status_invalid
jq -e '
  .result.structuredContent.androidBatteryStatusCompiled == true
  and .result.structuredContent.androidBatteryStatusEnabled == true
  and .result.structuredContent.androidVolumeStatusCompiled == true
  and .result.structuredContent.androidVolumeStatusEnabled == true
  and .result.structuredContent.androidVolumeControlCompiled == true
  and .result.structuredContent.androidVolumeControlEnabled == true
  and .result.structuredContent.commandExecutionCompiled == true
  and .result.structuredContent.commandExecution == true
  and .result.structuredContent.androidVolumeGrantRequired == true
  and .result.structuredContent.createDirectoryMutationEnabled == false
  and .result.structuredContent.copyFileMutationEnabled == false
  and .result.structuredContent.trashFileMutationEnabled == false
  and .result.structuredContent.fileWriteMutationEnabled == false
  and .result.structuredContent.highImpactTools == true
' "$BODY_FILE" >/dev/null || fail aggregate_enabled_runtime_contract_invalid

post_mcp '{"jsonrpc":"2.0","id":"full-suite-battery","method":"tools/call","params":{"name":"android_battery_status","arguments":{}}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail aggregate_battery_status_invalid
jq -e '.result.isError == false and .result.structuredContent.percentage == 88 and .result.structuredContent.cycle_count == 200' "$BODY_FILE" >/dev/null || fail aggregate_battery_contract_invalid
post_mcp '{"jsonrpc":"2.0","id":"full-suite-volume","method":"tools/call","params":{"name":"android_volume_status","arguments":{}}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail aggregate_volume_status_invalid
jq -e '.result.isError == false and [.result.structuredContent.streams[].stream] == ["alarm","call","music","notification","ring","system"] and .result.structuredContent.streams[2] == {stream:"music",volume:5,maxVolume:15}' "$BODY_FILE" >/dev/null || fail aggregate_volume_contract_invalid

MCP__CAPABILITY__CONFIG_FILE="$AGGREGATE_CAPABILITY_CONFIG" \
MCP__CAPABILITY__SESSION_ID="$SESSION_ID" \
MCP__CAPABILITY__VOLUME_STREAM=music \
MCP__CAPABILITY__VOLUME_LEVEL=9 \
  "$FULL_SUITE_ARTIFACT" --issue-android-volume-grant >"$AGGREGATE_GRANT_FILE" 2>/dev/null || fail aggregate_volume_grant_issuance_failed
[[ "$(wc -l <"$AGGREGATE_GRANT_FILE")" == 1 ]] || fail aggregate_volume_grant_line_count_invalid
AGGREGATE_GRANT="$(<"$AGGREGATE_GRANT_FILE")"
[[ "$AGGREGATE_GRANT" == v1.native-full-suite-1.* ]] || fail aggregate_volume_grant_invalid
post_mcp '{"jsonrpc":"2.0","id":"full-suite-volume-preview","method":"tools/call","params":{"name":"set_android_volume","arguments":{"stream":"music","level":9}}}' "$SESSION_ID" "$AGGREGATE_GRANT"
[[ "$MCP_STATUS" == 200 ]] || fail aggregate_volume_preview_status_invalid
jq -e '.result.isError == false and .result.structuredContent == {stream:"music",previousLevel:5,requestedLevel:9,maxVolume:15,dryRun:true,changed:false,verified:false,outcome:"preview",rollback:"not_required"}' "$BODY_FILE" >/dev/null || fail aggregate_volume_preview_contract_invalid
[[ "$(cat "$VOLUME_STATE")" == 5 && ! -e "$VOLUME_CALLS" ]] || fail aggregate_volume_preview_mutated
post_mcp '{"jsonrpc":"2.0","id":"full-suite-volume-wrong-binding","method":"tools/call","params":{"name":"set_android_volume","arguments":{"stream":"music","level":8,"dry_run":false}}}' "$SESSION_ID" "$AGGREGATE_GRANT"
[[ "$MCP_STATUS" == 403 ]] || fail aggregate_volume_grant_isolation_status_invalid
jq -e '.error.data.reason == "capability_grant_binding_mismatch"' "$BODY_FILE" >/dev/null || fail aggregate_volume_grant_isolation_contract_invalid
[[ "$(cat "$VOLUME_STATE")" == 5 && ! -e "$VOLUME_CALLS" ]] || fail aggregate_volume_grant_isolation_mutated
unset AGGREGATE_GRANT

post_mcp '{"jsonrpc":"2.0","id":"full-suite-command","method":"tools/call","params":{"name":"run_command_profile","arguments":{"profile":"server_version"}}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail aggregate_command_status_invalid
jq -e --arg version "$EXPECTED_VERSION" '.result.isError == false and .result.structuredContent.profile == "server_version" and .result.structuredContent.exitCode == 0 and .result.structuredContent.stdout == ("termux-mcp-server " + $version + "\n")' "$BODY_FILE" >/dev/null || fail aggregate_command_contract_invalid
[[ ! -e "$FULL_SUITE_REPLACEMENT_MARKER" ]] || fail aggregate_command_executable_replacement_ran
post_mcp '{"jsonrpc":"2.0","id":"full-suite-command-override","method":"tools/call","params":{"name":"run_command_profile","arguments":{"profile":"server_version","program":"/bin/sh"}}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 400 ]] || fail aggregate_command_override_status_invalid
jq -e '.error.code == -32602 and (.result | not)' "$BODY_FILE" >/dev/null || fail aggregate_command_override_contract_invalid

post_mcp "$(jq -cn --arg path "$AGGREGATE_CREATE_TARGET" '{jsonrpc:"2.0",id:"full-suite-create-disabled",method:"tools/call",params:{name:"create_directory",arguments:{path:$path,dry_run:false}}}')" "$SESSION_ID"
[[ "$MCP_STATUS" == 403 ]] || fail aggregate_create_directory_disabled_status_invalid
jq -e '.error.code == -32003 and .error.data.reason == "create_directory_mutation_disabled"' "$BODY_FILE" >/dev/null || fail aggregate_create_directory_disabled_contract_invalid
[[ ! -e "$AGGREGATE_CREATE_TARGET" && ! -L "$AGGREGATE_CREATE_TARGET" ]] || fail aggregate_create_directory_disabled_target_mutated

post_mcp "$(jq -cn --arg source "$AGGREGATE_COPY_SOURCE" --arg destination "$AGGREGATE_COPY_DESTINATION" '{jsonrpc:"2.0",id:"full-suite-copy-disabled",method:"tools/call",params:{name:"copy_file",arguments:{source_path:$source,destination_path:$destination,dry_run:false}}}')" "$SESSION_ID"
[[ "$MCP_STATUS" == 403 ]] || fail aggregate_copy_file_disabled_status_invalid
jq -e '.error.code == -32003 and .error.data.reason == "copy_file_mutation_disabled"' "$BODY_FILE" >/dev/null || fail aggregate_copy_file_disabled_contract_invalid
[[ "$(sha256sum "$AGGREGATE_COPY_SOURCE" | awk '{print $1}')" == "$AGGREGATE_COPY_SOURCE_SHA" ]] || fail aggregate_copy_file_disabled_source_content_mutated
[[ "$(stat -c '%d:%i:%s:%f:%y:%z' "$AGGREGATE_COPY_SOURCE")" == "$AGGREGATE_COPY_SOURCE_STATE" ]] || fail aggregate_copy_file_disabled_source_state_mutated
[[ ! -e "$AGGREGATE_COPY_DESTINATION" && ! -L "$AGGREGATE_COPY_DESTINATION" ]] || fail aggregate_copy_file_disabled_destination_mutated

post_mcp "$(jq -cn --arg path "$AGGREGATE_TRASH_TARGET" '{jsonrpc:"2.0",id:"full-suite-trash-disabled",method:"tools/call",params:{name:"trash_file",arguments:{path:$path,dry_run:false}}}')" "$SESSION_ID"
[[ "$MCP_STATUS" == 403 ]] || fail aggregate_trash_file_disabled_status_invalid
jq -e '.error.code == -32003 and .error.data.reason == "trash_file_mutation_disabled"' "$BODY_FILE" >/dev/null || fail aggregate_trash_file_disabled_contract_invalid
[[ "$(sha256sum "$AGGREGATE_TRASH_TARGET" | awk '{print $1}')" == "$AGGREGATE_TRASH_TARGET_SHA" ]] || fail aggregate_trash_file_disabled_target_content_mutated
[[ "$(stat -c '%d:%i:%s:%f:%y:%z' "$AGGREGATE_TRASH_TARGET")" == "$AGGREGATE_TRASH_TARGET_STATE" ]] || fail aggregate_trash_file_disabled_target_state_mutated
[[ ! -e "$AGGREGATE_TRASH_QUARANTINE" && ! -L "$AGGREGATE_TRASH_QUARANTINE" ]] || fail aggregate_trash_file_disabled_quarantine_mutated

post_mcp "$(jq -cn --arg path "$AGGREGATE_WRITE_DESTINATION" '{jsonrpc:"2.0",id:"full-suite-write-disabled",method:"tools/call",params:{name:"write_file",arguments:{path:$path,content:"inert",dry_run:false}}}')" "$SESSION_ID"
[[ "$MCP_STATUS" == 403 ]] || fail aggregate_write_disabled_status_invalid
jq -e '.error.code == -32003 and .error.data.reason == "write_file_mutation_disabled"' "$BODY_FILE" >/dev/null || fail aggregate_write_disabled_contract_invalid
[[ ! -e "$AGGREGATE_WRITE_DESTINATION" && ! -L "$AGGREGATE_WRITE_DESTINATION" ]] || fail aggregate_write_disabled_destination_mutated
[[ "$(sha256sum "$AGGREGATE_COPY_SOURCE" | awk '{print $1}')" == "$AGGREGATE_COPY_SOURCE_SHA" ]] || fail aggregate_filesystem_final_copy_source_content_mutated
[[ "$(stat -c '%d:%i:%s:%f:%y:%z' "$AGGREGATE_COPY_SOURCE")" == "$AGGREGATE_COPY_SOURCE_STATE" ]] || fail aggregate_filesystem_final_copy_source_state_mutated
[[ "$(sha256sum "$AGGREGATE_TRASH_TARGET" | awk '{print $1}')" == "$AGGREGATE_TRASH_TARGET_SHA" ]] || fail aggregate_filesystem_final_trash_target_content_mutated
[[ "$(stat -c '%d:%i:%s:%f:%y:%z' "$AGGREGATE_TRASH_TARGET")" == "$AGGREGATE_TRASH_TARGET_STATE" ]] || fail aggregate_filesystem_final_trash_target_state_mutated
[[ ! -e "$AGGREGATE_CREATE_TARGET" && ! -L "$AGGREGATE_CREATE_TARGET" ]] || fail aggregate_filesystem_final_create_target_mutated
[[ ! -e "$AGGREGATE_COPY_DESTINATION" && ! -L "$AGGREGATE_COPY_DESTINATION" ]] || fail aggregate_filesystem_final_copy_destination_mutated
[[ ! -e "$AGGREGATE_TRASH_QUARANTINE" && ! -L "$AGGREGATE_TRASH_QUARANTINE" ]] || fail aggregate_filesystem_final_quarantine_mutated
[[ ! -e "$AGGREGATE_WRITE_DESTINATION" && ! -L "$AGGREGATE_WRITE_DESTINATION" ]] || fail aggregate_filesystem_final_write_destination_mutated
close_aggregate_session
stop_aggregate_server
[[ "$AGGREGATE_CLEANUPS" == 6 ]] || fail aggregate_cleanup_count_invalid
AGGREGATE_REQUEST_COUNT=$((REQUEST_COUNT - AGGREGATE_REQUEST_START))

COMPLETED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
REPORT_NEXT="$WORK_ROOT/emulated-evidence.json"
jq -n \
  --arg gate_version "$GATE_VERSION" \
  --arg started_at "$STARTED_AT" \
  --arg completed_at "$COMPLETED_AT" \
  --arg commit "$EXPECTED_COMMIT" \
  --arg version "$EXPECTED_VERSION" \
  --arg ci_run_id "$CI_RUN_ID" \
  --arg security_run_id "$SECURITY_RUN_ID" \
  --arg android_run_id "$ANDROID_RUN_ID" \
  --arg default_sha "$DEFAULT_SHA" \
  --argjson default_bytes "$DEFAULT_BYTES" \
  --arg mcp_sha "$MCP_SHA" \
  --argjson mcp_bytes "$MCP_BYTES" \
  --arg volume_control_sha "$VOLUME_CONTROL_SHA" \
  --argjson volume_control_bytes "$VOLUME_CONTROL_BYTES" \
  --arg full_suite_sha "$FULL_SUITE_SHA" \
  --argjson full_suite_bytes "$FULL_SUITE_BYTES" \
  --arg full_suite_manifest_sha "$FULL_SUITE_MANIFEST_SHA" \
  --arg architecture "$(uname -m)" \
  --arg image "$EXPECTED_IMAGE" \
  --arg image_digest "$IMAGE_DIGEST" \
  --arg runtime_report_sha "$RUNTIME_REPORT_SHA" \
  --argjson runtime_results "$RUNTIME_RESULT_COUNT" \
  --argjson samples "$SAMPLES" \
  --argjson requests "$STRESS_REQUEST_COUNT" \
  --argjson aggregate_requests "$AGGREGATE_REQUEST_COUNT" '
  {
    schemaVersion: 3,
    gateVersion: $gate_version,
    status: "pass",
    failureCode: null,
    releaseQualificationEligible: false,
    startedAt: $started_at,
    completedAt: $completed_at,
    candidate: {
      commit: $commit,
      version: $version,
      ciRunId: $ci_run_id,
      securityRunId: $security_run_id,
      androidRunId: $android_run_id,
      defaultArtifact: {sha256: $default_sha, bytes: $default_bytes},
      mcpRuntimeArtifact: {sha256: $mcp_sha, bytes: $mcp_bytes},
      androidVolumeControlArtifact: {sha256: $volume_control_sha, bytes: $volume_control_bytes},
      fullSuiteArtifact: {
        sha256: $full_suite_sha,
        bytes: $full_suite_bytes,
        manifestSha256: $full_suite_manifest_sha,
        artifactName: "termux-mcp-server-aarch64-linux-android-full-suite",
        posture: "full-suite",
        features: ["full-suite"],
        fileName: "termux-mcp-server"
      }
    },
    environment: {
      executionMode: "official-termux-docker-native-arm64",
      architecture: $architecture,
      image: $image,
      imageDigest: $image_digest,
      androidLinker: true
    },
    runtimeValidation: {
      status: "pass",
      reportSha256: $runtime_report_sha,
      resultCount: $runtime_results,
      phases: {preflight: "pass", runtime: "pass", deployment: "not_run"}
    },
    aggregateValidation: {
      status: "pass",
      requests: $aggregate_requests,
      defaultDisabled: {
        toolCount: 17,
        exactToolOrder: true,
        optionalFeaturesCompiled: true,
        optionalToolsHidden: true,
        runtimeFlagsOmitted: true
      },
      fullyEnabled: {
        toolCount: 21,
        exactToolOrder: true,
        allOptionalToolsExposed: true,
        providerSuccesses: true,
        volumePreviewNoMutation: true,
        volumeGrantIsolation: true,
        commandExecutableIdentityPinned: true
      },
      independentRuntimeGates: true,
      filesystemMutationsDisabled: true,
      boundedCleanup: true,
      directPhysicalObservationRequired: true
    },
    stress: {
      status: "pass",
      samples: $samples,
      requests: $requests,
      servicePidStable: true,
      healthReadyStable: true,
      sessionLifecycle: true,
      exactToolAllowlist: true,
      safeRootIdentityPinned: true,
      safeRootAncestorIdentityPinned: true,
      copyFileMutationDisabled: true,
      highImpactDisabled: true,
      longObservationRequired: false
    }
  }' >"$REPORT_NEXT"
chmod 600 "$REPORT_NEXT"

jq -e '
  .schemaVersion == 3 and .gateVersion == "3" and .status == "pass" and .failureCode == null
  and .releaseQualificationEligible == false
  and .environment.executionMode == "official-termux-docker-native-arm64"
  and (.candidate.androidVolumeControlArtifact.sha256 | test("^[0-9a-f]{64}$"))
  and .candidate.androidVolumeControlArtifact.sha256 != .candidate.defaultArtifact.sha256
  and .candidate.androidVolumeControlArtifact.sha256 != .candidate.mcpRuntimeArtifact.sha256
  and (.candidate.fullSuiteArtifact.sha256 | test("^[0-9a-f]{64}$"))
  and (.candidate.fullSuiteArtifact.manifestSha256 | test("^[0-9a-f]{64}$"))
  and .candidate.fullSuiteArtifact.sha256 != .candidate.defaultArtifact.sha256
  and .candidate.fullSuiteArtifact.sha256 != .candidate.mcpRuntimeArtifact.sha256
  and .candidate.fullSuiteArtifact.sha256 != .candidate.androidVolumeControlArtifact.sha256
  and .candidate.fullSuiteArtifact.artifactName == "termux-mcp-server-aarch64-linux-android-full-suite"
  and .candidate.fullSuiteArtifact.posture == "full-suite"
  and .candidate.fullSuiteArtifact.features == ["full-suite"]
  and .candidate.fullSuiteArtifact.fileName == "termux-mcp-server"
  and (.environment.architecture == "aarch64" or .environment.architecture == "arm64")
  and .environment.androidLinker == true
  and .runtimeValidation.status == "pass"
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
  and .stress.status == "pass" and .stress.samples >= 32 and .stress.requests >= (.stress.samples * 3)
  and .stress.servicePidStable == true and .stress.healthReadyStable == true
  and .stress.sessionLifecycle == true and .stress.exactToolAllowlist == true
  and .stress.safeRootIdentityPinned == true
  and .stress.safeRootAncestorIdentityPinned == true
  and .stress.copyFileMutationDisabled == true
  and .stress.highImpactDisabled == true
  and .stress.longObservationRequired == false
' "$REPORT_NEXT" >/dev/null || fail generated_report_invalid

if grep -Eq '/data/|Bearer[[:space:]]|MCP__|[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}' "$REPORT_NEXT"; then
  fail generated_report_not_sanitized
fi
install -m 600 "$REPORT_NEXT" "$OUTPUT_REPORT" || fail report_publication_failed

REPORT_SHA="$(sha256sum "$OUTPUT_REPORT" | awk '{print $1}')"
log "report_sha256=$REPORT_SHA"
log "report=$OUTPUT_REPORT"
log "samples=$SAMPLES"
log "requests=$REQUEST_COUNT"
log 'TERMUX_MCP_EMULATED_RESULT=PASS'
