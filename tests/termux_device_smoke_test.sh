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
  '"runtime_status","platform_info","android_status","project_service_status","list_directory","read_file","search_text","write_file"' \
  '"name":"shell"' \
  'mcp_request_body_too_large' \
  'outside-secret-must-not-be-returned' \
  'write_mode'
do
  assert_contains "$protocol_marker" "$SCRIPT"
done

printf 'Termux device smoke harness contract tests passed\n'
