#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077

ROOT="$(mktemp -d)"
trap 'rm -rf -- "$ROOT"' EXIT INT TERM
chmod 700 "$ROOT"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
CONTROLLER="$REPO_ROOT/scripts/shizuku_rish_physical_controller.sh"
COMMIT=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
CARGO_LOCK_SHA=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
DIGEST=cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc
REAL_PATH="$PATH"

fail_test() {
  printf 'Shizuku rish physical controller test failed: %s\n' "$1" >&2
  exit 1
}

assert_fails() {
  local reason="$1"
  shift
  if "$@" >"$ROOT/last.stdout" 2>"$ROOT/last.stderr"; then
    fail_test "command unexpectedly succeeded: $reason"
  fi
  grep -Fxq "SHIZUKU_RISH_PHYSICAL_CONTROLLER_RESULT=FAIL reason=$reason" \
    "$ROOT/last.stderr" \
    || fail_test "expected failure reason was absent: $reason"
  [[ ! -s "$ROOT/last.stdout" ]] \
    || fail_test "failed controller wrote standard output: $reason"
}

bash -n "$CONTROLLER"
bash -n "${BASH_SOURCE[0]}"
bash "$CONTROLLER" --help | grep -Fq 'Usage:' \
  || fail_test help_contract_missing
assert_fails expected_commit_invalid bash "$CONTROLLER"
assert_fails expected_version_invalid \
  bash "$CONTROLLER" --expected-commit "$COMMIT" --expected-version 'not valid'

grep -Fq 'bash -s --' "$CONTROLLER" \
  || fail_test trusted_stdin_gate_execution_missing
grep -Fq 'ClearAllForwardings=yes' "$CONTROLLER" \
  || fail_test ssh_forwarding_lockdown_missing
grep -Fq 'StrictHostKeyChecking=yes' "$CONTROLLER" \
  || fail_test ssh_host_key_pinning_missing
grep -Fq 'IdentitiesOnly=yes' "$CONTROLLER" \
  || fail_test ssh_identity_pinning_missing
grep -Fq 'BatchMode=yes' "$CONTROLLER" \
  || fail_test ssh_interactive_auth_not_disabled
grep -Fq 'releaseEligible == false' "$CONTROLLER" \
  || fail_test release_ineligibility_not_asserted
grep -Fq 'productionControlQualified == false' "$CONTROLLER" \
  || fail_test production_control_boundary_not_asserted
grep -Fq 'sync -f -- "$marker"' "$CONTROLLER" \
  || fail_test quarantine_marker_durability_sync_missing
[[ "$(grep -Fc '      --security-run-id)' "$CONTROLLER")" == 1 ]] \
  || fail_test security_run_id_argument_case_not_unique
if grep -Eq \
  '(^|[[:space:]])(pkg|apt|apt-get|dpkg)[[:space:]]+(install|update|upgrade)|adb[[:space:]]+root' \
  "$CONTROLLER"
then
  fail_test package_or_root_escalation_present
fi
if grep -Eq '(^|[[:space:]])(source|\.)[[:space:]]+["'\'']?\$DEVICE_CONFIG|(^|[[:space:]])eval[[:space:]]' \
  "$CONTROLLER"
then
  fail_test private_config_code_execution_present
fi
if grep -Eq 'scp_to_device[[:space:]]+"?\\$DEVICE_GATE|scp_to_device[[:space:]].*scripts/' \
  "$CONTROLLER"
then
  fail_test candidate_script_transfer_present
fi

mkdir -m 700 "$ROOT/config" "$ROOT/ssh" "$ROOT/private-evidence"
printf 'test identity\n' >"$ROOT/ssh/id"
printf '127.0.0.1 ssh-ed25519 test-key\n' >"$ROOT/ssh/known_hosts"
chmod 600 "$ROOT/ssh/id" "$ROOT/ssh/known_hosts"

write_config() {
  local root="$1" slot="$2" rish_path="$3" termux_version="$4"
  mkdir -p "$root"
  chmod 700 "$root"
  jq -n \
    --arg slot "$slot" \
    --arg identity "$ROOT/ssh/id" \
    --arg known_hosts "$ROOT/ssh/known_hosts" \
    --arg private_evidence "$ROOT/private-evidence" \
    --arg rish_path "$rish_path" \
    --arg termux_version "$termux_version" \
    --arg digest "$DIGEST" '
    {
      schemaVersion:1,
      slot:$slot,
      adbSerial:"physical-serial-01",
      sshUser:"termux",
      sshIdentityFile:$identity,
      sshKnownHostsFile:$known_hosts,
      privateEvidenceRoot:$private_evidence,
      sshDevicePort:8022,
      rishDexPath:$rish_path,
      rishDexSha256:$digest,
      deviceProfileCommitment:$digest,
      termuxVersion:$termux_version,
      termuxSignerSha256:$digest,
      shizukuVersion:"13.5.4",
      shizukuSignerSha256:$digest
    }
  ' >"$root/$slot.json"
  chmod 600 "$root/$slot.json"
}

VALID_CONFIG="$ROOT/config-valid"
write_config \
  "$VALID_CONFIG" slot-a \
  /data/data/com.termux/files/home/.local/share/shizuku/rish_shizuku.dex \
  0.118.3
(
  # shellcheck source=/dev/null
  source "$CONTROLLER"
  CONFIG_ROOT="$VALID_CONFIG"
  DEVICE_SLOT=slot-a
  read_device_config
  [[ "$ADB_SERIAL" == physical-serial-01 ]] \
    || fail_test adb_serial_not_loaded
  [[ "$RISH_DEX_PATH" == /data/data/com.termux/files/home/.local/share/shizuku/rish_shizuku.dex ]] \
    || fail_test rish_path_not_loaded
  [[ "$EXPECTED_TERMUX_VERSION" == 0.118.3 ]] \
    || fail_test termux_version_not_loaded
  [[ "$PRIVATE_EVIDENCE_ROOT" == "$ROOT/private-evidence" ]] \
    || fail_test private_evidence_root_not_loaded
)

EXTRA_CONFIG="$ROOT/config-extra"
write_config \
  "$EXTRA_CONFIG" slot-b \
  /data/data/com.termux/files/home/.local/share/shizuku/rish_shizuku.dex \
  0.118.3
jq '.unexpected = "rejected"' "$EXTRA_CONFIG/slot-b.json" \
  >"$EXTRA_CONFIG/slot-b.next"
mv "$EXTRA_CONFIG/slot-b.next" "$EXTRA_CONFIG/slot-b.json"
chmod 600 "$EXTRA_CONFIG/slot-b.json"
assert_fails device_config_contract_invalid \
  bash -c '
    source "$1"
    CONFIG_ROOT="$2"
    DEVICE_SLOT=slot-b
    read_device_config
  ' _ "$CONTROLLER" "$EXTRA_CONFIG"

UNSAFE_CONFIG="$ROOT/config-unsafe"
write_config \
  "$UNSAFE_CONFIG" slot-c \
  /data/data/com.termux/files/home/../files/usr/bin/rish.dex \
  0.118.3
assert_fails rish_dex_path_invalid \
  bash -c '
    source "$1"
    CONFIG_ROOT="$2"
    DEVICE_SLOT=slot-c
    read_device_config
  ' _ "$CONTROLLER" "$UNSAFE_CONFIG"

VERSION_CONFIG="$ROOT/config-version"
write_config \
  "$VERSION_CONFIG" slot-d \
  /data/data/com.termux/files/home/.local/share/shizuku/rish_shizuku.dex \
  '0.118.3 invalid'
assert_fails termux_version_policy_invalid \
  bash -c '
    source "$1"
    CONFIG_ROOT="$2"
    DEVICE_SLOT=slot-d
    read_device_config
  ' _ "$CONTROLLER" "$VERSION_CONFIG"

make_bundle() {
  local bundle="$1"
  local sha bytes
  mkdir -m 700 "$bundle"
  printf '%s\n' '#!/usr/bin/env bash' 'exit 0' >"$bundle/termux-mcp-server"
  chmod 700 "$bundle/termux-mcp-server"
  sha="$(sha256sum "$bundle/termux-mcp-server" | awk '{print $1}')"
  bytes="$(stat -c '%s' "$bundle/termux-mcp-server")"
  printf '%s  termux-mcp-server\n' "$sha" >"$bundle/SHA256SUMS"
  jq -n \
    --arg commit "$COMMIT" \
    --arg sha "$sha" \
    --arg cargo_lock_sha "$CARGO_LOCK_SHA" \
    --argjson bytes "$bytes" '
    {
      schemaVersion:1,
      artifactClass:"android_rish_development_only_v1",
      releaseEligible:false,
      productionControlQualified:false,
      repository:"CyberBASSLord-666/termux-mcp-edge",
      commit:$commit,
      workflowRunId:"1001",
      artifactName:"termux-mcp-server-aarch64-linux-android-android-rish-development",
      posture:"android-rish-development",
      features:["android-rish"],
      target:"aarch64-linux-android",
      fileName:"termux-mcp-server",
      version:"0.7.0",
      sha256:$sha,
      bytes:$bytes,
      elf:"aarch64-android-elf",
      cargoLockSha256:$cargo_lock_sha,
      createdAt:"2026-07-31T12:00:00Z"
    }
  ' >"$bundle/artifact-manifest.json"
  chmod 600 "$bundle/SHA256SUMS" "$bundle/artifact-manifest.json"
}

validate_bundle() {
  local bundle="$1"
  bash -c '
    source "$1"
    ARTIFACT_DIR="$2"
    EXPECTED_COMMIT="$3"
    EXPECTED_VERSION=0.7.0
    WORKFLOW_RUN_ID=1001
    CARGO_LOCK_SHA256="$4"
    validate_artifact_bundle
  ' _ "$CONTROLLER" "$bundle" "$COMMIT" "$CARGO_LOCK_SHA"
}

BUNDLE="$ROOT/bundle"
make_bundle "$BUNDLE"
validate_bundle "$BUNDLE"

jq '.commit = "dddddddddddddddddddddddddddddddddddddddd"' \
  "$BUNDLE/artifact-manifest.json" >"$BUNDLE/manifest.next"
mv "$BUNDLE/manifest.next" "$BUNDLE/artifact-manifest.json"
chmod 600 "$BUNDLE/artifact-manifest.json"
assert_fails artifact_manifest_invalid validate_bundle "$BUNDLE"
rm -rf "$BUNDLE"
make_bundle "$BUNDLE"

printf 'unexpected\n' >"$BUNDLE/extra"
assert_fails artifact_bundle_members_invalid validate_bundle "$BUNDLE"
rm -f "$BUNDLE/extra"

jq '.artifactClass = "release"' "$BUNDLE/artifact-manifest.json" \
  >"$BUNDLE/manifest.next"
mv "$BUNDLE/manifest.next" "$BUNDLE/artifact-manifest.json"
chmod 600 "$BUNDLE/artifact-manifest.json"
assert_fails artifact_manifest_invalid validate_bundle "$BUNDLE"
rm -rf "$BUNDLE"
make_bundle "$BUNDLE"

printf '%s\n' "$DIGEST  termux-mcp-server" >"$BUNDLE/SHA256SUMS"
assert_fails artifact_checksums_invalid validate_bundle "$BUNDLE"

mkdir -m 700 "$ROOT/fake-bin" "$ROOT/probes"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'set -eu' \
  'if [[ -n "${FAKE_ADB_LOG:-}" ]]; then printf "%s\\n" "$*" >>"$FAKE_ADB_LOG"; fi' \
  'if [[ "$*" == *"shell dumpsys package"* ]]; then' \
  '  if [[ "${FAKE_ADB_AMBIGUOUS:-0}" == 1 ]]; then' \
  '    printf "  versionName=0.118.3\\n  versionName=0.119.0\\n"' \
  '  else' \
  '    printf "  versionName=0.118.3\\n"' \
  '  fi' \
  'elif [[ "$*" == *"shell pm path"* ]]; then' \
  '  printf "package:/data/app/~~fixture/com.termux-fixture/base.apk\\n"' \
  'elif [[ "$*" == *" pull "* ]]; then' \
  '  printf "fake apk\\n" >"${@: -1}"' \
  'elif [[ "$*" == *"forward tcp:0"* ]]; then' \
  '  printf "41234\\n"' \
  '  [[ "${FAKE_FORWARD_FAIL:-0}" == 1 ]] && exit 70' \
  'elif [[ "$*" == *" get-devpath"* ]]; then' \
  '  if [[ "${FAKE_WIRELESS:-0}" == 1 ]]; then' \
  '    printf "tcp:192.0.2.1:5555\\n"' \
  '  else' \
  '    printf "usb:1-2.3\\n"' \
  '  fi' \
  'elif [[ "$*" == *"shell getprop ro.kernel.qemu"* ]]; then' \
  '  [[ "${FAKE_QEMU:-0}" == 1 ]] && printf "1\\n"' \
  'elif [[ "$*" == *"settings get global airplane_mode_on"* ]]; then' \
  '  if [[ "${FAKE_ONLINE:-0}" == 1 ]]; then printf "0\\n"; else printf "1\\n"; fi' \
  'elif [[ "$*" == *"settings get global wifi_on"* ]]; then' \
  '  printf "0\\n"' \
  'elif [[ "$*" == *"settings get global mobile_data"* ]]; then' \
  '  printf "0\\n"' \
  'elif [[ "$*" == *"settings get global bluetooth_on"* ]]; then' \
  '  printf "0\\n"' \
  'elif [[ "$*" == *"/system/bin/ip -6 route show default"* ]]; then' \
  '  [[ "${FAKE_ROUTE6:-0}" == 1 ]] && printf "default via fe80::1 dev wlan0\\n"' \
  'elif [[ "$*" == *"/system/bin/ip route show default"* ]]; then' \
  '  [[ "${FAKE_ROUTE4:-0}" == 1 ]] && printf "default via 192.0.2.1 dev wlan0\\n"' \
  'elif [[ "$*" == *"forward --list"* ]]; then' \
  '  [[ "${FAKE_FORWARD_LINGERS:-0}" == 1 ]] && printf "physical-serial-01 tcp:41234 tcp:8022\\n"' \
  'elif [[ "$*" == *"forward --remove"* ]]; then' \
  '  exit 0' \
  'else' \
  '  exit 64' \
  'fi' \
  'exit 0' \
  >"$ROOT/fake-bin/adb"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'set -eu' \
  'printf "Signer #1 certificate SHA-256 digest: cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\\n"' \
  >"$ROOT/fake-bin/apksigner"
chmod 700 "$ROOT/fake-bin/adb" "$ROOT/fake-bin/apksigner"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'set -eu' \
  'if [[ -n "${FAKE_SSH_LOG:-}" ]]; then printf "%s\\n" "$*" >>"$FAKE_SSH_LOG"; fi' \
  'if [[ "$*" == *"mkdir -m 700"* ]]; then exit 70; fi' \
  'exit 0' \
  >"$ROOT/fake-bin/ssh"
chmod 700 "$ROOT/fake-bin/ssh"

(
  # shellcheck source=/dev/null
  source "$CONTROLLER"
  PATH="$ROOT/fake-bin:$REAL_PATH"
  WORK_ROOT="$ROOT/probes"
  ADB_SERIAL=physical-serial-01
  [[ "$(package_version com.termux)" == 0.118.3 ]] \
    || fail_test package_version_probe_invalid
  [[ "$(package_signer_sha256 com.termux)" == "$DIGEST" ]] \
    || fail_test package_signer_probe_invalid
  validate_usb_transport || fail_test physical_usb_transport_rejected
  validate_non_emulator || fail_test physical_device_misclassified_as_emulator
  validate_offline_device_posture || fail_test offline_device_posture_rejected

  configure_transport
  [[ "$LOCAL_PORT" == 41234 && "$FORWARD_CREATED" == 1 ]] \
    || fail_test adb_forward_configuration_invalid
  printf '%s\n' "${SSH_OPTIONS[@]}" >"$ROOT/ssh-options"
  for required_option in \
    StrictHostKeyChecking=yes IdentitiesOnly=yes ClearAllForwardings=yes
  do
    grep -Fxq "$required_option" "$ROOT/ssh-options" \
      || fail_test ssh_options_not_fail_closed
  done
  remove_forward || fail_test forward_cleanup_failed
  [[ "$FORWARD_CREATED" == 0 ]] || fail_test forward_cleanup_not_recorded
)

if (
  # shellcheck source=/dev/null
  source "$CONTROLLER"
  PATH="$ROOT/fake-bin:$REAL_PATH"
  ADB_SERIAL=physical-serial-01
  FAKE_WIRELESS=1
  export FAKE_WIRELESS
  validate_usb_transport
) >"$ROOT/wireless.stdout" 2>"$ROOT/wireless.stderr"; then
  fail_test wireless_adb_transport_accepted
fi

if (
  # shellcheck source=/dev/null
  source "$CONTROLLER"
  PATH="$ROOT/fake-bin:$REAL_PATH"
  ADB_SERIAL=physical-serial-01
  FAKE_QEMU=1
  export FAKE_QEMU
  validate_non_emulator
) >"$ROOT/qemu.stdout" 2>"$ROOT/qemu.stderr"; then
  fail_test emulator_marker_accepted
fi

if (
  # shellcheck source=/dev/null
  source "$CONTROLLER"
  PATH="$ROOT/fake-bin:$REAL_PATH"
  ADB_SERIAL=physical-serial-01
  FAKE_ONLINE=1
  export FAKE_ONLINE
  validate_offline_device_posture
) >"$ROOT/online.stdout" 2>"$ROOT/online.stderr"; then
  fail_test online_device_posture_accepted
fi

if (
  # shellcheck source=/dev/null
  source "$CONTROLLER"
  PATH="$ROOT/fake-bin:$REAL_PATH"
  ADB_SERIAL=physical-serial-01
  FAKE_ROUTE4=1
  export FAKE_ROUTE4
  validate_offline_device_posture
) >"$ROOT/route4.stdout" 2>"$ROOT/route4.stderr"; then
  fail_test ipv4_default_route_accepted
fi

if (
  # shellcheck source=/dev/null
  source "$CONTROLLER"
  PATH="$ROOT/fake-bin:$REAL_PATH"
  ADB_SERIAL=physical-serial-01
  FAKE_ROUTE6=1
  export FAKE_ROUTE6
  validate_offline_device_posture
) >"$ROOT/route6.stdout" 2>"$ROOT/route6.stderr"; then
  fail_test ipv6_default_route_accepted
fi

if (
  # shellcheck source=/dev/null
  source "$CONTROLLER"
  PATH="$ROOT/fake-bin:$REAL_PATH"
  WORK_ROOT="$ROOT/probes"
  ADB_SERIAL=physical-serial-01
  FAKE_ADB_AMBIGUOUS=1
  export FAKE_ADB_AMBIGUOUS
  package_version com.termux
) >"$ROOT/ambiguous.stdout" 2>"$ROOT/ambiguous.stderr"; then
  fail_test ambiguous_package_version_accepted
fi

if (
  # shellcheck source=/dev/null
  source "$CONTROLLER"
  PATH="$ROOT/fake-bin:$REAL_PATH"
  ADB_SERIAL=physical-serial-01
  LOCAL_PORT=41234
  FORWARD_CREATED=1
  FAKE_FORWARD_LINGERS=1
  export FAKE_FORWARD_LINGERS
  remove_forward
) >"$ROOT/forward.stdout" 2>"$ROOT/forward.stderr"; then
  fail_test lingering_forward_accepted
fi

ADB_LOG="$ROOT/adb-failed-forward.log"
if (
  # shellcheck source=/dev/null
  source "$CONTROLLER"
  PATH="$ROOT/fake-bin:$REAL_PATH"
  ADB_SERIAL=physical-serial-01
  SSH_DEVICE_PORT=8022
  FAKE_ADB_LOG="$ADB_LOG"
  FAKE_FORWARD_FAIL=1
  export FAKE_ADB_LOG FAKE_FORWARD_FAIL
  trap cleanup EXIT
  configure_transport
) >"$ROOT/forward-failure.stdout" 2>"$ROOT/forward-failure.stderr"; then
  fail_test failed_forward_creation_unexpectedly_succeeded
fi
grep -Fq 'forward --remove tcp:41234' "$ADB_LOG" \
  || fail_test failed_forward_was_not_cleaned

SSH_LOG="$ROOT/ssh-failed-create.log"
if (
  # shellcheck source=/dev/null
  source "$CONTROLLER"
  PATH="$ROOT/fake-bin:$REAL_PATH"
  REMOTE_ROOT="/data/data/com.termux/files/home/.termux-mcp-rish-controller-aaaaaaaaaaaa-1001"
  SSH_TARGET=termux@127.0.0.1
  FAKE_SSH_LOG="$SSH_LOG"
  export FAKE_SSH_LOG
  trap cleanup EXIT
  create_remote_root
) >"$ROOT/remote-failure.stdout" 2>"$ROOT/remote-failure.stderr"; then
  fail_test dropped_remote_create_response_unexpectedly_succeeded
fi
grep -Fq "rm -rf -- '/data/data/com.termux/files/home/.termux-mcp-rish-controller-aaaaaaaaaaaa-1001'" \
  "$SSH_LOG" || fail_test ambiguous_remote_root_was_not_cleaned

AUTO_QUARANTINE_ROOT="$ROOT/auto-quarantine"
mkdir -m 700 "$AUTO_QUARANTINE_ROOT"
(
  # shellcheck source=/dev/null
  source "$CONTROLLER"
  CONFIG_ROOT="$AUTO_QUARANTINE_ROOT"
  DEVICE_SLOT=slot-auto
  CANDIDATE_EXECUTED=1
  cleanup
)
[[ -f "$AUTO_QUARANTINE_ROOT/.quarantine-slot-auto" \
  && ! -L "$AUTO_QUARANTINE_ROOT/.quarantine-slot-auto" \
  && "$(stat -c '%a' "$AUTO_QUARANTINE_ROOT/.quarantine-slot-auto")" == 600 \
  && "$(<"$AUTO_QUARANTINE_ROOT/.quarantine-slot-auto")" == quarantined ]] \
  || fail_test candidate_exit_did_not_quarantine_slot

SYNC_FAILURE_ROOT="$ROOT/sync-failure-quarantine"
SYNC_FAILURE_BIN="$ROOT/sync-failure-bin"
mkdir -m 700 "$SYNC_FAILURE_ROOT" "$SYNC_FAILURE_BIN"
printf '%s\n' '#!/usr/bin/env bash' 'exit 1' >"$SYNC_FAILURE_BIN/sync"
chmod 700 "$SYNC_FAILURE_BIN/sync"
if (
  # shellcheck source=/dev/null
  source "$CONTROLLER"
  PATH="$SYNC_FAILURE_BIN:$REAL_PATH"
  CONFIG_ROOT="$SYNC_FAILURE_ROOT"
  DEVICE_SLOT=slot-sync-failure
  quarantine_slot
) >"$ROOT/sync-failure.stdout" 2>"$ROOT/sync-failure.stderr"; then
  fail_test quarantine_sync_failure_unexpectedly_accepted
fi

INTERMEDIATE="$ROOT/intermediate.json"
FINAL="$ROOT/final.json"
jq -n '
  {
    validation: {
      trustedDirectRishProbePreCandidate:true,
      trustedDirectRishProbePostCandidate:true,
      scenarioResults: [
        {id:"trusted_direct_rish_probe_pre_candidate", execution:"physical", outcome:"pass"},
        {id:"candidate_mcp_status_uid_2000", execution:"physical", outcome:"pass"},
        {id:"trusted_direct_rish_probe_post_candidate", execution:"physical", outcome:"pass"},
        {id:"bounded_test_fixture_cleanup", execution:"physical", outcome:"pending"}
      ]
    },
    cleanup: {
      candidateProcessGroupStopped:true,
      portReleased:true,
      deviceFixtureStateRemoved:false,
      controllerTransportRemoved:false
    }
  }
' >"$INTERMEDIATE"
(
  # shellcheck source=/dev/null
  source "$CONTROLLER"
  OFFLINE_POSTURE_PRE_OBSERVED=1
  OFFLINE_POSTURE_POST_OBSERVED=1
  CANDIDATE_EXECUTED=1
  SLOT_QUARANTINED=1
  promote_bounded_test_fixture_cleanup "$INTERMEDIATE" "$FINAL"
)
jq -e '
  .validation.controllerOfflinePosturePreCandidate == true
  and .validation.controllerOfflinePosturePostCandidate == true
  and [.validation.scenarioResults[].id] == [
    "controller_offline_posture_pre_candidate",
    "trusted_direct_rish_probe_pre_candidate",
    "candidate_mcp_status_uid_2000",
    "trusted_direct_rish_probe_post_candidate",
    "controller_offline_posture_post_candidate",
    "bounded_test_fixture_cleanup",
    "device_slot_quarantined_after_candidate"
  ]
  and [.validation.scenarioResults[]
    | select(.id == "bounded_test_fixture_cleanup")] == [
      {id:"bounded_test_fixture_cleanup", execution:"physical", outcome:"pass"}
    ]
  and .cleanup == {
    candidateProcessGroupStopped:true,
    portReleased:true,
    deviceFixtureStateRemoved:true,
    controllerTransportRemoved:true,
    deviceSlotQuarantinedAfterCandidate:true
  }
' "$FINAL" >/dev/null || fail_test bounded_cleanup_promotion_invalid

jq '(.validation.scenarioResults[]
  | select(.id == "bounded_test_fixture_cleanup")).outcome = "pass"' \
  "$INTERMEDIATE" >"$ROOT/already-pass.json"
if (
  # shellcheck source=/dev/null
  source "$CONTROLLER"
  OFFLINE_POSTURE_PRE_OBSERVED=1
  OFFLINE_POSTURE_POST_OBSERVED=1
  CANDIDATE_EXECUTED=1
  SLOT_QUARANTINED=1
  promote_bounded_test_fixture_cleanup \
    "$ROOT/already-pass.json" "$ROOT/invalid-final.json"
) >"$ROOT/promote.stdout" 2>"$ROOT/promote.stderr"; then
  fail_test nonpending_cleanup_promoted
fi

RAW="$ROOT/private-device-raw.txt"
printf 'private raw evidence\n' >"$RAW"
RAW_SHA="$(sha256sum "$RAW" | awk '{print $1}')"
(
  # shellcheck source=/dev/null
  source "$CONTROLLER"
  PRIVATE_EVIDENCE_ROOT="$ROOT/private-evidence"
  EXPECTED_COMMIT="$COMMIT"
  WORKFLOW_RUN_ID=1001
  WORKFLOW_RUN_ATTEMPT=1
  retain_private_raw_report "$RAW" "$RAW_SHA"
)
RETAINED="$ROOT/private-evidence/$COMMIT-1001-1-device-raw.txt"
[[ -f "$RETAINED" && ! -L "$RETAINED" \
  && "$(stat -c '%a' "$RETAINED")" == 600 \
  && "$(sha256sum "$RETAINED" | awk '{print $1}')" == "$RAW_SHA" ]] \
  || fail_test private_raw_retention_invalid
if (
  # shellcheck source=/dev/null
  source "$CONTROLLER"
  PRIVATE_EVIDENCE_ROOT="$ROOT/private-evidence"
  EXPECTED_COMMIT="$COMMIT"
  WORKFLOW_RUN_ID=1001
  WORKFLOW_RUN_ATTEMPT=1
  retain_private_raw_report "$RAW" "$RAW_SHA"
) >"$ROOT/retain.stdout" 2>"$ROOT/retain.stderr"; then
  fail_test private_raw_retention_clobbered_existing_record
fi

printf 'Shizuku rish physical controller tests passed\n'
