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
ARTIFACT_MAX_BYTES="${TERMUX_MCP_ARTIFACT_MAX_BYTES:-134217728}"
ALLOW_UNVERIFIED_ARTIFACT="${TERMUX_MCP_ALLOW_UNVERIFIED_ARTIFACT:-0}"
TEST_MODE="${TERMUX_MCP_TEST_MODE:-0}"
TEST_PROBE_SEQUENCE="${TERMUX_MCP_TEST_PROBE_SEQUENCE:-success}"
TEST_PROBE_INDEX=0
DRY_RUN="${TERMUX_MCP_DRY_RUN:-0}"

RELEASES_ROOT=""
LOCK_DIR=""
LOCK_HELD=0
STAGING_DIR=""
LINK_TMP=""
CURRENT_BEFORE=""
PREVIOUS_BEFORE=""
CURRENT_BEFORE_PRESENT=0
PREVIOUS_BEFORE_PRESENT=0

usage() {
  cat <<'EOF'
Usage:
  termux_deploy.sh install  --artifact PATH --version VERSION --sha256 HEX [--dry-run]
  termux_deploy.sh upgrade  --artifact PATH --version VERSION --sha256 HEX [--dry-run]
  termux_deploy.sh rollback [--dry-run]
  termux_deploy.sh status
  termux_deploy.sh uninstall [--purge-config] [--dry-run]

Artifact checksum verification is required by default. Advanced operators may
explicitly set TERMUX_MCP_ALLOW_UNVERIFIED_ARTIFACT=1 or pass
--allow-unverified after independently validating a local build.

Environment overrides:
  TERMUX_MCP_DEPLOY_ROOT, TERMUX_MCP_CONFIG_ROOT, TERMUX_MCP_SERVICE_ROOT
  TERMUX_MCP_SERVICE_SHELL, TERMUX_MCP_HEALTH_URL, TERMUX_MCP_READY_URL
  TERMUX_MCP_PROBE_ATTEMPTS, TERMUX_MCP_PROBE_DELAY_SECONDS
  TERMUX_MCP_ARTIFACT_MAX_BYTES
  TERMUX_MCP_TEST_MODE=1       Skip live runit, architecture, and HTTP operations.
  TERMUX_MCP_TEST_PROBE_SEQUENCE
                               Test-only comma-separated success/failure sequence.
  TERMUX_MCP_DRY_RUN=1         Print mutations without applying them.
EOF
}

log() { printf '[termux-deploy] %s\n' "$*"; }
fail() { printf '[termux-deploy] ERROR: %s\n' "$*" >&2; exit 1; }

run() {
  if is_true "$DRY_RUN"; then
    printf '[termux-deploy] DRY-RUN:'
    printf ' %q' "$@"
    printf '\n'
  else
    "$@"
  fi
}

cleanup() {
  local status=$?
  trap - EXIT
  if ! is_true "$DRY_RUN"; then
    [[ -n "$LINK_TMP" ]] && rm -f -- "$LINK_TMP" 2>/dev/null || true
    [[ -n "$STAGING_DIR" ]] && rm -rf -- "$STAGING_DIR" 2>/dev/null || true
    if [[ "$LOCK_HELD" == "1" && -n "$LOCK_DIR" ]]; then
      rm -rf -- "$LOCK_DIR" 2>/dev/null || true
    fi
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

require_command() { command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"; }

is_boolean() {
  case "${1,,}" in
    0|1|false|true|no|yes|off|on) return 0 ;;
    *) return 1 ;;
  esac
}

is_true() {
  case "${1,,}" in
    1|true|yes|on) return 0 ;;
    *) return 1 ;;
  esac
}

validate_integer_range() {
  local name="$1" value="$2" minimum="$3" maximum="$4"
  [[ "$value" =~ ^[0-9]+$ ]] || fail "$name must be an integer"
  ((value >= minimum && value <= maximum)) || fail "$name must be between $minimum and $maximum"
}

validate_absolute_safe_path() {
  local path="$1" label="${2:-path}"
  [[ "$path" == /* ]] || fail "$label must be absolute"
  [[ "$path" != "/" ]] || fail "$label must not be filesystem root"
  [[ "$path" =~ ^/[A-Za-z0-9._/@%+=,:/-]+$ ]] || fail "$label contains unsupported characters"
  case "$path" in
    *'/../'*|*/..|*'/./'*|*/.|*'//'*) fail "$label contains unsafe path segments" ;;
  esac
}

canonicalize_path() { realpath -m -- "$1"; }

is_descendant() {
  local child="$1" parent="$2"
  [[ "$child" == "$parent/"* ]]
}

paths_overlap() {
  local left="$1" right="$2"
  [[ "$left" == "$right" || "$left" == "$right/"* || "$right" == "$left/"* ]]
}

validate_environment_roots() {
  require_command realpath
  validate_absolute_safe_path "$HOME" HOME
  validate_absolute_safe_path "$TERMUX_PREFIX" PREFIX
  validate_absolute_safe_path "$DEPLOY_ROOT" TERMUX_MCP_DEPLOY_ROOT
  validate_absolute_safe_path "$CONFIG_ROOT" TERMUX_MCP_CONFIG_ROOT
  validate_absolute_safe_path "$SERVICE_ROOT" TERMUX_MCP_SERVICE_ROOT
  validate_absolute_safe_path "$SERVICE_SHELL" TERMUX_MCP_SERVICE_SHELL

  local home_root prefix_root
  home_root="$(canonicalize_path "$HOME")"
  prefix_root="$(canonicalize_path "$TERMUX_PREFIX")"
  DEPLOY_ROOT="$(canonicalize_path "$DEPLOY_ROOT")"
  CONFIG_ROOT="$(canonicalize_path "$CONFIG_ROOT")"
  SERVICE_ROOT="$(canonicalize_path "$SERVICE_ROOT")"
  SERVICE_SHELL="$(canonicalize_path "$SERVICE_SHELL")"

  is_descendant "$DEPLOY_ROOT" "$home_root" || fail "deployment root must remain beneath HOME"
  is_descendant "$CONFIG_ROOT" "$home_root" || fail "configuration root must remain beneath HOME"
  is_descendant "$SERVICE_ROOT" "$prefix_root" || fail "service root must remain beneath PREFIX"
  is_descendant "$SERVICE_SHELL" "$prefix_root" || fail "service shell must remain beneath PREFIX"
  paths_overlap "$DEPLOY_ROOT" "$CONFIG_ROOT" && fail "deployment and configuration roots must not overlap"

  RELEASES_ROOT="$DEPLOY_ROOT/releases"
  LOCK_DIR="${DEPLOY_ROOT}.deploy-lock"
  validate_absolute_safe_path "$LOCK_DIR" deployment_lock
}

validate_loopback_url() {
  local name="$1" url="$2" authority port
  [[ "$url" =~ ^http://(127\.0\.0\.1|localhost|\[::1\]):([0-9]{1,5})/[A-Za-z0-9._~/%:@+-]*$ ]] || fail "$name must be an explicit loopback HTTP URL"
  authority="${url#http://}"
  authority="${authority%%/*}"
  port="${authority##*:}"
  validate_integer_range "$name port" "$port" 1 65535
}

validate_version() {
  [[ "$1" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] || fail "invalid version"
}

validate_sha256() {
  [[ "$1" =~ ^[A-Fa-f0-9]{64}$ ]] || fail "sha256 must contain exactly 64 hexadecimal characters"
}

validate_common_settings() {
  is_boolean "$TEST_MODE" || fail "TERMUX_MCP_TEST_MODE must be boolean"
  is_boolean "$DRY_RUN" || fail "TERMUX_MCP_DRY_RUN must be boolean"
  validate_environment_roots
}

validate_deployment_settings() {
  is_boolean "$ALLOW_UNVERIFIED_ARTIFACT" || fail "TERMUX_MCP_ALLOW_UNVERIFIED_ARTIFACT must be boolean"
  validate_integer_range TERMUX_MCP_PROBE_ATTEMPTS "$PROBE_ATTEMPTS" 1 120
  validate_integer_range TERMUX_MCP_PROBE_DELAY_SECONDS "$PROBE_DELAY_SECONDS" 0 60
  validate_integer_range TERMUX_MCP_ARTIFACT_MAX_BYTES "$ARTIFACT_MAX_BYTES" 1 536870912
  validate_loopback_url TERMUX_MCP_HEALTH_URL "$HEALTH_URL"
  validate_loopback_url TERMUX_MCP_READY_URL "$READY_URL"
  [[ -x "$SERVICE_SHELL" ]] || fail "service shell is not executable"
}

ensure_layout() {
  run mkdir -p "$RELEASES_ROOT" "$CONFIG_ROOT" "$SERVICE_ROOT"
  run chmod 700 "$DEPLOY_ROOT" "$RELEASES_ROOT" "$CONFIG_ROOT"
}

acquire_lock() {
  if is_true "$DRY_RUN"; then
    log "dry-run: deployment lock not acquired"
    return 0
  fi

  local parent owner=""
  parent="$(dirname "$LOCK_DIR")"
  mkdir -p -- "$parent"
  if ! mkdir -- "$LOCK_DIR" 2>/dev/null; then
    [[ -f "$LOCK_DIR/owner.pid" ]] && read -r owner <"$LOCK_DIR/owner.pid" || true
    if [[ "$owner" =~ ^[0-9]+$ ]] && kill -0 "$owner" 2>/dev/null; then
      fail "another deployment operation is active"
    fi
    log "removing stale deployment lock"
    rm -rf -- "$LOCK_DIR"
    mkdir -- "$LOCK_DIR" || fail "unable to acquire deployment lock"
  fi
  chmod 700 "$LOCK_DIR"
  printf '%s\n' "$$" >"$LOCK_DIR/owner.pid"
  chmod 600 "$LOCK_DIR/owner.pid"
  LOCK_HELD=1
}

validate_runtime_config() {
  local config_file="$CONFIG_ROOT/runtime.env"
  [[ -f "$config_file" && ! -L "$config_file" ]] || fail "runtime configuration must be a regular non-symlink file"

  local mode permissions
  mode="$(stat -c '%a' "$config_file")"
  permissions=$((8#$mode))
  (( (permissions & 077) == 0 && (permissions & 0400) != 0 )) || fail "runtime configuration must be owner-readable and inaccessible to group/other"

  local line key value token_present=0 allow_local=0 server_host="127.0.0.1"
  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ "$line" != *$'\r'* ]] || fail "runtime configuration contains carriage returns"
    case "$line" in
      ''|'#'*) continue ;;
      *=*) ;;
      *) fail "runtime configuration lines must use KEY=value syntax" ;;
    esac
    key="${line%%=*}"
    value="${line#*=}"
    [[ "$key" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]] || fail "runtime configuration contains an invalid variable name"
    case "$key" in
      MCP__*|RUST_LOG|RUST_BACKTRACE) ;;
      *) fail "runtime configuration variable is not allowlisted" ;;
    esac
    case "$key" in
      MCP__AUTH__STATIC_TOKEN)
        [[ -n "$value" && "$value" != *[[:space:]]* ]] || fail "runtime bearer token must be non-empty and contain no whitespace"
        token_present=1
        ;;
      MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY)
        is_boolean "$value" || fail "localhost-only authentication setting must be boolean"
        is_true "$value" && allow_local=1
        ;;
      MCP__SERVER__HOST) server_host="$value" ;;
    esac
  done <"$config_file"

  if ((token_present == 0)); then
    ((allow_local == 1)) || fail "runtime configuration must define a bearer token or explicit localhost-only mode"
    case "$server_host" in
      localhost|127.0.0.1|::1) ;;
      *) fail "unauthenticated runtime configuration must bind to loopback" ;;
    esac
  fi
}

artifact_version() {
  local artifact="$1" output first_line
  require_command timeout
  output="$(timeout 5 "$artifact" --version 2>/dev/null)" || fail "artifact did not return a version within 5 seconds"
  first_line="${output%%$'\n'*}"
  [[ -n "$first_line" ]] || fail "artifact returned an empty version response"
  printf '%s\n' "${first_line##* }"
}

validate_artifact() {
  local artifact="$1" expected_version="$2" expected_sha="$3"
  [[ -f "$artifact" ]] || fail "artifact not found"
  [[ -s "$artifact" ]] || fail "artifact is empty"
  [[ ! -L "$artifact" ]] || fail "artifact must not be a symlink"
  [[ -x "$artifact" ]] || fail "artifact must be executable"

  local artifact_size
  artifact_size="$(stat -c '%s' "$artifact")"
  ((artifact_size <= ARTIFACT_MAX_BYTES)) || fail "artifact exceeds configured size limit"

  if [[ -n "$expected_sha" ]]; then
    require_command sha256sum
    validate_sha256 "$expected_sha"
    local actual_sha
    actual_sha="$(sha256sum -- "$artifact")"
    actual_sha="${actual_sha%% *}"
    [[ "${actual_sha,,}" == "${expected_sha,,}" ]] || fail "artifact checksum mismatch"
  elif ! is_true "$ALLOW_UNVERIFIED_ARTIFACT"; then
    fail "artifact verification requires --sha256 or explicit unverified-artifact opt-in"
  fi

  if ! is_true "$TEST_MODE"; then
    require_command file
    require_command uname
    local description architecture
    description="$(file -b -- "$artifact")"
    [[ "$description" == *"ELF"* ]] || fail "artifact must be an ELF executable"
    architecture="$(uname -m)"
    case "$architecture" in
      aarch64|arm64)
        [[ "$description" == *"aarch64"* || "$description" == *"ARM aarch64"* ]] || fail "artifact architecture mismatch"
        ;;
      x86_64)
        [[ "$description" == *"x86-64"* || "$description" == *"x86_64"* ]] || fail "artifact architecture mismatch"
        ;;
      *) fail "unsupported host architecture" ;;
    esac
  fi

  local reported_version
  reported_version="$(artifact_version "$artifact")"
  [[ "$reported_version" == "$expected_version" ]] || fail "artifact version mismatch"
}

write_service() {
  local service_dir="$SERVICE_ROOT/$SERVICE_NAME"
  local run_file="$service_dir/run"
  run mkdir -p "$service_dir"
  run chmod 700 "$service_dir"
  if is_true "$DRY_RUN"; then
    log "would write project-owned runit service at $run_file"
    return
  fi

  cat >"$run_file" <<EOF
#!$SERVICE_SHELL
set -eu
umask 077
CONFIG_FILE="$CONFIG_ROOT/runtime.env"
[ -f "\$CONFIG_FILE" ] && [ ! -L "\$CONFIG_FILE" ] || exit 111
mode=\$(stat -c '%a' "\$CONFIG_FILE") || exit 111
permissions=\$((0\$mode))
[ \$((permissions & 077)) -eq 0 ] || exit 111
while IFS= read -r line || [ -n "\$line" ]; do
  case "\$line" in
    ''|'#'*) continue ;;
    *=*) ;;
    *) exit 111 ;;
  esac
  key=\${line%%=*}
  value=\${line#*=}
  case "\$key" in
    ''|[0-9]*|*[!A-Za-z0-9_]*) exit 111 ;;
  esac
  case "\$key" in
    MCP__*|RUST_LOG|RUST_BACKTRACE) ;;
    *) exit 111 ;;
  esac
  export "\$key=\$value"
done <"\$CONFIG_FILE"
exec "$DEPLOY_ROOT/current/$PROGRAM"
EOF
  chmod 700 "$run_file"
}

stop_service() {
  is_true "$TEST_MODE" && return 0
  if command -v sv >/dev/null 2>&1 && [[ -d "$SERVICE_ROOT/$SERVICE_NAME" ]]; then
    run sv down "$SERVICE_ROOT/$SERVICE_NAME" || true
  fi
}

start_service() {
  is_true "$TEST_MODE" && return 0
  require_command sv
  run sv up "$SERVICE_ROOT/$SERVICE_NAME"
}

next_test_probe_result() {
  local -a results=()
  local result index
  IFS=',' read -r -a results <<<"$TEST_PROBE_SEQUENCE"
  ((${#results[@]} > 0)) || fail "test probe sequence must not be empty"
  index="$TEST_PROBE_INDEX"
  if ((index >= ${#results[@]})); then
    index=$((${#results[@]} - 1))
  fi
  result="${results[$index]}"
  ((TEST_PROBE_INDEX += 1))
  case "$result" in
    success) return 0 ;;
    failure) return 1 ;;
    *) fail "test probe sequence must contain success/failure values" ;;
  esac
}

probe_url() {
  local url="$1" kind="$2" attempt body
  require_command curl
  for ((attempt=1; attempt<=PROBE_ATTEMPTS; attempt++)); do
    body="$(curl -fsS --proto '=http' --noproxy '*' --max-time 3 "$url" 2>/dev/null || true)"
    case "$kind" in
      health) [[ "$body" == "ok" ]] && return 0 ;;
      ready) [[ "$body" == *'"status":"ready"'* || "$body" == *'"status": "ready"'* ]] && return 0 ;;
      *) return 1 ;;
    esac
    sleep "$PROBE_DELAY_SECONDS"
  done
  return 1
}

probe_runtime() {
  if is_true "$TEST_MODE"; then
    next_test_probe_result
    return
  fi
  probe_url "$HEALTH_URL" health && probe_url "$READY_URL" ready
}

release_target_from_link() {
  local link="$1" raw candidate canonical
  [[ -L "$link" ]] || return 1
  raw="$(readlink "$link")"
  if [[ "$raw" == /* ]]; then
    candidate="$raw"
  else
    candidate="$(dirname "$link")/$raw"
  fi
  canonical="$(canonicalize_path "$candidate")"
  is_descendant "$canonical" "$RELEASES_ROOT" || return 1
  [[ -d "$canonical" && -x "$canonical/$PROGRAM" ]] || return 1
  printf '%s\n' "$canonical"
}

validate_release_dir() {
  local release_dir
  release_dir="$(canonicalize_path "$1")"
  is_descendant "$release_dir" "$RELEASES_ROOT" || fail "release target escapes the releases root"
  if ! is_true "$DRY_RUN" || [[ -e "$release_dir" ]]; then
    [[ -d "$release_dir" && -x "$release_dir/$PROGRAM" ]] || fail "release target is incomplete"
  fi
  printf '%s\n' "$release_dir"
}

atomic_link() {
  local target link
  target="$(validate_release_dir "$1")"
  link="$2"
  validate_absolute_safe_path "$link" release_link
  LINK_TMP="${link}.next.$$"
  if ! is_true "$DRY_RUN"; then
    rm -f -- "$LINK_TMP"
  fi
  run ln -s -- "$target" "$LINK_TMP"
  run mv -Tf -- "$LINK_TMP" "$link"
  LINK_TMP=""
}

remove_link() { run rm -f -- "$1"; }

capture_link_state() {
  CURRENT_BEFORE=""
  PREVIOUS_BEFORE=""
  CURRENT_BEFORE_PRESENT=0
  PREVIOUS_BEFORE_PRESENT=0
  if [[ -L "$DEPLOY_ROOT/current" ]]; then
    CURRENT_BEFORE="$(release_target_from_link "$DEPLOY_ROOT/current")" || fail "current release link is invalid"
    CURRENT_BEFORE_PRESENT=1
  fi
  if [[ -L "$DEPLOY_ROOT/previous" ]]; then
    PREVIOUS_BEFORE="$(release_target_from_link "$DEPLOY_ROOT/previous")" || fail "previous release link is invalid"
    PREVIOUS_BEFORE_PRESENT=1
  fi
}

restore_link_state() {
  if ((CURRENT_BEFORE_PRESENT == 1)); then
    atomic_link "$CURRENT_BEFORE" "$DEPLOY_ROOT/current"
  else
    remove_link "$DEPLOY_ROOT/current"
  fi
  if ((PREVIOUS_BEFORE_PRESENT == 1)); then
    atomic_link "$PREVIOUS_BEFORE" "$DEPLOY_ROOT/previous"
  else
    remove_link "$DEPLOY_ROOT/previous"
  fi
}

activate_release() {
  local release_dir
  release_dir="$(validate_release_dir "$1")"
  if ((CURRENT_BEFORE_PRESENT == 1)); then
    atomic_link "$CURRENT_BEFORE" "$DEPLOY_ROOT/previous"
  else
    remove_link "$DEPLOY_ROOT/previous"
  fi
  atomic_link "$release_dir" "$DEPLOY_ROOT/current"
}

recover_failed_deployment() {
  local failed_release="$1"
  stop_service
  restore_link_state
  run rm -rf -- "$failed_release"
  if ((CURRENT_BEFORE_PRESENT == 1)); then
    start_service
    probe_runtime || return 1
  fi
  return 0
}

deploy() {
  local mode="$1" artifact="$2" version="$3" expected_sha="$4"
  validate_version "$version"
  validate_artifact "$artifact" "$version" "$expected_sha"
  ensure_layout
  validate_runtime_config
  acquire_lock
  capture_link_state

  case "$mode" in
    install)
      ((CURRENT_BEFORE_PRESENT == 0)) || fail "an active release already exists; use upgrade"
      ((PREVIOUS_BEFORE_PRESENT == 0)) || fail "deployment state is inconsistent: previous exists without current"
      ;;
    upgrade) ((CURRENT_BEFORE_PRESENT == 1)) || fail "no active release exists; use install" ;;
    *) fail "unsupported deployment mode" ;;
  esac

  local release_dir="$RELEASES_ROOT/$version"
  [[ ! -e "$release_dir" && ! -L "$release_dir" ]] || fail "release already exists"
  STAGING_DIR="$RELEASES_ROOT/.staging-$version-$$"
  run mkdir -p "$STAGING_DIR"
  run chmod 700 "$STAGING_DIR"
  run install -m 700 "$artifact" "$STAGING_DIR/$PROGRAM"
  if ! is_true "$DRY_RUN"; then
    printf '%s\n' "$version" >"$STAGING_DIR/VERSION"
    chmod 600 "$STAGING_DIR/VERSION"
  fi
  run mv -- "$STAGING_DIR" "$release_dir"
  STAGING_DIR=""

  write_service
  stop_service
  activate_release "$release_dir"
  start_service
  if ! probe_runtime; then
    log "$mode readiness validation failed; restoring the exact previous state"
    if recover_failed_deployment "$release_dir"; then
      fail "candidate failed readiness and was removed after recovery"
    fi
    fail "candidate failed readiness and the prior release could not be recovered"
  fi
  log "$mode complete: $version"
}

rollback() {
  ensure_layout
  validate_runtime_config
  acquire_lock
  capture_link_state
  ((CURRENT_BEFORE_PRESENT == 1)) || fail "no active release is available"
  ((PREVIOUS_BEFORE_PRESENT == 1)) || fail "no previous release is available"

  stop_service
  atomic_link "$PREVIOUS_BEFORE" "$DEPLOY_ROOT/current"
  atomic_link "$CURRENT_BEFORE" "$DEPLOY_ROOT/previous"
  start_service
  if ! probe_runtime; then
    log "rollback target failed readiness; restoring the original release state"
    stop_service
    restore_link_state
    start_service
    if probe_runtime; then
      fail "rollback target failed readiness and the original release was restored"
    fi
    fail "rollback target and original release both failed readiness"
  fi
  log "rollback complete"
}

status() {
  local current="none" previous="none" invalid=0
  if [[ -L "$DEPLOY_ROOT/current" ]]; then
    if current="$(release_target_from_link "$DEPLOY_ROOT/current")"; then :; else current="invalid"; invalid=1; fi
  fi
  if [[ -L "$DEPLOY_ROOT/previous" ]]; then
    if previous="$(release_target_from_link "$DEPLOY_ROOT/previous")"; then :; else previous="invalid"; invalid=1; fi
  fi
  printf 'deploy_root=%s\ncurrent=%s\nprevious=%s\nservice=%s\n' "$DEPLOY_ROOT" "$current" "$previous" "$SERVICE_NAME"
  if ! is_true "$TEST_MODE" && command -v sv >/dev/null 2>&1 && [[ -d "$SERVICE_ROOT/$SERVICE_NAME" ]]; then
    sv status "$SERVICE_ROOT/$SERVICE_NAME" || true
  fi
  ((invalid == 0)) || fail "one or more release links are invalid"
}

uninstall() {
  local purge_config="$1"
  acquire_lock
  stop_service
  run rm -rf -- "$SERVICE_ROOT/$SERVICE_NAME" "$DEPLOY_ROOT"
  if [[ "$purge_config" == "1" ]]; then
    run rm -rf -- "$CONFIG_ROOT"
  fi
  log "uninstall complete"
}

reject_extra_arguments() {
  local artifact="$1" version="$2" expected_sha="$3" purge_config="$4"
  [[ -z "$artifact" && -z "$version" && -z "$expected_sha" && "$purge_config" == "0" ]] || fail "arguments are not valid for this command"
}

main() {
  local command="${1:-}" artifact="" version="" expected_sha="" purge_config=0
  shift || true
  while (($#)); do
    case "$1" in
      --artifact) [[ $# -ge 2 ]] || fail "--artifact requires a value"; artifact="$2"; shift 2 ;;
      --version) [[ $# -ge 2 ]] || fail "--version requires a value"; version="$2"; shift 2 ;;
      --sha256) [[ $# -ge 2 ]] || fail "--sha256 requires a value"; expected_sha="$2"; shift 2 ;;
      --allow-unverified) ALLOW_UNVERIFIED_ARTIFACT=1; shift ;;
      --dry-run) DRY_RUN=1; shift ;;
      --purge-config) purge_config=1; shift ;;
      -h|--help) usage; exit 0 ;;
      *) fail "unknown argument" ;;
    esac
  done

  validate_common_settings
  case "$command" in
    install|upgrade)
      validate_deployment_settings
      [[ -n "$artifact" && -n "$version" ]] || fail "$command requires --artifact and --version"
      [[ "$purge_config" == "0" ]] || fail "--purge-config is only valid with uninstall"
      deploy "$command" "$artifact" "$version" "$expected_sha"
      ;;
    rollback)
      validate_deployment_settings
      reject_extra_arguments "$artifact" "$version" "$expected_sha" "$purge_config"
      rollback
      ;;
    status)
      reject_extra_arguments "$artifact" "$version" "$expected_sha" "$purge_config"
      status
      ;;
    uninstall)
      [[ -z "$artifact" && -z "$version" && -z "$expected_sha" ]] || fail "artifact arguments are not valid with uninstall"
      uninstall "$purge_config"
      ;;
    *) usage; exit 2 ;;
  esac
}

main "$@"
