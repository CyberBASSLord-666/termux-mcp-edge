#!/usr/bin/env bash
# Setup Named Cloudflare Tunnel for Termux MCP Server.
#
# Idempotency goals:
# - Reuse an existing named tunnel instead of recreating it.
# - Treat an existing DNS route as success.
# - Clean temporary files on every exit path.

set -euo pipefail
IFS=$'\n\t'

TUNNEL_NAME="${1:-termux-mcp}"
DOMAIN="${2:-mcp.yourdomain.com}"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/termux-mcp-tunnel.XXXXXX")"

log() {
  printf '[setup_named_tunnel] %s\n' "$*"
}

fail() {
  printf '[setup_named_tunnel] ERROR: %s\n' "$*" >&2
  exit 1
}

cleanup() {
  rm -rf "$TMP_DIR"
}

terminate() {
  cleanup
  trap - EXIT INT TERM
  exit "$1"
}

trap cleanup EXIT
trap 'terminate 130' INT
trap 'terminate 143' TERM

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "Required command not found: $1"
}

validate_token() {
  local name="$1"
  local value="$2"

  [[ -n "${value//[[:space:]]/}" ]] || fail "$name must not be empty"
  [[ "$value" =~ ^[[:alnum:]._-]+$ ]] || fail "$name contains invalid characters: $value"
}

tunnel_exists() {
  cloudflared tunnel info "$TUNNEL_NAME" >/dev/null 2>&1
}

dns_route_exists() {
  local route_list="${TMP_DIR}/route-list.log"

  if ! cloudflared tunnel route list >"$route_list" 2>/dev/null; then
    return 1
  fi

  grep -F -- "$DOMAIN" "$route_list" | grep -F -- "$TUNNEL_NAME" >/dev/null 2>&1
}

ensure_dns_route() {
  local route_log="${TMP_DIR}/route-dns.log"

  if dns_route_exists; then
    log "DNS route already exists: ${DOMAIN} -> ${TUNNEL_NAME}"
    return 0
  fi

  if cloudflared tunnel route dns "$TUNNEL_NAME" "$DOMAIN" >"$route_log" 2>&1; then
    log "DNS route ensured: ${DOMAIN} -> ${TUNNEL_NAME}"
    return 0
  fi

  cat "$route_log" >&2
  return 1
}

require_command cloudflared
require_command grep
validate_token TUNNEL_NAME "$TUNNEL_NAME"
validate_token DOMAIN "$DOMAIN"

log "Setting up named Cloudflare Tunnel: ${TUNNEL_NAME}"

if tunnel_exists; then
  log "Tunnel already exists; reusing: ${TUNNEL_NAME}"
else
  log "No existing tunnel found; starting Cloudflare login/create flow."
  cloudflared tunnel login
  cloudflared tunnel create "$TUNNEL_NAME"
  log "Tunnel created: ${TUNNEL_NAME}"
fi

ensure_dns_route

log "Tunnel is ready. Update your runit service to use:"
printf '  cloudflared tunnel run %q\n' "$TUNNEL_NAME"
