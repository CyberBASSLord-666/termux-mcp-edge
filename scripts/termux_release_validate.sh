#!/data/data/com.termux/files/usr/bin/bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077
set +x

readonly VALIDATOR_VERSION="11"
readonly EVIDENCE_SCHEMA_VERSION=2
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
      preflight   Verify metadata and all downloaded release artifacts without starting a listener.
      runtime     Run preflight, then explicitly confirmed isolated runtime checks.
      deployment  Run preflight, then an explicitly confirmed deployment exercise.
      all         Run preflight, isolated runtime checks, and deployment checks.
  --confirm-runtime-mutation
      Permit creation of a dedicated temporary directory below the configured safe root,
      direct candidate process startup, and bounded isolated filesystem mutations,
      including request-granted reversible file retention, inside that directory.
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
VOLUME_CONTROL_ARTIFACT=""
VOLUME_CONTROL_SHA256=""
VOLUME_CONTROL_MANIFEST=""
FULL_SUITE_ARTIFACT=""
FULL_SUITE_SHA256=""
FULL_SUITE_MANIFEST=""
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
COPY_CAPABILITY_GRANT_FILE=""
TRASH_CAPABILITY_GRANT_FILE=""
WRITE_CAPABILITY_GRANT_FILE=""
WRITE_CAPABILITY_CONTENT_FILE=""
CAPABILITY_RUNTIME_CONFIG_FILE=""
AUTH_HEADER_FILE=""
REQUEST_FILE=""
SESSION_HEADER_FILE=""
PINNED_ARTIFACT_ROOT=""
DEFAULT_PINNED_ARTIFACT=""
MCP_PINNED_ARTIFACT=""
VOLUME_CONTROL_PINNED_ARTIFACT=""
FULL_SUITE_PINNED_ARTIFACT=""
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

for command_name in bash awk curl date dd dirname env file grep install jq ln mktemp mkdir mv readlink realpath rm rmdir sha256sum sort stat timeout uname wc cmp chmod kill seq sleep tr; do
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
    VOLUME_CONTROL_ARTIFACT) VOLUME_CONTROL_ARTIFACT="$value" ;;
    VOLUME_CONTROL_SHA256) VOLUME_CONTROL_SHA256="$value" ;;
    VOLUME_CONTROL_MANIFEST) VOLUME_CONTROL_MANIFEST="$value" ;;
    FULL_SUITE_ARTIFACT) FULL_SUITE_ARTIFACT="$value" ;;
    FULL_SUITE_SHA256) FULL_SUITE_SHA256="$value" ;;
    FULL_SUITE_MANIFEST) FULL_SUITE_MANIFEST="$value" ;;
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
[[ "$DEFAULT_SHA256" =~ ^[0-9a-f]{64}$ \
  && "$MCP_SHA256" =~ ^[0-9a-f]{64}$ \
  && "$VOLUME_CONTROL_SHA256" =~ ^[0-9a-f]{64}$ \
  && "$FULL_SUITE_SHA256" =~ ^[0-9a-f]{64}$ ]] \
  || raw_fail "artifact_digest_metadata_invalid"
declare -A CANDIDATE_DIGESTS=()
for candidate_digest in "$DEFAULT_SHA256" "$MCP_SHA256" "$VOLUME_CONTROL_SHA256" "$FULL_SUITE_SHA256"; do
  [[ -z "${CANDIDATE_DIGESTS[$candidate_digest]+present}" ]] \
    || raw_fail "artifact_posture_digests_not_distinct"
  CANDIDATE_DIGESTS["$candidate_digest"]=1
done
unset candidate_digest CANDIDATE_DIGESTS
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
  --arg volume_control_sha "$VOLUME_CONTROL_SHA256" \
  --arg full_suite_sha "$FULL_SUITE_SHA256" \
  --arg production_action "$PRODUCTION_ACTION" \
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
      androidVolumeControl: {sha256: $volume_control_sha, bytes: null, version: null, elf: null},
      fullSuite: {sha256: $full_suite_sha, bytes: null, version: null, elf: null},
      baseline: null
    },
    deploymentCandidate: {
      posture: "full-suite",
      productionAction: (if $production_action == "" then null else $production_action end)
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
  local bytes actual_sha identity reported_version pinned_directory pinned_artifact pinned_bytes pinned_sha
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
  pinned_directory="$PINNED_ARTIFACT_ROOT/$posture"
  [[ ! -e "$pinned_directory" && ! -L "$pinned_directory" ]] || fail artifact_pinning_failed
  mkdir -m 700 -- "$pinned_directory" 2>/dev/null || fail artifact_pinning_failed
  pinned_artifact="$pinned_directory/termux-mcp-server"
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
    android_volume_control) validate_artifact_manifest "$posture" "$VOLUME_CONTROL_MANIFEST" "$actual_sha" "$bytes" "$expected_version" ;;
    full_suite) validate_artifact_manifest "$posture" "$FULL_SUITE_MANIFEST" "$actual_sha" "$bytes" "$expected_version" ;;
  esac
  reported_version="$(timeout -k 2 5 "$pinned_artifact" --version 2>/dev/null | awk 'NR==1 {print $NF}')" || fail "${posture}_artifact_version_failed"
  [[ "$reported_version" == "$expected_version" ]] || fail "${posture}_artifact_version_mismatch"
  case "$posture" in
    default) DEFAULT_PINNED_ARTIFACT="$pinned_artifact" ;;
    mcp_runtime) MCP_PINNED_ARTIFACT="$pinned_artifact" ;;
    android_volume_control) VOLUME_CONTROL_PINNED_ARTIFACT="$pinned_artifact" ;;
    full_suite) FULL_SUITE_PINNED_ARTIFACT="$pinned_artifact" ;;
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
    android_volume_control)
      manifest_posture=android-volume-control
      artifact_name=termux-mcp-server-aarch64-linux-android-android-volume-control
      ;;
    full_suite)
      manifest_posture=full-suite
      artifact_name=termux-mcp-server-aarch64-linux-android-full-suite
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
      and .features == (
        if $posture == "mcp-runtime" then ["mcp-runtime"]
        elif $posture == "android-volume-control" then ["android-volume-control"]
        elif $posture == "full-suite" then ["full-suite"]
        else []
        end
      )
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
  validate_artifact android_volume_control androidVolumeControl "$VOLUME_CONTROL_ARTIFACT" "$VOLUME_CONTROL_SHA256" "$EXPECTED_VERSION"
  validate_artifact full_suite fullSuite "$FULL_SUITE_ARTIFACT" "$FULL_SUITE_SHA256" "$EXPECTED_VERSION"
  local default_path mcp_path volume_control_path full_suite_path
  local default_manifest_path mcp_manifest_path volume_control_manifest_path full_suite_manifest_path
  local -A seen_candidate_paths=() seen_manifest_paths=()
  default_path="$(realpath -e "$DEFAULT_ARTIFACT" 2>/dev/null)"
  mcp_path="$(realpath -e "$MCP_ARTIFACT" 2>/dev/null)"
  volume_control_path="$(realpath -e "$VOLUME_CONTROL_ARTIFACT" 2>/dev/null)"
  full_suite_path="$(realpath -e "$FULL_SUITE_ARTIFACT" 2>/dev/null)"
  for candidate_path in "$default_path" "$mcp_path" "$volume_control_path" "$full_suite_path"; do
    [[ -n "$candidate_path" && -z "${seen_candidate_paths[$candidate_path]+present}" ]] \
      || fail artifact_postures_not_distinct
    seen_candidate_paths["$candidate_path"]=1
  done
  default_manifest_path="$(realpath -e "$DEFAULT_MANIFEST" 2>/dev/null)"
  mcp_manifest_path="$(realpath -e "$MCP_MANIFEST" 2>/dev/null)"
  volume_control_manifest_path="$(realpath -e "$VOLUME_CONTROL_MANIFEST" 2>/dev/null)"
  full_suite_manifest_path="$(realpath -e "$FULL_SUITE_MANIFEST" 2>/dev/null)"
  for candidate_manifest_path in "$default_manifest_path" "$mcp_manifest_path" "$volume_control_manifest_path" "$full_suite_manifest_path"; do
    [[ -n "$candidate_manifest_path" && -z "${seen_manifest_paths[$candidate_manifest_path]+present}" ]] \
      || fail artifact_manifests_not_distinct
    seen_manifest_paths["$candidate_manifest_path"]=1
  done
  unset candidate_path candidate_manifest_path
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
    COPY_CAPABILITY_GRANT_FILE="$TEMP_ROOT/copy-file-grant.txt"
    TRASH_CAPABILITY_GRANT_FILE="$TEMP_ROOT/trash-file-grant.txt"
    WRITE_CAPABILITY_GRANT_FILE="$TEMP_ROOT/write-file-grant.txt"
    WRITE_CAPABILITY_CONTENT_FILE="$TEMP_ROOT/write-file-content.txt"
    CAPABILITY_RUNTIME_CONFIG_FILE="$TEMP_ROOT/capability-runtime.env"
    printf 'Authorization: Bearer %s\n' "$MCP_TOKEN" >"$AUTH_HEADER_FILE" 2>/dev/null || fail private_request_staging_failed
    : >"$REQUEST_FILE" 2>/dev/null || fail private_request_staging_failed
    : >"$SESSION_HEADER_FILE" 2>/dev/null || fail private_request_staging_failed
    : >"$CAPABILITY_GRANT_FILE" 2>/dev/null || fail private_request_staging_failed
    : >"$COPY_CAPABILITY_GRANT_FILE" 2>/dev/null || fail private_request_staging_failed
    : >"$TRASH_CAPABILITY_GRANT_FILE" 2>/dev/null || fail private_request_staging_failed
    : >"$WRITE_CAPABILITY_GRANT_FILE" 2>/dev/null || fail private_request_staging_failed
    : >"$WRITE_CAPABILITY_CONTENT_FILE" 2>/dev/null || fail private_request_staging_failed
    : >"$CAPABILITY_RUNTIME_CONFIG_FILE" 2>/dev/null || fail private_request_staging_failed
    chmod 600 "$AUTH_HEADER_FILE" "$REQUEST_FILE" "$SESSION_HEADER_FILE" "$CAPABILITY_GRANT_FILE" "$COPY_CAPABILITY_GRANT_FILE" "$TRASH_CAPABILITY_GRANT_FILE" "$WRITE_CAPABILITY_GRANT_FILE" "$WRITE_CAPABILITY_CONTENT_FILE" "$CAPABILITY_RUNTIME_CONFIG_FILE" 2>/dev/null || fail private_request_staging_failed
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
  else
    VALIDATION_SAFE_ROOT="$SAFE_ROOT/.termux-mcp-release-validation-$RUN_ID"
    [[ ! -e "$VALIDATION_SAFE_ROOT" && ! -L "$VALIDATION_SAFE_ROOT" ]] || fail validation_safe_root_exists
    mkdir -m 700 -- "$VALIDATION_SAFE_ROOT" 2>/dev/null || fail validation_safe_root_create_failed
    printf '%s' validation-visible >"$VALIDATION_SAFE_ROOT/visible.txt" 2>/dev/null || fail validation_safe_root_write_failed
    chmod 600 "$VALIDATION_SAFE_ROOT/visible.txt" 2>/dev/null || fail validation_safe_root_write_failed
    local index
    for index in $(seq 1 64); do
      printf 'entry-%03d' "$index" >"$VALIDATION_SAFE_ROOT/entry-$(printf '%03d' "$index").txt" 2>/dev/null || fail validation_safe_root_write_failed
    done
  fi
  printf '%s\n' \
    "MCP__AUTH__STATIC_TOKEN=$MCP_TOKEN" \
    'MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=false' \
    "MCP__FILE__SAFE_ROOTS=$VALIDATION_SAFE_ROOT" \
    'MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=true' \
    'MCP__FILE__COPY_FILE_MUTATION_ENABLED=true' \
    'MCP__FILE__TRASH_FILE_MUTATION_ENABLED=true' \
    'MCP__FILE__WRITE_MUTATION_ENABLED=true' \
    "MCP__CAPABILITY__KEY_ID=$CAPABILITY_KEY_ID" \
    "MCP__CAPABILITY__HMAC_KEY_HEX=$CAPABILITY_KEY_HEX" \
    >"$CAPABILITY_RUNTIME_CONFIG_FILE" 2>/dev/null || fail capability_runtime_config_failed
  chmod 600 "$CAPABILITY_RUNTIME_CONFIG_FILE" 2>/dev/null || fail capability_runtime_config_failed
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

valid_capability_grant() {
  local grant="$1" prefix remainder payload signature
  prefix="v1.${CAPABILITY_KEY_ID}."
  [[ "$grant" == "$prefix"* ]] || return 1
  remainder="${grant#"$prefix"}"
  [[ "$remainder" == *.* ]] || return 1
  payload="${remainder%%.*}"
  signature="${remainder#*.}"
  [[ "$signature" != *.* ]] || return 1
  (((${#payload} == 130 || ${#payload} == 260) && ${#signature} == 64)) || return 1
  [[ "$payload$signature" != *[!0-9a-f]* ]]
}

capability_grant_has_signed_byte() {
  local grant="$1" expected_payload_hex_length="$2" byte_offset="$3" expected="$4" prefix remainder payload hex_offset
  valid_capability_grant "$grant" || return 1
  prefix="v1.${CAPABILITY_KEY_ID}."
  remainder="${grant#"$prefix"}"
  payload="${remainder%%.*}"
  [[ "${#payload}" == "$expected_payload_hex_length" ]] || return 1
  hex_offset=$((byte_offset * 2))
  ((${#payload} >= hex_offset + 2)) || return 1
  [[ "${payload:hex_offset:2}" == "$expected" ]]
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
      valid_capability_grant "$grant" || fail capability_grant_output_invalid
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
  if ! MCP__CAPABILITY__CONFIG_FILE="$CAPABILITY_RUNTIME_CONFIG_FILE" \
    MCP__CAPABILITY__SESSION_ID="$MCP_SESSION_ID" \
    MCP__CAPABILITY__CREATE_DIRECTORY_TARGET="$target" \
      "$MCP_PINNED_ARTIFACT" --issue-create-directory-grant >"$CAPABILITY_GRANT_FILE" 2>/dev/null
  then
    fail capability_grant_issue_failed
  fi
  [[ "$(wc -l <"$CAPABILITY_GRANT_FILE" 2>/dev/null)" == 1 ]] || fail capability_grant_output_invalid
  local grant
  grant="$(<"$CAPABILITY_GRANT_FILE")"
  valid_capability_grant "$grant" || fail capability_grant_output_invalid
  capability_grant_has_signed_byte "$grant" 260 64 01 || fail create_directory_capability_byte_invalid
  unset grant
}

issue_copy_file_grant() {
  local source="$1" destination="$2" grant
  : >"$COPY_CAPABILITY_GRANT_FILE" 2>/dev/null || fail copy_capability_grant_staging_failed
  chmod 600 "$COPY_CAPABILITY_GRANT_FILE" 2>/dev/null || fail copy_capability_grant_staging_failed
  if ! MCP__CAPABILITY__CONFIG_FILE="$CAPABILITY_RUNTIME_CONFIG_FILE" \
    MCP__CAPABILITY__SESSION_ID="$MCP_SESSION_ID" \
    MCP__CAPABILITY__COPY_FILE_SOURCE="$source" \
    MCP__CAPABILITY__COPY_FILE_DESTINATION="$destination" \
      "$MCP_PINNED_ARTIFACT" --issue-copy-file-grant >"$COPY_CAPABILITY_GRANT_FILE" 2>/dev/null
  then
    fail copy_capability_grant_issue_failed
  fi
  [[ "$(wc -l <"$COPY_CAPABILITY_GRANT_FILE" 2>/dev/null)" == 1 ]] || fail copy_capability_grant_output_invalid
  grant="$(<"$COPY_CAPABILITY_GRANT_FILE")"
  valid_capability_grant "$grant" || fail copy_capability_grant_output_invalid
  capability_grant_has_signed_byte "$grant" 130 16 04 || fail copy_file_capability_byte_invalid
  unset grant
}

issue_trash_file_grant() {
  local target="$1" grant
  : >"$TRASH_CAPABILITY_GRANT_FILE" 2>/dev/null || fail trash_capability_grant_staging_failed
  chmod 600 "$TRASH_CAPABILITY_GRANT_FILE" 2>/dev/null || fail trash_capability_grant_staging_failed
  if ! MCP__CAPABILITY__CONFIG_FILE="$CAPABILITY_RUNTIME_CONFIG_FILE" \
    MCP__CAPABILITY__SESSION_ID="$MCP_SESSION_ID" \
    MCP__CAPABILITY__TRASH_FILE_TARGET="$target" \
      "$MCP_PINNED_ARTIFACT" --issue-trash-file-grant >"$TRASH_CAPABILITY_GRANT_FILE" 2>/dev/null
  then
    fail trash_capability_grant_issue_failed
  fi
  [[ "$(wc -l <"$TRASH_CAPABILITY_GRANT_FILE" 2>/dev/null)" == 1 ]] || fail trash_capability_grant_output_invalid
  grant="$(<"$TRASH_CAPABILITY_GRANT_FILE")"
  valid_capability_grant "$grant" || fail trash_capability_grant_output_invalid
  capability_grant_has_signed_byte "$grant" 130 16 05 || fail trash_file_capability_byte_invalid
  unset grant
}

issue_write_file_grant() {
  local target="$1" content_file="$2" disposition="$3" grant
  validate_private_file "$content_file" write_capability_content_file_invalid
  : >"$WRITE_CAPABILITY_GRANT_FILE" 2>/dev/null || fail write_capability_grant_staging_failed
  chmod 600 "$WRITE_CAPABILITY_GRANT_FILE" 2>/dev/null || fail write_capability_grant_staging_failed
  if ! MCP__CAPABILITY__CONFIG_FILE="$CAPABILITY_RUNTIME_CONFIG_FILE" \
    MCP__CAPABILITY__SESSION_ID="$MCP_SESSION_ID" \
    MCP__CAPABILITY__WRITE_FILE_TARGET="$target" \
    MCP__CAPABILITY__WRITE_FILE_CONTENT_FILE="$content_file" \
    MCP__CAPABILITY__WRITE_FILE_DISPOSITION="$disposition" \
      "$MCP_PINNED_ARTIFACT" --issue-write-file-grant >"$WRITE_CAPABILITY_GRANT_FILE" 2>/dev/null
  then
    fail write_capability_grant_issue_failed
  fi
  [[ "$(wc -l <"$WRITE_CAPABILITY_GRANT_FILE" 2>/dev/null)" == 1 ]] || fail write_capability_grant_output_invalid
  grant="$(<"$WRITE_CAPABILITY_GRANT_FILE")"
  valid_capability_grant "$grant" || fail write_capability_grant_output_invalid
  capability_grant_has_signed_byte "$grant" 130 16 02 || fail write_capability_byte_invalid
  unset grant
}

inspect_write_file_recovery() {
  local expected_content="${1-}" expected_mode="${2-}" quarantine entry base mode size links residue
  local count=0 total_bytes=0 content_matches=0
  quarantine="$VALIDATION_SAFE_ROOT/.termux-mcp-write-quarantine"
  residue="$(find "$VALIDATION_SAFE_ROOT" -name '.termux-mcp-write-file-*.tmp' -print -quit 2>/dev/null)" \
    || fail write_legacy_staging_inspection_failed
  [[ -z "$residue" ]] || fail write_legacy_staging_residue_detected

  if [[ -e "$quarantine" || -L "$quarantine" ]]; then
    [[ -d "$quarantine" && ! -L "$quarantine" ]] || fail write_recovery_namespace_invalid
    [[ "$(stat -c '%a' "$quarantine" 2>/dev/null)" == 700 ]] || fail write_recovery_namespace_mode_invalid
    while IFS= read -r -d '' entry; do
      base="${entry##*/}"
      [[ "$base" =~ ^\.termux-mcp-write-artifact-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$ \
        && -f "$entry" && ! -L "$entry" ]] \
        || fail write_recovery_entry_invalid
      mode="$(stat -c '%a' "$entry" 2>/dev/null)" || fail write_recovery_entry_stat_failed
      size="$(stat -c '%s' "$entry" 2>/dev/null)" || fail write_recovery_entry_stat_failed
      links="$(stat -c '%h' "$entry" 2>/dev/null)" || fail write_recovery_entry_stat_failed
      [[ "$mode" =~ ^[0-7]{3,4}$ && "$size" =~ ^[0-9]+$ && "$links" == 1 ]] \
        || fail write_recovery_entry_contract_invalid
      ((size <= 1048576)) || fail write_recovery_entry_too_large
      ((count += 1, total_bytes += size))
      if [[ -n "$expected_content" && "$(<"$entry")" == "$expected_content" \
        && ( -z "$expected_mode" || "$mode" == "$expected_mode" ) ]]; then
        ((content_matches += 1))
      fi
    done < <(find "$quarantine" -mindepth 1 -maxdepth 1 -print0 2>/dev/null) \
      || fail write_recovery_namespace_inspection_failed
  fi

  ((count <= 32 && total_bytes <= 33554432)) || fail write_recovery_namespace_capacity_invalid
  WRITE_FILE_RECOVERY_COUNT="$count"
  WRITE_FILE_RECOVERY_CONTENT_MATCHES="$content_matches"
}

start_server() {
  local artifact="$1" posture="$2" max_body_bytes="${3:-1024}"
  local log_file="$TEMP_ROOT/server-$posture.log"
  local -a environment=(
    "MCP__AUTH__STATIC_TOKEN=$MCP_TOKEN"
    "MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=false"
    "MCP__SERVER__HOST=$BIND_HOST"
    "MCP__SERVER__PORT=$PORT"
    "MCP__TRANSPORT__ALLOWED_HOSTS=localhost:$PORT,127.0.0.1:$PORT"
    "MCP__TRANSPORT__ALLOWED_ORIGINS=http://localhost:$PORT,http://127.0.0.1:$PORT"
    "MCP__TRANSPORT__ALLOW_MISSING_ORIGIN=false"
    "MCP__TRANSPORT__SSE_ENABLED=false"
    "MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS=4"
    "MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS=30"
    "MCP__TRANSPORT__MAX_BODY_BYTES=$max_body_bytes"
    "MCP__FILE__SAFE_ROOTS=$VALIDATION_SAFE_ROOT"
    "MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=false"
    "MCP__FILE__COPY_FILE_MUTATION_ENABLED=false"
    "MCP__FILE__TRASH_FILE_MUTATION_ENABLED=false"
    "MCP__FILE__WRITE_MUTATION_ENABLED=false"
    "RUST_LOG=termux_mcp_server=info"
  )
  if [[ "$posture" == mcp ]]; then
    environment+=(
      "MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=true"
      "MCP__FILE__COPY_FILE_MUTATION_ENABLED=true"
      "MCP__FILE__TRASH_FILE_MUTATION_ENABLED=true"
      "MCP__FILE__WRITE_MUTATION_ENABLED=true"
      "MCP__CAPABILITY__KEY_ID=$CAPABILITY_KEY_ID"
      "MCP__CAPABILITY__HMAC_KEY_HEX=$CAPABILITY_KEY_HEX"
    )
  elif [[ "$posture" == full_suite_battery_only ]]; then
    environment+=("MCP__ANDROID__BATTERY_STATUS_ENABLED=true")
  elif [[ "$posture" == full_suite_volume_status_only ]]; then
    environment+=("MCP__ANDROID__VOLUME_STATUS_ENABLED=true")
  elif [[ "$posture" == full_suite_volume_control_only ]]; then
    environment+=(
      "MCP__ANDROID__VOLUME_CONTROL_ENABLED=true"
      "MCP__CAPABILITY__KEY_ID=$CAPABILITY_KEY_ID"
      "MCP__CAPABILITY__HMAC_KEY_HEX=$CAPABILITY_KEY_HEX"
    )
  elif [[ "$posture" == full_suite_command_only ]]; then
    environment+=("MCP__COMMAND__ENABLED=true")
  elif [[ "$posture" == full_suite_enabled ]]; then
    environment+=(
      "MCP__ANDROID__BATTERY_STATUS_ENABLED=true"
      "MCP__ANDROID__VOLUME_STATUS_ENABLED=true"
      "MCP__ANDROID__VOLUME_CONTROL_ENABLED=true"
      "MCP__COMMAND__ENABLED=true"
      "MCP__CAPABILITY__KEY_ID=$CAPABILITY_KEY_ID"
      "MCP__CAPABILITY__HMAC_KEY_HEX=$CAPABILITY_KEY_HEX"
    )
  fi
  if [[ "$TEST_MODE" == 1 && -n "${TERMUX_MCP_RELEASE_FIXTURE_VOLUME_STATE:-}" ]]; then
    environment+=("TERMUX_MCP_RELEASE_FIXTURE_VOLUME_STATE=$TERMUX_MCP_RELEASE_FIXTURE_VOLUME_STATE")
  fi
  if [[ "$TEST_MODE" == 1 && "$posture" == full_suite_enabled \
    && -n "${TERMUX_MCP_RELEASE_FIXTURE_VOLUME_FAULT:-}" ]]; then
    environment+=("TERMUX_MCP_RELEASE_FIXTURE_VOLUME_FAULT=$TERMUX_MCP_RELEASE_FIXTURE_VOLUME_FAULT")
  fi
  env -i \
    "HOME=$HOME" \
    "PREFIX=${PREFIX:-}" \
    "PATH=$PATH" \
    "${environment[@]}" \
    "$artifact" >"$log_file" 2>&1 &
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

mcp_post_staged() {
  local output="$1" session_id="${2:-}" include_protocol="${3:-1}" grant_file="${4:-}"
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

stage_write_file_request() {
  local identifier="$1" target="$2" content_file="$3" dry_run="$4"
  validate_private_file "$content_file" write_file_content_staging_invalid
  jq -cn \
    --arg identifier "$identifier" \
    --arg target "$target" \
    --rawfile content "$content_file" \
    --argjson dry_run "$dry_run" \
    '{jsonrpc:"2.0",id:$identifier,method:"tools/call",params:{name:"write_file",arguments:{path:$target,content:$content,dry_run:$dry_run}}}' \
    >"$REQUEST_FILE" 2>/dev/null || fail private_request_staging_failed
  chmod 600 "$REQUEST_FILE" 2>/dev/null || fail private_request_staging_failed
}

stage_write_file_request_with_id_file() {
  local identifier_file="$1" target="$2" content_file="$3"
  validate_private_file "$identifier_file" write_file_identifier_staging_invalid
  validate_private_file "$content_file" write_file_content_staging_invalid
  jq -cn \
    --rawfile identifier "$identifier_file" \
    --arg target "$target" \
    --rawfile content "$content_file" \
    '{jsonrpc:"2.0",id:$identifier,method:"tools/call",params:{name:"write_file",arguments:{path:$target,content:$content,dry_run:false}}}' \
    >"$REQUEST_FILE" 2>/dev/null || fail private_request_staging_failed
  chmod 600 "$REQUEST_FILE" 2>/dev/null || fail private_request_staging_failed
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
  local second="$TEMP_ROOT/mcp-second.json" status payload oversized bytes directory_target mismatch_target hash_digest
  local copy_source copy_target copy_mismatch_target copy_stale_source copy_stale_target
  local copy_oversized copy_retry_target copy_bytes copy_grant
  local trash_target trash_mismatch_target trash_oversized trash_exact_target trash_bytes
  local trash_quarantine trash_artifact trash_identity trash_digest trash_grant
  local replacement_content old_identity new_identity preflight_identity substitute_identity preserved_target
  local recovery_count_before recovery_count_after
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
    and .mcp_request_limits.sse_enabled == false
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
  jq -e '[.result.tools[].name] == ["runtime_status","platform_info","android_status","project_service_status","create_directory","copy_file","trash_file","find_paths","hash_file","list_directory","path_metadata","read_binary_file","read_binary_range","read_file","read_text_range","search_text","write_file"]' "$body" >/dev/null 2>&1 || fail tool_allowlist_mismatch
  jq -e '
    .result.tools
    | map(select(.name == "create_directory"))[0] as $tool
    | ($tool.inputSchema.properties.dry_run | has("const") | not)
      and ($tool.inputSchema.additionalProperties == false)
      and ($tool.description | contains("MCP-Capability-Grant"))
  ' "$body" >/dev/null 2>&1 || fail create_directory_grant_discovery_invalid
  jq -e '
    .result.tools
    | map(select(.name == "copy_file"))[0] as $tool
    | ($tool.inputSchema.properties.dry_run | has("const") | not)
      and ($tool.inputSchema.additionalProperties == false)
      and ($tool.description | contains("MCP-Capability-Grant"))
      and ($tool.description | contains("source-identity/content/destination-bound"))
  ' "$body" >/dev/null 2>&1 || fail copy_file_grant_discovery_invalid
  jq -e '
    .result.tools
    | map(select(.name == "trash_file"))[0] as $tool
    | ($tool.inputSchema.properties.dry_run | has("const") | not)
      and ($tool.inputSchema.additionalProperties == false)
      and ($tool.inputSchema.required == ["path"])
      and ($tool.description | contains("MCP-Capability-Grant"))
      and ($tool.description | contains("identity/content-bound"))
      and ($tool.description | contains("recovery"))
  ' "$body" >/dev/null 2>&1 || fail trash_file_grant_discovery_invalid
  jq -e '
    .result.tools
    | map(select(.name == "write_file"))[0] as $tool
    | ($tool.inputSchema.properties.dry_run | has("const") | not)
      and ($tool.inputSchema.additionalProperties == false)
      and ($tool.description | contains("MCP-Capability-Grant"))
      and ($tool.description | contains("target/content/disposition-bound"))
  ' "$body" >/dev/null 2>&1 || fail write_file_grant_discovery_invalid
  jq -e '
    .result.tools
    | map(select(.name == "find_paths"))[0].inputSchema as $schema
    | $schema.type == "object"
      and ($schema.properties | keys) == ["kind","max_depth","path","query"]
      and $schema.properties.path.type == "string"
      and $schema.properties.query.type == "string"
      and $schema.properties.query.minLength == 1
      and $schema.properties.query.maxLength == 256
      and $schema.properties.query."x-maxBytes" == 256
      and $schema.properties.kind.enum == ["any","regular_file","directory"]
      and $schema.properties.max_depth.minimum == 1
      and $schema.properties.max_depth.maximum == 5
      and $schema.required == ["path","query"]
      and $schema.additionalProperties == false
  ' "$body" >/dev/null 2>&1 || fail find_paths_discovery_schema_invalid
  jq -e '
    .result.tools
    | map(select(.name == "hash_file"))[0].inputSchema as $schema
    | $schema.type == "object"
      and ($schema.properties | keys) == ["path"]
      and $schema.properties.path.type == "string"
      and $schema.required == ["path"]
      and $schema.additionalProperties == false
  ' "$body" >/dev/null 2>&1 || fail hash_file_discovery_schema_invalid
  jq -e '
    .result.tools
    | map(select(.name == "read_binary_file"))[0].inputSchema as $schema
    | $schema.type == "object"
      and ($schema.properties | keys) == ["path"]
      and $schema.properties.path.type == "string"
      and $schema.required == ["path"]
      and $schema.additionalProperties == false
  ' "$body" >/dev/null 2>&1 || fail read_binary_file_discovery_schema_invalid
  jq -e '
    .result.tools
    | map(select(.name == "read_binary_range"))[0].inputSchema as $schema
    | $schema.type == "object"
      and ($schema.properties | keys) == ["length_bytes","offset_bytes","path"]
      and $schema.properties.path.type == "string"
      and $schema.properties.offset_bytes.type == "integer"
      and $schema.properties.offset_bytes.minimum == 0
      and $schema.properties.offset_bytes.maximum == 67108864
      and $schema.properties.length_bytes.type == "integer"
      and $schema.properties.length_bytes.minimum == 1
      and $schema.properties.length_bytes.maximum == 262144
      and $schema.required == ["path","offset_bytes","length_bytes"]
      and $schema.additionalProperties == false
  ' "$body" >/dev/null 2>&1 || fail read_binary_range_discovery_schema_invalid
  jq -e '
    .result.tools
    | map(select(.name == "read_text_range"))[0].inputSchema as $schema
    | $schema.type == "object"
      and ($schema.properties | keys) == ["max_bytes","offset_bytes","path"]
      and $schema.properties.path.type == "string"
      and $schema.properties.offset_bytes.type == "integer"
      and $schema.properties.offset_bytes.minimum == 0
      and $schema.properties.offset_bytes.maximum == 67108864
      and $schema.properties.max_bytes.type == "integer"
      and $schema.properties.max_bytes.minimum == 4
      and $schema.properties.max_bytes.maximum == 262144
      and $schema.required == ["path","offset_bytes","max_bytes"]
      and $schema.additionalProperties == false
  ' "$body" >/dev/null 2>&1 || fail read_text_range_discovery_schema_invalid
  record_result runtime tool_allowlist pass exact_tool_allowlist

  payload='{"jsonrpc":"2.0","id":"runtime","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status high_impact_gates "$status" 200 high_impact_status_read
  jq -e '
    .result.structuredContent.commandExecution == false
    and .result.structuredContent.androidPlatformTools == false
    and .result.structuredContent.highImpactTools == false
    and .result.structuredContent.serverSentEvents == false
    and .result.structuredContent.serverSentEventsMode == "disabled"
    and .result.structuredContent.sseMaxStreamsPerSession == 8
    and .result.structuredContent.sseMaxEventsPerStream == 2
    and .result.structuredContent.sseMaxEventDataBytes == 131072
    and .result.structuredContent.sseMaxReplayBytesPerSession == 262144
    and .result.structuredContent.sseMaxLastEventIdBytes == 64
    and .result.structuredContent.sseRetryMilliseconds == 1000
    and .result.structuredContent.createDirectoryMutationEnabled == true
    and .result.structuredContent.createDirectoryGrantRequired == true
    and .result.structuredContent.createDirectoryGrantHeader == "mcp-capability-grant"
    and .result.structuredContent.createDirectoryGrantTtlSeconds == 60
    and .result.structuredContent.createDirectoryMutationMode == "dry_run_or_request_scoped_single_use_grant"
    and .result.structuredContent.copyFileMutationEnabled == true
    and .result.structuredContent.copyFileMode == "dry_run_or_source_content_destination_scoped_single_use_grant"
    and .result.structuredContent.copyFileGrantRequired == true
    and .result.structuredContent.copyFileGrantHeader == "mcp-capability-grant"
    and .result.structuredContent.copyFileGrantTtlSeconds == 60
    and .result.structuredContent.copyFileGrantBinding == "source_root_path_identity_size_sha256_destination_root_path_absent_no_replace"
    and .result.structuredContent.copyFileMaxBytes == 1048576
    and .result.structuredContent.copyFileMaxResponseBytes == 16384
    and .result.structuredContent.copyFileResponsePosture == "path_free_bounded_metadata_only"
    and .result.structuredContent.trashFileMutationEnabled == true
    and .result.structuredContent.trashFileMode == "dry_run_or_identity_content_scoped_single_use_grant_with_recovery_retained"
    and .result.structuredContent.trashFileGrantRequired == true
    and .result.structuredContent.trashFileGrantHeader == "mcp-capability-grant"
    and .result.structuredContent.trashFileGrantTtlSeconds == 60
    and .result.structuredContent.trashFileGrantBinding == "root_path_single_link_identity_size_ctime_sha256_recovery_retained"
    and .result.structuredContent.trashFileMaxBytes == 1048576
    and .result.structuredContent.trashFileMaxResponseBytes == 16384
    and .result.structuredContent.trashFileQuarantineMaxArtifacts == 32
    and .result.structuredContent.trashFileQuarantineMaxBytes == 33554432
    and .result.structuredContent.trashFileResponsePosture == "path_and_artifact_free_bounded_metadata_only"
    and .result.structuredContent.fileWrites == true
    and .result.structuredContent.fileWriteMode == "dry_run_or_target_content_disposition_scoped_single_use_grant"
    and .result.structuredContent.fileWriteMutationEnabled == true
    and .result.structuredContent.fileWriteGrantRequired == true
    and .result.structuredContent.fileWriteGrantHeader == "mcp-capability-grant"
    and .result.structuredContent.fileWriteGrantTtlSeconds == 60
    and .result.structuredContent.fileWriteMaxBytes == 1048576
    and .result.structuredContent.fileWriteMaxResponseBytes == 16384
    and .result.structuredContent.pathDiscovery == true
    and .result.structuredContent.pathDiscoveryMatchMode == "case_sensitive_literal_basename"
    and .result.structuredContent.pathDiscoveryMaxDepth == 5
    and .result.structuredContent.pathDiscoveryMaxEntries == 8192
    and .result.structuredContent.pathDiscoveryMaxMatches == 512
    and .result.structuredContent.pathDiscoveryMaxQueryBytes == 256
    and .result.structuredContent.pathDiscoveryMaxResponseBytes == 262144
    and .result.structuredContent.binaryFileReads == true
    and .result.structuredContent.binaryFileReadEncoding == "base64"
    and .result.structuredContent.binaryFileReadMaxBytes == 1048576
    and .result.structuredContent.binaryFileReadMaxResponseBytes == 1507328
    and .result.structuredContent.binaryRangeReads == true
    and .result.structuredContent.binaryRangeReadEncoding == "base64"
    and .result.structuredContent.binaryRangeReadMaxFileBytes == 67108864
    and .result.structuredContent.binaryRangeReadMaxBytes == 262144
    and .result.structuredContent.binaryRangeReadMaxResponseBytes == 393216
    and .result.structuredContent.textRangeReads == true
    and .result.structuredContent.textRangeReadEncoding == "utf-8"
    and .result.structuredContent.textRangeReadMinBytes == 4
    and .result.structuredContent.textRangeReadMaxFileBytes == 67108864
    and .result.structuredContent.textRangeReadMaxBytes == 262144
    and .result.structuredContent.textRangeReadMaxResponseBytes == 1703936
    and .result.structuredContent.fileHashing == true
    and .result.structuredContent.fileHashAlgorithm == "sha256"
    and .result.structuredContent.fileHashMaxBytes == 16777216
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

  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT" --arg query visible '{"jsonrpc":"2.0","id":"find-paths","method":"tools/call","params":{"name":"find_paths","arguments":{"path":$path,"query":$query,"kind":"regular_file","max_depth":1}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status find_paths "$status" 200 safe_root_path_discovery_succeeded
  bytes="$(wc -c <"$body" 2>/dev/null)" || fail find_paths_response_size_failed
  ((bytes <= 262144)) || fail find_paths_response_too_large
  jq -e --arg path "$VALIDATION_SAFE_ROOT/visible.txt" '
    .result.structuredContent as $find
    | $find.matches == [{"path":$path,"kind":"regular_file"}]
      and $find.truncated == false
      and $find.queryBytes == 7
      and $find.kindFilter == "regular_file"
      and $find.maxDepth == 1
      and $find.maxEntries == 8192
      and $find.maxMatches == 512
      and $find.maxResponseBytes == 262144
  ' "$body" >/dev/null 2>&1 || fail find_paths_contract_invalid
  grep -Fq validation-visible "$body" && fail find_paths_content_reflected
  record_result runtime find_paths pass safe_root_path_discovery_verified

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

  hash_digest="$(sha256sum -- "$VALIDATION_SAFE_ROOT/visible.txt" | awk '{print $1}')" || fail hash_file_expected_digest_failed
  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT/visible.txt" '{"jsonrpc":"2.0","id":"hash","method":"tools/call","params":{"name":"hash_file","arguments":{"path":$path}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status hash_file "$status" 200 safe_root_file_hash_succeeded
  bytes="$(wc -c <"$body" 2>/dev/null)" || fail hash_file_response_size_failed
  ((bytes <= 16384)) || fail hash_file_response_too_large
  jq -e --arg digest "$hash_digest" '
    .result.structuredContent as $hash
    | ($hash | keys) == ["algorithm","digest","sizeBytes"]
      and $hash.algorithm == "sha256"
      and $hash.digest == $digest
      and $hash.sizeBytes == 18
  ' "$body" >/dev/null 2>&1 || fail hash_file_contract_invalid
  grep -Eq 'validation-visible|visible\.txt|/\.termux-mcp-release-validation-' "$body" && fail hash_file_path_or_content_reflected

  ln -s -- "$VALIDATION_SAFE_ROOT/visible.txt" "$VALIDATION_SAFE_ROOT/hash-link" 2>/dev/null || fail hash_file_symlink_fixture_create_failed
  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT/hash-link" '{"jsonrpc":"2.0","id":"hash-link","method":"tools/call","params":{"name":"hash_file","arguments":{"path":$path}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status hash_file_symlink "$status" 400 hash_file_symlink_rejected
  jq -e '.error.code == -32602' "$body" >/dev/null 2>&1 || fail hash_file_symlink_body_invalid
  grep -Fq "$VALIDATION_SAFE_ROOT" "$body" && fail hash_file_symlink_path_reflected
  rm -f -- "$VALIDATION_SAFE_ROOT/hash-link" || fail hash_file_symlink_fixture_cleanup_failed
  [[ ! -e "$VALIDATION_SAFE_ROOT/hash-link" && ! -L "$VALIDATION_SAFE_ROOT/hash-link" ]] || fail hash_file_symlink_fixture_cleanup_incomplete

  dd if=/dev/zero of="$VALIDATION_SAFE_ROOT/hash-oversized.bin" bs=1 seek=16777216 count=1 status=none 2>/dev/null || fail hash_file_oversized_fixture_create_failed
  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT/hash-oversized.bin" '{"jsonrpc":"2.0","id":"hash-oversized","method":"tools/call","params":{"name":"hash_file","arguments":{"path":$path}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status hash_file_oversized "$status" 413 hash_file_oversized_rejected
  jq -e '.error.code == -32001' "$body" >/dev/null 2>&1 || fail hash_file_oversized_body_invalid
  grep -Fq "$VALIDATION_SAFE_ROOT" "$body" && fail hash_file_oversized_path_reflected
  rm -f -- "$VALIDATION_SAFE_ROOT/hash-oversized.bin" || fail hash_file_oversized_fixture_cleanup_failed
  [[ ! -e "$VALIDATION_SAFE_ROOT/hash-oversized.bin" ]] || fail hash_file_oversized_fixture_cleanup_incomplete
  record_result runtime hash_file pass safe_root_file_hash_verified

  binary_read_target="$VALIDATION_SAFE_ROOT/binary-read.bin"
  printf '\000\377\200\141\012\001\376' >"$binary_read_target" || fail binary_read_fixture_create_failed
  binary_read_expected="$(base64 <"$binary_read_target" | tr -d '\n')" || fail binary_read_expected_encoding_failed
  payload="$(jq -cn --arg path "$binary_read_target" '{"jsonrpc":"2.0","id":"binary-read","method":"tools/call","params":{"name":"read_binary_file","arguments":{"path":$path}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status read_binary_file "$status" 200 safe_root_binary_read_succeeded
  bytes="$(wc -c <"$body" 2>/dev/null)" || fail binary_read_response_size_failed
  ((bytes <= 1507328)) || fail binary_read_response_too_large
  jq -e --arg data "$binary_read_expected" '
    .result.structuredContent as $binary
    | ($binary | keys) == ["data","encoding","maxFileBytes","maxResponseBytes","sizeBytes"]
      and $binary.encoding == "base64"
      and $binary.data == $data
      and $binary.sizeBytes == 7
      and $binary.maxFileBytes == 1048576
      and $binary.maxResponseBytes == 1507328
  ' "$body" >/dev/null 2>&1 || fail binary_read_contract_invalid
  grep -Eq 'binary-read\.bin|inode|device|uid|gid|mode|accessTime' "$body" && fail binary_read_path_or_metadata_reflected

  payload="$(jq -cn --arg path "$binary_read_target" '{"jsonrpc":"2.0","id":"binary-range","method":"tools/call","params":{"name":"read_binary_range","arguments":{"path":$path,"offset_bytes":2,"length_bytes":4}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status read_binary_range "$status" 200 safe_root_binary_range_read_succeeded
  bytes="$(wc -c <"$body" 2>/dev/null)" || fail binary_range_response_size_failed
  ((bytes <= 393216)) || fail binary_range_response_too_large
  jq -e '
    .result.structuredContent as $range
    | ($range | keys) == ["data","encoding","eof","fileSizeBytes","maxFileBytes","maxReadBytes","maxResponseBytes","offsetBytes","sizeBytes"]
      and $range.encoding == "base64"
      and $range.data == "gGEKAQ=="
      and $range.offsetBytes == 2
      and $range.sizeBytes == 4
      and $range.fileSizeBytes == 7
      and $range.eof == false
      and $range.maxReadBytes == 262144
      and $range.maxFileBytes == 67108864
      and $range.maxResponseBytes == 393216
  ' "$body" >/dev/null 2>&1 || fail binary_range_contract_invalid
  grep -Eq 'binary-read\.bin|inode|device|uid|gid|mode|accessTime' "$body" && fail binary_range_path_or_metadata_reflected

  payload="$(jq -cn --arg path "$binary_read_target" '{"jsonrpc":"2.0","id":"binary-range-short-final","method":"tools/call","params":{"name":"read_binary_range","arguments":{"path":$path,"offset_bytes":5,"length_bytes":10}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status read_binary_range_short_final "$status" 200 safe_root_binary_range_short_final_succeeded
  jq -e '.result.structuredContent.data == "Af4=" and .result.structuredContent.offsetBytes == 5 and .result.structuredContent.sizeBytes == 2 and .result.structuredContent.fileSizeBytes == 7 and .result.structuredContent.eof == true' "$body" >/dev/null 2>&1 || fail binary_range_short_final_contract_invalid

  payload="$(jq -cn --arg path "$binary_read_target" '{"jsonrpc":"2.0","id":"binary-range-eof","method":"tools/call","params":{"name":"read_binary_range","arguments":{"path":$path,"offset_bytes":7,"length_bytes":1}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status read_binary_range_eof "$status" 200 safe_root_binary_range_eof_succeeded
  jq -e '.result.structuredContent.data == "" and .result.structuredContent.sizeBytes == 0 and .result.structuredContent.eof == true' "$body" >/dev/null 2>&1 || fail binary_range_eof_contract_invalid

  payload="$(jq -cn --arg path "$binary_read_target" '{"jsonrpc":"2.0","id":"binary-range-invalid","method":"tools/call","params":{"name":"read_binary_range","arguments":{"path":$path,"offset_bytes":8,"length_bytes":1}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status read_binary_range_invalid "$status" 400 binary_range_invalid_rejected
  jq -e '.error.code == -32602' "$body" >/dev/null 2>&1 || fail binary_range_invalid_body_invalid

  ln -s -- "$binary_read_target" "$VALIDATION_SAFE_ROOT/binary-read-link" 2>/dev/null || fail binary_read_symlink_fixture_create_failed
  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT/binary-read-link" '{"jsonrpc":"2.0","id":"binary-read-link","method":"tools/call","params":{"name":"read_binary_file","arguments":{"path":$path}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status binary_read_symlink "$status" 400 binary_read_symlink_rejected
  jq -e '.error.code == -32602' "$body" >/dev/null 2>&1 || fail binary_read_symlink_body_invalid
  grep -Fq "$VALIDATION_SAFE_ROOT" "$body" && fail binary_read_symlink_path_reflected
  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT/binary-read-link" '{"jsonrpc":"2.0","id":"binary-range-link","method":"tools/call","params":{"name":"read_binary_range","arguments":{"path":$path,"offset_bytes":0,"length_bytes":1}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status binary_range_symlink "$status" 400 binary_range_symlink_rejected
  jq -e '.error.code == -32602' "$body" >/dev/null 2>&1 || fail binary_range_symlink_body_invalid
  grep -Fq "$VALIDATION_SAFE_ROOT" "$body" && fail binary_range_symlink_path_reflected
  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT/binary-read-link" '{"jsonrpc":"2.0","id":"text-range-link","method":"tools/call","params":{"name":"read_text_range","arguments":{"path":$path,"offset_bytes":0,"max_bytes":4}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status text_range_symlink "$status" 400 text_range_symlink_rejected
  jq -e '.error.code == -32602' "$body" >/dev/null 2>&1 || fail text_range_symlink_body_invalid
  grep -Fq "$VALIDATION_SAFE_ROOT" "$body" && fail text_range_symlink_path_reflected
  rm -f -- "$VALIDATION_SAFE_ROOT/binary-read-link" || fail binary_read_symlink_fixture_cleanup_failed

  dd if=/dev/zero of="$VALIDATION_SAFE_ROOT/binary-read-oversized.bin" bs=1 seek=1048576 count=1 status=none 2>/dev/null || fail binary_read_oversized_fixture_create_failed
  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT/binary-read-oversized.bin" '{"jsonrpc":"2.0","id":"binary-read-oversized","method":"tools/call","params":{"name":"read_binary_file","arguments":{"path":$path}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status binary_read_oversized "$status" 413 binary_read_oversized_rejected
  jq -e '.error.code == -32001' "$body" >/dev/null 2>&1 || fail binary_read_oversized_body_invalid
  grep -Fq "$VALIDATION_SAFE_ROOT" "$body" && fail binary_read_oversized_path_reflected
  dd if=/dev/zero of="$VALIDATION_SAFE_ROOT/binary-range-oversized.bin" bs=1 seek=67108864 count=1 status=none 2>/dev/null || fail binary_range_oversized_fixture_create_failed
  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT/binary-range-oversized.bin" '{"jsonrpc":"2.0","id":"binary-range-oversized","method":"tools/call","params":{"name":"read_binary_range","arguments":{"path":$path,"offset_bytes":0,"length_bytes":1}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status binary_range_oversized "$status" 413 binary_range_oversized_rejected
  jq -e '.error.code == -32001' "$body" >/dev/null 2>&1 || fail binary_range_oversized_body_invalid
  grep -Fq "$VALIDATION_SAFE_ROOT" "$body" && fail binary_range_oversized_path_reflected
  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT/binary-range-oversized.bin" '{"jsonrpc":"2.0","id":"text-range-oversized","method":"tools/call","params":{"name":"read_text_range","arguments":{"path":$path,"offset_bytes":0,"max_bytes":4}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status text_range_oversized "$status" 413 text_range_oversized_rejected
  jq -e '.error.code == -32001' "$body" >/dev/null 2>&1 || fail text_range_oversized_body_invalid
  grep -Fq "$VALIDATION_SAFE_ROOT" "$body" && fail text_range_oversized_path_reflected
  rm -f -- "$VALIDATION_SAFE_ROOT/binary-read-oversized.bin" "$VALIDATION_SAFE_ROOT/binary-range-oversized.bin" "$binary_read_target" || fail binary_read_fixture_cleanup_failed
  [[ ! -e "$VALIDATION_SAFE_ROOT/binary-read-oversized.bin" && ! -e "$VALIDATION_SAFE_ROOT/binary-range-oversized.bin" && ! -e "$binary_read_target" ]] || fail binary_read_fixture_cleanup_incomplete
  record_result runtime read_binary_file pass safe_root_binary_read_verified
  record_result runtime read_binary_range pass safe_root_binary_range_read_verified

  text_range_target="$VALIDATION_SAFE_ROOT/text-range-private.txt"
  text_range_invalid="$VALIDATION_SAFE_ROOT/text-range-invalid.txt"
  text_range_expanded="$VALIDATION_SAFE_ROOT/text-range-expanded.txt"
  printf '\141\303\251\360\237\231\202\172' >"$text_range_target" || fail text_range_fixture_create_failed
  printf '\141\377' >"$text_range_invalid" || fail text_range_invalid_fixture_create_failed

  payload="$(jq -cn --arg path "$text_range_target" '{"jsonrpc":"2.0","id":"text-range-first","method":"tools/call","params":{"name":"read_text_range","arguments":{"path":$path,"offset_bytes":0,"max_bytes":4}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status read_text_range "$status" 200 safe_root_text_range_read_succeeded
  bytes="$(wc -c <"$body" 2>/dev/null)" || fail text_range_response_size_failed
  ((bytes <= 1703936)) || fail text_range_response_too_large
  jq -e '
    .result.structuredContent as $range
    | ($range | keys) == ["content","eof","fileSizeBytes","maxFileBytes","maxReadBytes","maxResponseBytes","nextOffsetBytes","offsetBytes","sizeBytes"]
      and $range.content == "a\u00e9"
      and $range.offsetBytes == 0
      and $range.nextOffsetBytes == 3
      and $range.sizeBytes == 3
      and $range.fileSizeBytes == 8
      and $range.eof == false
      and $range.maxReadBytes == 262144
      and $range.maxFileBytes == 67108864
      and $range.maxResponseBytes == 1703936
  ' "$body" >/dev/null 2>&1 || fail text_range_contract_invalid
  grep -Eq 'text-range-private\.txt|inode|device|uid|gid|mode|accessTime' "$body" && fail text_range_path_or_metadata_reflected

  payload="$(jq -cn --arg path "$text_range_target" '{"jsonrpc":"2.0","id":"text-range-second","method":"tools/call","params":{"name":"read_text_range","arguments":{"path":$path,"offset_bytes":3,"max_bytes":4}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status read_text_range_second "$status" 200 safe_root_text_range_second_page_succeeded
  jq -e '.result.structuredContent.content == "\ud83d\ude42" and .result.structuredContent.nextOffsetBytes == 7 and .result.structuredContent.sizeBytes == 4 and .result.structuredContent.eof == false' "$body" >/dev/null 2>&1 || fail text_range_second_contract_invalid

  payload="$(jq -cn --arg path "$text_range_target" '{"jsonrpc":"2.0","id":"text-range-final","method":"tools/call","params":{"name":"read_text_range","arguments":{"path":$path,"offset_bytes":7,"max_bytes":4}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status read_text_range_final "$status" 200 safe_root_text_range_final_page_succeeded
  jq -e '.result.structuredContent.content == "z" and .result.structuredContent.nextOffsetBytes == 8 and .result.structuredContent.sizeBytes == 1 and .result.structuredContent.eof == true' "$body" >/dev/null 2>&1 || fail text_range_final_contract_invalid

  payload="$(jq -cn --arg path "$text_range_target" '{"jsonrpc":"2.0","id":"text-range-eof","method":"tools/call","params":{"name":"read_text_range","arguments":{"path":$path,"offset_bytes":8,"max_bytes":4}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status read_text_range_eof "$status" 200 safe_root_text_range_eof_succeeded
  jq -e '.result.structuredContent.content == "" and .result.structuredContent.nextOffsetBytes == 8 and .result.structuredContent.sizeBytes == 0 and .result.structuredContent.eof == true' "$body" >/dev/null 2>&1 || fail text_range_eof_contract_invalid

  payload="$(jq -cn --arg path "$text_range_target" '{"jsonrpc":"2.0","id":"text-range-mid-codepoint","method":"tools/call","params":{"name":"read_text_range","arguments":{"path":$path,"offset_bytes":2,"max_bytes":4}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status read_text_range_mid_codepoint "$status" 400 text_range_mid_codepoint_rejected
  jq -e '.error.code == -32602' "$body" >/dev/null 2>&1 || fail text_range_mid_codepoint_body_invalid

  payload="$(jq -cn --arg path "$text_range_invalid" '{"jsonrpc":"2.0","id":"text-range-invalid-encoding","method":"tools/call","params":{"name":"read_text_range","arguments":{"path":$path,"offset_bytes":0,"max_bytes":4}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status read_text_range_invalid_encoding "$status" 400 text_range_invalid_encoding_rejected
  jq -e '.error.code == -32602' "$body" >/dev/null 2>&1 || fail text_range_invalid_encoding_body_invalid
  grep -Fq "$VALIDATION_SAFE_ROOT" "$body" && fail text_range_invalid_encoding_path_reflected

  dd if=/dev/zero of="$text_range_expanded" bs=262144 count=1 status=none 2>/dev/null || fail text_range_expanded_fixture_create_failed
  payload="$(jq -cn --arg path "$text_range_expanded" '{"jsonrpc":"2.0","id":"text-range-expanded","method":"tools/call","params":{"name":"read_text_range","arguments":{"path":$path,"offset_bytes":0,"max_bytes":262144}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status read_text_range_expanded "$status" 200 text_range_expanded_succeeded
  bytes="$(wc -c <"$body" 2>/dev/null)" || fail text_range_expanded_response_size_failed
  ((bytes <= 1703936)) || fail text_range_expanded_response_too_large
  jq -e '.result.structuredContent.content | utf8bytelength == 262144' "$body" >/dev/null 2>&1 || fail text_range_expanded_contract_invalid
  rm -f -- "$text_range_target" "$text_range_invalid" "$text_range_expanded" || fail text_range_fixture_cleanup_failed
  [[ ! -e "$text_range_target" && ! -e "$text_range_invalid" && ! -e "$text_range_expanded" ]] || fail text_range_fixture_cleanup_incomplete
  record_result runtime read_text_range pass safe_root_text_range_read_verified

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

  copy_source="$VALIDATION_SAFE_ROOT/copy-source.bin"
  copy_target="$VALIDATION_SAFE_ROOT/copy.bin"
  copy_mismatch_target="$VALIDATION_SAFE_ROOT/copy-mismatch.bin"
  printf 'validation-copy\000\377binary' >"$copy_source" 2>/dev/null || fail copy_file_fixture_create_failed
  chmod 777 "$copy_source" 2>/dev/null || fail copy_file_fixture_create_failed
  copy_bytes="$(stat -c '%s' "$copy_source" 2>/dev/null)" || fail copy_file_fixture_create_failed

  payload="$(jq -cn --arg source "$copy_source" --arg destination "$VALIDATION_SAFE_ROOT/copy-dry.bin" '{"jsonrpc":"2.0","id":"copy-dry","method":"tools/call","params":{"name":"copy_file","arguments":{"source_path":$source,"destination_path":$destination}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status copy_file_dry_run "$status" 200 copy_file_dry_run_succeeded
  jq -e --argjson size "$copy_bytes" '
    .result.structuredContent == {
      dryRun:true,
      sizeBytes:$size,
      mode:"0600",
      maxFileBytes:1048576,
      maxResponseBytes:16384
    }
  ' "$body" >/dev/null 2>&1 || fail copy_file_dry_run_contract_invalid
  grep -Eq 'validation-copy|copy-source\.bin|copy-dry\.bin|termux-mcp-release-validation-' "$body" && fail copy_file_private_data_reflected
  [[ ! -e "$VALIDATION_SAFE_ROOT/copy-dry.bin" ]] || fail copy_file_dry_run_mutated

  payload="$(jq -cn --arg source "$copy_source" --arg destination "$copy_target" '{"jsonrpc":"2.0","id":"copy-missing-grant","method":"tools/call","params":{"name":"copy_file","arguments":{"source_path":$source,"destination_path":$destination,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status copy_file_missing_grant "$status" 403 copy_file_missing_grant_rejected
  jq -e '.error.code == -32003 and .error.data.reason == "capability_grant_missing"' "$body" >/dev/null 2>&1 || fail copy_file_missing_grant_body_invalid
  [[ ! -e "$copy_target" ]] || fail copy_file_missing_grant_mutated

  issue_copy_file_grant "$copy_source" "$copy_target"

  payload="$(jq -cn --arg source "$copy_source" --arg destination "$copy_target" '{"jsonrpc":"2.0","id":"copy-grant-preview","method":"tools/call","params":{"name":"copy_file","arguments":{"source_path":$source,"destination_path":$destination}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$COPY_CAPABILITY_GRANT_FILE")"
  expect_status copy_file_grant_preview "$status" 400 copy_file_grant_preview_rejected
  jq -e '.error.code == -32600' "$body" >/dev/null 2>&1 || fail copy_file_grant_preview_body_invalid
  [[ ! -e "$copy_target" ]] || fail copy_file_grant_preview_mutated

  payload="$(jq -cn --arg source "$copy_source" --arg destination "$copy_mismatch_target" '{"jsonrpc":"2.0","id":"copy-grant-mismatch","method":"tools/call","params":{"name":"copy_file","arguments":{"source_path":$source,"destination_path":$destination,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$COPY_CAPABILITY_GRANT_FILE")"
  expect_status copy_file_grant_binding "$status" 403 copy_file_grant_binding_rejected
  jq -e '.error.code == -32003 and .error.data.reason == "capability_grant_binding_mismatch"' "$body" >/dev/null 2>&1 || fail copy_file_grant_binding_body_invalid
  [[ ! -e "$copy_mismatch_target" && ! -e "$copy_target" ]] || fail copy_file_grant_binding_mutated

  payload="$(jq -cn --arg source "$copy_source" --arg destination "$copy_target" '{"jsonrpc":"2.0","id":"copy-authorized","method":"tools/call","params":{"name":"copy_file","arguments":{"source_path":$source,"destination_path":$destination,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$COPY_CAPABILITY_GRANT_FILE")"
  expect_status copy_file_authorized "$status" 200 copy_file_authorized_succeeded
  jq -e --argjson size "$copy_bytes" '
    .result.structuredContent == {
      dryRun:false,
      sizeBytes:$size,
      mode:"0600",
      maxFileBytes:1048576,
      maxResponseBytes:16384
    }
  ' "$body" >/dev/null 2>&1 || fail copy_file_contract_invalid
  grep -Eq 'validation-copy|copy-source\.bin|copy\.bin|termux-mcp-release-validation-' "$body" && fail copy_file_private_data_reflected
  cmp -s "$copy_source" "$copy_target" || fail copy_file_content_invalid
  [[ "$(stat -c '%a' "$copy_target" 2>/dev/null)" == 600 ]] || fail copy_file_mode_invalid

  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$COPY_CAPABILITY_GRANT_FILE")"
  expect_status copy_file_existing "$status" 400 copy_file_existing_rejected
  jq -e '.error.code == -32602' "$body" >/dev/null 2>&1 || fail copy_file_existing_body_invalid
  cmp -s "$copy_source" "$copy_target" || fail copy_file_existing_modified

  rm -f -- "$copy_target" 2>/dev/null || fail copy_file_replay_fixture_cleanup_failed
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$COPY_CAPABILITY_GRANT_FILE")"
  expect_status copy_file_replay "$status" 403 copy_file_grant_replay_rejected
  jq -e '.error.code == -32003 and .error.data.reason == "capability_grant_replayed"' "$body" >/dev/null 2>&1 || fail copy_file_replay_body_invalid
  [[ ! -e "$copy_target" ]] || fail copy_file_replay_mutated

  copy_stale_source="$VALIDATION_SAFE_ROOT/copy-stale-source.bin"
  copy_stale_target="$VALIDATION_SAFE_ROOT/copy-stale-target.bin"
  printf '%s' stale-original >"$copy_stale_source" 2>/dev/null || fail copy_file_stale_fixture_create_failed
  issue_copy_file_grant "$copy_stale_source" "$copy_stale_target"
  printf '%s' stale-mutated! >"$copy_stale_source" 2>/dev/null || fail copy_file_stale_fixture_change_failed
  payload="$(jq -cn --arg source "$copy_stale_source" --arg destination "$copy_stale_target" '{"jsonrpc":"2.0","id":"copy-stale-source","method":"tools/call","params":{"name":"copy_file","arguments":{"source_path":$source,"destination_path":$destination,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$COPY_CAPABILITY_GRANT_FILE")"
  expect_status copy_file_stale_source "$status" 403 copy_file_stale_source_rejected
  jq -e '.error.code == -32003 and .error.data.reason == "capability_grant_binding_mismatch"' "$body" >/dev/null 2>&1 || fail copy_file_stale_source_body_invalid
  [[ ! -e "$copy_stale_target" ]] || fail copy_file_stale_source_mutated

  ln -s -- "$VALIDATION_SAFE_ROOT/copy-source.bin" "$VALIDATION_SAFE_ROOT/copy-source-link" 2>/dev/null || fail copy_file_symlink_fixture_create_failed
  payload="$(jq -cn --arg source "$VALIDATION_SAFE_ROOT/copy-source-link" --arg destination "$VALIDATION_SAFE_ROOT/copy-from-link.bin" '{"jsonrpc":"2.0","id":"copy-link","method":"tools/call","params":{"name":"copy_file","arguments":{"source_path":$source,"destination_path":$destination,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$COPY_CAPABILITY_GRANT_FILE")"
  expect_status copy_file_symlink "$status" 400 copy_file_symlink_rejected
  jq -e '.error.code == -32602' "$body" >/dev/null 2>&1 || fail copy_file_symlink_body_invalid
  [[ ! -e "$VALIDATION_SAFE_ROOT/copy-from-link.bin" ]] || fail copy_file_symlink_mutated

  copy_oversized="$VALIDATION_SAFE_ROOT/copy-oversized.bin"
  copy_retry_target="$VALIDATION_SAFE_ROOT/copy-oversized-grant-retry.bin"
  dd if=/dev/zero of="$copy_oversized" bs=1048577 count=1 status=none 2>/dev/null || fail copy_file_oversized_fixture_create_failed
  issue_copy_file_grant "$copy_source" "$copy_retry_target"
  payload="$(jq -cn --arg source "$copy_oversized" --arg destination "$copy_retry_target" '{"jsonrpc":"2.0","id":"copy-oversized","method":"tools/call","params":{"name":"copy_file","arguments":{"source_path":$source,"destination_path":$destination,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$COPY_CAPABILITY_GRANT_FILE")"
  expect_status copy_file_oversized "$status" 413 copy_file_oversized_rejected
  jq -e '.error.code == -32001' "$body" >/dev/null 2>&1 || fail copy_file_oversized_body_invalid
  [[ ! -e "$copy_retry_target" ]] || fail copy_file_oversized_mutated

  payload="$(jq -cn --arg source "$copy_source" --arg destination "$copy_retry_target" '{"jsonrpc":"2.0","id":"copy-oversized-grant-retry","method":"tools/call","params":{"name":"copy_file","arguments":{"source_path":$source,"destination_path":$destination,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$COPY_CAPABILITY_GRANT_FILE")"
  expect_status copy_file_oversized_grant_retry "$status" 200 copy_file_oversized_grant_retry_succeeded
  cmp -s "$copy_source" "$copy_retry_target" || fail copy_file_oversized_grant_retry_content_invalid
  [[ "$(stat -c '%a' "$copy_retry_target" 2>/dev/null)" == 600 ]] || fail copy_file_oversized_grant_retry_mode_invalid

  payload='{"jsonrpc":"2.0","id":"copy-audit","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status copy_file_audit "$status" 200 copy_file_audit_read
  jq -e '
    .result.structuredContent.auditCounters as $audit
    | $audit.by_tool.copy_file.allowed >= 3
      and $audit.by_tool.copy_file.denied >= 6
      and $audit.by_reason_code.dry_run_preview.allowed >= 1
      and $audit.by_reason_code.safe_root_file_copied.allowed >= 2
      and $audit.by_reason_code.capability_grant_missing.denied >= 1
      and $audit.by_reason_code.capability_grant_binding_mismatch.denied >= 2
      and $audit.by_reason_code.capability_grant_replayed.denied >= 1
      and $audit.by_reason_code.filesystem_destination_exists.denied >= 1
      and $audit.by_reason_code.filesystem_copy_source_too_large.denied >= 1
  ' "$body" >/dev/null 2>&1 || fail copy_file_audit_contract_invalid
  copy_grant="$(<"$COPY_CAPABILITY_GRANT_FILE")"
  if grep -Eq 'validation-copy|stale-mutated|copy-(source|stale|oversized)|termux-mcp-release-validation-' "$body" \
    || grep -Fq "$copy_grant" "$body"; then
    fail copy_file_audit_private_data_reflected
  fi
  unset copy_grant

  record_result runtime copy_file pass safe_root_file_copy_verified
  record_result runtime copy_file_authorization pass request_scoped_single_use_copy_grant_enforced
  record_result runtime copy_file_binding pass source_content_destination_binding_enforced
  record_result runtime copy_file_exact_binary pass exact_binary_copy_verified
  record_result runtime copy_file_boundaries pass copy_file_boundary_denials_verified
  record_result runtime copy_file_private_audit pass copy_file_private_audit_verified

  trash_target="$VALIDATION_SAFE_ROOT/trash-target.bin"
  trash_mismatch_target="$VALIDATION_SAFE_ROOT/trash-mismatch.bin"
  trash_oversized="$VALIDATION_SAFE_ROOT/trash-oversized.bin"
  trash_exact_target="$VALIDATION_SAFE_ROOT/trash-exact-1mib.bin"
  trash_quarantine="$VALIDATION_SAFE_ROOT/.termux-mcp-trash-quarantine"
  printf '%s' validation-trash-private >"$trash_target" 2>/dev/null || fail trash_file_fixture_create_failed
  printf '%s' mismatch-private >"$trash_mismatch_target" 2>/dev/null || fail trash_file_fixture_create_failed
  chmod 640 "$trash_target" "$trash_mismatch_target" 2>/dev/null || fail trash_file_fixture_create_failed
  trash_bytes="$(stat -c '%s' "$trash_target" 2>/dev/null)" || fail trash_file_fixture_stat_failed
  trash_identity="$(stat -c '%d:%i:%a' "$trash_target" 2>/dev/null)" || fail trash_file_fixture_stat_failed
  trash_digest="$(sha256sum -- "$trash_target" 2>/dev/null | awk '{print $1}')" || fail trash_file_fixture_hash_failed

  payload="$(jq -cn --arg path "$trash_target" '{jsonrpc:"2.0",id:"trash-preview",method:"tools/call",params:{name:"trash_file",arguments:{path:$path}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status trash_file_preview "$status" 200 trash_file_preview_succeeded
  jq -e --argjson size "$trash_bytes" '
    .result.structuredContent == {
      dryRun:true,
      sizeBytes:$size,
      recoveryArtifactRetained:false,
      maxFileBytes:1048576,
      maxResponseBytes:16384
    }
  ' "$body" >/dev/null 2>&1 || fail trash_file_preview_contract_invalid
  grep -Eq 'validation-trash-private|trash-target\.bin|termux-mcp-release-validation-|termux-mcp-trash' "$body" \
    && fail trash_file_preview_private_data_reflected
  [[ -f "$trash_target" && ! -e "$trash_quarantine" ]] || fail trash_file_preview_mutated

  payload="$(jq -cn --arg path "$trash_target" '{jsonrpc:"2.0",id:"trash-missing-grant",method:"tools/call",params:{name:"trash_file",arguments:{path:$path,dry_run:false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status trash_file_missing_grant "$status" 403 trash_file_missing_grant_rejected
  jq -e '.error.code == -32003 and .error.data.reason == "capability_grant_missing"' "$body" >/dev/null 2>&1 || fail trash_file_missing_grant_body_invalid
  [[ -f "$trash_target" && ! -e "$trash_quarantine" ]] || fail trash_file_missing_grant_mutated

  issue_trash_file_grant "$trash_target"

  payload="$(jq -cn --arg path "$trash_target" '{jsonrpc:"2.0",id:"trash-grant-preview",method:"tools/call",params:{name:"trash_file",arguments:{path:$path}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$TRASH_CAPABILITY_GRANT_FILE")"
  expect_status trash_file_grant_preview "$status" 400 trash_file_grant_preview_rejected
  jq -e '.error.code == -32600' "$body" >/dev/null 2>&1 || fail trash_file_grant_preview_body_invalid
  [[ -f "$trash_target" && ! -e "$trash_quarantine" ]] || fail trash_file_grant_preview_mutated

  payload="$(jq -cn --arg path "$trash_mismatch_target" '{jsonrpc:"2.0",id:"trash-grant-mismatch",method:"tools/call",params:{name:"trash_file",arguments:{path:$path,dry_run:false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$TRASH_CAPABILITY_GRANT_FILE")"
  expect_status trash_file_grant_binding "$status" 403 trash_file_grant_binding_rejected
  jq -e '.error.code == -32003 and .error.data.reason == "capability_grant_binding_mismatch"' "$body" >/dev/null 2>&1 || fail trash_file_grant_binding_body_invalid
  [[ -f "$trash_target" && -f "$trash_mismatch_target" && ! -e "$trash_quarantine" ]] || fail trash_file_grant_binding_mutated

  dd if=/dev/zero of="$trash_oversized" bs=1048577 count=1 status=none 2>/dev/null || fail trash_file_oversized_fixture_create_failed
  payload="$(jq -cn --arg path "$trash_oversized" '{jsonrpc:"2.0",id:"trash-oversized",method:"tools/call",params:{name:"trash_file",arguments:{path:$path,dry_run:false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$TRASH_CAPABILITY_GRANT_FILE")"
  expect_status trash_file_oversized "$status" 413 trash_file_oversized_rejected
  jq -e '.error.code == -32001' "$body" >/dev/null 2>&1 || fail trash_file_oversized_body_invalid
  [[ -f "$trash_target" && -f "$trash_oversized" && ! -e "$trash_quarantine" ]] || fail trash_file_oversized_mutated

  payload="$(jq -cn --arg path "$trash_target" '{jsonrpc:"2.0",id:"trash-authorized",method:"tools/call",params:{name:"trash_file",arguments:{path:$path,dry_run:false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$TRASH_CAPABILITY_GRANT_FILE")"
  expect_status trash_file_authorized "$status" 200 trash_file_authorized_succeeded
  jq -e --argjson size "$trash_bytes" '
    .result.structuredContent == {
      dryRun:false,
      sizeBytes:$size,
      recoveryArtifactRetained:true,
      maxFileBytes:1048576,
      maxResponseBytes:16384
    }
  ' "$body" >/dev/null 2>&1 || fail trash_file_contract_invalid
  trash_grant="$(<"$TRASH_CAPABILITY_GRANT_FILE")"
  if grep -Eq 'validation-trash-private|trash-target\.bin|termux-mcp-release-validation-|termux-mcp-trash' "$body" \
    || grep -Fq "$trash_grant" "$body"; then
    fail trash_file_private_data_reflected
  fi
  unset trash_grant
  [[ ! -e "$trash_target" && ! -L "$trash_target" ]] || fail trash_file_target_retained_publicly
  [[ -d "$trash_quarantine" && ! -L "$trash_quarantine" ]] || fail trash_recovery_namespace_invalid
  [[ "$(stat -c '%a' "$trash_quarantine" 2>/dev/null)" == 700 ]] || fail trash_recovery_namespace_mode_invalid
  [[ "$(find "$trash_quarantine" -mindepth 1 -maxdepth 1 -type f -links 1 2>/dev/null | wc -l)" == 1 ]] || fail trash_recovery_artifact_count_invalid
  trash_artifact="$(find "$trash_quarantine" -mindepth 1 -maxdepth 1 -type f -links 1 -print -quit 2>/dev/null)"
  [[ -n "$trash_artifact" && ! -L "$trash_artifact" ]] || fail trash_recovery_artifact_invalid
  [[ "${trash_artifact##*/}" =~ ^\.termux-mcp-trash-artifact-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$ ]] || fail trash_recovery_artifact_name_invalid
  [[ "$(stat -c '%d:%i:%a' "$trash_artifact" 2>/dev/null)" == "$trash_identity" ]] || fail trash_recovery_identity_or_mode_invalid
  [[ "$(sha256sum -- "$trash_artifact" 2>/dev/null | awk '{print $1}')" == "$trash_digest" ]] || fail trash_recovery_content_invalid

  dd if=/dev/zero of="$trash_exact_target" bs=1048576 count=1 status=none 2>/dev/null || fail trash_file_exact_limit_fixture_create_failed
  chmod 600 "$trash_exact_target" 2>/dev/null || fail trash_file_exact_limit_fixture_create_failed
  issue_trash_file_grant "$trash_exact_target"
  payload="$(jq -cn --arg path "$trash_exact_target" '{jsonrpc:"2.0",id:"trash-exact-1mib",method:"tools/call",params:{name:"trash_file",arguments:{path:$path,dry_run:false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$TRASH_CAPABILITY_GRANT_FILE")"
  expect_status trash_file_exact_limit "$status" 200 trash_file_exact_limit_succeeded
  jq -e '.result.structuredContent.sizeBytes == 1048576 and .result.structuredContent.recoveryArtifactRetained == true' "$body" >/dev/null 2>&1 || fail trash_file_exact_limit_body_invalid
  [[ ! -e "$trash_exact_target" ]] || fail trash_file_exact_limit_target_present
  [[ "$(find "$trash_quarantine" -mindepth 1 -maxdepth 1 -type f -links 1 2>/dev/null | wc -l)" == 2 ]] || fail trash_recovery_exact_limit_count_invalid

  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT" '{jsonrpc:"2.0",id:"trash-quarantine-hidden",method:"tools/call",params:{name:"list_directory",arguments:{path:$path,max_depth:1}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status trash_file_namespace_hiding "$status" 200 trash_file_namespace_hiding_succeeded
  grep -Eq 'termux-mcp-trash|trash-target\.bin|validation-trash-private' "$body" && fail trash_recovery_namespace_reflected

  payload='{"jsonrpc":"2.0","id":"trash-audit","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status trash_file_audit "$status" 200 trash_file_audit_read
  jq -e '
    .result.structuredContent.auditCounters as $audit
    | $audit.by_tool.trash_file.allowed >= 3
      and $audit.by_tool.trash_file.denied >= 3
      and $audit.by_reason_code.dry_run_preview.allowed >= 1
      and $audit.by_reason_code.safe_root_file_trashed_recovery_retained.allowed >= 2
      and $audit.by_reason_code.capability_grant_missing.denied >= 1
      and $audit.by_reason_code.capability_grant_binding_mismatch.denied >= 1
      and $audit.by_reason_code.filesystem_trash_target_too_large.denied >= 1
  ' "$body" >/dev/null 2>&1 || fail trash_file_audit_contract_invalid
  grep -Eq 'validation-trash-private|trash-target\.bin|termux-mcp-release-validation-|termux-mcp-trash' "$body" \
    && fail trash_file_audit_private_data_reflected

  record_result runtime trash_file pass safe_root_file_trash_verified
  record_result runtime trash_file_authorization pass request_scoped_trash_grant_enforced
  record_result runtime trash_file_binding pass trash_identity_content_binding_enforced
  record_result runtime trash_file_boundaries pass exact_trash_file_byte_limit_verified
  record_result runtime trash_file_recovery pass trash_recovery_quarantine_verified
  record_result runtime trash_file_private_audit pass trash_file_private_audit_verified

  dd if=/dev/zero of="$VALIDATION_SAFE_ROOT/expanded-response.bin" bs=200000 count=1 status=none 2>/dev/null || fail read_bound_fixture_create_failed
  chmod 600 "$VALIDATION_SAFE_ROOT/expanded-response.bin" 2>/dev/null || fail read_bound_fixture_create_failed
  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT/expanded-response.bin" '{"jsonrpc":"2.0","id":"read-expanded","method":"tools/call","params":{"name":"read_file","arguments":{"path":$path}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status read_response_bound "$status" 413 read_response_bound_enforced
  jq -e '.error.code == -32001' "$body" >/dev/null 2>&1 || fail read_response_bound_body_invalid
  bytes="$(wc -c <"$body" 2>/dev/null)" || fail read_response_size_failed
  ((bytes <= 65536)) || fail read_error_response_too_large
  grep -Fq "$VALIDATION_SAFE_ROOT" "$body" && fail read_error_path_reflected

  local write_target="$VALIDATION_SAFE_ROOT/write.txt"
  printf '%s' validation-write >"$WRITE_CAPABILITY_CONTENT_FILE" 2>/dev/null || fail write_capability_content_staging_failed
  chmod 600 "$WRITE_CAPABILITY_CONTENT_FILE" 2>/dev/null || fail write_capability_content_staging_failed

  payload="$(jq -cn --arg path "$write_target" --arg content validation-write '{"jsonrpc":"2.0","id":"write-dry","method":"tools/call","params":{"name":"write_file","arguments":{"path":$path,"content":$content}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status write_dry_run "$status" 200 write_dry_run_succeeded
  jq -e '.result.structuredContent.dryRun == true' "$body" >/dev/null 2>&1 || fail write_dry_run_invalid
  [[ ! -e "$write_target" ]] || fail write_dry_run_mutated

  payload="$(jq -cn --arg path "$write_target" --arg content validation-write '{"jsonrpc":"2.0","id":"write-missing-grant","method":"tools/call","params":{"name":"write_file","arguments":{"path":$path,"content":$content,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status write_missing_grant "$status" 403 write_missing_grant_rejected
  jq -e '.error.code == -32003 and .error.data.reason == "capability_grant_missing"' "$body" >/dev/null 2>&1 || fail write_missing_grant_body_invalid
  [[ ! -e "$write_target" ]] || fail write_missing_grant_mutated

  issue_write_file_grant "$write_target" "$WRITE_CAPABILITY_CONTENT_FILE" create

  payload="$(jq -cn --arg path "$write_target" --arg content validation-write-mismatch '{"jsonrpc":"2.0","id":"write-grant-mismatch","method":"tools/call","params":{"name":"write_file","arguments":{"path":$path,"content":$content,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$WRITE_CAPABILITY_GRANT_FILE")"
  expect_status write_grant_binding "$status" 403 write_grant_binding_rejected
  jq -e '.error.code == -32003 and .error.data.reason == "capability_grant_binding_mismatch"' "$body" >/dev/null 2>&1 || fail write_grant_binding_body_invalid
  [[ ! -e "$write_target" ]] || fail write_grant_binding_mutated

  payload="$(jq -cn --arg path "$write_target" --arg content validation-write '{"jsonrpc":"2.0","id":"write","method":"tools/call","params":{"name":"write_file","arguments":{"path":$path,"content":$content,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$WRITE_CAPABILITY_GRANT_FILE")"
  expect_status write_explicit "$status" 200 write_explicit_succeeded
  jq -e '.result.structuredContent.dryRun == false and .result.structuredContent.disposition == "create" and .result.structuredContent.sizeBytes == 16 and .result.structuredContent.mode == "0600" and .result.structuredContent.recoveryArtifactRetained == false' "$body" >/dev/null 2>&1 || fail write_explicit_body_invalid
  [[ "$(stat -c '%a' "$write_target" 2>/dev/null)" == 600 ]] || fail write_mode_invalid
  [[ "$(<"$write_target")" == validation-write ]] || fail write_content_invalid

  rm -f -- "$write_target" 2>/dev/null || fail write_replay_fixture_cleanup_failed
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$WRITE_CAPABILITY_GRANT_FILE")"
  expect_status write_grant_replay "$status" 403 write_grant_replay_rejected
  jq -e '.error.code == -32003 and .error.data.reason == "capability_grant_replayed"' "$body" >/dev/null 2>&1 || fail write_grant_replay_body_invalid
  [[ ! -e "$write_target" ]] || fail write_grant_replay_mutated
  record_result runtime write_file_grant pass request_scoped_write_grant_enforced
  record_result runtime write_file_authorization pass request_scoped_single_use_write_grant_enforced

  printf '%s' validation-replace-original >"$write_target" 2>/dev/null || fail write_replace_fixture_create_failed
  chmod 640 "$write_target" 2>/dev/null || fail write_replace_fixture_create_failed
  inspect_write_file_recovery
  recovery_count_before="$WRITE_FILE_RECOVERY_COUNT"
  old_identity="$(stat -c '%d:%i' "$write_target" 2>/dev/null)" || fail write_replace_identity_preflight_failed
  replacement_content=validation-replacement
  printf '%s' "$replacement_content" >"$WRITE_CAPABILITY_CONTENT_FILE" 2>/dev/null || fail write_capability_content_staging_failed
  chmod 600 "$WRITE_CAPABILITY_CONTENT_FILE" 2>/dev/null || fail write_capability_content_staging_failed
  issue_write_file_grant "$write_target" "$WRITE_CAPABILITY_CONTENT_FILE" replace

  payload="$(jq -cn --arg path "$write_target" --arg content "$replacement_content" '{"jsonrpc":"2.0","id":"write-replace","method":"tools/call","params":{"name":"write_file","arguments":{"path":$path,"content":$content,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$WRITE_CAPABILITY_GRANT_FILE")"
  expect_status write_replace "$status" 200 write_replace_succeeded
  jq -e ".result.structuredContent.dryRun == false and .result.structuredContent.disposition == \"replace\" and .result.structuredContent.sizeBytes == ${#replacement_content} and .result.structuredContent.mode == \"0600\" and .result.structuredContent.recoveryArtifactRetained == true" "$body" >/dev/null 2>&1 || fail write_replace_body_invalid
  [[ "$(<"$write_target")" == "$replacement_content" ]] || fail write_replace_content_invalid
  [[ "$(stat -c '%a' "$write_target" 2>/dev/null)" == 600 ]] || fail write_replace_mode_invalid
  new_identity="$(stat -c '%d:%i' "$write_target" 2>/dev/null)" || fail write_replace_identity_check_failed
  [[ "$new_identity" != "$old_identity" ]] || fail write_replace_identity_reused
  inspect_write_file_recovery validation-replace-original 640
  recovery_count_after="$WRITE_FILE_RECOVERY_COUNT"
  ((recovery_count_after == recovery_count_before + 1)) || fail write_recovery_artifact_count_invalid
  ((WRITE_FILE_RECOVERY_CONTENT_MATCHES == 1)) || fail write_recovery_artifact_content_invalid
  record_result runtime write_file_replace pass bounded_recovery_write_replacement_verified
  record_result runtime write_file_publication pass safe_root_file_create_replace_verified

  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT" '{"jsonrpc":"2.0","id":"write-recovery-list","method":"tools/call","params":{"name":"list_directory","arguments":{"path":$path,"max_depth":1}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status write_recovery_list "$status" 200 write_recovery_list_succeeded
  grep -Fq '.termux-mcp-write-quarantine' "$body" && fail write_recovery_namespace_visible_in_list

  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT" --arg query '.termux-mcp-write-quarantine' '{"jsonrpc":"2.0","id":"write-recovery-find","method":"tools/call","params":{"name":"find_paths","arguments":{"path":$path,"query":$query,"kind":"directory","max_depth":1}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status write_recovery_find "$status" 200 write_recovery_find_succeeded
  jq -e '.result.structuredContent.matches == []' "$body" >/dev/null 2>&1 || fail write_recovery_namespace_visible_in_find
  record_result runtime write_file_recovery_namespace pass recovery_namespace_private_and_bounded

  preserved_target="$VALIDATION_SAFE_ROOT/write-preflight-original.txt"
  printf '%s' validation-preflight-original >"$write_target" 2>/dev/null || fail write_replace_binding_fixture_create_failed
  chmod 600 "$write_target" 2>/dev/null || fail write_replace_binding_fixture_create_failed
  preflight_identity="$(stat -c '%d:%i' "$write_target" 2>/dev/null)" || fail write_replace_binding_preflight_failed
  printf '%s' validation-binding-denied >"$WRITE_CAPABILITY_CONTENT_FILE" 2>/dev/null || fail write_capability_content_staging_failed
  chmod 600 "$WRITE_CAPABILITY_CONTENT_FILE" 2>/dev/null || fail write_capability_content_staging_failed
  issue_write_file_grant "$write_target" "$WRITE_CAPABILITY_CONTENT_FILE" replace
  mv -- "$write_target" "$preserved_target" 2>/dev/null || fail write_replace_binding_fixture_swap_failed
  printf '%s' validation-substitute >"$write_target" 2>/dev/null || fail write_replace_binding_fixture_swap_failed
  chmod 600 "$write_target" 2>/dev/null || fail write_replace_binding_fixture_swap_failed
  substitute_identity="$(stat -c '%d:%i' "$write_target" 2>/dev/null)" || fail write_replace_binding_fixture_swap_failed

  payload="$(jq -cn --arg path "$write_target" --arg content validation-binding-denied '{"jsonrpc":"2.0","id":"write-replace-binding","method":"tools/call","params":{"name":"write_file","arguments":{"path":$path,"content":$content,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$WRITE_CAPABILITY_GRANT_FILE")"
  expect_status write_replace_binding "$status" 403 write_replace_binding_rejected
  jq -e '.error.code == -32003 and .error.data.reason == "capability_grant_binding_mismatch"' "$body" >/dev/null 2>&1 || fail write_replace_binding_body_invalid
  [[ "$(<"$write_target")" == validation-substitute \
    && "$(stat -c '%d:%i' "$write_target" 2>/dev/null)" == "$substitute_identity" ]] \
    || fail write_replace_binding_substitute_modified
  [[ "$(<"$preserved_target")" == validation-preflight-original \
    && "$(stat -c '%d:%i' "$preserved_target" 2>/dev/null)" == "$preflight_identity" ]] \
    || fail write_replace_binding_original_modified
  inspect_write_file_recovery validation-replace-original 640
  ((WRITE_FILE_RECOVERY_COUNT == recovery_count_after)) || fail write_binding_denial_changed_recovery_state
  ((WRITE_FILE_RECOVERY_CONTENT_MATCHES == 1)) || fail write_binding_denial_changed_recovery_content
  record_result runtime write_file_replace_binding pass replacement_identity_binding_enforced
  rm -f -- "$write_target" "$preserved_target" 2>/dev/null || fail write_replace_binding_fixture_cleanup_failed

  rm -f -- "$WRITE_CAPABILITY_GRANT_FILE" "$WRITE_CAPABILITY_CONTENT_FILE" 2>/dev/null || fail write_capability_staging_cleanup_failed
  WRITE_CAPABILITY_GRANT_FILE=""
  WRITE_CAPABILITY_CONTENT_FILE=""

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

run_write_file_boundary_checks() {
  local body="$TEMP_ROOT/write-file-boundary-response.json"
  local headers="$TEMP_ROOT/write-file-boundary-headers.txt"
  local exact_content="$TEMP_ROOT/write-file-exact-limit.txt"
  local over_content="$TEMP_ROOT/write-file-over-limit.txt"
  local oversized_identifier="$TEMP_ROOT/write-file-oversized-id.txt"
  local exact_target="$VALIDATION_SAFE_ROOT/write-file-exact-limit.txt"
  local over_target="$VALIDATION_SAFE_ROOT/write-file-over-limit.txt"
  local preflight_target="$VALIDATION_SAFE_ROOT/write-file-response-preflight.txt"
  local trash_preflight_target="$VALIDATION_SAFE_ROOT/trash-file-response-preflight.txt"
  local trash_quarantine="$VALIDATION_SAFE_ROOT/.termux-mcp-trash-quarantine"
  local status payload bytes recovery_count_before trash_count_before trash_count_after

  WRITE_CAPABILITY_GRANT_FILE="$TEMP_ROOT/write-file-boundary-grant.txt"
  WRITE_CAPABILITY_CONTENT_FILE="$TEMP_ROOT/write-file-boundary-content.txt"
  : >"$WRITE_CAPABILITY_GRANT_FILE" 2>/dev/null || fail write_file_boundary_staging_failed
  : >"$WRITE_CAPABILITY_CONTENT_FILE" 2>/dev/null || fail write_file_boundary_staging_failed
  dd if=/dev/zero bs=1048576 count=1 status=none 2>/dev/null \
    | tr '\000' x >"$exact_content" \
    || fail write_file_exact_fixture_create_failed
  dd if=/dev/zero bs=1048577 count=1 status=none 2>/dev/null \
    | tr '\000' y >"$over_content" \
    || fail write_file_over_fixture_create_failed
  printf '%*s' 17000 '' | tr ' ' i >"$oversized_identifier" \
    || fail write_file_identifier_staging_failed
  chmod 600 "$WRITE_CAPABILITY_GRANT_FILE" "$WRITE_CAPABILITY_CONTENT_FILE" \
    "$exact_content" "$over_content" "$oversized_identifier" 2>/dev/null \
    || fail write_file_boundary_staging_failed
  [[ "$(stat -c '%s' "$exact_content" 2>/dev/null)" == 1048576 ]] \
    || fail write_file_exact_fixture_size_invalid
  [[ "$(stat -c '%s' "$over_content" 2>/dev/null)" == 1048577 ]] \
    || fail write_file_over_fixture_size_invalid

  inspect_write_file_recovery
  recovery_count_before="$WRITE_FILE_RECOVERY_COUNT"
  start_server "$MCP_PINNED_ARTIFACT" mcp 2097152
  curl_local -fsS -o "$body" "http://$BIND_HOST:$PORT/ready" 2>/dev/null \
    || fail write_file_readiness_failed
  jq -e --arg version "$EXPECTED_VERSION" '
    .status == "ready"
    and .version == $version
    and .mcp_runtime_enabled == true
    and .mcp_request_limits.max_body_bytes == 2097152
  ' "$body" >/dev/null 2>&1 || fail write_file_runtime_posture_mismatch
  record_result runtime write_file_readiness pass expanded_body_posture_verified

  payload='{"jsonrpc":"2.0","id":"write-file-boundary-initialize","method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"release-validator","version":"1.0.0"}}}'
  stage_request "$payload"
  status="$(curl_local -sS -D "$headers" -o "$body" -w '%{http_code}' \
    -H "@$AUTH_HEADER_FILE" \
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
    --data-binary "@$REQUEST_FILE" "http://$BIND_HOST:$PORT/mcp")"
  expect_status write_file_boundary_initialize "$status" 200 write_file_boundary_initialize_succeeded
  MCP_SESSION_ID="$(awk 'tolower($1) == "mcp-session-id:" {sub(/^[^:]*:[[:space:]]*/, ""); sub(/\r$/, ""); print; exit}' "$headers")"
  [[ "$MCP_SESSION_ID" =~ ^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$ ]] \
    || fail write_file_boundary_session_header_invalid
  payload='{"jsonrpc":"2.0","method":"notifications/initialized"}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status write_file_boundary_initialized "$status" 202 write_file_boundary_initialized_accepted
  [[ ! -s "$body" ]] || fail write_file_boundary_initialized_body_present

  printf '%s' validation-trash-response-preflight >"$trash_preflight_target" 2>/dev/null \
    || fail trash_file_response_preflight_fixture_create_failed
  chmod 600 "$trash_preflight_target" 2>/dev/null \
    || fail trash_file_response_preflight_fixture_create_failed
  issue_trash_file_grant "$trash_preflight_target"
  trash_count_before="$(find "$trash_quarantine" -mindepth 1 -maxdepth 1 -type f -links 1 2>/dev/null | wc -l)" \
    || fail trash_file_response_preflight_inspection_failed
  jq -cn \
    --rawfile identifier "$oversized_identifier" \
    --arg path "$trash_preflight_target" \
    '{jsonrpc:"2.0",id:$identifier,method:"tools/call",params:{name:"trash_file",arguments:{path:$path,dry_run:false}}}' \
    >"$REQUEST_FILE" 2>/dev/null || fail trash_file_response_preflight_fixture_create_failed
  chmod 600 "$REQUEST_FILE" 2>/dev/null || fail trash_file_response_preflight_fixture_create_failed
  status="$(mcp_post_staged "$body" "$MCP_SESSION_ID" 1 "$TRASH_CAPABILITY_GRANT_FILE")"
  expect_status trash_file_response_preflight "$status" 413 trash_file_response_preflight_rejected
  jq -e '.error.code == -32001 and .id == null' "$body" >/dev/null 2>&1 \
    || fail trash_file_response_preflight_body_invalid
  bytes="$(wc -c <"$body" 2>/dev/null)" || fail trash_file_response_preflight_size_failed
  ((bytes <= 16384)) || fail trash_file_response_preflight_error_too_large
  trash_count_after="$(find "$trash_quarantine" -mindepth 1 -maxdepth 1 -type f -links 1 2>/dev/null | wc -l)" \
    || fail trash_file_response_preflight_inspection_failed
  [[ -f "$trash_preflight_target" && "$trash_count_after" == "$trash_count_before" ]] \
    || fail trash_file_response_preflight_mutated

  payload="$(jq -cn --arg path "$trash_preflight_target" '{jsonrpc:"2.0",id:"trash-response-preflight-retry",method:"tools/call",params:{name:"trash_file",arguments:{path:$path,dry_run:false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" 1 "$TRASH_CAPABILITY_GRANT_FILE")"
  expect_status trash_file_response_preflight_retry "$status" 200 trash_file_response_preflight_retry_succeeded
  jq -e '
    .result.structuredContent.dryRun == false
    and .result.structuredContent.recoveryArtifactRetained == true
    and .result.structuredContent.maxFileBytes == 1048576
    and .result.structuredContent.maxResponseBytes == 16384
  ' "$body" >/dev/null 2>&1 || fail trash_file_response_preflight_retry_body_invalid
  trash_count_after="$(find "$trash_quarantine" -mindepth 1 -maxdepth 1 -type f -links 1 2>/dev/null | wc -l)" \
    || fail trash_file_response_preflight_inspection_failed
  [[ ! -e "$trash_preflight_target" && "$trash_count_after" == "$((trash_count_before + 1))" ]] \
    || fail trash_file_response_preflight_retry_mutated
  record_result runtime trash_file_response_preflight pass bounded_trash_file_response_preflight_verified

  issue_write_file_grant "$exact_target" "$exact_content" create
  stage_write_file_request write-file-exact-limit "$exact_target" "$exact_content" false
  status="$(mcp_post_staged "$body" "$MCP_SESSION_ID" 1 "$WRITE_CAPABILITY_GRANT_FILE")"
  expect_status write_file_exact_limit "$status" 200 write_file_exact_limit_succeeded
  jq -e '
    .result.structuredContent.dryRun == false
    and .result.structuredContent.sizeBytes == 1048576
    and .result.structuredContent.disposition == "create"
    and .result.structuredContent.mode == "0600"
    and .result.structuredContent.maxFileBytes == 1048576
    and .result.structuredContent.maxResponseBytes == 16384
    and .result.structuredContent.recoveryArtifactRetained == false
  ' "$body" >/dev/null 2>&1 || fail write_file_exact_limit_body_invalid
  cmp -s "$exact_content" "$exact_target" || fail write_file_exact_limit_content_invalid
  [[ "$(stat -c '%a' "$exact_target" 2>/dev/null)" == 600 ]] \
    || fail write_file_exact_limit_mode_invalid

  printf '%s' validation-over-limit-grant-retry >"$WRITE_CAPABILITY_CONTENT_FILE" 2>/dev/null \
    || fail write_file_content_staging_failed
  chmod 600 "$WRITE_CAPABILITY_CONTENT_FILE" 2>/dev/null || fail write_file_content_staging_failed
  issue_write_file_grant "$over_target" "$WRITE_CAPABILITY_CONTENT_FILE" create
  stage_write_file_request write-file-over-limit "$over_target" "$over_content" false
  status="$(mcp_post_staged "$body" "$MCP_SESSION_ID" 1 "$WRITE_CAPABILITY_GRANT_FILE")"
  expect_status write_file_over_limit "$status" 413 write_file_over_limit_rejected
  jq -e '.error.code == -32001' "$body" >/dev/null 2>&1 \
    || fail write_file_over_limit_body_invalid
  [[ ! -e "$over_target" ]] || fail write_file_over_limit_mutated

  stage_write_file_request write-file-over-limit-grant-retry "$over_target" \
    "$WRITE_CAPABILITY_CONTENT_FILE" false
  status="$(mcp_post_staged "$body" "$MCP_SESSION_ID" 1 "$WRITE_CAPABILITY_GRANT_FILE")"
  expect_status write_file_over_limit_grant_retry "$status" 200 write_file_over_limit_grant_retry_succeeded
  cmp -s "$WRITE_CAPABILITY_CONTENT_FILE" "$over_target" \
    || fail write_file_over_limit_grant_retry_content_invalid
  [[ "$(stat -c '%a' "$over_target" 2>/dev/null)" == 600 ]] \
    || fail write_file_over_limit_grant_retry_mode_invalid
  record_result runtime write_file_exact_limit pass exact_write_file_byte_limit_verified

  printf '%s' validation-write-preflight >"$WRITE_CAPABILITY_CONTENT_FILE" 2>/dev/null \
    || fail write_file_content_staging_failed
  chmod 600 "$WRITE_CAPABILITY_CONTENT_FILE" 2>/dev/null || fail write_file_content_staging_failed
  issue_write_file_grant "$preflight_target" "$WRITE_CAPABILITY_CONTENT_FILE" create
  stage_write_file_request_with_id_file "$oversized_identifier" "$preflight_target" \
    "$WRITE_CAPABILITY_CONTENT_FILE"
  status="$(mcp_post_staged "$body" "$MCP_SESSION_ID" 1 "$WRITE_CAPABILITY_GRANT_FILE")"
  expect_status write_file_response_preflight "$status" 413 write_file_response_preflight_rejected
  jq -e '.id == null and .error.code == -32001' "$body" >/dev/null 2>&1 \
    || fail write_file_response_preflight_body_invalid
  bytes="$(wc -c <"$body" 2>/dev/null)" || fail write_file_response_preflight_size_failed
  ((bytes <= 16384)) || fail write_file_response_preflight_size_invalid
  [[ ! -e "$preflight_target" ]] || fail write_file_response_preflight_mutated

  stage_write_file_request write-file-response-retry "$preflight_target" \
    "$WRITE_CAPABILITY_CONTENT_FILE" false
  status="$(mcp_post_staged "$body" "$MCP_SESSION_ID" 1 "$WRITE_CAPABILITY_GRANT_FILE")"
  expect_status write_file_response_retry "$status" 200 write_file_response_retry_succeeded
  cmp -s "$WRITE_CAPABILITY_CONTENT_FILE" "$preflight_target" \
    || fail write_file_response_retry_content_invalid
  [[ "$(stat -c '%a' "$preflight_target" 2>/dev/null)" == 600 ]] \
    || fail write_file_response_retry_mode_invalid
  inspect_write_file_recovery
  ((WRITE_FILE_RECOVERY_COUNT == recovery_count_before)) \
    || fail write_file_boundary_changed_recovery_state
  record_result runtime write_file_response_preflight pass bounded_write_file_response_preflight_verified

  stage_session_headers "$MCP_SESSION_ID"
  status="$(curl_local -sS -X DELETE -o "$body" -w '%{http_code}' \
    -H "@$SESSION_HEADER_FILE" \
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
    "http://$BIND_HOST:$PORT/mcp")"
  expect_status write_file_boundary_session_delete "$status" 204 write_file_boundary_session_deleted
  [[ ! -s "$body" ]] || fail write_file_boundary_session_delete_body_present
  MCP_SESSION_ID=""
  stop_server || fail write_file_boundary_runtime_stop_failed

  rm -f -- "$exact_target" "$over_target" "$preflight_target" "$trash_preflight_target" \
    "$WRITE_CAPABILITY_GRANT_FILE" "$WRITE_CAPABILITY_CONTENT_FILE" \
    "$exact_content" "$over_content" "$oversized_identifier" 2>/dev/null \
    || fail write_file_boundary_cleanup_failed
  WRITE_CAPABILITY_GRANT_FILE=""
  WRITE_CAPABILITY_CONTENT_FILE=""
}

run_volume_control_runtime_checks() {
  local body="$TEMP_ROOT/volume-control-response.json"
  local headers="$TEMP_ROOT/volume-control-headers.txt"
  local compile_log="$TEMP_ROOT/volume-control-compile-gate.log"
  local trash_quarantine="$VALIDATION_SAFE_ROOT/.termux-mcp-trash-quarantine"
  local trash_count_before trash_count_after trash_quarantine_before trash_quarantine_after
  local trash_target_before trash_target_after status payload compile_rc

  if timeout -k 2 5 env -i \
    "HOME=$HOME" \
    "PREFIX=${PREFIX:-}" \
    "PATH=$PATH" \
    "MCP__AUTH__STATIC_TOKEN=$MCP_TOKEN" \
    MCP__ANDROID__VOLUME_CONTROL_ENABLED=true \
    "MCP__CAPABILITY__KEY_ID=$CAPABILITY_KEY_ID" \
    "MCP__CAPABILITY__HMAC_KEY_HEX=$CAPABILITY_KEY_HEX" \
    "MCP__SERVER__HOST=$BIND_HOST" \
    "MCP__SERVER__PORT=$PORT" \
    "$MCP_PINNED_ARTIFACT" >"$compile_log" 2>&1
  then
    compile_rc=0
  else
    compile_rc=$?
  fi
  ((compile_rc != 0 && compile_rc != 124 && compile_rc != 137)) || fail volume_control_compile_gate_not_enforced
  grep -Fq 'MCP__ANDROID__VOLUME_CONTROL_ENABLED requires a binary built with the android-volume-control feature' "$compile_log" || fail volume_control_compile_gate_error_invalid
  record_result runtime volume_control_compile_gate pass incompatible_volume_control_artifact_rejected

  start_server "$VOLUME_CONTROL_PINNED_ARTIFACT" volume_control
  curl_local -fsS -o "$body" "http://$BIND_HOST:$PORT/ready" 2>/dev/null || fail volume_control_readiness_failed
  jq -e --arg version "$EXPECTED_VERSION" '
    .status == "ready"
    and .version == $version
    and .mcp_runtime_enabled == true
    and .safe_root_count == 1
    and .auth_posture == "static_token"
    and .mcp_request_limits.max_concurrent_requests == 4
    and .mcp_request_limits.request_timeout_seconds == 30
    and .mcp_request_limits.max_body_bytes == 1024
    and .mcp_request_limits.sse_enabled == false
  ' "$body" >/dev/null 2>&1 || fail volume_control_feature_posture_mismatch
  record_result runtime volume_control_readiness pass volume_control_posture_verified

  payload='{"jsonrpc":"2.0","id":"volume-control-initialize","method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"release-validator","version":"1.0.0"}}}'
  stage_request "$payload"
  status="$(curl_local -sS -D "$headers" -o "$body" -w '%{http_code}' \
    -H "@$AUTH_HEADER_FILE" \
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
    --data-binary "@$REQUEST_FILE" "http://$BIND_HOST:$PORT/mcp")"
  expect_status volume_control_initialize "$status" 200 volume_control_initialize_succeeded
  MCP_SESSION_ID="$(awk 'tolower($1) == "mcp-session-id:" {sub(/^[^:]*:[[:space:]]*/, ""); sub(/\r$/, ""); print; exit}' "$headers")"
  [[ "$MCP_SESSION_ID" =~ ^[A-Za-z0-9-]{1,128}$ ]] || fail volume_control_session_header_invalid

  payload='{"jsonrpc":"2.0","method":"notifications/initialized"}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status volume_control_initialized_notification "$status" 202 volume_control_initialized_notification_accepted
  [[ ! -s "$body" ]] || fail volume_control_initialized_notification_body_present

  payload='{"jsonrpc":"2.0","id":"volume-control-tools","method":"tools/list"}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status volume_control_tool_discovery "$status" 200 volume_control_tool_discovery_succeeded
  jq -e '[.result.tools[].name] == ["runtime_status","platform_info","android_status","project_service_status","create_directory","copy_file","trash_file","find_paths","hash_file","list_directory","path_metadata","read_binary_file","read_binary_range","read_file","read_text_range","search_text","write_file"]' "$body" >/dev/null 2>&1 || fail volume_control_disabled_discovery_invalid
  jq -e '
    .result.tools
    | map(select(.name == "write_file"))[0] as $tool
    | $tool.inputSchema.properties.dry_run.const == true
      and ($tool.description | contains("mutation gate is disabled"))
  ' "$body" >/dev/null 2>&1 || fail volume_control_write_discovery_invalid
  jq -e '
    .result.tools
    | map(select(.name == "copy_file"))[0] as $tool
    | $tool.inputSchema.properties.dry_run.const == true
      and ($tool.description | contains("copy mutation gate is disabled"))
  ' "$body" >/dev/null 2>&1 || fail volume_control_copy_discovery_invalid
  jq -e '
    .result.tools
    | map(select(.name == "trash_file"))[0] as $tool
    | $tool.inputSchema.properties.dry_run.const == true
      and ($tool.inputSchema.additionalProperties == false)
      and ($tool.description | contains("trash"))
      and ($tool.description | contains("mutation gate is disabled"))
  ' "$body" >/dev/null 2>&1 || fail volume_control_trash_discovery_invalid
  record_result runtime volume_control_disabled_discovery pass volume_control_hidden_while_disabled

  payload='{"jsonrpc":"2.0","id":"volume-control-status","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status volume_control_runtime_status "$status" 200 volume_control_runtime_status_read
  jq -e '
    .result.structuredContent.androidVolumeControlCompiled == true
    and .result.structuredContent.androidVolumeControlEnabled == false
    and .result.structuredContent.androidVolumeGrantRequired == false
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
    and .result.structuredContent.fileWriteMode == "dry_run_only_mutation_disabled"
    and .result.structuredContent.fileWriteMutationEnabled == false
    and .result.structuredContent.fileWriteGrantRequired == false
    and .result.structuredContent.fileWriteGrantHeader == "mcp-capability-grant"
    and .result.structuredContent.fileWriteGrantTtlSeconds == 60
    and .result.structuredContent.highImpactTools == false
    and .result.structuredContent.binaryRangeReads == true
    and .result.structuredContent.binaryRangeReadMaxFileBytes == 67108864
    and .result.structuredContent.binaryRangeReadMaxBytes == 262144
    and .result.structuredContent.binaryRangeReadMaxResponseBytes == 393216
    and .result.structuredContent.textRangeReads == true
    and .result.structuredContent.textRangeReadEncoding == "utf-8"
    and .result.structuredContent.textRangeReadMinBytes == 4
    and .result.structuredContent.textRangeReadMaxFileBytes == 67108864
    and .result.structuredContent.textRangeReadMaxBytes == 262144
    and .result.structuredContent.textRangeReadMaxResponseBytes == 1703936
  ' "$body" >/dev/null 2>&1 || fail volume_control_runtime_status_invalid

  payload="$(jq -cn --arg source "$VALIDATION_SAFE_ROOT/visible.txt" --arg destination "$VALIDATION_SAFE_ROOT/volume-copy-disabled.txt" '{jsonrpc:"2.0",id:"volume-copy-disabled",method:"tools/call",params:{name:"copy_file",arguments:{source_path:$source,destination_path:$destination,dry_run:false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status volume_control_copy_disabled "$status" 403 copy_file_mutation_disabled
  jq -e '.error.code == -32003 and .error.data.reason == "copy_file_mutation_disabled"' "$body" >/dev/null 2>&1 || fail volume_control_copy_disabled_body_invalid
  [[ ! -e "$VALIDATION_SAFE_ROOT/volume-copy-disabled.txt" ]] || fail volume_control_copy_disabled_mutated
  record_result runtime copy_file_disabled_posture pass copy_file_disabled_posture_verified

  trash_target_before="$(stat -c '%d:%i:%a:%s:%Y:%Z' "$VALIDATION_SAFE_ROOT/visible.txt" 2>/dev/null)" \
    || fail volume_control_trash_disabled_fixture_invalid
  trash_quarantine_before="$(stat -c '%d:%i:%a:%s:%Y:%Z' "$trash_quarantine" 2>/dev/null)" \
    || fail volume_control_trash_disabled_fixture_invalid
  trash_count_before="$(find "$trash_quarantine" -mindepth 1 -maxdepth 1 -type f -links 1 2>/dev/null | wc -l)" \
    || fail volume_control_trash_disabled_fixture_invalid
  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT/visible.txt" '{jsonrpc:"2.0",id:"volume-trash-disabled",method:"tools/call",params:{name:"trash_file",arguments:{path:$path,dry_run:false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status volume_control_trash_disabled "$status" 403 trash_file_mutation_disabled
  jq -e '.error.code == -32003 and .error.data.reason == "trash_file_mutation_disabled"' "$body" >/dev/null 2>&1 || fail volume_control_trash_disabled_body_invalid
  trash_target_after="$(stat -c '%d:%i:%a:%s:%Y:%Z' "$VALIDATION_SAFE_ROOT/visible.txt" 2>/dev/null)" \
    || fail volume_control_trash_disabled_mutated
  trash_quarantine_after="$(stat -c '%d:%i:%a:%s:%Y:%Z' "$trash_quarantine" 2>/dev/null)" \
    || fail volume_control_trash_disabled_mutated
  trash_count_after="$(find "$trash_quarantine" -mindepth 1 -maxdepth 1 -type f -links 1 2>/dev/null | wc -l)" \
    || fail volume_control_trash_disabled_mutated
  [[ "$trash_target_after" == "$trash_target_before" \
    && "$trash_quarantine_after" == "$trash_quarantine_before" \
    && "$trash_count_after" == "$trash_count_before" ]] \
    || fail volume_control_trash_disabled_mutated
  record_result runtime trash_file_disabled_posture pass trash_file_disabled_posture_verified

  payload='{"jsonrpc":"2.0","id":"volume-control-disabled-call","method":"tools/call","params":{"name":"set_android_volume","arguments":{"stream":"music","level":1,"dry_run":false}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status volume_control_disabled_call "$status" 200 volume_control_disabled_call_rejected
  jq -e '.result.isError == true and .result.structuredContent.reasonCode == "volume_control_runtime_disabled"' "$body" >/dev/null 2>&1 || fail volume_control_disabled_call_invalid

  stage_session_headers "$MCP_SESSION_ID"
  status="$(curl_local -sS -X DELETE -o "$body" -w '%{http_code}' \
    -H "@$SESSION_HEADER_FILE" \
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
    "http://$BIND_HOST:$PORT/mcp")"
  expect_status volume_control_session_delete "$status" 204 volume_control_session_deleted
  [[ ! -s "$body" ]] || fail volume_control_session_delete_body_present
  MCP_SESSION_ID=""
  stop_server || fail volume_control_runtime_stop_failed
}

snapshot_private_directory() {
  local directory="$1" staging="$2" entry base mode size links digest
  : >"$staging" 2>/dev/null || return 1
  chmod 600 "$staging" 2>/dev/null || return 1
  if [[ ! -e "$directory" && ! -L "$directory" ]]; then
    printf '%s\n' absent >"$staging" 2>/dev/null || return 1
  else
    [[ -d "$directory" && ! -L "$directory" ]] || return 1
    stat -c 'directory:%d:%i:%a:%s:%Y:%Z' "$directory" >>"$staging" 2>/dev/null || return 1
    while IFS= read -r -d '' entry; do
      [[ -f "$entry" && ! -L "$entry" ]] || return 1
      base="${entry##*/}"
      [[ "$base" != *$'\n'* && "$base" != *$'\r'* ]] || return 1
      mode="$(stat -c '%a' "$entry" 2>/dev/null)" || return 1
      size="$(stat -c '%s' "$entry" 2>/dev/null)" || return 1
      links="$(stat -c '%h' "$entry" 2>/dev/null)" || return 1
      digest="$(sha256sum -- "$entry" 2>/dev/null | awk '{print $1}')" || return 1
      [[ "$mode" =~ ^[0-7]{3,4}$ && "$size" =~ ^[0-9]+$ && "$links" =~ ^[1-9][0-9]*$ \
        && "$digest" =~ ^[0-9a-f]{64}$ ]] || return 1
      printf 'entry:%s:%s:%s:%s:%s\n' "$base" "$mode" "$size" "$links" "$digest" \
        >>"$staging" 2>/dev/null || return 1
    done < <(find "$directory" -mindepth 1 -maxdepth 1 -print0 2>/dev/null | sort -z)
  fi
  sha256sum -- "$staging" 2>/dev/null | awk '{print $1}'
}

restore_full_suite_music_level() {
  local body="$1" original_level="$2" failure_prefix="$3"
  local restore_program status observed_level payload
  restore_program="${PREFIX:-}/bin/termux-volume"
  [[ "$restore_program" == /* && -f "$restore_program" && ! -L "$restore_program" \
    && -x "$restore_program" ]] || fail "${failure_prefix}_restore_program_invalid"
  timeout -k 2 10 "$restore_program" music "$original_level" >/dev/null 2>&1 \
    || fail "${failure_prefix}_restore_failed"
  payload='{"jsonrpc":"2.0","id":"full-suite-volume-restore-verify","method":"tools/call","params":{"name":"android_volume_status","arguments":{}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  [[ "$status" == 200 ]] || fail "${failure_prefix}_restore_verification_failed"
  observed_level="$(jq -r '.result.structuredContent.streams[] | select(.stream == "music") | .volume' "$body")"
  [[ "$observed_level" =~ ^[0-9]+$ && "$observed_level" == "$original_level" ]] \
    || fail "${failure_prefix}_restore_verification_failed"
}

run_full_suite_single_gate_check() {
  local posture="$1" enabled_tool="$2" evidence_code="$3"
  local body="$TEMP_ROOT/${posture}-response.json"
  local headers="$TEMP_ROOT/${posture}-headers.txt"
  local status payload

  start_server "$FULL_SUITE_PINNED_ARTIFACT" "$posture"
  curl_local -fsS -o "$body" "http://$BIND_HOST:$PORT/ready" 2>/dev/null \
    || fail "${posture}_readiness_failed"
  jq -e --arg version "$EXPECTED_VERSION" '
    .status == "ready"
    and .version == $version
    and .mcp_runtime_enabled == true
    and .safe_root_count == 1
    and .auth_posture == "static_token"
  ' "$body" >/dev/null 2>&1 || fail "${posture}_readiness_invalid"

  payload='{"jsonrpc":"2.0","id":"full-suite-single-gate-initialize","method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"release-validator","version":"1.0.0"}}}'
  stage_request "$payload"
  status="$(curl_local -sS -D "$headers" -o "$body" -w '%{http_code}' \
    -H "@$AUTH_HEADER_FILE" \
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
    --data-binary "@$REQUEST_FILE" "http://$BIND_HOST:$PORT/mcp")"
  [[ "$status" == 200 ]] || fail "${posture}_initialize_failed"
  MCP_SESSION_ID="$(awk 'tolower($1) == "mcp-session-id:" {sub(/^[^:]*:[[:space:]]*/, ""); sub(/\r$/, ""); print; exit}' "$headers")"
  [[ "$MCP_SESSION_ID" =~ ^[A-Za-z0-9-]{1,128}$ ]] || fail "${posture}_session_header_invalid"

  payload='{"jsonrpc":"2.0","method":"notifications/initialized"}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  [[ "$status" == 202 && ! -s "$body" ]] || fail "${posture}_initialized_notification_invalid"

  payload='{"jsonrpc":"2.0","id":"full-suite-single-gate-tools","method":"tools/list"}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  [[ "$status" == 200 ]] || fail "${posture}_tool_discovery_failed"
  jq -e --arg enabled "$enabled_tool" '
    [.result.tools[].name] == ([
      "runtime_status","platform_info","android_status","project_service_status",
      "create_directory","copy_file","trash_file","find_paths","hash_file",
      "list_directory","path_metadata","read_binary_file","read_binary_range",
      "read_file","read_text_range","search_text","write_file"
    ] + [$enabled])
  ' "$body" >/dev/null 2>&1 || fail "${posture}_discovery_invalid"

  payload='{"jsonrpc":"2.0","id":"full-suite-single-gate-status","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  [[ "$status" == 200 ]] || fail "${posture}_runtime_status_failed"
  jq -e --arg enabled "$enabled_tool" '
    .result.structuredContent as $status
    | $status.availableTools == ([
        "runtime_status","platform_info","android_status","project_service_status",
        "create_directory","copy_file","trash_file","find_paths","hash_file",
        "list_directory","path_metadata","read_binary_file","read_binary_range",
        "read_file","read_text_range","search_text","write_file"
      ] + [$enabled])
      and $status.androidBatteryStatusCompiled == true
      and $status.androidVolumeStatusCompiled == true
      and $status.androidVolumeControlCompiled == true
      and $status.commandExecutionCompiled == true
      and $status.androidBatteryStatusEnabled == ($enabled == "android_battery_status")
      and $status.androidVolumeStatusEnabled == ($enabled == "android_volume_status")
      and $status.androidVolumeControlEnabled == ($enabled == "set_android_volume")
      and $status.androidVolumeGrantRequired == ($enabled == "set_android_volume")
      and $status.commandExecution == ($enabled == "run_command_profile")
      and $status.arbitraryCommandExecution == false
      and $status.androidPlatformTools == ($enabled != "run_command_profile")
      and $status.highImpactTools == ($enabled == "set_android_volume")
      and $status.createDirectoryMutationEnabled == false
      and $status.copyFileMutationEnabled == false
      and $status.trashFileMutationEnabled == false
      and $status.fileWriteMutationEnabled == false
      and $status.createDirectoryGrantRequired == false
      and $status.copyFileGrantRequired == false
      and $status.trashFileGrantRequired == false
      and $status.fileWriteGrantRequired == false
  ' "$body" >/dev/null 2>&1 || fail "${posture}_runtime_status_invalid"

  case "$enabled_tool" in
    android_battery_status)
      payload='{"jsonrpc":"2.0","id":"full-suite-battery-only-call","method":"tools/call","params":{"name":"android_battery_status","arguments":{}}}'
      status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
      [[ "$status" == 200 ]] || fail "${posture}_call_failed"
      jq -e '
        .result.isError == false
        and (.result.structuredContent | type) == "object"
        and (.result.structuredContent | keys | length) >= 1
        and ((.result.structuredContent | keys) - [
          "charge_counter_microamp_hours","current_average_microamps","current_microamps",
          "cycle_count","energy_nanowatt_hours","health","level","percentage","plugged",
          "present","scale","status","temperature_celsius","voltage_millivolts"
        ] | length) == 0
      ' "$body" >/dev/null 2>&1 || fail "${posture}_call_contract_invalid"
      ;;
    android_volume_status)
      payload='{"jsonrpc":"2.0","id":"full-suite-volume-status-only-call","method":"tools/call","params":{"name":"android_volume_status","arguments":{}}}'
      status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
      [[ "$status" == 200 ]] || fail "${posture}_call_failed"
      jq -e '
        .result.isError == false
        and [.result.structuredContent.streams[].stream] == ["alarm","call","music","notification","ring","system"]
        and all(.result.structuredContent.streams[]; .volume >= 0 and .volume <= .maxVolume)
      ' "$body" >/dev/null 2>&1 || fail "${posture}_call_contract_invalid"
      ;;
    set_android_volume)
      payload='{"jsonrpc":"2.0","id":"full-suite-volume-control-only-call","method":"tools/call","params":{"name":"set_android_volume","arguments":{"stream":"music","level":0,"dry_run":true}}}'
      status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
      [[ "$status" == 200 ]] || fail "${posture}_call_failed"
      jq -e '
        .result.isError == false
        and .result.structuredContent.stream == "music"
        and .result.structuredContent.requestedLevel == 0
        and .result.structuredContent.dryRun == true
        and .result.structuredContent.changed == false
        and .result.structuredContent.verified == false
        and .result.structuredContent.outcome == "preview"
      ' "$body" >/dev/null 2>&1 || fail "${posture}_call_contract_invalid"
      ;;
    run_command_profile)
      payload='{"jsonrpc":"2.0","id":"full-suite-command-only-call","method":"tools/call","params":{"name":"run_command_profile","arguments":{"profile":"server_version"}}}'
      status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
      [[ "$status" == 200 ]] || fail "${posture}_call_failed"
      jq -e --arg version "$EXPECTED_VERSION" '
        .result.isError == false
        and .result.structuredContent.profile == "server_version"
        and .result.structuredContent.exitCode == 0
        and .result.structuredContent.stdout == ("termux-mcp-server " + $version + "\n")
        and .result.structuredContent.stderr == ""
      ' "$body" >/dev/null 2>&1 || fail "${posture}_call_contract_invalid"
      ;;
    *) fail full_suite_single_gate_internal_error ;;
  esac

  stage_session_headers "$MCP_SESSION_ID"
  status="$(curl_local -sS -X DELETE -o "$body" -w '%{http_code}' \
    -H "@$SESSION_HEADER_FILE" \
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
    "http://$BIND_HOST:$PORT/mcp")"
  [[ "$status" == 204 && ! -s "$body" ]] || fail "${posture}_session_delete_invalid"
  MCP_SESSION_ID=""
  stop_server || fail "${posture}_runtime_stop_failed"
  record_result runtime "$posture" pass "$evidence_code"
}

run_full_suite_runtime_checks() {
  local body="$TEMP_ROOT/full-suite-response.json"
  local headers="$TEMP_ROOT/full-suite-headers.txt"
  local status payload music_level music_max music_target music_after
  local fs_source fs_create_target fs_copy_target fs_write_target trash_quarantine
  local fs_source_before fs_source_after fs_source_sha_before fs_source_sha_after
  local quarantine_before_file quarantine_after_file quarantine_before quarantine_after

  start_server "$FULL_SUITE_PINNED_ARTIFACT" full_suite_disabled
  curl_local -fsS -o "$body" "http://$BIND_HOST:$PORT/ready" 2>/dev/null \
    || fail full_suite_default_readiness_failed
  jq -e --arg version "$EXPECTED_VERSION" '
    .status == "ready"
    and .version == $version
    and .mcp_runtime_enabled == true
    and .safe_root_count == 1
    and .auth_posture == "static_token"
    and .mcp_request_limits.max_concurrent_requests == 4
    and .mcp_request_limits.request_timeout_seconds == 30
    and .mcp_request_limits.max_body_bytes == 1024
    and .mcp_request_limits.sse_enabled == false
  ' "$body" >/dev/null 2>&1 || fail full_suite_default_feature_posture_mismatch

  payload='{"jsonrpc":"2.0","id":"full-suite-default-initialize","method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"release-validator","version":"1.0.0"}}}'
  stage_request "$payload"
  status="$(curl_local -sS -D "$headers" -o "$body" -w '%{http_code}' \
    -H "@$AUTH_HEADER_FILE" \
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
    --data-binary "@$REQUEST_FILE" "http://$BIND_HOST:$PORT/mcp")"
  expect_status full_suite_default_initialize "$status" 200 full_suite_default_initialize_succeeded
  MCP_SESSION_ID="$(awk 'tolower($1) == "mcp-session-id:" {sub(/^[^:]*:[[:space:]]*/, ""); sub(/\r$/, ""); print; exit}' "$headers")"
  [[ "$MCP_SESSION_ID" =~ ^[A-Za-z0-9-]{1,128}$ ]] || fail full_suite_default_session_header_invalid

  payload='{"jsonrpc":"2.0","method":"notifications/initialized"}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status full_suite_default_initialized_notification "$status" 202 full_suite_default_initialized_notification_accepted
  [[ ! -s "$body" ]] || fail full_suite_default_initialized_notification_body_present

  payload='{"jsonrpc":"2.0","id":"full-suite-default-tools","method":"tools/list"}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status full_suite_default_tool_discovery "$status" 200 full_suite_default_tool_discovery_succeeded
  jq -e '
    [.result.tools[].name] == [
      "runtime_status","platform_info","android_status","project_service_status",
      "create_directory","copy_file","trash_file","find_paths","hash_file",
      "list_directory","path_metadata","read_binary_file","read_binary_range",
      "read_file","read_text_range","search_text","write_file"
    ]
    and (all(
      .result.tools[]
      | select(.name == "create_directory" or .name == "copy_file" or .name == "trash_file" or .name == "write_file");
      .inputSchema.properties.dry_run.const == true
    ))
  ' "$body" >/dev/null 2>&1 || fail full_suite_default_discovery_invalid

  payload='{"jsonrpc":"2.0","id":"full-suite-default-status","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status full_suite_default_runtime_status "$status" 200 full_suite_default_runtime_status_read
  jq -e '
    .result.structuredContent as $status
    | $status.availableTools == [
        "runtime_status","platform_info","android_status","project_service_status",
        "create_directory","copy_file","trash_file","find_paths","hash_file",
        "list_directory","path_metadata","read_binary_file","read_binary_range",
        "read_file","read_text_range","search_text","write_file"
      ]
      and $status.androidBatteryStatusCompiled == true
      and $status.androidBatteryStatusEnabled == false
      and $status.androidVolumeStatusCompiled == true
      and $status.androidVolumeStatusEnabled == false
      and $status.androidVolumeControlCompiled == true
      and $status.androidVolumeControlEnabled == false
      and $status.androidVolumeGrantRequired == false
      and $status.commandExecutionCompiled == true
      and $status.commandExecution == false
      and $status.arbitraryCommandExecution == false
      and $status.androidPlatformTools == false
      and $status.highImpactTools == false
      and $status.createDirectoryMutationEnabled == false
      and $status.copyFileMutationEnabled == false
      and $status.trashFileMutationEnabled == false
      and $status.fileWriteMutationEnabled == false
      and $status.createDirectoryGrantRequired == false
      and $status.copyFileGrantRequired == false
      and $status.trashFileGrantRequired == false
      and $status.fileWriteGrantRequired == false
  ' "$body" >/dev/null 2>&1 || fail full_suite_default_runtime_status_invalid

  local disabled_tool disabled_reason
  for disabled_tool in android_battery_status android_volume_status set_android_volume run_command_profile; do
    case "$disabled_tool" in
      android_battery_status)
        disabled_reason=battery_runtime_disabled
        payload='{"jsonrpc":"2.0","id":"full-suite-disabled-battery","method":"tools/call","params":{"name":"android_battery_status","arguments":{}}}'
        ;;
      android_volume_status)
        disabled_reason=volume_runtime_disabled
        payload='{"jsonrpc":"2.0","id":"full-suite-disabled-volume","method":"tools/call","params":{"name":"android_volume_status","arguments":{}}}'
        ;;
      set_android_volume)
        disabled_reason=volume_control_runtime_disabled
        payload='{"jsonrpc":"2.0","id":"full-suite-disabled-volume-control","method":"tools/call","params":{"name":"set_android_volume","arguments":{"stream":"music","level":0}}}'
        ;;
      run_command_profile)
        disabled_reason=command_runtime_disabled
        payload='{"jsonrpc":"2.0","id":"full-suite-disabled-command","method":"tools/call","params":{"name":"run_command_profile","arguments":{"profile":"server_version"}}}'
        ;;
    esac
    status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
    [[ "$status" == 200 ]] || fail full_suite_default_optional_call_status_invalid
    jq -e --arg reason "$disabled_reason" '
      .result.isError == true and .result.structuredContent.reasonCode == $reason
    ' "$body" >/dev/null 2>&1 || fail full_suite_default_optional_call_invalid
  done
  record_result runtime full_suite_default_posture pass full_suite_default_disabled_17_tool_posture_verified

  stage_session_headers "$MCP_SESSION_ID"
  status="$(curl_local -sS -X DELETE -o "$body" -w '%{http_code}' \
    -H "@$SESSION_HEADER_FILE" \
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
    "http://$BIND_HOST:$PORT/mcp")"
  expect_status full_suite_default_session_delete "$status" 204 full_suite_default_session_deleted
  [[ ! -s "$body" ]] || fail full_suite_default_session_delete_body_present
  MCP_SESSION_ID=""
  stop_server || fail full_suite_default_runtime_stop_failed

  run_full_suite_single_gate_check \
    full_suite_battery_only android_battery_status \
    full_suite_battery_runtime_gate_independence_verified
  run_full_suite_single_gate_check \
    full_suite_volume_status_only android_volume_status \
    full_suite_volume_status_runtime_gate_independence_verified
  run_full_suite_single_gate_check \
    full_suite_volume_control_only set_android_volume \
    full_suite_volume_control_runtime_gate_independence_verified
  run_full_suite_single_gate_check \
    full_suite_command_only run_command_profile \
    full_suite_command_runtime_gate_independence_verified

  start_server "$FULL_SUITE_PINNED_ARTIFACT" full_suite_enabled
  curl_local -fsS -o "$body" "http://$BIND_HOST:$PORT/ready" 2>/dev/null \
    || fail full_suite_enabled_readiness_failed
  jq -e --arg version "$EXPECTED_VERSION" '
    .status == "ready"
    and .version == $version
    and .mcp_runtime_enabled == true
    and .safe_root_count == 1
    and .auth_posture == "static_token"
  ' "$body" >/dev/null 2>&1 || fail full_suite_enabled_feature_posture_mismatch

  payload='{"jsonrpc":"2.0","id":"full-suite-enabled-initialize","method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"release-validator","version":"1.0.0"}}}'
  stage_request "$payload"
  status="$(curl_local -sS -D "$headers" -o "$body" -w '%{http_code}' \
    -H "@$AUTH_HEADER_FILE" \
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
    --data-binary "@$REQUEST_FILE" "http://$BIND_HOST:$PORT/mcp")"
  expect_status full_suite_enabled_initialize "$status" 200 full_suite_enabled_initialize_succeeded
  MCP_SESSION_ID="$(awk 'tolower($1) == "mcp-session-id:" {sub(/^[^:]*:[[:space:]]*/, ""); sub(/\r$/, ""); print; exit}' "$headers")"
  [[ "$MCP_SESSION_ID" =~ ^[A-Za-z0-9-]{1,128}$ ]] || fail full_suite_enabled_session_header_invalid

  payload='{"jsonrpc":"2.0","method":"notifications/initialized"}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status full_suite_enabled_initialized_notification "$status" 202 full_suite_enabled_initialized_notification_accepted

  payload='{"jsonrpc":"2.0","id":"full-suite-enabled-tools","method":"tools/list"}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status full_suite_enabled_tool_discovery "$status" 200 full_suite_enabled_tool_discovery_succeeded
  jq -e '
    [.result.tools[].name] == [
      "runtime_status","platform_info","android_status","project_service_status",
      "create_directory","copy_file","trash_file","find_paths","hash_file",
      "list_directory","path_metadata","read_binary_file","read_binary_range",
      "read_file","read_text_range","search_text","write_file",
      "android_battery_status","android_volume_status","set_android_volume","run_command_profile"
    ]
    and (.result.tools[] | select(.name == "android_battery_status") | .inputSchema) == {type:"object",properties:{},additionalProperties:false}
    and (.result.tools[] | select(.name == "android_volume_status") | .inputSchema) == {type:"object",properties:{},additionalProperties:false}
    and (.result.tools[] | select(.name == "set_android_volume") | .inputSchema.required) == ["stream","level"]
    and (.result.tools[] | select(.name == "set_android_volume") | .inputSchema.additionalProperties) == false
    and (.result.tools[] | select(.name == "run_command_profile") | .inputSchema.properties.profile.enum) == ["server_version","server_help","execution_boundary"]
    and (.result.tools[] | select(.name == "run_command_profile") | .inputSchema.additionalProperties) == false
  ' "$body" >/dev/null 2>&1 || fail full_suite_enabled_discovery_invalid
  record_result runtime full_suite_enabled_posture pass full_suite_enabled_21_tool_posture_verified

  payload='{"jsonrpc":"2.0","id":"full-suite-enabled-status","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status full_suite_enabled_runtime_status "$status" 200 full_suite_enabled_runtime_status_read
  jq -e '
    .result.structuredContent as $status
    | ($status.availableTools | length) == 21
      and $status.androidBatteryStatusCompiled == true
      and $status.androidBatteryStatusEnabled == true
      and $status.androidVolumeStatusCompiled == true
      and $status.androidVolumeStatusEnabled == true
      and $status.androidVolumeControlCompiled == true
      and $status.androidVolumeControlEnabled == true
      and $status.androidVolumeGrantRequired == true
      and $status.commandExecutionCompiled == true
      and $status.commandExecution == true
      and $status.arbitraryCommandExecution == false
      and $status.androidPlatformTools == true
      and $status.highImpactTools == true
      and $status.createDirectoryMutationEnabled == false
      and $status.copyFileMutationEnabled == false
      and $status.trashFileMutationEnabled == false
      and $status.fileWriteMutationEnabled == false
      and $status.createDirectoryGrantRequired == false
      and $status.copyFileGrantRequired == false
      and $status.trashFileGrantRequired == false
      and $status.fileWriteGrantRequired == false
  ' "$body" >/dev/null 2>&1 || fail full_suite_enabled_runtime_status_invalid

  payload='{"jsonrpc":"2.0","id":"full-suite-battery","method":"tools/call","params":{"name":"android_battery_status","arguments":{}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status full_suite_battery "$status" 200 full_suite_battery_succeeded
  jq -e '
    .result.isError == false
    and (.result.structuredContent | type) == "object"
    and ((.result.structuredContent | keys) - [
      "charge_counter_microamp_hours","current_average_microamps","current_microamps",
      "cycle_count","energy_nanowatt_hours","health","level","percentage","plugged",
      "present","scale","status","temperature_celsius","voltage_millivolts"
    ] | length) == 0
  ' "$body" >/dev/null 2>&1 || fail full_suite_battery_contract_invalid

  payload='{"jsonrpc":"2.0","id":"full-suite-volume","method":"tools/call","params":{"name":"android_volume_status","arguments":{}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status full_suite_volume "$status" 200 full_suite_volume_succeeded
  jq -e '
    .result.isError == false
    and [.result.structuredContent.streams[].stream] == ["alarm","call","music","notification","ring","system"]
    and all(.result.structuredContent.streams[];
      (.volume | type) == "number"
      and (.maxVolume | type) == "number"
      and .volume >= 0
      and .volume <= .maxVolume
    )
  ' "$body" >/dev/null 2>&1 || fail full_suite_volume_contract_invalid
  music_level="$(jq -r '.result.structuredContent.streams[] | select(.stream == "music") | .volume' "$body")"
  music_max="$(jq -r '.result.structuredContent.streams[] | select(.stream == "music") | .maxVolume' "$body")"
  [[ "$music_level" =~ ^[0-9]+$ && "$music_max" =~ ^[1-9][0-9]*$ && "$music_level" -le "$music_max" ]] \
    || fail full_suite_volume_distinct_target_unavailable
  if ((music_level < music_max)); then
    music_target=$((music_level + 1))
  else
    music_target=$((music_level - 1))
  fi
  record_result runtime full_suite_optional_providers pass full_suite_optional_provider_success_verified

  payload="$(jq -cn --argjson level "$music_target" \
    '{jsonrpc:"2.0",id:"full-suite-volume-preview",method:"tools/call",params:{name:"set_android_volume",arguments:{stream:"music",level:$level,dry_run:true}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status full_suite_volume_preview "$status" 200 full_suite_volume_preview_succeeded
  jq -e --argjson level "$music_target" --argjson previous "$music_level" '
    .result.isError == false
    and .result.structuredContent.stream == "music"
    and .result.structuredContent.previousLevel == $previous
    and .result.structuredContent.requestedLevel == $level
    and .result.structuredContent.dryRun == true
    and .result.structuredContent.changed == false
    and .result.structuredContent.verified == false
    and .result.structuredContent.outcome == "preview"
    and .result.structuredContent.rollback == "not_required"
  ' "$body" >/dev/null 2>&1 || fail full_suite_volume_preview_contract_invalid

  payload='{"jsonrpc":"2.0","id":"full-suite-volume-after-preview","method":"tools/call","params":{"name":"android_volume_status","arguments":{}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status full_suite_volume_after_preview "$status" 200 full_suite_volume_after_preview_succeeded
  music_after="$(jq -r '.result.structuredContent.streams[] | select(.stream == "music") | .volume' "$body")"
  if [[ "$music_after" != "$music_level" ]]; then
    restore_full_suite_music_level "$body" "$music_level" full_suite_volume_preview
    fail full_suite_volume_preview_mutated
  fi

  payload="$(jq -cn --argjson level "$music_target" \
    '{jsonrpc:"2.0",id:"full-suite-volume-missing-grant",method:"tools/call",params:{name:"set_android_volume",arguments:{stream:"music",level:$level,dry_run:false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status full_suite_volume_missing_grant "$status" 403 full_suite_volume_missing_grant_rejected
  jq -e '.error.code == -32003 and .error.data.reason == "capability_grant_missing"' "$body" >/dev/null 2>&1 \
    || fail full_suite_volume_missing_grant_body_invalid

  payload='{"jsonrpc":"2.0","id":"full-suite-volume-after-denial","method":"tools/call","params":{"name":"android_volume_status","arguments":{}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status full_suite_volume_after_denial "$status" 200 full_suite_volume_after_denial_succeeded
  music_after="$(jq -r '.result.structuredContent.streams[] | select(.stream == "music") | .volume' "$body")"
  if [[ "$music_after" != "$music_level" ]]; then
    restore_full_suite_music_level "$body" "$music_level" full_suite_volume_missing_grant
    fail full_suite_volume_missing_grant_mutated
  fi
  record_result runtime full_suite_volume_boundary pass full_suite_volume_preview_and_grant_boundary_verified

  payload='{"jsonrpc":"2.0","id":"full-suite-command","method":"tools/call","params":{"name":"run_command_profile","arguments":{"profile":"server_version"}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status full_suite_command "$status" 200 full_suite_command_succeeded
  jq -e --arg version "$EXPECTED_VERSION" '
    .result.isError == false
    and .result.structuredContent.profile == "server_version"
    and .result.structuredContent.exitCode == 0
    and .result.structuredContent.stdout == ("termux-mcp-server " + $version + "\n")
    and .result.structuredContent.stderr == ""
    and .result.structuredContent.stdoutBytes == (.result.structuredContent.stdout | utf8bytelength)
    and .result.structuredContent.stderrBytes == 0
    and (.result.structuredContent.durationMilliseconds | type) == "number"
  ' "$body" >/dev/null 2>&1 || fail full_suite_command_contract_invalid
  record_result runtime full_suite_command_profile pass full_suite_command_basename_and_profile_verified

  fs_source="$VALIDATION_SAFE_ROOT/full-suite-disabled-source.txt"
  fs_create_target="$VALIDATION_SAFE_ROOT/full-suite-disabled-directory"
  fs_copy_target="$VALIDATION_SAFE_ROOT/full-suite-disabled-copy.txt"
  fs_write_target="$VALIDATION_SAFE_ROOT/full-suite-disabled-write.txt"
  trash_quarantine="$VALIDATION_SAFE_ROOT/.termux-mcp-trash-quarantine"
  quarantine_before_file="$TEMP_ROOT/full-suite-quarantine-before.txt"
  quarantine_after_file="$TEMP_ROOT/full-suite-quarantine-after.txt"
  [[ ! -e "$fs_source" && ! -L "$fs_source" \
    && ! -e "$fs_create_target" && ! -L "$fs_create_target" \
    && ! -e "$fs_copy_target" && ! -L "$fs_copy_target" \
    && ! -e "$fs_write_target" && ! -L "$fs_write_target" ]] \
    || fail full_suite_filesystem_fixture_exists
  printf '%s' full-suite-disabled-source >"$fs_source" 2>/dev/null \
    || fail full_suite_filesystem_fixture_create_failed
  chmod 600 "$fs_source" 2>/dev/null || fail full_suite_filesystem_fixture_create_failed
  fs_source_before="$(stat -c '%d:%i:%a:%s:%Y:%Z' "$fs_source" 2>/dev/null)" \
    || fail full_suite_filesystem_fixture_invalid
  fs_source_sha_before="$(sha256sum -- "$fs_source" 2>/dev/null | awk '{print $1}')" \
    || fail full_suite_filesystem_fixture_invalid
  quarantine_before="$(snapshot_private_directory "$trash_quarantine" "$quarantine_before_file")" \
    || fail full_suite_filesystem_quarantine_snapshot_failed

  payload="$(jq -cn --arg path "$fs_create_target" \
    '{jsonrpc:"2.0",id:"full-suite-create-disabled",method:"tools/call",params:{name:"create_directory",arguments:{path:$path,dry_run:false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  [[ "$status" == 403 ]] || fail full_suite_create_directory_disabled_status_invalid
  jq -e '.error.code == -32003 and .error.data.reason == "create_directory_mutation_disabled"' "$body" >/dev/null 2>&1 \
    || fail full_suite_create_directory_disabled_body_invalid

  payload="$(jq -cn --arg source "$fs_source" --arg destination "$fs_copy_target" \
    '{jsonrpc:"2.0",id:"full-suite-copy-disabled",method:"tools/call",params:{name:"copy_file",arguments:{source_path:$source,destination_path:$destination,dry_run:false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  [[ "$status" == 403 ]] || fail full_suite_copy_file_disabled_status_invalid
  jq -e '.error.code == -32003 and .error.data.reason == "copy_file_mutation_disabled"' "$body" >/dev/null 2>&1 \
    || fail full_suite_copy_file_disabled_body_invalid

  payload="$(jq -cn --arg path "$fs_source" \
    '{jsonrpc:"2.0",id:"full-suite-trash-disabled",method:"tools/call",params:{name:"trash_file",arguments:{path:$path,dry_run:false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  [[ "$status" == 403 ]] || fail full_suite_trash_file_disabled_status_invalid
  jq -e '.error.code == -32003 and .error.data.reason == "trash_file_mutation_disabled"' "$body" >/dev/null 2>&1 \
    || fail full_suite_trash_file_disabled_body_invalid

  payload="$(jq -cn --arg path "$fs_write_target" \
    '{jsonrpc:"2.0",id:"full-suite-write-disabled",method:"tools/call",params:{name:"write_file",arguments:{path:$path,content:"full-suite-disabled-write",dry_run:false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  [[ "$status" == 403 ]] || fail full_suite_write_file_disabled_status_invalid
  jq -e '.error.code == -32003 and .error.data.reason == "write_file_mutation_disabled"' "$body" >/dev/null 2>&1 \
    || fail full_suite_write_file_disabled_body_invalid

  fs_source_after="$(stat -c '%d:%i:%a:%s:%Y:%Z' "$fs_source" 2>/dev/null)" \
    || fail full_suite_filesystem_disabled_mutated
  fs_source_sha_after="$(sha256sum -- "$fs_source" 2>/dev/null | awk '{print $1}')" \
    || fail full_suite_filesystem_disabled_mutated
  quarantine_after="$(snapshot_private_directory "$trash_quarantine" "$quarantine_after_file")" \
    || fail full_suite_filesystem_quarantine_snapshot_failed
  [[ "$fs_source_after" == "$fs_source_before" \
    && "$fs_source_sha_after" == "$fs_source_sha_before" \
    && "$quarantine_after" == "$quarantine_before" \
    && ! -e "$fs_create_target" && ! -L "$fs_create_target" \
    && ! -e "$fs_copy_target" && ! -L "$fs_copy_target" \
    && ! -e "$fs_write_target" && ! -L "$fs_write_target" ]] \
    || fail full_suite_filesystem_disabled_mutated
  rm -f -- "$fs_source" "$quarantine_before_file" "$quarantine_after_file" 2>/dev/null \
    || fail full_suite_filesystem_fixture_cleanup_failed
  record_result runtime full_suite_filesystem_posture pass full_suite_filesystem_mutations_independently_disabled

  stage_session_headers "$MCP_SESSION_ID"
  status="$(curl_local -sS -X DELETE -o "$body" -w '%{http_code}' \
    -H "@$SESSION_HEADER_FILE" \
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
    "http://$BIND_HOST:$PORT/mcp")"
  expect_status full_suite_enabled_session_delete "$status" 204 full_suite_enabled_session_deleted
  [[ ! -s "$body" ]] || fail full_suite_enabled_session_delete_body_present
  MCP_SESSION_ID=""
  stop_server || fail full_suite_enabled_runtime_stop_failed
}

run_runtime_phase() {
  CURRENT_PHASE=runtime
  set_phase runtime running
  ((CONFIRM_RUNTIME == 1)) || fail runtime_confirmation_missing
  prepare_runtime_inputs
  run_default_runtime_checks
  run_mcp_runtime_checks
  run_write_file_boundary_checks
  run_volume_control_runtime_checks
  run_full_suite_runtime_checks
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
MCP__TRANSPORT__SSE_ENABLED=false
MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS=4
MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS=30
MCP__TRANSPORT__MAX_BODY_BYTES=1024
MCP__FILE__SAFE_ROOTS=$VALIDATION_SAFE_ROOT
MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=false
MCP__FILE__COPY_FILE_MUTATION_ENABLED=false
MCP__FILE__TRASH_FILE_MUTATION_ENABLED=false
MCP__FILE__WRITE_MUTATION_ENABLED=false
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
  record_result deployment deployment_candidate_posture pass full_suite_deployment_candidate_selected

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
      bash "$DEPLOY_SCRIPT" upgrade --artifact "$FULL_SUITE_PINNED_ARTIFACT" --version "$EXPECTED_VERSION" --sha256 "$FULL_SUITE_SHA256"
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
      bash "$DEPLOY_SCRIPT" upgrade --artifact "$FULL_SUITE_PINNED_ARTIFACT" --version "$EXPECTED_VERSION" --sha256 "$FULL_SUITE_SHA256"
  fi
  link_equals "$DEDICATED_DEPLOY_ROOT/current" "$baseline_release" || fail candidate_failure_recovery_invalid
  [[ ! -e "$candidate_release" ]] || fail failed_candidate_not_removed

  run_deploy_success upgrade_candidate \
    bash "$DEPLOY_SCRIPT" upgrade --artifact "$FULL_SUITE_PINNED_ARTIFACT" --version "$EXPECTED_VERSION" --sha256 "$FULL_SUITE_SHA256"
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
  record_result deployment production_candidate_posture pass full_suite_production_candidate_selected
  case "$PRODUCTION_ACTION" in
    install|upgrade)
      run_deploy_success "production_$PRODUCTION_ACTION" \
        bash "$DEPLOY_SCRIPT" "$PRODUCTION_ACTION" --artifact "$FULL_SUITE_PINNED_ARTIFACT" --version "$EXPECTED_VERSION" --sha256 "$FULL_SUITE_SHA256"
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
        bash "$DEPLOY_SCRIPT" upgrade --artifact "$FULL_SUITE_PINNED_ARTIFACT" --version "$EXPECTED_VERSION" --sha256 "$FULL_SUITE_SHA256"
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
