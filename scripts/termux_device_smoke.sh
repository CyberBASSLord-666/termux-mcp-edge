#!/data/data/com.termux/files/usr/bin/bash
set -Eeuo pipefail
IFS=$'\n\t'
umask 077
set +x

usage() {
  cat <<'EOF'
Usage:
  TERMUX_MCP_SMOKE_EXPECTED_HEAD=<40-character-commit-sha> \
    bash scripts/termux_device_smoke.sh

Optional environment:
  TERMUX_MCP_SMOKE_FETCH_REF=<git-ref>          Fetch this ref, then require the exact SHA.
  TERMUX_MCP_SMOKE_BUILD_JOBS=<positive-int>    Cargo build jobs (default: 2).
  TERMUX_MCP_SMOKE_CARGO_TARGET_DIR=<path>      Reuse a target directory beneath HOME.
  TERMUX_MCP_SMOKE_SKIP_PACKAGE_BOOTSTRAP=true  Skip pkg update/install.
  TERMUX_MCP_SMOKE_UPGRADE_PACKAGES=false       Skip pkg upgrade during bootstrap.
  TERMUX_MCP_SMOKE_CI_EVIDENCE=<url-or-run-id>  Record companion exact-head CI evidence.

The harness creates isolated deployment, configuration, safe-root, and runsvdir
directories. It preserves its report, source checkout, build artifacts, and logs
under HOME. It removes isolated live state only after service shutdown is
confirmed; otherwise it fails and preserves that state for manual recovery.
EOF
}

case "$#" in
  0) ;;
  1)
    case "$1" in
      -h|--help) usage; exit 0 ;;
      *) usage >&2; exit 2 ;;
    esac
    ;;
  *) usage >&2; exit 2 ;;
esac

EXPECTED_HEAD="${TERMUX_MCP_SMOKE_EXPECTED_HEAD:-}"
if [[ ! "$EXPECTED_HEAD" =~ ^[0-9a-f]{40}$ ]]; then
  printf '%s\n' 'TERMUX_MCP_SMOKE_EXPECTED_HEAD must be a full lowercase 40-character commit SHA.' >&2
  exit 2
fi

FETCH_REF="${TERMUX_MCP_SMOKE_FETCH_REF:-$EXPECTED_HEAD}"
if [[ ! "$FETCH_REF" =~ ^[A-Za-z0-9._/-]+$ || "$FETCH_REF" == -* ]]; then
  printf '%s\n' 'TERMUX_MCP_SMOKE_FETCH_REF contains unsupported characters.' >&2
  exit 2
fi

BUILD_JOBS="${TERMUX_MCP_SMOKE_BUILD_JOBS:-2}"
if [[ ! "$BUILD_JOBS" =~ ^[1-9][0-9]*$ ]]; then
  printf '%s\n' 'TERMUX_MCP_SMOKE_BUILD_JOBS must be a positive integer.' >&2
  exit 2
fi

REPOSITORY_URL="https://github.com/CyberBASSLord-666/termux-mcp-edge.git"
HARNESS_VERSION="2"
HEAD_LABEL="${EXPECTED_HEAD:0:12}"
SMOKE_ID="$(date -u +%Y%m%dT%H%M%SZ)-$$"
WORK_ROOT="$HOME/termux-mcp-device-smoke-$HEAD_LABEL-$SMOKE_ID"
REPO_DIR="$WORK_ROOT/repository"
ARTIFACT_DIR="$WORK_ROOT/artifacts"
LOG_DIR="$WORK_ROOT/logs"
REPORT="$HOME/termux-mcp-device-report-$HEAD_LABEL-$SMOKE_ID.txt"
PACKAGE_LOG="$LOG_DIR/packages.log"
BUILD_LOG="$LOG_DIR/build.log"
RUNSVDIR_LOG="$LOG_DIR/runsvdir.log"
ORIGINAL_PATH="$PATH"
TERMUX_PREFIX_INITIAL="${PREFIX:-}"
REQUESTED_CARGO_TARGET_DIR="${TERMUX_MCP_SMOKE_CARGO_TARGET_DIR:-}"
SKIP_PACKAGE_BOOTSTRAP="${TERMUX_MCP_SMOKE_SKIP_PACKAGE_BOOTSTRAP:-false}"
UPGRADE_PACKAGES="${TERMUX_MCP_SMOKE_UPGRADE_PACKAGES:-true}"
CI_EVIDENCE="${TERMUX_MCP_SMOKE_CI_EVIDENCE:-not-supplied}"

DEPLOY_ROOT=""
CONFIG_ROOT=""
SERVICE_ROOT=""
SERVICE_DIR=""
SAFE_ROOT=""
RUNSVDIR_PID=""
MCP_TOKEN=""
MCP_SESSION_ID=""
CAPABILITY_KEY_ID="device-smoke-1"
CAPABILITY_KEY_HEX=""
CAPABILITY_GRANT_FILE=""
SMOKE_SUCCEEDED=0

mkdir -p -- "$WORK_ROOT" "$ARTIFACT_DIR" "$LOG_DIR"
touch "$REPORT"
chmod 600 "$REPORT"

log() {
  printf '%s\n' "$*" | tee -a "$REPORT"
}

fail() {
  log "TERMUX_MCP_DEVICE_RESULT=FAIL"
  log "failure=$*"
  exit 1
}

is_true() {
  case "$1" in
    1|true|TRUE|yes|YES) return 0 ;;
    0|false|FALSE|no|NO) return 1 ;;
    *) fail "boolean setting has an unsupported value" ;;
  esac
}

safe_cleanup_roots() {
  if [[ -n "$DEPLOY_ROOT" && "$DEPLOY_ROOT" == "$HOME"/.local/share/termux-mcp-device-smoke-* ]]; then
    rm -rf -- "$DEPLOY_ROOT"
  fi
  if [[ -n "$CONFIG_ROOT" && "$CONFIG_ROOT" == "$HOME"/.config/termux-mcp-device-smoke-* ]]; then
    rm -rf -- "$CONFIG_ROOT"
  fi
  if [[ -n "$SAFE_ROOT" && "$SAFE_ROOT" == "$HOME"/mcp-files-device-smoke-* ]]; then
    rm -rf -- "$SAFE_ROOT"
  fi
  if [[ -n "$SERVICE_ROOT" && -n "$TERMUX_PREFIX_INITIAL" && "$SERVICE_ROOT" == "$TERMUX_PREFIX_INITIAL"/var/service-termux-mcp-device-smoke-* ]]; then
    rm -rf -- "$SERVICE_ROOT"
  fi
}

cleanup_roots_absent() {
  local path
  for path in "$DEPLOY_ROOT" "$CONFIG_ROOT" "$SAFE_ROOT" "$SERVICE_ROOT"; do
    [[ -z "$path" || (! -e "$path" && ! -L "$path") ]] || return 1
  done
}

cleanup() {
  local status=$?
  local cleanup_confirmed=1
  trap - EXIT INT TERM HUP
  set +e
  PATH="$ORIGINAL_PATH"
  if [[ -n "$SERVICE_DIR" && -d "$SERVICE_DIR" ]] && command -v sv >/dev/null 2>&1; then
    if ! sv down "$SERVICE_DIR" >/dev/null 2>&1; then
      cleanup_confirmed=0
      log "cleanup_service_shutdown=unconfirmed"
    fi
  fi
  if [[ -n "$RUNSVDIR_PID" ]]; then
    kill "$RUNSVDIR_PID" >/dev/null 2>&1 || true
    wait "$RUNSVDIR_PID" >/dev/null 2>&1 || true
  fi
  unset MCP_TOKEN MCP_SESSION_ID CAPABILITY_KEY_HEX MCP__AUTH__STATIC_TOKEN TOKEN 2>/dev/null || true
  if ((cleanup_confirmed == 1)); then
    safe_cleanup_roots
    cleanup_roots_absent || cleanup_confirmed=0
  fi
  if ((cleanup_confirmed == 1)); then
    log "cleanup_complete=true"
  else
    status=1
    log "cleanup_complete=false"
    log "cleanup_isolated_state=preserved_for_manual_recovery"
  fi
  log "report=$REPORT"
  log "work_root=$WORK_ROOT"
  if ((status == 0 && SMOKE_SUCCEEDED == 1)); then
    log "final_status=PASS"
  else
    log "final_status=FAIL"
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "required command is unavailable after bootstrap: $1"
}

run_success() {
  local label="$1"
  shift
  local output="$LOG_DIR/$label.log" status
  set +e
  "$@" >"$output" 2>&1
  status=$?
  set -e
  log "--- $label ---"
  tee -a "$REPORT" <"$output"
  log "${label}_rc=$status"
  ((status == 0)) || fail "$label returned $status; expected success"
}

run_failure() {
  local label="$1"
  shift
  local output="$LOG_DIR/$label.log" status
  set +e
  "$@" >"$output" 2>&1
  status=$?
  set -e
  log "--- $label ---"
  tee -a "$REPORT" <"$output"
  log "${label}_rc=$status"
  ((status != 0)) || fail "$label unexpectedly succeeded"
}

assert_eq() {
  local label="$1" actual="$2" expected="$3"
  [[ "$actual" == "$expected" ]] || fail "$label expected '$expected' but got '$actual'"
  log "PASS $label=$actual"
}

assert_exists() {
  local label="$1" path="$2"
  [[ -e "$path" || -L "$path" ]] || fail "$label is missing"
  log "PASS $label=present"
}

assert_absent() {
  local label="$1" path="$2"
  [[ ! -e "$path" && ! -L "$path" ]] || fail "$label unexpectedly exists"
  log "PASS $label=absent"
}

assert_json() {
  local label="$1" file="$2" filter="$3"
  jq -e "$filter" "$file" >/dev/null || fail "$label JSON assertion failed"
  log "PASS $label=valid"
}

link_value() {
  local path="$1"
  if [[ -L "$path" ]]; then
    readlink "$path"
  else
    printf 'none\n'
  fi
}

file_sha() {
  sha256sum -- "$1" | awk '{print $1}'
}

wait_for_runit() {
  local expected="$1" attempt output=""
  for attempt in $(seq 1 40); do
    output="$(sv status "$SERVICE_DIR" 2>&1 || true)"
    case "$expected" in
      run) [[ "$output" == run:* ]] && { log "PASS runit_status=$output"; return 0; } ;;
      down) [[ "$output" == down:* ]] && { log "PASS runit_status=$output"; return 0; } ;;
    esac
    sleep 0.25
  done
  fail "runit did not reach $expected state; last status: $output"
}

wait_for_http() {
  local attempt health="" ready=""
  for attempt in $(seq 1 40); do
    health="$(curl -fsS --max-time 2 "$TERMUX_MCP_HEALTH_URL" 2>/dev/null || true)"
    ready="$(curl -fsS --max-time 2 "$TERMUX_MCP_READY_URL" 2>/dev/null || true)"
    if [[ "$health" == ok ]] && jq -e '.status == "ready"' <<<"$ready" >/dev/null 2>&1; then
      log "PASS health=ok"
      log "PASS readiness=ready"
      return 0
    fi
    sleep 0.25
  done
  fail "runtime did not become healthy and ready"
}

assert_running_state() {
  wait_for_runit run
  wait_for_http
  assert_absent service_down_marker "$SERVICE_DIR/down"
}

choose_port() {
  local port
  for port in $(seq 18765 18864); do
    if ! ss -ltnH 2>/dev/null | awk '{print $4}' | grep -Eq ":${port}$"; then
      printf '%s\n' "$port"
      return 0
    fi
  done
  return 1
}

valid_capability_grant() {
  local grant="$1" prefix remainder payload signature
  prefix="v1.${CAPABILITY_KEY_ID}."
  [[ "$grant" == "$prefix"* ]] || return 1
  remainder="${grant#"$prefix"}"
  [[ "$remainder" == *.* ]] || return 1
  payload="${remainder%%.*}"
  signature="${remainder#*.}"
  [[ "$signature" != *.* ]] || return 1
  ((${#payload} == 260 && ${#signature} == 64)) || return 1
  [[ "$payload$signature" != *[!0-9a-f]* ]]
}

mcp_post() {
  local output="$1" payload="$2" session_id="${3:-}" grant_file="${4:-}" grant=""
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
  if [[ -n "$session_id" ]]; then
    args+=(
      -H 'MCP-Protocol-Version: 2025-11-25'
      -H "MCP-Session-Id: $session_id"
    )
  fi
  if [[ -n "$grant_file" ]]; then
    [[ -f "$grant_file" && ! -L "$grant_file" && "$(stat -c '%a' "$grant_file")" == 600 ]] || fail "capability grant staging is invalid"
    grant="$(<"$grant_file")"
    valid_capability_grant "$grant" || fail "candidate emitted an invalid capability grant"
    args+=( -H "MCP-Capability-Grant: $grant" )
  fi
  curl "${args[@]}" --data-binary "$payload" "$MCP_URL"
}

issue_create_directory_grant() {
  local target="$1" grant=""
  : >"$CAPABILITY_GRANT_FILE"
  chmod 600 "$CAPABILITY_GRANT_FILE"
  if ! MCP__CAPABILITY__CONFIG_FILE="$CONFIG_ROOT/runtime.env" \
    MCP__CAPABILITY__SESSION_ID="$MCP_SESSION_ID" \
    MCP__CAPABILITY__CREATE_DIRECTORY_TARGET="$target" \
      "$CANDIDATE_ARTIFACT" --issue-create-directory-grant >"$CAPABILITY_GRANT_FILE" 2>/dev/null
  then
    fail "exact candidate could not issue a create_directory grant"
  fi
  [[ "$(wc -l <"$CAPABILITY_GRANT_FILE")" == 1 ]] || fail "candidate emitted an invalid capability grant"
  grant="$(<"$CAPABILITY_GRANT_FILE")"
  valid_capability_grant "$grant" || fail "candidate emitted an invalid capability grant"
  unset grant
}

protocol_smoke() {
  local label="$1"
  local body headers status payload target outside oversized copy_source copy_target copy_bytes directory_target
  headers="$LOG_DIR/$label-initialize.headers"
  body="$LOG_DIR/$label-response.json"

  payload='{"jsonrpc":"2.0","id":"unauthorized","method":"tools/list"}'
  status="$(curl -sS -o "$body" -w '%{http_code}' \
    -H "Host: localhost:$PORT" \
    -H "Origin: http://localhost:$PORT" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    --data-binary "$payload" "$MCP_URL")"
  assert_eq "${label}_unauthorized_http" "$status" 401
  assert_json "${label}_unauthorized_body" "$body" '.error == "unauthorized" and (.result | not)'

  payload='{"jsonrpc":"2.0","id":"initialize","method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"termux-device-smoke","version":"1.0.0"}}}'
  status="$(curl -sS -D "$headers" -o "$body" -w '%{http_code}' \
    -H "Authorization: Bearer $MCP_TOKEN" \
    -H "Host: localhost:$PORT" \
    -H "Origin: http://localhost:$PORT" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    --data-binary "$payload" "$MCP_URL")"
  assert_eq "${label}_initialize_http" "$status" 200
  assert_json "${label}_initialize_body" "$body" '.result.protocolVersion == "2025-11-25"'
  MCP_SESSION_ID="$(awk 'tolower($1) == "mcp-session-id:" {sub(/^[^:]*:[[:space:]]*/, ""); sub(/\r$/, ""); print; exit}' "$headers")"
  [[ "$MCP_SESSION_ID" =~ ^[A-Za-z0-9-]{1,128}$ ]] || fail "initialize response did not contain a bounded MCP session ID"
  log "PASS ${label}_session_id=present"

  payload='{"jsonrpc":"2.0","method":"notifications/initialized"}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_initialized_notification_http" "$status" 202
  [[ ! -s "$body" ]] || fail "initialized notification returned a response body"
  log "PASS ${label}_initialized_notification_body=empty"

  payload='{"jsonrpc":"2.0","id":"tools-list","method":"tools/list"}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_tools_list_http" "$status" 200
  assert_json "${label}_tool_allowlist" "$body" '[.result.tools[].name] == ["runtime_status","platform_info","android_status","project_service_status","create_directory","copy_file","list_directory","path_metadata","read_file","search_text","write_file"]'
  assert_json "${label}_create_directory_grant_discovery" "$body" '.result.tools | map(select(.name == "create_directory"))[0] as $tool | ($tool.inputSchema.properties.dry_run | has("const") | not) and ($tool.description | contains("MCP-Capability-Grant"))'

  payload='{"jsonrpc":"2.0","id":"runtime-status","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_runtime_status_http" "$status" 200
  assert_json "${label}_high_impact_disabled" "$body" '.result.structuredContent.commandExecution == false and .result.structuredContent.androidPlatformTools == false and .result.structuredContent.highImpactTools == false and .result.structuredContent.createDirectoryMutationEnabled == true and .result.structuredContent.createDirectoryGrantRequired == true and .result.structuredContent.createDirectoryGrantHeader == "mcp-capability-grant" and .result.structuredContent.createDirectoryGrantTtlSeconds == 60'

  payload="$(jq -cn --arg path "$SAFE_ROOT" '{"jsonrpc":"2.0","id":"list-directory","method":"tools/call","params":{"name":"list_directory","arguments":{"path":$path,"max_depth":1}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_list_directory_http" "$status" 200
  jq -e --arg expected "$SAFE_ROOT/visible.txt" '.result.structuredContent.entries | any(.path == $expected)' "$body" >/dev/null || fail "safe-root listing omitted the expected file"
  log "PASS ${label}_list_directory=expected-file"

  payload="$(jq -cn --arg path "$SAFE_ROOT/visible.txt" '{"jsonrpc":"2.0","id":"path-metadata","method":"tools/call","params":{"name":"path_metadata","arguments":{"path":$path}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_path_metadata_http" "$status" 200
  jq -e --arg expected "$SAFE_ROOT/visible.txt" '
    .result.structuredContent as $metadata
    | ($metadata | keys) == ["kind","maxResponseBytes","modified","path","sizeBytes"]
      and $metadata.path == $expected
      and $metadata.kind == "regular_file"
      and $metadata.sizeBytes == 20
      and ($metadata.modified | type) == "string"
      and $metadata.maxResponseBytes == 16384
  ' "$body" >/dev/null || fail "${label}_path_metadata_result JSON assertion failed"
  grep -Eq 'inode|device|uid|gid|mode|accessTime|device-smoke-visible' "$body" && fail "metadata response reflected a denied field or file content"
  log "PASS ${label}_path_metadata=valid"

  payload="$(jq -cn --arg path "$SAFE_ROOT/visible.txt" '{"jsonrpc":"2.0","id":"read-file","method":"tools/call","params":{"name":"read_file","arguments":{"path":$path}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_read_file_http" "$status" 200
  assert_json "${label}_read_file_content" "$body" '.result.structuredContent.content == "device-smoke-visible"'

  payload="$(jq -cn --arg path "$SAFE_ROOT" --arg query device-smoke-visible '{"jsonrpc":"2.0","id":"search-text","method":"tools/call","params":{"name":"search_text","arguments":{"path":$path,"query":$query,"max_depth":1}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_search_text_http" "$status" 200
  jq -e --arg expected "$SAFE_ROOT/visible.txt" '.result.structuredContent.matches == [{"path":$expected,"lineNumber":1,"columnByte":1}] and .result.structuredContent.truncated == false' "$body" >/dev/null || fail "${label}_search_text_result JSON assertion failed"
  log "PASS ${label}_search_text_result=valid"
  grep -Fq device-smoke-visible "$body" && fail "search response reflected query or file content"
  log "PASS ${label}_search_text_content=redacted"

  directory_target="$SAFE_ROOT/created-directory"
  payload="$(jq -cn --arg path "$directory_target" '{"jsonrpc":"2.0","id":"create-directory-missing-grant","method":"tools/call","params":{"name":"create_directory","arguments":{"path":$path,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_create_directory_missing_grant_http" "$status" 403
  assert_json "${label}_create_directory_missing_grant_body" "$body" '.error.code == -32003 and .error.data.reason == "capability_grant_missing"'
  assert_absent "${label}_create_directory_missing_grant_target" "$directory_target"

  issue_create_directory_grant "$directory_target"
  payload="$(jq -cn --arg path "$directory_target" '{"jsonrpc":"2.0","id":"create-directory-dry","method":"tools/call","params":{"name":"create_directory","arguments":{"path":$path}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" "$CAPABILITY_GRANT_FILE")"
  assert_eq "${label}_create_directory_dry_run_http" "$status" 200
  assert_json "${label}_create_directory_dry_run_body" "$body" '.result.structuredContent.dryRun == true and .result.structuredContent.mode == "0700" and .result.structuredContent.maxResponseBytes == 16384'
  assert_absent "${label}_create_directory_dry_run_target" "$directory_target"

  payload="$(jq -cn --arg path "$directory_target" '{"jsonrpc":"2.0","id":"create-directory","method":"tools/call","params":{"name":"create_directory","arguments":{"path":$path,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" "$CAPABILITY_GRANT_FILE")"
  assert_eq "${label}_create_directory_http" "$status" 200
  assert_json "${label}_create_directory_body" "$body" '.result.structuredContent.dryRun == false and .result.structuredContent.mode == "0700"'
  [[ -d "$directory_target" ]] || fail "explicit create_directory call did not create its target"
  log "PASS ${label}_create_directory_target=directory"
  assert_eq "${label}_create_directory_mode" "$(stat -c '%a' "$directory_target")" 700

  rmdir -- "$directory_target" || fail "could not prepare the create_directory replay check"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" "$CAPABILITY_GRANT_FILE")"
  assert_eq "${label}_create_directory_replay_http" "$status" 403
  assert_json "${label}_create_directory_replay_body" "$body" '.error.code == -32003 and .error.data.reason == "capability_grant_replayed"'
  assert_absent "${label}_create_directory_replay_target" "$directory_target"

  copy_source="$SAFE_ROOT/copy-source.bin"
  copy_target="$SAFE_ROOT/copy-target.bin"
  printf 'device-smoke-copy\000\377binary' >"$copy_source"
  chmod 777 "$copy_source"
  copy_bytes="$(wc -c <"$copy_source")"
  payload="$(jq -cn --arg source "$copy_source" --arg destination "$copy_target" '{"jsonrpc":"2.0","id":"copy-dry-run","method":"tools/call","params":{"name":"copy_file","arguments":{"source_path":$source,"destination_path":$destination}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_copy_dry_run_http" "$status" 200
  assert_json "${label}_copy_dry_run_body" "$body" ".result.structuredContent.dryRun == true and .result.structuredContent.sizeBytes == $copy_bytes and .result.structuredContent.mode == \"0600\" and .result.structuredContent.maxFileBytes == 1048576 and .result.structuredContent.maxResponseBytes == 16384"
  grep -Fq device-smoke-copy "$body" && fail "copy_file dry-run response reflected file content"
  assert_absent "${label}_copy_dry_run_target" "$copy_target"

  payload="$(jq -cn --arg source "$copy_source" --arg destination "$copy_target" '{"jsonrpc":"2.0","id":"copy-explicit","method":"tools/call","params":{"name":"copy_file","arguments":{"source_path":$source,"destination_path":$destination,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_copy_explicit_http" "$status" 200
  assert_json "${label}_copy_explicit_body" "$body" ".result.structuredContent.dryRun == false and .result.structuredContent.sizeBytes == $copy_bytes and .result.structuredContent.mode == \"0600\""
  grep -Fq device-smoke-copy "$body" && fail "copy_file response reflected file content"
  cmp -s "$copy_source" "$copy_target" || fail "copy_file did not preserve exact binary content"
  log "PASS ${label}_copy_content=exact"
  assert_eq "${label}_copy_mode" "$(stat -c '%a' "$copy_target")" 600

  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_copy_existing_http" "$status" 400
  assert_json "${label}_copy_existing_body" "$body" '.error.code == -32602'
  cmp -s "$copy_source" "$copy_target" || fail "copy_file existing-destination denial modified content"
  log "PASS ${label}_copy_existing=unchanged"

  target="$SAFE_ROOT/write-target.txt"
  payload="$(jq -cn --arg path "$target" --arg content 'device-smoke-write' '{"jsonrpc":"2.0","id":"write-dry-run","method":"tools/call","params":{"name":"write_file","arguments":{"path":$path,"content":$content}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_write_dry_run_http" "$status" 200
  assert_json "${label}_write_dry_run_body" "$body" '.result.structuredContent.dryRun == true'
  assert_absent "${label}_write_dry_run_target" "$target"

  payload="$(jq -cn --arg path "$target" --arg content 'device-smoke-write' '{"jsonrpc":"2.0","id":"write-explicit","method":"tools/call","params":{"name":"write_file","arguments":{"path":$path,"content":$content,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_write_explicit_http" "$status" 200
  assert_json "${label}_write_explicit_body" "$body" '.result.structuredContent.dryRun == false and .result.structuredContent.bytes == 18'
  assert_eq "${label}_write_content" "$(<"$target")" "device-smoke-write"
  assert_eq "${label}_write_mode" "$(stat -c '%a' "$target")" 600

  outside="$WORK_ROOT/outside-secret.txt"
  printf '%s' 'outside-secret-must-not-be-returned' >"$outside"
  payload="$(jq -cn --arg path "$outside" '{"jsonrpc":"2.0","id":"outside-read","method":"tools/call","params":{"name":"read_file","arguments":{"path":$path}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_outside_read_http" "$status" 400
  assert_json "${label}_outside_read_denied" "$body" '.error.code == -32602'
  if grep -Fq 'outside-secret-must-not-be-returned' "$body"; then
    fail "denied outside-root read reflected file content"
  fi
  log "PASS ${label}_outside_read_content=redacted"

  payload='{"jsonrpc":"2.0","id":"forbidden-shell","method":"tools/call","params":{"name":"shell","arguments":{}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_forbidden_tool_http" "$status" 501
  assert_json "${label}_forbidden_tool_body" "$body" '.error.code == -32601 and .error.message == "Method not found"'

  oversized="$(printf '%*s' 1500 '' | tr ' ' x)"
  payload="$(jq -cn --arg content "$oversized" '{"jsonrpc":"2.0","id":"oversized","method":"tools/call","params":{"name":"write_file","arguments":{"path":"/ignored","content":$content}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_authenticated_oversized_http" "$status" 413
  assert_json "${label}_authenticated_oversized_body" "$body" '.error == "mcp_request_body_too_large"'

  status="$(curl -sS -o "$body" -w '%{http_code}' \
    -H "Host: localhost:$PORT" \
    -H "Origin: http://localhost:$PORT" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    --data-binary "$payload" "$MCP_URL")"
  assert_eq "${label}_unauthenticated_oversized_http" "$status" 401
  assert_json "${label}_unauthenticated_oversized_body" "$body" '.error == "unauthorized"'

  status="$(curl -sS -X DELETE -o "$body" -w '%{http_code}' \
    -H "Authorization: Bearer $MCP_TOKEN" \
    -H "Host: localhost:$PORT" \
    -H "Origin: http://localhost:$PORT" \
    -H 'MCP-Protocol-Version: 2025-11-25' \
    -H "MCP-Session-Id: $MCP_SESSION_ID" \
    "$MCP_URL")"
  assert_eq "${label}_delete_session_http" "$status" 204
  [[ ! -s "$body" ]] || fail "session deletion returned a response body"
  log "PASS ${label}_delete_session_body=empty"
  MCP_SESSION_ID=""
}

log "Termux MCP exact-commit device production gate starting"
log "harness_version=$HARNESS_VERSION"
log "expected_head=$EXPECTED_HEAD"
log "fetch_ref=$FETCH_REF"
log "smoke_id=$SMOKE_ID"
log "report=$REPORT"
log "work_root=$WORK_ROOT"
if [[ "$CI_EVIDENCE" == *$'\n'* || "$CI_EVIDENCE" == *$'\r'* ]]; then
  fail "TERMUX_MCP_SMOKE_CI_EVIDENCE must be a single line"
fi
log "ci_evidence=$CI_EVIDENCE"

command -v pkg >/dev/null 2>&1 || fail "this script must run inside Termux with the pkg command available"
[[ -n "$TERMUX_PREFIX_INITIAL" && "$TERMUX_PREFIX_INITIAL" == /data/data/*/files/usr ]] || fail "PREFIX does not look like a Termux prefix"

if ! is_true "$SKIP_PACKAGE_BOOTSTRAP"; then
  log "Installing required Termux packages; detailed output is in $PACKAGE_LOG"
  set +e
  pkg update -y >"$PACKAGE_LOG" 2>&1
  package_status=$?
  if ((package_status == 0)) && is_true "$UPGRADE_PACKAGES"; then
    pkg upgrade -y >>"$PACKAGE_LOG" 2>&1
    package_status=$?
  fi
  if ((package_status == 0)); then
    pkg install -y git bash coreutils curl file gawk grep jq sed termux-services rust clang make pkg-config binutils iproute2 >>"$PACKAGE_LOG" 2>&1
    package_status=$?
  fi
  set -e
  if ((package_status != 0)); then
    tail -n 80 "$PACKAGE_LOG" | tee -a "$REPORT"
    fail "Termux package bootstrap failed with status $package_status"
  fi
else
  log "package_bootstrap=skipped"
fi

for command_name in git bash cargo rustc clang sv runsvdir awk base64 curl file grep install jq realpath sed seq sha256sum stat tee timeout tr ss; do
  require_command "$command_name"
done

AVAILABLE_KB="$(df -Pk "$HOME" | awk 'NR==2 {print $4}')"
log "available_home_kb=$AVAILABLE_KB"
if [[ "$AVAILABLE_KB" =~ ^[0-9]+$ ]] && ((AVAILABLE_KB < 1572864)); then
  fail "at least 1.5 GiB of free space is required for the two Rust builds"
fi

log "architecture=$(uname -m)"
case "$(uname -m)" in
  aarch64|arm64) ;;
  *) fail "the production gate requires an AArch64 Termux device" ;;
esac
log "rustc=$(rustc --version)"
log "cargo=$(cargo --version)"
log "clang=$(clang --version | head -n 1)"

mkdir -p -- "$REPO_DIR"
git -C "$REPO_DIR" init -q
git -C "$REPO_DIR" remote add origin "$REPOSITORY_URL"
log "Fetching exact release-candidate source"
if ! git -C "$REPO_DIR" fetch --depth=1 origin "$FETCH_REF" >"$LOG_DIR/git-fetch.log" 2>&1; then
  tee -a "$REPORT" <"$LOG_DIR/git-fetch.log"
  fail "unable to fetch the requested Git ref"
fi
git -C "$REPO_DIR" checkout -q --detach FETCH_HEAD
ACTUAL_HEAD="$(git -C "$REPO_DIR" rev-parse HEAD)"
assert_eq exact_git_head "$ACTUAL_HEAD" "$EXPECTED_HEAD"
[[ -z "$(git -C "$REPO_DIR" status --porcelain)" ]] || fail "fresh exact-head checkout is unexpectedly dirty"

if [[ -n "$REQUESTED_CARGO_TARGET_DIR" ]]; then
  [[ "$REQUESTED_CARGO_TARGET_DIR" == "$HOME"/* ]] || fail "TERMUX_MCP_SMOKE_CARGO_TARGET_DIR must be an absolute path beneath HOME"
  CARGO_TARGET_DIR="$REQUESTED_CARGO_TARGET_DIR"
else
  CARGO_TARGET_DIR="$REPO_DIR/target"
fi
mkdir -p -- "$CARGO_TARGET_DIR"
CARGO_TARGET_DIR="$(realpath "$CARGO_TARGET_DIR")"
[[ "$CARGO_TARGET_DIR" == "$HOME"/* ]] || fail "resolved Cargo target directory escapes HOME"
export CARGO_TARGET_DIR
log "cargo_target_dir=$CARGO_TARGET_DIR"

DEPLOY_SCRIPT="$REPO_DIR/scripts/termux_deploy.sh"
bash -n "$DEPLOY_SCRIPT"
log "PASS deploy_script_syntax=valid"

cd "$REPO_DIR"
CANDIDATE_VERSION="$(awk '
  /^\[package\]$/ { in_package=1; next }
  in_package && /^\[/ { exit }
  in_package && /^version = "/ {
    sub(/^version = "/, "")
    sub(/"$/, "")
    print
    exit
  }
' Cargo.toml)"
[[ -n "$CANDIDATE_VERSION" ]] || fail "could not read the package version"
BASELINE_VERSION="0.0.0-device-smoke.$HEAD_LABEL"
BASELINE_ARTIFACT="$ARTIFACT_DIR/termux-mcp-server-$BASELINE_VERSION"
CANDIDATE_ARTIFACT="$ARTIFACT_DIR/termux-mcp-server-$CANDIDATE_VERSION"

log "Building baseline and exact candidate; detailed output is in $BUILD_LOG"
: >"$BUILD_LOG"
sed -i "0,/^version = \"$CANDIDATE_VERSION\"$/s//version = \"$BASELINE_VERSION\"/" Cargo.toml
grep -Fx "version = \"$BASELINE_VERSION\"" Cargo.toml >/dev/null || fail "could not prepare the baseline package version"
if ! CARGO_INCREMENTAL=1 cargo build --release --features mcp-runtime -j "$BUILD_JOBS" >>"$BUILD_LOG" 2>&1; then
  tail -n 120 "$BUILD_LOG" | tee -a "$REPORT"
  fail "baseline Rust build failed"
fi
install -m 700 "$CARGO_TARGET_DIR/release/termux-mcp-server" "$BASELINE_ARTIFACT"

git restore --source=HEAD -- Cargo.toml Cargo.lock
[[ -z "$(git status --porcelain)" ]] || fail "repository did not return to exact-head state before the candidate build"
if ! CARGO_INCREMENTAL=1 cargo build --release --locked --features mcp-runtime -j "$BUILD_JOBS" >>"$BUILD_LOG" 2>&1; then
  tail -n 120 "$BUILD_LOG" | tee -a "$REPORT"
  fail "exact candidate Rust build failed"
fi
install -m 700 "$CARGO_TARGET_DIR/release/termux-mcp-server" "$CANDIDATE_ARTIFACT"

assert_eq baseline_reported_version "$("$BASELINE_ARTIFACT" --version | awk 'NR==1 {print $NF}')" "$BASELINE_VERSION"
assert_eq candidate_reported_version "$("$CANDIDATE_ARTIFACT" --version | awk 'NR==1 {print $NF}')" "$CANDIDATE_VERSION"
BASELINE_FILE="$(file -b "$BASELINE_ARTIFACT")"
CANDIDATE_FILE="$(file -b "$CANDIDATE_ARTIFACT")"
log "baseline_file=$BASELINE_FILE"
log "candidate_file=$CANDIDATE_FILE"
[[ "$CANDIDATE_FILE" == *"ARM aarch64"* && "$CANDIDATE_FILE" == *"Android"* ]] || fail "candidate is not an AArch64 Android ELF executable"
BASELINE_SHA="$(file_sha "$BASELINE_ARTIFACT")"
CANDIDATE_SHA="$(file_sha "$CANDIDATE_ARTIFACT")"
log "baseline_sha256=$BASELINE_SHA"
log "candidate_sha256=$CANDIDATE_SHA"
assert_eq candidate_build_head "$(git rev-parse HEAD)" "$EXPECTED_HEAD"
[[ -z "$(git status --porcelain)" ]] || fail "exact candidate build left tracked source changes"

PORT="$(choose_port)" || fail "could not find an unused local TCP port"
DEPLOY_ROOT="$HOME/.local/share/termux-mcp-device-smoke-$HEAD_LABEL-$SMOKE_ID"
CONFIG_ROOT="$HOME/.config/termux-mcp-device-smoke-$HEAD_LABEL-$SMOKE_ID"
SERVICE_ROOT="$PREFIX/var/service-termux-mcp-device-smoke-$HEAD_LABEL-$SMOKE_ID"
SERVICE_DIR="$SERVICE_ROOT/mcp_runtime"
SAFE_ROOT="$HOME/mcp-files-device-smoke-$HEAD_LABEL-$SMOKE_ID"

for path in "$DEPLOY_ROOT" "$CONFIG_ROOT" "$SERVICE_ROOT" "$SAFE_ROOT"; do
  [[ ! -e "$path" && ! -L "$path" ]] || fail "isolated smoke path already exists"
done

install -d -m 700 "$CONFIG_ROOT" "$SERVICE_ROOT" "$SAFE_ROOT"
printf '%s' 'device-smoke-visible' >"$SAFE_ROOT/visible.txt"
chmod 600 "$SAFE_ROOT/visible.txt"
MCP_TOKEN="$(head -c 48 /dev/urandom | base64 | tr -d '\n')"
[[ -n "$MCP_TOKEN" && "$MCP_TOKEN" != *[[:space:]]* ]] || fail "could not generate a private runtime token"
CAPABILITY_KEY_HEX="$(head -c 32 /dev/urandom | sha256sum | awk '{print $1}')"
[[ "$CAPABILITY_KEY_HEX" =~ ^[0-9a-f]{64}$ ]] || fail "could not generate a private capability key"
CAPABILITY_GRANT_FILE="$CONFIG_ROOT/create-directory-grant"
cat >"$CONFIG_ROOT/runtime.env" <<EOF
MCP__AUTH__STATIC_TOKEN=$MCP_TOKEN
MCP__SERVER__HOST=127.0.0.1
MCP__SERVER__PORT=$PORT
MCP__TRANSPORT__ALLOWED_HOSTS=localhost:$PORT,127.0.0.1:$PORT
MCP__TRANSPORT__ALLOWED_ORIGINS=http://localhost:$PORT,http://127.0.0.1:$PORT
MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS=4
MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS=30
MCP__TRANSPORT__MAX_BODY_BYTES=1024
MCP__FILE__SAFE_ROOTS=$SAFE_ROOT
MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=true
MCP__CAPABILITY__KEY_ID=$CAPABILITY_KEY_ID
MCP__CAPABILITY__HMAC_KEY_HEX=$CAPABILITY_KEY_HEX
RUST_LOG=termux_mcp_server=info
EOF
chmod 600 "$CONFIG_ROOT/runtime.env"

export TERMUX_MCP_DEPLOY_ROOT="$DEPLOY_ROOT"
export TERMUX_MCP_CONFIG_ROOT="$CONFIG_ROOT"
export TERMUX_MCP_SERVICE_ROOT="$SERVICE_ROOT"
export TERMUX_MCP_SERVICE_SHELL="$PREFIX/bin/sh"
export TERMUX_MCP_HEALTH_URL="http://127.0.0.1:$PORT/health"
export TERMUX_MCP_READY_URL="http://127.0.0.1:$PORT/ready"
export TERMUX_MCP_PROBE_ATTEMPTS=20
export TERMUX_MCP_PROBE_DELAY_SECONDS=1
export TERMUX_MCP_STOP_ATTEMPTS=20
export TERMUX_MCP_STOP_DELAY_SECONDS=1
MCP_URL="http://127.0.0.1:$PORT/mcp"

log "candidate_version=$CANDIDATE_VERSION"
log "test_port=$PORT"
log "deployment_root=$DEPLOY_ROOT"
log "service_root=$SERVICE_ROOT"
log "service_root_mode=isolated_real_runsvdir"
assert_eq config_mode "$(stat -c '%a' "$CONFIG_ROOT/runtime.env")" 600
HOME_DEVICE="$(stat -c '%d' "$HOME")"
SERVICE_DEVICE="$(stat -c '%d' "$SERVICE_ROOT")"
log "home_device=$HOME_DEVICE"
log "service_root_device=$SERVICE_DEVICE"
assert_eq atomic_publication_device "$SERVICE_DEVICE" "$HOME_DEVICE"

runsvdir "$SERVICE_ROOT" >"$RUNSVDIR_LOG" 2>&1 &
RUNSVDIR_PID=$!
sleep 1
kill -0 "$RUNSVDIR_PID" 2>/dev/null || fail "isolated runsvdir did not stay running"
log "runsvdir_pid=$RUNSVDIR_PID"

BASELINE_RELEASE="$DEPLOY_ROOT/releases/$BASELINE_VERSION"
CANDIDATE_RELEASE="$DEPLOY_ROOT/releases/$CANDIDATE_VERSION"

run_success initial_install bash "$DEPLOY_SCRIPT" install --artifact "$BASELINE_ARTIFACT" --version "$BASELINE_VERSION" --sha256 "$BASELINE_SHA"
assert_eq initial_current "$(link_value "$DEPLOY_ROOT/current")" "$BASELINE_RELEASE"
assert_eq initial_previous "$(link_value "$DEPLOY_ROOT/previous")" none
assert_exists initial_service_run "$SERVICE_DIR/run"
assert_running_state

FAKE_CURL_DIR="$WORK_ROOT/fake-curl-bin"
FAKE_CURL_COUNT="$LOG_DIR/fake-curl-count"
REAL_CURL="$(command -v curl)"
mkdir -p "$FAKE_CURL_DIR"
printf '#!%s\n' "$PREFIX/bin/sh" >"$FAKE_CURL_DIR/curl"
cat >>"$FAKE_CURL_DIR/curl" <<'EOF'
: "${TERMUX_MCP_SMOKE_FAKE_CURL_COUNT:?}"
: "${TERMUX_MCP_SMOKE_FAKE_CURL_FAILS:?}"
: "${TERMUX_MCP_SMOKE_REAL_CURL:?}"
count=0
[ ! -f "$TERMUX_MCP_SMOKE_FAKE_CURL_COUNT" ] || read -r count <"$TERMUX_MCP_SMOKE_FAKE_CURL_COUNT"
count=$((count + 1))
printf '%s\n' "$count" >"$TERMUX_MCP_SMOKE_FAKE_CURL_COUNT"
if [ "$count" -le "$TERMUX_MCP_SMOKE_FAKE_CURL_FAILS" ]; then exit 22; fi
exec "$TERMUX_MCP_SMOKE_REAL_CURL" "$@"
EOF
chmod 700 "$FAKE_CURL_DIR/curl"

printf '0\n' >"$FAKE_CURL_COUNT"
run_failure candidate_readiness_failure env \
  PATH="$FAKE_CURL_DIR:$ORIGINAL_PATH" \
  TERMUX_MCP_SMOKE_FAKE_CURL_COUNT="$FAKE_CURL_COUNT" \
  TERMUX_MCP_SMOKE_FAKE_CURL_FAILS=5 \
  TERMUX_MCP_SMOKE_REAL_CURL="$REAL_CURL" \
  TERMUX_MCP_PROBE_ATTEMPTS=5 \
  TERMUX_MCP_PROBE_DELAY_SECONDS=1 \
  bash "$DEPLOY_SCRIPT" upgrade --artifact "$CANDIDATE_ARTIFACT" --version "$CANDIDATE_VERSION" --sha256 "$CANDIDATE_SHA"
assert_eq readiness_failure_current "$(link_value "$DEPLOY_ROOT/current")" "$BASELINE_RELEASE"
assert_eq readiness_failure_previous "$(link_value "$DEPLOY_ROOT/previous")" none
assert_absent readiness_failure_candidate "$CANDIDATE_RELEASE"
RECOVERY_CURL_CALLS="$(<"$FAKE_CURL_COUNT")"
((RECOVERY_CURL_CALLS > 5)) || fail "candidate readiness recovery did not probe the restored runtime"
log "PASS readiness_recovery_curl_calls=$RECOVERY_CURL_CALLS"
assert_running_state

run_success successful_upgrade bash "$DEPLOY_SCRIPT" upgrade --artifact "$CANDIDATE_ARTIFACT" --version "$CANDIDATE_VERSION" --sha256 "$CANDIDATE_SHA"
assert_eq upgraded_current "$(link_value "$DEPLOY_ROOT/current")" "$CANDIDATE_RELEASE"
assert_eq upgraded_previous "$(link_value "$DEPLOY_ROOT/previous")" "$BASELINE_RELEASE"
assert_running_state
protocol_smoke candidate

DRY_CURRENT="$(link_value "$DEPLOY_ROOT/current")"
DRY_PREVIOUS="$(link_value "$DEPLOY_ROOT/previous")"
run_success production_dry_run bash "$DEPLOY_SCRIPT" rollback --dry-run
assert_eq dry_run_current "$(link_value "$DEPLOY_ROOT/current")" "$DRY_CURRENT"
assert_eq dry_run_previous "$(link_value "$DEPLOY_ROOT/previous")" "$DRY_PREVIOUS"
assert_running_state

printf '0\n' >"$FAKE_CURL_COUNT"
run_failure rollback_readiness_failure env \
  PATH="$FAKE_CURL_DIR:$ORIGINAL_PATH" \
  TERMUX_MCP_SMOKE_FAKE_CURL_COUNT="$FAKE_CURL_COUNT" \
  TERMUX_MCP_SMOKE_FAKE_CURL_FAILS=5 \
  TERMUX_MCP_SMOKE_REAL_CURL="$REAL_CURL" \
  TERMUX_MCP_PROBE_ATTEMPTS=5 \
  TERMUX_MCP_PROBE_DELAY_SECONDS=1 \
  bash "$DEPLOY_SCRIPT" rollback
assert_eq rollback_failure_current "$(link_value "$DEPLOY_ROOT/current")" "$CANDIDATE_RELEASE"
assert_eq rollback_failure_previous "$(link_value "$DEPLOY_ROOT/previous")" "$BASELINE_RELEASE"
ROLLBACK_RECOVERY_CALLS="$(<"$FAKE_CURL_COUNT")"
((ROLLBACK_RECOVERY_CALLS > 5)) || fail "failed rollback did not probe the restored runtime"
log "PASS rollback_recovery_curl_calls=$ROLLBACK_RECOVERY_CALLS"
assert_running_state

run_success successful_rollback bash "$DEPLOY_SCRIPT" rollback
assert_eq rolled_back_current "$(link_value "$DEPLOY_ROOT/current")" "$BASELINE_RELEASE"
assert_eq rolled_back_previous "$(link_value "$DEPLOY_ROOT/previous")" "$CANDIDATE_RELEASE"
assert_running_state

run_success successful_uninstall bash "$DEPLOY_SCRIPT" uninstall
assert_absent uninstall_deployment_root "$DEPLOY_ROOT"
assert_absent uninstall_service_directory "$SERVICE_DIR"
assert_exists uninstall_preserved_config "$CONFIG_ROOT/runtime.env"

log "exact_head=$EXPECTED_HEAD"
log "candidate_sha256=$CANDIDATE_SHA"
log "TERMUX_MCP_DEVICE_RESULT=PASS"
SMOKE_SUCCEEDED=1
