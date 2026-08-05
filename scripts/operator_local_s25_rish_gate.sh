#!/usr/bin/env bash
# On-device S2.5 operator gate for Termux on physical AArch64 Android.
# Does not replace the protected GitHub physical workflow; it proves the same
# identity claim locally when Shizuku is running so solo operators can unblock
# the foundation without an x64 controller host.
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077
set +x

fail() {
  printf '[operator-local-s25] FAIL reason=%s\n' "$1" >&2
  exit 1
}
log() { printf '[operator-local-s25] %s\n' "$*"; }

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
ARTIFACT="${1:-}"
DEX_PATH="${MCP__ANDROID__RISH_DEX_PATH:-$HOME/.local/share/termux-mcp-edge/rish/rish_shizuku.dex}"
TOKEN="${MCP__AUTH__STATIC_TOKEN:-operator-local-s25-token}"
HOST="127.0.0.1"
PORT="${OPERATOR_LOCAL_S25_PORT:-19177}"
WORKDIR=""
SERVER_PID=""

cleanup() {
  local status=$?
  trap - EXIT INT TERM HUP
  if [[ -n "${SERVER_PID:-}" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill -- "-$SERVER_PID" 2>/dev/null || kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  if [[ -n "${WORKDIR:-}" && -d "$WORKDIR" ]]; then
    rm -rf -- "$WORKDIR" 2>/dev/null || true
  fi
  exit "$status"
}
trap cleanup EXIT INT TERM HUP

[[ "$(uname -m)" == "aarch64" ]] || fail architecture_not_aarch64
[[ -f /system/bin/app_process64 && -x /system/bin/app_process64 ]] \
  || fail app_process64_missing
command -v curl >/dev/null 2>&1 || fail curl_missing
command -v sha256sum >/dev/null 2>&1 || fail sha256sum_missing
command -v jq >/dev/null 2>&1 || fail jq_missing

if [[ -z "$ARTIFACT" ]]; then
  if [[ -x "$REPO_ROOT/target/release/termux-mcp-server" ]]; then
    ARTIFACT="$REPO_ROOT/target/release/termux-mcp-server"
  else
    fail artifact_missing
  fi
fi
[[ -x "$ARTIFACT" && ! -L "$ARTIFACT" ]] || fail artifact_invalid
[[ -f "$DEX_PATH" && ! -L "$DEX_PATH" ]] || fail dex_missing

DEX_MODE="$(stat -c '%a' -- "$DEX_PATH" 2>/dev/null || true)"
DEX_OWNER="$(stat -c '%u' -- "$DEX_PATH" 2>/dev/null || true)"
DEX_LINKS="$(stat -c '%h' -- "$DEX_PATH" 2>/dev/null || true)"
[[ "$DEX_MODE" == "400" && "$DEX_OWNER" == "$(id -u)" && "$DEX_LINKS" == "1" ]] \
  || fail dex_identity_invalid
DEX_PARENT="$(dirname -- "$DEX_PATH")"
[[ "$(stat -c '%a' -- "$DEX_PARENT")" == "700" ]] || fail dex_parent_mode_invalid
DEX_SHA="$(sha256sum -- "$DEX_PATH" | awk '{print $1}')"
[[ "$DEX_SHA" =~ ^[0-9a-f]{64}$ ]] || fail dex_digest_invalid

# Live rish probe before starting the candidate.
export RISH_APPLICATION_ID="${RISH_APPLICATION_ID:-com.termux}"
export RISH_PRESERVE_ENV=0
RISH_BIN=""
for candidate in "$HOME/.local/bin/rish" "$PREFIX/bin/rish" rish; do
  if command -v "$candidate" >/dev/null 2>&1 || [[ -x "$candidate" ]]; then
    RISH_BIN="$candidate"
    break
  fi
done
[[ -n "$RISH_BIN" ]] || fail rish_launcher_missing

RISH_OUT="$(
  timeout 8 "$RISH_BIN" -c 'id -u' 2>/dev/null || true
)"
if [[ "$RISH_OUT" != $'2000\n' && "$RISH_OUT" != "2000" ]]; then
  log "Shizuku/rish is not providing shell UID 2000."
  log "Open the Shizuku app and start it via Wireless debugging / ADB, then re-run."
  fail shizuku_server_not_shell_uid
fi
log "trusted direct rish probe: uid=2000"

WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/operator-local-s25.XXXXXX")"
chmod 700 "$WORKDIR"
SAFE_ROOT="$WORKDIR/safe-root"
mkdir -m 700 "$SAFE_ROOT"
LOG="$WORKDIR/server.log"
export MCP__AUTH__STATIC_TOKEN="$TOKEN"
export MCP__SERVER__HOST="$HOST"
export MCP__SERVER__PORT="$PORT"
export MCP__TRANSPORT__ALLOWED_HOSTS="localhost:$PORT,127.0.0.1:$PORT"
export MCP__TRANSPORT__ALLOWED_ORIGINS="http://localhost:$PORT,http://127.0.0.1:$PORT"
export MCP__FILE__SAFE_ROOTS="$SAFE_ROOT"
export MCP__ANDROID__RISH_ENABLED=true
export MCP__ANDROID__RISH_DEX_PATH="$DEX_PATH"
export MCP__ANDROID__RISH_DEX_SHA256="$DEX_SHA"
export RUST_LOG=termux_mcp_server=info

setsid "$ARTIFACT" >"$LOG" 2>&1 &
SERVER_PID=$!
READY=0
for _ in $(seq 1 40); do
  if curl --fail --silent --show-error \
    --connect-timeout 1 --max-time 2 \
    "http://127.0.0.1:$PORT/health" >/dev/null 2>&1
  then
    READY=1
    break
  fi
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    cat "$LOG" >&2 || true
    fail server_exited_early
  fi
  sleep 0.25
done
((READY == 1)) || fail server_health_timeout

# MCP initialize + tools/call android_rish_status
HEADER_FILE="$WORKDIR/init.headers"
BODY_FILE="$WORKDIR/init.body"
curl --fail --silent --show-error \
  --connect-timeout 2 --max-time 5 \
  -D "$HEADER_FILE" -o "$BODY_FILE" \
  -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"operator-local-s25","version":"1"}}}' \
  "http://127.0.0.1:$PORT/mcp"
SESSION="$(awk 'BEGIN{IGNORECASE=1} /^mcp-session-id:/{print $2}' "$HEADER_FILE" | tr -d '\r')"
[[ -n "$SESSION" ]] || fail mcp_session_missing
jq -e '.result.protocolVersion != null' "$BODY_FILE" >/dev/null || fail mcp_initialize_invalid

STATUS_JSON="$(
  curl --fail --silent --show-error \
    --connect-timeout 2 --max-time 10 \
    -H "Authorization: Bearer $TOKEN" \
    -H "Mcp-Session-Id: $SESSION" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json' \
    -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"android_rish_status","arguments":{}}}' \
    "http://127.0.0.1:$PORT/mcp"
)"
printf '%s\n' "$STATUS_JSON" >"$WORKDIR/status.json"
# Tool result may be content[] text JSON
PAYLOAD="$(
  jq -er '
    if .result.structuredContent != null then .result.structuredContent
    elif (.result.content | type) == "array" then
      (.result.content[] | select(.type=="text") | .text | fromjson)
    else empty end
  ' <<<"$STATUS_JSON"
)" || fail status_payload_invalid

printf '%s\n' "$PAYLOAD" | jq -e '
  .available == true
  and .backend == "shizuku_rish"
  and .principal == "android_shell"
  and .uid == 2000
  and .state == "verified_shell_uid"
  and .rootAccepted == false
  and .arbitraryShell == false
  and .mutationReady == false
' >/dev/null || fail status_contract_invalid

# Extra arguments must fail closed.
EXTRA_RC=0
EXTRA_OUT="$(
  curl --silent --show-error \
    --connect-timeout 2 --max-time 10 \
    -H "Authorization: Bearer $TOKEN" \
    -H "Mcp-Session-Id: $SESSION" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json' \
    -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"android_rish_status","arguments":{"shell":"id"}}}' \
    "http://127.0.0.1:$PORT/mcp" || true
)"
printf '%s\n' "$EXTRA_OUT" | jq -e '
  (.error != null) or (.result.isError == true)
' >/dev/null || fail extra_arguments_not_rejected

EVIDENCE="$WORKDIR/operator-local-s25-evidence.json"
ARTIFACT_SHA="$(sha256sum -- "$ARTIFACT" | awk '{print $1}')"
jq -cn \
  --arg generatedAt "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg artifactSha "$ARTIFACT_SHA" \
  --arg dexSha "$DEX_SHA" \
  --arg sdk "$(getprop ro.build.version.sdk 2>/dev/null || echo unknown)" \
  --arg abi "$(getprop ro.product.cpu.abi 2>/dev/null || echo unknown)" \
  --argjson payload "$PAYLOAD" '
  {
    schemaVersion: 1,
    qualificationClass: "physical_shizuku_rish_identity_development_v1",
    scope: "s2_5_uid_probe_only",
    evidenceKind: "operator_local_on_device_v1",
    releaseEligible: false,
    productionControlQualified: false,
    generatedAt: $generatedAt,
    environment: {
      architecture: "aarch64",
      abi: $abi,
      apiLevel: $sdk,
      physicalDeviceObserved: true
    },
    artifact: {
      sha256: $artifactSha,
      features: ["android-rish"]
    },
    dex: { sha256: $dexSha, mode: "0400" },
    androidRishStatus: $payload,
    claimBoundary: {
      s3Attestation: false,
      typedReads: false,
      grantV2: false,
      deviceMutation: false,
      productionControl: false
    }
  }
' >"$EVIDENCE"
chmod 600 "$EVIDENCE"

log "PASS android_rish_status verified_shell_uid uid=2000"
log "evidence=$EVIDENCE"
printf 'OPERATOR_LOCAL_S25_RESULT=PASS evidence=%s\n' "$EVIDENCE"
# Keep evidence outside workdir cleanup by copying to a stable path
STABLE="${OPERATOR_LOCAL_S25_EVIDENCE:-$HOME/.local/share/termux-mcp-edge/evidence/operator-local-s25-evidence.json}"
mkdir -p "$(dirname -- "$STABLE")"
chmod 700 "$(dirname -- "$STABLE")"
cp -f -- "$EVIDENCE" "$STABLE"
chmod 600 "$STABLE"
log "stable_evidence=$STABLE"
