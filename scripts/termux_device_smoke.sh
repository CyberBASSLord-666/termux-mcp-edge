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
HARNESS_VERSION="8"
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
DIRECT_SERVER_PID=""
MCP_TOKEN=""
MCP_SESSION_ID=""
CAPABILITY_KEY_ID="device-smoke-1"
CAPABILITY_KEY_HEX=""
CAPABILITY_GRANT_FILE=""
WRITE_CAPABILITY_GRANT_FILE=""
WRITE_CAPABILITY_CONTENT_FILE=""
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
  if [[ -n "$DIRECT_SERVER_PID" ]]; then
    kill "$DIRECT_SERVER_PID" >/dev/null 2>&1 || true
    wait "$DIRECT_SERVER_PID" >/dev/null 2>&1 || true
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

curl_local() {
  command curl --disable --proto '=http' --noproxy '*' --connect-timeout 2 --max-time 10 "$@"
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
    health="$(curl_local -fsS --max-time 2 "$TERMUX_MCP_HEALTH_URL" 2>/dev/null || true)"
    ready="$(curl_local -fsS --max-time 2 "$TERMUX_MCP_READY_URL" 2>/dev/null || true)"
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
  (((${#payload} == 128 || ${#payload} == 260) && ${#signature} == 64)) || return 1
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
  curl_local "${args[@]}" --data-binary "$payload" "$MCP_URL"
}

mcp_post_file() {
  local output="$1" request_file="$2" session_id="${3:-}" grant_file="${4:-}" grant=""
  [[ -f "$request_file" && ! -L "$request_file" && "$(stat -c '%a' "$request_file")" == 600 ]] \
    || fail "MCP request staging is invalid"
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
    [[ -f "$grant_file" && ! -L "$grant_file" && "$(stat -c '%a' "$grant_file")" == 600 ]] \
      || fail "capability grant staging is invalid"
    grant="$(<"$grant_file")"
    valid_capability_grant "$grant" || fail "candidate emitted an invalid capability grant"
    args+=( -H "MCP-Capability-Grant: $grant" )
  fi
  curl_local "${args[@]}" --data-binary "@$request_file" "$MCP_URL"
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

issue_write_file_grant() {
  local target="$1" content_file="$2" disposition="$3" grant=""
  [[ -f "$content_file" && ! -L "$content_file" && "$(stat -c '%a' "$content_file")" == 600 ]] || fail "write_file capability content staging is invalid"
  : >"$WRITE_CAPABILITY_GRANT_FILE"
  chmod 600 "$WRITE_CAPABILITY_GRANT_FILE"
  if ! MCP__CAPABILITY__CONFIG_FILE="$CONFIG_ROOT/runtime.env" \
    MCP__CAPABILITY__SESSION_ID="$MCP_SESSION_ID" \
    MCP__CAPABILITY__WRITE_FILE_TARGET="$target" \
    MCP__CAPABILITY__WRITE_FILE_CONTENT_FILE="$content_file" \
    MCP__CAPABILITY__WRITE_FILE_DISPOSITION="$disposition" \
      "$CANDIDATE_ARTIFACT" --issue-write-file-grant >"$WRITE_CAPABILITY_GRANT_FILE" 2>/dev/null
  then
    fail "exact candidate could not issue a write_file grant"
  fi
  [[ "$(wc -l <"$WRITE_CAPABILITY_GRANT_FILE")" == 1 ]] || fail "candidate emitted an invalid write_file capability grant"
  grant="$(<"$WRITE_CAPABILITY_GRANT_FILE")"
  valid_capability_grant "$grant" || fail "candidate emitted an invalid write_file capability grant"
  unset grant
}

inspect_write_file_recovery() {
  local label="$1" expected_content="${2-}" expected_mode="${3-}" quarantine entry base mode size links
  local count=0 total_bytes=0 content_matches=0 residue
  quarantine="$SAFE_ROOT/.termux-mcp-write-quarantine"
  residue="$(find "$SAFE_ROOT" -name '.termux-mcp-write-file-*.tmp' -print -quit 2>/dev/null)" \
    || fail "write_file legacy staging inspection failed"
  [[ -z "$residue" ]] || fail "write_file left legacy staging state"

  if [[ -e "$quarantine" || -L "$quarantine" ]]; then
    [[ -d "$quarantine" && ! -L "$quarantine" ]] || fail "write_file recovery namespace is invalid"
    [[ "$(stat -c '%a' "$quarantine" 2>/dev/null)" == 700 ]] \
      || fail "write_file recovery namespace mode is invalid"
    while IFS= read -r -d '' entry; do
      base="${entry##*/}"
      [[ "$base" =~ ^\.termux-mcp-write-artifact-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$ \
        && -f "$entry" && ! -L "$entry" ]] \
        || fail "write_file recovery entry is invalid"
      mode="$(stat -c '%a' "$entry" 2>/dev/null)" || fail "write_file recovery entry stat failed"
      size="$(stat -c '%s' "$entry" 2>/dev/null)" || fail "write_file recovery entry stat failed"
      links="$(stat -c '%h' "$entry" 2>/dev/null)" || fail "write_file recovery entry stat failed"
      [[ "$mode" =~ ^[0-7]{3,4}$ && "$size" =~ ^[0-9]+$ && "$links" == 1 ]] \
        || fail "write_file recovery entry contract is invalid"
      ((size <= 1048576)) || fail "write_file recovery entry exceeds the file bound"
      ((count += 1, total_bytes += size))
      if [[ -n "$expected_content" && "$(<"$entry")" == "$expected_content" \
        && ( -z "$expected_mode" || "$mode" == "$expected_mode" ) ]]; then
        ((content_matches += 1))
      fi
    done < <(find "$quarantine" -mindepth 1 -maxdepth 1 -print0 2>/dev/null) \
      || fail "write_file recovery namespace inspection failed"
  fi

  ((count <= 32 && total_bytes <= 33554432)) \
    || fail "write_file recovery namespace exceeds its bounds"
  WRITE_FILE_RECOVERY_COUNT="$count"
  WRITE_FILE_RECOVERY_CONTENT_MATCHES="$content_matches"
  log "PASS ${label}=bounded"
}

protocol_smoke() {
  local label="$1"
  local body headers status payload target outside oversized copy_source copy_target copy_bytes directory_target hash_digest binary_read_target binary_read_expected
  local replacement_content old_identity new_identity preflight_identity substitute_identity preserved_target
  local recovery_count_before recovery_count_after
  local write_large_content write_large_request write_exact_target
  local write_oversized_content write_oversized_request write_oversized_target
  local write_id_file write_preflight_request write_preflight_target response_bytes
  local body_limit_content body_limit_request
  headers="$LOG_DIR/$label-initialize.headers"
  body="$LOG_DIR/$label-response.json"

  payload='{"jsonrpc":"2.0","id":"unauthorized","method":"tools/list"}'
  status="$(curl_local -sS -o "$body" -w '%{http_code}' \
    -H "Host: localhost:$PORT" \
    -H "Origin: http://localhost:$PORT" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    --data-binary "$payload" "$MCP_URL")"
  assert_eq "${label}_unauthorized_http" "$status" 401
  assert_json "${label}_unauthorized_body" "$body" '.error == "unauthorized" and (.result | not)'

  payload='{"jsonrpc":"2.0","id":"initialize","method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"termux-device-smoke","version":"1.0.0"}}}'
  status="$(curl_local -sS -D "$headers" -o "$body" -w '%{http_code}' \
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
  assert_json "${label}_tool_allowlist" "$body" '[.result.tools[].name] == ["runtime_status","platform_info","android_status","project_service_status","create_directory","copy_file","find_paths","hash_file","list_directory","path_metadata","read_binary_file","read_binary_range","read_file","read_text_range","search_text","write_file"]'
  assert_json "${label}_create_directory_grant_discovery" "$body" '.result.tools | map(select(.name == "create_directory"))[0] as $tool | ($tool.inputSchema.properties.dry_run | has("const") | not) and ($tool.description | contains("MCP-Capability-Grant"))'
  assert_json "${label}_write_file_grant_discovery" "$body" '.result.tools | map(select(.name == "write_file"))[0] as $tool | ($tool.inputSchema.properties.dry_run | has("const") | not) and ($tool.inputSchema.additionalProperties == false) and ($tool.description | contains("MCP-Capability-Grant")) and ($tool.description | contains("target/content/disposition-bound"))'
  assert_json "${label}_find_paths_schema" "$body" '.result.tools | map(select(.name == "find_paths"))[0].inputSchema as $schema | $schema.type == "object" and ($schema.properties | keys) == ["kind","max_depth","path","query"] and $schema.properties.path.type == "string" and $schema.properties.query.type == "string" and $schema.properties.query.minLength == 1 and $schema.properties.query.maxLength == 256 and $schema.properties.query."x-maxBytes" == 256 and $schema.properties.kind.enum == ["any","regular_file","directory"] and $schema.properties.max_depth.minimum == 1 and $schema.properties.max_depth.maximum == 5 and $schema.required == ["path","query"] and $schema.additionalProperties == false'
  assert_json "${label}_hash_file_schema" "$body" '.result.tools | map(select(.name == "hash_file"))[0].inputSchema as $schema | $schema.type == "object" and ($schema.properties | keys) == ["path"] and $schema.properties.path.type == "string" and $schema.required == ["path"] and $schema.additionalProperties == false'
  assert_json "${label}_read_binary_file_schema" "$body" '.result.tools | map(select(.name == "read_binary_file"))[0].inputSchema as $schema | $schema.type == "object" and ($schema.properties | keys) == ["path"] and $schema.properties.path.type == "string" and $schema.required == ["path"] and $schema.additionalProperties == false'
  assert_json "${label}_read_binary_range_schema" "$body" '.result.tools | map(select(.name == "read_binary_range"))[0].inputSchema as $schema | $schema.type == "object" and ($schema.properties | keys) == ["length_bytes","offset_bytes","path"] and $schema.properties.path.type == "string" and $schema.properties.offset_bytes.type == "integer" and $schema.properties.offset_bytes.minimum == 0 and $schema.properties.offset_bytes.maximum == 67108864 and $schema.properties.length_bytes.type == "integer" and $schema.properties.length_bytes.minimum == 1 and $schema.properties.length_bytes.maximum == 262144 and $schema.required == ["path","offset_bytes","length_bytes"] and $schema.additionalProperties == false'
  assert_json "${label}_read_text_range_schema" "$body" '.result.tools | map(select(.name == "read_text_range"))[0].inputSchema as $schema | $schema.type == "object" and ($schema.properties | keys) == ["max_bytes","offset_bytes","path"] and $schema.properties.path.type == "string" and $schema.properties.offset_bytes.type == "integer" and $schema.properties.offset_bytes.minimum == 0 and $schema.properties.offset_bytes.maximum == 67108864 and $schema.properties.max_bytes.type == "integer" and $schema.properties.max_bytes.minimum == 4 and $schema.properties.max_bytes.maximum == 262144 and $schema.required == ["path","offset_bytes","max_bytes"] and $schema.additionalProperties == false'

  payload='{"jsonrpc":"2.0","id":"runtime-status","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_runtime_status_http" "$status" 200
  assert_json "${label}_high_impact_disabled" "$body" '.result.structuredContent.commandExecution == false and .result.structuredContent.androidPlatformTools == false and .result.structuredContent.highImpactTools == false and .result.structuredContent.createDirectoryMutationEnabled == true and .result.structuredContent.createDirectoryGrantRequired == true and .result.structuredContent.createDirectoryGrantHeader == "mcp-capability-grant" and .result.structuredContent.createDirectoryGrantTtlSeconds == 60 and .result.structuredContent.fileWrites == true and .result.structuredContent.fileWriteMode == "dry_run_or_target_content_disposition_scoped_single_use_grant" and .result.structuredContent.fileWriteMutationEnabled == true and .result.structuredContent.fileWriteGrantRequired == true and .result.structuredContent.fileWriteGrantHeader == "mcp-capability-grant" and .result.structuredContent.fileWriteGrantTtlSeconds == 60 and .result.structuredContent.fileWriteMaxBytes == 1048576 and .result.structuredContent.fileWriteMaxResponseBytes == 16384 and .result.structuredContent.pathDiscovery == true and .result.structuredContent.pathDiscoveryMatchMode == "case_sensitive_literal_basename" and .result.structuredContent.pathDiscoveryMaxDepth == 5 and .result.structuredContent.pathDiscoveryMaxEntries == 8192 and .result.structuredContent.pathDiscoveryMaxMatches == 512 and .result.structuredContent.pathDiscoveryMaxQueryBytes == 256 and .result.structuredContent.pathDiscoveryMaxResponseBytes == 262144 and .result.structuredContent.binaryFileReads == true and .result.structuredContent.binaryFileReadEncoding == "base64" and .result.structuredContent.binaryFileReadMaxBytes == 1048576 and .result.structuredContent.binaryFileReadMaxResponseBytes == 1507328 and .result.structuredContent.binaryRangeReads == true and .result.structuredContent.binaryRangeReadEncoding == "base64" and .result.structuredContent.binaryRangeReadMaxFileBytes == 67108864 and .result.structuredContent.binaryRangeReadMaxBytes == 262144 and .result.structuredContent.binaryRangeReadMaxResponseBytes == 393216 and .result.structuredContent.textRangeReads == true and .result.structuredContent.textRangeReadEncoding == "utf-8" and .result.structuredContent.textRangeReadMinBytes == 4 and .result.structuredContent.textRangeReadMaxFileBytes == 67108864 and .result.structuredContent.textRangeReadMaxBytes == 262144 and .result.structuredContent.textRangeReadMaxResponseBytes == 1703936 and .result.structuredContent.fileHashing == true and .result.structuredContent.fileHashAlgorithm == "sha256" and .result.structuredContent.fileHashMaxBytes == 16777216'

  payload="$(jq -cn --arg path "$SAFE_ROOT" '{"jsonrpc":"2.0","id":"list-directory","method":"tools/call","params":{"name":"list_directory","arguments":{"path":$path,"max_depth":1}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_list_directory_http" "$status" 200
  jq -e --arg expected "$SAFE_ROOT/visible.txt" '.result.structuredContent.entries | any(.path == $expected)' "$body" >/dev/null || fail "safe-root listing omitted the expected file"
  log "PASS ${label}_list_directory=expected-file"

  payload="$(jq -cn --arg path "$SAFE_ROOT" --arg query visible '{"jsonrpc":"2.0","id":"find-paths","method":"tools/call","params":{"name":"find_paths","arguments":{"path":$path,"query":$query,"kind":"regular_file","max_depth":1}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_find_paths_http" "$status" 200
  jq -e --arg expected "$SAFE_ROOT/visible.txt" '
    .result.structuredContent as $find
    | $find.matches == [{"path":$expected,"kind":"regular_file"}]
      and $find.truncated == false
      and $find.queryBytes == 7
      and $find.kindFilter == "regular_file"
      and $find.maxDepth == 1
      and $find.maxEntries == 8192
      and $find.maxMatches == 512
      and $find.maxResponseBytes == 262144
  ' "$body" >/dev/null || fail "${label}_find_paths_result JSON assertion failed"
  grep -Fq device-smoke-visible "$body" && fail "path-discovery response reflected file content"
  log "PASS ${label}_find_paths=expected-file"

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

  hash_digest="$(sha256sum -- "$SAFE_ROOT/visible.txt" | awk '{print $1}')" || fail "could not calculate the device-smoke hash fixture digest"
  payload="$(jq -cn --arg path "$SAFE_ROOT/visible.txt" '{"jsonrpc":"2.0","id":"hash-file","method":"tools/call","params":{"name":"hash_file","arguments":{"path":$path}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_hash_file_http" "$status" 200
  jq -e --arg digest "$hash_digest" '
    .result.structuredContent as $hash
    | ($hash | keys) == ["algorithm","digest","sizeBytes"]
      and $hash.algorithm == "sha256"
      and $hash.digest == $digest
      and $hash.sizeBytes == 20
  ' "$body" >/dev/null || fail "${label}_hash_file_result JSON assertion failed"
  if grep -Eq 'device-smoke-visible|visible\.txt|termux-mcp-device-smoke-' "$body"; then
    fail "hash_file response reflected file content or a path"
  fi
  log "PASS ${label}_hash_file=sha256"

  binary_read_target="$SAFE_ROOT/binary-read.bin"
  printf '\000\377\200\141\012\001\376' >"$binary_read_target" || fail "could not create the binary-read device-smoke fixture"
  binary_read_expected="$(base64 <"$binary_read_target" | tr -d '\n')" || fail "could not encode the binary-read device-smoke fixture"
  payload="$(jq -cn --arg path "$binary_read_target" '{"jsonrpc":"2.0","id":"read-binary-file","method":"tools/call","params":{"name":"read_binary_file","arguments":{"path":$path}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_read_binary_file_http" "$status" 200
  jq -e --arg data "$binary_read_expected" '
    .result.structuredContent as $binary
    | ($binary | keys) == ["data","encoding","maxFileBytes","maxResponseBytes","sizeBytes"]
      and $binary.encoding == "base64"
      and $binary.data == $data
      and $binary.sizeBytes == 7
      and $binary.maxFileBytes == 1048576
      and $binary.maxResponseBytes == 1507328
  ' "$body" >/dev/null || fail "${label}_read_binary_file_result JSON assertion failed"
  (( $(wc -c <"$body") <= 1507328 )) || fail "read_binary_file response exceeded its full-response ceiling"
  if grep -Eq 'binary-read\.bin|inode|device|uid|gid|mode|accessTime|termux-mcp-device-smoke-' "$body"; then
    fail "read_binary_file response reflected a path or denied metadata"
  fi

  payload="$(jq -cn --arg path "$binary_read_target" '{"jsonrpc":"2.0","id":"read-binary-range","method":"tools/call","params":{"name":"read_binary_range","arguments":{"path":$path,"offset_bytes":2,"length_bytes":4}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_read_binary_range_http" "$status" 200
  jq -e '
    .result.structuredContent as $range
    | ($range | keys) == ["data","encoding","eof","fileSizeBytes","maxFileBytes","maxReadBytes","maxResponseBytes","offsetBytes","sizeBytes"]
      and $range.encoding == "base64"
      and $range.data == "gGEKAQ=="
      and $range.offsetBytes == 2
      and $range.sizeBytes == 4
      and $range.fileSizeBytes == 7
      and $range.eof == false
      and $range.maxReadBytes == 262144
      and $range.maxFileBytes == 67108864
      and $range.maxResponseBytes == 393216
  ' "$body" >/dev/null || fail "${label}_read_binary_range_result JSON assertion failed"
  (( $(wc -c <"$body") <= 393216 )) || fail "read_binary_range response exceeded its full-response ceiling"
  if grep -Eq 'binary-read\.bin|inode|device|uid|gid|mode|accessTime|termux-mcp-device-smoke-' "$body"; then
    fail "read_binary_range response reflected a path or denied metadata"
  fi

  payload="$(jq -cn --arg path "$binary_read_target" '{"jsonrpc":"2.0","id":"read-binary-range-short-final","method":"tools/call","params":{"name":"read_binary_range","arguments":{"path":$path,"offset_bytes":5,"length_bytes":10}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_read_binary_range_short_final_http" "$status" 200
  assert_json "${label}_read_binary_range_short_final" "$body" '.result.structuredContent.data == "Af4=" and .result.structuredContent.offsetBytes == 5 and .result.structuredContent.sizeBytes == 2 and .result.structuredContent.fileSizeBytes == 7 and .result.structuredContent.eof == true'

  payload="$(jq -cn --arg path "$binary_read_target" '{"jsonrpc":"2.0","id":"read-binary-range-eof","method":"tools/call","params":{"name":"read_binary_range","arguments":{"path":$path,"offset_bytes":7,"length_bytes":1}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_read_binary_range_eof_http" "$status" 200
  assert_json "${label}_read_binary_range_eof" "$body" '.result.structuredContent.data == "" and .result.structuredContent.sizeBytes == 0 and .result.structuredContent.eof == true'
  rm -f -- "$binary_read_target" || fail "could not remove the binary-read device-smoke fixture"
  log "PASS ${label}_read_binary_file=base64"
  log "PASS ${label}_read_binary_range=base64"

  text_range_target="$SAFE_ROOT/text-range-private.txt"
  printf '\141\303\251\360\237\231\202\172' >"$text_range_target" || fail "could not create the text-range device-smoke fixture"
  payload="$(jq -cn --arg path "$text_range_target" '{"jsonrpc":"2.0","id":"read-text-range","method":"tools/call","params":{"name":"read_text_range","arguments":{"path":$path,"offset_bytes":0,"max_bytes":4}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_read_text_range_http" "$status" 200
  assert_json "${label}_read_text_range_result" "$body" '.result.structuredContent.content == "a\u00e9" and .result.structuredContent.offsetBytes == 0 and .result.structuredContent.nextOffsetBytes == 3 and .result.structuredContent.sizeBytes == 3 and .result.structuredContent.fileSizeBytes == 8 and .result.structuredContent.eof == false and .result.structuredContent.maxReadBytes == 262144 and .result.structuredContent.maxFileBytes == 67108864 and .result.structuredContent.maxResponseBytes == 1703936'
  (( $(wc -c <"$body") <= 1703936 )) || fail "read_text_range response exceeded its full-response ceiling"
  if grep -Eq 'text-range-private\.txt|inode|device|uid|gid|mode|accessTime|termux-mcp-device-smoke-' "$body"; then
    fail "read_text_range response reflected a path or denied metadata"
  fi

  payload="$(jq -cn --arg path "$text_range_target" '{"jsonrpc":"2.0","id":"read-text-range-second","method":"tools/call","params":{"name":"read_text_range","arguments":{"path":$path,"offset_bytes":3,"max_bytes":4}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_read_text_range_second_http" "$status" 200
  assert_json "${label}_read_text_range_second" "$body" '.result.structuredContent.content == "\ud83d\ude42" and .result.structuredContent.nextOffsetBytes == 7 and .result.structuredContent.sizeBytes == 4 and .result.structuredContent.eof == false'

  payload="$(jq -cn --arg path "$text_range_target" '{"jsonrpc":"2.0","id":"read-text-range-mid-codepoint","method":"tools/call","params":{"name":"read_text_range","arguments":{"path":$path,"offset_bytes":2,"max_bytes":4}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_read_text_range_mid_codepoint_http" "$status" 400
  assert_json "${label}_read_text_range_mid_codepoint" "$body" '.error.code == -32602'
  rm -f -- "$text_range_target" || fail "could not remove the text-range device-smoke fixture"
  log "PASS ${label}_read_text_range=utf-8-boundaries"

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

  printf '%s' 'device-smoke-write' >"$WRITE_CAPABILITY_CONTENT_FILE"
  chmod 600 "$WRITE_CAPABILITY_CONTENT_FILE"

  payload="$(jq -cn --arg path "$target" --arg content 'device-smoke-write' '{"jsonrpc":"2.0","id":"write-missing-grant","method":"tools/call","params":{"name":"write_file","arguments":{"path":$path,"content":$content,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_write_missing_grant_http" "$status" 403
  assert_json "${label}_write_missing_grant_body" "$body" '.error.code == -32003 and .error.data.reason == "capability_grant_missing"'
  assert_absent "${label}_write_missing_grant_target" "$target"

  issue_write_file_grant "$target" "$WRITE_CAPABILITY_CONTENT_FILE" create

  payload="$(jq -cn --arg path "$target" --arg content 'device-smoke-write-mismatch' '{"jsonrpc":"2.0","id":"write-grant-mismatch","method":"tools/call","params":{"name":"write_file","arguments":{"path":$path,"content":$content,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" "$WRITE_CAPABILITY_GRANT_FILE")"
  assert_eq "${label}_write_grant_mismatch_http" "$status" 403
  assert_json "${label}_write_grant_mismatch_body" "$body" '.error.code == -32003 and .error.data.reason == "capability_grant_binding_mismatch"'
  assert_absent "${label}_write_grant_mismatch_target" "$target"

  payload="$(jq -cn --arg path "$target" --arg content 'device-smoke-write' '{"jsonrpc":"2.0","id":"write-explicit","method":"tools/call","params":{"name":"write_file","arguments":{"path":$path,"content":$content,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" "$WRITE_CAPABILITY_GRANT_FILE")"
  assert_eq "${label}_write_explicit_http" "$status" 200
  assert_json "${label}_write_explicit_body" "$body" '.result.structuredContent.dryRun == false and .result.structuredContent.sizeBytes == 18 and .result.structuredContent.disposition == "create" and .result.structuredContent.mode == "0600" and .result.structuredContent.recoveryArtifactRetained == false'
  assert_eq "${label}_write_content" "$(<"$target")" "device-smoke-write"
  assert_eq "${label}_write_mode" "$(stat -c '%a' "$target")" 600

  rm -f -- "$target"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" "$WRITE_CAPABILITY_GRANT_FILE")"
  assert_eq "${label}_write_grant_replay_http" "$status" 403
  assert_json "${label}_write_grant_replay_body" "$body" '.error.code == -32003 and .error.data.reason == "capability_grant_replayed"'
  assert_absent "${label}_write_grant_replay_target" "$target"

  printf '%s' 'device-smoke-replace-original' >"$target"
  chmod 640 "$target"
  inspect_write_file_recovery "${label}_write_replace_recovery_preflight"
  recovery_count_before="$WRITE_FILE_RECOVERY_COUNT"
  old_identity="$(stat -c '%d:%i' "$target")" || fail "write_file replacement identity preflight failed"
  replacement_content='device-smoke-replacement'
  printf '%s' "$replacement_content" >"$WRITE_CAPABILITY_CONTENT_FILE"
  chmod 600 "$WRITE_CAPABILITY_CONTENT_FILE"
  issue_write_file_grant "$target" "$WRITE_CAPABILITY_CONTENT_FILE" replace

  payload="$(jq -cn --arg path "$target" --arg content "$replacement_content" '{"jsonrpc":"2.0","id":"write-replace","method":"tools/call","params":{"name":"write_file","arguments":{"path":$path,"content":$content,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" "$WRITE_CAPABILITY_GRANT_FILE")"
  assert_eq "${label}_write_replace_http" "$status" 200
  assert_json "${label}_write_replace_body" "$body" ".result.structuredContent.dryRun == false and .result.structuredContent.sizeBytes == ${#replacement_content} and .result.structuredContent.disposition == \"replace\" and .result.structuredContent.mode == \"0600\" and .result.structuredContent.recoveryArtifactRetained == true"
  [[ "$(<"$target")" == "$replacement_content" ]] || fail "write_file replacement content verification failed"
  log "PASS ${label}_write_replace_content=valid"
  assert_eq "${label}_write_replace_mode" "$(stat -c '%a' "$target")" 600
  new_identity="$(stat -c '%d:%i' "$target")" || fail "write_file replacement identity verification failed"
  [[ "$new_identity" != "$old_identity" ]] || fail "write_file replacement retained the old target identity"
  log "PASS ${label}_write_replace_identity=fresh"
  inspect_write_file_recovery "${label}_write_replace_recovery" 'device-smoke-replace-original' 640
  recovery_count_after="$WRITE_FILE_RECOVERY_COUNT"
  ((recovery_count_after == recovery_count_before + 1)) \
    || fail "write_file replacement did not retain exactly one recovery artifact"
  ((WRITE_FILE_RECOVERY_CONTENT_MATCHES == 1)) \
    || fail "write_file replacement did not retain the displaced content exactly once"

  payload="$(jq -cn --arg path "$SAFE_ROOT" '{"jsonrpc":"2.0","id":"write-recovery-list","method":"tools/call","params":{"name":"list_directory","arguments":{"path":$path,"max_depth":1}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_write_recovery_list_http" "$status" 200
  if grep -Fq '.termux-mcp-write-quarantine' "$body"; then
    fail "write_file recovery namespace was visible through list_directory"
  fi
  log "PASS ${label}_write_recovery_list=private"

  payload="$(jq -cn --arg path "$SAFE_ROOT" --arg query '.termux-mcp-write-quarantine' '{"jsonrpc":"2.0","id":"write-recovery-find","method":"tools/call","params":{"name":"find_paths","arguments":{"path":$path,"query":$query,"kind":"directory","max_depth":1}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq "${label}_write_recovery_find_http" "$status" 200
  assert_json "${label}_write_recovery_find" "$body" '.result.structuredContent.matches == []'

  preserved_target="$SAFE_ROOT/write-preflight-original.txt"
  printf '%s' 'device-smoke-preflight-original' >"$target"
  chmod 600 "$target"
  preflight_identity="$(stat -c '%d:%i' "$target")" || fail "write_file binding identity preflight failed"
  printf '%s' 'device-smoke-binding-denied' >"$WRITE_CAPABILITY_CONTENT_FILE"
  chmod 600 "$WRITE_CAPABILITY_CONTENT_FILE"
  issue_write_file_grant "$target" "$WRITE_CAPABILITY_CONTENT_FILE" replace
  mv -- "$target" "$preserved_target" || fail "write_file binding fixture preservation failed"
  printf '%s' 'device-smoke-substitute' >"$target"
  chmod 600 "$target"
  substitute_identity="$(stat -c '%d:%i' "$target")" || fail "write_file substitute identity preflight failed"

  payload="$(jq -cn --arg path "$target" --arg content 'device-smoke-binding-denied' '{"jsonrpc":"2.0","id":"write-replace-binding","method":"tools/call","params":{"name":"write_file","arguments":{"path":$path,"content":$content,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" "$WRITE_CAPABILITY_GRANT_FILE")"
  assert_eq "${label}_write_replace_binding_http" "$status" 403
  assert_json "${label}_write_replace_binding_body" "$body" '.error.code == -32003 and .error.data.reason == "capability_grant_binding_mismatch"'
  [[ "$(<"$target")" == device-smoke-substitute \
    && "$(stat -c '%d:%i' "$target")" == "$substitute_identity" ]] \
    || fail "write_file binding denial modified the substitute"
  log "PASS ${label}_write_replace_substitute=preserved"
  [[ "$(<"$preserved_target")" == device-smoke-preflight-original \
    && "$(stat -c '%d:%i' "$preserved_target")" == "$preflight_identity" ]] \
    || fail "write_file binding denial modified the preflight original"
  log "PASS ${label}_write_replace_original=preserved"
  inspect_write_file_recovery "${label}_write_replace_binding_recovery" 'device-smoke-replace-original' 640
  ((WRITE_FILE_RECOVERY_COUNT == recovery_count_after)) \
    || fail "write_file binding denial changed recovery state"
  ((WRITE_FILE_RECOVERY_CONTENT_MATCHES == 1)) \
    || fail "write_file binding denial changed retained recovery content"
  rm -f -- "$target" "$preserved_target"

  write_large_content="$CONFIG_ROOT/write-exact-1mib.txt"
  write_large_request="$CONFIG_ROOT/write-exact-1mib.json"
  write_exact_target="$SAFE_ROOT/write-exact-1mib.txt"
  dd if=/dev/zero bs=1048576 count=1 status=none 2>/dev/null \
    | tr '\000' x >"$write_large_content" \
    || fail "could not stage the exact-limit write_file content"
  chmod 600 "$write_large_content"
  jq -cn --arg path "$write_exact_target" --rawfile content "$write_large_content" \
    '{jsonrpc:"2.0",id:"write-exact-1mib",method:"tools/call",params:{name:"write_file",arguments:{path:$path,content:$content,dry_run:false}}}' \
    >"$write_large_request" || fail "could not stage the exact-limit write_file request"
  chmod 600 "$write_large_request"
  issue_write_file_grant "$write_exact_target" "$write_large_content" create
  status="$(mcp_post_file "$body" "$write_large_request" "$MCP_SESSION_ID" "$WRITE_CAPABILITY_GRANT_FILE")"
  assert_eq "${label}_write_exact_1mib_http" "$status" 200
  assert_json "${label}_write_exact_1mib_body" "$body" '
    .result.structuredContent.dryRun == false
    and .result.structuredContent.sizeBytes == 1048576
    and .result.structuredContent.disposition == "create"
    and .result.structuredContent.mode == "0600"
    and .result.structuredContent.maxFileBytes == 1048576
    and .result.structuredContent.maxResponseBytes == 16384
    and .result.structuredContent.recoveryArtifactRetained == false
  '
  assert_eq "${label}_write_exact_1mib_size" "$(stat -c '%s' "$write_exact_target")" 1048576
  assert_eq "${label}_write_exact_1mib_mode" "$(stat -c '%a' "$write_exact_target")" 600
  cmp -s "$write_large_content" "$write_exact_target" \
    || fail "exact-limit write_file content differs"
  log "PASS ${label}_write_exact_1mib=exact"

  write_oversized_content="$CONFIG_ROOT/write-1mib-plus-one.txt"
  write_oversized_request="$CONFIG_ROOT/write-1mib-plus-one.json"
  write_oversized_target="$SAFE_ROOT/write-1mib-plus-one.txt"
  dd if=/dev/zero bs=1048577 count=1 status=none 2>/dev/null \
    | tr '\000' y >"$write_oversized_content" \
    || fail "could not stage the over-limit write_file content"
  chmod 600 "$write_oversized_content"
  jq -cn --arg path "$write_oversized_target" --rawfile content "$write_oversized_content" \
    '{jsonrpc:"2.0",id:"write-1mib-plus-one",method:"tools/call",params:{name:"write_file",arguments:{path:$path,content:$content,dry_run:false}}}' \
    >"$write_oversized_request" || fail "could not stage the over-limit write_file request"
  chmod 600 "$write_oversized_request"
  printf '%s' 'device-smoke-over-limit-grant-retry' >"$WRITE_CAPABILITY_CONTENT_FILE"
  chmod 600 "$WRITE_CAPABILITY_CONTENT_FILE"
  issue_write_file_grant "$write_oversized_target" "$WRITE_CAPABILITY_CONTENT_FILE" create
  status="$(mcp_post_file "$body" "$write_oversized_request" "$MCP_SESSION_ID" "$WRITE_CAPABILITY_GRANT_FILE")"
  assert_eq "${label}_write_1mib_plus_one_http" "$status" 413
  assert_json "${label}_write_1mib_plus_one_body" "$body" \
    '.id == "write-1mib-plus-one" and .error.code == -32001'
  assert_absent "${label}_write_1mib_plus_one_target" "$write_oversized_target"
  payload="$(jq -cn --arg path "$write_oversized_target" --arg content 'device-smoke-over-limit-grant-retry' '{jsonrpc:"2.0",id:"write-1mib-plus-one-grant-retry",method:"tools/call",params:{name:"write_file",arguments:{path:$path,content:$content,dry_run:false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" "$WRITE_CAPABILITY_GRANT_FILE")"
  assert_eq "${label}_write_1mib_plus_one_grant_retry_http" "$status" 200
  assert_eq "${label}_write_1mib_plus_one_grant_retry_content" \
    "$(<"$write_oversized_target")" device-smoke-over-limit-grant-retry

  printf '%s' preflight-content >"$WRITE_CAPABILITY_CONTENT_FILE"
  chmod 600 "$WRITE_CAPABILITY_CONTENT_FILE"
  write_preflight_target="$SAFE_ROOT/write-response-preflight.txt"
  write_id_file="$CONFIG_ROOT/write-oversized-id.txt"
  write_preflight_request="$CONFIG_ROOT/write-response-preflight.json"
  printf '%*s' 17000 '' | tr ' ' z >"$write_id_file" \
    || fail "could not stage the oversized write_file response identifier"
  chmod 600 "$write_id_file"
  jq -cn --rawfile request_id "$write_id_file" --arg path "$write_preflight_target" \
    --rawfile content "$WRITE_CAPABILITY_CONTENT_FILE" \
    '{jsonrpc:"2.0",id:$request_id,method:"tools/call",params:{name:"write_file",arguments:{path:$path,content:$content,dry_run:false}}}' \
    >"$write_preflight_request" || fail "could not stage the write_file response-preflight request"
  chmod 600 "$write_preflight_request"
  issue_write_file_grant "$write_preflight_target" "$WRITE_CAPABILITY_CONTENT_FILE" create
  status="$(mcp_post_file "$body" "$write_preflight_request" "$MCP_SESSION_ID" "$WRITE_CAPABILITY_GRANT_FILE")"
  assert_eq "${label}_write_response_preflight_http" "$status" 413
  assert_json "${label}_write_response_preflight_body" "$body" '.id == null and .error.code == -32001'
  response_bytes="$(wc -c <"$body")"
  ((response_bytes <= 16384)) || fail "write_file response-preflight error exceeded its bound"
  assert_absent "${label}_write_response_preflight_target" "$write_preflight_target"
  payload="$(jq -cn --arg path "$write_preflight_target" --arg content preflight-content '{jsonrpc:"2.0",id:"write-response-preflight-retry",method:"tools/call",params:{name:"write_file",arguments:{path:$path,content:$content,dry_run:false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID" "$WRITE_CAPABILITY_GRANT_FILE")"
  assert_eq "${label}_write_response_preflight_retry_http" "$status" 200
  assert_eq "${label}_write_response_preflight_content" \
    "$(<"$write_preflight_target")" preflight-content
  inspect_write_file_recovery "${label}_write_boundary_recovery"
  ((WRITE_FILE_RECOVERY_COUNT == recovery_count_after)) \
    || fail "write_file boundary checks changed retained recovery state"

  rm -f -- "$write_exact_target" "$write_oversized_target" "$write_preflight_target" \
    "$write_large_content" "$write_large_request" "$write_oversized_content" \
    "$write_oversized_request" "$write_id_file" "$write_preflight_request"

  rm -f -- "$WRITE_CAPABILITY_GRANT_FILE" "$WRITE_CAPABILITY_CONTENT_FILE"
  WRITE_CAPABILITY_GRANT_FILE=""
  WRITE_CAPABILITY_CONTENT_FILE=""

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

  body_limit_content="$CONFIG_ROOT/request-body-limit-content.txt"
  body_limit_request="$CONFIG_ROOT/request-body-limit.json"
  dd if=/dev/zero bs=2097152 count=1 status=none 2>/dev/null \
    | tr '\000' q >"$body_limit_content" \
    || fail "could not stage the request-body-limit fixture"
  chmod 600 "$body_limit_content"
  jq -cn --rawfile content "$body_limit_content" \
    '{jsonrpc:"2.0",id:"oversized",method:"tools/call",params:{name:"write_file",arguments:{path:"/ignored",content:$content}}}' \
    >"$body_limit_request" || fail "could not stage the request-body-limit request"
  chmod 600 "$body_limit_request"
  status="$(mcp_post_file "$body" "$body_limit_request" "$MCP_SESSION_ID")"
  assert_eq "${label}_authenticated_oversized_http" "$status" 413
  assert_json "${label}_authenticated_oversized_body" "$body" '.error == "mcp_request_body_too_large"'

  status="$(curl_local -sS -o "$body" -w '%{http_code}' \
    -H "Host: localhost:$PORT" \
    -H "Origin: http://localhost:$PORT" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    --data-binary "@$body_limit_request" "$MCP_URL")"
  assert_eq "${label}_unauthenticated_oversized_http" "$status" 401
  assert_json "${label}_unauthenticated_oversized_body" "$body" '.error == "unauthorized"'
  rm -f -- "$body_limit_content" "$body_limit_request"

  status="$(curl_local -sS -X DELETE -o "$body" -w '%{http_code}' \
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

volume_control_disabled_smoke() {
  local body="$LOG_DIR/volume-control-disabled-response.json"
  local headers="$LOG_DIR/volume-control-disabled-initialize.headers"
  local server_log="$LOG_DIR/volume-control-disabled-server.log"
  local status payload attempt

  env -i \
    "HOME=$HOME" \
    "PREFIX=$PREFIX" \
    "PATH=$ORIGINAL_PATH" \
    "MCP__AUTH__STATIC_TOKEN=$MCP_TOKEN" \
    MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=false \
    MCP__SERVER__HOST=127.0.0.1 \
    "MCP__SERVER__PORT=$PORT" \
    "MCP__TRANSPORT__ALLOWED_HOSTS=localhost:$PORT,127.0.0.1:$PORT" \
    "MCP__TRANSPORT__ALLOWED_ORIGINS=http://localhost:$PORT,http://127.0.0.1:$PORT" \
    MCP__TRANSPORT__SSE_ENABLED=false \
    MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS=4 \
    MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS=30 \
    MCP__TRANSPORT__MAX_BODY_BYTES=1024 \
    "MCP__FILE__SAFE_ROOTS=$SAFE_ROOT" \
    "$VOLUME_CONTROL_ARTIFACT" >"$server_log" 2>&1 &
  DIRECT_SERVER_PID=$!
  for attempt in $(seq 1 40); do
    kill -0 "$DIRECT_SERVER_PID" >/dev/null 2>&1 || fail "volume-control disabled runtime exited before readiness"
    if [[ "$(curl_local -fsS --max-time 2 "http://127.0.0.1:$PORT/health" 2>/dev/null || true)" == ok ]]; then
      break
    fi
    sleep 0.1
  done
  [[ "$(curl_local -fsS --max-time 2 "http://127.0.0.1:$PORT/health" 2>/dev/null || true)" == ok ]] || fail "volume-control disabled runtime did not become healthy"

  payload='{"jsonrpc":"2.0","id":"initialize-volume-control-disabled","method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"termux-device-smoke","version":"1.0.0"}}}'
  status="$(curl_local -sS -D "$headers" -o "$body" -w '%{http_code}' \
    -H "Authorization: Bearer $MCP_TOKEN" \
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
    --data-binary "$payload" "$MCP_URL")"
  assert_eq volume_control_disabled_initialize_http "$status" 200
  assert_json volume_control_disabled_initialize_body "$body" '.result.protocolVersion == "2025-11-25"'
  MCP_SESSION_ID="$(awk 'tolower($1) == "mcp-session-id:" {sub(/^[^:]*:[[:space:]]*/, ""); sub(/\r$/, ""); print; exit}' "$headers")"
  [[ "$MCP_SESSION_ID" =~ ^[A-Za-z0-9-]{1,128}$ ]] || fail "volume-control disabled runtime omitted its session ID"

  payload='{"jsonrpc":"2.0","method":"notifications/initialized"}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq volume_control_disabled_initialized_http "$status" 202
  [[ ! -s "$body" ]] || fail "volume-control disabled initialized notification returned a body"

  payload='{"jsonrpc":"2.0","id":"volume-control-disabled-tools","method":"tools/list"}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq volume_control_disabled_tools_http "$status" 200
  assert_json volume_control_disabled_discovery "$body" '[.result.tools[].name] == ["runtime_status","platform_info","android_status","project_service_status","create_directory","copy_file","find_paths","hash_file","list_directory","path_metadata","read_binary_file","read_binary_range","read_file","read_text_range","search_text","write_file"]'
  assert_json volume_control_disabled_write_discovery "$body" '.result.tools | map(select(.name == "write_file"))[0] as $tool | $tool.inputSchema.properties.dry_run.const == true and ($tool.description | contains("mutation gate is disabled"))'

  payload='{"jsonrpc":"2.0","id":"volume-control-disabled-status","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq volume_control_disabled_status_http "$status" 200
  assert_json volume_control_disabled_status "$body" '.result.structuredContent.androidVolumeControlCompiled == true and .result.structuredContent.androidVolumeControlEnabled == false and .result.structuredContent.androidVolumeGrantRequired == false and .result.structuredContent.highImpactTools == false and .result.structuredContent.fileWrites == true and .result.structuredContent.fileWriteMode == "dry_run_only_mutation_disabled" and .result.structuredContent.fileWriteMutationEnabled == false and .result.structuredContent.fileWriteGrantRequired == false and .result.structuredContent.fileWriteGrantHeader == "mcp-capability-grant" and .result.structuredContent.fileWriteGrantTtlSeconds == 60 and .result.structuredContent.binaryFileReads == true and .result.structuredContent.binaryFileReadEncoding == "base64" and .result.structuredContent.binaryFileReadMaxBytes == 1048576 and .result.structuredContent.binaryFileReadMaxResponseBytes == 1507328 and .result.structuredContent.binaryRangeReads == true and .result.structuredContent.binaryRangeReadMaxFileBytes == 67108864 and .result.structuredContent.binaryRangeReadMaxBytes == 262144 and .result.structuredContent.binaryRangeReadMaxResponseBytes == 393216 and .result.structuredContent.textRangeReads == true and .result.structuredContent.textRangeReadEncoding == "utf-8" and .result.structuredContent.textRangeReadMinBytes == 4 and .result.structuredContent.textRangeReadMaxFileBytes == 67108864 and .result.structuredContent.textRangeReadMaxBytes == 262144 and .result.structuredContent.textRangeReadMaxResponseBytes == 1703936 and .result.structuredContent.fileHashing == true and .result.structuredContent.fileHashAlgorithm == "sha256" and .result.structuredContent.fileHashMaxBytes == 16777216'

  payload='{"jsonrpc":"2.0","id":"volume-control-disabled-call","method":"tools/call","params":{"name":"set_android_volume","arguments":{"stream":"music","level":1,"dry_run":false}}}'
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  assert_eq volume_control_disabled_call_http "$status" 200
  assert_json volume_control_disabled_call "$body" '.result.isError == true and .result.structuredContent.reasonCode == "volume_control_runtime_disabled"'

  status="$(curl_local -sS -X DELETE -o "$body" -w '%{http_code}' \
    -H "Authorization: Bearer $MCP_TOKEN" \
    -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
    -H 'MCP-Protocol-Version: 2025-11-25' -H "MCP-Session-Id: $MCP_SESSION_ID" \
    "$MCP_URL")"
  assert_eq volume_control_disabled_delete_http "$status" 204
  MCP_SESSION_ID=""

  kill "$DIRECT_SERVER_PID" >/dev/null 2>&1 || fail "volume-control disabled runtime could not be stopped"
  for attempt in $(seq 1 40); do
    kill -0 "$DIRECT_SERVER_PID" >/dev/null 2>&1 || break
    sleep 0.1
  done
  if kill -0 "$DIRECT_SERVER_PID" >/dev/null 2>&1; then
    kill -KILL "$DIRECT_SERVER_PID" >/dev/null 2>&1 || true
    wait "$DIRECT_SERVER_PID" >/dev/null 2>&1 || true
    DIRECT_SERVER_PID=""
    fail "volume-control disabled runtime required forced termination"
  fi
  wait "$DIRECT_SERVER_PID" >/dev/null 2>&1 || true
  DIRECT_SERVER_PID=""
  log "PASS volume_control_disabled_runtime=verified_without_device_mutation"
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

for command_name in git bash cargo rustc clang sv runsvdir awk base64 cmp curl dd file find grep install jq realpath sed seq sha256sum stat tee timeout tr ss wc; do
  require_command "$command_name"
done

AVAILABLE_KB="$(df -Pk "$HOME" | awk 'NR==2 {print $4}')"
log "available_home_kb=$AVAILABLE_KB"
if [[ "$AVAILABLE_KB" =~ ^[0-9]+$ ]] && ((AVAILABLE_KB < 1572864)); then
  fail "at least 1.5 GiB of free space is required for the release builds"
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
VOLUME_CONTROL_ARTIFACT="$ARTIFACT_DIR/termux-mcp-server-$CANDIDATE_VERSION-android-volume-control"

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
if ! CARGO_INCREMENTAL=1 cargo build --release --locked --features android-volume-control -j "$BUILD_JOBS" >>"$BUILD_LOG" 2>&1; then
  tail -n 120 "$BUILD_LOG" | tee -a "$REPORT"
  fail "exact volume-control candidate Rust build failed"
fi
install -m 700 "$CARGO_TARGET_DIR/release/termux-mcp-server" "$VOLUME_CONTROL_ARTIFACT"

assert_eq baseline_reported_version "$("$BASELINE_ARTIFACT" --version | awk 'NR==1 {print $NF}')" "$BASELINE_VERSION"
assert_eq candidate_reported_version "$("$CANDIDATE_ARTIFACT" --version | awk 'NR==1 {print $NF}')" "$CANDIDATE_VERSION"
assert_eq volume_control_reported_version "$("$VOLUME_CONTROL_ARTIFACT" --version | awk 'NR==1 {print $NF}')" "$CANDIDATE_VERSION"
BASELINE_FILE="$(file -b "$BASELINE_ARTIFACT")"
CANDIDATE_FILE="$(file -b "$CANDIDATE_ARTIFACT")"
VOLUME_CONTROL_FILE="$(file -b "$VOLUME_CONTROL_ARTIFACT")"
log "baseline_file=$BASELINE_FILE"
log "candidate_file=$CANDIDATE_FILE"
log "volume_control_file=$VOLUME_CONTROL_FILE"
[[ "$CANDIDATE_FILE" == *"ARM aarch64"* && "$CANDIDATE_FILE" == *"Android"* ]] || fail "candidate is not an AArch64 Android ELF executable"
[[ "$VOLUME_CONTROL_FILE" == *"ARM aarch64"* && "$VOLUME_CONTROL_FILE" == *"Android"* ]] || fail "volume-control candidate is not an AArch64 Android ELF executable"
BASELINE_SHA="$(file_sha "$BASELINE_ARTIFACT")"
CANDIDATE_SHA="$(file_sha "$CANDIDATE_ARTIFACT")"
VOLUME_CONTROL_SHA="$(file_sha "$VOLUME_CONTROL_ARTIFACT")"
log "baseline_sha256=$BASELINE_SHA"
log "candidate_sha256=$CANDIDATE_SHA"
log "volume_control_sha256=$VOLUME_CONTROL_SHA"

set +e
timeout -k 2 5 env -i \
  "HOME=$HOME" \
  "PREFIX=$PREFIX" \
  "PATH=$ORIGINAL_PATH" \
  MCP__AUTH__STATIC_TOKEN=device-smoke-compile-gate \
  MCP__ANDROID__VOLUME_CONTROL_ENABLED=true \
  MCP__CAPABILITY__KEY_ID=device-smoke-compile-gate \
  MCP__CAPABILITY__HMAC_KEY_HEX=0000000000000000000000000000000000000000000000000000000000000000 \
  MCP__SERVER__HOST=127.0.0.1 MCP__SERVER__PORT=18765 \
  "$CANDIDATE_ARTIFACT" >"$LOG_DIR/volume-control-compile-gate.log" 2>&1
volume_control_compile_rc=$?
set -e
((volume_control_compile_rc != 0 && volume_control_compile_rc != 124 && volume_control_compile_rc != 137)) || fail "incompatible candidate did not reject the volume-control runtime gate"
grep -Fq 'MCP__ANDROID__VOLUME_CONTROL_ENABLED requires a binary built with the android-volume-control feature' "$LOG_DIR/volume-control-compile-gate.log" || fail "incompatible candidate returned the wrong volume-control compile-gate error"
log "PASS volume_control_compile_gate=rejected_incompatible_artifact"
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
WRITE_CAPABILITY_GRANT_FILE="$CONFIG_ROOT/write-file-grant"
WRITE_CAPABILITY_CONTENT_FILE="$CONFIG_ROOT/write-file-content"
: >"$WRITE_CAPABILITY_GRANT_FILE"
: >"$WRITE_CAPABILITY_CONTENT_FILE"
chmod 600 "$WRITE_CAPABILITY_GRANT_FILE" "$WRITE_CAPABILITY_CONTENT_FILE"
cat >"$CONFIG_ROOT/runtime.env" <<EOF
MCP__AUTH__STATIC_TOKEN=$MCP_TOKEN
MCP__SERVER__HOST=127.0.0.1
MCP__SERVER__PORT=$PORT
MCP__TRANSPORT__ALLOWED_HOSTS=localhost:$PORT,127.0.0.1:$PORT
MCP__TRANSPORT__ALLOWED_ORIGINS=http://localhost:$PORT,http://127.0.0.1:$PORT
MCP__TRANSPORT__SSE_ENABLED=false
MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS=4
MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS=30
MCP__TRANSPORT__MAX_BODY_BYTES=2097152
MCP__FILE__SAFE_ROOTS=$SAFE_ROOT
MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=true
MCP__FILE__WRITE_MUTATION_ENABLED=true
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

volume_control_disabled_smoke

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
log "volume_control_sha256=$VOLUME_CONTROL_SHA"
log "TERMUX_MCP_DEVICE_RESULT=PASS"
SMOKE_SUCCEEDED=1
