#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077

ROOT="$(mktemp -d)"
trap 'rm -rf -- "$ROOT"' EXIT INT TERM
chmod 700 "$ROOT"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/package_physical_qualification.sh"
SCHEMA="$REPO_ROOT/docs/release-physical-qualification-schema-v1.json"
BUNDLE_PARENT="$ROOT/bundles"
mkdir -m 700 "$BUNDLE_PARENT"

COMMIT=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
OTHER_COMMIT=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
VERSION=0.6.0
DEFAULT_SHA=1111111111111111111111111111111111111111111111111111111111111111
MCP_SHA=2222222222222222222222222222222222222222222222222222222222222222
VOLUME_SHA=3333333333333333333333333333333333333333333333333333333333333333
WORKFLOW_SHA=4444444444444444444444444444444444444444444444444444444444444444
BASELINE_SHA=5555555555555555555555555555555555555555555555555555555555555555
NATIVE_MCP_SHA=6666666666666666666666666666666666666666666666666666666666666666
NATIVE_VOLUME_SHA=7777777777777777777777777777777777777777777777777777777777777777
NATIVE_SHA=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
OTHER_SHA=cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc
PRIVATE_TOKEN=fixture-private-token-never-dispatch

fail_test() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

sha() {
  sha256sum -- "$1" | awk '{print $1}'
}

write_validator() {
  local path="$1" full_suite_sha="${2:-$WORKFLOW_SHA}"
  jq -n \
    --arg commit "$COMMIT" \
    --arg version "$VERSION" \
    --arg default_sha "$DEFAULT_SHA" \
    --arg mcp_sha "$MCP_SHA" \
    --arg volume_sha "$VOLUME_SHA" \
    --arg full_suite_sha "$full_suite_sha" \
    --arg baseline_sha "$BASELINE_SHA" '
    {
      schemaVersion: 2,
      validatorVersion: "11",
      status: "pass",
      failureCode: null,
      releaseEligible: true,
      startedAt: "2026-07-22T12:00:00Z",
      completedAt: "2026-07-22T13:05:00Z",
      repository: {
        commit: $commit,
        version: $version,
        ciRunId: "1001",
        securityRunId: "1002",
        androidRunId: "1003"
      },
      environment: {
        architecture: "aarch64",
        fixtureMode: false,
        tools: {
          bash: "GNU bash, version 5.3.0",
          curl: "curl 8.14.1 libcurl/8.14.1",
          file: "file-5.46",
          jq: "jq-1.7.1"
        }
      },
      requestedPhase: "all",
      artifacts: {
        default: {sha256: $default_sha, bytes: 1001, version: $version, elf: "aarch64-android-elf"},
        mcpRuntime: {sha256: $mcp_sha, bytes: 1002, version: $version, elf: "aarch64-android-elf"},
        androidVolumeControl: {sha256: $volume_sha, bytes: 1003, version: $version, elf: "aarch64-android-elf"},
        fullSuite: {sha256: $full_suite_sha, bytes: 1004, version: $version, elf: "aarch64-android-elf"},
        baseline: {sha256: $baseline_sha, bytes: 900, version: "0.5.0", elf: "aarch64-android-elf"}
      },
      deploymentCandidate: {posture: "full-suite", productionAction: null},
      phases: {preflight: "pass", runtime: "pass", deployment: "pass"},
      results: [
        {phase:"runtime",check:"full_suite_default_posture",outcome:"pass",code:"full_suite_default_disabled_17_tool_posture_verified"},
        {phase:"runtime",check:"full_suite_battery_gate",outcome:"pass",code:"full_suite_battery_runtime_gate_independence_verified"},
        {phase:"runtime",check:"full_suite_volume_status_gate",outcome:"pass",code:"full_suite_volume_status_runtime_gate_independence_verified"},
        {phase:"runtime",check:"full_suite_volume_control_gate",outcome:"pass",code:"full_suite_volume_control_runtime_gate_independence_verified"},
        {phase:"runtime",check:"full_suite_command_gate",outcome:"pass",code:"full_suite_command_runtime_gate_independence_verified"},
        {phase:"runtime",check:"full_suite_enabled_posture",outcome:"pass",code:"full_suite_enabled_21_tool_posture_verified"},
        {phase:"runtime",check:"full_suite_optional_providers",outcome:"pass",code:"full_suite_optional_provider_success_verified"},
        {phase:"runtime",check:"full_suite_volume_boundary",outcome:"pass",code:"full_suite_volume_preview_and_grant_boundary_verified"},
        {phase:"runtime",check:"full_suite_command_profile",outcome:"pass",code:"full_suite_command_basename_and_profile_verified"},
        {phase:"runtime",check:"full_suite_filesystem_posture",outcome:"pass",code:"full_suite_filesystem_mutations_independently_disabled"},
        {phase:"deployment",check:"deployment_candidate_posture",outcome:"pass",code:"full_suite_deployment_candidate_selected"}
      ],
      sustainedObservation: {
        operatorSupplied: true,
        status: "pass",
        minutes: 65,
        reasonCode: "stable",
        minimumMinutes: 60
      }
    }
  ' >"$path"
  chmod 600 "$path"
}

write_harness() {
  local path="$1"
  local commit="${2:-$COMMIT}"
  local version="${3:-$VERSION}"
  local candidate_sha="${4:-$NATIVE_SHA}"
  local full_suite_sha="${5:-$NATIVE_SHA}"
  local cleanup="${6:-true}"
  local final_status="${7:-PASS}"
  local result="${8:-PASS}"
  local architecture="${9:-aarch64}"
  {
    printf '%s\n' \
      'Termux MCP exact-commit device production gate starting' \
      'harness_version=11' \
      "report=/data/data/com.termux/files/home/private/$PRIVATE_TOKEN/report.txt" \
      "work_root=$ROOT/device-private-work-root" \
      "session=$PRIVATE_TOKEN" \
      "architecture=$architecture" \
      "candidate_version=$version" \
      "mcp_runtime_sha256=$NATIVE_MCP_SHA" \
      "volume_control_sha256=$NATIVE_VOLUME_SHA" \
      "full_suite_sha256=$full_suite_sha" \
      "exact_head=$commit" \
      "candidate_sha256=$candidate_sha" \
      "mcp_runtime_sha256=$NATIVE_MCP_SHA" \
      "volume_control_sha256=$NATIVE_VOLUME_SHA" \
      "full_suite_sha256=$full_suite_sha" \
      "TERMUX_MCP_DEVICE_RESULT=$result" \
      "cleanup_complete=$cleanup" \
      "final_status=$final_status"
  } >"$path"
  chmod 600 "$path"
}

make_validator_variant() {
  local source="$1" destination="$2" filter="$3"
  jq "$filter" "$source" >"$destination"
  chmod 600 "$destination"
}

run_package() {
  bash "$SCRIPT" \
    --validator-report "$1" \
    --harness-report "$2" \
    --output-dir "$3"
}

assert_package_fails() {
  local validator="$1" harness="$2" output="$3" reason="$4"
  if run_package "$validator" "$harness" "$output" >"$ROOT/last.stdout" 2>"$ROOT/last.stderr"; then
    fail_test "package unexpectedly succeeded for $reason"
  fi
  grep -Fq "reason=$reason" "$ROOT/last.stderr" \
    || fail_test "expected failure reason $reason was absent"
  [[ ! -e "$output" && ! -L "$output" ]] \
    || fail_test "failed package $reason published output"
  [[ -z "$(find "$BUNDLE_PARENT" -maxdepth 1 -name '.physical-qualification.*' -print -quit)" ]] \
    || fail_test "failed package $reason left staging state"
  assert_no_private_data "$ROOT/last.stdout" "$ROOT/last.stderr"
}

assert_no_private_data() {
  local file
  for file in "$@"; do
    if grep -Fq "$PRIVATE_TOKEN" "$file" || grep -Fq "$ROOT" "$file" \
      || grep -Fq '/data/data/com.termux/files/home/private' "$file"; then
      fail_test "private device data escaped into $(basename "$file")"
    fi
  done
}

bash -n "$SCRIPT"
bash -n "${BASH_SOURCE[0]}"
jq -e '
  ."$schema" == "https://json-schema.org/draft/2020-12/schema"
  and .type == "object"
  and .additionalProperties == false
  and ((.required | length) == 20 and (.required | unique | length) == 20)
  and (.properties | keys == ["androidRunId","architecture","ciRunId","cleanupConfirmed","commit","envelopeVersion","failureCode","harnessPassed","harnessVersion","nativeFullSuiteSha256","rawHarnessReportSha256","releaseEligible","repository","schemaVersion","securityRunId","status","validatorReportSha256","validatorVersion","version","workflowFullSuiteSha256"])
  and .properties.schemaVersion.const == 1
  and .properties.envelopeVersion.const == "1"
  and .properties.repository.const == "CyberBASSLord-666/termux-mcp-edge"
  and .properties.validatorVersion.const == "11"
  and .properties.harnessVersion.const == "11"
  and .properties.architecture.const == "aarch64"
  and (.description | contains("byte equality is neither required nor asserted"))
' "$SCHEMA" >/dev/null

bash "$SCRIPT" --help >"$ROOT/help.stdout" 2>"$ROOT/help.stderr"
grep -Fq 'physical-qualification-v1.json' "$ROOT/help.stdout" \
  || fail_test "help omits envelope filename"
grep -Fq 'release-validator-v11.json' "$ROOT/help.stdout" \
  || fail_test "help omits validator filename"
grep -Fq 'never copied into the bundle' "$ROOT/help.stdout" \
  || fail_test "help omits raw harness exclusion"
grep -Fq 'equality is neither required nor asserted' "$ROOT/help.stdout" \
  || fail_test "help overclaims cross-toolchain byte identity"
[[ ! -s "$ROOT/help.stderr" ]] || fail_test "help wrote stderr"

VALIDATOR="$ROOT/release-validator-source.json"
HARNESS="$ROOT/raw-device-harness.txt"
write_validator "$VALIDATOR"
write_harness "$HARNESS"
VALIDATOR_SHA="$(sha "$VALIDATOR")"
HARNESS_SHA="$(sha "$HARNESS")"
BUNDLE="$BUNDLE_PARENT/pass"
run_package "$VALIDATOR" "$HARNESS" "$BUNDLE" >"$ROOT/pass.stdout" 2>"$ROOT/pass.stderr"
[[ "$(<"$ROOT/pass.stdout")" == PHYSICAL_QUALIFICATION_PACKAGE_RESULT=PASS ]] \
  || fail_test "success output contract changed"
[[ ! -s "$ROOT/pass.stderr" ]] || fail_test "successful package wrote stderr"
[[ "$(stat -c '%a' "$BUNDLE")" == 700 ]] || fail_test "bundle mode is not 700"
[[ "$(stat -c '%a' "$BUNDLE/physical-qualification-v1.json")" == 600 ]] \
  || fail_test "envelope mode is not 600"
[[ "$(stat -c '%a' "$BUNDLE/release-validator-v11.json")" == 600 ]] \
  || fail_test "validator copy mode is not 600"
mapfile -t BUNDLE_FILES < <(find "$BUNDLE" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)
[[ "${#BUNDLE_FILES[@]}" == 2 \
  && "${BUNDLE_FILES[0]}" == physical-qualification-v1.json \
  && "${BUNDLE_FILES[1]}" == release-validator-v11.json ]] \
  || fail_test "dispatch bundle is not the exact two-file contract"
cmp -s "$VALIDATOR" "$BUNDLE/release-validator-v11.json" \
  || fail_test "validator report was not copied byte-for-byte"
[[ "$(sha "$BUNDLE/release-validator-v11.json")" == "$VALIDATOR_SHA" ]] \
  || fail_test "validator copy digest changed"

ENVELOPE="$BUNDLE/physical-qualification-v1.json"
jq -e \
  --arg commit "$COMMIT" \
  --arg version "$VERSION" \
  --arg validator_sha "$VALIDATOR_SHA" \
  --arg harness_sha "$HARNESS_SHA" \
  --arg workflow_sha "$WORKFLOW_SHA" \
  --arg native_sha "$NATIVE_SHA" '
  (keys == ["androidRunId","architecture","ciRunId","cleanupConfirmed","commit","envelopeVersion","failureCode","harnessPassed","harnessVersion","nativeFullSuiteSha256","rawHarnessReportSha256","releaseEligible","repository","schemaVersion","securityRunId","status","validatorReportSha256","validatorVersion","version","workflowFullSuiteSha256"])
  and .schemaVersion == 1
  and .envelopeVersion == "1"
  and .status == "pass"
  and .failureCode == null
  and .releaseEligible == true
  and .repository == "CyberBASSLord-666/termux-mcp-edge"
  and .commit == $commit
  and .version == $version
  and .ciRunId == "1001"
  and .securityRunId == "1002"
  and .androidRunId == "1003"
  and .validatorVersion == "11"
  and .harnessVersion == "11"
  and .architecture == "aarch64"
  and .validatorReportSha256 == $validator_sha
  and .rawHarnessReportSha256 == $harness_sha
  and .workflowFullSuiteSha256 == $workflow_sha
  and .nativeFullSuiteSha256 == $native_sha
  and .workflowFullSuiteSha256 != .nativeFullSuiteSha256
  and .harnessPassed == true
  and .cleanupConfirmed == true
' "$ENVELOPE" >/dev/null || fail_test "envelope contents are invalid"
assert_no_private_data \
  "$ENVELOPE" \
  "$BUNDLE/release-validator-v11.json" \
  "$ROOT/pass.stdout" \
  "$ROOT/pass.stderr"

# Equal workflow/native digests remain valid: the envelope records independent
# provenance and makes no byte-equality claim in either direction.
EQUAL_VALIDATOR="$ROOT/equal-digests-validator.json"
write_validator "$EQUAL_VALIDATOR" "$NATIVE_SHA"
EQUAL_BUNDLE="$BUNDLE_PARENT/equal-digests"
run_package "$EQUAL_VALIDATOR" "$HARNESS" "$EQUAL_BUNDLE" >/dev/null
jq -e '.workflowFullSuiteSha256 == .nativeFullSuiteSha256' \
  "$EQUAL_BUNDLE/physical-qualification-v1.json" >/dev/null \
  || fail_test "equal independent digests were not preserved"

MISMATCH_COMMIT_HARNESS="$ROOT/mismatch-commit-harness.txt"
write_harness "$MISMATCH_COMMIT_HARNESS" "$OTHER_COMMIT"
assert_package_fails "$VALIDATOR" "$MISMATCH_COMMIT_HARNESS" \
  "$BUNDLE_PARENT/mismatch-commit" report_commit_mismatch

MISMATCH_VERSION_HARNESS="$ROOT/mismatch-version-harness.txt"
write_harness "$MISMATCH_VERSION_HARNESS" "$COMMIT" 0.6.1
assert_package_fails "$VALIDATOR" "$MISMATCH_VERSION_HARNESS" \
  "$BUNDLE_PARENT/mismatch-version" report_version_mismatch

MISMATCH_DIGEST_HARNESS="$ROOT/mismatch-digest-harness.txt"
write_harness "$MISMATCH_DIGEST_HARNESS" "$COMMIT" "$VERSION" "$OTHER_SHA" "$NATIVE_SHA"
assert_package_fails "$VALIDATOR" "$MISMATCH_DIGEST_HARNESS" \
  "$BUNDLE_PARENT/mismatch-digest" harness_candidate_digest_mismatch

CONFLICT_HARNESS="$ROOT/conflicting-fact-harness.txt"
write_harness "$CONFLICT_HARNESS"
printf 'exact_head=%s\n' "$OTHER_COMMIT" >>"$CONFLICT_HARNESS"
assert_package_fails "$VALIDATOR" "$CONFLICT_HARNESS" \
  "$BUNDLE_PARENT/conflicting-fact" harness_report_contract_invalid

FIXTURE_VALIDATOR="$ROOT/fixture-validator.json"
make_validator_variant "$VALIDATOR" "$FIXTURE_VALIDATOR" \
  '.status = "fixture" | .releaseEligible = false | .environment.fixtureMode = true'
assert_package_fails "$FIXTURE_VALIDATOR" "$HARNESS" \
  "$BUNDLE_PARENT/fixture" validator_report_contract_invalid

NONELIGIBLE_VALIDATOR="$ROOT/noneligible-validator.json"
make_validator_variant "$VALIDATOR" "$NONELIGIBLE_VALIDATOR" '.releaseEligible = false'
assert_package_fails "$NONELIGIBLE_VALIDATOR" "$HARNESS" \
  "$BUNDLE_PARENT/noneligible" validator_report_contract_invalid

CLEANUP_FAILURE_HARNESS="$ROOT/cleanup-failure-harness.txt"
write_harness "$CLEANUP_FAILURE_HARNESS" "$COMMIT" "$VERSION" "$NATIVE_SHA" "$NATIVE_SHA" false FAIL PASS
assert_package_fails "$VALIDATOR" "$CLEANUP_FAILURE_HARNESS" \
  "$BUNDLE_PARENT/cleanup-failure" harness_cleanup_unconfirmed

FAILED_RESULT_HARNESS="$ROOT/failed-result-harness.txt"
write_harness "$FAILED_RESULT_HARNESS" "$COMMIT" "$VERSION" "$NATIVE_SHA" "$NATIVE_SHA" true FAIL FAIL
assert_package_fails "$VALIDATOR" "$FAILED_RESULT_HARNESS" \
  "$BUNDLE_PARENT/failed-result" harness_result_not_pass

MISSING_LINEAGE_VALIDATOR="$ROOT/missing-lineage-validator.json"
make_validator_variant "$VALIDATOR" "$MISSING_LINEAGE_VALIDATOR" \
  '[.results[] | select(.code != "full_suite_deployment_candidate_selected")] as $results | .results = $results'
assert_package_fails "$MISSING_LINEAGE_VALIDATOR" "$HARNESS" \
  "$BUNDLE_PARENT/missing-lineage" validator_report_contract_invalid

FAILED_RESULT_VALIDATOR="$ROOT/failed-result-validator.json"
make_validator_variant "$VALIDATOR" "$FAILED_RESULT_VALIDATOR" \
  '.results += [{phase:"runtime",check:"unexpected_failure",outcome:"fail",code:"unexpected_failure"}]'
assert_package_fails "$FAILED_RESULT_VALIDATOR" "$HARNESS" \
  "$BUNDLE_PARENT/failed-validator-result" validator_report_contract_invalid

INVALID_TIME_VALIDATOR="$ROOT/invalid-time-validator.json"
make_validator_variant "$VALIDATOR" "$INVALID_TIME_VALIDATOR" '.completedAt = "2026-99-99T99:99:99Z"'
assert_package_fails "$INVALID_TIME_VALIDATOR" "$HARNESS" \
  "$BUNDLE_PARENT/invalid-time" validator_report_timestamp_invalid

OVERSIZED_JQ_VERSION_VALIDATOR="$ROOT/oversized-jq-version-validator.json"
make_validator_variant "$VALIDATOR" "$OVERSIZED_JQ_VERSION_VALIDATOR" \
  '.environment.tools.jq = ("j" * 65)'
assert_package_fails "$OVERSIZED_JQ_VERSION_VALIDATOR" "$HARNESS" \
  "$BUNDLE_PARENT/oversized-jq-version" validator_report_contract_invalid

WRONG_MODE_VALIDATOR="$ROOT/wrong-mode-validator.json"
cp "$VALIDATOR" "$WRONG_MODE_VALIDATOR"
chmod 644 "$WRONG_MODE_VALIDATOR"
assert_package_fails "$WRONG_MODE_VALIDATOR" "$HARNESS" \
  "$BUNDLE_PARENT/wrong-mode" validator_report_invalid

SYMLINK_HARNESS="$ROOT/symlink-harness.txt"
ln -s "$HARNESS" "$SYMLINK_HARNESS"
assert_package_fails "$VALIDATOR" "$SYMLINK_HARNESS" \
  "$BUNDLE_PARENT/symlink-input" harness_report_invalid

mkdir -m 700 "$BUNDLE_PARENT/existing-output"
printf '%s\n' preserve-existing >"$BUNDLE_PARENT/existing-output/sentinel"
if run_package "$VALIDATOR" "$HARNESS" "$BUNDLE_PARENT/existing-output" \
  >"$ROOT/last.stdout" 2>"$ROOT/last.stderr"; then
  fail_test "package unexpectedly replaced existing output"
fi
grep -Fq 'reason=output_directory_invalid' "$ROOT/last.stderr" \
  || fail_test "existing-output failure reason was absent"
[[ "$(<"$BUNDLE_PARENT/existing-output/sentinel")" == preserve-existing ]] \
  || fail_test "existing output was modified"
[[ -z "$(find "$BUNDLE_PARENT" -maxdepth 1 -name '.physical-qualification.*' -print -quit)" ]] \
  || fail_test "existing-output rejection left staging state"

assert_no_private_data "$ROOT/last.stdout" "$ROOT/last.stderr"
printf 'Physical qualification package tests passed\n'
