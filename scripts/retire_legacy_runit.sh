#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

TERMUX_PREFIX="${PREFIX:-/data/data/com.termux/files/usr}"
SERVICE_ROOT="${TERMUX_MCP_SERVICE_ROOT:-${TERMUX_PREFIX}/var/service}"
LEGACY_SERVICE_NAME="mcp-server"
CANONICAL_SERVICE_NAME="mcp_runtime"
LEGACY_SERVICE_DIR="${SERVICE_ROOT}/${LEGACY_SERVICE_NAME}"
CANONICAL_SERVICE_DIR="${SERVICE_ROOT}/${CANONICAL_SERVICE_NAME}"
LEGACY_TOKEN_FILE="${HOME}/.termux_mcp_token"
DRY_RUN="${TERMUX_MCP_DRY_RUN:-0}"

log() { printf '[legacy-retirement] %s\n' "$*"; }
fail() { printf '[legacy-retirement] ERROR: %s\n' "$*" >&2; exit 1; }

is_true() {
  case "${1,,}" in
    1|true|yes|on) return 0 ;;
    *) return 1 ;;
  esac
}

run() {
  if is_true "$DRY_RUN"; then
    printf '[legacy-retirement] DRY-RUN:'
    printf ' %q' "$@"
    printf '\n'
  else
    "$@"
  fi
}

[[ "$SERVICE_ROOT" == /* ]] || fail "service root must be absolute"
[[ "$SERVICE_ROOT" != "/" ]] || fail "service root must not be filesystem root"
[[ ! -L "$LEGACY_SERVICE_DIR" ]] || fail "legacy service path must not be a symlink"

if [[ ! -e "$LEGACY_SERVICE_DIR" ]]; then
  log "legacy service is not installed"
  exit 0
fi

[[ -d "$LEGACY_SERVICE_DIR" ]] || fail "legacy service path is not a directory"

if command -v sv >/dev/null 2>&1; then
  run sv down "$LEGACY_SERVICE_DIR"
  if ! is_true "$DRY_RUN"; then
    status="$(sv status "$LEGACY_SERVICE_DIR" 2>&1 || true)"
    case "$status" in
      down:*) ;;
      *) fail "legacy service did not reach a confirmed down state" ;;
    esac
  fi
else
  fail "sv is required to retire an installed legacy service safely"
fi

run rm -rf -- "$LEGACY_SERVICE_DIR"

if [[ -e "$CANONICAL_SERVICE_DIR" ]]; then
  log "canonical ${CANONICAL_SERVICE_NAME} service remains installed"
else
  log "canonical service is not installed; deploy it with scripts/termux_deploy.sh"
fi

if [[ -e "$LEGACY_TOKEN_FILE" ]]; then
  log "preserved legacy token file at ${LEGACY_TOKEN_FILE}; migrate its value into the canonical runtime.env manually, then remove it"
fi

log "legacy ${LEGACY_SERVICE_NAME} service retired"
