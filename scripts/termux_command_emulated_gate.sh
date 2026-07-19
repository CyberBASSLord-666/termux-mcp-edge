#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
umask 077

GATE_VERSION=2
EXPECTED_IMAGE='termux/termux-docker:aarch64'
DEFAULT_PORT=18769
EXPECTED_REQUEST_COUNT=29

ARTIFACT_DIR=''
DEFAULT_ARTIFACT_DIR=''
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
REQUEST_COUNT=0
MCP_STATUS=''

log() { printf '[termux-command-emulated] %s\n' "$*"; }
fail() {
  printf 'TERMUX_MCP_COMMAND_EMULATED_RESULT=FAIL reason=%s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage: termux_command_emulated_gate.sh \
  --artifact-dir DIR \
  --default-dir DIR \
  --expected-commit SHA \
  --expected-version VERSION \
  --ci-run-id ID \
  --security-run-id ID \
  --android-run-id ID \
  --output REPORT.json \
  [--port PORT]

Run an exact command-execution artifact natively in the pinned official ARM64
Termux environment. This validates the compile/runtime truth table, fixed
profile registry, closed schema, cleared environment, null stdin, safe-rooted
descriptor-pinned working directory, bounded structured output, wrong-name
pre-listener rejection, loaded-inode replacement isolation, stable errors, and
audit counters.
It does not run a long observation window or enable arbitrary commands.
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
  [[ -z "$WORK_ROOT" ]] || rm -rf -- "$WORK_ROOT" >/dev/null 2>&1 || status=1
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

while (($#)); do
  case "$1" in
    --artifact-dir) (($# >= 2)) || fail missing_artifact_dir; ARTIFACT_DIR="$2"; shift 2 ;;
    --default-dir) (($# >= 2)) || fail missing_default_dir; DEFAULT_ARTIFACT_DIR="$2"; shift 2 ;;
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
[[ "$ARTIFACT_DIR" == /* && "$DEFAULT_ARTIFACT_DIR" == /* && "$OUTPUT_REPORT" == /* ]] || fail absolute_paths_required

[[ "${TERMUX_MCP_EMULATED_ENVIRONMENT:-}" == official-termux-docker-native-arm64 ]] || fail environment_attestation_missing
IMAGE_DIGEST="${TERMUX_MCP_TERMUX_IMAGE_DIGEST:-}"
[[ "$IMAGE_DIGEST" =~ ^sha256:[0-9a-f]{64}$ ]] || fail image_digest_invalid
[[ "$(uname -m)" == aarch64 || "$(uname -m)" == arm64 ]] || fail architecture_not_arm64
[[ "${PREFIX:-}" == /data/data/com.termux/files/usr ]] || fail termux_prefix_invalid
[[ "${HOME:-}" == /data/data/com.termux/files/home ]] || fail termux_home_invalid
[[ -x /system/bin/linker64 ]] || fail android_linker_missing

for command in awk cat chmod curl date dd dirname file find grep install jq kill mkdir mktemp mv realpath rm seq sha256sum sleep stat timeout uname wc; do
  command -v "$command" >/dev/null 2>&1 || fail "required_command_missing_$command"
done

ARTIFACT="$ARTIFACT_DIR/termux-mcp-server"
MANIFEST="$ARTIFACT_DIR/artifact-manifest.json"
CHECKSUMS="$ARTIFACT_DIR/SHA256SUMS"
for path in "$ARTIFACT" "$MANIFEST" "$CHECKSUMS"; do
  [[ -f "$path" && ! -L "$path" ]] || fail artifact_bundle_member_invalid
done
[[ -x "$ARTIFACT" ]] || fail artifact_binary_not_executable
[[ "$(find "$ARTIFACT_DIR" -mindepth 1 -maxdepth 1 | wc -l)" == 3 ]] || fail artifact_bundle_member_count_invalid
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
    and .artifactName == "termux-mcp-server-aarch64-linux-android-command-execution"
    and .posture == "command-execution"
    and .features == ["command-execution"]
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

DEFAULT_ARTIFACT="$DEFAULT_ARTIFACT_DIR/termux-mcp-server"
DEFAULT_MANIFEST="$DEFAULT_ARTIFACT_DIR/artifact-manifest.json"
DEFAULT_CHECKSUMS="$DEFAULT_ARTIFACT_DIR/SHA256SUMS"
for path in "$DEFAULT_ARTIFACT" "$DEFAULT_MANIFEST" "$DEFAULT_CHECKSUMS"; do
  [[ -f "$path" && ! -L "$path" ]] || fail default_artifact_bundle_member_invalid
done
[[ -x "$DEFAULT_ARTIFACT" ]] || fail default_artifact_binary_not_executable
[[ "$(find "$DEFAULT_ARTIFACT_DIR" -mindepth 1 -maxdepth 1 | wc -l)" == 3 ]] || fail default_artifact_bundle_member_count_invalid
(cd "$DEFAULT_ARTIFACT_DIR" && sha256sum -c SHA256SUMS >/dev/null) || fail default_artifact_checksum_invalid
jq -e \
  --arg commit "$EXPECTED_COMMIT" \
  --arg version "$EXPECTED_VERSION" \
  --arg run_id "$ANDROID_RUN_ID" '
    (keys == ["artifactName","bytes","commit","createdAt","elf","features","fileName","posture","repository","schemaVersion","sha256","target","version","workflowRunId"])
    and .schemaVersion == 1
    and .repository == "CyberBASSLord-666/termux-mcp-edge"
    and .commit == $commit
    and .workflowRunId == $run_id
    and .artifactName == "termux-mcp-server-aarch64-linux-android-default"
    and .posture == "default"
    and .features == []
    and .target == "aarch64-linux-android"
    and .fileName == "termux-mcp-server"
    and .version == $version
    and .elf == "aarch64-android-elf"
    and (.sha256 | test("^[0-9a-f]{64}$"))
    and (.bytes >= 1 and .bytes <= 67108864)
  ' "$DEFAULT_MANIFEST" >/dev/null || fail default_artifact_manifest_invalid
DEFAULT_ARTIFACT_SHA="$(jq -r .sha256 "$DEFAULT_MANIFEST")"
DEFAULT_ARTIFACT_BYTES="$(jq -r .bytes "$DEFAULT_MANIFEST")"
[[ "$(sha256sum "$DEFAULT_ARTIFACT" | awk '{print $1}')" == "$DEFAULT_ARTIFACT_SHA" ]] || fail default_artifact_digest_mismatch
[[ "$(stat -c %s "$DEFAULT_ARTIFACT")" == "$DEFAULT_ARTIFACT_BYTES" ]] || fail default_artifact_size_mismatch
default_identity="$(file -b "$DEFAULT_ARTIFACT")" || fail default_artifact_identity_failed
[[ "$default_identity" == *ELF* && "$default_identity" == *"ARM aarch64"* ]] || fail default_artifact_architecture_mismatch
[[ "$default_identity" == *Android* || "$default_identity" == *"/system/bin/linker64"* ]] || fail default_artifact_android_identity_missing
[[ "$(timeout -k 2 5 "$DEFAULT_ARTIFACT" --version)" == "termux-mcp-server $EXPECTED_VERSION" ]] || fail default_artifact_version_mismatch

OUTPUT_PARENT="$(dirname "$OUTPUT_REPORT")"
[[ -d "$OUTPUT_PARENT" && ! -L "$OUTPUT_PARENT" ]] || fail output_parent_invalid
[[ "$(realpath -e "$OUTPUT_PARENT")" == "$OUTPUT_PARENT" ]] || fail output_parent_not_canonical
[[ "$(stat -c %a "$OUTPUT_PARENT")" == 700 ]] || fail output_parent_not_private
[[ ! -e "$OUTPUT_REPORT" && ! -L "$OUTPUT_REPORT" ]] || fail output_already_exists

WORK_ROOT="$(mktemp -d "$HOME/.termux-mcp-command-gate.XXXXXX")" || fail work_root_create_failed
chmod 700 "$WORK_ROOT"
SAFE_ROOT="$WORK_ROOT/safe-root"
SERVER_LOG="$WORK_ROOT/server.log"
BODY_FILE="$WORK_ROOT/body.json"
HEADER_FILE="$WORK_ROOT/headers.txt"
REQUEST_FILE="$WORK_ROOT/request.json"
mkdir -m 700 "$SAFE_ROOT"

MCP_TOKEN="$(dd if=/dev/urandom bs=32 count=1 status=none | sha256sum | awk '{print $1}')"
[[ "$MCP_TOKEN" =~ ^[0-9a-f]{64}$ ]] || fail token_generation_failed

log 'validating compile-time default deny posture'
set +e
MCP__AUTH__STATIC_TOKEN="$MCP_TOKEN" \
MCP__COMMAND__ENABLED=true \
MCP__SERVER__HOST=127.0.0.1 \
MCP__SERVER__PORT="$PORT" \
MCP__FILE__SAFE_ROOTS="$SAFE_ROOT" \
  timeout -k 2 5 "$DEFAULT_ARTIFACT" >"$WORK_ROOT/default-command-rejection.log" 2>&1
default_command_rc=$?
set -e
((default_command_rc != 0 && default_command_rc != 124 && default_command_rc != 137)) || fail default_artifact_command_gate_not_rejected
grep -F 'MCP__COMMAND__ENABLED requires a binary built with the command-execution feature' \
  "$WORK_ROOT/default-command-rejection.log" >/dev/null || fail default_artifact_command_gate_error_invalid

curl_local() {
  command curl --disable --proto '=http' --noproxy '*' --connect-timeout 2 --max-time 10 "$@"
}

start_server() {
  local enabled="$1"
  local candidate="${2:-$ARTIFACT}"
  local launch_directory="${3:-}"
  (
    if [[ -n "$launch_directory" ]]; then
      cd "$launch_directory"
    fi
    MCP__AUTH__STATIC_TOKEN="$MCP_TOKEN" \
    MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=false \
    MCP__COMMAND__ENABLED="$enabled" \
    MCP__SERVER__HOST=127.0.0.1 \
    MCP__SERVER__PORT="$PORT" \
    MCP__TRANSPORT__ALLOWED_HOSTS="localhost:$PORT,127.0.0.1:$PORT" \
    MCP__TRANSPORT__ALLOWED_ORIGINS="http://localhost:$PORT,http://127.0.0.1:$PORT" \
    MCP__TRANSPORT__ALLOW_MISSING_ORIGIN=false \
    MCP__TRANSPORT__SSE_ENABLED=false \
    MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS=4 \
    MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS=30 \
    MCP__TRANSPORT__MAX_BODY_BYTES=32768 \
    MCP__FILE__SAFE_ROOTS="$SAFE_ROOT" \
    MCP__FILE__WRITE_MUTATION_ENABLED=false \
    RUST_LOG=termux_mcp_server=info \
      exec "$candidate"
  ) >"$SERVER_LOG" 2>&1 &
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

initialize_session() {
  printf '%s' '{"jsonrpc":"2.0","id":"initialize","method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"termux-command-emulated-gate","version":"1"}}}' >"$REQUEST_FILE"
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

log 'validating enabled fixed-profile command posture'
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
    "create_directory",
    "copy_file",
    "find_paths",
    "hash_file",
    "list_directory",
    "path_metadata",
    "read_binary_file",
    "read_binary_range",
    "read_file",
    "read_text_range",
    "search_text",
    "write_file",
    "run_command_profile"
  ]
  and (.result.tools[] | select(.name == "run_command_profile") | .inputSchema)
      == {
        type:"object",
        properties:{profile:{type:"string",enum:["server_version","server_help","execution_boundary"],description:"Reviewed project-owned diagnostic profile identifier."}},
        required:["profile"],
        additionalProperties:false
      }
  and ((.result.tools[] | select(.name == "write_file") | .inputSchema.properties.dry_run.const) == true)
' "$BODY_FILE" >/dev/null || fail enabled_tool_discovery_invalid

post_mcp '{"jsonrpc":"2.0","id":"runtime","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail runtime_status_http_invalid
jq -e '
  .result.structuredContent.commandExecutionCompiled == true
  and .result.structuredContent.commandExecution == true
  and .result.structuredContent.commandExecutionMode == "fixed_read_only_server_diagnostics"
  and .result.structuredContent.arbitraryCommandExecution == false
  and .result.structuredContent.androidDeviceControl == false
  and .result.structuredContent.fileWriteMutationEnabled == false
  and .result.structuredContent.fileWriteGrantRequired == false
  and .result.structuredContent.fileWriteMode == "dry_run_only_mutation_disabled"
  and .result.structuredContent.highImpactTools == false
' "$BODY_FILE" >/dev/null || fail runtime_status_gate_invalid

post_mcp "$(jq -cn --arg path "$SAFE_ROOT/command-write-disabled.txt" '{jsonrpc:"2.0",id:"write-disabled",method:"tools/call",params:{name:"write_file",arguments:{path:$path,content:"inert",dry_run:false}}}')" "$SESSION_ID"
[[ "$MCP_STATUS" == 403 ]] || fail write_file_disabled_http_invalid
jq -e '.error.code == -32003 and .error.data.reason == "write_file_mutation_disabled"' "$BODY_FILE" >/dev/null || fail write_file_disabled_contract_invalid
[[ ! -e "$SAFE_ROOT/command-write-disabled.txt" && ! -L "$SAFE_ROOT/command-write-disabled.txt" ]] || fail write_file_disabled_mutated

post_mcp '{"jsonrpc":"2.0","id":"android","method":"tools/call","params":{"name":"android_status","arguments":{}}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail android_status_http_invalid
jq -e '
  .result.structuredContent.command_execution_enabled == true
  and .result.structuredContent.android_control_enabled == false
  and .result.structuredContent.shell_fallback_enabled == false
  and .result.structuredContent.high_impact_controls_enabled == false
' "$BODY_FILE" >/dev/null || fail android_status_gate_invalid

post_mcp '{"jsonrpc":"2.0","id":"version","method":"tools/call","params":{"name":"run_command_profile","arguments":{"profile":"server_version"}}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail version_profile_http_invalid
jq -e --arg version "$EXPECTED_VERSION" '
  .result.isError == false
  and (.result.structuredContent | keys) == ["durationMilliseconds","exitCode","profile","stderr","stderrBytes","stdout","stdoutBytes"]
  and .result.structuredContent.profile == "server_version"
  and .result.structuredContent.exitCode == 0
  and .result.structuredContent.stdout == ("termux-mcp-server " + $version + "\n")
  and .result.structuredContent.stderr == ""
  and .result.structuredContent.stdoutBytes == (.result.structuredContent.stdout | utf8bytelength)
  and .result.structuredContent.stderrBytes == 0
  and (.result.structuredContent.durationMilliseconds >= 0 and .result.structuredContent.durationMilliseconds <= 5000)
' "$BODY_FILE" >/dev/null || fail version_profile_contract_invalid

post_mcp '{"jsonrpc":"2.0","id":"boundary","method":"tools/call","params":{"name":"run_command_profile","arguments":{"profile":"execution_boundary"}}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail boundary_profile_http_invalid
jq -e '
  .result.isError == false
  and .result.structuredContent.profile == "execution_boundary"
  and .result.structuredContent.exitCode == 0
  and .result.structuredContent.stdout == "termux-mcp-command-boundary ok\n"
  and .result.structuredContent.stderr == ""
  and .result.structuredContent.stdoutBytes == 31
  and .result.structuredContent.stderrBytes == 0
' "$BODY_FILE" >/dev/null || fail boundary_profile_contract_invalid

post_mcp '{"jsonrpc":"2.0","id":"help","method":"tools/call","params":{"name":"run_command_profile","arguments":{"profile":"server_help"}}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail help_profile_http_invalid
jq -e '
  .result.isError == false
  and .result.structuredContent.profile == "server_help"
  and .result.structuredContent.exitCode == 0
  and (.result.structuredContent.stdout | startswith("Termux MCP Edge\n\nUsage:\n"))
  and .result.structuredContent.stderr == ""
  and .result.structuredContent.stdoutBytes == (.result.structuredContent.stdout | utf8bytelength)
  and (.result.structuredContent.stdoutBytes >= 1 and .result.structuredContent.stdoutBytes <= 16384)
' "$BODY_FILE" >/dev/null || fail help_profile_contract_invalid

override_cases=(
  '{"profile":"server_version","command":"sh -c id"}'
  '{"profile":"server_version","program":"/bin/sh"}'
  '{"profile":"server_version","argv":["--help"]}'
  '{"profile":"server_version","workingDirectory":"/"}'
  '{"profile":"server_version","environment":{"TOKEN":"secret"}}'
  '{"profile":"server_version","stdin":"private"}'
  '{"profile":"server_version","timeout":999}'
  '{"profile":"server_version","stdoutLimit":999999}'
  '{"profile":"server_version","stderrLimit":999999}'
)
for index in "${!override_cases[@]}"; do
  payload="$(jq -cn --arg id "override-$index" --argjson arguments "${override_cases[$index]}" \
    '{jsonrpc:"2.0",id:$id,method:"tools/call",params:{name:"run_command_profile",arguments:$arguments}}')"
  post_mcp "$payload" "$SESSION_ID"
  [[ "$MCP_STATUS" == 400 ]] || fail "override_${index}_http_invalid"
  jq -e '.error.code == -32602 and (.result | not)' "$BODY_FILE" >/dev/null || fail "override_${index}_contract_invalid"
done

post_mcp '{"jsonrpc":"2.0","id":"unknown","method":"tools/call","params":{"name":"run_command_profile","arguments":{"profile":"sh -c id"}}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 400 ]] || fail unknown_profile_http_invalid
jq -e '.error.code == -32602 and (.result | not)' "$BODY_FILE" >/dev/null || fail unknown_profile_contract_invalid

post_mcp '{"jsonrpc":"2.0","id":"missing","method":"tools/call","params":{"name":"run_command_profile"}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 400 ]] || fail missing_arguments_http_invalid
jq -e '.error.code == -32602 and (.result | not)' "$BODY_FILE" >/dev/null || fail missing_arguments_contract_invalid

post_mcp '{"jsonrpc":"2.0","id":"audit","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail audit_status_http_invalid
jq -e '
  .result.structuredContent.auditCounters.by_tool.run_command_profile.allowed == 3
  and .result.structuredContent.auditCounters.by_tool.run_command_profile.denied == 11
  and .result.structuredContent.auditCounters.by_reason_code.command_profile_execution_allowed.allowed == 3
  and .result.structuredContent.auditCounters.by_reason_code.command_profile_invalid_arguments.denied == 9
  and .result.structuredContent.auditCounters.by_reason_code.command_profile_not_allowlisted.denied == 1
  and .result.structuredContent.auditCounters.by_reason_code.command_profile_missing_arguments.denied == 1
' "$BODY_FILE" >/dev/null || fail audit_counter_contract_invalid
stop_server

log 'validating loaded executable and working-directory inode replacement isolation'
PINNED_DIR="$WORK_ROOT/pinned-server"
PINNED_ARTIFACT="$PINNED_DIR/termux-mcp-server"
PINNED_MARKER="$WORK_ROOT/replacement-marker"
PINNED_SAFE_ROOT="$WORK_ROOT/pinned-safe-root"
PINNED_SAFE_ROOT_MARKER="$SAFE_ROOT/original-directory-marker"
SAFE_ROOT_REPLACEMENT_CONTENT='replacement-path-must-not-be-used'
mkdir -m 700 "$PINNED_DIR"
printf '%s' 'original-directory' >"$PINNED_SAFE_ROOT_MARKER"
install -m 700 "$ARTIFACT" "$PINNED_ARTIFACT"
start_server true "$PINNED_ARTIFACT" /
initialize_session
mv "$SAFE_ROOT" "$PINNED_SAFE_ROOT"
printf '%s' "$SAFE_ROOT_REPLACEMENT_CONTENT" >"$SAFE_ROOT"
rm -f -- "$PINNED_ARTIFACT"
{
  printf '#!/data/data/com.termux/files/usr/bin/bash\n'
  printf ': > %q\n' "$PINNED_MARKER"
  printf "printf 'replacement-executed\\n'\n"
} >"$PINNED_ARTIFACT"
chmod 700 "$PINNED_ARTIFACT"
post_mcp '{"jsonrpc":"2.0","id":"pinned-boundary","method":"tools/call","params":{"name":"run_command_profile","arguments":{"profile":"execution_boundary"}}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail pinned_boundary_http_invalid
jq -e '
  .result.isError == false
  and .result.structuredContent.profile == "execution_boundary"
  and .result.structuredContent.stdout == "termux-mcp-command-boundary ok\n"
  and .result.structuredContent.stderr == ""
' "$BODY_FILE" >/dev/null || fail pinned_boundary_contract_invalid
[[ ! -e "$PINNED_MARKER" && ! -L "$PINNED_MARKER" ]] || fail executable_path_replacement_ran
[[ "$(cat "$PINNED_SAFE_ROOT/original-directory-marker")" == original-directory ]] || fail working_directory_original_inode_lost
[[ "$(cat "$SAFE_ROOT")" == "$SAFE_ROOT_REPLACEMENT_CONTENT" ]] || fail working_directory_path_replacement_used
stop_server
rm -f -- "$SAFE_ROOT"
mv "$PINNED_SAFE_ROOT" "$SAFE_ROOT"

log 'validating wrong executable name is rejected before listener startup'
RENAMED_ARTIFACT="$WORK_ROOT/renamed-command-server"
install -m 700 "$ARTIFACT" "$RENAMED_ARTIFACT"
set +e
MCP__AUTH__STATIC_TOKEN="$MCP_TOKEN" \
MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=false \
MCP__COMMAND__ENABLED=true \
MCP__SERVER__HOST=127.0.0.1 \
MCP__SERVER__PORT="$PORT" \
MCP__TRANSPORT__ALLOWED_HOSTS="localhost:$PORT,127.0.0.1:$PORT" \
MCP__TRANSPORT__ALLOWED_ORIGINS="http://localhost:$PORT,http://127.0.0.1:$PORT" \
MCP__TRANSPORT__ALLOW_MISSING_ORIGIN=false \
MCP__TRANSPORT__SSE_ENABLED=false \
MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS=4 \
MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS=30 \
MCP__TRANSPORT__MAX_BODY_BYTES=32768 \
MCP__FILE__SAFE_ROOTS="$SAFE_ROOT" \
MCP__FILE__WRITE_MUTATION_ENABLED=false \
RUST_LOG=termux_mcp_server=info \
  timeout -k 2 5 "$RENAMED_ARTIFACT" >"$SERVER_LOG" 2>&1
wrong_name_rc=$?
set -e
((wrong_name_rc != 0 && wrong_name_rc != 124 && wrong_name_rc != 137)) \
  || fail wrong_name_startup_exit_invalid
grep -F 'requested MCP optional client is unavailable: command_execution' "$SERVER_LOG" >/dev/null \
  || fail wrong_name_startup_error_invalid
if grep -F "$MCP_TOKEN" "$SERVER_LOG" >/dev/null; then
  fail wrong_name_startup_error_leaked_token
fi
if grep -F "$RENAMED_ARTIFACT" "$SERVER_LOG" >/dev/null || grep -F "$SAFE_ROOT" "$SERVER_LOG" >/dev/null; then
  fail wrong_name_startup_error_leaked_path
fi
if grep -F 'Listening on http://' "$SERVER_LOG" >/dev/null; then
  fail wrong_name_listener_started
fi
if curl_local --silent --max-time 1 "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then
  fail wrong_name_listener_reachable
fi

log 'validating disabled fixed-profile command posture'
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
    "create_directory",
    "copy_file",
    "find_paths",
    "hash_file",
    "list_directory",
    "path_metadata",
    "read_binary_file",
    "read_binary_range",
    "read_file",
    "read_text_range",
    "search_text",
    "write_file"
  ]
  and ((.result.tools[] | select(.name == "write_file") | .inputSchema.properties.dry_run.const) == true)
' "$BODY_FILE" >/dev/null || fail disabled_tool_discovery_invalid

post_mcp '{"jsonrpc":"2.0","id":"runtime-disabled","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}' "$SESSION_ID"
jq -e '
  .result.structuredContent.commandExecutionCompiled == true
  and .result.structuredContent.commandExecution == false
  and .result.structuredContent.commandExecutionMode == "disabled"
  and .result.structuredContent.arbitraryCommandExecution == false
  and .result.structuredContent.fileWriteMutationEnabled == false
  and .result.structuredContent.fileWriteGrantRequired == false
' "$BODY_FILE" >/dev/null || fail disabled_runtime_status_invalid

post_mcp '{"jsonrpc":"2.0","id":"command-disabled","method":"tools/call","params":{"name":"run_command_profile","arguments":{"profile":"server_version"}}}' "$SESSION_ID"
[[ "$MCP_STATUS" == 200 ]] || fail disabled_call_http_invalid
jq -e '
  .result.isError == true
  and .result.structuredContent.reasonCode == "command_runtime_disabled"
' "$BODY_FILE" >/dev/null || fail disabled_call_contract_invalid
stop_server

((REQUEST_COUNT == EXPECTED_REQUEST_COUNT)) || fail request_count_invalid

COMPLETED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
REPORT_NEXT="$WORK_ROOT/command-emulated-evidence.json"
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
  --arg default_artifact_sha "$DEFAULT_ARTIFACT_SHA" \
  --argjson default_artifact_bytes "$DEFAULT_ARTIFACT_BYTES" \
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
      artifact: {sha256: $artifact_sha, bytes: $artifact_bytes},
      defaultArtifact: {sha256: $default_artifact_sha, bytes: $default_artifact_bytes}
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
      fixedCurrentExecutable: true,
      wrongExecutableNameFailsClosed: true,
      wrongExecutableNameRejectedBeforeListener: true,
      runningInodePinned: true,
      workingDirectoryDescriptorPinned: true,
      fixedArgvProfiles: true,
      closedInputSchema: true,
      overrideFieldsRejected: true,
      unknownProfileRejected: true,
      fixedWorkingDirectory: true,
      inheritedEnvironmentCleared: true,
      nullStdin: true,
      boundedOutput: true,
      utf8Output: true,
      versionProfile: true,
      helpProfile: true,
      boundaryProfile: true,
      auditCounters: true,
      stableErrors: true,
      arbitraryCommandExecutionDisabled: true,
      androidDeviceControlDisabled: true,
      highImpactToolsDisabled: true,
      longObservationRequired: false
    }
  }' >"$REPORT_NEXT" || fail report_generation_failed
chmod 600 "$REPORT_NEXT" || fail report_mode_failed

jq -e --argjson expected_requests "$EXPECTED_REQUEST_COUNT" '
  .schemaVersion == 2 and .gateVersion == "2" and .status == "pass"
  and .failureCode == null and .releaseQualificationEligible == false
  and .environment.executionMode == "official-termux-docker-native-arm64"
  and .environment.androidLinker == true
  and .validation.status == "pass"
  and .validation.requests == $expected_requests
  and .validation.longObservationRequired == false
  and ([
    .validation
    | to_entries[]
    | select(.key != "longObservationRequired" and (.value | type) == "boolean")
    | .value
  ] | all)
' "$REPORT_NEXT" >/dev/null || fail generated_report_invalid
if grep -Eq '/data/|Bearer[[:space:]]|MCP__|secret|private|[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}' "$REPORT_NEXT"; then
  fail report_contains_sensitive_value
fi

install -m 600 "$REPORT_NEXT" "$OUTPUT_REPORT" || fail report_publication_failed
REPORT_SHA="$(sha256sum "$OUTPUT_REPORT" | awk '{print $1}')"
log "report_sha256=$REPORT_SHA"
log "report=$OUTPUT_REPORT"
printf 'TERMUX_MCP_COMMAND_EMULATED_RESULT=PASS\n'
