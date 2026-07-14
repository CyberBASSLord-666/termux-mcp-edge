#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
umask 077

GATE_VERSION=2
EXPECTED_IMAGE='termux/termux-docker:aarch64'
DEFAULT_PORT=18767

ARTIFACT_DIR=''
EXPECTED_COMMIT=''
EXPECTED_VERSION=''
CI_RUN_ID=''
SECURITY_RUN_ID=''
ANDROID_RUN_ID=''
OUTPUT_REPORT=''
PORT="$DEFAULT_PORT"

STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
WORK_ROOT=''
SERVER_PID=''
SESSION_ID=''
BATTERY_PROGRAM=''
BATTERY_PROGRAM_CREATED=false
BATTERY_DIRECT_PID_FILE=''
BATTERY_DESCENDANT_PID_FILE=''
REQUEST_COUNT=0
MCP_STATUS=''

log() { printf '[termux-battery-emulated] %s\n' "$*"; }
fail() {
  printf 'TERMUX_MCP_BATTERY_EMULATED_RESULT=FAIL reason=%s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage: termux_battery_emulated_gate.sh \
  --artifact-dir DIR \
  --expected-commit SHA \
  --expected-version VERSION \
  --ci-run-id ID \
  --security-run-id ID \
  --android-run-id ID \
  --output REPORT.json \
  [--port PORT]

Run an exact android-battery-status artifact natively in the pinned official
ARM64 Termux environment. A temporary fixed-path Termux:API fixture validates
the compile gate, runtime gate, process boundary, output bounds, normalization,
audit-visible error contract, and disabled discovery without Android hardware.
EOF
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM HUP
  if [[ -n "$SERVER_PID" ]] && kill -0 "$SERVER_PID" >/dev/null 2>&1; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  SERVER_PID=''
  unset MCP_TOKEN SESSION_ID 2>/dev/null || true
  if [[ "$BATTERY_PROGRAM_CREATED" == true && -n "$BATTERY_PROGRAM" && "$BATTERY_PROGRAM" == /data/data/com.termux/files/usr/bin/termux-battery-status ]]; then
    rm -f -- "$BATTERY_PROGRAM" >/dev/null 2>&1 || status=1
  fi
  [[ -z "$WORK_ROOT" ]] || rm -rf -- "$WORK_ROOT" >/dev/null 2>&1 || status=1
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

while (($#)); do
  case "$1" in
    --artifact-dir) (($# >= 2)) || fail missing_artifact_dir; ARTIFACT_DIR="$2"; shift 2 ;;
    --expected-commit) (($# >= 2)) || fail missing_expected_commit; EXPECTED_COMMIT="$2"; shift 2 ;;
    --expected-version) (($# >= 2)) || fail missing_expected_version; EXPECTED_VERSION="$2"; shift 2 ;;
    --ci-run-id) (($# >= 2)) || fail missing_ci_run_id; CI_RUN_ID="$2"; shift 2 ;;
    --security-run-id) (($# >= 2)) || fail missing_security_run_id; SECURITY_RUN_ID="$2"; shift 2 ;;
    --android-run-id) (($# >= 2)) || fail missing_android_run_id; ANDROID_RUN_ID="$2"; shift 2 ;;
    --output) (($# >= 2)) || fail missing_output; OUTPUT_REPORT="$2"; shift 2 ;;
    --port) (($# >= 2)) || fail missing_port; PORT="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) fail unknown_argument ;;
  esac
done

[[ "$EXPECTED_COMMIT" =~ ^[0-9a-f]{40}$ ]] || fail expected_commit_invalid
[[ "$EXPECTED_VERSION" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] || fail expected_version_invalid
[[ "$CI_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail ci_run_id_invalid
[[ "$SECURITY_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail security_run_id_invalid
[[ "$ANDROID_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail android_run_id_invalid
[[ "$PORT" =~ ^[0-9]+$ ]] || fail port_invalid
((PORT >= 1024 && PORT <= 65535)) || fail port_invalid
[[ "$ARTIFACT_DIR" == /* && "$OUTPUT_REPORT" == /* ]] || fail absolute_paths_required

[[ "${TERMUX_MCP_EMULATED_ENVIRONMENT:-}" == official-termux-docker-native-arm64 ]] || fail environment_attestation_missing
IMAGE_DIGEST="${TERMUX_MCP_TERMUX_IMAGE_DIGEST:-}"
[[ "$IMAGE_DIGEST" =~ ^sha256:[0-9a-f]{64}$ ]] || fail image_digest_invalid
[[ "$(uname -m)" == aarch64 || "$(uname -m)" == arm64 ]] || fail architecture_not_arm64
[[ "${PREFIX:-}" == /data/data/com.termux/files/usr ]] || fail termux_prefix_invalid
[[ "${HOME:-}" == /data/data/com.termux/files/home ]] || fail termux_home_invalid
[[ -x /system/bin/linker64 ]] || fail android_linker_missing

for command in awk bash cat chmod curl date dd dirname file find grep install jq kill mkdir mktemp readlink realpath rm seq sha256sum sleep stat timeout uname wc; do
  command -v "$command" >/dev/null 2>&1 || fail "required_command_missing_$command"
done
[[ "$(command -v readlink)" == /data/data/com.termux/files/usr/bin/readlink ]] || fail readlink_path_invalid

ARTIFACT="$ARTIFACT_DIR/termux-mcp-server"
MANIFEST="$ARTIFACT_DIR/artifact-manifest.json"
CHECKSUMS="$ARTIFACT_DIR/SHA256SUMS"
for path in "$ARTIFACT" "$MANIFEST" "$CHECKSUMS"; do
  [[ -f "$path" && ! -L "$path" ]] || fail artifact_bundle_member_invalid
done
[[ -x "$ARTIFACT" ]] || fail artifact_binary_not_executable
[[ "$(find "$ARTIFACT_DIR" -mindepth 1 -maxdepth 1 -type f | wc -l)" == 3 ]] || fail artifact_bundle_member_count_invalid
(cd "$ARTIFACT_DIR" && sha256sum -c SHA256SUMS >/dev/null) || fail artifact_checksum_invalid

jq -e \
  --arg commit "$EXPECTED_COMMIT" \
  --arg version "$EXPECTED_VERSION" \
  --arg run_id "$ANDROID_RUN_ID" '
    (keys == ["artifactName","bytes","commit","createdAt","elf","features","fileName","posture","repository","schemaVersion","sha256","target","version","workflowRunId"])
    and .schemaVersion == 1
    and .repository == "CyberBASSLord-666/termux-mcp-edge"
    and .commit == $commit
    and .workflowRunId == $run_id
    and .artifactName == "termux-mcp-server-aarch64-linux-android-android-battery-status"
    and .posture == "android-battery-status"
    and .features == ["android-battery-status"]
    and .target == "aarch64-linux-android"
    and .fileName == "termux-mcp-server"
    and .version == $version
    and .elf == "aarch64-android-elf"
    and (.sha256 | test("^[0-9a-f]{64}$"))
    and (.bytes >= 1 and .bytes <= 67108864)
  ' "$MANIFEST" >/dev/null || fail artifact_manifest_invalid

ARTIFACT_SHA="$(jq -r .sha256 "$MANIFEST")"
ARTIFACT_BYTES="$(jq -r .bytes "$MANIFEST")"
[[ "$(sha256sum "$ARTIFACT" | awk '{print $1}')" == "$ARTIFACT_SHA" ]] || fail artifact_digest_mismatch
[[ "$(stat -c %s "$ARTIFACT")" == "$ARTIFACT_BYTES" ]] || fail artifact_size_mismatch
identity="$(file -b "$ARTIFACT")" || fail artifact_identity_failed
[[ "$identity" == *ELF* && "$identity" == *"ARM aarch64"* ]] || fail artifact_architecture_mismatch
[[ "$identity" == *Android* || "$identity" == *"/system/bin/linker64"* ]] || fail artifact_android_identity_missing
[[ "$(timeout -k 2 5 "$ARTIFACT" --version)" == "termux-mcp-server $EXPECTED_VERSION" ]] || fail artifact_version_mismatch

OUTPUT_PARENT="$(dirname "$OUTPUT_REPORT")"
[[ -d "$OUTPUT_PARENT" && ! -L "$OUTPUT_PARENT" ]] || fail output_parent_invalid
[[ "$(realpath -e "$OUTPUT_PARENT")" == "$OUTPUT_PARENT" ]] || fail output_parent_not_canonical
[[ "$(stat -c %a "$OUTPUT_PARENT")" == 700 ]] || fail output_parent_not_private
[[ ! -e "$OUTPUT_REPORT" && ! -L "$OUTPUT_REPORT" ]] || fail output_already_exists

WORK_ROOT="$(mktemp -d "$HOME/.termux-mcp-battery-gate.XXXXXX")" || fail work_root_create_failed
chmod 700 "$WORK_ROOT"
SAFE_ROOT="$WORK_ROOT/safe-root"
SERVER_LOG="$WORK_ROOT/server.log"
BODY_FILE="$WORK_ROOT/body.json"
HEADER_FILE="$WORK_ROOT/headers.txt"
REQUEST_FILE="$WORK_ROOT/request.json"
BATTERY_DIRECT_PID_FILE="$WORK_ROOT/battery-direct.pid"
BATTERY_DESCENDANT_PID_FILE="$WORK_ROOT/battery-descendant.pid"
mkdir -m 700 "$SAFE_ROOT"

MCP_TOKEN="$(dd if=/dev/urandom bs=32 count=1 status=none | sha256sum | awk '{print $1}')"
[[ "$MCP_TOKEN" =~ ^[0-9a-f]{64}$ ]] || fail token_generation_failed

BATTERY_PROGRAM="$PREFIX/bin/termux-battery-status"
[[ "$BATTERY_PROGRAM" == /data/data/com.termux/files/usr/bin/termux-battery-status ]] || fail battery_program_path_invalid
[[ ! -e "$BATTERY_PROGRAM" && ! -L "$BATTERY_PROGRAM" ]] || fail battery_program_already_present
BATTERY_PROGRAM_CREATED=true

write_success_fixture() {
  local next="$WORK_ROOT/battery-success.next"
  cat >"$next" <<'EOF'
#!/data/data/com.termux/files/usr/bin/bash
set -euo pipefail
[[ "$#" -eq 0 ]]
[[ "$PWD" == / ]]
[[ "$(/data/data/com.termux/files/usr/bin/readlink /proc/self/fd/0)" == /dev/null ]]
[[ -z "${MCP__AUTH__STATIC_TOKEN+x}" ]]
[[ -z "${MCP__ANDROID__BATTERY_STATUS_ENABLED+x}" ]]
printf '%s' '{"present":true,"technology":"vendor-private","health":"GOOD","plugged":"PLUGGED_USB","status":"CHARGING","temperature":31.2,"voltage":4210,"current":123456,"current_average":120000,"percentage":87,"level":87,"scale":100,"charge_counter":4100000,"energy":17000000,"cycle":234,"android_id":"private-identifier"}'
EOF
  chmod 700 "$next"
  install -m 700 "$next" "$BATTERY_PROGRAM"
  rm -f -- "$next"
}

write_overflow_fixture() {
  local next="$WORK_ROOT/battery-overflow.next"
  cat >"$next" <<'EOF'
#!/data/data/com.termux/files/usr/bin/bash
set -euo pipefail
[[ "$#" -eq 0 ]]
[[ "$PWD" == / ]]
i=0
while ((i < 16385)); do
  printf x
  i=$((i + 1))
done
EOF
  chmod 700 "$next"
  install -m 700 "$next" "$BATTERY_PROGRAM"
  rm -f -- "$next"
}

write_endless_output_fixture() {
  local stream="$1" next="$WORK_ROOT/battery-endless.next" redirection=''
  case "$stream" in
    stdout) redirection='' ;;
    stderr) redirection='>&2' ;;
    *) fail endless_stream_invalid ;;
  esac
  rm -f -- "$BATTERY_DIRECT_PID_FILE" "$BATTERY_DESCENDANT_PID_FILE"
  cat >"$next" <<EOF
#!/data/data/com.termux/files/usr/bin/bash
set -euo pipefail
[[ "\$#" -eq 0 ]]
[[ "\$PWD" == / ]]
printf '%s\n' "\$\$" >'$BATTERY_DIRECT_PID_FILE'
while :; do
  printf '%s' xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx $redirection
done
EOF
  chmod 700 "$next"
  install -m 700 "$next" "$BATTERY_PROGRAM"
  rm -f -- "$next"
}

write_pipe_holding_descendant_fixture() {
  local stream="$1" next="$WORK_ROOT/battery-pipe-holder.next" redirection=''
  case "$stream" in
    stdout) redirection='2>/dev/null' ;;
    stderr) redirection='>/dev/null' ;;
    *) fail pipe_holder_stream_invalid ;;
  esac
  rm -f -- "$BATTERY_DIRECT_PID_FILE" "$BATTERY_DESCENDANT_PID_FILE"
  cat >"$next" <<EOF
#!/data/data/com.termux/files/usr/bin/bash
set -euo pipefail
[[ "\$#" -eq 0 ]]
[[ "\$PWD" == / ]]
printf '%s\n' "\$\$" >'$BATTERY_DIRECT_PID_FILE'
$PREFIX/bin/sleep 30 $redirection &
printf '%s\n' "\$!" >'$BATTERY_DESCENDANT_PID_FILE'
printf '%s' '{"percentage":50}'
exit 0
EOF
  chmod 700 "$next"
  install -m 700 "$next" "$BATTERY_PROGRAM"
  rm -f -- "$next"
}

write_cancellation_fixture() {
  local next="$WORK_ROOT/battery-cancellation.next"
  rm -f -- "$BATTERY_DIRECT_PID_FILE" "$BATTERY_DESCENDANT_PID_FILE"
  cat >"$next" <<EOF
#!/data/data/com.termux/files/usr/bin/bash
set -euo pipefail
[[ "\$#" -eq 0 ]]
[[ "\$PWD" == / ]]
printf '%s\n' "\$\$" >'$BATTERY_DIRECT_PID_FILE'
$PREFIX/bin/sleep 30 >/dev/null 2>&1 &
printf '%s\n' "\$!" >'$BATTERY_DESCENDANT_PID_FILE'
wait
EOF
  chmod 700 "$next"
  install -m 700 "$next" "$BATTERY_PROGRAM"
  rm -f -- "$next"
}

write_failure_fixture() {
  local next="$WORK_ROOT/battery-failure.next"
  cat >"$next" <<'EOF'
#!/data/data/com.termux/files/usr/bin/bash
set -euo pipefail
[[ "$#" -eq 0 ]]
[[ "$PWD" == / ]]
exit 7
EOF
  chmod 700 "$next"
  install -m 700 "$next" "$BATTERY_PROGRAM"
  rm -f -- "$next"
}

curl_local() {
  command curl --disable --proto '=http' --noproxy '*' --connect-timeout 2 --max-time 10 "$@"
}

start_server() {
  local enabled="$1"
  MCP__AUTH__STATIC_TOKEN="$MCP_TOKEN" \
  MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=false \
  MCP__ANDROID__BATTERY_STATUS_ENABLED="$enabled" \
  MCP__SERVER__HOST=127.0.0.1 \
  MCP__SERVER__PORT="$PORT" \
  MCP__TRANSPORT__ALLOWED_HOSTS="localhost:$PORT,127.0.0.1:$PORT" \
  MCP__TRANSPORT__ALLOWED_ORIGINS="http://localhost:$PORT,http://127.0.0.1:$PORT" \
  MCP__TRANSPORT__ALLOW_MISSING_ORIGIN=false \
  MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS=4 \
  MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS=30 \
  MCP__TRANSPORT__MAX_BODY_BYTES=32768 \
  MCP__FILE__SAFE_ROOTS="$SAFE_ROOT" \
  RUST_LOG=termux_mcp_server=info \
    "$ARTIFACT" >"$SERVER_LOG" 2>&1 &
  SERVER_PID=$!
  local attempt
  for attempt in $(seq 1 100); do
    kill -0 "$SERVER_PID" >/dev/null 2>&1 || fail server_exited
    if [[ "$(curl_local --silent --max-time 2 "http://127.0.0.1:$PORT/health" 2>/dev/null || true)" == ok ]]; then
      return 0
    fi
    sleep 0.1
  done
  fail "server_not_ready_after_${attempt}_attempts"
}

stop_server() {
  [[ -n "$SERVER_PID" ]] || return 0
  kill "$SERVER_PID" >/dev/null 2>&1 || fail server_shutdown_signal_failed
  wait "$SERVER_PID" 2>/dev/null || true
  SERVER_PID=''
  SESSION_ID=''
}

post_mcp() {
  local payload="$1" session="${2:-}" max_time="${3:-10}"
  printf '%s' "$payload" >"$REQUEST_FILE"
  local -a headers=(
    -H "Authorization: Bearer $MCP_TOKEN"
    -H "Host: localhost:$PORT"
    -H "Origin: http://localhost:$PORT"
    -H 'Content-Type: application/json'
    -H 'Accept: application/json, text/event-stream'
  )
  if [[ -n "$session" ]]; then
    headers+=( -H "MCP-Session-Id: $session" -H 'MCP-Protocol-Version: 2025-11-25' )
  fi
  MCP_STATUS="$(curl_local --silent --show-error --max-time "$max_time" --output "$BODY_FILE" --write-out '%{http_code}' \
    "${headers[@]}" --data-binary "@$REQUEST_FILE" "http://127.0.0.1:$PORT/mcp")"
  REQUEST_COUNT=$((REQUEST_COUNT + 1))
}

cancel_mcp_request() {
  printf '%s' '{"jsonrpc":"2.0","id":"cancelled-battery","method":"tools/call","params":{"name":"android_battery_status","arguments":{}}}' >"$REQUEST_FILE"
  local curl_rc
  set +e
  curl_local --silent --show-error --max-time 1 --output "$BODY_FILE" \
    -H "Authorization: Bearer $MCP_TOKEN" \
    -H "Host: localhost:$PORT" \
    -H "Origin: http://localhost:$PORT" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    -H "MCP-Session-Id: $SESSION_ID" \
    -H 'MCP-Protocol-Version: 2025-11-25' \
    --data-binary "@$REQUEST_FILE" "http://127.0.0.1:$PORT/mcp"
  curl_rc=$?
  set -e
  REQUEST_COUNT=$((REQUEST_COUNT + 1))
  [[ "$curl_rc" == 28 ]] || fail cancellation_request_not_aborted
}

read_fixture_pid() {
  local path="$1" reason="$2" attempt value=''
  for attempt in $(seq 1 200); do
    if [[ -s "$path" ]]; then
      value="$(cat "$path")"
      [[ "$value" =~ ^[1-9][0-9]*$ ]] || fail "${reason}_invalid"
      printf '%s\n' "$value"
      return 0
    fi
    sleep 0.01
  done
  fail "${reason}_missing"
}

assert_process_gone() {
  local pid="$1" reason="$2" attempt
  for attempt in $(seq 1 200); do
    if ! kill -0 "$pid" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.01
  done
  fail "${reason}_survived"
}

initialize_session() {
  printf '%s' '{"jsonrpc":"2.0","id":"initialize","method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"termux-battery-emulated-gate","version":"1"}}}' >"$REQUEST_FILE"
  MCP_STATUS="$(curl_local --silent --show-error --dump-header "$HEADER_FILE" --output "$BODY_FILE" --write-out '%{http_code}' \
    -H "Authorization: Bearer $MCP_TOKEN" \
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
    --data-binary "@$REQUEST_FILE" "http://127.0.0.1:$PORT/mcp")"
  REQUEST_COUNT=$((REQUEST_COUNT + 1))
  [[ "$MCP_STATUS" == 200 ]] || fail initialize_status_invalid
  jq -e --arg version "$EXPECTED_VERSION" '
    .result.protocolVersion == "2025-11-25"
    and .result.serverInfo.version == $version
  ' "$BODY_FILE" >/dev/null || fail initialize_body_invalid
  SESSION_ID="$(awk 'tolower($1) == "mcp-session-id:" {sub(/^[^:]*:[[:space:]]*/, ""); sub(/\r$/, ""); print; exit}' "$HEADER_FILE")"
  [[ "$SESSION_ID" =~ ^[A-Za-z0-9-]{1,128}$ ]] || fail session_invalid
  post_mcp '{"jsonrpc":"2.0","method":"notifications/initialized"}' "$SESSION_ID"
  [[ "$MCP_STATUS" == 202 && ! -s "$BODY_FILE" ]] || fail initialized_notification_invalid
}

log 'validating enabled battery posture'
write_success_fixture
start_server true
initialize_session

post_mcp '{"jsonrpc":"2.0","id":"tools","method":"tools/list"}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail tools_status_invalid
jq -e '
  [.result.tools[].name] == [
    "runtime_status",
    "platform_info",
    "android_status",
    "project_service_status",
    "list_directory",
    "read_file",
    "search_text",
    "write_file",
    "android_battery_status"
  ]
  and (.result.tools[] | select(.name == "android_battery_status") | .inputSchema)
      == {"type":"object","properties":{},"additionalProperties":false}
' "$BODY_FILE" >/dev/null || fail enabled_tool_discovery_invalid

post_mcp '{"jsonrpc":"2.0","id":"runtime","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail runtime_status_http_invalid
jq -e '
  .result.structuredContent.androidPlatformTools == true
  and .result.structuredContent.androidPlatformToolMode == "read_only_battery_telemetry"
  and .result.structuredContent.androidBatteryStatusCompiled == true
  and .result.structuredContent.androidBatteryStatusEnabled == true
  and .result.structuredContent.androidDeviceControl == false
  and .result.structuredContent.commandExecution == false
  and .result.structuredContent.highImpactTools == false
' "$BODY_FILE" >/dev/null || fail runtime_status_gate_invalid

post_mcp '{"jsonrpc":"2.0","id":"battery","method":"tools/call","params":{"name":"android_battery_status","arguments":{}}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail battery_status_http_invalid
jq -e '
  .result.isError == false
  and (.result.structuredContent | keys) == [
    "charge_counter_microamp_hours",
    "current_average_microamps",
    "current_microamps",
    "cycle_count",
    "energy_nanowatt_hours",
    "health",
    "level",
    "percentage",
    "plugged",
    "present",
    "scale",
    "status",
    "temperature_celsius",
    "voltage_millivolts"
  ]
  and .result.structuredContent.present == true
  and .result.structuredContent.percentage == 87
  and .result.structuredContent.temperature_celsius == 31.2
  and .result.structuredContent.voltage_millivolts == 4210
  and .result.structuredContent.current_microamps == 123456
  and .result.structuredContent.cycle_count == 234
' "$BODY_FILE" >/dev/null || fail battery_normalization_invalid
if grep -Fq -e vendor-private -e private-identifier "$BODY_FILE"; then
  fail battery_output_not_redacted
fi

post_mcp '{"jsonrpc":"2.0","id":"invalid","method":"tools/call","params":{"name":"android_battery_status","arguments":{"unexpected":true}}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 400 ]] || fail invalid_arguments_http_invalid
jq -e '.error.code == -32602 and (.result | not)' "$BODY_FILE" >/dev/null || fail invalid_arguments_contract_invalid

write_overflow_fixture
post_mcp '{"jsonrpc":"2.0","id":"overflow","method":"tools/call","params":{"name":"android_battery_status","arguments":{}}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail overflow_http_invalid
jq -e '
  .result.isError == true
  and .result.structuredContent.error == "android_battery_status_unavailable"
  and .result.structuredContent.reasonCode == "battery_stdout_limit_exceeded"
' "$BODY_FILE" >/dev/null || fail overflow_contract_invalid

for stream in stdout stderr; do
  write_endless_output_fixture "$stream"
  payload="$(jq -cn --arg id "endless-$stream" '{jsonrpc:"2.0",id:$id,method:"tools/call",params:{name:"android_battery_status",arguments:{}}}')"
  post_mcp "$payload" "$SESSION_ID" 2
  [[ "$MCP_STATUS" == 200 ]] || fail "endless_${stream}_http_invalid"
  jq -e --arg reason "battery_${stream}_limit_exceeded" '
    .result.isError == true
    and .result.structuredContent.reasonCode == $reason
  ' "$BODY_FILE" >/dev/null || fail "endless_${stream}_contract_invalid"
  direct_pid="$(read_fixture_pid "$BATTERY_DIRECT_PID_FILE" "endless_${stream}_direct_pid")"
  assert_process_gone "$direct_pid" "endless_${stream}_direct_process"
done

for stream in stdout stderr; do
  write_pipe_holding_descendant_fixture "$stream"
  payload="$(jq -cn --arg id "pipe-holder-$stream" '{jsonrpc:"2.0",id:$id,method:"tools/call",params:{name:"android_battery_status",arguments:{}}}')"
  post_mcp "$payload" "$SESSION_ID" 7
  [[ "$MCP_STATUS" == 200 ]] || fail "pipe_holder_${stream}_http_invalid"
  jq -e '
    .result.isError == true
    and .result.structuredContent.reasonCode == "battery_api_timeout"
  ' "$BODY_FILE" >/dev/null || fail "pipe_holder_${stream}_contract_invalid"
  direct_pid="$(read_fixture_pid "$BATTERY_DIRECT_PID_FILE" "pipe_holder_${stream}_direct_pid")"
  descendant_pid="$(read_fixture_pid "$BATTERY_DESCENDANT_PID_FILE" "pipe_holder_${stream}_descendant_pid")"
  assert_process_gone "$direct_pid" "pipe_holder_${stream}_direct_process"
  assert_process_gone "$descendant_pid" "pipe_holder_${stream}_descendant_process"
done

write_cancellation_fixture
cancel_mcp_request
direct_pid="$(read_fixture_pid "$BATTERY_DIRECT_PID_FILE" cancellation_direct_pid)"
descendant_pid="$(read_fixture_pid "$BATTERY_DESCENDANT_PID_FILE" cancellation_descendant_pid)"
assert_process_gone "$direct_pid" cancellation_direct_process
assert_process_gone "$descendant_pid" cancellation_descendant_process

write_failure_fixture
post_mcp '{"jsonrpc":"2.0","id":"failed","method":"tools/call","params":{"name":"android_battery_status","arguments":{}}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail api_failure_http_invalid
jq -e '
  .result.isError == true
  and .result.structuredContent.reasonCode == "battery_api_failed"
' "$BODY_FILE" >/dev/null || fail api_failure_contract_invalid
stop_server

log 'validating disabled battery posture'
write_success_fixture
start_server false
initialize_session
post_mcp '{"jsonrpc":"2.0","id":"tools-disabled","method":"tools/list"}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail disabled_tools_status_invalid
jq -e '
  [.result.tools[].name] == [
    "runtime_status",
    "platform_info",
    "android_status",
    "project_service_status",
    "list_directory",
    "read_file",
    "search_text",
    "write_file"
  ]
' "$BODY_FILE" >/dev/null || fail disabled_tool_discovery_invalid

post_mcp '{"jsonrpc":"2.0","id":"runtime-disabled","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}' "$SESSION_ID"
jq -e '
  .result.structuredContent.androidPlatformTools == false
  and .result.structuredContent.androidBatteryStatusCompiled == true
  and .result.structuredContent.androidBatteryStatusEnabled == false
  and .result.structuredContent.androidDeviceControl == false
' "$BODY_FILE" >/dev/null || fail disabled_runtime_status_invalid

post_mcp '{"jsonrpc":"2.0","id":"battery-disabled","method":"tools/call","params":{"name":"android_battery_status","arguments":{}}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail disabled_call_http_invalid
jq -e '
  .result.isError == true
  and .result.structuredContent.reasonCode == "battery_runtime_disabled"
' "$BODY_FILE" >/dev/null || fail disabled_call_contract_invalid
stop_server

COMPLETED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
REPORT_NEXT="$WORK_ROOT/battery-emulated-evidence.json"
jq -n \
  --arg gate_version "$GATE_VERSION" \
  --arg started_at "$STARTED_AT" \
  --arg completed_at "$COMPLETED_AT" \
  --arg commit "$EXPECTED_COMMIT" \
  --arg version "$EXPECTED_VERSION" \
  --arg ci_run_id "$CI_RUN_ID" \
  --arg security_run_id "$SECURITY_RUN_ID" \
  --arg android_run_id "$ANDROID_RUN_ID" \
  --arg artifact_sha "$ARTIFACT_SHA" \
  --argjson artifact_bytes "$ARTIFACT_BYTES" \
  --arg architecture "$(uname -m)" \
  --arg image "$EXPECTED_IMAGE" \
  --arg image_digest "$IMAGE_DIGEST" \
  --argjson requests "$REQUEST_COUNT" '
  {
    schemaVersion: 2,
    gateVersion: $gate_version,
    status: "pass",
    failureCode: null,
    releaseQualificationEligible: false,
    startedAt: $started_at,
    completedAt: $completed_at,
    candidate: {
      commit: $commit,
      version: $version,
      ciRunId: $ci_run_id,
      securityRunId: $security_run_id,
      androidRunId: $android_run_id,
      artifact: {sha256: $artifact_sha, bytes: $artifact_bytes}
    },
    environment: {
      architecture: $architecture,
      executionMode: "official-termux-docker-native-arm64",
      image: $image,
      imageDigest: $image_digest,
      androidLinker: true
    },
    validation: {
      status: "pass",
      requests: $requests,
      exactArtifact: true,
      compileGate: true,
      runtimeDefaultDisabled: true,
      disabledDiscovery: true,
      fixedProgram: true,
      fixedWorkingDirectory: true,
      noArguments: true,
      inheritedEnvironmentCleared: true,
      normalizedAllowlist: true,
      sensitiveFieldsRedacted: true,
      boundedOutput: true,
      immediateOverflowTermination: true,
      processGroupIsolation: true,
      pipeHoldingDescendantCleanup: true,
      callerCancellationCleanup: true,
      boundedSupervisorCleanup: true,
      stableErrors: true,
      androidDeviceControlDisabled: true,
      commandExecutionDisabled: true,
      highImpactToolsDisabled: true
    }
  }' >"$REPORT_NEXT" || fail report_generation_failed
chmod 600 "$REPORT_NEXT" || fail report_mode_failed

jq -e '
  .schemaVersion == 2 and .gateVersion == "2" and .status == "pass"
  and .failureCode == null and .releaseQualificationEligible == false
  and .environment.executionMode == "official-termux-docker-native-arm64"
  and .environment.androidLinker == true
  and .validation.status == "pass"
  and ([.validation[] | select(type == "boolean")] | all)
' "$REPORT_NEXT" >/dev/null || fail generated_report_invalid
if grep -Eq '/data/|Bearer[[:space:]]|MCP__|private-identifier|vendor-private|[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}' "$REPORT_NEXT"; then
  fail report_contains_sensitive_value
fi

install -m 600 "$REPORT_NEXT" "$OUTPUT_REPORT" || fail report_publication_failed
REPORT_SHA="$(sha256sum "$OUTPUT_REPORT" | awk '{print $1}')"
log "report_sha256=$REPORT_SHA"
log "report=$OUTPUT_REPORT"
printf 'TERMUX_MCP_BATTERY_EMULATED_RESULT=PASS\n'
