#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
umask 077

GATE_VERSION=1
EXPECTED_IMAGE='termux/termux-docker:aarch64'
DEFAULT_SAMPLES=256
MAX_SAMPLES=4096
DEFAULT_PORT=18766

DEFAULT_DIR=''
MCP_DIR=''
VOLUME_CONTROL_DIR=''
EXPECTED_COMMIT=''
EXPECTED_VERSION=''
CI_RUN_ID=''
SECURITY_RUN_ID=''
ANDROID_RUN_ID=''
OUTPUT_REPORT=''
SAMPLES="$DEFAULT_SAMPLES"
PORT="$DEFAULT_PORT"

STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
WORK_ROOT=''
SERVER_PID=''
SESSION_ID=''
REQUEST_COUNT=0
MCP_STATUS=''

log() { printf '[termux-emulated] %s\n' "$*"; }
fail() {
  printf 'TERMUX_MCP_EMULATED_RESULT=FAIL reason=%s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage: termux_emulated_gate.sh \
  --default-dir DIR \
  --mcp-dir DIR \
  --volume-control-dir DIR \
  --expected-commit SHA \
  --expected-version VERSION \
  --ci-run-id ID \
  --security-run-id ID \
  --android-run-id ID \
  --output REPORT.json \
  [--samples COUNT] \
  [--port PORT]

This gate must run natively on AArch64 inside the pinned official Termux
Docker environment. It validates exact workflow bundles through the canonical
runtime validator, then performs a bounded high-frequency MCP stability pass.
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
    --default-dir)
      (($# >= 2)) || fail missing_default_dir
      DEFAULT_DIR="$2"
      shift 2
      ;;
    --mcp-dir)
      (($# >= 2)) || fail missing_mcp_dir
      MCP_DIR="$2"
      shift 2
      ;;
    --volume-control-dir)
      (($# >= 2)) || fail missing_volume_control_dir
      VOLUME_CONTROL_DIR="$2"
      shift 2
      ;;
    --expected-commit)
      (($# >= 2)) || fail missing_expected_commit
      EXPECTED_COMMIT="$2"
      shift 2
      ;;
    --expected-version)
      (($# >= 2)) || fail missing_expected_version
      EXPECTED_VERSION="$2"
      shift 2
      ;;
    --ci-run-id)
      (($# >= 2)) || fail missing_ci_run_id
      CI_RUN_ID="$2"
      shift 2
      ;;
    --security-run-id)
      (($# >= 2)) || fail missing_security_run_id
      SECURITY_RUN_ID="$2"
      shift 2
      ;;
    --android-run-id)
      (($# >= 2)) || fail missing_android_run_id
      ANDROID_RUN_ID="$2"
      shift 2
      ;;
    --output)
      (($# >= 2)) || fail missing_output
      OUTPUT_REPORT="$2"
      shift 2
      ;;
    --samples)
      (($# >= 2)) || fail missing_samples
      SAMPLES="$2"
      shift 2
      ;;
    --port)
      (($# >= 2)) || fail missing_port
      PORT="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *) fail unknown_argument ;;
  esac
done

[[ "$EXPECTED_COMMIT" =~ ^[0-9a-f]{40}$ ]] || fail expected_commit_invalid
[[ "$EXPECTED_VERSION" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] || fail expected_version_invalid
[[ "$CI_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail ci_run_id_invalid
[[ "$SECURITY_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail security_run_id_invalid
[[ "$ANDROID_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail android_run_id_invalid
[[ "$SAMPLES" =~ ^[0-9]+$ ]] || fail samples_invalid
((SAMPLES >= 32 && SAMPLES <= MAX_SAMPLES)) || fail samples_out_of_range
[[ "$PORT" =~ ^[0-9]+$ ]] || fail port_invalid
((PORT >= 1024 && PORT <= 65535)) || fail port_invalid
[[ "$DEFAULT_DIR" == /* && "$MCP_DIR" == /* && "$VOLUME_CONTROL_DIR" == /* && "$OUTPUT_REPORT" == /* ]] || fail absolute_paths_required

[[ "${TERMUX_MCP_EMULATED_ENVIRONMENT:-}" == official-termux-docker-native-arm64 ]] || fail environment_attestation_missing
IMAGE_DIGEST="${TERMUX_MCP_TERMUX_IMAGE_DIGEST:-}"
[[ "$IMAGE_DIGEST" =~ ^sha256:[0-9a-f]{64}$ ]] || fail image_digest_invalid
[[ "$(uname -m)" == aarch64 || "$(uname -m)" == arm64 ]] || fail architecture_not_arm64
[[ "${PREFIX:-}" == /data/data/com.termux/files/usr ]] || fail termux_prefix_invalid
[[ "${HOME:-}" == /data/data/com.termux/files/home ]] || fail termux_home_invalid
[[ -x /system/bin/linker64 ]] || fail android_linker_missing

for command in awk bash cat chmod curl date dd dirname file find grep install jq kill mkdir mktemp mv readlink realpath rm sed seq sha256sum sleep stat timeout wc; do
  command -v "$command" >/dev/null 2>&1 || fail "required_command_missing_$command"
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VALIDATOR="$SCRIPT_DIR/termux_release_validate.sh"
DEPLOY_SCRIPT="$SCRIPT_DIR/termux_deploy.sh"
[[ -f "$VALIDATOR" && ! -L "$VALIDATOR" ]] || fail validator_invalid
[[ -f "$DEPLOY_SCRIPT" && ! -L "$DEPLOY_SCRIPT" ]] || fail deploy_script_invalid
bash -n "$VALIDATOR"
bash -n "$DEPLOY_SCRIPT"

DEFAULT_ARTIFACT="$DEFAULT_DIR/termux-mcp-server"
DEFAULT_MANIFEST="$DEFAULT_DIR/artifact-manifest.json"
DEFAULT_CHECKSUMS="$DEFAULT_DIR/SHA256SUMS"
MCP_ARTIFACT="$MCP_DIR/termux-mcp-server"
MCP_MANIFEST="$MCP_DIR/artifact-manifest.json"
MCP_CHECKSUMS="$MCP_DIR/SHA256SUMS"
VOLUME_CONTROL_ARTIFACT="$VOLUME_CONTROL_DIR/termux-mcp-server"
VOLUME_CONTROL_MANIFEST="$VOLUME_CONTROL_DIR/artifact-manifest.json"
VOLUME_CONTROL_CHECKSUMS="$VOLUME_CONTROL_DIR/SHA256SUMS"

for path in \
  "$DEFAULT_ARTIFACT" "$DEFAULT_MANIFEST" "$DEFAULT_CHECKSUMS" \
  "$MCP_ARTIFACT" "$MCP_MANIFEST" "$MCP_CHECKSUMS" \
  "$VOLUME_CONTROL_ARTIFACT" "$VOLUME_CONTROL_MANIFEST" "$VOLUME_CONTROL_CHECKSUMS"; do
  [[ -f "$path" && ! -L "$path" ]] || fail artifact_bundle_member_invalid
done
[[ -x "$DEFAULT_ARTIFACT" && -x "$MCP_ARTIFACT" && -x "$VOLUME_CONTROL_ARTIFACT" ]] || fail artifact_binary_not_executable
[[ "$(find "$DEFAULT_DIR" -mindepth 1 -maxdepth 1 -type f | wc -l)" == 3 ]] || fail default_bundle_member_count_invalid
[[ "$(find "$MCP_DIR" -mindepth 1 -maxdepth 1 -type f | wc -l)" == 3 ]] || fail mcp_bundle_member_count_invalid
[[ "$(find "$VOLUME_CONTROL_DIR" -mindepth 1 -maxdepth 1 -type f | wc -l)" == 3 ]] || fail volume_control_bundle_member_count_invalid
(cd "$DEFAULT_DIR" && sha256sum -c SHA256SUMS >/dev/null) || fail default_checksum_invalid
(cd "$MCP_DIR" && sha256sum -c SHA256SUMS >/dev/null) || fail mcp_checksum_invalid
(cd "$VOLUME_CONTROL_DIR" && sha256sum -c SHA256SUMS >/dev/null) || fail volume_control_checksum_invalid

validate_manifest() {
  local manifest="$1" artifact_name="$2" posture="$3" expected_features="$4"
  jq -e \
    --arg commit "$EXPECTED_COMMIT" \
    --arg version "$EXPECTED_VERSION" \
    --arg run_id "$ANDROID_RUN_ID" \
    --arg artifact_name "$artifact_name" \
    --arg posture "$posture" \
    --argjson features "$expected_features" '
      (keys == ["artifactName","bytes","commit","createdAt","elf","features","fileName","posture","repository","schemaVersion","sha256","target","version","workflowRunId"])
      and .schemaVersion == 1
      and .repository == "CyberBASSLord-666/termux-mcp-edge"
      and .commit == $commit
      and .workflowRunId == $run_id
      and .artifactName == $artifact_name
      and .posture == $posture
      and .features == $features
      and .target == "aarch64-linux-android"
      and .fileName == "termux-mcp-server"
      and .version == $version
      and .elf == "aarch64-android-elf"
      and (.sha256 | test("^[0-9a-f]{64}$"))
      and (.bytes >= 1 and .bytes <= 67108864)
    ' "$manifest" >/dev/null
}

validate_manifest "$DEFAULT_MANIFEST" termux-mcp-server-aarch64-linux-android-default default '[]' || fail default_manifest_invalid
validate_manifest "$MCP_MANIFEST" termux-mcp-server-aarch64-linux-android-mcp-runtime mcp-runtime '["mcp-runtime"]' || fail mcp_manifest_invalid
validate_manifest "$VOLUME_CONTROL_MANIFEST" termux-mcp-server-aarch64-linux-android-android-volume-control android-volume-control '["android-volume-control"]' || fail volume_control_manifest_invalid

DEFAULT_SHA="$(jq -r .sha256 "$DEFAULT_MANIFEST")"
MCP_SHA="$(jq -r .sha256 "$MCP_MANIFEST")"
VOLUME_CONTROL_SHA="$(jq -r .sha256 "$VOLUME_CONTROL_MANIFEST")"
DEFAULT_BYTES="$(jq -r .bytes "$DEFAULT_MANIFEST")"
MCP_BYTES="$(jq -r .bytes "$MCP_MANIFEST")"
VOLUME_CONTROL_BYTES="$(jq -r .bytes "$VOLUME_CONTROL_MANIFEST")"
[[ "$DEFAULT_SHA" != "$MCP_SHA" && "$DEFAULT_SHA" != "$VOLUME_CONTROL_SHA" && "$MCP_SHA" != "$VOLUME_CONTROL_SHA" ]] || fail artifact_postures_not_distinct
[[ "$(sha256sum "$DEFAULT_ARTIFACT" | awk '{print $1}')" == "$DEFAULT_SHA" ]] || fail default_digest_mismatch
[[ "$(sha256sum "$MCP_ARTIFACT" | awk '{print $1}')" == "$MCP_SHA" ]] || fail mcp_digest_mismatch
[[ "$(sha256sum "$VOLUME_CONTROL_ARTIFACT" | awk '{print $1}')" == "$VOLUME_CONTROL_SHA" ]] || fail volume_control_digest_mismatch
[[ "$(stat -c %s "$DEFAULT_ARTIFACT")" == "$DEFAULT_BYTES" ]] || fail default_size_mismatch
[[ "$(stat -c %s "$MCP_ARTIFACT")" == "$MCP_BYTES" ]] || fail mcp_size_mismatch
[[ "$(stat -c %s "$VOLUME_CONTROL_ARTIFACT")" == "$VOLUME_CONTROL_BYTES" ]] || fail volume_control_size_mismatch
[[ "$(timeout -k 2 5 "$DEFAULT_ARTIFACT" --version)" == "termux-mcp-server $EXPECTED_VERSION" ]] || fail default_version_mismatch
[[ "$(timeout -k 2 5 "$MCP_ARTIFACT" --version)" == "termux-mcp-server $EXPECTED_VERSION" ]] || fail mcp_version_mismatch
[[ "$(timeout -k 2 5 "$VOLUME_CONTROL_ARTIFACT" --version)" == "termux-mcp-server $EXPECTED_VERSION" ]] || fail volume_control_version_mismatch

OUTPUT_PARENT="$(dirname "$OUTPUT_REPORT")"
[[ -d "$OUTPUT_PARENT" && ! -L "$OUTPUT_PARENT" ]] || fail output_parent_invalid
[[ "$(realpath -e "$OUTPUT_PARENT")" == "$OUTPUT_PARENT" ]] || fail output_parent_not_canonical
[[ "$(stat -c %a "$OUTPUT_PARENT")" == 700 ]] || fail output_parent_not_private
[[ ! -e "$OUTPUT_REPORT" && ! -L "$OUTPUT_REPORT" ]] || fail output_already_exists

WORK_ROOT="$(mktemp -d "$HOME/.termux-mcp-emulated-gate.XXXXXX")" || fail work_root_create_failed
chmod 700 "$WORK_ROOT"
SAFE_ROOT="$WORK_ROOT/safe-root"
TOKEN_FILE="$WORK_ROOT/token"
CONFIG_FILE="$WORK_ROOT/release-validator.env"
RUNTIME_REPORT="$WORK_ROOT/runtime-evidence.json"
STRESS_LOG="$WORK_ROOT/stress-server.log"
BODY_FILE="$WORK_ROOT/body.json"
HEADER_FILE="$WORK_ROOT/headers.txt"
REQUEST_FILE="$WORK_ROOT/request.json"
mkdir -m 700 "$SAFE_ROOT"
printf '%s' emulated-visible >"$SAFE_ROOT/visible.txt"
chmod 600 "$SAFE_ROOT/visible.txt"

MCP_TOKEN="$(dd if=/dev/urandom bs=32 count=1 status=none | sha256sum | awk '{print $1}')"
[[ "$MCP_TOKEN" =~ ^[0-9a-f]{64}$ ]] || fail token_generation_failed
printf '%s' "$MCP_TOKEN" >"$TOKEN_FILE"
chmod 600 "$TOKEN_FILE"

cat >"$CONFIG_FILE" <<EOF
EXPECTED_COMMIT=$EXPECTED_COMMIT
EXPECTED_VERSION=$EXPECTED_VERSION
DEFAULT_ARTIFACT=$DEFAULT_ARTIFACT
DEFAULT_SHA256=$DEFAULT_SHA
DEFAULT_MANIFEST=$DEFAULT_MANIFEST
MCP_ARTIFACT=$MCP_ARTIFACT
MCP_SHA256=$MCP_SHA
MCP_MANIFEST=$MCP_MANIFEST
VOLUME_CONTROL_ARTIFACT=$VOLUME_CONTROL_ARTIFACT
VOLUME_CONTROL_SHA256=$VOLUME_CONTROL_SHA
VOLUME_CONTROL_MANIFEST=$VOLUME_CONTROL_MANIFEST
AUTH_TOKEN_FILE=$TOKEN_FILE
SAFE_ROOT=$SAFE_ROOT
BIND_HOST=127.0.0.1
PORT=$PORT
DEPLOY_SCRIPT=$DEPLOY_SCRIPT
CI_RUN_ID=$CI_RUN_ID
SECURITY_RUN_ID=$SECURITY_RUN_ID
ANDROID_RUN_ID=$ANDROID_RUN_ID
SUSTAINED_OBSERVATION_STATUS=not_run
SUSTAINED_OBSERVATION_MINUTES=0
SUSTAINED_OBSERVATION_REASON_CODE=not_observed
EOF
chmod 600 "$CONFIG_FILE"

log 'running canonical exact-artifact runtime validator'
bash "$VALIDATOR" \
  --config "$CONFIG_FILE" \
  --report "$RUNTIME_REPORT" \
  --phase runtime \
  --confirm-runtime-mutation

jq -e \
  --arg commit "$EXPECTED_COMMIT" \
  --arg version "$EXPECTED_VERSION" \
  --arg ci "$CI_RUN_ID" \
  --arg security "$SECURITY_RUN_ID" \
  --arg android "$ANDROID_RUN_ID" '
    .status == "pass"
    and .failureCode == null
    and .releaseEligible == false
    and .repository == {commit:$commit,version:$version,ciRunId:$ci,securityRunId:$security,androidRunId:$android}
    and .phases == {preflight:"pass",runtime:"pass",deployment:"not_run"}
    and .sustainedObservation.status == "not_run"
    and ([.results[].code] | index("default_posture_verified") != null)
    and ([.results[].code] | index("mcp_posture_verified") != null)
    and ([.results[].code] | index("volume_control_posture_verified") != null)
    and ([.results[].code] | index("volume_control_hidden_while_disabled") != null)
    and ([.results[].code] | index("volume_control_disabled_call_rejected") != null)
    and ([.results[].code] | index("exact_tool_allowlist") != null)
    and ([.results[].code] | index("request_scoped_single_use_grant_enforced") != null)
    and ([.results[].code] | index("symlink_escape_rejected") != null)
    and ([.results[].code] | index("authentication_precedes_body_limit") != null)
  ' "$RUNTIME_REPORT" >/dev/null || fail runtime_report_invalid
RUNTIME_REPORT_SHA="$(sha256sum "$RUNTIME_REPORT" | awk '{print $1}')"
RUNTIME_RESULT_COUNT="$(jq '.results | length' "$RUNTIME_REPORT")"

curl_local() {
  command curl --disable --proto '=http' --noproxy '*' --connect-timeout 2 --max-time 10 "$@"
}

start_stress_server() {
  MCP__AUTH__STATIC_TOKEN="$MCP_TOKEN" \
  MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=false \
  MCP__SERVER__HOST=127.0.0.1 \
  MCP__SERVER__PORT="$PORT" \
  MCP__TRANSPORT__ALLOWED_HOSTS="localhost:$PORT,127.0.0.1:$PORT" \
  MCP__TRANSPORT__ALLOWED_ORIGINS="http://localhost:$PORT,http://127.0.0.1:$PORT" \
  MCP__TRANSPORT__ALLOW_MISSING_ORIGIN=false \
  MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS=4 \
  MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS=30 \
  MCP__TRANSPORT__MAX_BODY_BYTES=1024 \
  MCP__FILE__SAFE_ROOTS="$SAFE_ROOT" \
  RUST_LOG=termux_mcp_server=info \
    "$MCP_ARTIFACT" >"$STRESS_LOG" 2>&1 &
  SERVER_PID=$!
  local attempt
  for attempt in $(seq 1 100); do
    kill -0 "$SERVER_PID" >/dev/null 2>&1 || fail stress_server_exited
    if [[ "$(curl_local --silent --max-time 2 "http://127.0.0.1:$PORT/health" 2>/dev/null || true)" == ok ]]; then
      return 0
    fi
    sleep 0.1
  done
  fail "stress_server_not_ready_after_${attempt}_attempts"
}

post_mcp() {
  local payload="$1" session="${2:-}"
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
  MCP_STATUS="$(curl_local --silent --show-error --output "$BODY_FILE" --write-out '%{http_code}' \
    "${headers[@]}" --data-binary "@$REQUEST_FILE" "http://127.0.0.1:$PORT/mcp")"
  REQUEST_COUNT=$((REQUEST_COUNT + 1))
}

log "running $SAMPLES-sample native ARM64 stress pass"
start_stress_server
STRESS_PID="$SERVER_PID"

printf '%s' '{"jsonrpc":"2.0","id":"initialize","method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"termux-emulated-gate","version":"1"}}}' >"$REQUEST_FILE"
status="$(curl_local --silent --show-error --dump-header "$HEADER_FILE" --output "$BODY_FILE" --write-out '%{http_code}' \
  -H "Authorization: Bearer $MCP_TOKEN" \
  -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
  -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  --data-binary "@$REQUEST_FILE" "http://127.0.0.1:$PORT/mcp")"
REQUEST_COUNT=$((REQUEST_COUNT + 1))
[[ "$status" == 200 ]] || fail stress_initialize_status_invalid
jq -e --arg version "$EXPECTED_VERSION" '.result.protocolVersion == "2025-11-25" and .result.serverInfo.version == $version' "$BODY_FILE" >/dev/null || fail stress_initialize_body_invalid
SESSION_ID="$(awk 'tolower($1) == "mcp-session-id:" {sub(/^[^:]*:[[:space:]]*/, ""); sub(/\r$/, ""); print; exit}' "$HEADER_FILE")"
[[ "$SESSION_ID" =~ ^[A-Za-z0-9-]{1,128}$ ]] || fail stress_session_invalid

post_mcp '{"jsonrpc":"2.0","method":"notifications/initialized"}' "$SESSION_ID"
[[ "$MCP_STATUS" == 202 && ! -s "$BODY_FILE" ]] || fail stress_initialized_notification_invalid

for sample in $(seq 1 "$SAMPLES"); do
  kill -0 "$STRESS_PID" >/dev/null 2>&1 || fail stress_pid_lost
  [[ "$SERVER_PID" == "$STRESS_PID" ]] || fail stress_pid_changed
  [[ "$(curl_local --fail --silent "http://127.0.0.1:$PORT/health")" == ok ]] || fail stress_health_invalid
  REQUEST_COUNT=$((REQUEST_COUNT + 1))
  curl_local --fail --silent "http://127.0.0.1:$PORT/ready" >"$BODY_FILE"
  REQUEST_COUNT=$((REQUEST_COUNT + 1))
  jq -e --arg version "$EXPECTED_VERSION" '
    .status == "ready" and .version == $version and .mcp_runtime_enabled == true
    and .safe_root_count == 1 and .auth_posture == "static_token"
  ' "$BODY_FILE" >/dev/null || fail stress_readiness_invalid

  post_mcp '{"jsonrpc":"2.0","id":"tools","method":"tools/list"}' "$SESSION_ID"
  [[ "$MCP_STATUS" == 200 ]] || fail stress_tools_status_invalid
  jq -e '[.result.tools[].name] == ["runtime_status","platform_info","android_status","project_service_status","create_directory","copy_file","hash_file","list_directory","path_metadata","read_binary_file","read_binary_range","read_file","search_text","write_file"]' "$BODY_FILE" >/dev/null || fail stress_tool_allowlist_invalid
  jq -e '
    .result.tools
    | map(select(.name == "create_directory"))[0] as $tool
    | $tool.inputSchema.properties.dry_run.const == true
      and ($tool.description | contains("mutation gate is disabled"))
  ' "$BODY_FILE" >/dev/null || fail stress_create_directory_disabled_posture_invalid

  if ((sample % 16 == 0)); then
    post_mcp '{"jsonrpc":"2.0","id":"runtime","method":"tools/call","params":{"name":"runtime_status","arguments":{}}}' "$SESSION_ID"
    [[ "$MCP_STATUS" == 200 ]] || fail stress_runtime_status_invalid
    jq -e '
      .result.structuredContent.commandExecution == false
      and .result.structuredContent.androidPlatformTools == false
      and .result.structuredContent.androidBatteryStatusCompiled == false
      and .result.structuredContent.androidBatteryStatusEnabled == false
      and .result.structuredContent.androidVolumeStatusCompiled == false
      and .result.structuredContent.androidVolumeStatusEnabled == false
      and .result.structuredContent.androidDeviceControl == false
      and .result.structuredContent.createDirectoryMutationEnabled == false
      and .result.structuredContent.createDirectoryGrantRequired == false
      and .result.structuredContent.createDirectoryMutationMode == "dry_run_only_mutation_disabled"
      and .result.structuredContent.highImpactTools == false
    ' "$BODY_FILE" >/dev/null || fail stress_high_impact_gate_invalid
  fi

  if ((sample == 1)); then
    post_mcp '{"jsonrpc":"2.0","id":"battery-uncompiled","method":"tools/call","params":{"name":"android_battery_status","arguments":{}}}' "$SESSION_ID"
    [[ "$MCP_STATUS" == 200 ]] || fail stress_battery_uncompiled_status_invalid
    jq -e '
      .result.isError == true
      and .result.structuredContent.error == "android_battery_status_unavailable"
      and .result.structuredContent.reasonCode == "battery_feature_not_compiled"
    ' "$BODY_FILE" >/dev/null || fail stress_battery_uncompiled_contract_invalid

    post_mcp '{"jsonrpc":"2.0","id":"volume-uncompiled","method":"tools/call","params":{"name":"android_volume_status","arguments":{}}}' "$SESSION_ID"
    [[ "$MCP_STATUS" == 200 ]] || fail stress_volume_uncompiled_status_invalid
    jq -e '
      .result.isError == true
      and .result.structuredContent.error == "android_volume_status_unavailable"
      and .result.structuredContent.reasonCode == "volume_feature_not_compiled"
    ' "$BODY_FILE" >/dev/null || fail stress_volume_uncompiled_contract_invalid
  fi
done

status="$(curl_local --silent --show-error --output "$BODY_FILE" --write-out '%{http_code}' -X DELETE \
  -H "Authorization: Bearer $MCP_TOKEN" -H "MCP-Session-Id: $SESSION_ID" \
  -H 'MCP-Protocol-Version: 2025-11-25' \
  -H "Host: localhost:$PORT" -H "Origin: http://localhost:$PORT" \
  "http://127.0.0.1:$PORT/mcp")"
REQUEST_COUNT=$((REQUEST_COUNT + 1))
[[ "$status" == 204 && ! -s "$BODY_FILE" ]] || fail stress_session_delete_invalid
unset SESSION_ID

kill "$SERVER_PID" >/dev/null 2>&1 || fail stress_shutdown_signal_failed
wait "$SERVER_PID" 2>/dev/null || true
SERVER_PID=''

COMPLETED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
REPORT_NEXT="$WORK_ROOT/emulated-evidence.json"
jq -n \
  --arg gate_version "$GATE_VERSION" \
  --arg started_at "$STARTED_AT" \
  --arg completed_at "$COMPLETED_AT" \
  --arg commit "$EXPECTED_COMMIT" \
  --arg version "$EXPECTED_VERSION" \
  --arg ci_run_id "$CI_RUN_ID" \
  --arg security_run_id "$SECURITY_RUN_ID" \
  --arg android_run_id "$ANDROID_RUN_ID" \
  --arg default_sha "$DEFAULT_SHA" \
  --argjson default_bytes "$DEFAULT_BYTES" \
  --arg mcp_sha "$MCP_SHA" \
  --argjson mcp_bytes "$MCP_BYTES" \
  --arg volume_control_sha "$VOLUME_CONTROL_SHA" \
  --argjson volume_control_bytes "$VOLUME_CONTROL_BYTES" \
  --arg architecture "$(uname -m)" \
  --arg image "$EXPECTED_IMAGE" \
  --arg image_digest "$IMAGE_DIGEST" \
  --arg runtime_report_sha "$RUNTIME_REPORT_SHA" \
  --argjson runtime_results "$RUNTIME_RESULT_COUNT" \
  --argjson samples "$SAMPLES" \
  --argjson requests "$REQUEST_COUNT" '
  {
    schemaVersion: 1,
    gateVersion: $gate_version,
    status: "pass",
    failureCode: null,
    startedAt: $started_at,
    completedAt: $completed_at,
    candidate: {
      commit: $commit,
      version: $version,
      ciRunId: $ci_run_id,
      securityRunId: $security_run_id,
      androidRunId: $android_run_id,
      defaultArtifact: {sha256: $default_sha, bytes: $default_bytes},
      mcpRuntimeArtifact: {sha256: $mcp_sha, bytes: $mcp_bytes},
      androidVolumeControlArtifact: {sha256: $volume_control_sha, bytes: $volume_control_bytes}
    },
    environment: {
      executionMode: "official-termux-docker-native-arm64",
      architecture: $architecture,
      image: $image,
      imageDigest: $image_digest,
      androidLinker: true
    },
    runtimeValidation: {
      status: "pass",
      reportSha256: $runtime_report_sha,
      resultCount: $runtime_results,
      phases: {preflight: "pass", runtime: "pass", deployment: "not_run"}
    },
    stress: {
      status: "pass",
      samples: $samples,
      requests: $requests,
      servicePidStable: true,
      healthReadyStable: true,
      sessionLifecycle: true,
      exactToolAllowlist: true,
      highImpactDisabled: true
    }
  }' >"$REPORT_NEXT"
chmod 600 "$REPORT_NEXT"

jq -e '
  .schemaVersion == 1 and .gateVersion == "1" and .status == "pass" and .failureCode == null
  and .environment.executionMode == "official-termux-docker-native-arm64"
  and (.candidate.androidVolumeControlArtifact.sha256 | test("^[0-9a-f]{64}$"))
  and .candidate.androidVolumeControlArtifact.sha256 != .candidate.defaultArtifact.sha256
  and .candidate.androidVolumeControlArtifact.sha256 != .candidate.mcpRuntimeArtifact.sha256
  and (.environment.architecture == "aarch64" or .environment.architecture == "arm64")
  and .environment.androidLinker == true
  and .runtimeValidation.status == "pass"
  and .stress.status == "pass" and .stress.samples >= 32 and .stress.requests >= (.stress.samples * 3)
  and .stress.servicePidStable == true and .stress.healthReadyStable == true
  and .stress.sessionLifecycle == true and .stress.exactToolAllowlist == true
  and .stress.highImpactDisabled == true
' "$REPORT_NEXT" >/dev/null || fail generated_report_invalid

if grep -Eq '/data/|Bearer[[:space:]]|MCP__|[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}' "$REPORT_NEXT"; then
  fail generated_report_not_sanitized
fi
install -m 600 "$REPORT_NEXT" "$OUTPUT_REPORT" || fail report_publication_failed

REPORT_SHA="$(sha256sum "$OUTPUT_REPORT" | awk '{print $1}')"
log "report_sha256=$REPORT_SHA"
log "report=$OUTPUT_REPORT"
log "samples=$SAMPLES"
log "requests=$REQUEST_COUNT"
log 'TERMUX_MCP_EMULATED_RESULT=PASS'
