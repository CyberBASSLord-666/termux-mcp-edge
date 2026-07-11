#!/usr/bin/env bash
# Safely provision an explicitly named Cloudflare Tunnel and DNS hostname.

set -euo pipefail
IFS=$'\n\t'
umask 077

CREATE_ALLOWED=0
DRY_RUN=0
POSITIONAL=()
TMP_DIR=""

usage() {
  cat <<'EOF'
Usage:
  setup_named_tunnel.sh [--create] [--dry-run] TUNNEL_NAME HOSTNAME

Options:
  --create   Explicitly authorize tunnel creation when the exact tunnel name
             does not already exist. Authentication must already be configured
             with cloudflared.
  --dry-run  Validate and print the bounded plan without calling cloudflared.

The script never overwrites an existing DNS record. A hostname owned by a
different tunnel or record type is a hard error requiring operator review.
EOF
}

log() { printf '[setup_named_tunnel] %s\n' "$*"; }
fail() { printf '[setup_named_tunnel] ERROR: %s\n' "$*" >&2; exit 1; }

cleanup() {
  [[ -n "$TMP_DIR" ]] && rm -rf -- "$TMP_DIR"
  return 0
}

terminate() {
  local status="$1"
  trap - EXIT INT TERM HUP
  cleanup
  exit "$status"
}

trap cleanup EXIT
trap 'terminate 130' INT
trap 'terminate 143' TERM HUP

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

validate_tunnel_name() {
  local value="$1"
  ((${#value} >= 1 && ${#value} <= 63)) || fail "tunnel name must contain 1 to 63 characters"
  [[ "$value" =~ ^[A-Za-z0-9][A-Za-z0-9_-]*$ ]] || fail "tunnel name must start with an alphanumeric character and contain only alphanumerics, underscore, or hyphen"
}

validate_hostname() {
  local value="$1" label
  local -a labels=()
  ((${#value} >= 1 && ${#value} <= 253)) || fail "hostname must contain 1 to 253 characters"
  [[ "$value" != *[[:space:][:cntrl:]]* ]] || fail "hostname must not contain whitespace or control characters"
  [[ "$value" != *://* && "$value" != *:* && "$value" != */* && "$value" != *'?'* && "$value" != *'#'* && "$value" != *'*'* ]] || fail "hostname must be a plain DNS name without a scheme, port, path, query, fragment, or wildcard"
  [[ "$value" != .* && "$value" != *. && "$value" != *..* ]] || fail "hostname contains an empty DNS label"
  IFS='.' read -r -a labels <<<"$value"
  ((${#labels[@]} >= 2)) || fail "hostname must contain at least two DNS labels"
  for label in "${labels[@]}"; do
    ((${#label} >= 1 && ${#label} <= 63)) || fail "hostname contains a DNS label outside the 1 to 63 character range"
    [[ "$label" =~ ^[A-Za-z0-9]([A-Za-z0-9-]*[A-Za-z0-9])?$ ]] || fail "hostname contains an invalid DNS label"
  done
}

for argument in "$@"; do
  case "$argument" in
    --create) CREATE_ALLOWED=1 ;;
    --dry-run) DRY_RUN=1 ;;
    --help|-h) usage; exit 0 ;;
    --*) fail "unsupported option: $argument" ;;
    *) POSITIONAL+=("$argument") ;;
  esac
done

((${#POSITIONAL[@]} == 2)) || { usage >&2; fail "explicit tunnel name and hostname are required"; }
TUNNEL_NAME="${POSITIONAL[0]}"
HOSTNAME="${POSITIONAL[1]}"
validate_tunnel_name "$TUNNEL_NAME"
validate_hostname "$HOSTNAME"

if ((DRY_RUN == 1)); then
  log "dry-run: would inspect exact tunnel name: $TUNNEL_NAME"
  if ((CREATE_ALLOWED == 1)); then
    log "dry-run: would create the tunnel only if authenticated inventory confirms it is absent"
  else
    log "dry-run: tunnel creation is not authorized"
  fi
  log "dry-run: would create a non-overwriting DNS route for: $HOSTNAME"
  printf '  cloudflared tunnel run %q\n' "$TUNNEL_NAME"
  exit 0
fi

require_command cloudflared
require_command jq
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/termux-mcp-tunnel.XXXXXX")"
chmod 700 "$TMP_DIR"
TUNNEL_LIST="$TMP_DIR/tunnels.json"
ROUTE_LOG="$TMP_DIR/route-dns.log"

list_tunnels() {
  if ! cloudflared tunnel list --output json >"$TUNNEL_LIST" 2>/dev/null; then
    return 1
  fi
  jq -e 'type == "array" and all(.[]; (.name | type) == "string")' "$TUNNEL_LIST" >/dev/null 2>&1 ||
    fail "cloudflared returned an unsupported tunnel-list JSON shape"
}

tunnel_match_count() {
  jq -r --arg name "$TUNNEL_NAME" '[.[] | select(.name == $name)] | length' "$TUNNEL_LIST"
}

if ! list_tunnels; then
  fail "unable to list tunnels; authenticate cloudflared and resolve network or Cloudflare service errors before mutation"
fi
MATCH_COUNT="$(tunnel_match_count)"
[[ "$MATCH_COUNT" =~ ^[0-9]+$ ]] || fail "unable to determine exact tunnel identity"

case "$MATCH_COUNT" in
  1) log "exact tunnel already exists; reusing it" ;;
  0)
    ((CREATE_ALLOWED == 1)) || fail "exact tunnel does not exist; rerun with --create only after reviewing the external login/create operation"
    log "exact tunnel is absent; starting explicitly authorized create flow"
    cloudflared tunnel create "$TUNNEL_NAME" >/dev/null 2>&1 || fail "cloudflared tunnel creation failed"
    list_tunnels
    [[ "$(tunnel_match_count)" == 1 ]] || fail "created tunnel could not be confirmed by exact name"
    log "tunnel created and confirmed"
    ;;
  *) fail "multiple active tunnels matched the exact requested name" ;;
esac

if ! cloudflared tunnel route dns "$TUNNEL_NAME" "$HOSTNAME" >"$ROUTE_LOG" 2>&1; then
  fail "DNS route was not created; an existing record conflict or Cloudflare error requires operator review (no overwrite attempted)"
fi

log "DNS route confirmed without overwrite"
log "tunnel is ready; run it with:"
printf '  cloudflared tunnel run %q\n' "$TUNNEL_NAME"
