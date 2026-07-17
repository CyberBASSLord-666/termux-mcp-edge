#!/data/data/com.termux/files/usr/bin/bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077
set +x

readonly VALIDATOR_VERSION="2"
readonly EVIDENCE_SCHEMA_VERSION=1
readonly MIN_SUSTAINED_MINUTES=60
readonly MAX_ARTIFACT_BYTES=67108864
readonly MCP_PROTOCOL_VERSION="2025-11-25"
readonly MAX_LIST_RESPONSE_BYTES=262144
readonly PRODUCTION_CONFIRMATION_PREFIX="termux-mcp-edge-production"

usage() {
  cat <<'EOF'
Usage:
  termux_release_validate.sh --config FILE --report FILE [options]

Options:
  --phase preflight|runtime|deployment|all
      preflight   Verify metadata and both downloaded artifacts without starting a listener.
      runtime     Run preflight, then explicitly confirmed isolated runtime checks.
      deployment  Run preflight, then an explicitly confirmed deployment exercise.
      all         Run preflight, isolated runtime checks, and deployment checks.
  --confirm-runtime-mutation
      Permit creation of a dedicated temporary directory below the configured safe root,
      direct candidate process startup, and bounded isolated filesystem mutations,
      including one request-granted directory creation, inside that directory.
  --confirm-deployment-mutation
      Permit termux_deploy.sh to exercise install/upgrade/recovery/rollback/uninstall.
  --production-action install|upgrade|upgrade-failure|rollback|uninstall
      Replace the dedicated deployment cycle with one action against canonical roots.
  --confirm-production-roots VALUE
      Required with --production-action. VALUE must equal
      termux-mcp-edge-production-<action>.
  -h, --help

The validator never downloads artifacts or installs packages. The configuration file
must be a private regular mode-0600 file and is parsed as literal allowlisted NAME=value
records; it is never sourced or evaluated.
EOF
}

CONFIG_FILE=""
REPORT_FILE=""
PHASE="preflight"
CONFIRM_RUNTIME=0
CONFIRM_DEPLOYMENT=0
PRODUCTION_ACTION=""
PRODUCTION_CONFIRMATION=""

while (($# > 0)); do
  case "$1" in
    --config)
      (($# >= 2)) || { usage >&2; exit 2; }
      CONFIG_FILE="$2"
      shift 2
      ;;
    --report)
      (($# >= 2)) || { usage >&2; exit 2; }
      REPORT_FILE="$2"
      shift 2
      ;;
    --phase)
      (($# >= 2)) || { usage >&2; exit 2; }
      PHASE="$2"
      shift 2
      ;;
    --confirm-runtime-mutation)
      CONFIRM_RUNTIME=1
      shift
      ;;
    --confirm-deployment-mutation)
      CONFIRM_DEPLOYMENT=1
      shift
      ;;
    --production-action)
      (($# >= 2)) || { usage >&2; exit 2; }
      PRODUCTION_ACTION="$2"
      shift 2
      ;;
    --confirm-production-roots)
      (($# >= 2)) || { usage >&2; exit 2; }
      PRODUCTION_CONFIRMATION="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
done

[[ -n "$CONFIG_FILE" && -n "$REPORT_FILE" ]] || { usage >&2; exit 2; }
case "$PHASE" in preflight|runtime|deployment|all) ;; *) usage >&2; exit 2 ;; esac
case "$PRODUCTION_ACTION" in ""|install|upgrade|upgrade-failure|rollback|uninstall) ;; *) usage >&2; exit 2 ;; esac
if [[ -n "$PRODUCTION_ACTION" && "$PHASE" != deployment ]]; then
  printf '%s\n' 'A production action requires --phase deployment.' >&2
  exit 2
fi

TEST_MODE="${TERMUX_MCP_RELEASE_VALIDATOR_TEST_MODE:-0}"
[[ "$TEST_MODE" == 0 || "$TEST_MODE" == 1 ]] || {
  printf '%s\n' 'TERMUX_MCP_RELEASE_VALIDATOR_TEST_MODE must be 0 or 1.' >&2
  exit 2
}
if [[ "$TEST_MODE" == 1 && -n "$PRODUCTION_ACTION" ]]; then
  printf '%s\n' 'Fixture mode cannot target production roots.' >&2
  exit 2
fi

EXPECTED_COMMIT=""
EXPECTED_VERSION=""
DEFAULT_ARTIFACT=""
DEFAULT_SHA256=""
DEFAULT_MANIFEST=""
MCP_ARTIFACT=""
MCP_SHA256=""
MCP_MANIFEST=""
BASELINE_ARTIFACT=""
BASELINE_VERSION=""
BASELINE_SHA256=""
AUTH_TOKEN_FILE=""
SAFE_ROOT=""
BIND_HOST=""
PORT="18765"
DEPLOY_SCRIPT=""
CI_RUN_ID=""
SECURITY_RUN_ID=""
ANDROID_RUN_ID=""
SUSTAINED_OBSERVATION_STATUS="not_run"
SUSTAINED_OBSERVATION_MINUTES="0"
SUSTAINED_OBSERVATION_REASON_CODE="not_observed"

REPORT_INITIALIZED=0
REPORT_TMP=""
REPORT_NEXT=""
REPORT_PUBLIC_NEXT=""
TEMP_ROOT=""
VALIDATION_SAFE_ROOT=""
SERVER_PID=""
RUNSVDIR_PID=""
DEDICATED_DEPLOY_ROOT=""
DEDICATED_CONFIG_ROOT=""
DEDICATED_SERVICE_ROOT=""
FAILURE_CODE=""
COMPLETED=0
CURRENT_PHASE="preflight"
MCP_TOKEN=""
MCP_SESSION_ID=""
CAPABILITY_KEY_ID="release-validator-1"
CAPABILITY_KEY_HEX=""
CAPABILITY_GRANT_FILE=""
AUTH_HEADER_FILE=""
REQUEST_FILE=""
SESSION_HEADER_FILE=""
PINNED_ARTIFACT_ROOT=""
DEFAULT_PINNED_ARTIFACT=""
MCP_PINNED_ARTIFACT=""
BASELINE_PINNED_ARTIFACT=""
STARTED_AT=""
RUN_ID=""

log() {
  printf '[release-validate] %s\n' "$*"
}

raw_fail() {
  printf '[release-validate] ERROR: %s\n' "$1" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || raw_fail "required_command_missing"
}

validate_private_file() {
  local path="$1" code="$2" mode permissions
  [[ -f "$path" && ! -L "$path" ]] || privacy_fail "$code"
  mode="$(stat -c '%a' "$path" 2>/dev/null)" || privacy_fail "$code"
  permissions=$((8#$mode))
  ((permissions == 0600)) || privacy_fail "$code"
}

privacy_fail() {
  if ((REPORT_INITIALIZED == 1)); then
    fail "$1"
  else
    raw_fail "$1"
  fi
}

validate_private_directory() {
  local path="$1" code="$2" mode permissions
  [[ -d "$path" && ! -L "$path" ]] || raw_fail "$code"
  mode="$(stat -c '%a' "$path" 2>/dev/null)" || raw_fail "$code"
  permissions=$((8#$mode))
  (((permissions & 077) == 0 && (permissions & 0300) == 0300)) || raw_fail "$code"
}

early_cleanup() {
  local status=$?
  trap - EXIT INT TERM HUP
  set +e
  [[ -z "$REPORT_PUBLIC_NEXT" ]] || rm -f -- "$REPORT_PUBLIC_NEXT" >/dev/null 2>&1
  [[ -z "$TEMP_ROOT" ]] || rm -rf -- "$TEMP_ROOT" >/dev/null 2>&1
  exit "$status"
}

for command_name in bash awk curl date dd dirname env file grep install jq ln mktemp mkdir mv readlink realpath rm rmdir sha256sum stat timeout uname wc cmp chmod kill seq sleep tr; do
  require_command "$command_name"
done

STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)-$$"

validate_private_file "$CONFIG_FILE" "config_not_private_regular_file"
CONFIG_BYTES="$(stat -c '%s' "$CONFIG_FILE" 2>/dev/null)" || raw_fail "config_size_invalid"
[[ "$CONFIG_BYTES" =~ ^[0-9]+$ ]] || raw_fail "config_size_invalid"
((CONFIG_BYTES > 0 && CONFIG_BYTES <= 65536)) || raw_fail "config_size_invalid"
[[ "$REPORT_FILE" == /* ]] || raw_fail "report_path_not_absolute"
[[ ! -e "$REPORT_FILE" && ! -L "$REPORT_FILE" ]] || raw_fail "report_already_exists"
REPORT_PARENT="$(dirname "$REPORT_FILE")"
validate_private_directory "$REPORT_PARENT" "report_parent_invalid"
[[ "$(realpath -e "$REPORT_PARENT" 2>/dev/null)" == "$REPORT_PARENT" ]] || raw_fail "report_parent_not_canonical"

declare -A SEEN_CONFIG_KEYS=()
config_line_count=0
while IFS= read -r line || [[ -n "$line" ]]; do
  config_line_count=$((config_line_count + 1))
  ((config_line_count <= 128)) || raw_fail "config_line_limit_exceeded"
  [[ "$line" != *$'\r'* ]] || raw_fail "config_carriage_return"
  [[ -z "$line" || "$line" == \#* ]] && continue
  [[ "$line" == *=* ]] || raw_fail "config_line_invalid"
  key="${line%%=*}"
  value="${line#*=}"
  [[ "$key" =~ ^[A-Z][A-Z0-9_]*$ ]] || raw_fail "config_key_invalid"
  [[ -z "${SEEN_CONFIG_KEYS[$key]+present}" ]] || raw_fail "config_key_duplicate"
  SEEN_CONFIG_KEYS["$key"]=1
  case "$key" in
    EXPECTED_COMMIT) EXPECTED_COMMIT="$value" ;;
    EXPECTED_VERSION) EXPECTED_VERSION="$value" ;;
    DEFAULT_ARTIFACT) DEFAULT_ARTIFACT="$value" ;;
    DEFAULT_SHA256) DEFAULT_SHA256="$value" ;;
    DEFAULT_MANIFEST) DEFAULT_MANIFEST="$value" ;;
    MCP_ARTIFACT) MCP_ARTIFACT="$value" ;;
    MCP_SHA256) MCP_SHA256="$value" ;;
    MCP_MANIFEST) MCP_MANIFEST="$value" ;;
    BASELINE_ARTIFACT) BASELINE_ARTIFACT="$value" ;;
    BASELINE_VERSION) BASELINE_VERSION="$value" ;;
    BASELINE_SHA256) BASELINE_SHA256="$value" ;;
    AUTH_TOKEN_FILE) AUTH_TOKEN_FILE="$value" ;;
    SAFE_ROOT) SAFE_ROOT="$value" ;;
    BIND_HOST) BIND_HOST="$value" ;;
    PORT) PORT="$value" ;;
    DEPLOY_SCRIPT) DEPLOY_SCRIPT="$value" ;;
    CI_RUN_ID) CI_RUN_ID="$value" ;;
    SECURITY_RUN_ID) SECURITY_RUN_ID="$value" ;;
    ANDROID_RUN_ID) ANDROID_RUN_ID="$value" ;;
    SUSTAINED_OBSERVATION_STATUS) SUSTAINED_OBSERVATION_STATUS="$value" ;;
    SUSTAINED_OBSERVATION_MINUTES) SUSTAINED_OBSERVATION_MINUTES="$value" ;;
    SUSTAINED_OBSERVATION_REASON_CODE) SUSTAINED_OBSERVATION_REASON_CODE="$value" ;;
    *) raw_fail "config_key_unknown" ;;
  esac
done <"$CONFIG_FILE"
unset line key value

[[ "$EXPECTED_COMMIT" =~ ^[0-9a-f]{40}$ ]] || raw_fail "expected_commit_invalid"
[[ "$EXPECTED_VERSION" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] || raw_fail "expected_version_invalid"
[[ "$DEFAULT_SHA256" =~ ^[0-9a-f]{64}$ && "$MCP_SHA256" =~ ^[0-9a-f]{64}$ ]] || raw_fail "artifact_digest_metadata_invalid"
[[ "$DEFAULT_SHA256" != "$MCP_SHA256" ]] || raw_fail "artifact_posture_digests_not_distinct"
[[ "$CI_RUN_ID" =~ ^[1-9][0-9]*$ && "$SECURITY_RUN_ID" =~ ^[1-9][0-9]*$ && "$ANDROID_RUN_ID" =~ ^[1-9][0-9]*$ ]] || raw_fail "workflow_metadata_invalid"
[[ "$BIND_HOST" == 127.0.0.1 ]] || raw_fail "bind_host_invalid"
[[ "$PORT" =~ ^[0-9]+$ ]] || raw_fail "port_invalid"
((PORT >= 1024 && PORT <= 65535)) || raw_fail "port_invalid"
case "$SUSTAINED_OBSERVATION_STATUS" in not_run|pass|fail) ;; *) raw_fail "sustained_status_invalid" ;; esac
[[ "$SUSTAINED_OBSERVATION_MINUTES" =~ ^[0-9]+$ ]] || raw_fail "sustained_minutes_invalid"
((SUSTAINED_OBSERVATION_MINUTES <= 10080)) || raw_fail "sustained_minutes_invalid"
case "$SUSTAINED_OBSERVATION_REASON_CODE" in
  not_observed|stable|battery_limit|thermal_limit|process_restriction|network_instability|operator_abort|other) ;;
  *) raw_fail "sustained_reason_invalid" ;;
esac
case "$SUSTAINED_OBSERVATION_STATUS" in
  not_run)
    ((SUSTAINED_OBSERVATION_MINUTES == 0)) || raw_fail "sustained_evidence_inconsistent"
    [[ "$SUSTAINED_OBSERVATION_REASON_CODE" == not_observed ]] || raw_fail "sustained_evidence_inconsistent"
    ;;
  pass)
    ((SUSTAINED_OBSERVATION_MINUTES >= MIN_SUSTAINED_MINUTES)) || raw_fail "sustained_window_too_short"
    [[ "$SUSTAINED_OBSERVATION_REASON_CODE" == stable ]] || raw_fail "sustained_evidence_inconsistent"
    ;;
  fail)
    ((SUSTAINED_OBSERVATION_MINUTES > 0)) || raw_fail "sustained_evidence_inconsistent"
    [[ "$SUSTAINED_OBSERVATION_REASON_CODE" != stable && "$SUSTAINED_OBSERVATION_REASON_CODE" != not_observed ]] || raw_fail "sustained_evidence_inconsistent"
    ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEPLOY_SCRIPT="${DEPLOY_SCRIPT:-$SCRIPT_DIR/termux_deploy.sh}"

TEMP_BASE="${HOME:-}"
[[ "$TEMP_BASE" == /* ]] || raw_fail "temporary_root_invalid"
validate_private_directory "$TEMP_BASE" "temporary_root_invalid"
[[ "$(realpath -e "$TEMP_BASE" 2>/dev/null)" == "$TEMP_BASE" ]] || raw_fail "temporary_root_invalid"
TEMP_ROOT="$(mktemp -d "$TEMP_BASE/termux-mcp-release-validate.XXXXXX" 2>/dev/null)" || raw_fail "temporary_workspace_create_failed"
trap early_cleanup EXIT
trap 'exit 130' INT TERM HUP
chmod 700 "$TEMP_ROOT" 2>/dev/null || raw_fail "temporary_workspace_create_failed"
REPORT_TMP="$TEMP_ROOT/evidence.json"
REPORT_NEXT="$TEMP_ROOT/evidence.next.json"
REPORT_PUBLIC_NEXT="$(mktemp "$REPORT_PARENT/.termux-mcp-release-evidence.XXXXXX" 2>/dev/null)" || raw_fail "report_staging_create_failed"
[[ -f "$REPORT_PUBLIC_NEXT" && ! -L "$REPORT_PUBLIC_NEXT" ]] || raw_fail "report_staging_create_failed"
chmod 600 "$REPORT_PUBLIC_NEXT" 2>/dev/null || raw_fail "report_staging_create_failed"

jq -n \
  --arg validator_version "$VALIDATOR_VERSION" \
  --arg started_at "$STARTED_AT" \
  --arg commit "$EXPECTED_COMMIT" \
  --arg version "$EXPECTED_VERSION" \
  --arg ci_run_id "$CI_RUN_ID" \
  --arg security_run_id "$SECURITY_RUN_ID" \
  --arg android_run_id "$ANDROID_RUN_ID" \
  --arg architecture "$(uname -m)" \
  --arg bash_version "$(bash --version | awk 'NR==1 {print}')" \
  --arg curl_version "$(curl --version | awk 'NR==1 {print}')" \
  --arg file_version "$(file --version | awk 'NR==1 {print}')" \
  --arg jq_version "$(jq --version)" \
  --arg default_sha "$DEFAULT_SHA256" \
  --arg mcp_sha "$MCP_SHA256" \
  --arg sustained_status "$SUSTAINED_OBSERVATION_STATUS" \
  --argjson sustained_minutes "$SUSTAINED_OBSERVATION_MINUTES" \
  --arg sustained_reason "$SUSTAINED_OBSERVATION_REASON_CODE" \
  --arg phase "$PHASE" \
  --argjson schema_version "$EVIDENCE_SCHEMA_VERSION" \
  --argjson fixture_mode "$([[ "$TEST_MODE" == 1 ]] && printf true || printf false)" \
  --argjson minimum_minutes "$MIN_SUSTAINED_MINUTES" \
  '{
    schemaVersion: $schema_version,
    validatorVersion: $validator_version,
    status: "running",
    failureCode: null,
    releaseEligible: false,
    startedAt: $started_at,
    completedAt: null,
    repository: {
      commit: $commit,
      version: $version,
      ciRunId: $ci_run_id,
      securityRunId: $security_run_id,
      androidRunId: $android_run_id
    },
    environment: {
      architecture: $architecture,
      fixtureMode: $fixture_mode,
      tools: {
        bash: $bash_version,
        curl: $curl_version,
        file: $file_version,
        jq: $jq_version
      }
    },
    requestedPhase: $phase,
    artifacts: {
      default: {sha256: $default_sha, bytes: null, version: null, elf: null},
      mcpRuntime: {sha256: $mcp_sha, bytes: null, version: null, elf: null},
      baseline: null
    },
    phases: {
      preflight: "not_run",
      runtime: "not_run",
      deployment: "not_run"
    },
    results: [],
    sustainedObservation: {
      operatorSupplied: true,
      status: $sustained_status,
      minutes: $sustained_minutes,
      reasonCode: $sustained_reason,
      minimumMinutes: $minimum_minutes
    }
  }' >"$REPORT_TMP" 2>/dev/null || raw_fail "evidence_initialization_failed"
chmod 600 "$REPORT_TMP" 2>/dev/null || raw_fail "evidence_initialization_failed"
REPORT_INITIALIZED=1

json_update() {
  local filter="$1"
  shift
  jq "$@" "$filter" "$REPORT_TMP" >"$REPORT_NEXT" 2>/dev/null || return 1
  chmod 600 "$REPORT_NEXT" 2>/dev/null || return 1
  mv -f -- "$REPORT_NEXT" "$REPORT_TMP" 2>/dev/null
}

record_result() {
  local phase="$1" check="$2" outcome="$3" code="$4"
  [[ "$phase" =~ ^[a-z_]+$ && "$check" =~ ^[a-z0-9_]+$ && "$outcome" =~ ^(pass|fail|info)$ && "$code" =~ ^[a-z0-9_]+$ ]] || return 1
  json_update \
    '.results += [{phase: $phase, check: $check, outcome: $outcome, code: $code}]' \
    --arg phase "$phase" --arg check "$check" --arg outcome "$outcome" --arg code "$code"
}

set_phase() {
  local phase="$1" state="$2"
  json_update '.phases[$phase] = $state' --arg phase "$phase" --arg state "$state"
}

set_artifact_evidence() {
  local posture="$1" bytes="$2" version="$3"
  json_update \
    '.artifacts[$posture].bytes = $bytes | .artifacts[$posture].version = $version | .artifacts[$posture].elf = "aarch64-android-elf"' \
    --arg posture "$posture" --argjson bytes "$bytes" --arg version "$version"
}

fail() {
  local code="$1"
  FAILURE_CODE="$code"
  if ((REPORT_INITIALIZED == 1)); then
    record_result "$CURRENT_PHASE" "phase_failure" fail "$code" || true
    set_phase "$CURRENT_PHASE" fail || true
  fi
  log "ERROR code=$code"
  exit 1
}

stop_server() {
  [[ -n "$SERVER_PID" ]] || return 0
  if kill -0 "$SERVER_PID" >/dev/null 2>&1; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    local attempt
    for attempt in $(seq 1 40); do
      kill -0 "$SERVER_PID" >/dev/null 2>&1 || break
      sleep 0.1
    done
    if kill -0 "$SERVER_PID" >/dev/null 2>&1; then
      kill -KILL "$SERVER_PID" >/dev/null 2>&1 || true
      wait "$SERVER_PID" >/dev/null 2>&1 || true
      SERVER_PID=""
      return 1
    fi
  fi
  wait "$SERVER_PID" >/dev/null 2>&1 || true
  SERVER_PID=""
}

cleanup_dedicated_deployment() {
  local cleanup_ok=1
  if [[ -n "$DEDICATED_SERVICE_ROOT" && -d "$DEDICATED_SERVICE_ROOT/mcp_runtime" && "$TEST_MODE" == 0 ]]; then
    if ! command -v sv >/dev/null 2>&1 || ! sv down "$DEDICATED_SERVICE_ROOT/mcp_runtime" >/dev/null 2>&1; then
      cleanup_ok=0
    fi
  fi
  if [[ -n "$RUNSVDIR_PID" ]]; then
    kill "$RUNSVDIR_PID" >/dev/null 2>&1 || true
    wait "$RUNSVDIR_PID" >/dev/null 2>&1 || true
    RUNSVDIR_PID=""
  fi
  if ((cleanup_ok == 1)); then
    if [[ -n "$DEDICATED_DEPLOY_ROOT" && "$DEDICATED_DEPLOY_ROOT" == "$HOME"/.local/share/termux-mcp-release-validation-* ]]; then
      rm -rf -- "$DEDICATED_DEPLOY_ROOT" >/dev/null 2>&1 || cleanup_ok=0
    fi
    if [[ -n "$DEDICATED_CONFIG_ROOT" && "$DEDICATED_CONFIG_ROOT" == "$HOME"/.config/termux-mcp-release-validation-* ]]; then
      rm -rf -- "$DEDICATED_CONFIG_ROOT" >/dev/null 2>&1 || cleanup_ok=0
    fi
    if [[ -n "$DEDICATED_SERVICE_ROOT" && -n "${PREFIX:-}" && "$DEDICATED_SERVICE_ROOT" == "$PREFIX"/var/service-termux-mcp-release-validation-* ]]; then
      rm -rf -- "$DEDICATED_SERVICE_ROOT" >/dev/null 2>&1 || cleanup_ok=0
    fi
  fi
  if ((cleanup_ok == 1)); then
    local path
    for path in "$DEDICATED_DEPLOY_ROOT" "$DEDICATED_CONFIG_ROOT" "$DEDICATED_SERVICE_ROOT"; do
      [[ -z "$path" || (! -e "$path" && ! -L "$path") ]] || cleanup_ok=0
    done
  fi
  return "$((cleanup_ok == 1 ? 0 : 1))"
}

finalize_report() {
  ((REPORT_INITIALIZED == 1)) || return 0
  local completed_at status failure_json release_eligible=false
  completed_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  if [[ -z "$FAILURE_CODE" && "$COMPLETED" == 1 ]]; then
    if [[ "$TEST_MODE" == 1 ]]; then status=fixture; else status=pass; fi
  else
    status=fail
    [[ -n "$FAILURE_CODE" ]] || FAILURE_CODE=unexpected_error
  fi
  if [[ "$status" == pass && "$PHASE" == all && "$SUSTAINED_OBSERVATION_STATUS" == pass ]]; then
    release_eligible=true
  fi
  if [[ -n "$FAILURE_CODE" ]]; then
    failure_json="$FAILURE_CODE"
  else
    failure_json=""
  fi
  json_update \
    '.status = $status
     | .failureCode = (if $failure == "" then null else $failure end)
     | .releaseEligible = $release_eligible
     | .completedAt = $completed_at' \
    --arg status "$status" --arg failure "$failure_json" \
    --argjson release_eligible "$release_eligible" --arg completed_at "$completed_at" || return 1
  [[ -f "$REPORT_PUBLIC_NEXT" && ! -L "$REPORT_PUBLIC_NEXT" ]] || return 1
  install -m 600 "$REPORT_TMP" "$REPORT_PUBLIC_NEXT" 2>/dev/null || return 1
  if [[ "$TEST_MODE" == 1 && "${TERMUX_MCP_RELEASE_VALIDATOR_TEST_CREATE_REPORT_COLLISION:-0}" == 1 ]]; then
    printf '%s' preserve-existing-report >"$REPORT_FILE" 2>/dev/null || return 1
    chmod 600 "$REPORT_FILE" 2>/dev/null || return 1
  fi
  mv -Tn -- "$REPORT_PUBLIC_NEXT" "$REPORT_FILE" 2>/dev/null || return 1
  [[ ! -e "$REPORT_PUBLIC_NEXT" && ! -L "$REPORT_PUBLIC_NEXT" && -f "$REPORT_FILE" && ! -L "$REPORT_FILE" ]] || return 1
}

cleanup() {
  local status=$? cleanup_failed=0
  trap - EXIT ERR INT TERM HUP
  set +e
  stop_server || cleanup_failed=1
  if [[ -n "$VALIDATION_SAFE_ROOT" && "$VALIDATION_SAFE_ROOT" == "$SAFE_ROOT"/.termux-mcp-release-validation-* ]]; then
    rm -rf -- "$VALIDATION_SAFE_ROOT" >/dev/null 2>&1 || cleanup_failed=1
  fi
  if [[ -z "$PRODUCTION_ACTION" ]]; then
    cleanup_dedicated_deployment || cleanup_failed=1
  fi
  unset MCP_TOKEN MCP_SESSION_ID CAPABILITY_KEY_HEX 2>/dev/null || true
  if ((cleanup_failed != 0)); then
    FAILURE_CODE=cleanup_unconfirmed
    status=1
  fi
  if ((status != 0)) && [[ -z "$FAILURE_CODE" ]]; then
    FAILURE_CODE=unexpected_error
  fi
  if ! finalize_report; then
    FAILURE_CODE=report_write_failed
    status=1
  fi
  [[ -z "$REPORT_PUBLIC_NEXT" ]] || rm -f -- "$REPORT_PUBLIC_NEXT" >/dev/null 2>&1
  [[ -z "$TEMP_ROOT" ]] || rm -rf -- "$TEMP_ROOT" >/dev/null 2>&1
  if ((status == 0)); then
    log "result=PASS"
  else
    log "result=FAIL code=$FAILURE_CODE"
  fi
  log "report_written=$([[ -f "$REPORT_FILE" ]] && printf true || printf false)"
  exit "$status"
}
trap cleanup EXIT
trap 'FAILURE_CODE=unexpected_error; exit 1' ERR
trap 'FAILURE_CODE=interrupted; exit 130' INT TERM HUP

validate_artifact() {
  local posture="$1" json_posture="$2" artifact="$3" expected_sha="$4" expected_version="$5"
  local bytes actual_sha identity reported_version pinned_artifact pinned_bytes pinned_sha
  [[ -f "$artifact" && ! -L "$artifact" && -x "$artifact" ]] || fail "${posture}_artifact_invalid"
  bytes="$(stat -c '%s' "$artifact" 2>/dev/null)" || fail "${posture}_artifact_stat_failed"
  [[ "$bytes" =~ ^[0-9]+$ ]] || fail "${posture}_artifact_stat_failed"
  ((bytes > 0 && bytes <= MAX_ARTIFACT_BYTES)) || fail "${posture}_artifact_size_invalid"
  actual_sha="$(sha256sum -- "$artifact" 2>/dev/null | awk '{print $1}')" || fail "${posture}_artifact_digest_failed"
  [[ "$actual_sha" == "$expected_sha" ]] || fail "${posture}_artifact_digest_mismatch"
  if [[ -z "$PINNED_ARTIFACT_ROOT" ]]; then
    PINNED_ARTIFACT_ROOT="$TEMP_ROOT/verified-artifacts"
    mkdir -m 700 -- "$PINNED_ARTIFACT_ROOT" 2>/dev/null || fail artifact_pinning_failed
  fi
  pinned_artifact="$PINNED_ARTIFACT_ROOT/$posture"
  [[ ! -e "$pinned_artifact" && ! -L "$pinned_artifact" ]] || fail artifact_pinning_failed
  install -m 700 "$artifact" "$pinned_artifact" 2>/dev/null || fail artifact_pinning_failed
  pinned_bytes="$(stat -c '%s' "$pinned_artifact" 2>/dev/null)" || fail artifact_pinning_failed
  [[ "$pinned_bytes" == "$bytes" ]] || fail artifact_changed_during_pinning
  pinned_sha="$(sha256sum -- "$pinned_artifact" 2>/dev/null | awk '{print $1}')" || fail artifact_pinning_failed
  [[ "$pinned_sha" == "$actual_sha" ]] || fail artifact_changed_during_pinning
  identity="$(file -b -- "$pinned_artifact" 2>/dev/null)" || fail "${posture}_artifact_identity_failed"
  [[ "$identity" == *ELF* && "$identity" == *"ARM aarch64"* && "$identity" == *Android* ]] || fail "${posture}_artifact_architecture_mismatch"
  case "$posture" in
    default) validate_artifact_manifest "$posture" "$DEFAULT_MANIFEST" "$actual_sha" "$bytes" "$expected_version" ;;
    mcp_runtime) validate_artifact_manifest "$posture" "$MCP_MANIFEST" "$actual_sha" "$bytes" "$expected_version" ;;
  esac
  reported_version="$(timeout -k 2 5 "$pinned_artifact" --version 2>/dev/null | awk 'NR==1 {print $NF}')" || fail "${posture}_artifact_version_failed"
  [[ "$reported_version" == "$expected_version" ]] || fail "${posture}_artifact_version_mismatch"
  case "$posture" in
    default) DEFAULT_PINNED_ARTIFACT="$pinned_artifact" ;;
    mcp_runtime) MCP_PINNED_ARTIFACT="$pinned_artifact" ;;
    baseline) BASELINE_PINNED_ARTIFACT="$pinned_artifact" ;;
  esac
  record_result preflight "${posture}_artifact" pass artifact_verified
  if [[ "$posture" == baseline ]]; then
    json_update \
      '.artifacts.baseline = {sha256: $sha, bytes: $bytes, version: $version, elf: "aarch64-android-elf"}' \
      --arg sha "$actual_sha" --argjson bytes "$bytes" --arg version "$reported_version"
  else
    set_artifact_evidence "$json_posture" "$bytes" "$reported_version"
  fi
}

validate_artifact_manifest() {
  local posture="$1" manifest="$2" expected_sha="$3" expected_bytes="$4" expected_version="$5"
  local manifest_bytes manifest_posture artifact_name
  [[ -f "$manifest" && ! -L "$manifest" ]] || fail "${posture}_manifest_invalid"
  manifest_bytes="$(stat -c '%s' "$manifest" 2>/dev/null)" || fail "${posture}_manifest_invalid"
  [[ "$manifest_bytes" =~ ^[0-9]+$ ]] || fail "${posture}_manifest_invalid"
  ((manifest_bytes > 0 && manifest_bytes <= 16384)) || fail "${posture}_manifest_size_invalid"
  case "$posture" in
    default)
      manifest_posture=default
      artifact_name=termux-mcp-server-aarch64-linux-android-default
      ;;
    mcp_runtime)
      manifest_posture=mcp-runtime
      artifact_name=termux-mcp-server-aarch64-linux-android-mcp-runtime
      ;;
    *) fail artifact_manifest_posture_invalid ;;
  esac
  jq -e \
    --arg repository "CyberBASSLord-666/termux-mcp-edge" \
    --arg commit "$EXPECTED_COMMIT" \
    --arg run_id "$ANDROID_RUN_ID" \
    --arg artifact_name "$artifact_name" \
    --arg posture "$manifest_posture" \
    --arg version "$expected_version" \
    --arg sha "$expected_sha" \
    --argjson bytes "$expected_bytes" '
      (keys == ["artifactName","bytes","commit","createdAt","elf","features","fileName","posture","repository","schemaVersion","sha256","target","version","workflowRunId"])
      and .schemaVersion == 1
      and .repository == $repository
      and .commit == $commit
      and .workflowRunId == $run_id
      and .artifactName == $artifact_name
      and .posture == $posture
      and .features == (if $posture == "mcp-runtime" then ["mcp-runtime"] else [] end)
      and .target == "aarch64-linux-android"
      and .fileName == "termux-mcp-server"
      and .version == $version
      and .sha256 == $sha
      and .bytes == $bytes
      and .elf == "aarch64-android-elf"
      and (.createdAt | type == "string" and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"))
    ' "$manifest" >/dev/null 2>&1 || fail "${posture}_manifest_mismatch"
  record_result preflight "${posture}_manifest" pass artifact_manifest_verified
}

run_preflight() {
  CURRENT_PHASE=preflight
  set_phase preflight running
  validate_artifact default default "$DEFAULT_ARTIFACT" "$DEFAULT_SHA256" "$EXPECTED_VERSION"
  validate_artifact mcp_runtime mcpRuntime "$MCP_ARTIFACT" "$MCP_SHA256" "$EXPECTED_VERSION"
  [[ "$(realpath -e "$DEFAULT_ARTIFACT" 2>/dev/null)" != "$(realpath -e "$MCP_ARTIFACT" 2>/dev/null)" ]] || fail artifact_postures_not_distinct
  [[ "$(realpath -e "$DEFAULT_MANIFEST" 2>/dev/null)" != "$(realpath -e "$MCP_MANIFEST" 2>/dev/null)" ]] || fail artifact_manifests_not_distinct
  if [[ "$PHASE" == deployment || "$PHASE" == all ]] && [[ -z "$PRODUCTION_ACTION" ]]; then
    [[ "$BASELINE_SHA256" =~ ^[0-9a-f]{64}$ && "$BASELINE_VERSION" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] || fail baseline_metadata_invalid
    [[ "$BASELINE_VERSION" != "$EXPECTED_VERSION" ]] || fail baseline_version_not_distinct
    validate_artifact baseline baseline "$BASELINE_ARTIFACT" "$BASELINE_SHA256" "$BASELINE_VERSION"
  fi
  record_result preflight metadata pass exact_metadata_supplied
  set_phase preflight pass
}

prepare_runtime_inputs() {
  validate_private_file "$AUTH_TOKEN_FILE" auth_token_file_invalid
  local token_bytes
  token_bytes="$(stat -c '%s' "$AUTH_TOKEN_FILE" 2>/dev/null)" || fail auth_token_size_invalid
  [[ "$token_bytes" =~ ^[0-9]+$ ]] || fail auth_token_size_invalid
  ((token_bytes > 0 && token_bytes <= 4096)) || fail auth_token_size_invalid
  MCP_TOKEN="$(<"$AUTH_TOKEN_FILE")"
  ((${#MCP_TOKEN} == token_bytes)) || fail auth_token_invalid
  [[ "$MCP_TOKEN" =~ ^[!-~]+$ ]] || fail auth_token_invalid
  if [[ -z "$CAPABILITY_KEY_HEX" ]]; then
    CAPABILITY_KEY_HEX="$(dd if=/dev/urandom bs=32 count=1 status=none | sha256sum | awk '{print $1}')" || fail capability_key_generation_failed
    [[ "$CAPABILITY_KEY_HEX" =~ ^[0-9a-f]{64}$ ]] || fail capability_key_generation_failed
  fi
  if [[ -z "$AUTH_HEADER_FILE" ]]; then
    AUTH_HEADER_FILE="$TEMP_ROOT/auth-header.txt"
    REQUEST_FILE="$TEMP_ROOT/request.json"
    SESSION_HEADER_FILE="$TEMP_ROOT/session-headers.txt"
    CAPABILITY_GRANT_FILE="$TEMP_ROOT/create-directory-grant.txt"
    printf 'Authorization: Bearer %s\n' "$MCP_TOKEN" >"$AUTH_HEADER_FILE" 2>/dev/null || fail private_request_staging_failed
    : >"$REQUEST_FILE" 2>/dev/null || fail private_request_staging_failed
    : >"$SESSION_HEADER_FILE" 2>/dev/null || fail private_request_staging_failed
    : >"$CAPABILITY_GRANT_FILE" 2>/dev/null || fail private_request_staging_failed
    chmod 600 "$AUTH_HEADER_FILE" "$REQUEST_FILE" "$SESSION_HEADER_FILE" "$CAPABILITY_GRANT_FILE" 2>/dev/null || fail private_request_staging_failed
  fi
  [[ "$SAFE_ROOT" == /* && -d "$SAFE_ROOT" && ! -L "$SAFE_ROOT" ]] || fail safe_root_invalid
  SAFE_ROOT="${SAFE_ROOT%/}"
  [[ "$SAFE_ROOT" != "$HOME" && "$SAFE_ROOT" == "$HOME"/* ]] || fail safe_root_invalid
  [[ "$(realpath -e "$SAFE_ROOT" 2>/dev/null)" == "$SAFE_ROOT" ]] || fail safe_root_not_canonical
  if (exec 9<>"/dev/tcp/$BIND_HOST/$PORT") 2>/dev/null; then
    exec 9>&-
    fail runtime_port_in_use
  fi
  if [[ -n "$VALIDATION_SAFE_ROOT" ]]; then
    [[ -d "$VALIDATION_SAFE_ROOT" && ! -L "$VALIDATION_SAFE_ROOT" ]] || fail validation_safe_root_invalid
    return 0
  fi
  VALIDATION_SAFE_ROOT="$SAFE_ROOT/.termux-mcp-release-validation-$RUN_ID"
  [[ ! -e "$VALIDATION_SAFE_ROOT" && ! -L "$VALIDATION_SAFE_ROOT" ]] || fail validation_safe_root_exists
  mkdir -m 700 -- "$VALIDATION_SAFE_ROOT" 2>/dev/null || fail validation_safe_root_create_failed
  printf '%s' validation-visible >"$VALIDATION_SAFE_ROOT/visible.txt" 2>/dev/null || fail validation_safe_root_write_failed
  chmod 600 "$VALIDATION_SAFE_ROOT/visible.txt" 2>/dev/null || fail validation_safe_root_write_failed
  local index
  for index in $(seq 1 64); do
    printf 'entry-%03d' "$index" >"$VALIDATION_SAFE_ROOT/entry-$(printf '%03d' "$index").txt" 2>/dev/null || fail validation_safe_root_write_failed
  done
}

curl_local() {
  command curl --disable --proto '=http' --noproxy '*' --connect-timeout 2 --max-time 10 "$@"
}

require_termux_aarch64_environment() {
  local failure_code="$1" prefix_base
  [[ "$(uname -m)" == aarch64 || "$(uname -m)" == arm64 ]] || fail "$failure_code"
  [[ -n "${PREFIX:-}" && "$PREFIX" =~ ^/data/data/[^/]+/files/usr$ ]] || fail "$failure_code"
  prefix_base="${PREFIX%/usr}"
  [[ "$HOME" == "$prefix_base/home" ]] || fail "$failure_code"
  [[ "$(realpath -e "$HOME" 2>/dev/null)" == "$HOME" ]] || fail "$failure_code"
}

stage_request() {
  printf '%s' "$1" >"$REQUEST_FILE" 2>/dev/null || fail private_request_staging_failed
}

stage_session_headers() {
  local session_id="${1:-}" include_protocol="${2:-1}" grant_file="${3:-}" grant=""
  {
    printf 'Authorization: Bearer %s\n' "$MCP_TOKEN"
    [[ -z "$session_id" ]] || printf 'MCP-Session-Id: %s\n' "$session_id"
    if [[ -n "$session_id" && "$include_protocol" != 0 ]]; then
      printf 'MCP-Protocol-Version: %s\n' "$MCP_PROTOCOL_VERSION"
    fi
    if [[ -n "$grant_file" ]]; then
      validate_private_file "$grant_file" capability_grant_file_invalid
      grant="$(<"$grant_file")"
      [[ "$grant" =~ ^v1\.${CAPABILITY_KEY_ID}\.[0-9a-f]{260}\.[0-9a-f]{64}$ ]] || fail capability_grant_output_invalid
      printf 'MCP-Capability-Grant: %s\n' "$grant"
    fi
  } >"$SESSION_HEADER_FILE" 2>/dev/null || fail private_request_staging_failed
  unset grant
  chmod 600 "$SESSION_HEADER_FILE" 2>/dev/null || fail private_request_staging_failed
}

issue_create_directory_grant() {
  local target="$1"
  : >"$CAPABILITY_GRANT_FILE" 2>/dev/null || fail capability_grant_staging_failed
  chmod 600 "$CAPABILITY_GRANT_FILE" 2>/dev/null || fail capability_grant_staging_failed
  if ! MCP__AUTH__STATIC_TOKEN="$MCP_TOKEN" \
    MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=false \
    MCP__FILE__SAFE_ROOTS="$VALIDATION_SAFE_ROOT" \
    MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=true \
    MCP__CAPABILITY__KEY_ID="$CAPABILITY_KEY_ID" \
    MCP__CAPABILITY__HMAC_KEY_HEX="$CAPABILITY_KEY_HEX" \
    MCP__CAPABILITY__SESSION_ID="$MCP_SESSION_ID" \
    MCP__CAPABILITY__CREATE_DIRECTORY_TARGET="$target" \
      "$MCP_PINNED_ARTIFACT" --issue-create-directory-grant >"$CAPABILITY_GRANT_FILE" 2>/dev/null
  then
    fail capability_grant_issue_failed
  fi
  [[ "$(wc -l <"$CAPABILITY_GRANT_FILE" 2>/dev/null)" == 1 ]] || fail capability_grant_output_invalid
  local grant
  grant="$(<"$CAPABILITY_GRANT_FILE")"
  [[ "$grant" =~ ^v1\.${CAPABILITY_KEY_ID}\.[0-9a-f]{260}\.[0-9a-f]{64}$ ]] || fail capability_grant_output_invalid
  unset grant
}

start_server() {
  local artifact="$1" posture="$2"
  local log_file="$TEMP_ROOT/server-$posture.log"
  local -a environment=(
    "MCP__AUTH__STATIC_TOKEN=$MCP_TOKEN"
    "MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=false"
    "MCP__SERVER__HOST=$BIND_HOST"
    "MCP__SERVER__PORT=$PORT"
    "MCP__TRANSPORT__ALLOWED_HOSTS=localhost:$PORT,127.0.0.1:$PORT"
    "MCP__TRANSPORT__ALLOWED_ORIGINS=http://localhost:$PORT,http://127.0.0.1:$PORT"
    "MCP__TRANSPORT__ALLOW_MISSING_ORIGIN=false"
    "MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS=4"
    "MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS=30"
    "MCP__TRANSPORT__MAX_BODY_BYTES=1024"
    "MCP__FILE__SAFE_ROOTS=$VALIDATION_SAFE_ROOT"
    "RUST_LOG=termux_mcp_server=info"
  )
  if [[ "$posture" == mcp ]]; then
    environment+=(
      "MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=true"
      "MCP__CAPABILITY__KEY_ID=$CAPABILITY_KEY_ID"
      "MCP__CAPABILITY__HMAC_KEY_HEX=$CAPABILITY_KEY_HEX"
    )
  fi
  env "${environment[@]}" "$artifact" >"$log_file" 2>&1 &
  SERVER_PID=$!
  local attempt
  for attempt in $(seq 1 80); do
    kill -0 "$SERVER_PID" >/dev/null 2>&1 || fail "${posture}_runtime_exited"
    if [[ "$(curl_local -fsS --max-time 2 "http://$BIND_HOST:$PORT/health" 2>/dev/null || true)" == ok ]]; then
      return 0
    fi
    sleep 0.1
  done
  fail "${posture}_runtime_not_ready"
}

expect_status() {
  local check="$1" actual="$2" expected="$3" code="$4"
  local failure_code="${5:-${check}_status_mismatch}"
  [[ "$actual" == "$expected" ]] || fail "$failure_code"
  record_result runtime "$check" pass "$code"
}

mcp_post() {
  local output="$1" payload="$2" session_id="${3:-}" include_protocol="${4:-1}" grant_file="${5:-}"
  stage_request "$payload"
  stage_session_headers "$session_id" "$include_protocol" "$grant_file"
  local -a args=(
    -sS -o "$output" -w '%{http_code}'
    -H "@$SESSION_HEADER_FILE"
    -H "Host: localhost:$PORT"
    -H "Origin: http://localhost:$PORT"
    -H 'Content-Type: application/json'
    -H 'Accept: application/json, text/event-stream'
  )
  curl_local "${args[@]}" --data-binary "@$REQUEST_FILE" "http://$BIND_HOST:$PORT/mcp"
}

run_default_runtime_checks() {
  local body="$TEMP_ROOT/default-response.json" status
  start_server "$DEFAULT_PINNED_ARTIFACT" default
  curl_local -fsS -o "$body" "http://$BIND_HOST:$PORT/ready" 2>/dev/null || fail default_readiness_failed
  jq -e --arg version "$EXPECTED_VERSION" '
    .status == "ready"
    and .version == $version
    and .mcp_runtime_enabled == false
    and .safe_root_count == 1
    and .auth_posture == "static_token"
    and (has("mcp_request_limits") | not)
  ' "$body" >/dev/null 2>&1 || fail default_feature_posture_mismatch
  record_result runtime default_readiness pass default_posture_verified
  stage_request '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"release-validator","version":"1"}}}'
  status="$(curl_local -sS -o "$body" -w '%{http_code}' \
    -H "@$AUTH_HEADER_FILE" \
    -H "Host: localhost:$PORT" \
    -H "Origin: http://localhost:$PORT" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    --data-binary "@$REQUEST_FILE" \
    "http://$BIND_HOST:$PORT/mcp")"
  expect_status default_mcp_absent "$status" 404 default_mcp_route_absent default_mcp_route_present
  stop_server || fail default_runtime_stop_failed
}

run_mcp_runtime_checks() {
  local body="$TEMP_ROOT/mcp-response.json" headers="$TEMP_ROOT/mcp-headers.txt"
  local second="$TEMP_ROOT/mcp-second.json" status payload oversized bytes directory_target mismatch_target
  start_server "$MCP_PINNED_ARTIFACT" mcp
  curl_local -fsS -o "$body" "http://$BIND_HOST:$PORT/ready" 2>/dev/null || fail mcp_readiness_failed
  jq -e --arg version "$EXPECTED_VERSION" '
    .status == "ready"
    and .version == $version
    and .mcp_runtime_enabled == true
    and .safe_root_count == 1
    and .auth_posture == "static_token"
    and .mcp_request_limits.max_concurrent_requests == 4
    and .mcp_request_limits.request_timeout_seconds == 30
    and .mcp_request_limits.max_body_bytes == 1024
  ' "$body" >/dev/null 2>&1 || fail mcp_feature_posture_mismatch
  record_result runtime mcp_readiness pass mcp_posture_verified

  payload='{"jsonrpc":"2.0","id":"unauthorized","method":"tools/list"}'
  stage_request "$payload"
  status="$(curl_local -sS -o "$body" -w '%{http_code}' \
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
    --data-binary "@$REQUEST_FILE" "http://$BIND_HOST:$PORT/mcp")"
  expect_status unauthenticated_rejection "$status" 401 unauthenticated_rejected
  jq -e '.error == "unauthorized"' "$body" >/dev/null 2>&1 || fail unauthenticated_body_invalid

  status="$(curl_local -sS -o "$body" -w '%{http_code}' \
    -H "Host: attacker.invalid:$PORT" -H 'Origin: https://attacker.invalid' \
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
    --data-binary "@$REQUEST_FILE" "http://$BIND_HOST:$PORT/mcp")"
  expect_status authentication_before_transport "$status" 401 authentication_precedes_transport_security
  jq -e '.error == "unauthorized"' "$body" >/dev/null 2>&1 || fail authentication_order_body_invalid

  status="$(curl_local -sS -o "$body" -w '%{http_code}' \
    -H "@$AUTH_HEADER_FILE" \
    -H "Host: attacker.invalid:$PORT" -H "Origin: http://localhost:$PORT" \
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
    --data-binary "@$REQUEST_FILE" "http://$BIND_HOST:$PORT/mcp")"
  expect_status disallowed_host "$status" 403 disallowed_host_rejected
  jq -e '.error == "transport_security_rejected" and .message == "host_not_allowed"' "$body" >/dev/null 2>&1 || fail disallowed_host_body_invalid

  status="$(curl_local -sS -o "$body" -w '%{http_code}' \
    -H "@$AUTH_HEADER_FILE" \
    -H "Host: localhost:$PORT" \
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
    --data-binary "@$REQUEST_FILE" "http://$BIND_HOST:$PORT/mcp")"
  expect_status required_origin "$status" 403 missing_origin_rejected
  jq -e '.error == "transport_security_rejected" and .message == "origin_required"' "$body" >/dev/null 2>&1 || fail missing_origin_body_invalid

  status="$(curl_local -sS -o "$body" -w '%{http_code}' \
    -H "@$AUTH_HEADER_FILE" \
    -H "Host: localhost:$PORT" -H 'Origin: https://attacker.invalid' \
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
    --data-binary "@$REQUEST_FILE" "http://$BIND_HOST:$PORT/mcp")"
  expect_status disallowed_origin "$status" 403 disallowed_origin_rejected
  jq -e '.error == "transport_security_rejected" and .message == "origin_not_allowed"' "$body" >/dev/null 2>&1 || fail disallowed_origin_body_invalid

  payload='{"jsonrpc":"2.0","id":"initialize","method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"release-validator","version":"1.0.0"}}}'
  stage_request "$payload"
  status="$(curl_local -sS -D "$headers" -o "$body" -w '%{http_code}' \
    -H "@$AUTH_HEADER_FILE" \
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
    --data-binary "@$REQUEST_FILE" "http://$BIND_HOST:$PORT/mcp")"
  expect_status initialize "$status" 200 initialize_succeeded
  jq -e --arg version "$EXPECTED_VERSION" \
    '.result.protocolVersion == "2025-11-25" and .result.serverInfo.name == "termux-mcp-edge" and .result.serverInfo.version == $version' \
    "$body" >/dev/null 2>&1 || fail initialize_body_invalid
  MCP_SESSION_ID="$(awk 'tolower($1) == "mcp-session-id:" {sub(/^[^:]*:[[:space:]]*/, ""); sub(/\r$/, ""); print; exit}' "$headers")"
  [[ "$MCP_SESSION_ID" =~ ^[A-Za-z0-9-]{1,128}$ ]] || fail session_header_invalid

  payload='{"jsonrpc":"2.0","method":"notifications/initialized"}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status initialized_notification "$status" 202 initialized_notification_accepted
  [[ ! -s "$body" ]] || fail initialized_notification_body_present

  payload='{"jsonrpc":"2.0","id":"missing-protocol","method":"tools/list"}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 0)"
  expect_status protocol_header_required "$status" 400 protocol_header_enforced
  jq -e '.error == "protocol_version_required"' "$body" >/dev/null 2>&1 || fail protocol_header_error_invalid

  status="$(mcp_post "$body" "$payload" "00000000-0000-4000-8000-000000000000")"
  expect_status unknown_session "$status" 404 unknown_session_rejected
  jq -e '.error == "session_not_found"' "$body" >/dev/null 2>&1 || fail session_error_invalid

  payload='{"jsonrpc":"2.0","id":"tools-list","method":"tools/list"}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status tool_discovery "$status" 200 tool_discovery_succeeded
  jq -e '[.result.tools[].name] == ["runtime_status","platform_info","android_status","project_service_status","create_directory","copy_file","list_directory","path_metadata","read_file","search_text","write_file"]' "$body" >/dev/null 2>&1 || fail tool_allowlist_mismatch
  jq -e '
    .result.tools
    | map(select(.name == "create_directory"))[0] as $tool
    | ($tool.inputSchema.properties.dry_run | has("const") | not)
      and ($tool.inputSchema.additionalProperties == false)
      and ($tool.description | contains("MCP-Capability-Grant"))
  ' "$body" >/dev/null 2>&1 || fail create_directory_grant_discovery_invalid
  record_result runtime tool_allowlist pass exact_tool_allowlist

  payload='{"jsonrpc":"2.0","id":"runtime","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status high_impact_gates "$status" 200 high_impact_status_read
  jq -e '
    .result.structuredContent.commandExecution == false
    and .result.structuredContent.androidPlatformTools == false
    and .result.structuredContent.highImpactTools == false
    and .result.structuredContent.createDirectoryMutationEnabled == true
    and .result.structuredContent.createDirectoryGrantRequired == true
    and .result.structuredContent.createDirectoryGrantHeader == "mcp-capability-grant"
    and .result.structuredContent.createDirectoryGrantTtlSeconds == 60
    and .result.structuredContent.createDirectoryMutationMode == "dry_run_or_request_scoped_single_use_grant"
  ' "$body" >/dev/null 2>&1 || fail high_impact_gate_enabled

  payload='{"jsonrpc":"2.0","id":"platform","method":"tools/call","params":{"name":"platform_info","arguments":{}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status platform_info "$status" 200 platform_info_read
  jq -e --arg version "$EXPECTED_VERSION" '
    .result.structuredContent as $value
    | ($value | keys) == ["arch","available_parallelism","family","os","package_version"]
      and $value.os == "android"
      and $value.arch == "aarch64"
      and $value.family == "unix"
      and $value.available_parallelism >= 1
      and $value.package_version == $version
  ' "$body" >/dev/null 2>&1 || fail platform_info_contract_invalid

  payload='{"jsonrpc":"2.0","id":"android","method":"tools/call","params":{"name":"android_status","arguments":{}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status android_status "$status" 200 android_status_read
  jq -e --arg version "$EXPECTED_VERSION" '
    .result.structuredContent as $value
    | ($value | keys) == ["android_api_access","android_control_enabled","command_execution_enabled","high_impact_controls_enabled","package_version","shell_fallback_enabled","status_mode","target_arch","target_family","target_os","termux_runtime_hint"]
      and $value.status_mode == "read_only_allowlisted_status"
      and $value.target_os == "android"
      and $value.target_arch == "aarch64"
      and $value.package_version == $version
      and $value.android_api_access == "not_used"
      and $value.android_control_enabled == false
      and $value.shell_fallback_enabled == false
      and $value.command_execution_enabled == false
      and $value.high_impact_controls_enabled == false
  ' "$body" >/dev/null 2>&1 || fail android_status_contract_invalid

  payload='{"jsonrpc":"2.0","id":"service","method":"tools/call","params":{"name":"project_service_status","arguments":{"service_name":"mcp_runtime"}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status project_service_status "$status" 200 project_service_status_read
  jq -e '
    .result.structuredContent as $value
    | ($value | keys) == ["command_execution_enabled","command_line_exposed","environment_exposed","health","lifecycle_state","mutation_enabled","ownership","pid_inspection_enabled","process_listing_enabled","service_name","status_mode"]
      and $value.service_name == "mcp_runtime"
      and $value.ownership == "project_owned_allowlisted"
      and $value.status_mode == "read_only_project_service_status"
      and $value.pid_inspection_enabled == false
      and $value.process_listing_enabled == false
      and $value.command_line_exposed == false
      and $value.environment_exposed == false
      and $value.command_execution_enabled == false
      and $value.mutation_enabled == false
  ' "$body" >/dev/null 2>&1 || fail project_service_status_contract_invalid
  bytes="$(wc -c <"$body" 2>/dev/null)" || fail metadata_response_size_failed
  ((bytes <= 65536)) || fail metadata_response_too_large

  payload='{"jsonrpc":"2.0","id":"service-denied","method":"tools/call","params":{"name":"project_service_status","arguments":{"service_name":"ssh"}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status project_service_allowlist "$status" 400 unsupported_project_service_rejected
  jq -e '.error.code == -32602' "$body" >/dev/null 2>&1 || fail project_service_allowlist_body_invalid
  record_result runtime metadata_tools pass read_only_metadata_verified

  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT" '{"jsonrpc":"2.0","id":"list","method":"tools/call","params":{"name":"list_directory","arguments":{"path":$path,"max_depth":1}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status list_directory "$status" 200 list_directory_succeeded
  status="$(mcp_post "$second" "$payload" "$MCP_SESSION_ID")"
  expect_status list_directory_repeat "$status" 200 list_directory_repeat_succeeded
  cmp -s "$body" "$second" || fail list_response_not_deterministic
  bytes="$(wc -c <"$body")"
  ((bytes <= MAX_LIST_RESPONSE_BYTES)) || fail list_response_too_large
  jq -e '
    .result.structuredContent as $listing
    | ($listing.entries | length) == 65
      and $listing.truncated == false
      and $listing.maxEntries == 4096
      and $listing.maxResponseBytes == 262144
      and ([$listing.entries[].path] == ([$listing.entries[].path] | sort))
  ' "$body" >/dev/null 2>&1 || fail list_response_contract_invalid
  record_result runtime list_contract pass deterministic_bounded_list

  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT/visible.txt" '{"jsonrpc":"2.0","id":"path-metadata","method":"tools/call","params":{"name":"path_metadata","arguments":{"path":$path}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status path_metadata "$status" 200 safe_root_path_metadata_succeeded
  bytes="$(wc -c <"$body" 2>/dev/null)" || fail path_metadata_response_size_failed
  ((bytes <= 16384)) || fail path_metadata_response_too_large
  jq -e --arg path "$VALIDATION_SAFE_ROOT/visible.txt" '
    .result.structuredContent as $metadata
    | ($metadata | keys) == ["kind","maxResponseBytes","modified","path","sizeBytes"]
      and $metadata.path == $path
      and $metadata.kind == "regular_file"
      and $metadata.sizeBytes == 18
      and ($metadata.modified | type) == "string"
      and $metadata.maxResponseBytes == 16384
  ' "$body" >/dev/null 2>&1 || fail path_metadata_contract_invalid
  grep -Eq 'inode|device|uid|gid|mode|accessTime|validation-visible' "$body" && fail path_metadata_sensitive_field_reflected

  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT/visible.txt" '{"jsonrpc":"2.0","id":"read","method":"tools/call","params":{"name":"read_file","arguments":{"path":$path}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status read_file "$status" 200 safe_root_read_succeeded
  bytes="$(wc -c <"$body" 2>/dev/null)" || fail read_response_size_failed
  ((bytes <= 65536)) || fail read_response_too_large
  jq -e '.result.structuredContent.content == "validation-visible"' "$body" >/dev/null 2>&1 || fail read_content_invalid

  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT" --arg query validation-visible '{"jsonrpc":"2.0","id":"search","method":"tools/call","params":{"name":"search_text","arguments":{"path":$path,"query":$query,"max_depth":1}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status search_text "$status" 200 safe_root_text_search_succeeded
  bytes="$(wc -c <"$body" 2>/dev/null)" || fail search_response_size_failed
  ((bytes <= 262144)) || fail search_response_too_large
  jq -e --arg path "$VALIDATION_SAFE_ROOT/visible.txt" '
    .result.structuredContent as $search
    | $search.matches == [{"path":$path,"lineNumber":1,"columnByte":1}]
      and $search.truncated == false
      and $search.queryBytes == 18
      and $search.maxDepth == 1
      and $search.maxEntries == 8192
      and $search.maxFiles == 4096
      and $search.maxFileBytes == 1048576
      and $search.maxTotalBytes == 8388608
      and $search.maxMatches == 256
      and $search.maxResponseBytes == 262144
  ' "$body" >/dev/null 2>&1 || fail search_response_contract_invalid
  grep -Fq validation-visible "$body" && fail search_query_or_content_reflected

  directory_target="$VALIDATION_SAFE_ROOT/created-directory"
  mismatch_target="$VALIDATION_SAFE_ROOT/create-directory-mismatch"
  payload="$(jq -cn --arg path "$directory_target" '{"jsonrpc":"2.0","id":"create-directory-missing-grant","method":"tools/call","params":{"name":"create_directory","arguments":{"path":$path,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status create_directory_missing_grant "$status" 403 create_directory_missing_grant_rejected
  jq -e '.error.code == -32003 and .error.data.reason == "capability_grant_missing"' "$body" >/dev/null 2>&1 || fail create_directory_missing_grant_body_invalid
  [[ ! -e "$directory_target" ]] || fail create_directory_missing_grant_mutated

  issue_create_directory_grant "$directory_target"

  payload='{"jsonrpc":"2.0","id":"grant-wrong-context","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$CAPABILITY_GRANT_FILE")"
  expect_status create_directory_grant_context "$status" 400 create_directory_grant_wrong_context_rejected
  jq -e '.error.code == -32600' "$body" >/dev/null 2>&1 || fail create_directory_grant_context_body_invalid

  payload="$(jq -cn --arg path "$mismatch_target" '{"jsonrpc":"2.0","id":"create-directory-mismatch","method":"tools/call","params":{"name":"create_directory","arguments":{"path":$path,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$CAPABILITY_GRANT_FILE")"
  expect_status create_directory_grant_binding "$status" 403 create_directory_grant_binding_rejected
  jq -e '.error.code == -32003 and .error.data.reason == "capability_grant_binding_mismatch"' "$body" >/dev/null 2>&1 || fail create_directory_grant_binding_body_invalid
  [[ ! -e "$mismatch_target" ]] || fail create_directory_grant_binding_mutated

  payload="$(jq -cn --arg path "$directory_target" '{"jsonrpc":"2.0","id":"create-directory-dry","method":"tools/call","params":{"name":"create_directory","arguments":{"path":$path}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$CAPABILITY_GRANT_FILE")"
  expect_status create_directory_dry_run "$status" 200 create_directory_dry_run_succeeded
  jq -e --arg path "$directory_target" '
    .result.structuredContent == {
      path:$path,
      dryRun:true,
      mode:"0700",
      maxResponseBytes:16384
    }
  ' "$body" >/dev/null 2>&1 || fail create_directory_dry_run_contract_invalid
  [[ ! -e "$directory_target" ]] || fail create_directory_dry_run_mutated

  payload="$(jq -cn --arg path "$directory_target" '{"jsonrpc":"2.0","id":"create-directory","method":"tools/call","params":{"name":"create_directory","arguments":{"path":$path,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$CAPABILITY_GRANT_FILE")"
  expect_status create_directory "$status" 200 create_directory_succeeded
  jq -e --arg path "$directory_target" '
    .result.structuredContent == {
      path:$path,
      dryRun:false,
      mode:"0700",
      maxResponseBytes:16384
    }
  ' "$body" >/dev/null 2>&1 || fail create_directory_contract_invalid
  [[ -d "$directory_target" ]] || fail create_directory_target_missing
  [[ "$(stat -c '%a' "$directory_target" 2>/dev/null)" == 700 ]] || fail create_directory_mode_invalid
  record_result runtime create_directory pass safe_root_directory_creation_verified

  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$CAPABILITY_GRANT_FILE")"
  expect_status create_directory_existing "$status" 400 create_directory_existing_rejected
  jq -e '.error.code == -32602' "$body" >/dev/null 2>&1 || fail create_directory_existing_body_invalid

  rmdir -- "$directory_target" 2>/dev/null || fail create_directory_replay_fixture_cleanup_failed
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$CAPABILITY_GRANT_FILE")"
  expect_status create_directory_replay "$status" 403 create_directory_grant_replay_rejected
  jq -e '.error.code == -32003 and .error.data.reason == "capability_grant_replayed"' "$body" >/dev/null 2>&1 || fail create_directory_replay_body_invalid
  [[ ! -e "$directory_target" ]] || fail create_directory_replay_mutated
  record_result runtime create_directory_grant pass request_scoped_single_use_grant_enforced

  printf 'validation-copy\000\377binary' >"$VALIDATION_SAFE_ROOT/copy-source.bin" 2>/dev/null || fail copy_file_fixture_create_failed
  chmod 777 "$VALIDATION_SAFE_ROOT/copy-source.bin" 2>/dev/null || fail copy_file_fixture_create_failed
  payload="$(jq -cn --arg source "$VALIDATION_SAFE_ROOT/copy-source.bin" --arg destination "$VALIDATION_SAFE_ROOT/copy-dry.bin" '{"jsonrpc":"2.0","id":"copy-dry","method":"tools/call","params":{"name":"copy_file","arguments":{"source_path":$source,"destination_path":$destination}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status copy_file_dry_run "$status" 200 copy_file_dry_run_succeeded
  jq -e --arg source "$VALIDATION_SAFE_ROOT/copy-source.bin" --arg destination "$VALIDATION_SAFE_ROOT/copy-dry.bin" '
    .result.structuredContent == {
      sourcePath:$source,
      destinationPath:$destination,
      dryRun:true,
      sizeBytes:23,
      mode:"0600",
      maxFileBytes:1048576,
      maxResponseBytes:16384
    }
  ' "$body" >/dev/null 2>&1 || fail copy_file_dry_run_contract_invalid
  grep -Fq validation-copy "$body" && fail copy_file_content_reflected
  [[ ! -e "$VALIDATION_SAFE_ROOT/copy-dry.bin" ]] || fail copy_file_dry_run_mutated

  payload="$(jq -cn --arg source "$VALIDATION_SAFE_ROOT/copy-source.bin" --arg destination "$VALIDATION_SAFE_ROOT/copy.bin" '{"jsonrpc":"2.0","id":"copy","method":"tools/call","params":{"name":"copy_file","arguments":{"source_path":$source,"destination_path":$destination,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status copy_file_explicit "$status" 200 copy_file_explicit_succeeded
  jq -e --arg source "$VALIDATION_SAFE_ROOT/copy-source.bin" --arg destination "$VALIDATION_SAFE_ROOT/copy.bin" '
    .result.structuredContent == {
      sourcePath:$source,
      destinationPath:$destination,
      dryRun:false,
      sizeBytes:23,
      mode:"0600",
      maxFileBytes:1048576,
      maxResponseBytes:16384
    }
  ' "$body" >/dev/null 2>&1 || fail copy_file_contract_invalid
  grep -Fq validation-copy "$body" && fail copy_file_content_reflected
  cmp -s "$VALIDATION_SAFE_ROOT/copy-source.bin" "$VALIDATION_SAFE_ROOT/copy.bin" || fail copy_file_content_invalid
  [[ "$(stat -c '%a' "$VALIDATION_SAFE_ROOT/copy.bin" 2>/dev/null)" == 600 ]] || fail copy_file_mode_invalid

  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status copy_file_existing "$status" 400 copy_file_existing_rejected
  jq -e '.error.code == -32602' "$body" >/dev/null 2>&1 || fail copy_file_existing_body_invalid
  cmp -s "$VALIDATION_SAFE_ROOT/copy-source.bin" "$VALIDATION_SAFE_ROOT/copy.bin" || fail copy_file_existing_modified

  ln -s -- "$VALIDATION_SAFE_ROOT/copy-source.bin" "$VALIDATION_SAFE_ROOT/copy-source-link" 2>/dev/null || fail copy_file_symlink_fixture_create_failed
  payload="$(jq -cn --arg source "$VALIDATION_SAFE_ROOT/copy-source-link" --arg destination "$VALIDATION_SAFE_ROOT/copy-from-link.bin" '{"jsonrpc":"2.0","id":"copy-link","method":"tools/call","params":{"name":"copy_file","arguments":{"source_path":$source,"destination_path":$destination,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status copy_file_symlink "$status" 400 copy_file_symlink_rejected
  jq -e '.error.code == -32602' "$body" >/dev/null 2>&1 || fail copy_file_symlink_body_invalid
  [[ ! -e "$VALIDATION_SAFE_ROOT/copy-from-link.bin" ]] || fail copy_file_symlink_mutated

  dd if=/dev/zero of="$VALIDATION_SAFE_ROOT/copy-oversized.bin" bs=1048577 count=1 status=none 2>/dev/null || fail copy_file_oversized_fixture_create_failed
  payload="$(jq -cn --arg source "$VALIDATION_SAFE_ROOT/copy-oversized.bin" --arg destination "$VALIDATION_SAFE_ROOT/copy-oversized-destination.bin" '{"jsonrpc":"2.0","id":"copy-oversized","method":"tools/call","params":{"name":"copy_file","arguments":{"source_path":$source,"destination_path":$destination,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status copy_file_oversized "$status" 413 copy_file_oversized_rejected
  jq -e '.error.code == -32001' "$body" >/dev/null 2>&1 || fail copy_file_oversized_body_invalid
  [[ ! -e "$VALIDATION_SAFE_ROOT/copy-oversized-destination.bin" ]] || fail copy_file_oversized_mutated
  record_result runtime copy_file pass safe_root_file_copy_verified

  dd if=/dev/zero of="$VALIDATION_SAFE_ROOT/expanded-response.bin" bs=200000 count=1 status=none 2>/dev/null || fail read_bound_fixture_create_failed
  chmod 600 "$VALIDATION_SAFE_ROOT/expanded-response.bin" 2>/dev/null || fail read_bound_fixture_create_failed
  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT/expanded-response.bin" '{"jsonrpc":"2.0","id":"read-expanded","method":"tools/call","params":{"name":"read_file","arguments":{"path":$path}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status read_response_bound "$status" 413 read_response_bound_enforced
  jq -e '.error.code == -32001' "$body" >/dev/null 2>&1 || fail read_response_bound_body_invalid
  bytes="$(wc -c <"$body" 2>/dev/null)" || fail read_response_size_failed
  ((bytes <= 65536)) || fail read_error_response_too_large
  grep -Fq "$VALIDATION_SAFE_ROOT" "$body" && fail read_error_path_reflected

  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT/write.txt" --arg content validation-write '{"jsonrpc":"2.0","id":"write-dry","method":"tools/call","params":{"name":"write_file","arguments":{"path":$path,"content":$content}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status write_dry_run "$status" 200 write_dry_run_succeeded
  jq -e '.result.structuredContent.dryRun == true' "$body" >/dev/null 2>&1 || fail write_dry_run_invalid
  [[ ! -e "$VALIDATION_SAFE_ROOT/write.txt" ]] || fail write_dry_run_mutated

  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT/write.txt" --arg content validation-write '{"jsonrpc":"2.0","id":"write","method":"tools/call","params":{"name":"write_file","arguments":{"path":$path,"content":$content,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status write_explicit "$status" 200 write_explicit_succeeded
  jq -e '.result.structuredContent.dryRun == false' "$body" >/dev/null 2>&1 || fail write_explicit_body_invalid
  [[ "$(stat -c '%a' "$VALIDATION_SAFE_ROOT/write.txt" 2>/dev/null)" == 600 ]] || fail write_mode_invalid
  [[ "$(<"$VALIDATION_SAFE_ROOT/write.txt")" == validation-write ]] || fail write_content_invalid

  printf '%s' outside-private-content >"$TEMP_ROOT/outside.txt"
  payload="$(jq -cn --arg path "$TEMP_ROOT/outside.txt" '{"jsonrpc":"2.0","id":"outside","method":"tools/call","params":{"name":"read_file","arguments":{"path":$path}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status outside_safe_root "$status" 400 outside_safe_root_rejected
  jq -e '.error.code == -32602' "$body" >/dev/null 2>&1 || fail outside_error_invalid
  grep -Fq outside-private-content "$body" && fail outside_content_reflected
  grep -Fq "$TEMP_ROOT" "$body" && fail outside_path_reflected

  ln -s -- "$TEMP_ROOT/outside.txt" "$VALIDATION_SAFE_ROOT/escape-link" 2>/dev/null || fail symlink_fixture_create_failed
  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT/escape-link" '{"jsonrpc":"2.0","id":"symlink-escape","method":"tools/call","params":{"name":"read_file","arguments":{"path":$path}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status symlink_escape "$status" 400 symlink_escape_rejected
  jq -e '.error.code == -32602' "$body" >/dev/null 2>&1 || fail symlink_escape_body_invalid
  grep -Fq outside-private-content "$body" && fail symlink_escape_content_reflected
  grep -Fq "$VALIDATION_SAFE_ROOT" "$body" && fail symlink_escape_path_reflected

  payload='{"jsonrpc":"2.0","id":"shell","method":"tools/call","params":{"name":"shell","arguments":{}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status forbidden_high_impact "$status" 501 high_impact_tool_unavailable
  jq -e '.error.code == -32601' "$body" >/dev/null 2>&1 || fail high_impact_error_invalid

  oversized="$(printf '%*s' 1500 '' | tr ' ' x)"
  payload="$(jq -cn --arg content "$oversized" '{"jsonrpc":"2.0","id":"oversized","method":"tools/call","params":{"name":"write_file","arguments":{"path":"/ignored","content":$content}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status authenticated_body_limit "$status" 413 authenticated_body_limit_enforced
  jq -e '.error == "mcp_request_body_too_large"' "$body" >/dev/null 2>&1 || fail body_limit_error_invalid
  stage_request "$payload"
  status="$(curl_local -sS -o "$body" -w '%{http_code}' \
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
    --data-binary "@$REQUEST_FILE" "http://$BIND_HOST:$PORT/mcp")"
  expect_status authentication_ordering "$status" 401 authentication_precedes_body_limit
  jq -e '.error == "unauthorized"' "$body" >/dev/null 2>&1 || fail authentication_order_body_invalid

  stage_session_headers "$MCP_SESSION_ID"
  status="$(curl_local -sS -o "$body" -w '%{http_code}' \
    -H "@$SESSION_HEADER_FILE" \
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
    -H 'Accept: text/event-stream' \
    "http://$BIND_HOST:$PORT/mcp")"
  expect_status non_sse_get "$status" 405 non_sse_get_documented
  [[ ! -s "$body" ]] || fail non_sse_get_body_present

  stage_session_headers "$MCP_SESSION_ID"
  status="$(curl_local -sS -X DELETE -o "$body" -w '%{http_code}' \
    -H "@$SESSION_HEADER_FILE" \
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
    "http://$BIND_HOST:$PORT/mcp")"
  expect_status session_delete "$status" 204 session_deleted
  [[ ! -s "$body" ]] || fail session_delete_body_present
  MCP_SESSION_ID=""
  stop_server || fail mcp_runtime_stop_failed
}

run_runtime_phase() {
  CURRENT_PHASE=runtime
  set_phase runtime running
  ((CONFIRM_RUNTIME == 1)) || fail runtime_confirmation_missing
  prepare_runtime_inputs
  run_default_runtime_checks
  run_mcp_runtime_checks
  record_result runtime cleanup info isolated_runtime_cleanup_armed
  set_phase runtime pass
}

link_equals() {
  local link="$1" expected="$2"
  [[ -L "$link" && "$(readlink "$link")" == "$expected" ]]
}

run_deploy_success() {
  local check="$1"
  shift
  if ! "$@" >"$TEMP_ROOT/$check.log" 2>&1; then
    fail "${check}_failed"
  fi
  record_result deployment "$check" pass "${check}_succeeded"
}

run_deploy_expected_failure() {
  local check="$1"
  shift
  if "$@" >"$TEMP_ROOT/$check.log" 2>&1; then
    fail "${check}_unexpected_success"
  fi
  record_result deployment "$check" pass "${check}_rejected_and_recovered"
}

prepare_dedicated_deployment() {
  ((CONFIRM_DEPLOYMENT == 1)) || fail deployment_confirmation_missing
  if [[ "$TEST_MODE" == 0 ]]; then
    require_termux_aarch64_environment termux_environment_invalid
  else
    [[ -n "${PREFIX:-}" && "$PREFIX" == /* ]] || fail termux_prefix_invalid
  fi
  [[ -f "$DEPLOY_SCRIPT" && ! -L "$DEPLOY_SCRIPT" ]] || fail deploy_script_invalid
  bash -n "$DEPLOY_SCRIPT" || fail deploy_script_invalid
  DEDICATED_DEPLOY_ROOT="$HOME/.local/share/termux-mcp-release-validation-$RUN_ID"
  DEDICATED_CONFIG_ROOT="$HOME/.config/termux-mcp-release-validation-$RUN_ID"
  DEDICATED_SERVICE_ROOT="$PREFIX/var/service-termux-mcp-release-validation-$RUN_ID"
  for path in "$DEDICATED_DEPLOY_ROOT" "$DEDICATED_CONFIG_ROOT" "$DEDICATED_SERVICE_ROOT"; do
    [[ ! -e "$path" && ! -L "$path" ]] || fail dedicated_deployment_path_exists
  done
  mkdir -p "$DEDICATED_CONFIG_ROOT" "$DEDICATED_SERVICE_ROOT" 2>/dev/null || fail dedicated_layout_create_failed
  chmod 700 "$DEDICATED_CONFIG_ROOT" "$DEDICATED_SERVICE_ROOT" 2>/dev/null || fail dedicated_layout_create_failed
  if ! cat >"$DEDICATED_CONFIG_ROOT/runtime.env" 2>/dev/null <<EOF
MCP__AUTH__STATIC_TOKEN=$MCP_TOKEN
MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=false
MCP__SERVER__HOST=$BIND_HOST
MCP__SERVER__PORT=$PORT
MCP__TRANSPORT__ALLOWED_HOSTS=localhost:$PORT,127.0.0.1:$PORT
MCP__TRANSPORT__ALLOWED_ORIGINS=http://localhost:$PORT,http://127.0.0.1:$PORT
MCP__TRANSPORT__ALLOW_MISSING_ORIGIN=false
MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS=4
MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS=30
MCP__TRANSPORT__MAX_BODY_BYTES=1024
MCP__FILE__SAFE_ROOTS=$VALIDATION_SAFE_ROOT
RUST_LOG=termux_mcp_server=info
EOF
  then
    fail dedicated_config_write_failed
  fi
  chmod 600 "$DEDICATED_CONFIG_ROOT/runtime.env" 2>/dev/null || fail dedicated_config_write_failed
  export TERMUX_MCP_DEPLOY_ROOT="$DEDICATED_DEPLOY_ROOT"
  export TERMUX_MCP_CONFIG_ROOT="$DEDICATED_CONFIG_ROOT"
  export TERMUX_MCP_SERVICE_ROOT="$DEDICATED_SERVICE_ROOT"
  export TERMUX_MCP_SERVICE_SHELL="$PREFIX/bin/sh"
  export TERMUX_MCP_HEALTH_URL="http://$BIND_HOST:$PORT/health"
  export TERMUX_MCP_READY_URL="http://$BIND_HOST:$PORT/ready"
  export TERMUX_MCP_PROBE_ATTEMPTS=5
  export TERMUX_MCP_PROBE_DELAY_SECONDS=1
  export TERMUX_MCP_STOP_ATTEMPTS=20
  export TERMUX_MCP_STOP_DELAY_SECONDS=1
  if [[ "$TEST_MODE" == 1 ]]; then
    export TERMUX_MCP_TEST_MODE=1
    export TERMUX_MCP_TEST_PROBE_SEQUENCE=success
    export TERMUX_MCP_TEST_STOP_SEQUENCE=success
    export TERMUX_MCP_TEST_START_SEQUENCE=success
  else
    require_command runsvdir
    require_command sv
    [[ "$(stat -c '%d' "$HOME")" == "$(stat -c '%d' "$DEDICATED_SERVICE_ROOT")" ]] || fail atomic_publication_filesystem_mismatch
    runsvdir "$DEDICATED_SERVICE_ROOT" >"$TEMP_ROOT/runsvdir.log" 2>&1 &
    RUNSVDIR_PID=$!
    sleep 1
    kill -0 "$RUNSVDIR_PID" >/dev/null 2>&1 || fail runsvdir_start_failed
  fi
}

run_dedicated_deployment_cycle() {
  prepare_runtime_inputs
  prepare_dedicated_deployment
  local baseline_release="$DEDICATED_DEPLOY_ROOT/releases/$BASELINE_VERSION"
  local candidate_release="$DEDICATED_DEPLOY_ROOT/releases/$EXPECTED_VERSION"

  run_deploy_success default_install_baseline \
    bash "$DEPLOY_SCRIPT" install --artifact "$BASELINE_PINNED_ARTIFACT" --version "$BASELINE_VERSION" --sha256 "$BASELINE_SHA256"
  link_equals "$DEDICATED_DEPLOY_ROOT/current" "$baseline_release" || fail default_install_state_invalid
  run_deploy_success default_upgrade_candidate \
    bash "$DEPLOY_SCRIPT" upgrade --artifact "$DEFAULT_PINNED_ARTIFACT" --version "$EXPECTED_VERSION" --sha256 "$DEFAULT_SHA256"
  link_equals "$DEDICATED_DEPLOY_ROOT/current" "$candidate_release" || fail default_upgrade_state_invalid
  link_equals "$DEDICATED_DEPLOY_ROOT/previous" "$baseline_release" || fail default_upgrade_previous_invalid
  run_deploy_success default_rollback_success bash "$DEPLOY_SCRIPT" rollback
  link_equals "$DEDICATED_DEPLOY_ROOT/current" "$baseline_release" || fail default_rollback_state_invalid
  run_deploy_success default_uninstall_success bash "$DEPLOY_SCRIPT" uninstall
  [[ ! -e "$DEDICATED_DEPLOY_ROOT" && ! -e "$DEDICATED_SERVICE_ROOT/mcp_runtime" ]] || fail default_uninstall_state_invalid
  [[ -f "$DEDICATED_CONFIG_ROOT/runtime.env" ]] || fail default_uninstall_config_not_preserved

  run_deploy_success install_baseline \
    bash "$DEPLOY_SCRIPT" install --artifact "$BASELINE_PINNED_ARTIFACT" --version "$BASELINE_VERSION" --sha256 "$BASELINE_SHA256"
  link_equals "$DEDICATED_DEPLOY_ROOT/current" "$baseline_release" || fail install_state_invalid
  if [[ "$TEST_MODE" == 1 && "${TERMUX_MCP_RELEASE_VALIDATOR_TEST_INTERRUPT_AFTER_INSTALL:-0}" == 1 ]]; then
    FAILURE_CODE=interrupted
    exit 130
  fi

  if [[ "$TEST_MODE" == 1 ]]; then
    run_deploy_expected_failure forced_candidate_failure \
      env TERMUX_MCP_TEST_PROBE_SEQUENCE=failure,success \
      bash "$DEPLOY_SCRIPT" upgrade --artifact "$MCP_PINNED_ARTIFACT" --version "$EXPECTED_VERSION" --sha256 "$MCP_SHA256"
  else
    local fake_dir="$TEMP_ROOT/fake-curl" count_file="$TEMP_ROOT/fake-curl-count" real_curl
    real_curl="$(command -v curl)"
    mkdir -p "$fake_dir"
    printf '#!%s\n' "$PREFIX/bin/sh" >"$fake_dir/curl"
    cat >>"$fake_dir/curl" <<'EOF'
: "${TERMUX_MCP_RELEASE_FAKE_COUNT:?}"
: "${TERMUX_MCP_RELEASE_REAL_CURL:?}"
count=0
[ ! -f "$TERMUX_MCP_RELEASE_FAKE_COUNT" ] || read -r count <"$TERMUX_MCP_RELEASE_FAKE_COUNT"
count=$((count + 1))
printf '%s\n' "$count" >"$TERMUX_MCP_RELEASE_FAKE_COUNT"
if [ "$count" -le 5 ]; then exit 22; fi
exec "$TERMUX_MCP_RELEASE_REAL_CURL" "$@"
EOF
    chmod 700 "$fake_dir/curl"
    printf '0\n' >"$count_file"
    run_deploy_expected_failure forced_candidate_failure \
      env PATH="$fake_dir:$PATH" TERMUX_MCP_RELEASE_FAKE_COUNT="$count_file" TERMUX_MCP_RELEASE_REAL_CURL="$real_curl" \
      bash "$DEPLOY_SCRIPT" upgrade --artifact "$MCP_PINNED_ARTIFACT" --version "$EXPECTED_VERSION" --sha256 "$MCP_SHA256"
  fi
  link_equals "$DEDICATED_DEPLOY_ROOT/current" "$baseline_release" || fail candidate_failure_recovery_invalid
  [[ ! -e "$candidate_release" ]] || fail failed_candidate_not_removed

  run_deploy_success upgrade_candidate \
    bash "$DEPLOY_SCRIPT" upgrade --artifact "$MCP_PINNED_ARTIFACT" --version "$EXPECTED_VERSION" --sha256 "$MCP_SHA256"
  link_equals "$DEDICATED_DEPLOY_ROOT/current" "$candidate_release" || fail upgrade_state_invalid
  link_equals "$DEDICATED_DEPLOY_ROOT/previous" "$baseline_release" || fail upgrade_previous_invalid

  if [[ "$TEST_MODE" == 1 ]]; then
    run_deploy_expected_failure rollback_failure_recovery \
      env TERMUX_MCP_TEST_PROBE_SEQUENCE=failure,success bash "$DEPLOY_SCRIPT" rollback
  else
    local fake_dir="$TEMP_ROOT/fake-curl" count_file="$TEMP_ROOT/fake-curl-count" real_curl
    real_curl="$(command -v curl)"
    printf '0\n' >"$count_file"
    run_deploy_expected_failure rollback_failure_recovery \
      env PATH="$fake_dir:$PATH" TERMUX_MCP_RELEASE_FAKE_COUNT="$count_file" TERMUX_MCP_RELEASE_REAL_CURL="$real_curl" \
      bash "$DEPLOY_SCRIPT" rollback
  fi
  link_equals "$DEDICATED_DEPLOY_ROOT/current" "$candidate_release" || fail rollback_failure_recovery_invalid

  run_deploy_success rollback_success bash "$DEPLOY_SCRIPT" rollback
  link_equals "$DEDICATED_DEPLOY_ROOT/current" "$baseline_release" || fail rollback_state_invalid
  run_deploy_success uninstall_success bash "$DEPLOY_SCRIPT" uninstall
  [[ ! -e "$DEDICATED_DEPLOY_ROOT" && ! -e "$DEDICATED_SERVICE_ROOT/mcp_runtime" ]] || fail uninstall_state_invalid
  [[ -f "$DEDICATED_CONFIG_ROOT/runtime.env" ]] || fail uninstall_config_not_preserved
  record_result deployment dedicated_cleanup info dedicated_cleanup_armed
}

run_production_action() {
  ((CONFIRM_DEPLOYMENT == 1)) || fail deployment_confirmation_missing
  local expected_confirmation="$PRODUCTION_CONFIRMATION_PREFIX-$PRODUCTION_ACTION"
  [[ "$PRODUCTION_CONFIRMATION" == "$expected_confirmation" ]] || fail production_confirmation_invalid
  require_termux_aarch64_environment production_termux_environment_required
  [[ -f "$DEPLOY_SCRIPT" && ! -L "$DEPLOY_SCRIPT" ]] || fail deploy_script_invalid
  unset TERMUX_MCP_DEPLOY_ROOT TERMUX_MCP_CONFIG_ROOT TERMUX_MCP_SERVICE_ROOT TERMUX_MCP_TEST_MODE
  case "$PRODUCTION_ACTION" in
    install|upgrade)
      run_deploy_success "production_$PRODUCTION_ACTION" \
        bash "$DEPLOY_SCRIPT" "$PRODUCTION_ACTION" --artifact "$MCP_PINNED_ARTIFACT" --version "$EXPECTED_VERSION" --sha256 "$MCP_SHA256"
      ;;
    upgrade-failure)
      local fake_dir="$TEMP_ROOT/production-fake-curl" count_file="$TEMP_ROOT/production-fake-curl-count" real_curl
      local production_root="$HOME/.local/share/termux-mcp-edge" current_before previous_before="" previous_present=0
      [[ -L "$production_root/current" ]] || fail production_upgrade_failure_requires_current
      current_before="$(readlink "$production_root/current" 2>/dev/null)" || fail production_state_invalid
      if [[ -L "$production_root/previous" ]]; then
        previous_before="$(readlink "$production_root/previous" 2>/dev/null)" || fail production_state_invalid
        previous_present=1
      elif [[ -e "$production_root/previous" ]]; then
        fail production_state_invalid
      fi
      [[ ! -e "$production_root/releases/$EXPECTED_VERSION" && ! -L "$production_root/releases/$EXPECTED_VERSION" ]] || fail production_candidate_release_exists
      real_curl="$(command -v curl)"
      mkdir -p "$fake_dir"
      printf '#!%s\n' "$PREFIX/bin/sh" >"$fake_dir/curl"
      cat >>"$fake_dir/curl" <<'EOF'
: "${TERMUX_MCP_RELEASE_FAKE_COUNT:?}"
: "${TERMUX_MCP_RELEASE_REAL_CURL:?}"
count=0
[ ! -f "$TERMUX_MCP_RELEASE_FAKE_COUNT" ] || read -r count <"$TERMUX_MCP_RELEASE_FAKE_COUNT"
count=$((count + 1))
printf '%s\n' "$count" >"$TERMUX_MCP_RELEASE_FAKE_COUNT"
if [ "$count" -le 5 ]; then exit 22; fi
exec "$TERMUX_MCP_RELEASE_REAL_CURL" "$@"
EOF
      chmod 700 "$fake_dir/curl"
      printf '0\n' >"$count_file"
      run_deploy_expected_failure production_upgrade_failure \
        env PATH="$fake_dir:$PATH" TERMUX_MCP_RELEASE_FAKE_COUNT="$count_file" TERMUX_MCP_RELEASE_REAL_CURL="$real_curl" \
        TERMUX_MCP_PROBE_ATTEMPTS=5 TERMUX_MCP_PROBE_DELAY_SECONDS=1 \
        bash "$DEPLOY_SCRIPT" upgrade --artifact "$MCP_PINNED_ARTIFACT" --version "$EXPECTED_VERSION" --sha256 "$MCP_SHA256"
      [[ -L "$production_root/current" && "$(readlink "$production_root/current" 2>/dev/null)" == "$current_before" ]] || fail production_upgrade_failure_recovery_invalid
      if ((previous_present == 1)); then
        [[ -L "$production_root/previous" && "$(readlink "$production_root/previous" 2>/dev/null)" == "$previous_before" ]] || fail production_upgrade_failure_recovery_invalid
      else
        [[ ! -e "$production_root/previous" && ! -L "$production_root/previous" ]] || fail production_upgrade_failure_recovery_invalid
      fi
      [[ ! -e "$production_root/releases/$EXPECTED_VERSION" && ! -L "$production_root/releases/$EXPECTED_VERSION" ]] || fail production_upgrade_failure_recovery_invalid
      record_result deployment production_upgrade_failure_recovery pass production_state_restored
      ;;
    rollback)
      run_deploy_success production_rollback bash "$DEPLOY_SCRIPT" rollback
      ;;
    uninstall)
      run_deploy_success production_uninstall bash "$DEPLOY_SCRIPT" uninstall
      ;;
  esac
}

run_deployment_phase() {
  CURRENT_PHASE=deployment
  set_phase deployment running
  ((CONFIRM_DEPLOYMENT == 1)) || fail deployment_confirmation_missing
  if [[ -n "$PRODUCTION_ACTION" ]]; then
    run_production_action
  else
    run_dedicated_deployment_cycle
  fi
  set_phase deployment pass
}

log "validator_version=$VALIDATOR_VERSION"
log "requested_phase=$PHASE"
log "fixture_mode=$([[ "$TEST_MODE" == 1 ]] && printf true || printf false)"

run_preflight
case "$PHASE" in
  preflight) ;;
  runtime) run_runtime_phase ;;
  deployment) run_deployment_phase ;;
  all)
    run_runtime_phase
    run_deployment_phase
    ;;
esac

if [[ "$SUSTAINED_OBSERVATION_STATUS" == fail ]]; then
  FAILURE_CODE=sustained_observation_failed
  exit 1
fi

COMPLETED=1
