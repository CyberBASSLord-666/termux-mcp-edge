#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

PROGRAM="termux-mcp-server"
SERVICE_NAME="mcp_runtime"
TERMUX_PREFIX="${PREFIX:-/data/data/com.termux/files/usr}"
DEPLOY_ROOT="${TERMUX_MCP_DEPLOY_ROOT:-${HOME}/.local/share/termux-mcp-edge}"
CONFIG_ROOT="${TERMUX_MCP_CONFIG_ROOT:-${HOME}/.config/termux-mcp-edge}"
SERVICE_ROOT="${TERMUX_MCP_SERVICE_ROOT:-${TERMUX_PREFIX}/var/service}"
SERVICE_SHELL="${TERMUX_MCP_SERVICE_SHELL:-${TERMUX_PREFIX}/bin/sh}"
HEALTH_URL="${TERMUX_MCP_HEALTH_URL:-http://127.0.0.1:8000/health}"
READY_URL="${TERMUX_MCP_READY_URL:-http://127.0.0.1:8000/ready}"
PROBE_ATTEMPTS="${TERMUX_MCP_PROBE_ATTEMPTS:-15}"
PROBE_DELAY_SECONDS="${TERMUX_MCP_PROBE_DELAY_SECONDS:-1}"
TEST_MODE="${TERMUX_MCP_TEST_MODE:-0}"
TEST_PROBE_RESULT="${TERMUX_MCP_TEST_PROBE_RESULT:-success}"
TEST_PROBE_SEQUENCE="${TERMUX_MCP_TEST_PROBE_SEQUENCE:-}"
TEST_PROBE_INDEX=0
DRY_RUN="${TERMUX_MCP_DRY_RUN:-0}"

usage() {
  cat <<'EOF'
Usage:
  termux_deploy.sh install  --artifact PATH --version VERSION
  termux_deploy.sh upgrade  --artifact PATH --version VERSION
  termux_deploy.sh rollback
  termux_deploy.sh status
  termux_deploy.sh uninstall [--purge-config]

Environment overrides:
  TERMUX_MCP_DEPLOY_ROOT, TERMUX_MCP_CONFIG_ROOT, TERMUX_MCP_SERVICE_ROOT
  TERMUX_MCP_SERVICE_SHELL, TERMUX_MCP_HEALTH_URL, TERMUX_MCP_READY_URL
  TERMUX_MCP_PROBE_ATTEMPTS, TERMUX_MCP_PROBE_DELAY_SECONDS
  TERMUX_MCP_TEST_MODE=1       Skip live runit operations and use test probes.
  TERMUX_MCP_TEST_PROBE_RESULT Test-only probe result: success or failure.
  TERMUX_MCP_TEST_PROBE_SEQUENCE
                               Test-only comma-separated probe sequence.
  TERMUX_MCP_DRY_RUN=1         Print mutations without applying them.
EOF
}

log() { printf '[termux-deploy] %s\n' "$*"; }
fail() { printf '[termux-deploy] ERROR: %s\n' "$*" >&2; exit 1; }

run() {
  if [[ "$DRY_RUN" == "1" ]]; then
    printf '[termux-deploy] DRY-RUN:'
    printf ' %q' "$@"
    printf '\n'
  else
    "$@"
  fi
}

require_command() { command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"; }

validate_positive_integer() {
  local name="$1" value="$2"
  [[ "$value" =~ ^[1-9][0-9]*$ ]] || fail "$name must be a positive integer"
}

validate_absolute_safe_path() {
  local path="$1"
  [[ "$path" == /* ]] || fail "path must be absolute: $path"
  [[ "$path" != "/" ]] || fail "refusing root path"
  case "$path" in
    *$'\n'*|*$'\r'*|*".."*) fail "unsafe path: $path" ;;
  esac
}

validate_version() {
  [[ "$1" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] || fail "invalid version: $1"
}

validate_runtime_env_file() {
  local config_file="$CONFIG_ROOT/runtime.env"
  [[ -e "$config_file" ]] || return 0
  [[ -f "$config_file" && ! -L "$config_file" ]] || fail "runtime.env must be a regular file"

  local mode
  mode="$(stat -c '%a' "$config_file")"
  local permissions=$((8#$mode))
  (( (permissions & 077) == 0 )) || fail "runtime.env must not be group- or world-accessible"

  local line line_number=0 key value
  while IFS= read -r line || [[ -n "$line" ]]; do
    ((line_number += 1))
    case "$line" in
      ''|'#'*) continue ;;
      *=*) ;;
      *) fail "runtime.env line $line_number must use NAME=VALUE syntax" ;;
    esac
    key="${line%%=*}"
    value="${line#*=}"
    [[ "$key" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]] || fail "runtime.env line $line_number has an invalid variable name"
    [[ "$value" != *$'\r'* ]] || fail "runtime.env line $line_number contains a carriage return"
  done <"$config_file"
}

ensure_layout() {
  validate_absolute_safe_path "$DEPLOY_ROOT"
  validate_absolute_safe_path "$CONFIG_ROOT"
  validate_absolute_safe_path "$SERVICE_ROOT"
  validate_absolute_safe_path "$SERVICE_SHELL"
  validate_positive_integer TERMUX_MCP_PROBE_ATTEMPTS "$PROBE_ATTEMPTS"
  validate_positive_integer TERMUX_MCP_PROBE_DELAY_SECONDS "$PROBE_DELAY_SECONDS"
  [[ -x "$SERVICE_SHELL" ]] || fail "service shell is not executable: $SERVICE_SHELL"
  run mkdir -p "$DEPLOY_ROOT/releases" "$CONFIG_ROOT" "$SERVICE_ROOT"
  run chmod 700 "$DEPLOY_ROOT" "$DEPLOY_ROOT/releases" "$CONFIG_ROOT"
  validate_runtime_env_file
}

artifact_architecture() {
  local artifact="$1"
  if command -v file >/dev/null 2>&1; then
    file -b "$artifact"
  else
    printf 'unknown'
  fi
}

artifact_version() {
  local artifact="$1" output
  require_command timeout
  output="$(timeout 5 "$artifact" --version 2>/dev/null)" || fail "artifact did not return a version within 5 seconds"
  [[ -n "$output" ]] || fail "artifact returned an empty version response"
  printf '%s\n' "${output##* }"
}

validate_artifact() {
  local artifact="$1" expected_version="$2"
  [[ -f "$artifact" ]] || fail "artifact not found: $artifact"
  [[ -s "$artifact" ]] || fail "artifact is empty: $artifact"
  [[ ! -L "$artifact" ]] || fail "artifact must not be a symlink"
  [[ -x "$artifact" ]] || fail "artifact must be executable"

  local reported_version
  reported_version="$(artifact_version "$artifact")"
  [[ "$reported_version" == "$expected_version" ]] || fail "artifact version mismatch: expected $expected_version, got $reported_version"

  local description
  description="$(artifact_architecture "$artifact")"
  if [[ "$TEST_MODE" != "1" && "$description" != "unknown" ]]; then
    case "$(uname -m)" in
      aarch64|arm64) [[ "$description" == *"aarch64"* || "$description" == *"ARM aarch64"* ]] || fail "artifact architecture mismatch: $description" ;;
      x86_64) [[ "$description" == *"x86-64"* || "$description" == *"x86_64"* ]] || fail "artifact architecture mismatch: $description" ;;
      *) fail "unsupported host architecture: $(uname -m)" ;;
    esac
  fi
}

write_service() {
  local service_dir="$SERVICE_ROOT/$SERVICE_NAME"
  local run_file="$service_dir/run"
  run mkdir -p "$service_dir/log"
  if [[ "$DRY_RUN" == "1" ]]; then
    log "would write project-owned runit service at $run_file"
    return
  fi
  cat >"$run_file" <<EOF
#!$SERVICE_SHELL
set -eu
umask 077
CONFIG_FILE="$CONFIG_ROOT/runtime.env"
load_runtime_env() {
  [ -f "\$CONFIG_FILE" ] || return 0
  while IFS= read -r line || [ -n "\$line" ]; do
    case "\$line" in
      ''|'#'*) continue ;;
      *=*) ;;
      *) printf '%s\n' 'invalid runtime.env entry: expected NAME=VALUE' >&2; exit 78 ;;
    esac
    key=\${line%%=*}
    value=\${line#*=}
    case "\$key" in
      ''|[0-9]*|*[!A-Za-z0-9_]*) printf '%s\n' 'invalid runtime.env variable name' >&2; exit 78 ;;
    esac
    case "\$value" in
      *"\$(printf '\r')"*) printf '%s\n' 'invalid carriage return in runtime.env value' >&2; exit 78 ;;
    esac
    export "\$key=\$value"
  done <"\$CONFIG_FILE"
}
load_runtime_env
exec "$DEPLOY_ROOT/current/$PROGRAM"
EOF
  chmod 700 "$run_file"
}

stop_service() {
  [[ "$TEST_MODE" == "1" ]] && return 0
  if command -v sv >/dev/null 2>&1 && [[ -d "$SERVICE_ROOT/$SERVICE_NAME" ]]; then
    run sv down "$SERVICE_ROOT/$SERVICE_NAME" || true
  fi
}

start_service() {
  [[ "$TEST_MODE" == "1" ]] && return 0
  require_command sv
  run sv up "$SERVICE_ROOT/$SERVICE_NAME"
}

probe_url() {
  local url="$1" expected="$2" attempt
  require_command curl
  for ((attempt=1; attempt<=PROBE_ATTEMPTS; attempt++)); do
    local body
    body="$(curl -fsS --max-time 3 "$url" 2>/dev/null || true)"
    if [[ "$body" == *"$expected"* ]]; then return 0; fi
    sleep "$PROBE_DELAY_SECONDS"
  done
  return 1
}

next_test_probe_result() {
  local sequence result index
  sequence="${TEST_PROBE_SEQUENCE:-$TEST_PROBE_RESULT}"
  local -a results=()
  IFS=',' read -r -a results <<<"$sequence"
  ((${#results[@]} > 0)) || fail "test probe sequence must not be empty"
  index="$TEST_PROBE_INDEX"
  if ((index >= ${#results[@]})); then index=$((${#results[@]} - 1)); fi
  result="${results[$index]}"
  ((TEST_PROBE_INDEX += 1))
  case "$result" in
    success|failure) printf '%s\n' "$result" ;;
    *) fail "invalid test probe result: $result" ;;
  esac
}

probe_runtime() {
  if [[ "$TEST_MODE" == "1" ]]; then
    [[ "$(next_test_probe_result)" == "success" ]]
    return
  fi
  probe_url "$HEALTH_URL" "ok" && probe_url "$READY_URL" "ready"
}

atomic_link() {
  local target="$1"
  local link="$2"
  local tmp="${link}.next.$$"
  validate_absolute_safe_path "$target"
  validate_absolute_safe_path "$link"
  run ln -s "$target" "$tmp"
  run mv -Tf "$tmp" "$link"
}

activate_release() {
  local release_dir="$1"
  local old=""
  if [[ -L "$DEPLOY_ROOT/current" ]]; then old="$(readlink "$DEPLOY_ROOT/current")"; fi
  if [[ -n "$old" ]]; then atomic_link "$old" "$DEPLOY_ROOT/previous"; fi
  atomic_link "$release_dir" "$DEPLOY_ROOT/current"
}

restore_previous() {
  [[ -L "$DEPLOY_ROOT/previous" ]] || return 1
  local previous
  previous="$(readlink "$DEPLOY_ROOT/previous")"
  atomic_link "$previous" "$DEPLOY_ROOT/current"
  stop_service
  start_service
  probe_runtime
}

deploy() {
  local mode="$1" artifact="$2" version="$3"
  validate_version "$version"
  validate_artifact "$artifact" "$version"
  ensure_layout
  local release_dir="$DEPLOY_ROOT/releases/$version"
  [[ ! -e "$release_dir" ]] || fail "release already exists: $version"
  local staging="$DEPLOY_ROOT/releases/.staging-$version-$$"
  run mkdir -p "$staging"
  trap 'rm -rf -- "$staging"' EXIT INT TERM
  run install -m 700 "$artifact" "$staging/$PROGRAM"
  if [[ "$DRY_RUN" != "1" ]]; then printf '%s\n' "$version" >"$staging/VERSION"; fi
  run mv "$staging" "$release_dir"
  trap - EXIT INT TERM
  write_service
  stop_service
  activate_release "$release_dir"
  start_service
  if ! probe_runtime; then
    log "$mode readiness validation failed; restoring previous release"
    if restore_previous; then
      fail "candidate $version failed readiness and was rolled back"
    fi
    fail "candidate $version failed readiness and automatic rollback was unavailable"
  fi
  log "$mode complete: $version"
}

rollback() {
  ensure_layout
  [[ -L "$DEPLOY_ROOT/previous" ]] || fail "no previous release is available"
  local current="" previous
  [[ -L "$DEPLOY_ROOT/current" ]] && current="$(readlink "$DEPLOY_ROOT/current")"
  previous="$(readlink "$DEPLOY_ROOT/previous")"
  atomic_link "$previous" "$DEPLOY_ROOT/current"
  [[ -n "$current" ]] && atomic_link "$current" "$DEPLOY_ROOT/previous"
  stop_service
  start_service
  probe_runtime || fail "rollback target failed readiness"
  log "rollback complete"
}

status() {
  validate_absolute_safe_path "$DEPLOY_ROOT"
  local current="none" previous="none"
  [[ -L "$DEPLOY_ROOT/current" ]] && current="$(readlink "$DEPLOY_ROOT/current")"
  [[ -L "$DEPLOY_ROOT/previous" ]] && previous="$(readlink "$DEPLOY_ROOT/previous")"
  printf 'deploy_root=%s\ncurrent=%s\nprevious=%s\nservice=%s\n' "$DEPLOY_ROOT" "$current" "$previous" "$SERVICE_NAME"
  if [[ "$TEST_MODE" != "1" ]] && command -v sv >/dev/null 2>&1 && [[ -d "$SERVICE_ROOT/$SERVICE_NAME" ]]; then
    sv status "$SERVICE_ROOT/$SERVICE_NAME" || true
  fi
}

uninstall() {
  local purge_config="$1"
  validate_absolute_safe_path "$DEPLOY_ROOT"
  validate_absolute_safe_path "$CONFIG_ROOT"
  validate_absolute_safe_path "$SERVICE_ROOT"
  stop_service
  run rm -rf -- "$SERVICE_ROOT/$SERVICE_NAME" "$DEPLOY_ROOT"
  if [[ "$purge_config" == "1" ]]; then run rm -rf -- "$CONFIG_ROOT"; fi
  log "uninstall complete"
}

main() {
  local command="${1:-}" artifact="" version="" purge_config=0
  shift || true
  while (($#)); do
    case "$1" in
      --artifact) [[ $# -ge 2 ]] || fail "--artifact requires a value"; artifact="$2"; shift 2 ;;
      --version) [[ $# -ge 2 ]] || fail "--version requires a value"; version="$2"; shift 2 ;;
      --purge-config) purge_config=1; shift ;;
      -h|--help) usage; exit 0 ;;
      *) fail "unknown argument: $1" ;;
    esac
  done
  case "$command" in
    install|upgrade) [[ -n "$artifact" && -n "$version" ]] || fail "$command requires --artifact and --version"; deploy "$command" "$artifact" "$version" ;;
    rollback) rollback ;;
    status) status ;;
    uninstall) uninstall "$purge_config" ;;
    *) usage; exit 2 ;;
  esac
}

main "$@"
