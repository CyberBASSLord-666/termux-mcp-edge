#!/usr/bin/env bash
# On-device S2.5 operator gate for Termux on physical AArch64 Android.
# Does not replace the protected GitHub physical workflow. It records the same
# narrow exact-token diagnostic locally, without claiming trustworthy remote
# exit/stream separation or Binder identity/lifecycle.
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077
set +x

readonly ENABLED_TOOLS='["runtime_status","platform_info","android_status","project_service_status","create_directory","copy_file","trash_file","find_paths","hash_file","list_directory","path_metadata","read_binary_file","read_binary_range","read_file","read_text_range","search_text","write_file","android_rish_status"]'
readonly -a ART_RUNTIME_ENV_KEYS=(
  ANDROID_ART_ROOT
  ANDROID_ASSETS
  ANDROID_DATA
  ANDROID_I18N_ROOT
  ANDROID_ROOT
  ANDROID_RUNTIME_ROOT
  ANDROID_STORAGE
  ANDROID_TZDATA_ROOT
  ANDROID__BUILD_VERSION_SDK
  BOOTCLASSPATH
  DEX2OATBOOTCLASSPATH
  SYSTEMSERVERCLASSPATH
)

fail() {
  printf '[operator-local-s25] FAIL reason=%s\n' "$1" >&2
  exit 1
}
log() { printf '[operator-local-s25] %s\n' "$*"; }

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
ARTIFACT=""
DEX_PATH="${MCP__ANDROID__RISH_DEX_PATH:-$HOME/.local/share/termux-mcp-edge/rish/rish_shizuku.dex}"
TOKEN=""
HOST="127.0.0.1"
PORT="${OPERATOR_LOCAL_S25_PORT:-19177}"
WORKDIR=""
SERVER_PID=""
ART_RUNTIME_ENV=()

capture_art_runtime_environment() {
  local key
  ART_RUNTIME_ENV=()
  for key in "${ART_RUNTIME_ENV_KEYS[@]}"; do
    if [[ -v "$key" && -n "${!key}" ]]; then
      ART_RUNTIME_ENV+=("$key=${!key}")
    fi
  done
}

is_sha256() {
  [[ "$1" =~ ^[0-9a-f]{64}$ ]]
}

stop_server_bounded() {
  local pid="${SERVER_PID:-}"
  [[ -z "$pid" ]] && return 0
  [[ "$pid" =~ ^[1-9][0-9]*$ ]] || return 1
  kill -TERM -- "-$pid" 2>/dev/null || kill -TERM "$pid" 2>/dev/null || true
  for _ in $(seq 1 50); do
    if ! kill -0 -- "-$pid" 2>/dev/null; then
      wait "$pid" 2>/dev/null || true
      SERVER_PID=""
      return 0
    fi
    sleep 0.1
  done
  kill -KILL -- "-$pid" 2>/dev/null || kill -KILL "$pid" 2>/dev/null || true
  for _ in $(seq 1 20); do
    if ! kill -0 -- "-$pid" 2>/dev/null; then
      wait "$pid" 2>/dev/null || true
      SERVER_PID=""
      return 0
    fi
    sleep 0.1
  done
  return 1
}

cleanup_runtime() {
  local cleanup_status=0
  stop_server_bounded || cleanup_status=1
  if [[ -n "${WORKDIR:-}" && -d "$WORKDIR" ]]; then
    rm -rf -- "$WORKDIR" 2>/dev/null || cleanup_status=1
  fi
  if [[ -n "${WORKDIR:-}" && (-e "$WORKDIR" || -L "$WORKDIR") ]]; then
    cleanup_status=1
  else
    WORKDIR=""
  fi
  return "$cleanup_status"
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM HUP
  cleanup_runtime || status=1
  exit "$status"
}

uid_predicate_observed_in_one_bounded_capture() {
  local expected="$1" stdout="$2" stderr="$3" stdout_bytes stderr_bytes
  stdout_bytes="$(stat -c '%s' -- "$stdout" 2>/dev/null)" || return 1
  stderr_bytes="$(stat -c '%s' -- "$stderr" 2>/dev/null)" || return 1
  [[ "$stdout_bytes" =~ ^[0-9]+$ && "$stderr_bytes" =~ ^[0-9]+$ ]] || return 1
  ((stdout_bytes <= 1024 && stderr_bytes <= 4096)) || return 1
  if cmp -s -- "$expected" "$stdout" && [[ ! -s "$stderr" ]]; then
    return 0
  fi
  cmp -s -- "$expected" "$stderr" && [[ ! -s "$stdout" ]]
}

main() {
  ARTIFACT="${1:-}"
  trap cleanup EXIT INT TERM HUP

[[ "$(uname -m)" == "aarch64" ]] || fail architecture_not_aarch64
[[ -f /system/bin/app_process64 && -x /system/bin/app_process64 ]] \
  || fail app_process64_missing
command -v curl >/dev/null 2>&1 || fail curl_missing
command -v sha256sum >/dev/null 2>&1 || fail sha256sum_missing
command -v jq >/dev/null 2>&1 || fail jq_missing
command -v dd >/dev/null 2>&1 || fail dd_missing
command -v timeout >/dev/null 2>&1 || fail timeout_missing
command -v env >/dev/null 2>&1 || fail env_missing
[[ "${HOME:-}" == /data/data/com.termux/files/home ]] || fail termux_home_invalid
[[ "${PREFIX:-}" == /data/data/com.termux/files/usr ]] || fail termux_prefix_invalid
[[ -x "$PREFIX/bin/setsid" && -f "$PREFIX/bin/setsid" \
  && ! -L "$PREFIX/bin/setsid" \
  && "$(realpath -e -- "$PREFIX/bin/setsid" 2>/dev/null)" == "$PREFIX/bin/setsid" \
  && "$(stat -c '%u:%h' -- "$PREFIX/bin/setsid" 2>/dev/null)" == "$(id -u):1" ]] \
  || fail setsid_invalid

TOKEN="${MCP__AUTH__STATIC_TOKEN:-}"
if [[ -z "$TOKEN" ]]; then
  TOKEN="$(dd if=/dev/urandom bs=32 count=1 status=none \
    | sha256sum | awk '{print $1}')"
fi
is_sha256 "$TOKEN" || fail token_invalid
capture_art_runtime_environment

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

# Live rish diagnostic before starting the candidate.
RISH_BIN=""
for candidate in "$HOME/.local/bin/rish" "$PREFIX/bin/rish" rish; do
  if command -v "$candidate" >/dev/null 2>&1 || [[ -x "$candidate" ]]; then
    RISH_BIN="$candidate"
    break
  fi
done
[[ -n "$RISH_BIN" ]] || fail rish_launcher_missing

WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/operator-local-s25.XXXXXX")"
chmod 700 "$WORKDIR"
SAFE_ROOT="$WORKDIR/safe-root"
mkdir -m 700 "$SAFE_ROOT"
LOG="$WORKDIR/server.log"

RISH_STDOUT="$WORKDIR/direct-rish.stdout"
RISH_STDERR="$WORKDIR/direct-rish.stderr"
RISH_EXPECTED="$WORKDIR/direct-rish.expected"
printf '2000\n' >"$RISH_EXPECTED"
set +e
(
  # Bound capture growth before the exact post-run byte checks below.
  ulimit -f 8
  exec timeout --signal=TERM --kill-after=2s 8s \
    env -i \
      "${ART_RUNTIME_ENV[@]}" \
      "HOME=$HOME" \
      "PREFIX=$PREFIX" \
      "PATH=$PREFIX/bin:/system/bin" \
      RISH_APPLICATION_ID=com.termux \
      RISH_PRESERVE_ENV=0 \
      "$RISH_BIN" -c 'id -u'
) >"$RISH_STDOUT" 2>"$RISH_STDERR"
RISH_LOCAL_STATUS=$?
set -e
((RISH_LOCAL_STATUS == 0)) || fail rish_launcher_failed
if ! uid_predicate_observed_in_one_bounded_capture \
  "$RISH_EXPECTED" "$RISH_STDOUT" "$RISH_STDERR"
then
  log "Shizuku/rish did not produce the exact UID-2000 predicate token in one local capture."
  log "Open the Shizuku app and start it via Wireless debugging / ADB, then re-run."
  fail shizuku_uid_predicate_not_observed
fi
log "direct rish diagnostic: exact UID-2000 predicate token observed; remote transport remains unqualified"
local -a server_environment
server_environment=(
  "${ART_RUNTIME_ENV[@]}"
  "HOME=$HOME"
  "PREFIX=$PREFIX"
  "PATH=$PREFIX/bin:/system/bin"
  "MCP__AUTH__STATIC_TOKEN=$TOKEN"
  "MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=false"
  "MCP__SERVER__HOST=$HOST"
  "MCP__SERVER__PORT=$PORT"
  "MCP__TRANSPORT__ALLOWED_HOSTS=localhost:$PORT,127.0.0.1:$PORT"
  "MCP__TRANSPORT__ALLOWED_ORIGINS=http://localhost:$PORT,http://127.0.0.1:$PORT"
  "MCP__TRANSPORT__ALLOW_MISSING_ORIGIN=false"
  "MCP__TRANSPORT__SSE_ENABLED=false"
  "MCP__TRANSPORT__STATELESS_2026_07_28_ENABLED=false"
  "MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS=4"
  "MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS=15"
  "MCP__TRANSPORT__MAX_BODY_BYTES=1048576"
  "MCP__FILE__SAFE_ROOTS=$SAFE_ROOT"
  "MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=false"
  "MCP__FILE__COPY_FILE_MUTATION_ENABLED=false"
  "MCP__FILE__TRASH_FILE_MUTATION_ENABLED=false"
  "MCP__FILE__WRITE_MUTATION_ENABLED=false"
  "MCP__ANDROID__BATTERY_STATUS_ENABLED=false"
  "MCP__ANDROID__VOLUME_STATUS_ENABLED=false"
  "MCP__ANDROID__VOLUME_CONTROL_ENABLED=false"
  "MCP__COMMAND__ENABLED=false"
  "MCP__ANDROID__RISH_ENABLED=true"
  "MCP__ANDROID__RISH_DEX_PATH=$DEX_PATH"
  "MCP__ANDROID__RISH_DEX_SHA256=$DEX_SHA"
  "RUST_LOG=termux_mcp_server=info"
)

# Monitor mode can make an asynchronous child a process-group leader, which
# forces util-linux setsid to fork and makes `$!` cease to identify the group.
set +m
(
  cd /
  ulimit -f 32768
  exec env -i "${server_environment[@]}" \
    "$PREFIX/bin/setsid" --wait -- "$ARTIFACT"
) >"$LOG" 2>&1 &
SERVER_PID=$!
HOST_HEADER="127.0.0.1:$PORT"
ORIGIN_HEADER="http://127.0.0.1:$PORT"
ACCEPT_HEADER='application/json, text/event-stream'
PROTOCOL_VERSION='2025-11-25'

mcp_curl() {
  # usage: mcp_curl <curl-extra-args...>
  curl --fail --silent --show-error \
    --connect-timeout 2 --max-time 15 \
    -H "Host: $HOST_HEADER" \
    -H "Origin: $ORIGIN_HEADER" \
    -H "Authorization: Bearer $TOKEN" \
    -H 'Content-Type: application/json' \
    -H "Accept: $ACCEPT_HEADER" \
    "$@"
}

READY=0
for _ in $(seq 1 40); do
  if curl --fail --silent --show-error \
    --connect-timeout 1 --max-time 2 \
    -H "Host: $HOST_HEADER" \
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
mcp_curl \
  -D "$HEADER_FILE" -o "$BODY_FILE" \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"$PROTOCOL_VERSION\",\"capabilities\":{},\"clientInfo\":{\"name\":\"operator-local-s25\",\"version\":\"1\"}}}" \
  "http://127.0.0.1:$PORT/mcp"
SESSION="$(awk 'BEGIN{IGNORECASE=1} /^mcp-session-id:/{print $2}' "$HEADER_FILE" | tr -d '\r')"
[[ -n "$SESSION" ]] || fail mcp_session_missing
jq -e '.result.protocolVersion != null' "$BODY_FILE" >/dev/null || fail mcp_initialize_invalid

# required notifications/initialized for sessionful MCP
mcp_curl \
  -H "Mcp-Session-Id: $SESSION" \
  -H "MCP-Protocol-Version: $PROTOCOL_VERSION" \
  -d '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  "http://127.0.0.1:$PORT/mcp" >/dev/null \
  || fail mcp_initialized_notification_failed

TOOLS_JSON="$(
  mcp_curl \
    -H "Mcp-Session-Id: $SESSION" \
    -H "MCP-Protocol-Version: $PROTOCOL_VERSION" \
    -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
    "http://127.0.0.1:$PORT/mcp"
)"
printf '%s\n' "$TOOLS_JSON" | jq -e --argjson expected "$ENABLED_TOOLS" '
  [.result.tools[].name] == $expected
  and (.result.tools | map(select(.name == "android_rish_status")) | length) == 1
  and (.result.tools | map(select(.name == "android_rish_status"))[0].inputSchema
    | .type == "object"
    and .properties == {}
    and (has("required") | not)
    and .additionalProperties == false)
  and all(.result.tools[]
    | select(.name == "create_directory" or .name == "copy_file"
      or .name == "trash_file" or .name == "write_file");
    .inputSchema.properties.dry_run.const == true)
' >/dev/null || fail enabled_tool_posture_invalid

RUNTIME_JSON="$(
  mcp_curl \
    -H "Mcp-Session-Id: $SESSION" \
    -H "MCP-Protocol-Version: $PROTOCOL_VERSION" \
    -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"runtime_status","arguments":{}}}' \
    "http://127.0.0.1:$PORT/mcp"
)"
printf '%s\n' "$RUNTIME_JSON" | jq -e --argjson expected "$ENABLED_TOOLS" '
  .result.isError == false
  and .result.structuredContent.availableTools == $expected
  and .result.structuredContent.androidRishCompiled == true
  and .result.structuredContent.androidRishEnabled == true
  and .result.structuredContent.androidRishMode == "configured_probe_on_call_adb_shell_uid_2000"
  and .result.structuredContent.androidRishArbitraryShell == false
  and .result.structuredContent.androidRishMutations == false
  and .result.structuredContent.createDirectoryMutationEnabled == false
  and .result.structuredContent.copyFileMutationEnabled == false
  and .result.structuredContent.trashFileMutationEnabled == false
  and .result.structuredContent.fileWriteMutationEnabled == false
  and .result.structuredContent.androidVolumeControlEnabled == false
  and .result.structuredContent.commandExecution == false
' >/dev/null || fail runtime_status_contract_invalid

STATUS_JSON="$(
  mcp_curl \
    -H "Mcp-Session-Id: $SESSION" \
    -H "MCP-Protocol-Version: $PROTOCOL_VERSION" \
    -d '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"android_rish_status","arguments":{}}}' \
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
EXTRA_OUT="$(
  curl --silent --show-error \
    --connect-timeout 2 --max-time 10 \
    -H "Host: $HOST_HEADER" \
    -H "Origin: $ORIGIN_HEADER" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Mcp-Session-Id: $SESSION" \
    -H "MCP-Protocol-Version: $PROTOCOL_VERSION" \
    -H 'Content-Type: application/json' \
    -H "Accept: $ACCEPT_HEADER" \
    -d '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"android_rish_status","arguments":{"shell":"id"}}}' \
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

# Keep evidence outside workdir cleanup by copying to a stable path
STABLE="${OPERATOR_LOCAL_S25_EVIDENCE:-$HOME/.local/share/termux-mcp-edge/evidence/operator-local-s25-evidence.json}"
mkdir -p "$(dirname -- "$STABLE")"
chmod 700 "$(dirname -- "$STABLE")"
cp -f -- "$EVIDENCE" "$STABLE"
chmod 600 "$STABLE"
if ! cleanup_runtime; then
  rm -f -- "$STABLE" 2>/dev/null || true
  fail cleanup_unconfirmed
fi
trap - EXIT INT TERM HUP
log "PASS android_rish_status exact UID-2000 predicate token observed; remote exit/stream/Binder lifecycle unqualified"
log "stable_evidence=$STABLE"
printf 'OPERATOR_LOCAL_S25_RESULT=PASS evidence=%s\n' "$STABLE"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
