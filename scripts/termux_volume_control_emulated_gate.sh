#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
umask 077

GATE_VERSION=1
EXPECTED_IMAGE='termux/termux-docker:aarch64'
DEFAULT_PORT=18770

ARTIFACT_DIR=''
INCOMPATIBLE_ARTIFACT_DIR=''
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
BACKGROUND_CURL_PID=''
SESSION_ID=''
VOLUME_PROGRAM=''
VOLUME_PROGRAM_CREATED=false
REQUEST_COUNT=0
MCP_STATUS=''

log() { printf '[termux-volume-control-emulated] %s\n' "$*"; }
fail() {
  printf 'TERMUX_MCP_VOLUME_CONTROL_EMULATED_RESULT=FAIL reason=%s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage: termux_volume_control_emulated_gate.sh \
  --artifact-dir DIR \
  --incompatible-dir DIR \
  --expected-commit SHA \
  --expected-version VERSION \
  --ci-run-id ID \
  --security-run-id ID \
  --android-run-id ID \
  --output REPORT.json \
  [--port PORT]

Run an exact android-volume-control artifact natively in the pinned official
ARM64 Termux environment. A fixed-path fixture proves the compile/runtime/auth
truth table, preview-first contract, exact single-use grants, fixed process
boundary, fresh bounds, non-queueing concurrency, verification, restoration,
and cancellation-independent recovery without a long observation window.
EOF
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM HUP
  if [[ -n "$BACKGROUND_CURL_PID" ]] && kill -0 "$BACKGROUND_CURL_PID" >/dev/null 2>&1; then
    kill "$BACKGROUND_CURL_PID" >/dev/null 2>&1 || true
    wait "$BACKGROUND_CURL_PID" 2>/dev/null || true
  fi
  BACKGROUND_CURL_PID=''
  if [[ -n "$SERVER_PID" ]] && kill -0 "$SERVER_PID" >/dev/null 2>&1; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  SERVER_PID=''
  unset MCP_TOKEN SESSION_ID 2>/dev/null || true
  if [[ "$VOLUME_PROGRAM_CREATED" == true && "$VOLUME_PROGRAM" == /data/data/com.termux/files/usr/bin/termux-volume ]]; then
    rm -f -- "$VOLUME_PROGRAM" >/dev/null 2>&1 || status=1
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
    --incompatible-dir) (($# >= 2)) || fail missing_incompatible_dir; INCOMPATIBLE_ARTIFACT_DIR="$2"; shift 2 ;;
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
[[ "$ARTIFACT_DIR" == /* && "$INCOMPATIBLE_ARTIFACT_DIR" == /* && "$OUTPUT_REPORT" == /* ]] || fail absolute_paths_required
for artifact_root in "$ARTIFACT_DIR" "$INCOMPATIBLE_ARTIFACT_DIR"; do
  [[ -d "$artifact_root" && ! -L "$artifact_root" ]] || fail artifact_root_invalid
  [[ "$(realpath -e "$artifact_root")" == "$artifact_root" ]] || fail artifact_root_not_canonical
done

[[ "${TERMUX_MCP_EMULATED_ENVIRONMENT:-}" == official-termux-docker-native-arm64 ]] || fail environment_attestation_missing
IMAGE_DIGEST="${TERMUX_MCP_TERMUX_IMAGE_DIGEST:-}"
[[ "$IMAGE_DIGEST" =~ ^sha256:[0-9a-f]{64}$ ]] || fail image_digest_invalid
[[ "$(uname -m)" == aarch64 || "$(uname -m)" == arm64 ]] || fail architecture_not_arm64
[[ "${PREFIX:-}" == /data/data/com.termux/files/usr ]] || fail termux_prefix_invalid
[[ "${HOME:-}" == /data/data/com.termux/files/home ]] || fail termux_home_invalid
[[ -x /system/bin/linker64 ]] || fail android_linker_missing

for command in awk cat chmod curl date dd dirname env file find grep install jq kill mkdir mktemp readlink realpath rm seq sha256sum sleep stat timeout uname wc; do
  command -v "$command" >/dev/null 2>&1 || fail "required_command_missing_$command"
done

validate_bundle() {
  local root="$1" artifact_name="$2" posture="$3" features="$4"
  local artifact="$root/termux-mcp-server" manifest="$root/artifact-manifest.json" checksums="$root/SHA256SUMS"
  for path in "$artifact" "$manifest" "$checksums"; do
    [[ -f "$path" && ! -L "$path" ]] || fail artifact_bundle_member_invalid
  done
  [[ -x "$artifact" ]] || fail artifact_binary_not_executable
  [[ "$(find "$root" -mindepth 1 -maxdepth 1 | wc -l)" == 3 ]] || fail artifact_bundle_member_count_invalid
  (cd "$root" && sha256sum -c SHA256SUMS >/dev/null) || fail artifact_checksum_invalid
  jq -e \
    --arg commit "$EXPECTED_COMMIT" --arg version "$EXPECTED_VERSION" \
    --arg run_id "$ANDROID_RUN_ID" --arg artifact_name "$artifact_name" \
    --arg posture "$posture" --argjson features "$features" '
      .schemaVersion == 1
      and .repository == "CyberBASSLord-666/termux-mcp-edge"
      and .commit == $commit and .workflowRunId == $run_id
      and .artifactName == $artifact_name and .posture == $posture
      and .features == $features and .target == "aarch64-linux-android"
      and .fileName == "termux-mcp-server" and .version == $version
      and .elf == "aarch64-android-elf"
      and (.sha256 | test("^[0-9a-f]{64}$"))
      and (.bytes >= 1 and .bytes <= 67108864)
    ' "$manifest" >/dev/null || fail artifact_manifest_invalid
  local sha bytes identity
  sha="$(jq -r .sha256 "$manifest")"
  bytes="$(jq -r .bytes "$manifest")"
  [[ "$(sha256sum "$artifact" | awk '{print $1}')" == "$sha" ]] || fail artifact_digest_mismatch
  [[ "$(stat -c %s "$artifact")" == "$bytes" ]] || fail artifact_size_mismatch
  identity="$(file -b "$artifact")" || fail artifact_identity_failed
  [[ "$identity" == *ELF* && "$identity" == *"ARM aarch64"* ]] || fail artifact_architecture_mismatch
  [[ "$identity" == *Android* || "$identity" == *"/system/bin/linker64"* ]] || fail artifact_android_identity_missing
  [[ "$(timeout -k 2 5 "$artifact" --version)" == "termux-mcp-server $EXPECTED_VERSION" ]] || fail artifact_version_mismatch
}

validate_bundle "$ARTIFACT_DIR" \
  termux-mcp-server-aarch64-linux-android-android-volume-control \
  android-volume-control '["android-volume-control"]'
validate_bundle "$INCOMPATIBLE_ARTIFACT_DIR" \
  termux-mcp-server-aarch64-linux-android-android-volume-status \
  android-volume-status '["android-volume-status"]'

ARTIFACT="$ARTIFACT_DIR/termux-mcp-server"
INCOMPATIBLE_ARTIFACT="$INCOMPATIBLE_ARTIFACT_DIR/termux-mcp-server"
ARTIFACT_SHA="$(jq -r .sha256 "$ARTIFACT_DIR/artifact-manifest.json")"
ARTIFACT_BYTES="$(jq -r .bytes "$ARTIFACT_DIR/artifact-manifest.json")"
INCOMPATIBLE_SHA="$(jq -r .sha256 "$INCOMPATIBLE_ARTIFACT_DIR/artifact-manifest.json")"
INCOMPATIBLE_BYTES="$(jq -r .bytes "$INCOMPATIBLE_ARTIFACT_DIR/artifact-manifest.json")"

OUTPUT_PARENT="$(dirname "$OUTPUT_REPORT")"
[[ -d "$OUTPUT_PARENT" && ! -L "$OUTPUT_PARENT" ]] || fail output_parent_invalid
[[ "$(realpath -e "$OUTPUT_PARENT")" == "$OUTPUT_PARENT" ]] || fail output_parent_not_canonical
[[ "$(stat -c %a "$OUTPUT_PARENT")" == 700 ]] || fail output_parent_not_private
[[ ! -e "$OUTPUT_REPORT" && ! -L "$OUTPUT_REPORT" ]] || fail output_already_exists

WORK_ROOT="$(mktemp -d "$HOME/.termux-mcp-volume-control-gate.XXXXXX")" || fail work_root_create_failed
chmod 700 "$WORK_ROOT"
SAFE_ROOT="$WORK_ROOT/safe-root"
SERVER_LOG="$WORK_ROOT/server.log"
BODY_FILE="$WORK_ROOT/body.json"
HEADER_FILE="$WORK_ROOT/headers.txt"
REQUEST_FILE="$WORK_ROOT/request.json"
CURL_CONFIG="$WORK_ROOT/curl-private.conf"
CAPABILITY_CONFIG="$WORK_ROOT/runtime.env"
VOLUME_STATE="$WORK_ROOT/music-level"
VOLUME_MODE="$WORK_ROOT/fixture-mode"
VOLUME_CALLS="$WORK_ROOT/volume-calls"
VOLUME_STARTED="$WORK_ROOT/volume-started"
mkdir -m 700 "$SAFE_ROOT"
printf '5\n' >"$VOLUME_STATE"
printf 'success\n' >"$VOLUME_MODE"
chmod 600 "$VOLUME_STATE" "$VOLUME_MODE"

MCP_TOKEN="$(dd if=/dev/urandom bs=32 count=1 status=none | sha256sum | awk '{print $1}')"
CAPABILITY_KEY="$(dd if=/dev/urandom bs=32 count=1 status=none | sha256sum | awk '{print $1}')"
[[ "$MCP_TOKEN" =~ ^[0-9a-f]{64}$ && "$CAPABILITY_KEY" =~ ^[0-9a-f]{64}$ ]] || fail secret_generation_failed
cat >"$CAPABILITY_CONFIG" <<EOF
MCP__AUTH__STATIC_TOKEN=$MCP_TOKEN
MCP__ANDROID__VOLUME_CONTROL_ENABLED=true
MCP__FILE__WRITE_MUTATION_ENABLED=false
MCP__CAPABILITY__KEY_ID=native-volume-1
MCP__CAPABILITY__HMAC_KEY_HEX=$CAPABILITY_KEY
EOF
chmod 600 "$CAPABILITY_CONFIG"

VOLUME_PROGRAM="$PREFIX/bin/termux-volume"
[[ "$VOLUME_PROGRAM" == /data/data/com.termux/files/usr/bin/termux-volume ]] || fail volume_program_path_invalid
[[ ! -e "$VOLUME_PROGRAM" && ! -L "$VOLUME_PROGRAM" ]] || fail volume_program_already_present
VOLUME_PROGRAM_CREATED=true
FIXTURE_NEXT="$WORK_ROOT/termux-volume.next"
cat >"$FIXTURE_NEXT" <<EOF
#!/data/data/com.termux/files/usr/bin/bash
set -euo pipefail
state='$VOLUME_STATE'
mode_file='$VOLUME_MODE'
calls='$VOLUME_CALLS'
started='$VOLUME_STARTED'
[[ "\$PWD" == / ]]
[[ "\$(/data/data/com.termux/files/usr/bin/readlink /proc/self/fd/0)" == /dev/null ]]
mcp_env_count="\$(/data/data/com.termux/files/usr/bin/env | /data/data/com.termux/files/usr/bin/awk '/^MCP__/{count++} END{print count+0}')"
[[ "\$mcp_env_count" == 0 ]]
if ((\$# == 0)); then
  IFS= read -r music <"\$state"
  printf '[{"stream":"alarm","volume":4,"max_volume":7},{"stream":"call","volume":1,"max_volume":5},{"stream":"music","volume":%s,"max_volume":15},{"stream":"notification","volume":3,"max_volume":7},{"stream":"ring","volume":6,"max_volume":7},{"stream":"system","volume":2,"max_volume":7}]' "\$music"
  exit 0
fi
[[ \$# -eq 2 && "\$1" == music && "\$2" =~ ^[0-9]+$ ]]
printf '%s|%s|%s|%s|%s|%s\n' "\$#" "\$1" "\$2" "\$PWD" "\$(/data/data/com.termux/files/usr/bin/readlink /proc/self/fd/0)" "\$mcp_env_count" >>"\$calls"
IFS= read -r mode <"\$mode_file"
case "\$mode" in
  success) printf '%s\n' "\$2" >"\$state" ;;
  fail_target) [[ "\$2" != 9 ]] || exit 7; printf '%s\n' "\$2" >"\$state" ;;
  fail_all) exit 7 ;;
  wrong_target) if [[ "\$2" == 9 ]]; then printf '8\n' >"\$state"; else printf '%s\n' "\$2" >"\$state"; fi ;;
  delayed_wrong) if [[ "\$2" == 9 ]]; then : >"\$started"; sleep 1; printf '8\n' >"\$state"; else printf '%s\n' "\$2" >"\$state"; fi ;;
  delayed_success) [[ "\$2" != 9 ]] || { : >"\$started"; sleep 1; }; printf '%s\n' "\$2" >"\$state" ;;
  *) exit 8 ;;
esac
EOF
chmod 700 "$FIXTURE_NEXT"
install -m 700 "$FIXTURE_NEXT" "$VOLUME_PROGRAM"
rm -f -- "$FIXTURE_NEXT"

log 'validating compile-time default deny posture'
set +e
MCP__AUTH__STATIC_TOKEN="$MCP_TOKEN" \
MCP__ANDROID__VOLUME_CONTROL_ENABLED=true \
MCP__CAPABILITY__KEY_ID=native-volume-1 \
MCP__CAPABILITY__HMAC_KEY_HEX="$CAPABILITY_KEY" \
MCP__SERVER__HOST=127.0.0.1 MCP__SERVER__PORT="$PORT" \
  timeout -k 2 5 "$INCOMPATIBLE_ARTIFACT" >"$WORK_ROOT/incompatible-rejection.log" 2>&1
incompatible_rc=$?
set -e
((incompatible_rc != 0 && incompatible_rc != 124 && incompatible_rc != 137)) || fail incompatible_artifact_gate_not_rejected
grep -F 'MCP__ANDROID__VOLUME_CONTROL_ENABLED requires a binary built with the android-volume-control feature' \
  "$WORK_ROOT/incompatible-rejection.log" >/dev/null || fail incompatible_artifact_error_invalid

log 'validating privileged startup requirements'
set +e
MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=true \
MCP__ANDROID__VOLUME_CONTROL_ENABLED=true \
MCP__CAPABILITY__KEY_ID=native-volume-1 \
MCP__CAPABILITY__HMAC_KEY_HEX="$CAPABILITY_KEY" \
MCP__SERVER__HOST=127.0.0.1 MCP__SERVER__PORT="$PORT" \
  timeout -k 2 5 "$ARTIFACT" >"$WORK_ROOT/missing-static-token.log" 2>&1
missing_token_rc=$?
MCP__AUTH__STATIC_TOKEN="$MCP_TOKEN" \
MCP__ANDROID__VOLUME_CONTROL_ENABLED=true \
MCP__SERVER__HOST=127.0.0.1 MCP__SERVER__PORT="$PORT" \
  timeout -k 2 5 "$ARTIFACT" >"$WORK_ROOT/missing-capability-key.log" 2>&1
missing_key_rc=$?
set -e
((missing_token_rc != 0 && missing_token_rc != 124 && missing_token_rc != 137)) || fail missing_static_token_not_rejected
((missing_key_rc != 0 && missing_key_rc != 124 && missing_key_rc != 137)) || fail missing_capability_key_not_rejected
grep -F 'MCP__ANDROID__VOLUME_CONTROL_ENABLED requires MCP__AUTH__STATIC_TOKEN' "$WORK_ROOT/missing-static-token.log" >/dev/null || fail missing_static_token_error_invalid
grep -F 'MCP__ANDROID__VOLUME_CONTROL_ENABLED requires MCP__CAPABILITY__KEY_ID and MCP__CAPABILITY__HMAC_KEY_HEX' "$WORK_ROOT/missing-capability-key.log" >/dev/null || fail missing_capability_key_error_invalid

curl_local() {
  command curl --disable --proto '=http' --noproxy '*' --connect-timeout 2 --max-time 10 "$@"
}

assert_no_private_material() {
  local response_file="$1"
  [[ -f "$response_file" && ! -L "$response_file" ]] || fail response_file_invalid
  if grep -Fq "$MCP_TOKEN" "$response_file" ||
    grep -Fq "$CAPABILITY_KEY" "$response_file" ||
    grep -Fq 'v1.native-volume-1.' "$response_file"; then
    fail response_contains_private_material
  fi
}

write_curl_config() {
  local grant_file="${1:-}"
  printf 'header = "Authorization: Bearer %s"\n' "$MCP_TOKEN" >"$CURL_CONFIG"
  if [[ -n "$grant_file" ]]; then
    [[ -f "$grant_file" && ! -L "$grant_file" ]] || fail grant_file_invalid
    local grant
    IFS= read -r grant <"$grant_file"
    [[ "$grant" =~ ^v1\.[a-z0-9_-]+\.[0-9a-f]+\.[0-9a-f]{64}$ ]] || fail grant_value_invalid
    printf 'header = "MCP-Capability-Grant: %s"\n' "$grant" >>"$CURL_CONFIG"
  fi
  chmod 600 "$CURL_CONFIG"
}

start_server() {
  local enabled="$1"
  MCP__AUTH__STATIC_TOKEN="$MCP_TOKEN" \
  MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=false \
  MCP__ANDROID__VOLUME_CONTROL_ENABLED="$enabled" \
  MCP__CAPABILITY__KEY_ID=native-volume-1 \
  MCP__CAPABILITY__HMAC_KEY_HEX="$CAPABILITY_KEY" \
  MCP__SERVER__HOST=127.0.0.1 MCP__SERVER__PORT="$PORT" \
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
  fail server_not_ready
}

stop_server() {
  [[ -n "$SERVER_PID" ]] || return 0
  kill "$SERVER_PID" >/dev/null 2>&1 || fail server_shutdown_signal_failed
  wait "$SERVER_PID" 2>/dev/null || true
  SERVER_PID=''
  SESSION_ID=''
}

post_mcp() {
  local payload="$1" session="${2:-}" grant_file="${3:-}" max_time="${4:-10}"
  printf '%s' "$payload" >"$REQUEST_FILE"
  write_curl_config "$grant_file"
  local -a headers=(
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT"
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream'
  )
  [[ -z "$session" ]] || headers+=( -H "MCP-Session-Id: $session" -H 'MCP-Protocol-Version: 2025-11-25' )
  MCP_STATUS="$(curl_local --config "$CURL_CONFIG" --silent --show-error --max-time "$max_time" \
    --output "$BODY_FILE" --write-out '%{http_code}' "${headers[@]}" \
    --data-binary "@$REQUEST_FILE" "http://127.0.0.1:$PORT/mcp")"
  : >"$CURL_CONFIG"
  REQUEST_COUNT=$((REQUEST_COUNT + 1))
  assert_no_private_material "$BODY_FILE"
}

initialize_session() {
  printf '%s' '{"jsonrpc":"2.0","id":"initialize","method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"termux-volume-control-emulated-gate","version":"1"}}}' >"$REQUEST_FILE"
  write_curl_config
  MCP_STATUS="$(curl_local --config "$CURL_CONFIG" --silent --show-error --dump-header "$HEADER_FILE" \
    --output "$BODY_FILE" --write-out '%{http_code}' \
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
    --data-binary "@$REQUEST_FILE" "http://127.0.0.1:$PORT/mcp")"
  : >"$CURL_CONFIG"
  REQUEST_COUNT=$((REQUEST_COUNT + 1))
  assert_no_private_material "$BODY_FILE"
  [[ "$MCP_STATUS" == 200 ]] || fail initialize_status_invalid
  SESSION_ID="$(awk 'tolower($1) == "mcp-session-id:" {sub(/^[^:]*:[[:space:]]*/, ""); sub(/\r$/, ""); print; exit}' "$HEADER_FILE")"
  [[ "$SESSION_ID" =~ ^[A-Za-z0-9-]{1,128}$ ]] || fail session_invalid
  post_mcp '{"jsonrpc":"2.0","method":"notifications/initialized"}' "$SESSION_ID"
  [[ "$MCP_STATUS" == 202 && ! -s "$BODY_FILE" ]] || fail initialized_notification_invalid
}

issue_grant() {
  local stream="$1" level="$2" output="$3"
  : >"$output"; chmod 600 "$output"
  MCP__CAPABILITY__CONFIG_FILE="$CAPABILITY_CONFIG" \
  MCP__CAPABILITY__SESSION_ID="$SESSION_ID" \
  MCP__CAPABILITY__VOLUME_STREAM="$stream" \
  MCP__CAPABILITY__VOLUME_LEVEL="$level" \
    "$ARTIFACT" --issue-android-volume-grant >"$output" 2>/dev/null || fail grant_issuance_failed
  [[ "$(wc -l <"$output")" == 1 ]] || fail grant_line_count_invalid
}

log 'validating disabled runtime posture'
start_server false
initialize_session
post_mcp '{"jsonrpc":"2.0","id":"tools-disabled","method":"tools/list"}' "$SESSION_ID"
jq -e '([.result.tools[].name] | index("set_android_volume") == null) and ((.result.tools[] | select(.name == "write_file") | .inputSchema.properties.dry_run.const) == true)' "$BODY_FILE" >/dev/null || fail disabled_discovery_invalid
post_mcp '{"jsonrpc":"2.0","id":"runtime-disabled","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}' "$SESSION_ID"
jq -e '.result.structuredContent.androidVolumeControlCompiled == true and .result.structuredContent.androidVolumeControlEnabled == false and .result.structuredContent.fileWriteMutationEnabled == false and .result.structuredContent.fileWriteGrantRequired == false and .result.structuredContent.fileWriteMode == "dry_run_only_mutation_disabled" and .result.structuredContent.highImpactTools == false' "$BODY_FILE" >/dev/null || fail disabled_runtime_invalid
post_mcp "$(jq -cn --arg path "$SAFE_ROOT/volume-control-write-disabled.txt" '{jsonrpc:"2.0",id:"write-disabled-shared-key",method:"tools/call",params:{name:"write_file",arguments:{path:$path,content:"inert",dry_run:false}}}')" "$SESSION_ID"
[[ "$MCP_STATUS" == 403 ]] || fail disabled_write_file_http_invalid
jq -e '.error.code == -32003 and .error.data.reason == "write_file_mutation_disabled"' "$BODY_FILE" >/dev/null || fail disabled_write_file_contract_invalid
[[ ! -e "$SAFE_ROOT/volume-control-write-disabled.txt" && ! -L "$SAFE_ROOT/volume-control-write-disabled.txt" ]] || fail disabled_write_file_mutated
post_mcp '{"jsonrpc":"2.0","id":"call-disabled","method":"tools/call","params":{"name":"set_android_volume","arguments":{"stream":"music","level":9}}}' "$SESSION_ID"
jq -e '.result.isError == true and .result.structuredContent.reasonCode == "volume_control_runtime_disabled"' "$BODY_FILE" >/dev/null || fail disabled_call_invalid
stop_server

log 'validating enabled request-authorized volume posture'
start_server true
initialize_session
post_mcp '{"jsonrpc":"2.0","id":"tools","method":"tools/list"}' "$SESSION_ID"
jq -e '
  [.result.tools[].name] == ["runtime_status","platform_info","android_status","project_service_status","create_directory","copy_file","find_paths","hash_file","list_directory","path_metadata","read_binary_file","read_binary_range","read_file","read_text_range","search_text","write_file","set_android_volume"]
  and (.result.tools[] | select(.name == "set_android_volume") | .inputSchema.type) == "object"
  and (.result.tools[] | select(.name == "set_android_volume") | .inputSchema.required) == ["stream","level"]
  and (.result.tools[] | select(.name == "set_android_volume") | .inputSchema.additionalProperties) == false
  and (.result.tools[] | select(.name == "set_android_volume") | .inputSchema.properties.stream.enum) == ["alarm","call","music","notification","ring","system"]
  and (.result.tools[] | select(.name == "set_android_volume") | .inputSchema.properties.level.type) == "integer"
  and (.result.tools[] | select(.name == "set_android_volume") | .inputSchema.properties.level.minimum) == 0
  and ((.result.tools[] | select(.name == "write_file") | .inputSchema.properties.dry_run.const) == true)
' "$BODY_FILE" >/dev/null || fail enabled_discovery_invalid
post_mcp '{"jsonrpc":"2.0","id":"runtime","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}' "$SESSION_ID"
jq -e '
  .result.structuredContent.androidVolumeControlEnabled == true
  and .result.structuredContent.androidVolumeControlMode == "preview_or_request_scoped_single_use_grant"
  and .result.structuredContent.androidVolumeGrantRequired == true
  and .result.structuredContent.androidVolumeGrantHeader == "mcp-capability-grant"
  and .result.structuredContent.androidVolumeGrantTtlSeconds == 60
  and .result.structuredContent.androidDeviceControl == true
  and .result.structuredContent.fileWriteMutationEnabled == false
  and .result.structuredContent.fileWriteGrantRequired == false
  and .result.structuredContent.fileWriteMode == "dry_run_only_mutation_disabled"
  and .result.structuredContent.highImpactTools == true
  and .result.structuredContent.arbitraryCommandExecution == false
' "$BODY_FILE" >/dev/null || fail enabled_runtime_invalid

post_mcp "$(jq -cn --arg path "$SAFE_ROOT/volume-control-write-key-isolation.txt" '{jsonrpc:"2.0",id:"write-key-isolation",method:"tools/call",params:{name:"write_file",arguments:{path:$path,content:"inert",dry_run:false}}}')" "$SESSION_ID"
[[ "$MCP_STATUS" == 403 ]] || fail shared_key_enabled_write_file_http_invalid
jq -e '.error.code == -32003 and .error.data.reason == "write_file_mutation_disabled"' "$BODY_FILE" >/dev/null || fail shared_key_enabled_write_file_contract_invalid
[[ ! -e "$SAFE_ROOT/volume-control-write-key-isolation.txt" && ! -L "$SAFE_ROOT/volume-control-write-key-isolation.txt" ]] || fail shared_key_enabled_write_file_mutated

GRANT_MAIN="$WORK_ROOT/grant-main"
issue_grant music 9 "$GRANT_MAIN"
post_mcp '{"jsonrpc":"2.0","id":"preview","method":"tools/call","params":{"name":"set_android_volume","arguments":{"stream":"music","level":9}}}' "$SESSION_ID" "$GRANT_MAIN"
jq -e '.result.isError == false and .result.structuredContent == {stream:"music",previousLevel:5,requestedLevel:9,maxVolume:15,dryRun:true,changed:false,verified:false,outcome:"preview",rollback:"not_required"}' "$BODY_FILE" >/dev/null || fail preview_contract_invalid
[[ ! -e "$VOLUME_CALLS" ]] || fail preview_spawned_setter

post_mcp '{"jsonrpc":"2.0","id":"wrong-context","method":"tools/list"}' "$SESSION_ID" "$GRANT_MAIN"
[[ "$MCP_STATUS" == 400 ]] || fail header_context_http_invalid
jq -e '
  .jsonrpc == "2.0"
  and .id == "wrong-context"
  and .error.code == -32600
  and .error.message == "Invalid Request"
  and .error.data == "A request-scoped capability grant is accepted only for an exact grant-authorized tool call."
' "$BODY_FILE" >/dev/null || fail header_context_body_invalid

post_mcp '{"jsonrpc":"2.0","id":"wrong-level","method":"tools/call","params":{"name":"set_android_volume","arguments":{"stream":"music","level":8,"dry_run":false}}}' "$SESSION_ID" "$GRANT_MAIN"
[[ "$MCP_STATUS" == 403 ]] || fail wrong_binding_http_invalid
jq -e '.error.data.reason == "capability_grant_binding_mismatch"' "$BODY_FILE" >/dev/null || fail wrong_binding_body_invalid

post_mcp '{"jsonrpc":"2.0","id":"mutation","method":"tools/call","params":{"name":"set_android_volume","arguments":{"stream":"music","level":9,"dry_run":false}}}' "$SESSION_ID" "$GRANT_MAIN"
jq -e '.result.isError == false and .result.structuredContent.outcome == "mutation_verified" and .result.structuredContent.previousLevel == 5 and .result.structuredContent.requestedLevel == 9 and .result.structuredContent.verified == true' "$BODY_FILE" >/dev/null || fail mutation_contract_invalid
grep -Fx '2|music|9|/|/dev/null|0' "$VOLUME_CALLS" >/dev/null || fail fixed_process_boundary_invalid
[[ "$(cat "$VOLUME_STATE")" == 9 ]] || fail mutation_state_invalid

post_mcp '{"jsonrpc":"2.0","id":"replay","method":"tools/call","params":{"name":"set_android_volume","arguments":{"stream":"music","level":9,"dry_run":false}}}' "$SESSION_ID" "$GRANT_MAIN"
[[ "$MCP_STATUS" == 403 ]] || fail replay_http_invalid
jq -e '.error.data.reason == "capability_grant_replayed"' "$BODY_FILE" >/dev/null || fail replay_body_invalid

post_mcp '{"jsonrpc":"2.0","id":"range","method":"tools/call","params":{"name":"set_android_volume","arguments":{"stream":"music","level":16}}}' "$SESSION_ID"
jq -e '.result.isError == true and .result.structuredContent.reasonCode == "volume_control_level_out_of_range"' "$BODY_FILE" >/dev/null || fail fresh_bound_invalid

printf '5\n' >"$VOLUME_STATE"; printf 'fail_target\n' >"$VOLUME_MODE"
GRANT_SET_FAIL="$WORK_ROOT/grant-set-fail"; issue_grant music 9 "$GRANT_SET_FAIL"
post_mcp '{"jsonrpc":"2.0","id":"set-fail","method":"tools/call","params":{"name":"set_android_volume","arguments":{"stream":"music","level":9,"dry_run":false}}}' "$SESSION_ID" "$GRANT_SET_FAIL"
jq -e '.result.isError == true and .result.structuredContent.reasonCode == "volume_control_set_failed_rollback_confirmed"' "$BODY_FILE" >/dev/null || fail set_failure_recovery_invalid
[[ "$(cat "$VOLUME_STATE")" == 5 ]] || fail set_failure_restore_invalid

printf '5\n' >"$VOLUME_STATE"; printf 'fail_all\n' >"$VOLUME_MODE"
GRANT_UNCONFIRMED="$WORK_ROOT/grant-unconfirmed"; issue_grant music 9 "$GRANT_UNCONFIRMED"
post_mcp '{"jsonrpc":"2.0","id":"unconfirmed","method":"tools/call","params":{"name":"set_android_volume","arguments":{"stream":"music","level":9,"dry_run":false}}}' "$SESSION_ID" "$GRANT_UNCONFIRMED"
jq -e '.result.isError == true and .result.structuredContent.reasonCode == "volume_control_set_failed_rollback_unconfirmed"' "$BODY_FILE" >/dev/null || fail unconfirmed_recovery_invalid

printf '5\n' >"$VOLUME_STATE"; printf 'wrong_target\n' >"$VOLUME_MODE"
GRANT_VERIFY_FAIL="$WORK_ROOT/grant-verify-fail"; issue_grant music 9 "$GRANT_VERIFY_FAIL"
post_mcp '{"jsonrpc":"2.0","id":"verify-fail","method":"tools/call","params":{"name":"set_android_volume","arguments":{"stream":"music","level":9,"dry_run":false}}}' "$SESSION_ID" "$GRANT_VERIFY_FAIL"
jq -e '.result.isError == true and .result.structuredContent.reasonCode == "volume_control_verification_failed_rollback_confirmed"' "$BODY_FILE" >/dev/null || fail verification_recovery_invalid
[[ "$(cat "$VOLUME_STATE")" == 5 ]] || fail verification_restore_invalid

log 'validating post-consumption cancellation recovery'
printf '5\n' >"$VOLUME_STATE"; printf 'delayed_wrong\n' >"$VOLUME_MODE"; rm -f -- "$VOLUME_STARTED"
GRANT_CANCEL="$WORK_ROOT/grant-cancel"; issue_grant music 9 "$GRANT_CANCEL"
cancel_calls_before="$(wc -l <"$VOLUME_CALLS")"
printf '%s' '{"jsonrpc":"2.0","id":"cancel","method":"tools/call","params":{"name":"set_android_volume","arguments":{"stream":"music","level":9,"dry_run":false}}}' >"$REQUEST_FILE"
write_curl_config "$GRANT_CANCEL"
set +e
curl_local --config "$CURL_CONFIG" --silent --max-time 0.1 --output "$WORK_ROOT/cancel-body" \
  -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
  -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  -H "MCP-Session-Id: $SESSION_ID" -H 'MCP-Protocol-Version: 2025-11-25' \
  --data-binary "@$REQUEST_FILE" "http://127.0.0.1:$PORT/mcp"
cancel_rc=$?
set -e
: >"$CURL_CONFIG"; REQUEST_COUNT=$((REQUEST_COUNT + 1))
[[ "$cancel_rc" == 28 ]] || fail cancellation_request_not_aborted
[[ ! -e "$WORK_ROOT/cancel-body" ]] || assert_no_private_material "$WORK_ROOT/cancel-body"
for _attempt in $(seq 1 100); do
  cancel_calls_now="$(wc -l <"$VOLUME_CALLS")"
  [[ -e "$VOLUME_STARTED" && "$cancel_calls_now" -ge $((cancel_calls_before + 2)) && "$(cat "$VOLUME_STATE")" == 5 ]] && break
  sleep 0.05
done
cancel_calls_now="$(wc -l <"$VOLUME_CALLS")"
[[ -e "$VOLUME_STARTED" && "$cancel_calls_now" -ge $((cancel_calls_before + 2)) && "$(cat "$VOLUME_STATE")" == 5 ]] || fail cancellation_recovery_not_completed
post_mcp '{"jsonrpc":"2.0","id":"cancel-replay","method":"tools/call","params":{"name":"set_android_volume","arguments":{"stream":"music","level":9,"dry_run":false}}}' "$SESSION_ID" "$GRANT_CANCEL"
[[ "$MCP_STATUS" == 403 ]] || fail cancellation_grant_not_consumed

log 'validating non-queueing mutation lane'
printf '5\n' >"$VOLUME_STATE"; printf 'delayed_success\n' >"$VOLUME_MODE"; rm -f -- "$VOLUME_STARTED"
GRANT_FIRST="$WORK_ROOT/grant-first"; GRANT_SECOND="$WORK_ROOT/grant-second"
issue_grant music 9 "$GRANT_FIRST"; issue_grant music 7 "$GRANT_SECOND"
FIRST_CONFIG="$WORK_ROOT/first-curl.conf"; FIRST_REQUEST="$WORK_ROOT/first-request.json"; FIRST_BODY="$WORK_ROOT/first-body.json"
printf '%s' '{"jsonrpc":"2.0","id":"first","method":"tools/call","params":{"name":"set_android_volume","arguments":{"stream":"music","level":9,"dry_run":false}}}' >"$FIRST_REQUEST"
IFS= read -r first_grant <"$GRANT_FIRST"
printf 'header = "Authorization: Bearer %s"\nheader = "MCP-Capability-Grant: %s"\n' "$MCP_TOKEN" "$first_grant" >"$FIRST_CONFIG"; chmod 600 "$FIRST_CONFIG"; unset first_grant
curl_local --config "$FIRST_CONFIG" --silent --show-error --output "$FIRST_BODY" \
  -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
  -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  -H "MCP-Session-Id: $SESSION_ID" -H 'MCP-Protocol-Version: 2025-11-25' \
  --data-binary "@$FIRST_REQUEST" "http://127.0.0.1:$PORT/mcp" &
BACKGROUND_CURL_PID=$!; REQUEST_COUNT=$((REQUEST_COUNT + 1))
for _attempt in $(seq 1 100); do [[ -e "$VOLUME_STARTED" ]] && break; sleep 0.02; done
[[ -e "$VOLUME_STARTED" ]] || fail first_mutation_not_started
post_mcp '{"jsonrpc":"2.0","id":"conflict","method":"tools/call","params":{"name":"set_android_volume","arguments":{"stream":"music","level":7,"dry_run":false}}}' "$SESSION_ID" "$GRANT_SECOND"
jq -e '.result.isError == true and .result.structuredContent.reasonCode == "volume_control_concurrency_limit"' "$BODY_FILE" >/dev/null || fail concurrency_contract_invalid
wait "$BACKGROUND_CURL_PID" || fail first_mutation_request_failed
BACKGROUND_CURL_PID=''
: >"$FIRST_CONFIG"
assert_no_private_material "$FIRST_BODY"
jq -e '.result.isError == false and .result.structuredContent.verified == true' "$FIRST_BODY" >/dev/null || fail first_mutation_body_invalid
printf 'success\n' >"$VOLUME_MODE"
post_mcp '{"jsonrpc":"2.0","id":"second-retry","method":"tools/call","params":{"name":"set_android_volume","arguments":{"stream":"music","level":7,"dry_run":false}}}' "$SESSION_ID" "$GRANT_SECOND"
jq -e '.result.isError == false and .result.structuredContent.requestedLevel == 7 and .result.structuredContent.verified == true' "$BODY_FILE" >/dev/null || fail conflict_grant_was_consumed

post_mcp '{"jsonrpc":"2.0","id":"audit","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}' "$SESSION_ID"
jq -e '
  .result.structuredContent.auditCounters.by_tool.set_android_volume.allowed >= 4
  and .result.structuredContent.auditCounters.by_tool.set_android_volume.denied >= 7
  and .result.structuredContent.auditCounters.by_reason_code.volume_control_preview.allowed >= 1
  and .result.structuredContent.auditCounters.by_reason_code.volume_control_mutation_verified.allowed >= 3
  and .result.structuredContent.auditCounters.by_reason_code.volume_control_concurrency_limit.denied >= 1
  and .result.structuredContent.auditCounters.by_reason_code.volume_control_set_failed_rollback_confirmed.denied >= 1
  and .result.structuredContent.auditCounters.by_reason_code.volume_control_set_failed_rollback_unconfirmed.denied >= 1
  and .result.structuredContent.auditCounters.by_reason_code.volume_control_verification_failed_rollback_confirmed.denied >= 1
' "$BODY_FILE" >/dev/null || fail audit_contract_invalid
stop_server

COMPLETED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
REPORT_NEXT="$WORK_ROOT/volume-control-emulated-evidence.json"
jq -n \
  --arg gate_version "$GATE_VERSION" --arg started_at "$STARTED_AT" --arg completed_at "$COMPLETED_AT" \
  --arg commit "$EXPECTED_COMMIT" --arg version "$EXPECTED_VERSION" \
  --arg ci_run_id "$CI_RUN_ID" --arg security_run_id "$SECURITY_RUN_ID" --arg android_run_id "$ANDROID_RUN_ID" \
  --arg artifact_sha "$ARTIFACT_SHA" --argjson artifact_bytes "$ARTIFACT_BYTES" \
  --arg incompatible_sha "$INCOMPATIBLE_SHA" --argjson incompatible_bytes "$INCOMPATIBLE_BYTES" \
  --arg architecture "$(uname -m)" --arg image "$EXPECTED_IMAGE" --arg image_digest "$IMAGE_DIGEST" \
  --argjson requests "$REQUEST_COUNT" '
  {
    schemaVersion:1, gateVersion:$gate_version, status:"pass", failureCode:null,
    releaseQualificationEligible:false, startedAt:$started_at, completedAt:$completed_at,
    candidate:{commit:$commit,version:$version,ciRunId:$ci_run_id,securityRunId:$security_run_id,androidRunId:$android_run_id,
      artifact:{sha256:$artifact_sha,bytes:$artifact_bytes},incompatibleArtifact:{sha256:$incompatible_sha,bytes:$incompatible_bytes}},
    environment:{architecture:$architecture,executionMode:"official-termux-docker-native-arm64",image:$image,imageDigest:$image_digest,androidLinker:true},
    validation:{status:"pass",requests:$requests,exactArtifact:true,compileGate:true,runtimeDefaultDisabled:true,disabledDiscovery:true,
      staticTokenRequired:true,capabilityKeyRequired:true,closedInputSchema:true,previewNoMutation:true,previewDoesNotConsumeGrant:true,
      headerContextEnforced:true,exactGrantBinding:true,singleUseReplay:true,freshMaximum:true,fixedProgram:true,exactTwoArguments:true,
      fixedWorkingDirectory:true,inheritedEnvironmentCleared:true,nullStdin:true,nonQueueingConcurrency:true,mutationVerified:true,
      rollbackConfirmed:true,rollbackUnconfirmed:true,cancellationIndependentRecovery:true,boundedSupervisor:true,auditCounters:true,
      redactedResponses:true,arbitraryCommandExecutionDisabled:true,broaderAndroidControlDisabled:true,longObservationRequired:false}
  }' >"$REPORT_NEXT" || fail report_generation_failed
chmod 600 "$REPORT_NEXT"
jq -e '
  .schemaVersion == 1 and .gateVersion == "1" and .status == "pass"
  and .failureCode == null and .releaseQualificationEligible == false
  and .validation.status == "pass" and .validation.requests >= 20
  and ([.validation | to_entries[] | select(.key != "longObservationRequired" and (.value|type)=="boolean") | .value] | all)
' "$REPORT_NEXT" >/dev/null || fail generated_report_invalid
if grep -Eq '/data/|Bearer[[:space:]]|MCP__|native-volume|secret|private|[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}' "$REPORT_NEXT"; then
  fail report_contains_sensitive_value
fi
install -m 600 "$REPORT_NEXT" "$OUTPUT_REPORT" || fail report_publication_failed
log "report_sha256=$(sha256sum "$OUTPUT_REPORT" | awk '{print $1}')"
log "report=$OUTPUT_REPORT"
printf 'TERMUX_MCP_VOLUME_CONTROL_EMULATED_RESULT=PASS\n'
