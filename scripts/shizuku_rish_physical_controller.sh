#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077
set +x

readonly REPOSITORY="CyberBASSLord-666/termux-mcp-edge"
readonly ARTIFACT_NAME="termux-mcp-server-aarch64-linux-android-android-rish-development"
readonly TERMUX_PACKAGE="com.termux"
readonly SHIZUKU_PACKAGE="moe.shizuku.privileged.api"

ARTIFACT_DIR=""
EXPECTED_COMMIT=""
EXPECTED_VERSION=""
POLICY_SHA256=""
CARGO_LOCK_SHA256=""
WORKFLOW_DEFINITION_SHA256=""
WORKFLOW_RUN_ID=""
WORKFLOW_RUN_ATTEMPT=""
CI_RUN_ID=""
SECURITY_RUN_ID=""
ANDROID_RUN_ID=""
CONTROLLER_CHALLENGE_FILE=""
DEVICE_SLOT=""
DEVICE_GATE=""
OUTPUT=""

CONFIG_ROOT="${TERMUX_MCP_PHYSICAL_CONFIG_ROOT:-/etc/termux-mcp-edge/physical-devices}"
DEVICE_CONFIG=""
ADB_SERIAL=""
SSH_USER=""
SSH_IDENTITY_FILE=""
SSH_KNOWN_HOSTS_FILE=""
SSH_DEVICE_PORT=""
RISH_DEX_PATH=""
RISH_DEX_SHA256=""
RISH_DEX_BYTES=""
DEVICE_PROFILE_COMMITMENT=""
EXPECTED_TERMUX_VERSION=""
EXPECTED_TERMUX_SIGNER_SHA256=""
EXPECTED_SHIZUKU_VERSION=""
EXPECTED_SHIZUKU_SIGNER_SHA256=""
PRIVATE_EVIDENCE_ROOT=""

WORK_ROOT=""
LOCAL_PORT=""
REMOTE_ROOT=""
REMOTE_CREATED=0
FORWARD_CREATED=0
PUBLISH_NEXT=""
PRIVATE_EVIDENCE_NEXT=""
CLEANUP_CONFIRMED=1
CANDIDATE_EXECUTED=0
SLOT_QUARANTINED=0
OFFLINE_POSTURE_PRE_OBSERVED=0
OFFLINE_POSTURE_POST_OBSERVED=0
SSH_TARGET=""
CONTROLLER_CHALLENGE_SHA256=""
API_LEVEL=""
SECURITY_PATCH=""
BUILD_FINGERPRINT_SHA256=""
ADB_SHELL_UID=""
TERMUX_VERSION=""
TERMUX_SIGNER_SHA256=""
SHIZUKU_VERSION=""
SHIZUKU_SIGNER_SHA256=""

declare -a SSH_OPTIONS=()
declare -a SCP_OPTIONS=()

usage() {
  cat <<'EOF'
Usage: shizuku_rish_physical_controller.sh \
  --artifact-dir ABSOLUTE_BUNDLE \
  --expected-commit SHA \
  --expected-version VERSION \
  --policy-sha256 SHA256 \
  --cargo-lock-sha256 SHA256 \
  --workflow-definition-sha256 SHA256 \
  --workflow-run-id ID \
  --workflow-run-attempt 1 \
  --ci-run-id ID \
  --security-run-id ID \
  --android-run-id ID \
  --controller-challenge-file ABSOLUTE_32_BYTE_FILE \
  --device-slot SLOT \
  --device-gate ABSOLUTE_TRUSTED_SCRIPT \
  --output ABSOLUTE_JSON

Runs a fixed physical-device S2.5 identity qualification through one locally
provisioned device slot. Device serials, SSH material, package signer policy,
and rish paths are accepted only from the private controller configuration,
never from workflow inputs. Candidate repository scripts are not executed.

Configuration defaults to:
  /etc/termux-mcp-edge/physical-devices/<slot>.json
and may be relocated only by the runner-owned
TERMUX_MCP_PHYSICAL_CONFIG_ROOT environment variable.
The private configuration also names a runner-owned, mode-0700 evidence
directory where the mode-0600 raw device transcript is retained. That path is
never accepted from workflow input and is never included in public evidence.

The configuration root must be a canonical mode-0700 directory owned by the
runner and backed by durable storage that survives runner-process and
controller-host restart; a replacement controller must not infer slot safety
from an absent local quarantine marker. Each <slot>.json must be a canonical
mode-0600 regular file owned by the runner, with exactly these fields:
  schemaVersion             number, exactly 1
  slot                      string
  adbSerial                 string
  sshUser                   string
  sshIdentityFile           absolute mode-0600 file
  sshKnownHostsFile         absolute mode-0600 file
  sshDevicePort             integer, 1024..65535
  rishDexPath               canonical private Termux-home path
  rishDexSha256             lowercase SHA-256
  deviceProfileCommitment   lowercase SHA-256
  termuxVersion             bounded version string
  termuxSignerSha256        lowercase SHA-256
  shizukuVersion            bounded version string
  shizukuSignerSha256       lowercase SHA-256
  privateEvidenceRoot       absolute mode-0700 runner-owned directory

The public offline-posture booleans are point-in-time controller observations,
immediately before and after candidate execution. They mean airplane mode was
on, Wi-Fi/mobile data/Bluetooth settings were off, and the default-route query
was empty at both points. They do not claim continuous or adversarial network
isolation.
EOF
}

fail() {
  local reason="${1:-internal_error}"
  case "$reason" in
    *[!a-z0-9_]*|"") reason="internal_error" ;;
  esac
  printf 'SHIZUKU_RISH_PHYSICAL_CONTROLLER_RESULT=FAIL reason=%s\n' "$reason" >&2
  exit 1
}

is_sha256() {
  [[ "$1" =~ ^[0-9a-f]{64}$ ]]
}

canonical_regular_file() {
  local path="$1" mode="$2" maximum_bytes="$3" actual_mode bytes links
  [[ "$path" == /* && -f "$path" && ! -L "$path" ]] || return 1
  [[ "$(realpath -e -- "$path" 2>/dev/null)" == "$path" ]] || return 1
  actual_mode="$(stat -c '%a' -- "$path" 2>/dev/null)" || return 1
  [[ "$actual_mode" == "$mode" ]] || return 1
  links="$(stat -c '%h' -- "$path" 2>/dev/null)" || return 1
  [[ "$links" == 1 ]] || return 1
  bytes="$(stat -c '%s' -- "$path" 2>/dev/null)" || return 1
  [[ "$bytes" =~ ^[0-9]+$ ]] || return 1
  ((bytes >= 1 && bytes <= maximum_bytes))
}

private_directory() {
  local path="$1"
  [[ "$path" == /* && -d "$path" && ! -L "$path" ]] || return 1
  [[ "$(realpath -e -- "$path" 2>/dev/null)" == "$path" ]] || return 1
  [[ "$(stat -c '%a:%u' -- "$path" 2>/dev/null)" == "700:$(id -u)" ]]
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail required_command_missing
}

adb_command() {
  timeout --signal=TERM --kill-after=2s 15s \
    adb -s "$ADB_SERIAL" "$@"
}

adb_scalar() {
  local value line_count
  value="$(adb_command "$@" 2>/dev/null)" || return 1
  value="${value%$'\r'}"
  [[ "$value" != *$'\n'* && "$value" != *$'\r'* && -n "$value" && "${#value}" -le 1024 ]] \
    || return 1
  line_count="$(printf '%s\n' "$value" | wc -l)"
  [[ "$line_count" == 1 ]] || return 1
  printf '%s\n' "$value"
}

adb_optional_scalar() {
  local value
  value="$(adb_command "$@" 2>/dev/null)" || return 1
  value="${value%$'\r'}"
  [[ "$value" != *$'\n'* && "$value" != *$'\r'* && "${#value}" -le 1024 ]] \
    || return 1
  printf '%s' "$value"
}

ssh_command() {
  timeout --signal=TERM --kill-after=5s 30s \
    ssh "${SSH_OPTIONS[@]}" "$SSH_TARGET" "$@"
}

scp_to_device() {
  timeout --signal=TERM --kill-after=5s 30s \
    scp "${SCP_OPTIONS[@]}" -- "$1" "$SSH_TARGET:$2"
}

scp_from_device() {
  timeout --signal=TERM --kill-after=5s 30s \
    scp "${SCP_OPTIONS[@]}" -- "$SSH_TARGET:$1" "$2"
}

quarantine_slot() {
  local marker="$CONFIG_ROOT/.quarantine-$DEVICE_SLOT"
  [[ "$DEVICE_SLOT" =~ ^[a-z0-9][a-z0-9_-]{0,31}$ ]] || return 1
  private_directory "$CONFIG_ROOT" || return 1
  if [[ ! -e "$marker" && ! -L "$marker" ]]; then
    (set -o noclobber; printf 'quarantined\n' >"$marker") 2>/dev/null || return 1
    chmod 600 "$marker" || return 1
  fi
  [[ -f "$marker" && ! -L "$marker" \
    && "$(realpath -e -- "$marker" 2>/dev/null)" == "$marker" \
    && "$(stat -c '%a:%u:%h' -- "$marker" 2>/dev/null)" == "600:$(id -u):1" \
    && "$(<"$marker")" == quarantined ]] || return 1
  sync -f -- "$marker" || return 1
  SLOT_QUARANTINED=1
}

remove_remote_root() {
  ((REMOTE_CREATED == 1)) || return 0
  [[ "$REMOTE_ROOT" =~ ^/data/data/com\.termux/files/home/\.termux-mcp-rish-controller-[0-9a-f]{12}-[1-9][0-9]{0,19}$ ]] \
    || return 1
  ssh_command "rm -rf -- '$REMOTE_ROOT' && test ! -e '$REMOTE_ROOT' && test ! -L '$REMOTE_ROOT'" \
    >/dev/null 2>&1 || return 1
  REMOTE_CREATED=0
}

remove_forward() {
  ((FORWARD_CREATED == 1)) || return 0
  [[ "$LOCAL_PORT" =~ ^[1-9][0-9]{0,4}$ ]] || return 1
  adb_command forward --remove "tcp:$LOCAL_PORT" >/dev/null 2>&1 || return 1
  if adb_command forward --list 2>/dev/null \
    | grep -Fq "$ADB_SERIAL tcp:$LOCAL_PORT "; then
    return 1
  fi
  FORWARD_CREATED=0
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM HUP
  set +e
  remove_remote_root || CLEANUP_CONFIRMED=0
  remove_forward || CLEANUP_CONFIRMED=0
  [[ -z "$PUBLISH_NEXT" ]] || rm -f -- "$PUBLISH_NEXT" >/dev/null 2>&1 || CLEANUP_CONFIRMED=0
  [[ -z "$PRIVATE_EVIDENCE_NEXT" ]] \
    || rm -f -- "$PRIVATE_EVIDENCE_NEXT" >/dev/null 2>&1 \
    || CLEANUP_CONFIRMED=0
  if [[ -n "$WORK_ROOT" && "$WORK_ROOT" == /tmp/termux-mcp-rish-controller.* ]]; then
    rm -rf -- "$WORK_ROOT" >/dev/null 2>&1 || CLEANUP_CONFIRMED=0
    [[ ! -e "$WORK_ROOT" && ! -L "$WORK_ROOT" ]] || CLEANUP_CONFIRMED=0
  fi
  if ((CANDIDATE_EXECUTED == 1)); then
    quarantine_slot || CLEANUP_CONFIRMED=0
  fi
  if ((CLEANUP_CONFIRMED == 0)); then
    quarantine_slot || true
    status=1
    printf 'SHIZUKU_RISH_PHYSICAL_CONTROLLER_CLEANUP=UNCONFIRMED\n' >&2
  fi
  exit "$status"
}

read_device_config() {
  DEVICE_CONFIG="$CONFIG_ROOT/$DEVICE_SLOT.json"
  canonical_regular_file "$DEVICE_CONFIG" 600 65536 || fail device_config_invalid
  [[ "$(stat -c '%u' -- "$DEVICE_CONFIG")" == "$(id -u)" ]] || fail device_config_invalid
  jq -e --arg slot "$DEVICE_SLOT" '
    . as $config
    | type == "object"
    and (keys == [
      "adbSerial",
      "deviceProfileCommitment",
      "privateEvidenceRoot",
      "rishDexPath",
      "rishDexSha256",
      "schemaVersion",
      "shizukuSignerSha256",
      "shizukuVersion",
      "slot",
      "sshDevicePort",
      "sshIdentityFile",
      "sshKnownHostsFile",
      "sshUser",
      "termuxSignerSha256",
      "termuxVersion"
    ])
    and .schemaVersion == 1
    and .slot == $slot
    and (.sshDevicePort | type == "number" and . == floor)
    and all(
      keys[]
      | select(. != "schemaVersion" and . != "sshDevicePort");
      . as $key | $config[$key] | type == "string"
    )
  ' "$DEVICE_CONFIG" >/dev/null || fail device_config_contract_invalid

  ADB_SERIAL="$(jq -r '.adbSerial' "$DEVICE_CONFIG")"
  SSH_USER="$(jq -r '.sshUser' "$DEVICE_CONFIG")"
  SSH_IDENTITY_FILE="$(jq -r '.sshIdentityFile' "$DEVICE_CONFIG")"
  SSH_KNOWN_HOSTS_FILE="$(jq -r '.sshKnownHostsFile' "$DEVICE_CONFIG")"
  SSH_DEVICE_PORT="$(jq -r '.sshDevicePort' "$DEVICE_CONFIG")"
  RISH_DEX_PATH="$(jq -r '.rishDexPath' "$DEVICE_CONFIG")"
  RISH_DEX_SHA256="$(jq -r '.rishDexSha256' "$DEVICE_CONFIG")"
  DEVICE_PROFILE_COMMITMENT="$(jq -r '.deviceProfileCommitment' "$DEVICE_CONFIG")"
  EXPECTED_TERMUX_VERSION="$(jq -r '.termuxVersion' "$DEVICE_CONFIG")"
  EXPECTED_TERMUX_SIGNER_SHA256="$(jq -r '.termuxSignerSha256' "$DEVICE_CONFIG")"
  EXPECTED_SHIZUKU_VERSION="$(jq -r '.shizukuVersion' "$DEVICE_CONFIG")"
  EXPECTED_SHIZUKU_SIGNER_SHA256="$(jq -r '.shizukuSignerSha256' "$DEVICE_CONFIG")"
  PRIVATE_EVIDENCE_ROOT="$(jq -r '.privateEvidenceRoot' "$DEVICE_CONFIG")"

  [[ "$ADB_SERIAL" =~ ^[A-Za-z0-9._:-]{1,128}$ ]] || fail adb_serial_invalid
  [[ "$SSH_USER" =~ ^[A-Za-z0-9._-]{1,64}$ ]] || fail ssh_user_invalid
  [[ "$SSH_DEVICE_PORT" =~ ^[0-9]+$ ]] \
    && ((SSH_DEVICE_PORT >= 1024 && SSH_DEVICE_PORT <= 65535)) \
    || fail ssh_device_port_invalid
  [[ "$RISH_DEX_PATH" =~ ^/data/data/com\.termux/files/home/[A-Za-z0-9._/-]{1,512}$ \
    && "$RISH_DEX_PATH" != *"/../"* && "$RISH_DEX_PATH" != *"//"* ]] \
    || fail rish_dex_path_invalid
  for digest in \
    "$RISH_DEX_SHA256" "$DEVICE_PROFILE_COMMITMENT" \
    "$EXPECTED_TERMUX_SIGNER_SHA256" "$EXPECTED_SHIZUKU_SIGNER_SHA256"
  do
    is_sha256 "$digest" || fail device_config_digest_invalid
  done
  [[ "$EXPECTED_TERMUX_VERSION" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] \
    || fail termux_version_policy_invalid
  [[ "$EXPECTED_SHIZUKU_VERSION" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] \
    || fail shizuku_version_policy_invalid
  canonical_regular_file "$SSH_IDENTITY_FILE" 600 65536 || fail ssh_identity_invalid
  canonical_regular_file "$SSH_KNOWN_HOSTS_FILE" 600 1048576 || fail ssh_known_hosts_invalid
  [[ "$(stat -c '%u' -- "$SSH_IDENTITY_FILE")" == "$(id -u)" \
    && "$(stat -c '%u' -- "$SSH_KNOWN_HOSTS_FILE")" == "$(id -u)" ]] \
    || fail ssh_material_owner_invalid
  private_directory "$PRIVATE_EVIDENCE_ROOT" || fail private_evidence_root_invalid
}

validate_usb_transport() {
  local devpath
  devpath="$(adb_scalar get-devpath)" || return 1
  [[ "$devpath" =~ ^usb:[A-Za-z0-9._:/-]{1,256}$ ]] || return 1
  unset devpath
}

validate_non_emulator() {
  local qemu_state
  qemu_state="$(adb_optional_scalar shell getprop ro.kernel.qemu)" || return 1
  [[ -z "$qemu_state" || "$qemu_state" == 0 ]]
}

validate_offline_device_posture() {
  [[ "$(adb_scalar shell /system/bin/settings get global airplane_mode_on)" == 1 ]] \
    || return 1
  [[ "$(adb_scalar shell /system/bin/settings get global wifi_on)" == 0 ]] \
    || return 1
  [[ "$(adb_scalar shell /system/bin/settings get global mobile_data)" == 0 ]] \
    || return 1
  [[ "$(adb_scalar shell /system/bin/settings get global bluetooth_on)" == 0 ]] \
    || return 1
  [[ -z "$(adb_optional_scalar shell /system/bin/ip route show default)" ]] \
    || return 1
  [[ -z "$(adb_optional_scalar shell /system/bin/ip -6 route show default)" ]]
}

retain_private_raw_report() {
  local source="$1" expected_sha="$2" destination
  is_sha256 "$expected_sha" || return 1
  [[ -f "$source" && ! -L "$source" ]] || return 1
  [[ "$(sha256sum -- "$source" | awk '{print $1}')" == "$expected_sha" ]] \
    || return 1
  destination="$PRIVATE_EVIDENCE_ROOT/${EXPECTED_COMMIT}-${WORKFLOW_RUN_ID}-${WORKFLOW_RUN_ATTEMPT}-device-raw.txt"
  [[ ! -e "$destination" && ! -L "$destination" ]] || return 1
  PRIVATE_EVIDENCE_NEXT="$(mktemp "$PRIVATE_EVIDENCE_ROOT/.android-rish-physical-raw.XXXXXX")" \
    || return 1
  chmod 600 "$PRIVATE_EVIDENCE_NEXT" || return 1
  cp -- "$source" "$PRIVATE_EVIDENCE_NEXT" || return 1
  [[ "$(sha256sum -- "$PRIVATE_EVIDENCE_NEXT" | awk '{print $1}')" == "$expected_sha" ]] \
    || return 1
  mv -Tn -- "$PRIVATE_EVIDENCE_NEXT" "$destination" || return 1
  PRIVATE_EVIDENCE_NEXT=""
  canonical_regular_file "$destination" 600 1048576 || return 1
  [[ "$(stat -c '%u' -- "$destination")" == "$(id -u)" ]] || return 1
  [[ "$(sha256sum -- "$destination" | awk '{print $1}')" == "$expected_sha" ]]
}

promote_bounded_test_fixture_cleanup() {
  local source="$1" destination="$2"
  ((OFFLINE_POSTURE_PRE_OBSERVED == 1 \
    && OFFLINE_POSTURE_POST_OBSERVED == 1 \
    && CANDIDATE_EXECUTED == 1 \
    && SLOT_QUARANTINED == 1)) || return 1
  jq '
    if
      ([.validation.scenarioResults[]
        | select(.id == "bounded_test_fixture_cleanup")] | length) != 1
      or
      ([.validation.scenarioResults[]
        | select(
            .id == "bounded_test_fixture_cleanup"
            and .execution == "physical"
            and .outcome == "pending"
          )] | length) != 1
      or .cleanup != {
        candidateProcessGroupStopped:true,
        portReleased:true,
        deviceFixtureStateRemoved:false,
        controllerTransportRemoved:false
      }
      or .validation.scenarioResults[-1] != {
        id:"bounded_test_fixture_cleanup",
        execution:"physical",
        outcome:"pending"
      }
    then
      error("invalid intermediate cleanup state")
    else
      (.validation.scenarioResults[]
        | select(.id == "bounded_test_fixture_cleanup")).outcome = "pass"
      | .validation.controllerOfflinePosturePreCandidate = true
      | .validation.controllerOfflinePosturePostCandidate = true
      | .validation.scenarioResults = (
          [{id:"controller_offline_posture_pre_candidate",execution:"physical",outcome:"pass"}]
          + .validation.scenarioResults[0:-1]
          + [{id:"controller_offline_posture_post_candidate",execution:"physical",outcome:"pass"}]
          + [.validation.scenarioResults[-1]]
          + [{id:"device_slot_quarantined_after_candidate",execution:"physical",outcome:"pass"}]
        )
      | .cleanup.deviceFixtureStateRemoved = true
      | .cleanup.controllerTransportRemoved = true
      | .cleanup.deviceSlotQuarantinedAfterCandidate = true
    end
  ' "$source" >"$destination"
}

validate_artifact_bundle() {
  local manifest="$ARTIFACT_DIR/artifact-manifest.json"
  local checksums="$ARTIFACT_DIR/SHA256SUMS"
  local binary="$ARTIFACT_DIR/termux-mcp-server"
  local actual_sha actual_bytes expected_line
  [[ "$ARTIFACT_DIR" == /* && -d "$ARTIFACT_DIR" && ! -L "$ARTIFACT_DIR" ]] \
    || fail artifact_directory_invalid
  [[ "$(realpath -e -- "$ARTIFACT_DIR")" == "$ARTIFACT_DIR" ]] \
    || fail artifact_directory_invalid
  [[ "$(find "$ARTIFACT_DIR" -mindepth 1 -maxdepth 1 -type f | wc -l)" == 3 \
    && -z "$(find "$ARTIFACT_DIR" -mindepth 1 -maxdepth 1 ! -type f -print -quit)" ]] \
    || fail artifact_bundle_members_invalid
  for member in "$manifest" "$checksums" "$binary"; do
    [[ -f "$member" && ! -L "$member" ]] || fail artifact_bundle_member_invalid
  done
  [[ -x "$binary" ]] || fail artifact_binary_not_executable
  actual_sha="$(sha256sum -- "$binary" | awk '{print $1}')" || fail artifact_digest_failed
  actual_bytes="$(stat -c '%s' -- "$binary")" || fail artifact_size_invalid
  is_sha256 "$actual_sha" || fail artifact_digest_failed
  ((actual_bytes >= 1 && actual_bytes <= 67108864)) || fail artifact_size_invalid
  expected_line="$actual_sha  termux-mcp-server"
  [[ "$(wc -l <"$checksums")" == 1 && "$(<"$checksums")" == "$expected_line" ]] \
    || fail artifact_checksums_invalid
  jq -e \
    --arg repository "$REPOSITORY" \
    --arg commit "$EXPECTED_COMMIT" \
    --arg workflow_run_id "$WORKFLOW_RUN_ID" \
    --arg artifact_name "$ARTIFACT_NAME" \
    --arg version "$EXPECTED_VERSION" \
    --arg sha "$actual_sha" \
    --arg cargo_lock_sha "$CARGO_LOCK_SHA256" \
    --argjson bytes "$actual_bytes" '
    type == "object"
    and (keys == [
      "artifactClass",
      "artifactName",
      "bytes",
      "cargoLockSha256",
      "commit",
      "createdAt",
      "elf",
      "features",
      "fileName",
      "posture",
      "productionControlQualified",
      "releaseEligible",
      "repository",
      "schemaVersion",
      "sha256",
      "target",
      "version",
      "workflowRunId"
    ])
    and .schemaVersion == 1
    and .artifactClass == "android_rish_development_only_v1"
    and .releaseEligible == false
    and .productionControlQualified == false
    and .repository == $repository
    and .commit == $commit
    and .workflowRunId == $workflow_run_id
    and .artifactName == $artifact_name
    and .posture == "android-rish-development"
    and .features == ["android-rish"]
    and .target == "aarch64-linux-android"
    and .fileName == "termux-mcp-server"
    and .version == $version
    and .sha256 == $sha
    and .bytes == $bytes
    and .elf == "aarch64-android-elf"
    and .cargoLockSha256 == $cargo_lock_sha
    and (.createdAt | type == "string"
      and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"))
  ' "$manifest" >/dev/null || fail artifact_manifest_invalid
}

measure_remote_dex() {
  local parent output
  parent="$(dirname -- "$RISH_DEX_PATH")"
  output="$(ssh_command \
    "test -f '$RISH_DEX_PATH' \
      && test ! -L '$RISH_DEX_PATH' \
      && test \"\$(realpath -e '$RISH_DEX_PATH')\" = '$RISH_DEX_PATH' \
      && test \"\$(stat -c '%a' '$RISH_DEX_PATH')\" = 400 \
      && test \"\$(stat -c '%h' '$RISH_DEX_PATH')\" = 1 \
      && test \"\$(stat -c '%a' '$parent')\" = 700 \
      && test \"\$(sha256sum '$RISH_DEX_PATH' | awk '{print \$1}')\" = '$RISH_DEX_SHA256' \
      && stat -c '%s' '$RISH_DEX_PATH'" 2>/dev/null)" || return 1
  output="${output%$'\r'}"
  [[ "$output" =~ ^[1-9][0-9]{0,7}$ \
    && "$output" -le 16777216 ]] || return 1
  RISH_DEX_BYTES="$output"
}

validate_intermediate_evidence() {
  local evidence="$1" raw_report="$2" artifact_sha="$3" artifact_bytes="$4"
  local raw_sha started_at completed_at expected_raw
  raw_sha="$(sha256sum -- "$raw_report" | awk '{print $1}')" || return 1
  is_sha256 "$raw_sha" || return 1
  started_at="$(jq -er '.startedAt' "$evidence")" || return 1
  completed_at="$(jq -er '.completedAt' "$evidence")" || return 1
  [[ "$started_at" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$ \
    && "$completed_at" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$ ]] \
    || return 1
  [[ "$started_at" > "$completed_at" ]] && return 1

  expected_raw="$WORK_ROOT/expected-device-raw.txt"
  {
    printf 'gate_version=1\n'
    printf 'qualification_class=physical_shizuku_rish_identity_development_v1\n'
    printf 'scope=s2_5_uid_probe_only\n'
    printf 'started_at=%s\n' "$started_at"
    printf 'completed_at=%s\n' "$completed_at"
    printf 'repository=%s\n' "$REPOSITORY"
    printf 'commit=%s\n' "$EXPECTED_COMMIT"
    printf 'version=%s\n' "$EXPECTED_VERSION"
    printf 'workflow_run_id=%s\n' "$WORKFLOW_RUN_ID"
    printf 'workflow_run_attempt=%s\n' "$WORKFLOW_RUN_ATTEMPT"
    printf 'ci_run_id=%s\n' "$CI_RUN_ID"
    printf 'security_run_id=%s\n' "$SECURITY_RUN_ID"
    printf 'android_run_id=%s\n' "$ANDROID_RUN_ID"
    printf 'artifact_sha256=%s\n' "$artifact_sha"
    printf 'artifact_bytes=%s\n' "$artifact_bytes"
    printf 'rish_dex_sha256=%s\n' "$RISH_DEX_SHA256"
    printf 'rish_dex_bytes=%s\n' "$RISH_DEX_BYTES"
    printf 'api_level=%s\n' "$API_LEVEL"
    printf 'security_patch=%s\n' "$SECURITY_PATCH"
    printf 'adb_shell_uid=2000\n'
    printf 'trusted_direct_rish_probe_pre_candidate=pass\n'
    printf 'runtime_disabled_tool_absent=pass\n'
    printf 'candidate_mcp_status_uid_2000=pass\n'
    printf 'extra_arguments_rejected=pass\n'
    printf 'unknown_shell_rejected=pass\n'
    printf 'dex_tamper_rejected=pass\n'
    printf 'dex_mode_rejected=pass\n'
    printf 'dex_symlink_rejected=pass\n'
    printf 'all_mutation_gates_disabled=pass\n'
    printf 'trusted_direct_rish_probe_post_candidate=pass\n'
    printf 'bounded_test_fixture_cleanup=pending\n'
  } >"$expected_raw" || return 1
  cmp -s -- "$expected_raw" "$raw_report" || return 1

  jq -e \
    --arg repository "$REPOSITORY" \
    --arg commit "$EXPECTED_COMMIT" \
    --arg version "$EXPECTED_VERSION" \
    --arg policy_sha "$POLICY_SHA256" \
    --arg cargo_lock_sha "$CARGO_LOCK_SHA256" \
    --arg workflow_definition_sha "$WORKFLOW_DEFINITION_SHA256" \
    --arg workflow_run_id "$WORKFLOW_RUN_ID" \
    --argjson workflow_run_attempt "$WORKFLOW_RUN_ATTEMPT" \
    --arg ci_run_id "$CI_RUN_ID" \
    --arg security_run_id "$SECURITY_RUN_ID" \
    --arg android_run_id "$ANDROID_RUN_ID" \
    --arg challenge_sha "$CONTROLLER_CHALLENGE_SHA256" \
    --arg raw_sha "$raw_sha" \
    --arg artifact_sha "$artifact_sha" \
    --argjson artifact_bytes "$artifact_bytes" \
    --argjson api_level "$API_LEVEL" \
    --arg security_patch "$SECURITY_PATCH" \
    --arg device_profile "$DEVICE_PROFILE_COMMITMENT" \
    --arg fingerprint_sha "$BUILD_FINGERPRINT_SHA256" \
    --arg termux_version "$TERMUX_VERSION" \
    --arg termux_signer "$TERMUX_SIGNER_SHA256" \
    --arg shizuku_version "$SHIZUKU_VERSION" \
    --arg shizuku_signer "$SHIZUKU_SIGNER_SHA256" \
    --arg dex_sha "$RISH_DEX_SHA256" \
    --argjson dex_bytes "$RISH_DEX_BYTES" '
    type == "object"
    and (keys == [
      "artifact",
      "backend",
      "cargoLockSha256",
      "claimBoundary",
      "cleanup",
      "commit",
      "completedAt",
      "environment",
      "failureCode",
      "gateVersion",
      "policySha256",
      "productionControlQualified",
      "qualificationClass",
      "rawReportSha256",
      "releaseEligible",
      "repository",
      "schemaVersion",
      "scope",
      "startedAt",
      "status",
      "validation",
      "version",
      "workflow"
    ])
    and .schemaVersion == 1
    and .gateVersion == "1"
    and .status == "pass"
    and .failureCode == null
    and .releaseEligible == false
    and .productionControlQualified == false
    and .qualificationClass == "physical_shizuku_rish_identity_development_v1"
    and .scope == "s2_5_uid_probe_only"
    and .repository == $repository
    and .commit == $commit
    and .version == $version
    and .policySha256 == $policy_sha
    and .cargoLockSha256 == $cargo_lock_sha
    and .rawReportSha256 == $raw_sha
    and .workflow == {
      name:"Android Rish Physical Identity",
      definitionSha256:$workflow_definition_sha,
      runId:$workflow_run_id,
      runAttempt:$workflow_run_attempt,
      event:"workflow_dispatch",
      protectedEnvironment:"android-rish-physical-development",
      controllerChallengeSha256:$challenge_sha,
      ciRunId:$ci_run_id,
      securityRunId:$security_run_id,
      androidRunId:$android_run_id
    }
    and .artifact == {
      artifactName:"termux-mcp-server-aarch64-linux-android-android-rish-development",
      posture:"android-rish-development",
      features:["android-rish"],
      target:"aarch64-linux-android",
      sha256:$artifact_sha,
      bytes:$artifact_bytes
    }
    and .environment == {
      physicalDeviceObserved:true,
      androidFrameworkObserved:true,
      architecture:"aarch64",
      apiLevel:$api_level,
      securityPatch:$security_patch,
      deviceProfileCommitment:$device_profile,
      buildFingerprintSha256:$fingerprint_sha,
      termuxVersion:$termux_version,
      termuxSignerSha256:$termux_signer,
      shizukuVersion:$shizuku_version,
      shizukuSignerSha256:$shizuku_signer,
      adbShellUid:2000,
      shizukuStartMode:"adb"
    }
    and .backend == {
      name:"shizuku_rish",
      dexSha256:$dex_sha,
      dexBytes:$dex_bytes,
      dexMode:"0400",
      dexParentMode:"0700",
      dexLinkCount:1,
      dexOwnerMatchesTermuxUid:true,
      dexCanonicalPrivatePath:true,
      principal:"android_shell",
      uid:2000,
      state:"verified_shell_uid",
      rootAccepted:false,
      arbitraryShell:false,
      mutationReady:false
    }
    and .validation == {
      disabledToolCount:17,
      enabledToolCount:18,
      exactToolOrder:true,
      emptyArgumentsSchema:true,
      extraArgumentsRejected:true,
      unknownShellRejected:true,
      allMutationGatesDisabled:true,
      trustedDirectRishProbePreCandidate:true,
      trustedDirectRishProbePostCandidate:true,
      dexTamperRejected:true,
      dexModeRejected:true,
      dexSymlinkRejected:true,
      scenarioResults:[
        {id:"trusted_direct_rish_probe_pre_candidate",execution:"physical",outcome:"pass"},
        {id:"runtime_disabled_tool_absent",execution:"physical",outcome:"pass"},
        {id:"candidate_mcp_status_uid_2000",execution:"physical",outcome:"pass"},
        {id:"extra_arguments_rejected",execution:"physical",outcome:"pass"},
        {id:"unknown_shell_rejected",execution:"physical",outcome:"pass"},
        {id:"dex_tamper_rejected",execution:"physical",outcome:"pass"},
        {id:"dex_mode_rejected",execution:"physical",outcome:"pass"},
        {id:"dex_symlink_rejected",execution:"physical",outcome:"pass"},
        {id:"all_mutation_gates_disabled",execution:"physical",outcome:"pass"},
        {id:"trusted_direct_rish_probe_post_candidate",execution:"physical",outcome:"pass"},
        {id:"bounded_test_fixture_cleanup",execution:"physical",outcome:"pending"}
      ]
    }
    and .claimBoundary == {
      s3Attestation:false,
      typedReads:false,
      grantV2:false,
      deviceMutation:false,
      productionControl:false,
      sameUidPersistenceExcluded:false,
      continuousNetworkIsolation:false,
      adversarialNetworkIsolation:false
    }
    and .cleanup == {
      candidateProcessGroupStopped:true,
      portReleased:true,
      deviceFixtureStateRemoved:false,
      controllerTransportRemoved:false
    }
  ' "$evidence" >/dev/null
}

package_version() {
  local package dump version
  package="$1"
  dump="$WORK_ROOT/$package.dump"
  timeout --signal=TERM --kill-after=2s 15s \
    adb -s "$ADB_SERIAL" shell dumpsys package "$package" >"$dump" 2>/dev/null \
    || return 1
  [[ "$(stat -c '%s' "$dump")" -le 1048576 ]] || return 1
  version="$(awk -F= '
    /^[[:space:]]*versionName=/ {
      sub(/^[[:space:]]*/, "", $2)
      if (found && value != $2) exit 2
      value = $2
      found = 1
    }
    END { if (!found) exit 1; print value }
  ' "$dump")" || return 1
  [[ "$version" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] || return 1
  printf '%s\n' "$version"
}

package_signer_sha256() {
  local package package_paths apk output
  local remote_path signer
  package="$1"
  package_paths="$WORK_ROOT/$package.paths"
  apk="$WORK_ROOT/$package.base.apk"
  output="$WORK_ROOT/$package.apksigner"
  adb_command shell pm path "$package" >"$package_paths" 2>/dev/null || return 1
  [[ "$(stat -c '%s' "$package_paths")" -le 65536 ]] || return 1
  remote_path="$(sed -n 's/\r$//; s/^package://p' "$package_paths" \
    | awk '/\/base[.]apk$/ { if (found) exit 2; value=$0; found=1 } END { if (!found) exit 1; print value }')" \
    || return 1
  [[ "$remote_path" =~ ^/data/app/[A-Za-z0-9_+=~./-]+/base\.apk$ \
    && "$remote_path" != *"/../"* && "$remote_path" != *"//"* ]] || return 1
  adb_command pull "$remote_path" "$apk" >/dev/null 2>&1 || return 1
  [[ -f "$apk" && ! -L "$apk" && "$(stat -c '%s' "$apk")" -le 268435456 ]] || return 1
  timeout --signal=TERM --kill-after=2s 30s \
    apksigner verify --print-certs "$apk" >"$output" 2>/dev/null || return 1
  signer="$(awk -F': ' '
    /^Signer #[0-9]+ certificate SHA-256 digest: / {
      value=tolower($2)
      count++
    }
    END { if (count != 1) exit 1; print value }
  ' "$output")" || return 1
  is_sha256 "$signer" || return 1
  printf '%s\n' "$signer"
}

configure_transport() {
  local forward_status=0
  LOCAL_PORT="$(adb_command forward tcp:0 "tcp:$SSH_DEVICE_PORT" 2>/dev/null)" \
    || forward_status=$?
  if [[ "$LOCAL_PORT" =~ ^[1-9][0-9]{0,4}$ && "$LOCAL_PORT" -le 65535 ]]; then
    FORWARD_CREATED=1
  else
    CLEANUP_CONFIRMED=0
  fi
  ((forward_status == 0)) || fail adb_forward_failed
  ((FORWARD_CREATED == 1)) || fail adb_forward_invalid
  SSH_TARGET="$SSH_USER@127.0.0.1"
  SSH_OPTIONS=(
    -p "$LOCAL_PORT"
    -i "$SSH_IDENTITY_FILE"
    -o "UserKnownHostsFile=$SSH_KNOWN_HOSTS_FILE"
    -o GlobalKnownHostsFile=/dev/null
    -o StrictHostKeyChecking=yes
    -o IdentitiesOnly=yes
    -o BatchMode=yes
    -o PasswordAuthentication=no
    -o KbdInteractiveAuthentication=no
    -o PubkeyAuthentication=yes
    -o ForwardAgent=no
    -o ForwardX11=no
    -o PermitLocalCommand=no
    -o ClearAllForwardings=yes
    -o ConnectTimeout=10
    -o ServerAliveInterval=5
    -o ServerAliveCountMax=2
    -o LogLevel=ERROR
  )
  SCP_OPTIONS=(
    -P "$LOCAL_PORT"
    -i "$SSH_IDENTITY_FILE"
    -o "UserKnownHostsFile=$SSH_KNOWN_HOSTS_FILE"
    -o GlobalKnownHostsFile=/dev/null
    -o StrictHostKeyChecking=yes
    -o IdentitiesOnly=yes
    -o BatchMode=yes
    -o PasswordAuthentication=no
    -o KbdInteractiveAuthentication=no
    -o PubkeyAuthentication=yes
    -o ForwardAgent=no
    -o ForwardX11=no
    -o ClearAllForwardings=yes
    -o ConnectTimeout=10
    -o LogLevel=ERROR
  )
}

create_remote_root() {
  REMOTE_CREATED=1
  ssh_command "umask 077; test ! -e '$REMOTE_ROOT'; mkdir -m 700 '$REMOTE_ROOT'" \
    >/dev/null 2>&1 || fail remote_root_create_failed
}

run_device_gate() {
  local binary="$ARTIFACT_DIR/termux-mcp-server"
  local artifact_sha artifact_bytes remote_binary remote_evidence remote_raw
  local local_evidence="$WORK_ROOT/device-evidence.json"
  local local_raw="$WORK_ROOT/device-raw.txt"
  local remote_command gate_stdout="$WORK_ROOT/gate.stdout" gate_stderr="$WORK_ROOT/gate.stderr"
  artifact_sha="$(sha256sum -- "$binary" | awk '{print $1}')"
  artifact_bytes="$(stat -c '%s' -- "$binary")"
  measure_remote_dex || fail remote_rish_dex_measurement_failed
  REMOTE_ROOT="/data/data/com.termux/files/home/.termux-mcp-rish-controller-${EXPECTED_COMMIT:0:12}-$WORKFLOW_RUN_ID"
  remote_binary="$REMOTE_ROOT/termux-mcp-server"
  remote_evidence="$REMOTE_ROOT/device-evidence.json"
  remote_raw="$REMOTE_ROOT/device-raw.txt"
  create_remote_root
  scp_to_device "$binary" "$remote_binary" >/dev/null 2>&1 \
    || fail artifact_transfer_failed
  ssh_command \
    "chmod 700 '$remote_binary' && test \"\$(sha256sum '$remote_binary' | awk '{print \$1}')\" = '$artifact_sha'" \
    >/dev/null 2>&1 || fail remote_artifact_validation_failed

  printf -v remote_command '%q ' \
    bash -s -- \
    --artifact "$remote_binary" \
    --artifact-sha256 "$artifact_sha" \
    --expected-commit "$EXPECTED_COMMIT" \
    --expected-version "$EXPECTED_VERSION" \
    --policy-sha256 "$POLICY_SHA256" \
    --cargo-lock-sha256 "$CARGO_LOCK_SHA256" \
    --workflow-definition-sha256 "$WORKFLOW_DEFINITION_SHA256" \
    --workflow-run-id "$WORKFLOW_RUN_ID" \
    --workflow-run-attempt "$WORKFLOW_RUN_ATTEMPT" \
    --ci-run-id "$CI_RUN_ID" \
    --security-run-id "$SECURITY_RUN_ID" \
    --android-run-id "$ANDROID_RUN_ID" \
    --controller-challenge-sha256 "$CONTROLLER_CHALLENGE_SHA256" \
    --rish-dex "$RISH_DEX_PATH" \
    --rish-dex-sha256 "$RISH_DEX_SHA256" \
    --api-level "$API_LEVEL" \
    --security-patch "$SECURITY_PATCH" \
    --device-profile-commitment "$DEVICE_PROFILE_COMMITMENT" \
    --build-fingerprint-sha256 "$BUILD_FINGERPRINT_SHA256" \
    --termux-version "$TERMUX_VERSION" \
    --termux-signer-sha256 "$TERMUX_SIGNER_SHA256" \
    --shizuku-version "$SHIZUKU_VERSION" \
    --shizuku-signer-sha256 "$SHIZUKU_SIGNER_SHA256" \
    --adb-shell-uid "$ADB_SHELL_UID" \
    --raw-report "$remote_raw" \
    --output "$remote_evidence"
  CANDIDATE_EXECUTED=1
  # Persist the one-shot quarantine before candidate execution so abrupt
  # controller termination cannot make this slot silently reusable.
  quarantine_slot || fail device_slot_quarantine_failed
  if ! timeout --signal=TERM --kill-after=10s 900s \
    ssh "${SSH_OPTIONS[@]}" "$SSH_TARGET" "$remote_command" \
      <"$DEVICE_GATE" >"$gate_stdout" 2>"$gate_stderr"
  then
    fail device_gate_failed
  fi
  [[ "$(<"$gate_stdout")" == TERMUX_RISH_PHYSICAL_GATE_RESULT=PASS \
    && ! -s "$gate_stderr" ]] || fail device_gate_output_invalid
  validate_offline_device_posture || fail offline_posture_post_candidate_invalid
  OFFLINE_POSTURE_POST_OBSERVED=1
  scp_from_device "$remote_evidence" "$local_evidence" >/dev/null 2>&1 \
    || fail evidence_transfer_failed
  scp_from_device "$remote_raw" "$local_raw" >/dev/null 2>&1 \
    || fail raw_report_transfer_failed
  [[ -f "$local_evidence" && ! -L "$local_evidence" \
    && -f "$local_raw" && ! -L "$local_raw" ]] || fail transferred_evidence_invalid
  chmod 600 "$local_evidence" "$local_raw" || fail transferred_evidence_mode_failed
  [[ "$(sha256sum -- "$local_raw" | awk '{print $1}')" \
    == "$(jq -r '.rawReportSha256' "$local_evidence")" ]] \
    || fail raw_report_digest_mismatch
  local raw_report_sha
  raw_report_sha="$(sha256sum -- "$local_raw" | awk '{print $1}')"
  is_sha256 "$raw_report_sha" || fail raw_report_digest_mismatch
  validate_intermediate_evidence \
    "$local_evidence" "$local_raw" "$artifact_sha" "$artifact_bytes" \
    || fail intermediate_evidence_reconciliation_failed

  remove_remote_root || fail remote_cleanup_failed
  remove_forward || fail transport_cleanup_failed
  retain_private_raw_report "$local_raw" "$raw_report_sha" \
    || fail private_raw_report_retention_failed
  quarantine_slot || fail device_slot_quarantine_failed

  PUBLISH_NEXT="$(mktemp "$(dirname -- "$OUTPUT")/.android-rish-physical-final.XXXXXX")" \
    || fail evidence_finalization_failed
  promote_bounded_test_fixture_cleanup "$local_evidence" "$PUBLISH_NEXT" \
    || fail evidence_finalization_failed
  chmod 600 "$PUBLISH_NEXT" || fail evidence_finalization_failed
  jq -e '
    .status == "pass"
    and .releaseEligible == false
    and .productionControlQualified == false
    and .qualificationClass == "physical_shizuku_rish_identity_development_v1"
    and .scope == "s2_5_uid_probe_only"
    and .backend.uid == 2000
    and .backend.rootAccepted == false
    and .backend.arbitraryShell == false
    and .backend.mutationReady == false
    and .validation.controllerOfflinePosturePreCandidate == true
    and .validation.controllerOfflinePosturePostCandidate == true
    and .claimBoundary.sameUidPersistenceExcluded == false
    and ([.validation.scenarioResults[]
      | select(.id == "bounded_test_fixture_cleanup")] == [
        {id:"bounded_test_fixture_cleanup", execution:"physical", outcome:"pass"}
      ])
    and .cleanup == {
      candidateProcessGroupStopped:true,
      portReleased:true,
      deviceFixtureStateRemoved:true,
      controllerTransportRemoved:true,
      deviceSlotQuarantinedAfterCandidate:true
    }
  ' "$PUBLISH_NEXT" >/dev/null || fail evidence_finalization_failed
  if grep -Eq \
    '/data/|Bearer[[:space:]]|MCP__|localhost|127\.0\.0\.1|[[:space:]]serial[[:space:]]|ssh-' \
    "$PUBLISH_NEXT"
  then
    fail evidence_not_sanitized
  fi
  mv -Tn -- "$PUBLISH_NEXT" "$OUTPUT" || fail evidence_publication_failed
  PUBLISH_NEXT=""
  [[ -f "$OUTPUT" && ! -L "$OUTPUT" && "$(stat -c '%a' "$OUTPUT")" == 600 ]] \
    || fail evidence_publication_failed
}

main() {
  while (($#)); do
    case "$1" in
      --artifact-dir) (($# >= 2)) || fail missing_artifact_directory; ARTIFACT_DIR="$2"; shift 2 ;;
      --expected-commit) (($# >= 2)) || fail missing_expected_commit; EXPECTED_COMMIT="$2"; shift 2 ;;
      --expected-version) (($# >= 2)) || fail missing_expected_version; EXPECTED_VERSION="$2"; shift 2 ;;
      --policy-sha256) (($# >= 2)) || fail missing_policy_digest; POLICY_SHA256="$2"; shift 2 ;;
      --cargo-lock-sha256) (($# >= 2)) || fail missing_cargo_lock_digest; CARGO_LOCK_SHA256="$2"; shift 2 ;;
      --workflow-definition-sha256) (($# >= 2)) || fail missing_workflow_definition_digest; WORKFLOW_DEFINITION_SHA256="$2"; shift 2 ;;
      --workflow-run-id) (($# >= 2)) || fail missing_workflow_run_id; WORKFLOW_RUN_ID="$2"; shift 2 ;;
      --workflow-run-attempt) (($# >= 2)) || fail missing_workflow_run_attempt; WORKFLOW_RUN_ATTEMPT="$2"; shift 2 ;;
      --ci-run-id) (($# >= 2)) || fail missing_ci_run_id; CI_RUN_ID="$2"; shift 2 ;;
      --security-run-id) (($# >= 2)) || fail missing_security_run_id; SECURITY_RUN_ID="$2"; shift 2 ;;
      --android-run-id) (($# >= 2)) || fail missing_android_run_id; ANDROID_RUN_ID="$2"; shift 2 ;;
      --controller-challenge-file) (($# >= 2)) || fail missing_controller_challenge; CONTROLLER_CHALLENGE_FILE="$2"; shift 2 ;;
      --device-slot) (($# >= 2)) || fail missing_device_slot; DEVICE_SLOT="$2"; shift 2 ;;
      --device-gate) (($# >= 2)) || fail missing_device_gate; DEVICE_GATE="$2"; shift 2 ;;
      --output) (($# >= 2)) || fail missing_output; OUTPUT="$2"; shift 2 ;;
      -h|--help) usage; return 0 ;;
      *) fail unknown_argument ;;
    esac
  done

  [[ "$EXPECTED_COMMIT" =~ ^[0-9a-f]{40}$ ]] || fail expected_commit_invalid
  [[ "$EXPECTED_VERSION" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] || fail expected_version_invalid
  for digest in "$POLICY_SHA256" "$CARGO_LOCK_SHA256" "$WORKFLOW_DEFINITION_SHA256"; do
    is_sha256 "$digest" || fail input_digest_invalid
  done
  for run_id in "$WORKFLOW_RUN_ID" "$CI_RUN_ID" "$SECURITY_RUN_ID" "$ANDROID_RUN_ID"; do
    [[ "$run_id" =~ ^[1-9][0-9]{0,19}$ ]] || fail workflow_run_binding_invalid
  done
  [[ "$WORKFLOW_RUN_ATTEMPT" == 1 ]] || fail workflow_run_attempt_invalid
  [[ "$DEVICE_SLOT" =~ ^[a-z0-9][a-z0-9_-]{0,31}$ ]] || fail device_slot_invalid

  for command_name in adb apksigner awk bash chmod cmp cp dirname find flock grep id jq mktemp mv realpath rm scp sed sha256sum ssh stat sync timeout wc; do
    require_command "$command_name"
  done
  private_directory "$CONFIG_ROOT" || fail config_root_invalid
  [[ ! -e "$CONFIG_ROOT/.quarantine-$DEVICE_SLOT" \
    && ! -L "$CONFIG_ROOT/.quarantine-$DEVICE_SLOT" ]] || fail device_slot_quarantined
  exec 9>"$CONFIG_ROOT/.lock-$DEVICE_SLOT"
  flock -n 9 || fail device_slot_busy
  read_device_config

  canonical_regular_file "$DEVICE_GATE" 755 1048576 || fail device_gate_invalid
  [[ "$(stat -c '%u' -- "$DEVICE_GATE")" == "$(id -u)" ]] || fail device_gate_invalid
  canonical_regular_file "$CONTROLLER_CHALLENGE_FILE" 600 32 \
    || fail controller_challenge_invalid
  [[ "$(stat -c '%s:%u' -- "$CONTROLLER_CHALLENGE_FILE")" == "32:$(id -u)" ]] \
    || fail controller_challenge_invalid
  CONTROLLER_CHALLENGE_SHA256="$(sha256sum -- "$CONTROLLER_CHALLENGE_FILE" | awk '{print $1}')"
  is_sha256 "$CONTROLLER_CHALLENGE_SHA256" || fail controller_challenge_invalid
  [[ "$OUTPUT" == /* && ! -e "$OUTPUT" && ! -L "$OUTPUT" ]] || fail output_invalid
  private_directory "$(dirname -- "$OUTPUT")" || fail output_parent_invalid
  validate_artifact_bundle

  WORK_ROOT="$(mktemp -d /tmp/termux-mcp-rish-controller.XXXXXX)" \
    || fail work_root_create_failed
  chmod 700 "$WORK_ROOT" || fail work_root_mode_failed
  trap cleanup EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM HUP

  [[ "$(adb_scalar get-state)" == device ]] || fail adb_device_unavailable
  validate_usb_transport || fail physical_usb_transport_invalid
  [[ "$(adb_scalar shell id -u)" == 2000 ]] || fail adb_shell_uid_invalid
  ADB_SHELL_UID=2000
  validate_non_emulator || fail emulator_not_allowed
  [[ "$(adb_scalar shell getprop ro.product.cpu.abi)" == arm64-v8a ]] || fail device_abi_invalid
  [[ "$(adb_scalar shell uname -m)" == aarch64 ]] || fail device_architecture_invalid
  [[ "$(adb_scalar shell getprop ro.build.type)" == user ]] || fail build_type_invalid
  [[ "$(adb_scalar shell getprop ro.boot.verifiedbootstate)" == green ]] \
    || fail verified_boot_invalid
  [[ "$(adb_scalar shell getprop ro.boot.flash.locked)" == 1 ]] \
    || fail bootloader_state_invalid
  API_LEVEL="$(adb_scalar shell getprop ro.build.version.sdk)" || fail api_level_probe_failed
  [[ "$API_LEVEL" =~ ^[0-9]+$ ]] && ((API_LEVEL >= 30 && API_LEVEL <= 36)) \
    || fail api_level_invalid
  SECURITY_PATCH="$(adb_scalar shell getprop ro.build.version.security_patch)" \
    || fail security_patch_probe_failed
  [[ "$SECURITY_PATCH" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] \
    || fail security_patch_invalid
  local build_fingerprint
  build_fingerprint="$(adb_scalar shell getprop ro.build.fingerprint)" \
    || fail build_fingerprint_probe_failed
  BUILD_FINGERPRINT_SHA256="$(printf '%s' "$build_fingerprint" | sha256sum | awk '{print $1}')"
  unset build_fingerprint
  is_sha256 "$BUILD_FINGERPRINT_SHA256" || fail build_fingerprint_probe_failed

  TERMUX_VERSION="$(package_version "$TERMUX_PACKAGE")" || fail termux_version_probe_failed
  [[ "$TERMUX_VERSION" == "$EXPECTED_TERMUX_VERSION" ]] || fail termux_version_mismatch
  TERMUX_SIGNER_SHA256="$(package_signer_sha256 "$TERMUX_PACKAGE")" \
    || fail termux_signer_probe_failed
  [[ "$TERMUX_SIGNER_SHA256" == "$EXPECTED_TERMUX_SIGNER_SHA256" ]] \
    || fail termux_signer_mismatch
  SHIZUKU_VERSION="$(package_version "$SHIZUKU_PACKAGE")" \
    || fail shizuku_version_probe_failed
  [[ "$SHIZUKU_VERSION" == "$EXPECTED_SHIZUKU_VERSION" ]] \
    || fail shizuku_version_mismatch
  SHIZUKU_SIGNER_SHA256="$(package_signer_sha256 "$SHIZUKU_PACKAGE")" \
    || fail shizuku_signer_probe_failed
  [[ "$SHIZUKU_SIGNER_SHA256" == "$EXPECTED_SHIZUKU_SIGNER_SHA256" ]] \
    || fail shizuku_signer_mismatch

  validate_offline_device_posture || fail offline_posture_pre_candidate_invalid
  OFFLINE_POSTURE_PRE_OBSERVED=1
  configure_transport
  run_device_gate
  printf 'SHIZUKU_RISH_PHYSICAL_CONTROLLER_RESULT=PASS\n'
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
