#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077

GATE_VERSION=1
QUALIFICATION_CLASS=official_termux_native_automated_v1
EXPECTED_IMAGE=termux/termux-docker:aarch64
DEFAULT_PORT=18773
CANONICAL_OUTPUT_NAME=automated-native-deployment-v1.json
SCENARIO_FILE_NAME=automated-native-deployment-scenarios-v1.json
SCENARIO_COUNT=6

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEPLOY_SCRIPT="$REPO_ROOT/scripts/termux_deploy.sh"
SCENARIO_SOURCE="$REPO_ROOT/docs/$SCENARIO_FILE_NAME"

ARTIFACT_DIR=''
EXPECTED_COMMIT=''
EXPECTED_VERSION=''
CI_RUN_ID=''
SECURITY_RUN_ID=''
NATIVE_RUN_ID=''
OUTPUT_REPORT=''
PORT="$DEFAULT_PORT"
FIXTURE_MODE="${TERMUX_MCP_AUTOMATED_DEPLOYMENT_FIXTURE_MODE:-0}"
TEST_PAUSE_PATH="${TERMUX_MCP_AUTOMATED_DEPLOYMENT_TEST_PAUSE_AFTER_SNAPSHOT:-}"
TEST_PUBLISH_PAUSE_PATH="${TERMUX_MCP_AUTOMATED_DEPLOYMENT_TEST_PAUSE_AFTER_PUBLISH:-}"

STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
LOCK_DIR=''
LOCK_HELD=0
WORK_ROOT=''
SERVICE_SANDBOX=''
RUNSVDIR_PID=''
RUNSVDIR_IDENTITY=''
declare -a TRACKED_RUNSV_IDENTITIES=()
declare -a TRACKED_SERVICE_IDENTITIES=()
declare -a TRACKED_ASSOCIATED_IDENTITIES=()
REPORT_NEXT=''
TEST_RUNSVDIR_PATH="${TERMUX_MCP_AUTOMATED_DEPLOYMENT_TEST_RUNSVDIR:-}"
TEST_SUPERVISOR_PID_FILE="${TERMUX_MCP_AUTOMATED_DEPLOYMENT_TEST_SUPERVISOR_PID_FILE:-}"
TEST_SUPERVISOR_PAUSE_PATH="${TERMUX_MCP_AUTOMATED_DEPLOYMENT_TEST_PAUSE_AFTER_SUPERVISOR_START:-}"
PID_NAMESPACE_ID="$(readlink /proc/self/ns/pid 2>/dev/null || true)"

log() {
  printf '[termux-automated-deployment] %s\n' "$*"
}

fail() {
  printf 'TERMUX_MCP_AUTOMATED_DEPLOYMENT_RESULT=FAIL reason=%s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<EOF
Usage: termux_automated_deployment_gate.sh \\
  --artifact-dir DIR \\
  --expected-commit SHA \\
  --expected-version VERSION \\
  --ci-run-id ID \\
  --security-run-id ID \\
  --native-run-id ID \\
  --output /private/output/$CANONICAL_OUTPUT_NAME \\
  [--port PORT]

Execute the committed six-scenario deployment contract against the exact
full-suite artifact in an isolated official native ARM64 Termux rootfs.
The pass path exercises a fresh deploy, deterministic failed-upgrade recovery,
runit-supervised restart, deterministic rollback recovery, uninstall, and
bounded cleanup. Recovery fault injection is limited to readiness probes for
the deliberately faulted release target; the restored target is rechecked
with the real loopback HTTP client.

This component report is never standalone release authority. It does not
claim Android framework observation, physical hardware/device observation,
or a sustained physical soak.
EOF
}

is_true() {
  case "${1,,}" in
    1|true|yes|on) return 0 ;;
    *) return 1 ;;
  esac
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "required_command_missing_$1"
}

proc_namespace_pid() {
  local proc_id="$1"
  [[ "$proc_id" =~ ^[1-9][0-9]*$ && -r "/proc/$proc_id/status" ]] || return 1
  awk '/^NSpid:/{print $NF; found=1} END{if (!found) exit 1}' "/proc/$proc_id/status"
}

proc_in_gate_pid_namespace() {
  local proc_id="$1" namespace
  [[ -n "$PID_NAMESPACE_ID" ]] || return 1
  namespace="$(readlink "/proc/$proc_id/ns/pid" 2>/dev/null || true)"
  [[ "$namespace" == "$PID_NAMESPACE_ID" ]]
}

proc_id_for_namespace_pid() {
  local pid="$1" proc proc_id namespace_pid
  [[ "$pid" =~ ^[1-9][0-9]*$ ]] || return 1
  if [[ -d "/proc/$pid" ]] && proc_in_gate_pid_namespace "$pid"; then
    namespace_pid="$(proc_namespace_pid "$pid" 2>/dev/null || true)"
    if [[ "$namespace_pid" == "$pid" ]]; then
      printf '%s\n' "$pid"
      return 0
    fi
  fi
  for proc in /proc/[1-9]*; do
    [[ -d "$proc" ]] || continue
    proc_id="${proc##*/}"
    proc_in_gate_pid_namespace "$proc_id" || continue
    namespace_pid="$(proc_namespace_pid "$proc_id" 2>/dev/null || true)"
    if [[ "$namespace_pid" == "$pid" ]]; then
      printf '%s\n' "$proc_id"
      return 0
    fi
  done
  return 1
}

proc_start_time() {
  local proc_id="$1" stat suffix
  [[ "$proc_id" =~ ^[1-9][0-9]*$ && -r "/proc/$proc_id/stat" ]] || return 1
  IFS= read -r stat <"/proc/$proc_id/stat" || return 1
  [[ "$stat" == *") "* ]] || return 1
  suffix="${stat##*) }"
  awk '{print $20}' <<<"$suffix"
}

proc_state() {
  local proc_id="$1" stat suffix
  [[ "$proc_id" =~ ^[1-9][0-9]*$ && -r "/proc/$proc_id/stat" ]] || return 1
  IFS= read -r stat <"/proc/$proc_id/stat" || return 1
  [[ "$stat" == *") "* ]] || return 1
  suffix="${stat##*) }"
  awk '{print $1}' <<<"$suffix"
}

identity_running() {
  local identity="$1" pid proc_id expected current state namespace_pid
  IFS=: read -r pid proc_id expected <<<"$identity"
  [[ "$pid" =~ ^[1-9][0-9]*$ && "$proc_id" =~ ^[1-9][0-9]*$ ]] || return 1
  proc_in_gate_pid_namespace "$proc_id" || return 1
  namespace_pid="$(proc_namespace_pid "$proc_id" 2>/dev/null || true)"
  [[ "$namespace_pid" == "$pid" ]] || return 1
  current="$(proc_start_time "$proc_id" 2>/dev/null || true)"
  [[ -n "$current" && "$current" == "$expected" ]] || return 1
  state="$(proc_state "$proc_id" 2>/dev/null || true)"
  [[ -n "$state" && "$state" != Z && "$state" != X ]]
}

append_identity() {
  local role="$1" pid="$2" proc_id="${3:-}" start identity existing
  [[ "$pid" =~ ^[1-9][0-9]*$ && "$pid" != "$$" ]] || return 0
  if [[ -z "$proc_id" ]]; then
    proc_id="$(proc_id_for_namespace_pid "$pid" 2>/dev/null || true)"
  fi
  [[ "$proc_id" =~ ^[1-9][0-9]*$ ]] || return 0
  start="$(proc_start_time "$proc_id" 2>/dev/null || true)"
  [[ -n "$start" ]] || return 0
  identity="$pid:$proc_id:$start"
  case "$role" in
    runsv)
      for existing in "${TRACKED_RUNSV_IDENTITIES[@]}"; do
        [[ "$existing" == "$identity" ]] && return 0
      done
      TRACKED_RUNSV_IDENTITIES+=("$identity")
      ;;
    service)
      for existing in "${TRACKED_SERVICE_IDENTITIES[@]}"; do
        [[ "$existing" == "$identity" ]] && return 0
      done
      TRACKED_SERVICE_IDENTITIES+=("$identity")
      ;;
    associated)
      for existing in "${TRACKED_ASSOCIATED_IDENTITIES[@]}"; do
        [[ "$existing" == "$identity" ]] && return 0
      done
      TRACKED_ASSOCIATED_IDENTITIES+=("$identity")
      ;;
    *)
      return 1
      ;;
  esac
}

track_runsv_children() {
  local parent_proc_id children_file child_proc_id child
  [[ -n "$RUNSVDIR_IDENTITY" ]] || return 0
  IFS=: read -r _ parent_proc_id _ <<<"$RUNSVDIR_IDENTITY"
  children_file="/proc/$parent_proc_id/task/$parent_proc_id/children"
  [[ -r "$children_file" ]] || return 0
  while IFS= read -r child_proc_id; do
    child="$(proc_namespace_pid "$child_proc_id" 2>/dev/null || true)"
    if [[ -n "$child" ]]; then
      append_identity runsv "$child" "$child_proc_id"
    fi
  done < <(tr ' ' '\n' <"$children_file" | sed '/^$/d')
  return 0
}

path_is_isolated() {
  local path="${1% (deleted)}" root
  for root in "$SERVICE_SANDBOX" "$WORK_ROOT"; do
    [[ -n "$root" ]] || continue
    [[ "$path" == "$root" || "$path" == "$root/"* ]] && return 0
  done
  return 1
}

pid_is_isolated() {
  local proc_id="$1" target fd
  [[ "$proc_id" =~ ^[1-9][0-9]*$ && -d "/proc/$proc_id" ]] || return 1
  for target in \
    "$(readlink "/proc/$proc_id/cwd" 2>/dev/null || true)" \
    "$(readlink "/proc/$proc_id/exe" 2>/dev/null || true)"; do
    [[ -n "$target" ]] && path_is_isolated "$target" && return 0
  done
  if [[ -d "/proc/$proc_id/fd" ]]; then
    for fd in "/proc/$proc_id/fd/"*; do
      [[ -e "$fd" || -L "$fd" ]] || continue
      target="$(readlink "$fd" 2>/dev/null || true)"
      [[ -n "$target" ]] && path_is_isolated "$target" && return 0
    done
  fi
  return 1
}

collect_isolated_processes() {
  local proc proc_id pid current_service
  track_runsv_children
  if [[ -n "${SERVICE_DIR:-}" && -d "${SERVICE_DIR:-}" ]] && command -v sv >/dev/null 2>&1; then
    current_service="$(service_pid 2>/dev/null || true)"
    [[ "$current_service" =~ ^[1-9][0-9]*$ ]] \
      && append_identity service "$current_service"
  fi
  for proc in /proc/[1-9]*; do
    [[ -d "$proc" ]] || continue
    proc_id="${proc##*/}"
    proc_in_gate_pid_namespace "$proc_id" || continue
    pid="$(proc_namespace_pid "$proc_id" 2>/dev/null || true)"
    [[ "$pid" =~ ^[1-9][0-9]*$ && "$pid" != "$$" ]] || continue
    if pid_is_isolated "$proc_id"; then
      append_identity associated "$pid" "$proc_id"
    fi
  done
  return 0
}

signal_identity() {
  local identity="$1" signal="$2" pid
  IFS=: read -r pid _ <<<"$identity"
  identity_running "$identity" || return 0
  kill "-$signal" "$pid" >/dev/null 2>&1 || true
}

supervision_processes_running() {
  local identity
  if [[ -n "$RUNSVDIR_IDENTITY" ]] && identity_running "$RUNSVDIR_IDENTITY"; then
    return 0
  fi
  for identity in \
    "${TRACKED_RUNSV_IDENTITIES[@]}" \
    "${TRACKED_SERVICE_IDENTITIES[@]}" \
    "${TRACKED_ASSOCIATED_IDENTITIES[@]}"; do
    identity_running "$identity" && return 0
  done
  return 1
}

wait_for_supervision_exit() {
  local attempt
  for attempt in $(seq 1 50); do
    collect_isolated_processes
    if ! supervision_processes_running; then
      if [[ -n "$RUNSVDIR_PID" ]]; then
        wait "$RUNSVDIR_PID" 2>/dev/null || true
      fi
      return 0
    fi
    sleep 0.1
  done
  return 1
}

signal_tracked_processes() {
  local signal="$1" identity
  for identity in \
    "${TRACKED_SERVICE_IDENTITIES[@]}" \
    "${TRACKED_RUNSV_IDENTITIES[@]}" \
    "${TRACKED_ASSOCIATED_IDENTITIES[@]}"; do
    signal_identity "$identity" "$signal"
  done
  if [[ -n "$RUNSVDIR_IDENTITY" ]]; then
    signal_identity "$RUNSVDIR_IDENTITY" "$signal"
  fi
  return 0
}

shutdown_isolated_supervision() {
  collect_isolated_processes
  if [[ -n "$RUNSVDIR_IDENTITY" ]]; then
    if identity_running "$RUNSVDIR_IDENTITY"; then
      # runsvdir's HUP contract terminates each monitored runsv before it exits.
      kill -HUP "$RUNSVDIR_PID" >/dev/null 2>&1 || return 1
    fi
  fi
  if ! wait_for_supervision_exit; then
    signal_tracked_processes TERM
    if ! wait_for_supervision_exit; then
      signal_tracked_processes KILL
      wait_for_supervision_exit || return 1
    fi
  fi
  collect_isolated_processes
  supervision_processes_running && return 1
  if [[ -n "$RUNSVDIR_PID" ]]; then
    wait "$RUNSVDIR_PID" 2>/dev/null || true
  fi
  RUNSVDIR_PID=''
  RUNSVDIR_IDENTITY=''
  return 0
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM HUP

  if [[ -n "$SERVICE_SANDBOX" && -d "$SERVICE_SANDBOX/service/mcp_runtime" ]] \
    && command -v sv >/dev/null 2>&1; then
    timeout -k 2 5 sv down "$SERVICE_SANDBOX/service/mcp_runtime" \
      >/dev/null 2>&1 || true
  fi
  if ! shutdown_isolated_supervision >/dev/null 2>&1; then
    if ((status == 0)); then
      printf 'TERMUX_MCP_AUTOMATED_DEPLOYMENT_RESULT=FAIL reason=cleanup_unconfirmed\n' >&2
    fi
    status=1
  fi

  if [[ -n "$REPORT_NEXT" && -f "$REPORT_NEXT" ]]; then
    rm -f -- "$REPORT_NEXT" >/dev/null 2>&1 || status=1
  fi
  if [[ -n "$SERVICE_SANDBOX" && "$SERVICE_SANDBOX" == */termux-mcp-automated.* ]]; then
    rm -rf -- "$SERVICE_SANDBOX" >/dev/null 2>&1 || status=1
    SERVICE_SANDBOX=''
  fi
  if [[ -n "$WORK_ROOT" && "$WORK_ROOT" == "$HOME"/.termux-mcp-automated.* ]]; then
    rm -rf -- "$WORK_ROOT" >/dev/null 2>&1 || status=1
    WORK_ROOT=''
  fi
  if ((LOCK_HELD == 1)) && [[ -n "$LOCK_DIR" && "$LOCK_DIR" == "$HOME"/.termux-mcp-automated-deployment-gate.lock ]]; then
    rm -rf -- "$LOCK_DIR" >/dev/null 2>&1 || status=1
    LOCK_HELD=0
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

while (($#)); do
  case "$1" in
    --artifact-dir)
      (($# >= 2)) || fail missing_artifact_dir
      ARTIFACT_DIR="$2"
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
    --native-run-id)
      (($# >= 2)) || fail missing_native_run_id
      NATIVE_RUN_ID="$2"
      shift 2
      ;;
    --output)
      (($# >= 2)) || fail missing_output
      OUTPUT_REPORT="$2"
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
    *)
      fail unknown_argument
      ;;
  esac
done

[[ "$EXPECTED_COMMIT" =~ ^[0-9a-f]{40}$ ]] || fail expected_commit_invalid
[[ "$EXPECTED_VERSION" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] || fail expected_version_invalid
[[ "$CI_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail ci_run_id_invalid
[[ "$SECURITY_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail security_run_id_invalid
[[ "$NATIVE_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail native_run_id_invalid
[[ "$PORT" =~ ^[0-9]+$ ]] || fail port_invalid
((PORT >= 1024 && PORT <= 65535)) || fail port_invalid
[[ "$ARTIFACT_DIR" == /* && "$OUTPUT_REPORT" == /* ]] || fail absolute_paths_required
[[ "${OUTPUT_REPORT##*/}" == "$CANONICAL_OUTPUT_NAME" ]] || fail output_filename_invalid
case "${FIXTURE_MODE,,}" in 0|1|false|true|no|yes|off|on) ;; *) fail fixture_mode_invalid ;; esac
if [[ -n "$TEST_RUNSVDIR_PATH" || -n "$TEST_SUPERVISOR_PID_FILE" || -n "$TEST_SUPERVISOR_PAUSE_PATH" ]]; then
  is_true "$FIXTURE_MODE" || fail test_supervisor_requires_fixture_mode
  [[ "$TEST_RUNSVDIR_PATH" == /* && -f "$TEST_RUNSVDIR_PATH" \
    && ! -L "$TEST_RUNSVDIR_PATH" && -x "$TEST_RUNSVDIR_PATH" ]] \
    || fail test_runsvdir_invalid
  [[ "$TEST_SUPERVISOR_PID_FILE" == /* ]] || fail test_supervisor_pid_file_invalid
  [[ -z "$TEST_SUPERVISOR_PAUSE_PATH" || "$TEST_SUPERVISOR_PAUSE_PATH" == /* ]] \
    || fail test_supervisor_pause_path_invalid
fi
if [[ -n "$TEST_PUBLISH_PAUSE_PATH" ]]; then
  is_true "$FIXTURE_MODE" || fail test_publish_pause_requires_fixture_mode
  [[ "$TEST_PUBLISH_PAUSE_PATH" == /* ]] || fail test_publish_pause_path_invalid
fi

for command_name in \
  awk bash chmod cp curl date dirname file find grep install jq kill ln mkdir mktemp \
  mv python3 readlink realpath rm sed seq sha256sum sleep sort stat timeout tr uname uniq wc; do
  require_command "$command_name"
done
COMMIT_HELPER="$REPO_ROOT/scripts/commit_verified_file.py"
[[ -f "$COMMIT_HELPER" && ! -L "$COMMIT_HELPER" ]] || fail commit_helper_invalid
[[ -f "$DEPLOY_SCRIPT" && ! -L "$DEPLOY_SCRIPT" ]] || fail deploy_script_invalid
[[ -f "$SCENARIO_SOURCE" && ! -L "$SCENARIO_SOURCE" ]] || fail scenario_source_invalid

ARTIFACT_DIR="$(realpath -e "$ARTIFACT_DIR")" || fail artifact_dir_not_canonical
[[ -d "$ARTIFACT_DIR" && ! -L "$ARTIFACT_DIR" ]] || fail artifact_dir_invalid
OUTPUT_PARENT="$(dirname "$OUTPUT_REPORT")"
[[ -d "$OUTPUT_PARENT" && ! -L "$OUTPUT_PARENT" ]] || fail output_parent_invalid
[[ "$(realpath -e "$OUTPUT_PARENT")" == "$OUTPUT_PARENT" ]] || fail output_parent_not_canonical
[[ "$(stat -c %a "$OUTPUT_PARENT")" == 700 ]] || fail output_parent_not_private
[[ ! -e "$OUTPUT_REPORT" && ! -L "$OUTPUT_REPORT" ]] || fail output_already_exists
[[ "$OUTPUT_REPORT" != "$ARTIFACT_DIR/"* ]] || fail output_overlaps_artifact_dir

LOCK_DIR="$HOME/.termux-mcp-automated-deployment-gate.lock"
if ! mkdir -m 700 -- "$LOCK_DIR" 2>/dev/null; then
  fail gate_lock_held
fi
LOCK_HELD=1
printf '%s\n' "$$" >"$LOCK_DIR/owner.pid"
chmod 600 "$LOCK_DIR/owner.pid"

WORK_ROOT="$(mktemp -d "$HOME/.termux-mcp-automated.XXXXXX")" || fail work_root_create_failed
chmod 700 "$WORK_ROOT"
SNAPSHOT_DIR="$WORK_ROOT/input"
ARTIFACT_SNAPSHOT_DIR="$SNAPSHOT_DIR/artifact"
mkdir -m 700 "$SNAPSHOT_DIR" "$ARTIFACT_SNAPSHOT_DIR"

mapfile -t source_members < <(find "$ARTIFACT_DIR" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)
[[ "${#source_members[@]}" == 3 \
  && "${source_members[0]}" == SHA256SUMS \
  && "${source_members[1]}" == artifact-manifest.json \
  && "${source_members[2]}" == termux-mcp-server ]] || fail artifact_bundle_member_set_invalid
for member in termux-mcp-server artifact-manifest.json SHA256SUMS; do
  [[ -f "$ARTIFACT_DIR/$member" && ! -L "$ARTIFACT_DIR/$member" ]] || fail artifact_bundle_member_invalid
done
[[ -x "$ARTIFACT_DIR/termux-mcp-server" ]] || fail artifact_binary_not_executable

SOURCE_ARTIFACT_SHA="$(sha256sum "$ARTIFACT_DIR/termux-mcp-server" | awk '{print $1}')"
SOURCE_MANIFEST_SHA="$(sha256sum "$ARTIFACT_DIR/artifact-manifest.json" | awk '{print $1}')"
SOURCE_CHECKSUMS_SHA="$(sha256sum "$ARTIFACT_DIR/SHA256SUMS" | awk '{print $1}')"
SOURCE_SCENARIO_SHA="$(sha256sum "$SCENARIO_SOURCE" | awk '{print $1}')"
install -m 700 "$ARTIFACT_DIR/termux-mcp-server" "$ARTIFACT_SNAPSHOT_DIR/termux-mcp-server"
install -m 600 "$ARTIFACT_DIR/artifact-manifest.json" "$ARTIFACT_SNAPSHOT_DIR/artifact-manifest.json"
install -m 600 "$ARTIFACT_DIR/SHA256SUMS" "$ARTIFACT_SNAPSHOT_DIR/SHA256SUMS"
install -m 600 "$SCENARIO_SOURCE" "$SNAPSHOT_DIR/$SCENARIO_FILE_NAME"

if is_true "$FIXTURE_MODE" && [[ -n "$TEST_PAUSE_PATH" ]]; then
  [[ "$TEST_PAUSE_PATH" == /* ]] || fail test_pause_path_invalid
  : >"$TEST_PAUSE_PATH.ready"
  for _ in $(seq 1 100); do
    [[ -f "$TEST_PAUSE_PATH.continue" ]] && break
    sleep 0.05
  done
  [[ -f "$TEST_PAUSE_PATH.continue" ]] || fail test_pause_timeout
elif [[ -n "$TEST_PAUSE_PATH" ]]; then
  fail test_pause_requires_fixture_mode
fi

reject_duplicate_json_keys() {
  python3 - "$1" <<'PY'
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

try:
    json.loads(
        pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"),
        object_pairs_hook=closed_object,
        parse_constant=reject_constant,
    )
except (OSError, UnicodeError, json.JSONDecodeError, ValueError):
    raise SystemExit(1)
PY
}

ARTIFACT="$ARTIFACT_SNAPSHOT_DIR/termux-mcp-server"
MANIFEST="$ARTIFACT_SNAPSHOT_DIR/artifact-manifest.json"
CHECKSUMS="$ARTIFACT_SNAPSHOT_DIR/SHA256SUMS"
SCENARIO_SET="$SNAPSHOT_DIR/$SCENARIO_FILE_NAME"
reject_duplicate_json_keys "$MANIFEST" || fail artifact_manifest_duplicate_key
reject_duplicate_json_keys "$SCENARIO_SET" || fail scenario_set_duplicate_key
(cd "$ARTIFACT_SNAPSHOT_DIR" && sha256sum -c SHA256SUMS >/dev/null) || fail artifact_checksum_invalid

jq -e \
  --arg commit "$EXPECTED_COMMIT" \
  --arg version "$EXPECTED_VERSION" \
  --arg run_id "$NATIVE_RUN_ID" '
    (keys == ["artifactName","bytes","commit","createdAt","elf","features","fileName","posture","repository","schemaVersion","sha256","target","version","workflowRunId"])
    and .schemaVersion == 1
    and .repository == "CyberBASSLord-666/termux-mcp-edge"
    and .commit == $commit
    and .workflowRunId == $run_id
    and .artifactName == "termux-mcp-server-aarch64-linux-android-full-suite"
    and .posture == "full-suite"
    and .features == ["full-suite"]
    and .target == "aarch64-linux-android"
    and .fileName == "termux-mcp-server"
    and .version == $version
    and .elf == "aarch64-android-elf"
    and (.sha256 | type == "string" and test("^[0-9a-f]{64}$"))
    and (.bytes | type == "number" and . >= 1 and . <= 67108864 and floor == .)
    and (.createdAt | type == "string" and length <= 64)
  ' "$MANIFEST" >/dev/null || fail artifact_manifest_invalid

ARTIFACT_SHA="$(jq -r .sha256 "$MANIFEST")"
ARTIFACT_BYTES="$(jq -r .bytes "$MANIFEST")"
MANIFEST_SHA="$(sha256sum "$MANIFEST" | awk '{print $1}')"
[[ "$(sha256sum "$ARTIFACT" | awk '{print $1}')" == "$ARTIFACT_SHA" ]] || fail artifact_digest_mismatch
[[ "$(stat -c %s "$ARTIFACT")" == "$ARTIFACT_BYTES" ]] || fail artifact_size_mismatch
[[ "$(timeout -k 2 5 "$ARTIFACT" --version)" == "termux-mcp-server $EXPECTED_VERSION" ]] \
  || fail artifact_version_mismatch

jq -e '
  keys == ["qualificationClass","scenarioSetVersion","scenarios","schemaVersion"]
  and .schemaVersion == 1
  and .scenarioSetVersion == "1"
  and .qualificationClass == "official_termux_native_automated_v1"
  and .scenarios == [
    {
      id: "isolated_fresh_deploy",
      execution: "native",
      faultInjection: "none",
      expectedOutcome: "pass"
    },
    {
      id: "failed_upgrade_recovery",
      execution: "native",
      faultInjection: "target_scoped_readiness_probe_failure",
      expectedOutcome: "recovered"
    },
    {
      id: "supervised_restart",
      execution: "native",
      faultInjection: "supervised_process_termination",
      expectedOutcome: "restarted"
    },
    {
      id: "rollback_recovery",
      execution: "native",
      faultInjection: "target_scoped_readiness_probe_failure",
      expectedOutcome: "recovered"
    },
    {
      id: "uninstall",
      execution: "native",
      faultInjection: "none",
      expectedOutcome: "removed"
    },
    {
      id: "bounded_cleanup",
      execution: "native",
      faultInjection: "none",
      expectedOutcome: "clean"
    }
  ]
' "$SCENARIO_SET" >/dev/null || fail scenario_set_invalid
SCENARIO_SHA="$(sha256sum "$SCENARIO_SET" | awk '{print $1}')"

if is_true "$FIXTURE_MODE"; then
  EXECUTION_STATUS=fixture
  EXECUTION_KIND=fixture
  EXECUTION_MODE=fixture-host-test
  ROOTFS_IMAGE_JSON=null
  ROOTFS_DIGEST_JSON=null
  ROOTFS_IMAGE_ID_JSON=null
  RUNTIME_IMAGE_DIGEST_JSON=null
  LINKER_OBSERVED=false
  LINKER_PATH_JSON=null
  LINKER_SHA_JSON=null
  LINKER_BYTES_JSON=null
  RUNIT_OBSERVED=false
  NATIVE_OBSERVED=false
  DEPLOY_PREFIX="$WORK_ROOT/prefix"
  REPORT_TERMUX_PREFIX=fixture-prefix-not-observed
  mkdir -p "$DEPLOY_PREFIX/bin"
  cp -L -- /bin/sh "$DEPLOY_PREFIX/bin/sh"
  chmod 700 "$DEPLOY_PREFIX/bin/sh"
else
  [[ "${TERMUX_MCP_AUTOMATED_DEPLOYMENT_ENVIRONMENT:-}" == official-termux-docker-native-arm64 ]] \
    || fail environment_attestation_missing
  ROOTFS_DIGEST="${TERMUX_MCP_TERMUX_IMAGE_DIGEST:-}"
  [[ "$ROOTFS_DIGEST" =~ ^sha256:[0-9a-f]{64}$ ]] || fail rootfs_digest_invalid
  ROOTFS_IMAGE_ID="${TERMUX_MCP_TERMUX_ROOTFS_IMAGE_ID:-}"
  [[ "$ROOTFS_IMAGE_ID" =~ ^sha256:[0-9a-f]{64}$ ]] || fail rootfs_image_id_invalid
  RUNTIME_IMAGE_DIGEST="${TERMUX_MCP_TERMUX_RUNTIME_IMAGE_DIGEST:-}"
  [[ "$RUNTIME_IMAGE_DIGEST" =~ ^sha256:[0-9a-f]{64}$ ]] \
    || fail runtime_image_digest_invalid
  [[ "$RUNTIME_IMAGE_DIGEST" != "$ROOTFS_IMAGE_ID" ]] \
    || fail runtime_image_digest_not_derived
  [[ "$(uname -m)" == aarch64 ]] || fail architecture_not_aarch64
  [[ "${PREFIX:-}" == /data/data/com.termux/files/usr ]] || fail termux_prefix_invalid
  [[ "${HOME:-}" == /data/data/com.termux/files/home ]] || fail termux_home_invalid
  [[ -x /system/bin/linker64 ]] || fail android_linker_invalid
  for command_name in runsvdir sv; do
    require_command "$command_name"
  done
  identity="$(file -b "$ARTIFACT")" || fail artifact_identity_failed
  [[ "$identity" == *ELF* && "$identity" == *"ARM aarch64"* ]] || fail artifact_architecture_mismatch
  [[ "$identity" == *Android* || "$identity" == *"/system/bin/linker64"* ]] \
    || fail artifact_android_identity_missing
  EXECUTION_STATUS=pass
  EXECUTION_KIND=native
  EXECUTION_MODE=official-termux-docker-native-arm64
  ROOTFS_IMAGE_JSON="\"$EXPECTED_IMAGE\""
  ROOTFS_DIGEST_JSON="\"$ROOTFS_DIGEST\""
  ROOTFS_IMAGE_ID_JSON="\"$ROOTFS_IMAGE_ID\""
  RUNTIME_IMAGE_DIGEST_JSON="\"$RUNTIME_IMAGE_DIGEST\""
  LINKER_OBSERVED=true
  LINKER_PATH_JSON='"/system/bin/linker64"'
  LINKER_SHA="$(sha256sum /system/bin/linker64 | awk '{print $1}')"
  LINKER_BYTES="$(stat -c %s /system/bin/linker64)"
  [[ "$LINKER_SHA" =~ ^[0-9a-f]{64}$ && "$LINKER_BYTES" =~ ^[1-9][0-9]*$ ]] \
    || fail android_linker_identity_invalid
  LINKER_SHA_JSON="\"$LINKER_SHA\""
  LINKER_BYTES_JSON="$LINKER_BYTES"
  RUNIT_OBSERVED=true
  NATIVE_OBSERVED=true
  DEPLOY_PREFIX="$PREFIX"
  REPORT_TERMUX_PREFIX="$PREFIX"
fi

DEPLOY_HOME="$WORK_ROOT/deploy-home"
DEPLOY_ROOT="$DEPLOY_HOME/.local/share/termux-mcp-edge"
CONFIG_ROOT="$DEPLOY_HOME/.config/termux-mcp-edge"
SAFE_ROOT="$DEPLOY_HOME/safe-root"
RUN_TEMPLATE="$WORK_ROOT/service-run-template"
mkdir -m 700 "$DEPLOY_HOME" "$SAFE_ROOT"

mkdir -p "$DEPLOY_PREFIX/var/tmp"
SERVICE_SANDBOX="$(mktemp -d "$DEPLOY_PREFIX/var/tmp/termux-mcp-automated.XXXXXX")" \
  || fail service_sandbox_create_failed
chmod 700 "$SERVICE_SANDBOX"
SERVICE_ROOT="$SERVICE_SANDBOX/service"
SERVICE_DIR="$SERVICE_ROOT/mcp_runtime"
mkdir -m 700 "$SERVICE_ROOT"

REAL_CURL="$(command -v curl)"
FAULT_BIN="$WORK_ROOT/fault-bin"
mkdir -m 700 "$FAULT_BIN"
cat >"$FAULT_BIN/curl" <<EOF
#!$DEPLOY_PREFIX/bin/sh
set -eu
current=\$(readlink "\${TERMUX_MCP_GATE_DEPLOY_ROOT}/current" 2>/dev/null || true)
if [ "\$current" = "\${TERMUX_MCP_GATE_FAULT_TARGET}" ]; then
  exit 22
fi
exec "$REAL_CURL" "\$@"
EOF
chmod 700 "$FAULT_BIN/curl"

write_runtime_config() {
  mkdir -p "$CONFIG_ROOT"
  chmod 700 "$CONFIG_ROOT"
  cat >"$CONFIG_ROOT/runtime.env" <<EOF
MCP__AUTH__STATIC_TOKEN=automated-native-deployment-gate-token
MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=false
MCP__SERVER__HOST=127.0.0.1
MCP__SERVER__PORT=$PORT
MCP__TRANSPORT__ALLOWED_HOSTS=localhost:$PORT,127.0.0.1:$PORT
MCP__TRANSPORT__ALLOWED_ORIGINS=http://localhost:$PORT,http://127.0.0.1:$PORT
MCP__TRANSPORT__ALLOW_MISSING_ORIGIN=false
MCP__TRANSPORT__SSE_ENABLED=false
MCP__FILE__SAFE_ROOTS=$SAFE_ROOT
MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=false
MCP__FILE__COPY_FILE_MUTATION_ENABLED=false
MCP__FILE__TRASH_FILE_MUTATION_ENABLED=false
MCP__FILE__WRITE_MUTATION_ENABLED=false
MCP__ANDROID__BATTERY_STATUS_ENABLED=false
MCP__ANDROID__VOLUME_STATUS_ENABLED=false
MCP__ANDROID__VOLUME_CONTROL_ENABLED=false
MCP__COMMAND__ENABLED=false
RUST_LOG=termux_mcp_server=info
EOF
  chmod 600 "$CONFIG_ROOT/runtime.env"
}

DEPLOY_ENV=(
  "HOME=$DEPLOY_HOME"
  "PREFIX=$DEPLOY_PREFIX"
  "TERMUX_MCP_DEPLOY_ROOT=$DEPLOY_ROOT"
  "TERMUX_MCP_CONFIG_ROOT=$CONFIG_ROOT"
  "TERMUX_MCP_SERVICE_ROOT=$SERVICE_ROOT"
  "TERMUX_MCP_SERVICE_SHELL=$DEPLOY_PREFIX/bin/sh"
  "TERMUX_MCP_HEALTH_URL=http://127.0.0.1:$PORT/health"
  "TERMUX_MCP_READY_URL=http://127.0.0.1:$PORT/ready"
  "TERMUX_MCP_PROBE_ATTEMPTS=15"
  "TERMUX_MCP_PROBE_DELAY_SECONDS=1"
  "TERMUX_MCP_STOP_ATTEMPTS=15"
  "TERMUX_MCP_STOP_DELAY_SECONDS=1"
  "TERMUX_MCP_START_ATTEMPTS=15"
  "TERMUX_MCP_START_DELAY_SECONDS=1"
)
if is_true "$FIXTURE_MODE"; then
  DEPLOY_ENV+=(
    "TERMUX_MCP_TEST_MODE=1"
    "TERMUX_MCP_TEST_PROBE_SEQUENCE=success"
    "TERMUX_MCP_TEST_STOP_SEQUENCE=success"
    "TERMUX_MCP_TEST_START_SEQUENCE=success"
  )
else
  DEPLOY_ENV+=("TERMUX_MCP_TEST_MODE=0")
fi

run_deploy() {
  env "${DEPLOY_ENV[@]}" bash "$DEPLOY_SCRIPT" "$@"
}

run_faulted_deploy() {
  local fault_target="$1"
  shift
  if is_true "$FIXTURE_MODE"; then
    env "${DEPLOY_ENV[@]}" \
      TERMUX_MCP_TEST_PROBE_SEQUENCE=failure,success \
      bash "$DEPLOY_SCRIPT" "$@"
  else
    env "${DEPLOY_ENV[@]}" \
      "PATH=$FAULT_BIN:$PATH" \
      "TERMUX_MCP_GATE_DEPLOY_ROOT=$DEPLOY_ROOT" \
      "TERMUX_MCP_GATE_FAULT_TARGET=$fault_target" \
      bash "$DEPLOY_SCRIPT" "$@"
  fi
}

real_runtime_ready() {
  local health ready
  is_true "$FIXTURE_MODE" && return 0
  health="$("$REAL_CURL" --disable --proto '=http' --noproxy '*' \
    --connect-timeout 2 --max-time 3 --fail --silent \
    "http://127.0.0.1:$PORT/health" 2>/dev/null || true)"
  [[ "$health" == ok ]] || return 1
  ready="$("$REAL_CURL" --disable --proto '=http' --noproxy '*' \
    --connect-timeout 2 --max-time 3 --fail --silent \
    "http://127.0.0.1:$PORT/ready" 2>/dev/null || true)"
  jq -e --arg version "$EXPECTED_VERSION" \
    '.status == "ready" and .version == $version' <<<"$ready" >/dev/null
}

wait_runtime_ready() {
  local attempt
  for attempt in $(seq 1 50); do
    if real_runtime_ready; then
      capture_current_service_pid
      return 0
    fi
    sleep 0.1
  done
  return 1
}

wait_service_registered() {
  local attempt
  is_true "$FIXTURE_MODE" && return 0
  for attempt in $(seq 1 100); do
    [[ -p "$SERVICE_DIR/supervise/ok" ]] && return 0
    sleep 0.05
  done
  return 1
}

service_pid() {
  local output
  output="$(sv status "$SERVICE_DIR" 2>/dev/null || true)"
  sed -n 's/.*(pid \([1-9][0-9]*\)).*/\1/p' <<<"$output"
}

capture_current_service_pid() {
  local pid
  is_true "$FIXTURE_MODE" && [[ -z "$TEST_RUNSVDIR_PATH" ]] && return 0
  pid="$(service_pid 2>/dev/null || true)"
  [[ "$pid" =~ ^[1-9][0-9]*$ ]] && append_identity service "$pid"
  collect_isolated_processes
}

verify_current_target() {
  local expected="$1" current
  [[ -L "$DEPLOY_ROOT/current" ]] || return 1
  current="$(readlink "$DEPLOY_ROOT/current")"
  [[ "$current" == "$expected" ]]
}

seed_predecessor() {
  local predecessor="$DEPLOY_ROOT/releases/predecessor"
  write_runtime_config
  mkdir -p "$DEPLOY_ROOT/releases"
  chmod 700 "$DEPLOY_ROOT" "$DEPLOY_ROOT/releases"
  mkdir -m 700 "$predecessor"
  install -m 700 "$ARTIFACT" "$predecessor/termux-mcp-server"
  printf '%s\n' "$EXPECTED_VERSION" >"$predecessor/VERSION"
  chmod 600 "$predecessor/VERSION"
  ln -s "$predecessor" "$DEPLOY_ROOT/current"

  local service_stage="$SERVICE_SANDBOX/.seed-service"
  rm -rf -- "$service_stage"
  mkdir -m 700 "$service_stage"
  install -m 700 "$RUN_TEMPLATE" "$service_stage/run"
  mv -T "$service_stage" "$SERVICE_DIR"
  if ! is_true "$FIXTURE_MODE"; then
    wait_service_registered || fail predecessor_service_registration_timeout
    sv up "$SERVICE_DIR" >/dev/null || fail predecessor_service_start_failed
    wait_runtime_ready || fail predecessor_runtime_not_ready
  fi
}

write_runtime_config
if ! is_true "$FIXTURE_MODE"; then
  runsvdir -P "$SERVICE_ROOT" >"$WORK_ROOT/runsvdir.log" 2>&1 &
  RUNSVDIR_PID=$!
elif [[ -n "$TEST_RUNSVDIR_PATH" ]]; then
  "$TEST_RUNSVDIR_PATH" "$SERVICE_ROOT" "$WORK_ROOT" "$TEST_SUPERVISOR_PID_FILE" \
    >"$WORK_ROOT/runsvdir.log" 2>&1 &
  RUNSVDIR_PID=$!
fi
if [[ -n "$RUNSVDIR_PID" ]]; then
  RUNSVDIR_PROC_ID="$(proc_id_for_namespace_pid "$RUNSVDIR_PID" 2>/dev/null || true)"
  RUNSVDIR_START="$(proc_start_time "$RUNSVDIR_PROC_ID" 2>/dev/null || true)"
  [[ -n "$RUNSVDIR_PROC_ID" && -n "$RUNSVDIR_START" ]] || fail runsvdir_identity_failed
  RUNSVDIR_IDENTITY="$RUNSVDIR_PID:$RUNSVDIR_PROC_ID:$RUNSVDIR_START"
  kill -0 "$RUNSVDIR_PID" >/dev/null 2>&1 || fail runsvdir_start_failed
  if [[ -n "$TEST_SUPERVISOR_PID_FILE" ]]; then
    for _ in $(seq 1 500); do
      if [[ -f "$TEST_SUPERVISOR_PID_FILE" \
        && "$(wc -l <"$TEST_SUPERVISOR_PID_FILE")" == 3 ]]; then
        break
      fi
      kill -0 "$RUNSVDIR_PID" >/dev/null 2>&1 || break
      sleep 0.02
    done
    [[ -f "$TEST_SUPERVISOR_PID_FILE" \
      && "$(wc -l <"$TEST_SUPERVISOR_PID_FILE")" == 3 ]] \
      || fail test_supervisor_identity_timeout
  fi
  for _ in $(seq 1 100); do
    track_runsv_children
    [[ "${#TRACKED_RUNSV_IDENTITIES[@]}" -ge 1 || -z "$TEST_RUNSVDIR_PATH" ]] && break
    kill -0 "$RUNSVDIR_PID" >/dev/null 2>&1 || break
    sleep 0.02
  done
  collect_isolated_processes
fi
if [[ -n "$TEST_SUPERVISOR_PAUSE_PATH" ]]; then
  : >"$TEST_SUPERVISOR_PAUSE_PATH.ready"
  for _ in $(seq 1 600); do
    [[ -f "$TEST_SUPERVISOR_PAUSE_PATH.continue" ]] && break
    sleep 0.05
  done
  [[ -f "$TEST_SUPERVISOR_PAUSE_PATH.continue" ]] || fail test_supervisor_pause_timeout
fi

log 'scenario 1/6: isolated fresh deploy'
run_deploy install \
  --artifact "$ARTIFACT" \
  --version "$EXPECTED_VERSION" \
  --sha256 "$ARTIFACT_SHA" \
  >"$WORK_ROOT/fresh-install.log" 2>&1 || fail isolated_fresh_deploy_failed
verify_current_target "$DEPLOY_ROOT/releases/$EXPECTED_VERSION" || fail fresh_deploy_current_invalid
[[ "$(sha256sum "$DEPLOY_ROOT/current/termux-mcp-server" | awk '{print $1}')" == "$ARTIFACT_SHA" ]] \
  || fail fresh_deploy_artifact_mismatch
wait_runtime_ready || fail fresh_deploy_runtime_not_ready
install -m 700 "$SERVICE_DIR/run" "$RUN_TEMPLATE"

run_deploy uninstall --purge-config >"$WORK_ROOT/fresh-reset.log" 2>&1 \
  || fail fresh_deploy_reset_failed
[[ ! -e "$DEPLOY_ROOT" && ! -e "$SERVICE_DIR" && ! -e "$CONFIG_ROOT" ]] \
  || fail fresh_deploy_reset_incomplete

log 'scenario 2/6: failed-upgrade recovery'
seed_predecessor
PREDECESSOR="$DEPLOY_ROOT/releases/predecessor"
if run_faulted_deploy "$DEPLOY_ROOT/releases/$EXPECTED_VERSION" \
  upgrade \
  --artifact "$ARTIFACT" \
  --version "$EXPECTED_VERSION" \
  --sha256 "$ARTIFACT_SHA" \
  >"$WORK_ROOT/failed-upgrade.log" 2>&1; then
  fail faulted_upgrade_unexpectedly_succeeded
fi
verify_current_target "$PREDECESSOR" || fail failed_upgrade_current_not_restored
[[ ! -e "$DEPLOY_ROOT/releases/$EXPECTED_VERSION" && ! -e "$DEPLOY_ROOT/previous" ]] \
  || fail failed_upgrade_state_not_restored
wait_runtime_ready || fail failed_upgrade_recovered_runtime_not_ready

log 'scenario 3/6: runit-supervised restart'
if ! is_true "$FIXTURE_MODE"; then
  OLD_PID="$(service_pid)"
  [[ "$OLD_PID" =~ ^[1-9][0-9]*$ ]] || fail supervised_restart_initial_pid_invalid
  append_identity service "$OLD_PID"
  kill -TERM "$OLD_PID" || fail supervised_restart_termination_failed
  NEW_PID=''
  for _ in $(seq 1 100); do
    NEW_PID="$(service_pid)"
    if [[ "$NEW_PID" =~ ^[1-9][0-9]*$ && "$NEW_PID" != "$OLD_PID" ]] \
      && kill -0 "$NEW_PID" >/dev/null 2>&1; then
      break
    fi
    sleep 0.1
  done
  [[ "$NEW_PID" =~ ^[1-9][0-9]*$ && "$NEW_PID" != "$OLD_PID" ]] \
    || fail supervised_restart_new_pid_invalid
  append_identity service "$NEW_PID"
  wait_runtime_ready || fail supervised_restart_runtime_not_ready
else
  [[ -x "$SERVICE_DIR/run" && ! -e "$SERVICE_DIR/down" ]] \
    || fail fixture_service_definition_invalid
fi

log 'scenario 4/6: rollback recovery'
run_deploy upgrade \
  --artifact "$ARTIFACT" \
  --version "$EXPECTED_VERSION" \
  --sha256 "$ARTIFACT_SHA" \
  >"$WORK_ROOT/upgrade-precondition.log" 2>&1 || fail rollback_precondition_upgrade_failed
verify_current_target "$DEPLOY_ROOT/releases/$EXPECTED_VERSION" \
  || fail rollback_precondition_current_invalid
[[ "$(readlink "$DEPLOY_ROOT/previous")" == "$PREDECESSOR" ]] \
  || fail rollback_precondition_previous_invalid
wait_runtime_ready || fail rollback_precondition_runtime_not_ready

if run_faulted_deploy "$PREDECESSOR" rollback \
  >"$WORK_ROOT/failed-rollback.log" 2>&1; then
  fail faulted_rollback_unexpectedly_succeeded
fi
verify_current_target "$DEPLOY_ROOT/releases/$EXPECTED_VERSION" \
  || fail rollback_recovery_current_not_restored
[[ "$(readlink "$DEPLOY_ROOT/previous")" == "$PREDECESSOR" ]] \
  || fail rollback_recovery_previous_not_restored
wait_runtime_ready || fail rollback_recovered_runtime_not_ready

log 'scenario 5/6: uninstall'
run_deploy uninstall --purge-config >"$WORK_ROOT/uninstall.log" 2>&1 \
  || fail uninstall_failed
[[ ! -e "$DEPLOY_ROOT" && ! -e "$SERVICE_DIR" && ! -e "$CONFIG_ROOT" ]] \
  || fail uninstall_state_retained

log 'scenario 6/6: bounded cleanup'
shutdown_isolated_supervision || fail runsvdir_cleanup_unbounded
rm -rf -- "$SERVICE_SANDBOX"
[[ ! -e "$SERVICE_SANDBOX" ]] || fail service_sandbox_cleanup_failed
SERVICE_SANDBOX=''

[[ "$(sha256sum "$ARTIFACT_DIR/termux-mcp-server" | awk '{print $1}')" == "$SOURCE_ARTIFACT_SHA" ]] \
  || fail artifact_source_changed
[[ "$(sha256sum "$ARTIFACT_DIR/artifact-manifest.json" | awk '{print $1}')" == "$SOURCE_MANIFEST_SHA" ]] \
  || fail manifest_source_changed
[[ "$(sha256sum "$ARTIFACT_DIR/SHA256SUMS" | awk '{print $1}')" == "$SOURCE_CHECKSUMS_SHA" ]] \
  || fail checksums_source_changed
[[ "$(sha256sum "$SCENARIO_SOURCE" | awk '{print $1}')" == "$SOURCE_SCENARIO_SHA" ]] \
  || fail scenario_source_changed

rm -rf -- "$WORK_ROOT"
[[ ! -e "$WORK_ROOT" ]] || fail work_root_cleanup_failed
WORK_ROOT=''
rm -rf -- "$LOCK_DIR"
[[ ! -e "$LOCK_DIR" ]] || fail gate_lock_cleanup_failed
LOCK_HELD=0

COMPLETED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
REPORT_NEXT="$(mktemp "$OUTPUT_PARENT/.automated-native-deployment.XXXXXX")" \
  || fail report_staging_create_failed
chmod 600 "$REPORT_NEXT"

if is_true "$NATIVE_OBSERVED"; then
  NATIVE_VALIDATION_JSON=true
else
  NATIVE_VALIDATION_JSON=false
fi

jq -n \
  --arg gate_version "$GATE_VERSION" \
  --arg status "$EXECUTION_STATUS" \
  --arg qualification_class "$QUALIFICATION_CLASS" \
  --arg started_at "$STARTED_AT" \
  --arg completed_at "$COMPLETED_AT" \
  --arg commit "$EXPECTED_COMMIT" \
  --arg version "$EXPECTED_VERSION" \
  --arg ci_run_id "$CI_RUN_ID" \
  --arg security_run_id "$SECURITY_RUN_ID" \
  --arg native_run_id "$NATIVE_RUN_ID" \
  --arg artifact_sha "$ARTIFACT_SHA" \
  --arg manifest_sha "$MANIFEST_SHA" \
  --argjson artifact_bytes "$ARTIFACT_BYTES" \
  --arg scenario_sha "$SCENARIO_SHA" \
  --argjson scenario_count "$SCENARIO_COUNT" \
  --arg architecture "$(uname -m)" \
  --arg execution_mode "$EXECUTION_MODE" \
  --argjson rootfs_image "$ROOTFS_IMAGE_JSON" \
  --argjson rootfs_digest "$ROOTFS_DIGEST_JSON" \
  --argjson rootfs_image_id "$ROOTFS_IMAGE_ID_JSON" \
  --argjson runtime_image_digest "$RUNTIME_IMAGE_DIGEST_JSON" \
  --arg termux_prefix "$REPORT_TERMUX_PREFIX" \
  --argjson linker_observed "$LINKER_OBSERVED" \
  --argjson linker_path "$LINKER_PATH_JSON" \
  --argjson linker_sha "$LINKER_SHA_JSON" \
  --argjson linker_bytes "$LINKER_BYTES_JSON" \
  --argjson runit_observed "$RUNIT_OBSERVED" \
  --arg execution_kind "$EXECUTION_KIND" \
  --argjson native_validation "$NATIVE_VALIDATION_JSON" '
  {
    schemaVersion: 1,
    gateVersion: $gate_version,
    status: $status,
    failureCode: null,
    releaseQualificationEligible: false,
    qualificationClass: $qualification_class,
    startedAt: $started_at,
    completedAt: $completed_at,
    candidate: {
      repository: "CyberBASSLord-666/termux-mcp-edge",
      commit: $commit,
      version: $version,
      ciRunId: $ci_run_id,
      securityRunId: $security_run_id,
      nativeRunId: $native_run_id,
      artifact: {
        artifactName: "termux-mcp-server-aarch64-linux-android-full-suite",
        posture: "full-suite",
        features: ["full-suite"],
        sha256: $artifact_sha,
        manifestSha256: $manifest_sha,
        bytes: $artifact_bytes,
        target: "aarch64-linux-android",
        elf: "aarch64-android-elf"
      }
    },
    scenarioSet: {
      fileName: "automated-native-deployment-scenarios-v1.json",
      schemaVersion: 1,
      scenarioSetVersion: "1",
      sha256: $scenario_sha,
      scenarioCount: $scenario_count,
      scenarioIds: [
        "isolated_fresh_deploy",
        "failed_upgrade_recovery",
        "supervised_restart",
        "rollback_recovery",
        "uninstall",
        "bounded_cleanup"
      ]
    },
    environment: {
      architecture: $architecture,
      executionMode: $execution_mode,
      rootfsImage: $rootfs_image,
      rootfsDigest: $rootfs_digest,
      rootfsImageId: $rootfs_image_id,
      runtimeImageDigest: $runtime_image_digest,
      termuxPrefix: $termux_prefix,
      androidLinker: {
        observed: $linker_observed,
        path: $linker_path,
        sha256: $linker_sha,
        bytes: $linker_bytes
      },
      supervisor: "runit",
      runitSupervisorObserved: $runit_observed,
      androidFrameworkObserved: false,
      physicalHardwareObserved: false,
      physicalDeviceObserved: false,
      sustainedPhysicalSoak: false
    },
    validation: {
      status: $status,
      scenarioResults: [
        {
          id: "isolated_fresh_deploy",
          execution: $execution_kind,
          outcome: "pass",
          faultBoundary: "none"
        },
        {
          id: "failed_upgrade_recovery",
          execution: $execution_kind,
          outcome: "recovered",
          faultBoundary: "target_readiness_probe"
        },
        {
          id: "supervised_restart",
          execution: $execution_kind,
          outcome: "restarted",
          faultBoundary: "supervised_process"
        },
        {
          id: "rollback_recovery",
          execution: $execution_kind,
          outcome: "recovered",
          faultBoundary: "target_readiness_probe"
        },
        {
          id: "uninstall",
          execution: $execution_kind,
          outcome: "removed",
          faultBoundary: "none"
        },
        {
          id: "bounded_cleanup",
          execution: $execution_kind,
          outcome: "clean",
          faultBoundary: "none"
        }
      ],
      artifactManifestStrict: true,
      scenarioSetStrict: true,
      nativeArtifactExecuted: $native_validation,
      isolatedFreshDeploy: $native_validation,
      failedUpgradeRecovery: $native_validation,
      supervisedRestart: $native_validation,
      rollbackRecovery: $native_validation,
      uninstall: $native_validation,
      boundedCleanup: $native_validation,
      exactArtifact: true,
      isolatedServiceRoot: true,
      runitSupervisorObserved: $runit_observed,
      realLoopbackProbes: $native_validation,
      probeFaultInjectionBounded: true,
      outputNoClobber: true,
      workspaceRemoved: true,
      serviceRemoved: true,
      runsvdirTerminated: true,
      physicalCertification: "not_run"
    }
  }' >"$REPORT_NEXT" || fail report_generation_failed

jq -e \
  --arg status "$EXECUTION_STATUS" \
  --arg execution "$EXECUTION_KIND" \
  --arg scenario_sha "$SCENARIO_SHA" \
  --argjson native "$NATIVE_VALIDATION_JSON" '
    (keys == ["candidate","completedAt","environment","failureCode","gateVersion","qualificationClass","releaseQualificationEligible","scenarioSet","schemaVersion","startedAt","status","validation"])
    and .schemaVersion == 1
    and .gateVersion == "1"
    and .status == $status
    and .failureCode == null
    and .releaseQualificationEligible == false
    and .qualificationClass == "official_termux_native_automated_v1"
    and .scenarioSet.sha256 == $scenario_sha
    and .scenarioSet.scenarioCount == 6
    and ([.validation.scenarioResults[].id] == .scenarioSet.scenarioIds)
    and ([.validation.scenarioResults[].execution] | all(. == $execution))
    and .validation.nativeArtifactExecuted == $native
    and .validation.runitSupervisorObserved == $native
    and .validation.realLoopbackProbes == $native
    and (if $native then
      (.environment.rootfsDigest | test("^sha256:[0-9a-f]{64}$"))
      and (.environment.rootfsImageId | test("^sha256:[0-9a-f]{64}$"))
      and (.environment.runtimeImageDigest | test("^sha256:[0-9a-f]{64}$"))
      and .environment.runtimeImageDigest != .environment.rootfsImageId
    else
      .environment.rootfsDigest == null
      and .environment.rootfsImageId == null
      and .environment.runtimeImageDigest == null
    end)
    and .environment.androidFrameworkObserved == false
    and .environment.physicalHardwareObserved == false
    and .environment.physicalDeviceObserved == false
    and .environment.sustainedPhysicalSoak == false
    and .validation.physicalCertification == "not_run"
  ' "$REPORT_NEXT" >/dev/null || fail generated_report_invalid

if grep -Eq 'automated-native-deployment-gate-token|MCP__AUTH__|safe-root|deploy-home' "$REPORT_NEXT"; then
  fail report_contains_private_runtime_value
fi

REPORT_SHA="$(sha256sum -- "$REPORT_NEXT" | awk '{print $1}')" \
  || fail report_publication_failed
if ! python3 "$COMMIT_HELPER" \
  --source "$REPORT_NEXT" \
  --destination "$OUTPUT_REPORT" \
  --sha256 "$REPORT_SHA" \
  --mode 600
then
  fail report_publication_race
fi
if is_true "$FIXTURE_MODE" && [[ -n "$TEST_PUBLISH_PAUSE_PATH" ]]; then
  : >"$TEST_PUBLISH_PAUSE_PATH.ready"
  for _ in $(seq 1 100); do
    [[ -f "$TEST_PUBLISH_PAUSE_PATH.continue" ]] && break
    sleep 0.05
  done
  [[ -f "$TEST_PUBLISH_PAUSE_PATH.continue" ]] || fail test_publish_pause_timeout
fi
rm -f -- "$REPORT_NEXT" >/dev/null 2>&1 || true
REPORT_NEXT=''
log "report_sha256=$REPORT_SHA"
log "report=$OUTPUT_REPORT"
printf 'TERMUX_MCP_AUTOMATED_DEPLOYMENT_RESULT=%s\n' "${EXECUTION_STATUS^^}"
