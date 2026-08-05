#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077
set +x

readonly GATE_VERSION="1"
readonly REPOSITORY="CyberBASSLord-666/termux-mcp-edge"
readonly QUALIFICATION_CLASS="physical_shizuku_rish_identity_development_v1"
readonly QUALIFICATION_SCOPE="s2_5_uid_probe_only"
readonly ARTIFACT_NAME="termux-mcp-server-aarch64-linux-android-android-rish-development"
readonly BASELINE_TOOLS='["runtime_status","platform_info","android_status","project_service_status","create_directory","copy_file","trash_file","find_paths","hash_file","list_directory","path_metadata","read_binary_file","read_binary_range","read_file","read_text_range","search_text","write_file"]'
readonly ENABLED_TOOLS='["runtime_status","platform_info","android_status","project_service_status","create_directory","copy_file","trash_file","find_paths","hash_file","list_directory","path_metadata","read_binary_file","read_binary_range","read_file","read_text_range","search_text","write_file","android_rish_status"]'

ARTIFACT=""
ARTIFACT_SHA256=""
EXPECTED_COMMIT=""
EXPECTED_VERSION=""
POLICY_SHA256=""
CARGO_LOCK_SHA256=""
WORKFLOW_DEFINITION_SHA256=""
WORKFLOW_RUN_ID=""
WORKFLOW_RUN_ATTEMPT=""
CI_RUN_ID=""
SECURITY_RUN_ID=""
ANDROID_RUN_ID=""
CONTROLLER_CHALLENGE_SHA256=""
RISH_DEX=""
RISH_DEX_SHA256=""
API_LEVEL=""
SECURITY_PATCH=""
DEVICE_PROFILE_COMMITMENT=""
BUILD_FINGERPRINT_SHA256=""
TERMUX_VERSION=""
TERMUX_SIGNER_SHA256=""
SHIZUKU_VERSION=""
SHIZUKU_SIGNER_SHA256=""
ADB_SHELL_UID=""
OUTPUT=""
RAW_REPORT=""
REQUESTED_PORT=""

WORK_ROOT=""
SAFE_ROOT=""
DEX_ROOT=""
PINNED_DEX=""
SERVER_PID=""
SERVER_PGID=""
SERVER_LOG=""
PORT=""
MCP_TOKEN=""
MCP_SESSION_ID=""
OUTPUT_NEXT=""
RAW_REPORT_NEXT=""
CLEANUP_CONFIRMED=1
STARTED_AT=""
COMPLETED_AT=""
# Captured from the live android_rish_status payload during validation.
OBSERVED_RISH_STATE=""

usage() {
  cat <<'EOF'
Usage: termux_rish_physical_gate.sh \
  --artifact ABSOLUTE_BINARY \
  --artifact-sha256 SHA256 \
  --expected-commit SHA \
  --expected-version VERSION \
  --policy-sha256 SHA256 \
  --cargo-lock-sha256 SHA256 \
  --workflow-definition-sha256 SHA256 \
  --workflow-run-id ID \
  --workflow-run-attempt 1 \
  --ci-run-id ID \
  --security-run-id ID \
  --android-run-id ID \
  --controller-challenge-sha256 SHA256 \
  --rish-dex ABSOLUTE_DEX \
  --rish-dex-sha256 SHA256 \
  --api-level API \
  --security-patch YYYY-MM-DD \
  --device-profile-commitment SHA256 \
  --build-fingerprint-sha256 SHA256 \
  --termux-version VERSION \
  --termux-signer-sha256 SHA256 \
  --shizuku-version VERSION \
  --shizuku-signer-sha256 SHA256 \
  --adb-shell-uid 2000 \
  --raw-report ABSOLUTE_PRIVATE_TRANSCRIPT \
  --output ABSOLUTE_JSON \
  [--port PORT]

Runs the fixed, test-only S2.5 Shizuku/rish identity gate inside official
Termux on a physical AArch64 Android device. It installs no packages, runs no
caller-selected command, performs no Android mutation, and never makes the
candidate release eligible or production-control qualified.
EOF
}

fail() {
  local reason="${1:-internal_error}"
  case "$reason" in
    *[!a-z0-9_]*|"") reason="internal_error" ;;
  esac
  printf 'TERMUX_RISH_PHYSICAL_GATE_RESULT=FAIL reason=%s\n' "$reason" >&2
  exit 1
}

is_sha256() {
  [[ "$1" =~ ^[0-9a-f]{64}$ ]]
}

is_canonical_private_regular_file() {
  local path="$1" expected_mode="$2" maximum_bytes="$3"
  local canonical mode owner links bytes
  [[ "$path" == /* && -f "$path" && ! -L "$path" ]] || return 1
  canonical="$(realpath -e -- "$path" 2>/dev/null)" || return 1
  [[ "$canonical" == "$path" ]] || return 1
  mode="$(stat -c '%a' -- "$path" 2>/dev/null)" || return 1
  owner="$(stat -c '%u' -- "$path" 2>/dev/null)" || return 1
  links="$(stat -c '%h' -- "$path" 2>/dev/null)" || return 1
  bytes="$(stat -c '%s' -- "$path" 2>/dev/null)" || return 1
  [[ "$mode" == "$expected_mode" && "$owner" == "$(id -u)" && "$links" == 1 ]] || return 1
  [[ "$bytes" =~ ^[0-9]+$ ]] || return 1
  ((bytes >= 1 && bytes <= maximum_bytes))
}

validate_private_parent() {
  local path="$1" parent canonical mode owner
  parent="$(dirname -- "$path")"
  [[ "$parent" == /* && -d "$parent" && ! -L "$parent" ]] || return 1
  canonical="$(realpath -e -- "$parent" 2>/dev/null)" || return 1
  [[ "$canonical" == "$parent" ]] || return 1
  mode="$(stat -c '%a' -- "$parent" 2>/dev/null)" || return 1
  owner="$(stat -c '%u' -- "$parent" 2>/dev/null)" || return 1
  [[ "$mode" == 700 && "$owner" == "$(id -u)" ]]
}

port_is_free() {
  local port="$1"
  ! ss -ltnH 2>/dev/null | awk '{print $4}' | grep -Eq ":${port}$"
}

choose_port() {
  local candidate
  if [[ -n "$REQUESTED_PORT" ]]; then
    port_is_free "$REQUESTED_PORT" || return 1
    printf '%s\n' "$REQUESTED_PORT"
    return 0
  fi
  for candidate in $(seq 19120 19219); do
    if port_is_free "$candidate"; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  return 1
}

read_process_group_and_session() {
  local pid="$1" record remainder state parent group session rest
  [[ "$pid" =~ ^[1-9][0-9]*$ && -r "/proc/$pid/stat" ]] || return 1
  IFS= read -r record <"/proc/$pid/stat" || return 1
  [[ "$record" == *") "* ]] || return 1
  remainder="${record##*) }"
  IFS=' ' read -r state parent group session rest <<<"$remainder"
  [[ "$group" =~ ^[1-9][0-9]*$ && "$session" =~ ^[1-9][0-9]*$ ]] || return 1
  printf '%s:%s\n' "$group" "$session"
}

process_group_alive() {
  local expected_group="$1" stat_file record remainder state parent group session rest
  [[ "$expected_group" =~ ^[1-9][0-9]*$ ]] || return 1
  for stat_file in /proc/[0-9]*/stat; do
    [[ -r "$stat_file" ]] || continue
    IFS= read -r record <"$stat_file" 2>/dev/null || continue
    [[ "$record" == *") "* ]] || continue
    remainder="${record##*) }"
    IFS=' ' read -r state parent group session rest <<<"$remainder"
    [[ "$group" == "$expected_group" && "$state" != Z ]] && return 0
  done
  return 1
}

server_process_group_is_isolated() {
  local identity shell_identity shell_group
  [[ "$SERVER_PID" =~ ^[1-9][0-9]*$ && "$SERVER_PGID" == "$SERVER_PID" ]] \
    || return 1
  identity="$(read_process_group_and_session "$SERVER_PID")" || return 1
  [[ "$identity" == "$SERVER_PGID:$SERVER_PGID" ]] || return 1
  shell_identity="$(read_process_group_and_session "$$")" || return 1
  shell_group="${shell_identity%%:*}"
  [[ "$SERVER_PGID" != "$shell_group" ]]
}

terminate_process_group_bounded() {
  local pid="${1:-}" group="${2:-}"
  [[ "$pid" =~ ^[1-9][0-9]*$ && "$group" == "$pid" ]] || return 1
  kill -TERM -- "-$group" >/dev/null 2>&1 || true
  for _ in $(seq 1 50); do
    if ! process_group_alive "$group"; then
      wait "$pid" >/dev/null 2>&1 || true
      return 0
    fi
    sleep 0.1
  done
  kill -KILL -- "-$group" >/dev/null 2>&1 || true
  for _ in $(seq 1 20); do
    if ! process_group_alive "$group"; then
      wait "$pid" >/dev/null 2>&1 || true
      return 0
    fi
    sleep 0.1
  done
  return 1
}

stop_server() {
  if [[ -n "$SERVER_PID" || -n "$SERVER_PGID" ]]; then
    terminate_process_group_bounded "$SERVER_PID" "$SERVER_PGID" || return 1
    SERVER_PID=""
    SERVER_PGID=""
  fi
  for _ in $(seq 1 20); do
    if port_is_free "$PORT"; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

trusted_direct_rish_probe() {
  local phase="$1" stdout stderr expected
  [[ "$phase" == pre_candidate || "$phase" == post_candidate ]] || return 1
  is_canonical_private_regular_file "$PINNED_DEX" 400 16777216 || return 1
  validate_private_parent "$PINNED_DEX" || return 1
  [[ "$(sha256sum -- "$PINNED_DEX" | awk '{print $1}')" == "$RISH_DEX_SHA256" ]] \
    || return 1
  stdout="$WORK_ROOT/trusted-rish-$phase.stdout"
  stderr="$WORK_ROOT/trusted-rish-$phase.stderr"
  expected="$WORK_ROOT/trusted-rish-$phase.expected"
  printf '2000\n' >"$expected" || return 1
  if ! (
    cd /
    ulimit -f 8
    exec timeout --signal=TERM --kill-after=2s 5s \
      env -i \
        RISH_APPLICATION_ID=com.termux \
        RISH_PRESERVE_ENV=0 \
        /system/bin/app_process64 \
          "-Djava.class.path=$PINNED_DEX" \
          /system/bin \
          --nice-name=termux-mcp-rish \
          rikka.shizuku.shell.ShizukuShellLoader \
          -c 'exec /system/bin/id -u'
  ) </dev/null >"$stdout" 2>"$stderr"
  then
    return 1
  fi
  [[ "$(stat -c '%s' "$stdout")" -le 1024 \
    && "$(stat -c '%s' "$stderr")" -le 4096 \
    && ! -s "$stderr" ]] || return 1
  cmp -s -- "$expected" "$stdout"
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM HUP
  set +e
  if ! stop_server; then
    CLEANUP_CONFIRMED=0
    status=1
  fi
  unset MCP_TOKEN MCP_SESSION_ID 2>/dev/null || true
  if ((CLEANUP_CONFIRMED == 1)); then
    if [[ -n "$WORK_ROOT" && "$WORK_ROOT" == "$HOME"/.termux-mcp-rish-physical.* ]]; then
      rm -rf -- "$WORK_ROOT" >/dev/null 2>&1 || CLEANUP_CONFIRMED=0
    fi
    if [[ -n "$WORK_ROOT" && (-e "$WORK_ROOT" || -L "$WORK_ROOT") ]]; then
      CLEANUP_CONFIRMED=0
    fi
  fi
  if ((CLEANUP_CONFIRMED == 0)); then
    status=1
    printf 'TERMUX_RISH_PHYSICAL_GATE_CLEANUP=UNCONFIRMED\n' >&2
  fi
  [[ -z "$OUTPUT_NEXT" ]] || rm -f -- "$OUTPUT_NEXT" >/dev/null 2>&1 || status=1
  [[ -z "$RAW_REPORT_NEXT" ]] || rm -f -- "$RAW_REPORT_NEXT" >/dev/null 2>&1 || status=1
  exit "$status"
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail required_command_missing
}

validate_fixed_setsid_program() {
  local program="$PREFIX/bin/setsid"
  [[ -x "$program" && -f "$program" && ! -L "$program" ]] || return 1
  [[ "$(realpath -e -- "$program" 2>/dev/null)" == "$program" ]] || return 1
  [[ "$(stat -c '%u:%h' -- "$program" 2>/dev/null)" == "$(id -u):1" ]]
}

curl_local() {
  command curl --disable --proto '=http' --noproxy '*' \
    --connect-timeout 2 --max-time 10 "$@"
}

wait_for_ready() {
  local health ready
  for _ in $(seq 1 50); do
    kill -0 "$SERVER_PID" >/dev/null 2>&1 || return 1
    health="$(curl_local -fsS --max-time 2 "http://127.0.0.1:$PORT/health" 2>/dev/null || true)"
    ready="$(curl_local -fsS --max-time 2 "http://127.0.0.1:$PORT/ready" 2>/dev/null || true)"
    if [[ "$health" == ok ]] \
      && jq -e --arg version "$EXPECTED_VERSION" \
        '.status == "ready" and .version == $version' <<<"$ready" >/dev/null 2>&1
    then
      server_process_group_is_isolated || return 1
      return 0
    fi
    sleep 0.1
  done
  return 1
}

launch_server() {
  local rish_enabled="$1" dex_path="${2:-}" dex_sha="${3:-}"
  local -a environment=(
    "HOME=$HOME"
    "PREFIX=$PREFIX"
    "PATH=$PREFIX/bin:/system/bin"
    "MCP__AUTH__STATIC_TOKEN=$MCP_TOKEN"
    "MCP__SERVER__HOST=127.0.0.1"
    "MCP__SERVER__PORT=$PORT"
    "MCP__TRANSPORT__ALLOWED_HOSTS=localhost:$PORT,127.0.0.1:$PORT"
    "MCP__TRANSPORT__ALLOWED_ORIGINS=http://localhost:$PORT,http://127.0.0.1:$PORT"
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
    "MCP__ANDROID__RISH_ENABLED=$rish_enabled"
  )
  if [[ "$rish_enabled" == true ]]; then
    environment+=(
      "MCP__ANDROID__RISH_DEX_PATH=$dex_path"
      "MCP__ANDROID__RISH_DEX_SHA256=$dex_sha"
    )
  fi
  (
    ulimit -f 32768
    exec env -i "${environment[@]}" \
      "$PREFIX/bin/setsid" --wait -- "$ARTIFACT"
  ) >"$SERVER_LOG" 2>&1 &
  SERVER_PID=$!
  SERVER_PGID=$SERVER_PID
}

expect_startup_rejection() {
  local dex_path="$1" dex_sha="$2"
  launch_server true "$dex_path" "$dex_sha"
  for _ in $(seq 1 30); do
    if ! kill -0 "$SERVER_PID" >/dev/null 2>&1; then
      wait "$SERVER_PID" >/dev/null 2>&1 || true
      if process_group_alive "$SERVER_PGID"; then
        terminate_process_group_bounded "$SERVER_PID" "$SERVER_PGID" \
          >/dev/null 2>&1 || true
        SERVER_PID=""
        SERVER_PGID=""
        return 1
      fi
      SERVER_PID=""
      SERVER_PGID=""
      port_is_free "$PORT" || return 1
      return 0
    fi
    sleep 0.1
  done
  terminate_process_group_bounded "$SERVER_PID" "$SERVER_PGID" >/dev/null 2>&1 || true
  SERVER_PID=""
  SERVER_PGID=""
  return 1
}

mcp_post() {
  local output="$1" payload="$2" session="${3:-}"
  local -a args=(
    -sS
    -o "$output"
    -w '%{http_code}'
    -H "Authorization: Bearer $MCP_TOKEN"
    -H "Host: localhost:$PORT"
    -H "Origin: http://localhost:$PORT"
    -H 'Content-Type: application/json'
    -H 'Accept: application/json, text/event-stream'
  )
  if [[ -n "$session" ]]; then
    args+=(
      -H 'MCP-Protocol-Version: 2025-11-25'
      -H "MCP-Session-Id: $session"
    )
  fi
  curl_local "${args[@]}" --data-binary "$payload" "http://127.0.0.1:$PORT/mcp"
}

initialize_session() {
  local body="$1" headers="$2" status
  status="$(curl_local -sS -D "$headers" -o "$body" -w '%{http_code}' \
    -H "Authorization: Bearer $MCP_TOKEN" \
    -H "Host: localhost:$PORT" \
    -H "Origin: http://localhost:$PORT" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    --data-binary '{"jsonrpc":"2.0","id":"initialize","method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"termux-rish-physical-gate","version":"1"}}}' \
    "http://127.0.0.1:$PORT/mcp")"
  [[ "$status" == 200 ]] || return 1
  jq -e '.result.protocolVersion == "2025-11-25"' "$body" >/dev/null || return 1
  MCP_SESSION_ID="$(awk 'tolower($1) == "mcp-session-id:" {sub(/^[^:]*:[[:space:]]*/, ""); sub(/\r$/, ""); print; exit}' "$headers")"
  [[ "$MCP_SESSION_ID" =~ ^[A-Za-z0-9-]{1,128}$ ]] || return 1
  status="$(mcp_post "$body" '{"jsonrpc":"2.0","method":"notifications/initialized"}' "$MCP_SESSION_ID")"
  [[ "$status" == 202 && ! -s "$body" ]]
}

close_session() {
  local body="$1" status
  [[ -n "$MCP_SESSION_ID" ]] || return 0
  status="$(curl_local -sS -X DELETE -o "$body" -w '%{http_code}' \
    -H "Authorization: Bearer $MCP_TOKEN" \
    -H "Host: localhost:$PORT" \
    -H "Origin: http://localhost:$PORT" \
    -H 'MCP-Protocol-Version: 2025-11-25' \
    -H "MCP-Session-Id: $MCP_SESSION_ID" \
    "http://127.0.0.1:$PORT/mcp")"
  MCP_SESSION_ID=""
  [[ "$status" == 204 && ! -s "$body" ]]
}

validate_disabled_posture() {
  local body="$WORK_ROOT/disabled-body.json" headers="$WORK_ROOT/disabled-headers" status
  SERVER_LOG="$WORK_ROOT/disabled-server.log"
  launch_server false
  wait_for_ready || fail disabled_runtime_not_ready
  initialize_session "$body" "$headers" || fail disabled_initialize_failed

  status="$(mcp_post "$body" '{"jsonrpc":"2.0","id":"disabled-tools","method":"tools/list"}' "$MCP_SESSION_ID")"
  [[ "$status" == 200 ]] || fail disabled_tools_http_invalid
  jq -e --argjson expected "$BASELINE_TOOLS" \
    '[.result.tools[].name] == $expected' "$body" >/dev/null \
    || fail disabled_tool_posture_invalid

  status="$(mcp_post "$body" '{"jsonrpc":"2.0","id":"disabled-rish","method":"tools/call","params":{"name":"android_rish_status","arguments":{}}}' "$MCP_SESSION_ID")"
  [[ "$status" == 200 ]] || fail disabled_direct_call_http_invalid
  jq -e '
    .result.isError == true
    and .result.structuredContent.reasonCode == "rish_runtime_disabled"
  ' "$body" >/dev/null || fail disabled_direct_call_contract_invalid

  close_session "$body" || fail disabled_session_cleanup_failed
  stop_server || fail disabled_server_cleanup_failed
}

validate_enabled_posture() {
  local body="$WORK_ROOT/enabled-body.json" headers="$WORK_ROOT/enabled-headers" status replacement
  SERVER_LOG="$WORK_ROOT/enabled-server.log"
  launch_server true "$PINNED_DEX" "$RISH_DEX_SHA256"
  wait_for_ready || fail enabled_runtime_not_ready
  initialize_session "$body" "$headers" || fail enabled_initialize_failed

  status="$(mcp_post "$body" '{"jsonrpc":"2.0","id":"enabled-tools","method":"tools/list"}' "$MCP_SESSION_ID")"
  [[ "$status" == 200 ]] || fail enabled_tools_http_invalid
  jq -e --argjson expected "$ENABLED_TOOLS" '
    [.result.tools[].name] == $expected
    and (.result.tools | map(select(.name == "android_rish_status")) | length) == 1
    and (.result.tools | map(select(.name == "android_rish_status"))[0].inputSchema
      | .type == "object"
      and .properties == {}
      and .required == []
      and .additionalProperties == false)
    and all(.result.tools[] | select(.name == "create_directory" or .name == "copy_file" or .name == "trash_file" or .name == "write_file");
      .inputSchema.properties.dry_run.const == true)
  ' "$body" >/dev/null || fail enabled_tool_posture_invalid

  status="$(mcp_post "$body" '{"jsonrpc":"2.0","id":"runtime","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}' "$MCP_SESSION_ID")"
  [[ "$status" == 200 ]] || fail runtime_status_http_invalid
  jq -e '
    .result.structuredContent.androidRishCompiled == true
    and .result.structuredContent.androidRishEnabled == true
    and .result.structuredContent.androidRishMode == "configured_s3_attestation_on_call_adb_shell_uid_2000"
    and .result.structuredContent.androidRishArbitraryShell == false
    and .result.structuredContent.androidRishMutations == false
    and .result.structuredContent.createDirectoryMutationEnabled == false
    and .result.structuredContent.copyFileMutationEnabled == false
    and .result.structuredContent.trashFileMutationEnabled == false
    and .result.structuredContent.fileWriteMutationEnabled == false
    and .result.structuredContent.androidVolumeControlEnabled == false
    and .result.structuredContent.commandExecution == false
  ' "$body" >/dev/null || fail runtime_status_contract_invalid

  status="$(mcp_post "$body" '{"jsonrpc":"2.0","id":"rish","method":"tools/call","params":{"name":"android_rish_status","arguments":{}}}' "$MCP_SESSION_ID")"
  [[ "$status" == 200 ]] || fail rish_status_http_invalid
  jq -e '
    .result.isError == false
    and .result.structuredContent.available == true
    and .result.structuredContent.backend == "shizuku_rish"
    and .result.structuredContent.principal == "android_shell"
    and .result.structuredContent.uid == 2000
    and (
      .result.structuredContent.state == "verified_shell_uid"
      or .result.structuredContent.state == "attested_read_only"
    )
    and .result.structuredContent.rootAccepted == false
    and .result.structuredContent.arbitraryShell == false
    and .result.structuredContent.mutationReady == false
  ' "$body" >/dev/null || fail rish_status_contract_invalid
  # Record the observed state for evidence — never hard-code a different posture.
  OBSERVED_RISH_STATE="$(jq -r '.result.structuredContent.state' "$body")"
  case "$OBSERVED_RISH_STATE" in
    verified_shell_uid|attested_read_only) ;;
    *) fail rish_status_state_invalid ;;
  esac

  status="$(mcp_post "$body" '{"jsonrpc":"2.0","id":"arguments","method":"tools/call","params":{"name":"android_rish_status","arguments":{"command":"id","argv":["-u"],"dry_run":false}}}' "$MCP_SESSION_ID")"
  [[ "$status" == 400 ]] || fail rish_arguments_http_invalid
  jq -e '.error.code == -32602' "$body" >/dev/null || fail rish_arguments_contract_invalid

  status="$(mcp_post "$body" '{"jsonrpc":"2.0","id":"shell","method":"tools/call","params":{"name":"shell","arguments":{"command":"id"}}}' "$MCP_SESSION_ID")"
  [[ "$status" == 400 ]] || fail arbitrary_shell_http_invalid
  jq -e '.error.code == -32602' "$body" >/dev/null || fail arbitrary_shell_contract_invalid

  replacement="$DEX_ROOT/replacement.dex"
  install -m 400 -- "$RISH_DEX" "$replacement" || fail dex_replacement_staging_failed
  [[ "$(sha256sum -- "$replacement" | awk '{print $1}')" == "$RISH_DEX_SHA256" ]] \
    || fail dex_replacement_digest_invalid
  mv -f -- "$replacement" "$PINNED_DEX" || fail dex_replacement_failed
  status="$(mcp_post "$body" '{"jsonrpc":"2.0","id":"tamper","method":"tools/call","params":{"name":"android_rish_status","arguments":{}}}' "$MCP_SESSION_ID")"
  [[ "$status" == 200 ]] || fail dex_tamper_http_invalid
  jq -e '
    .result.isError == true
    and .result.structuredContent.reasonCode == "rish_dex_identity_changed"
  ' "$body" >/dev/null || fail dex_tamper_contract_invalid

  close_session "$body" || fail enabled_session_cleanup_failed
  stop_server || fail enabled_server_cleanup_failed
}

write_raw_report() {
  local artifact_bytes dex_bytes
  artifact_bytes="$(stat -c '%s' -- "$ARTIFACT")" || fail artifact_size_invalid
  dex_bytes="$(stat -c '%s' -- "$RISH_DEX")" || fail rish_dex_size_invalid
  RAW_REPORT_NEXT="$(mktemp "$(dirname -- "$RAW_REPORT")/.android-rish-physical-raw.XXXXXX")" \
    || fail raw_report_staging_failed
  {
    printf 'gate_version=%s\n' "$GATE_VERSION"
    printf 'qualification_class=%s\n' "$QUALIFICATION_CLASS"
    printf 'scope=%s\n' "$QUALIFICATION_SCOPE"
    printf 'started_at=%s\n' "$STARTED_AT"
    printf 'completed_at=%s\n' "$COMPLETED_AT"
    printf 'repository=%s\n' "$REPOSITORY"
    printf 'commit=%s\n' "$EXPECTED_COMMIT"
    printf 'version=%s\n' "$EXPECTED_VERSION"
    printf 'workflow_run_id=%s\n' "$WORKFLOW_RUN_ID"
    printf 'workflow_run_attempt=%s\n' "$WORKFLOW_RUN_ATTEMPT"
    printf 'ci_run_id=%s\n' "$CI_RUN_ID"
    printf 'security_run_id=%s\n' "$SECURITY_RUN_ID"
    printf 'android_run_id=%s\n' "$ANDROID_RUN_ID"
    printf 'artifact_sha256=%s\n' "$ARTIFACT_SHA256"
    printf 'artifact_bytes=%s\n' "$artifact_bytes"
    printf 'rish_dex_sha256=%s\n' "$RISH_DEX_SHA256"
    printf 'rish_dex_bytes=%s\n' "$dex_bytes"
    printf 'api_level=%s\n' "$API_LEVEL"
    printf 'security_patch=%s\n' "$SECURITY_PATCH"
    printf 'adb_shell_uid=%s\n' "$ADB_SHELL_UID"
    printf 'trusted_direct_rish_probe_pre_candidate=pass\n'
    printf 'runtime_disabled_tool_absent=pass\n'
    printf 'candidate_mcp_status_uid_2000=pass\n'
    printf 'extra_arguments_rejected=pass\n'
    printf 'unknown_shell_rejected=pass\n'
    printf 'dex_tamper_rejected=pass\n'
    printf 'dex_mode_rejected=pass\n'
    printf 'dex_symlink_rejected=pass\n'
    printf 'all_mutation_gates_disabled=pass\n'
    printf 'trusted_direct_rish_probe_post_candidate=pass\n'
    printf 'bounded_test_fixture_cleanup=pending\n'
  } >"$RAW_REPORT_NEXT" || fail raw_report_write_failed
  chmod 600 "$RAW_REPORT_NEXT" || fail raw_report_mode_failed
  mv -Tn -- "$RAW_REPORT_NEXT" "$RAW_REPORT" || fail raw_report_publication_failed
  RAW_REPORT_NEXT=""
  [[ -f "$RAW_REPORT" && ! -L "$RAW_REPORT" && "$(stat -c '%a' "$RAW_REPORT")" == 600 ]] \
    || fail raw_report_publication_failed
}

write_evidence() {
  local artifact_bytes dex_sha dex_bytes raw_report_sha
  artifact_bytes="$(stat -c '%s' -- "$ARTIFACT")" || fail artifact_size_invalid
  dex_sha="$(sha256sum -- "$RISH_DEX" | awk '{print $1}')" || fail rish_dex_digest_invalid
  dex_bytes="$(stat -c '%s' -- "$RISH_DEX")" || fail rish_dex_size_invalid
  [[ "$dex_sha" == "$RISH_DEX_SHA256" ]] || fail rish_dex_changed
  case "$OBSERVED_RISH_STATE" in
    verified_shell_uid|attested_read_only) ;;
    *) fail observed_rish_state_missing ;;
  esac
  raw_report_sha="$(sha256sum -- "$RAW_REPORT" | awk '{print $1}')" \
    || fail raw_report_digest_failed
  is_sha256 "$raw_report_sha" || fail raw_report_digest_failed
  OUTPUT_NEXT="$(mktemp "$(dirname -- "$OUTPUT")/.android-rish-physical.XXXXXX")" \
    || fail output_staging_failed
  jq -cn \
    --arg gate_version "$GATE_VERSION" \
    --arg repository "$REPOSITORY" \
    --arg commit "$EXPECTED_COMMIT" \
    --arg version "$EXPECTED_VERSION" \
    --arg started_at "$STARTED_AT" \
    --arg completed_at "$COMPLETED_AT" \
    --arg policy_sha "$POLICY_SHA256" \
    --arg cargo_lock_sha "$CARGO_LOCK_SHA256" \
    --arg workflow_definition_sha "$WORKFLOW_DEFINITION_SHA256" \
    --arg workflow_run_id "$WORKFLOW_RUN_ID" \
    --argjson workflow_run_attempt "$WORKFLOW_RUN_ATTEMPT" \
    --arg ci_run_id "$CI_RUN_ID" \
    --arg security_run_id "$SECURITY_RUN_ID" \
    --arg android_run_id "$ANDROID_RUN_ID" \
    --arg controller_challenge_sha "$CONTROLLER_CHALLENGE_SHA256" \
    --arg raw_report_sha "$raw_report_sha" \
    --arg artifact_name "$ARTIFACT_NAME" \
    --arg artifact_sha "$ARTIFACT_SHA256" \
    --argjson artifact_bytes "$artifact_bytes" \
    --argjson api_level "$API_LEVEL" \
    --arg security_patch "$SECURITY_PATCH" \
    --arg device_profile "$DEVICE_PROFILE_COMMITMENT" \
    --arg build_fingerprint_sha "$BUILD_FINGERPRINT_SHA256" \
    --arg termux_version "$TERMUX_VERSION" \
    --arg termux_signer_sha "$TERMUX_SIGNER_SHA256" \
    --arg shizuku_version "$SHIZUKU_VERSION" \
    --arg shizuku_signer_sha "$SHIZUKU_SIGNER_SHA256" \
    --argjson adb_shell_uid "$ADB_SHELL_UID" \
    --arg dex_sha "$dex_sha" \
    --argjson dex_bytes "$dex_bytes" \
    --arg observed_rish_state "$OBSERVED_RISH_STATE" '
    {
      schemaVersion: 1,
      gateVersion: $gate_version,
      status: "pass",
      failureCode: null,
      releaseEligible: false,
      productionControlQualified: false,
      qualificationClass: "physical_shizuku_rish_identity_development_v1",
      scope: "s2_5_uid_probe_only",
      repository: $repository,
      commit: $commit,
      version: $version,
      startedAt: $started_at,
      completedAt: $completed_at,
      policySha256: $policy_sha,
      cargoLockSha256: $cargo_lock_sha,
      workflow: {
        name: "Android Rish Physical Identity",
        definitionSha256: $workflow_definition_sha,
        runId: $workflow_run_id,
        runAttempt: $workflow_run_attempt,
        event: "workflow_dispatch",
        protectedEnvironment: "android-rish-physical-development",
        controllerChallengeSha256: $controller_challenge_sha,
        ciRunId: $ci_run_id,
        securityRunId: $security_run_id,
        androidRunId: $android_run_id
      },
      rawReportSha256: $raw_report_sha,
      artifact: {
        artifactName: $artifact_name,
        posture: "android-rish-development",
        features: ["android-rish"],
        target: "aarch64-linux-android",
        sha256: $artifact_sha,
        bytes: $artifact_bytes
      },
      environment: {
        physicalDeviceObserved: true,
        androidFrameworkObserved: true,
        architecture: "aarch64",
        apiLevel: $api_level,
        securityPatch: $security_patch,
        deviceProfileCommitment: $device_profile,
        buildFingerprintSha256: $build_fingerprint_sha,
        termuxVersion: $termux_version,
        termuxSignerSha256: $termux_signer_sha,
        shizukuVersion: $shizuku_version,
        shizukuSignerSha256: $shizuku_signer_sha,
        adbShellUid: $adb_shell_uid,
        shizukuStartModeObserved: false
      },
      backend: {
        name: "shizuku_rish",
        dexSha256: $dex_sha,
        dexBytes: $dex_bytes,
        dexMode: "0400",
        dexParentMode: "0700",
        dexLinkCount: 1,
        dexOwnerMatchesTermuxUid: true,
        dexCanonicalPrivatePath: true,
        principal: "android_shell",
        uid: 2000,
        state: $observed_rish_state,
        rootAccepted: false,
        arbitraryShell: false,
        mutationReady: false
      },
      validation: {
        disabledToolCount: 17,
        enabledToolCount: 18,
        exactToolOrder: true,
        emptyArgumentsSchema: true,
        extraArgumentsRejected: true,
        unknownShellRejected: true,
        allMutationGatesDisabled: true,
        trustedDirectRishProbePreCandidate: true,
        trustedDirectRishProbePostCandidate: true,
        dexTamperRejected: true,
        dexModeRejected: true,
        dexSymlinkRejected: true,
        scenarioResults: [
          {id:"trusted_direct_rish_probe_pre_candidate",execution:"physical",outcome:"pass"},
          {id:"runtime_disabled_tool_absent",execution:"physical",outcome:"pass"},
          {id:"candidate_mcp_status_uid_2000",execution:"physical",outcome:"pass"},
          {id:"extra_arguments_rejected",execution:"physical",outcome:"pass"},
          {id:"unknown_shell_rejected",execution:"physical",outcome:"pass"},
          {id:"dex_tamper_rejected",execution:"physical",outcome:"pass"},
          {id:"dex_mode_rejected",execution:"physical",outcome:"pass"},
          {id:"dex_symlink_rejected",execution:"physical",outcome:"pass"},
          {id:"all_mutation_gates_disabled",execution:"physical",outcome:"pass"},
          {id:"trusted_direct_rish_probe_post_candidate",execution:"physical",outcome:"pass"},
          {id:"bounded_test_fixture_cleanup",execution:"physical",outcome:"pending"}
        ]
      },
      claimBoundary: {
        s3Attestation: false,
        typedReads: false,
        grantV2: false,
        deviceMutation: false,
        productionControl: false,
        sameUidPersistenceExcluded: false,
        continuousNetworkIsolation: false,
        adversarialNetworkIsolation: false
      },
      cleanup: {
        candidateProcessGroupStopped: true,
        portReleased: true,
        deviceFixtureStateRemoved: false,
        controllerTransportRemoved: false
      }
    }
  ' >"$OUTPUT_NEXT" || fail evidence_write_failed
  chmod 600 "$OUTPUT_NEXT" || fail evidence_mode_failed
  if grep -Eq \
    '/data/|Bearer[[:space:]]|MCP__|[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}|localhost|127\.0\.0\.1' \
    "$OUTPUT_NEXT"
  then
    fail evidence_not_sanitized
  fi
  mv -Tn -- "$OUTPUT_NEXT" "$OUTPUT" || fail output_publication_failed
  OUTPUT_NEXT=""
  [[ -f "$OUTPUT" && ! -L "$OUTPUT" && "$(stat -c '%a' "$OUTPUT")" == 600 ]] \
    || fail output_publication_failed
}

main() {
  while (($#)); do
    case "$1" in
      --artifact) (($# >= 2)) || fail missing_artifact; ARTIFACT="$2"; shift 2 ;;
      --artifact-sha256) (($# >= 2)) || fail missing_artifact_digest; ARTIFACT_SHA256="$2"; shift 2 ;;
      --expected-commit) (($# >= 2)) || fail missing_expected_commit; EXPECTED_COMMIT="$2"; shift 2 ;;
      --expected-version) (($# >= 2)) || fail missing_expected_version; EXPECTED_VERSION="$2"; shift 2 ;;
      --policy-sha256) (($# >= 2)) || fail missing_policy_digest; POLICY_SHA256="$2"; shift 2 ;;
      --cargo-lock-sha256) (($# >= 2)) || fail missing_cargo_lock_digest; CARGO_LOCK_SHA256="$2"; shift 2 ;;
      --workflow-definition-sha256) (($# >= 2)) || fail missing_workflow_definition_digest; WORKFLOW_DEFINITION_SHA256="$2"; shift 2 ;;
      --workflow-run-id) (($# >= 2)) || fail missing_workflow_run_id; WORKFLOW_RUN_ID="$2"; shift 2 ;;
      --workflow-run-attempt) (($# >= 2)) || fail missing_workflow_run_attempt; WORKFLOW_RUN_ATTEMPT="$2"; shift 2 ;;
      --ci-run-id) (($# >= 2)) || fail missing_ci_run_id; CI_RUN_ID="$2"; shift 2 ;;
      --security-run-id) (($# >= 2)) || fail missing_security_run_id; SECURITY_RUN_ID="$2"; shift 2 ;;
      --android-run-id) (($# >= 2)) || fail missing_android_run_id; ANDROID_RUN_ID="$2"; shift 2 ;;
      --controller-challenge-sha256) (($# >= 2)) || fail missing_controller_challenge_digest; CONTROLLER_CHALLENGE_SHA256="$2"; shift 2 ;;
      --rish-dex) (($# >= 2)) || fail missing_rish_dex; RISH_DEX="$2"; shift 2 ;;
      --rish-dex-sha256) (($# >= 2)) || fail missing_rish_dex_digest; RISH_DEX_SHA256="$2"; shift 2 ;;
      --api-level) (($# >= 2)) || fail missing_api_level; API_LEVEL="$2"; shift 2 ;;
      --security-patch) (($# >= 2)) || fail missing_security_patch; SECURITY_PATCH="$2"; shift 2 ;;
      --device-profile-commitment) (($# >= 2)) || fail missing_device_profile; DEVICE_PROFILE_COMMITMENT="$2"; shift 2 ;;
      --build-fingerprint-sha256) (($# >= 2)) || fail missing_build_fingerprint_digest; BUILD_FINGERPRINT_SHA256="$2"; shift 2 ;;
      --termux-version) (($# >= 2)) || fail missing_termux_version; TERMUX_VERSION="$2"; shift 2 ;;
      --termux-signer-sha256) (($# >= 2)) || fail missing_termux_signer_digest; TERMUX_SIGNER_SHA256="$2"; shift 2 ;;
      --shizuku-version) (($# >= 2)) || fail missing_shizuku_version; SHIZUKU_VERSION="$2"; shift 2 ;;
      --shizuku-signer-sha256) (($# >= 2)) || fail missing_shizuku_signer_digest; SHIZUKU_SIGNER_SHA256="$2"; shift 2 ;;
      --adb-shell-uid) (($# >= 2)) || fail missing_adb_shell_uid; ADB_SHELL_UID="$2"; shift 2 ;;
      --raw-report) (($# >= 2)) || fail missing_raw_report; RAW_REPORT="$2"; shift 2 ;;
      --output) (($# >= 2)) || fail missing_output; OUTPUT="$2"; shift 2 ;;
      --port) (($# >= 2)) || fail missing_port; REQUESTED_PORT="$2"; shift 2 ;;
      -h|--help) usage; return 0 ;;
      *) fail unknown_argument ;;
    esac
  done

  [[ "$EXPECTED_COMMIT" =~ ^[0-9a-f]{40}$ ]] || fail expected_commit_invalid
  [[ "$EXPECTED_VERSION" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] || fail expected_version_invalid
  is_sha256 "$ARTIFACT_SHA256" || fail artifact_digest_invalid
  is_sha256 "$POLICY_SHA256" || fail policy_digest_invalid
  is_sha256 "$CARGO_LOCK_SHA256" || fail cargo_lock_digest_invalid
  is_sha256 "$WORKFLOW_DEFINITION_SHA256" || fail workflow_definition_digest_invalid
  is_sha256 "$CONTROLLER_CHALLENGE_SHA256" || fail controller_challenge_digest_invalid
  is_sha256 "$RISH_DEX_SHA256" || fail rish_dex_digest_invalid
  is_sha256 "$DEVICE_PROFILE_COMMITMENT" || fail device_profile_invalid
  is_sha256 "$BUILD_FINGERPRINT_SHA256" || fail build_fingerprint_digest_invalid
  is_sha256 "$TERMUX_SIGNER_SHA256" || fail termux_signer_digest_invalid
  is_sha256 "$SHIZUKU_SIGNER_SHA256" || fail shizuku_signer_digest_invalid
  for run_id in "$WORKFLOW_RUN_ID" "$CI_RUN_ID" "$SECURITY_RUN_ID" "$ANDROID_RUN_ID"; do
    [[ "$run_id" =~ ^[1-9][0-9]*$ ]] || fail workflow_run_binding_invalid
  done
  [[ "$WORKFLOW_RUN_ATTEMPT" == 1 ]] || fail workflow_run_attempt_invalid
  [[ "$TERMUX_VERSION" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] \
    || fail termux_version_invalid
  [[ "$SHIZUKU_VERSION" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] \
    || fail shizuku_version_invalid
  [[ "$API_LEVEL" =~ ^[0-9]+$ ]] && ((API_LEVEL >= 30 && API_LEVEL <= 36)) \
    || fail api_level_invalid
  [[ "$SECURITY_PATCH" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || fail security_patch_invalid
  local security_patch_epoch
  security_patch_epoch="$(date -u -d "$SECURITY_PATCH" '+%s' 2>/dev/null)" \
    || fail security_patch_invalid
  [[ "$(date -u -d "@$security_patch_epoch" '+%Y-%m-%d')" == "$SECURITY_PATCH" ]] \
    || fail security_patch_invalid
  [[ "$ADB_SHELL_UID" == 2000 ]] || fail adb_shell_uid_invalid
  if [[ -n "$REQUESTED_PORT" ]]; then
    [[ "$REQUESTED_PORT" =~ ^[0-9]+$ ]] \
      && ((REQUESTED_PORT >= 1024 && REQUESTED_PORT <= 65535)) \
      || fail port_invalid
  fi

  for command_name in awk basename chmod cmp curl date dd dirname env grep id install jq kill ln mktemp mv realpath rm seq setsid sha256sum sleep ss stat timeout uname wc; do
    require_command "$command_name"
  done

  [[ "${HOME:-}" == /data/data/com.termux/files/home ]] || fail termux_home_invalid
  [[ "${PREFIX:-}" == /data/data/com.termux/files/usr ]] || fail termux_prefix_invalid
  [[ "$(uname -m)" == aarch64 || "$(uname -m)" == arm64 ]] || fail architecture_invalid
  [[ -x /system/bin/app_process64 && -x /system/bin/id ]] || fail android_framework_invalid
  [[ "$(id -u)" =~ ^[0-9]+$ ]] && (( $(id -u) >= 10000 )) || fail termux_uid_invalid
  validate_fixed_setsid_program || fail setsid_program_invalid

  is_canonical_private_regular_file "$ARTIFACT" 700 67108864 \
    || fail artifact_invalid
  [[ -x "$ARTIFACT" ]] || fail artifact_invalid
  validate_private_parent "$ARTIFACT" || fail artifact_parent_invalid
  [[ "$(sha256sum -- "$ARTIFACT" | awk '{print $1}')" == "$ARTIFACT_SHA256" ]] \
    || fail artifact_digest_mismatch
  is_canonical_private_regular_file "$RISH_DEX" 400 16777216 \
    || fail rish_dex_invalid
  validate_private_parent "$RISH_DEX" || fail rish_dex_parent_invalid
  [[ "$(sha256sum -- "$RISH_DEX" | awk '{print $1}')" == "$RISH_DEX_SHA256" ]] \
    || fail rish_dex_digest_mismatch
  [[ "$OUTPUT" == /* && ! -e "$OUTPUT" && ! -L "$OUTPUT" ]] || fail output_invalid
  validate_private_parent "$OUTPUT" || fail output_parent_invalid
  [[ "$RAW_REPORT" == /* && ! -e "$RAW_REPORT" && ! -L "$RAW_REPORT" ]] \
    || fail raw_report_invalid
  validate_private_parent "$RAW_REPORT" || fail raw_report_parent_invalid
  [[ "$RAW_REPORT" != "$OUTPUT" ]] || fail output_paths_not_distinct
  STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)" || fail timestamp_failed

  WORK_ROOT="$(mktemp -d "$HOME/.termux-mcp-rish-physical.XXXXXX")" \
    || fail work_root_create_failed
  chmod 700 "$WORK_ROOT" || fail work_root_mode_failed
  SAFE_ROOT="$WORK_ROOT/safe-root"
  DEX_ROOT="$WORK_ROOT/rish"
  install -d -m 700 "$SAFE_ROOT" "$DEX_ROOT" || fail private_state_create_failed
  PINNED_DEX="$DEX_ROOT/rish_shizuku.dex"
  install -m 400 -- "$RISH_DEX" "$PINNED_DEX" || fail rish_dex_snapshot_failed
  [[ "$(sha256sum -- "$PINNED_DEX" | awk '{print $1}')" == "$RISH_DEX_SHA256" ]] \
    || fail rish_dex_snapshot_changed
  MCP_TOKEN="$(dd if=/dev/urandom bs=32 count=1 status=none | sha256sum | awk '{print $1}')"
  is_sha256 "$MCP_TOKEN" || fail token_generation_failed
  PORT="$(choose_port)" || fail port_unavailable
  trap cleanup EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM HUP
  trusted_direct_rish_probe pre_candidate \
    || fail trusted_direct_rish_probe_pre_candidate_failed

  SERVER_LOG="$WORK_ROOT/rejection-server.log"
  local wrong_digest mode_dex symlink_dex
  wrong_digest="$RISH_DEX_SHA256"
  if [[ "${wrong_digest:0:1}" == 0 ]]; then
    wrong_digest="1${wrong_digest:1}"
  else
    wrong_digest="0${wrong_digest:1}"
  fi
  expect_startup_rejection "$PINNED_DEX" "$wrong_digest" \
    || fail wrong_digest_not_rejected

  mode_dex="$DEX_ROOT/writable.dex"
  install -m 600 -- "$RISH_DEX" "$mode_dex" || fail mode_fixture_create_failed
  expect_startup_rejection "$mode_dex" "$RISH_DEX_SHA256" \
    || fail writable_dex_not_rejected
  rm -f -- "$mode_dex" || fail mode_fixture_cleanup_failed

  symlink_dex="$DEX_ROOT/symlink.dex"
  ln -s -- "$PINNED_DEX" "$symlink_dex" || fail symlink_fixture_create_failed
  expect_startup_rejection "$symlink_dex" "$RISH_DEX_SHA256" \
    || fail symlink_dex_not_rejected
  rm -f -- "$symlink_dex" || fail symlink_fixture_cleanup_failed

  validate_disabled_posture
  validate_enabled_posture
  trusted_direct_rish_probe post_candidate \
    || fail trusted_direct_rish_probe_post_candidate_failed
  port_is_free "$PORT" || fail final_port_not_released
  COMPLETED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)" || fail timestamp_failed
  local started_epoch completed_epoch
  started_epoch="$(date -u -d "$STARTED_AT" '+%s' 2>/dev/null)" || fail timestamp_failed
  completed_epoch="$(date -u -d "$COMPLETED_AT" '+%s' 2>/dev/null)" || fail timestamp_failed
  ((completed_epoch >= started_epoch && completed_epoch - started_epoch <= 1800)) \
    || fail qualification_duration_invalid
  ((security_patch_epoch <= completed_epoch)) || fail security_patch_in_future
  write_raw_report
  write_evidence
  printf 'TERMUX_RISH_PHYSICAL_GATE_RESULT=PASS\n'
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
