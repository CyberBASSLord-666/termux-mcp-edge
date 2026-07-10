#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

PROGRAM="termux-mcp-server"
SERVICE_NAME="mcp_runtime"
DEPLOY_ROOT="${TERMUX_MCP_DEPLOY_ROOT:-${HOME}/.local/share/termux-mcp-edge}"
CONFIG_ROOT="${TERMUX_MCP_CONFIG_ROOT:-${HOME}/.config/termux-mcp-edge}"
SERVICE_ROOT="${TERMUX_MCP_SERVICE_ROOT:-${PREFIX:-/data/data/com.termux/files/usr}/var/service}"
HEALTH_URL="${TERMUX_MCP_HEALTH_URL:-http://127.0.0.1:8000/health}"
READY_URL="${TERMUX_MCP_READY_URL:-http://127.0.0.1:8000/ready}"
PROBE_ATTEMPTS="${TERMUX_MCP_PROBE_ATTEMPTS:-15}"
PROBE_DELAY_SECONDS="${TERMUX_MCP_PROBE_DELAY_SECONDS:-1}"
TEST_MODE="${TERMUX_MCP_TEST_MODE:-0}"
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
  TERMUX_MCP_HEALTH_URL, TERMUX_MCP_READY_URL
  TERMUX_MCP_TEST_MODE=1       Skip live runit and HTTP operations.
  TERMUX_MCP_DRY_RUN=1        Print mutations without applying them.
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

ensure_layout() {
  validate_absolute_safe_path "$DEPLOY_ROOT"
  validate_absolute_safe_path "$CONFIG_ROOT"
  validate_absolute_safe_path "$SERVICE_ROOT"
  run mkdir -p "$DEPLOY_ROOT/releases" "$CONFIG_ROOT" "$SERVICE_ROOT"
  run chmod 700 "$DEPLOY_ROOT" "$DEPLOY_ROOT/releases" "$CONFIG_ROOT"
}

artifact_architecture() {
  local artifact="$1"
  if command -v file >/dev/null 2>&1; then
    file -b "$artifact"
  else
    printf 'unknown'
  fi
}

validate_artifact() {
  local artifact="$1"
  [[ -f "$artifact" ]] || fail "artifact not found: $artifact"
  [[ -s "$artifact" ]] || fail "artifact is empty: $artifact"
  [[ ! -L "$artifact" ]] || fail "artifact must not be a symlink"
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
#!/data/data/com.termux/files/usr/bin/sh
set -eu
umask 077
CONFIG_FILE="$CONFIG_ROOT/runtime.env"
if [ -f "\$CONFIG_FILE" ]; then
  set -a
  . "\$CONFIG_FILE"
  set +a
fi
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
  [[ "$TEST_MODE" == "1" ]] && return 0
  require_command curl
  for ((attempt=1; attempt<=PROBE_ATTEMPTS; attempt++)); do
    local body
    body="$(curl -fsS --max-time 3 "$url" 2>/dev/null || true)"
    if [[ "$body" == *"$expected"* ]]; then return 0; fi
    sleep "$PROBE_DELAY_SECONDS"
  done
  return 1
}

probe_runtime() {
  probe_url "$HEALTH_URL" "ok" && probe_url "$READY_URL" "ready"
}

atomic_link() {
  local target="$1" link="$2"
  local tmp="${link}.next.$$"
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
  validate_artifact "$artifact"
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
