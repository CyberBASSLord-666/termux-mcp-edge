#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT/scripts/termux_device_smoke.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

assert_contains() {
  local needle="$1" file="$2"
  grep -Fq -- "$needle" "$file" || fail "missing expected contract marker: $needle"
}

[[ -x "$SCRIPT" ]] || fail "device smoke harness must be executable"
bash -n "$SCRIPT"
assert_contains 'HARNESS_VERSION="8"' "$SCRIPT"
assert_contains 'valid_capability_grant()' "$SCRIPT"
assert_contains 'capability_grant_has_signed_byte "$grant" 64 01' "$SCRIPT"
assert_contains 'capability_grant_has_signed_byte "$grant" 16 02' "$SCRIPT"
assert_contains "--proto '=http' --noproxy '*' --connect-timeout 2 --max-time 10" "$SCRIPT"
if grep -Fq -- '{260}' "$SCRIPT"; then
  fail "device harness uses a non-portable ERE repetition above Android RE_DUP_MAX"
fi

mkdir -p "$TMP/home"
HOME="$TMP/home" bash "$SCRIPT" --help >"$TMP/help"
assert_contains 'TERMUX_MCP_SMOKE_EXPECTED_HEAD=<40-character-commit-sha>' "$TMP/help"
assert_contains 'TERMUX_MCP_SMOKE_FETCH_REF=<git-ref>' "$TMP/help"
[[ -z "$(find "$TMP/home" -mindepth 1 -print -quit)" ]] || fail "--help created state beneath HOME"

if HOME="$TMP/home" TERMUX_MCP_SMOKE_EXPECTED_HEAD=not-a-sha bash "$SCRIPT" >"$TMP/stdout" 2>"$TMP/stderr"; then
  fail "invalid expected head unexpectedly succeeded"
fi
assert_contains 'must be a full lowercase 40-character commit SHA' "$TMP/stderr"
[[ -z "$(find "$TMP/home" -mindepth 1 -print -quit)" ]] || fail "invalid input created state beneath HOME"

VALID_HEAD='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
if HOME="$TMP/home" TERMUX_MCP_SMOKE_EXPECTED_HEAD="$VALID_HEAD" TERMUX_MCP_SMOKE_FETCH_REF=--upload-pack=bad bash "$SCRIPT" >"$TMP/stdout" 2>"$TMP/stderr"; then
  fail "option-shaped fetch ref unexpectedly succeeded"
fi
assert_contains 'fetch_ref contains unsupported characters' <(tr '[:upper:]' '[:lower:]' <"$TMP/stderr")

if HOME="$TMP/home" TERMUX_MCP_SMOKE_EXPECTED_HEAD="$VALID_HEAD" TERMUX_MCP_SMOKE_BUILD_JOBS=0 bash "$SCRIPT" >"$TMP/stdout" 2>"$TMP/stderr"; then
  fail "zero build jobs unexpectedly succeeded"
fi
assert_contains 'must be a positive integer' "$TMP/stderr"
[[ -z "$(find "$TMP/home" -mindepth 1 -print -quit)" ]] || fail "invalid options created state beneath HOME"

for marker in \
  'cargo build --release --locked --features mcp-runtime' \
  'cargo build --release --locked --features android-volume-control' \
  'volume_control_compile_gate=rejected_incompatible_artifact' \
  'volume_control_disabled_runtime=verified_without_device_mutation' \
  'candidate_readiness_failure' \
  'successful_upgrade' \
  'protocol_smoke candidate' \
  'rollback_readiness_failure' \
  'successful_rollback' \
  'successful_uninstall' \
  'TERMUX_MCP_DEVICE_RESULT=PASS'
do
  assert_contains "$marker" "$SCRIPT"
done

for protocol_marker in \
  '"notifications/initialized"' \
  '"runtime_status","platform_info","android_status","project_service_status","create_directory","copy_file","find_paths","hash_file","list_directory","path_metadata","read_binary_file","read_binary_range","read_file","read_text_range","search_text","write_file"' \
  'create_directory_dry_run_http' \
  'create_directory_mode' \
  'create_directory_missing_grant_http' \
  'create_directory_replay_http' \
  '--issue-create-directory-grant' \
  '--issue-write-file-grant' \
  'MCP__CAPABILITY__CONFIG_FILE="$CONFIG_ROOT/runtime.env"' \
  'MCP__CAPABILITY__WRITE_FILE_TARGET="$target"' \
  'MCP__CAPABILITY__WRITE_FILE_CONTENT_FILE="$content_file"' \
  'MCP__CAPABILITY__WRITE_FILE_DISPOSITION="$disposition"' \
  'MCP__FILE__WRITE_MUTATION_ENABLED=true' \
  'MCP__TRANSPORT__MAX_BODY_BYTES=2097152' \
  'mcp_post_file()' \
  'write_file_grant_discovery' \
  'write_missing_grant_http' \
  'write_grant_mismatch_http' \
  'write_grant_replay_http' \
  'write_replace_http' \
  'write_replace_identity=fresh' \
  'write_replace_recovery' \
  'write_recovery_list=private' \
  'write_recovery_find' \
  'write_replace_binding_http' \
  'write_replace_substitute=preserved' \
  'write_replace_original=preserved' \
  'write_exact_1mib_http' \
  'write_1mib_plus_one_http' \
  'write_1mib_plus_one_grant_retry_http' \
  'write_response_preflight_http' \
  'write_response_preflight_retry_http' \
  '.termux-mcp-write-quarantine' \
  '^\.termux-mcp-write-artifact-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' \
  'recoveryArtifactRetained' \
  'androidVolumeControlCompiled == true' \
  'volume_control_runtime_disabled' \
  'copy_dry_run_http' \
  'copy_existing=unchanged' \
  'find_paths_http' \
  'find_paths_schema' \
  'pathDiscoveryMaxEntries == 8192' \
  'pathDiscoveryMaxMatches == 512' \
  'hash_file_http' \
  'hash_file=sha256' \
  'hash_file_schema' \
  'fileHashMaxBytes == 16777216' \
  'read_binary_file_http' \
  'read_binary_file=base64' \
  'read_binary_file_schema' \
  'binaryFileReadMaxBytes == 1048576' \
  'binaryFileReadMaxResponseBytes == 1507328' \
  'read_binary_range_http' \
  'read_binary_range=base64' \
  'read_binary_range_schema' \
  'binaryRangeReadMaxFileBytes == 67108864' \
  'binaryRangeReadMaxBytes == 262144' \
  'binaryRangeReadMaxResponseBytes == 393216' \
  'read_text_range_http' \
  'read_text_range=utf-8-boundaries' \
  'read_text_range_schema' \
  'textRangeReadMinBytes == 4' \
  'textRangeReadMaxFileBytes == 67108864' \
  'textRangeReadMaxBytes == 262144' \
  'textRangeReadMaxResponseBytes == 1703936' \
  '"name":"shell"' \
  'mcp_request_body_too_large' \
  'outside-secret-must-not-be-returned' \
  'write_mode'
do
  assert_contains "$protocol_marker" "$SCRIPT"
done

printf 'Termux device smoke harness contract tests passed\n'
