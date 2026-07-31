#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'

ROOT="$(mktemp -d)"
trap 'find "$ROOT" -depth -mindepth 1 -delete; rmdir "$ROOT"' EXIT INT TERM
chmod 700 "$ROOT"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
SCRIPT="$REPO_ROOT/scripts/resolve_shizuku_rish_candidate.sh"
MOCK_CURL="$REPO_ROOT/tests/fixtures/rish_candidate_mock_curl.py"
REAL_PATH="$PATH"
COMMIT="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

fail_test() {
  printf 'Shizuku/rish candidate resolver test failed: %s\n' "$1" >&2
  exit 1
}

run_resolver() {
  local case_name="$1" output="$2"
  MOCK_CASE="$case_name" \
  GH_TOKEN="fixture-token" \
  GITHUB_API_URL="https://api.github.invalid" \
  PATH="$ROOT/fake-bin:$REAL_PATH" \
    bash "$SCRIPT" \
      --repository CyberBASSLord-666/termux-mcp-edge \
      --commit "$COMMIT" \
      --pull-request 328 \
      --output "$output"
}

assert_case_fails() {
  local case_name="$1" reason="$2"
  local output="$ROOT/output-$case_name.json"
  if run_resolver "$case_name" "$output" \
    >"$ROOT/$case_name.stdout" 2>"$ROOT/$case_name.stderr"
  then
    fail_test "case unexpectedly passed: $case_name"
  fi
  grep -Fq "$reason" "$ROOT/$case_name.stderr" \
    || fail_test "case did not report $reason: $case_name"
  [[ ! -e "$output" && ! -L "$output" ]] \
    || fail_test "failed case published output: $case_name"
}

[[ -x "$SCRIPT" && ! -L "$SCRIPT" ]] || fail_test resolver_not_executable
[[ -x "$MOCK_CURL" && ! -L "$MOCK_CURL" ]] || fail_test mock_curl_not_executable
mkdir -m 700 "$ROOT/fake-bin"
ln -s "$MOCK_CURL" "$ROOT/fake-bin/curl"

PASS_OUTPUT="$ROOT/pass.json"
if ! run_resolver pass "$PASS_OUTPUT" >"$ROOT/pass.stdout" 2>"$ROOT/pass.stderr"; then
  cat "$ROOT/pass.stderr" >&2
  fail_test valid_candidate_rejected
fi
[[ "$(<"$ROOT/pass.stdout")" == '[android-rish-candidate] result=PASS' ]] \
  || fail_test success_output_contract_changed
[[ ! -s "$ROOT/pass.stderr" ]] || fail_test successful_resolver_wrote_stderr
[[ "$(stat -c '%a' "$PASS_OUTPUT")" == 600 ]] || fail_test output_mode_invalid
jq -e '
  keys == [
    "androidRunId",
    "ciRunId",
    "commit",
    "headBranch",
    "pullRequestNumber",
    "repository",
    "runAttempt",
    "schemaVersion",
    "securityRunId",
    "workflowEvent"
  ]
  and .schemaVersion == 1
  and .repository == "CyberBASSLord-666/termux-mcp-edge"
  and .commit == "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  and .pullRequestNumber == 328
  and .headBranch == "next/android-rish"
  and .workflowEvent == "pull_request"
  and .runAttempt == 1
  and .ciRunId == 101
  and .securityRunId == 102
  and .androidRunId == 103
' "$PASS_OUTPUT" >/dev/null || fail_test success_identity_invalid
if grep -Eq 'reviewer|candidate-author' "$PASS_OUTPUT"; then
  fail_test reviewer_identity_leaked
fi

assert_case_fails fork pull_request_identity_invalid
assert_case_fails closed pull_request_identity_invalid
assert_case_fails moved pull_request_identity_invalid
assert_case_fails non_main_base pull_request_identity_invalid
assert_case_fails draft pull_request_identity_invalid
assert_case_fails insufficient pull_request_reviews_invalid
assert_case_fails outdated pull_request_reviews_invalid
assert_case_fails dismissed pull_request_reviews_invalid
assert_case_fails changes_requested pull_request_reviews_invalid
assert_case_fails commented_after_approval pull_request_reviews_invalid
assert_case_fails duplicate_reviewer pull_request_reviews_invalid
assert_case_fails self_approval pull_request_reviews_invalid
assert_case_fails untrusted_approval pull_request_reviews_invalid
assert_case_fails review_page_full pull_request_reviews_invalid
assert_case_fails missing_companion workflow_run_selection_failed
assert_case_fails stale_latest_failure workflow_run_identity_invalid
assert_case_fails rerun workflow_run_identity_invalid
assert_case_fails incomplete_run_page workflow_run_selection_failed

EXISTING="$ROOT/existing.json"
printf '%s\n' preserve >"$EXISTING"
if run_resolver pass "$EXISTING" >"$ROOT/existing.stdout" 2>"$ROOT/existing.stderr"; then
  fail_test existing_output_unexpectedly_accepted
fi
grep -Fq output_invalid "$ROOT/existing.stderr" \
  || fail_test existing_output_failure_changed
[[ "$(<"$EXISTING")" == preserve ]] || fail_test existing_output_modified

SYMLINK_TARGET="$ROOT/symlink-target.json"
printf '%s\n' preserve >"$SYMLINK_TARGET"
SYMLINK_OUTPUT="$ROOT/symlink-output.json"
ln -s "$SYMLINK_TARGET" "$SYMLINK_OUTPUT"
if run_resolver pass "$SYMLINK_OUTPUT" \
  >"$ROOT/symlink.stdout" 2>"$ROOT/symlink.stderr"
then
  fail_test symlink_output_unexpectedly_accepted
fi
grep -Fq output_invalid "$ROOT/symlink.stderr" \
  || fail_test symlink_output_failure_changed
[[ "$(<"$SYMLINK_TARGET")" == preserve ]] || fail_test symlink_target_modified

if env -u GH_TOKEN \
  MOCK_CASE=pass \
  GITHUB_API_URL=https://api.github.invalid \
  PATH="$ROOT/fake-bin:$REAL_PATH" \
  bash "$SCRIPT" \
    --repository CyberBASSLord-666/termux-mcp-edge \
    --commit "$COMMIT" \
    --pull-request 328 \
    --output "$ROOT/no-token.json" \
    >"$ROOT/no-token.stdout" 2>"$ROOT/no-token.stderr"
then
  fail_test missing_token_unexpectedly_accepted
fi
grep -Fq github_token_missing "$ROOT/no-token.stderr" \
  || fail_test missing_token_failure_changed

[[ -z "$(find "$ROOT" -maxdepth 1 -type d -name '.android-rish-candidate.*' -print -quit)" ]] \
  || fail_test resolver_left_private_temporary_state

printf 'Shizuku/rish candidate resolver tests passed\n'
