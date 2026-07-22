#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077
set +x

usage() {
  cat <<'EOF'
Usage: package_physical_qualification.sh \
  --validator-report RELEASE-VALIDATOR-V11.json \
  --harness-report TERMUX-DEVICE-HARNESS-V11.txt \
  --output-dir DIR

Creates DIR as a private, atomic, two-file dispatch bundle containing exactly:
  physical-qualification-v1.json  compact qualification envelope
  release-validator-v11.json      unchanged sanitized validator report

Both inputs must be absolute, canonical, mode-0600 regular files. DIR must not
exist and its absolute canonical parent must have mode 0700. No archive is
created; archive only the two named output files for workflow dispatch.

The raw device-harness report is validated and SHA-256-bound by the envelope,
but is never copied into the bundle because it contains device-local data.
The workflow and native full-suite digests identify separate builds. Their byte
equality is neither required nor asserted.
EOF
}

VALIDATOR_REPORT=""
HARNESS_REPORT=""
OUTPUT_DIR=""
STAGING_DIR=""
COMPLETED=0

fail() {
  printf 'PHYSICAL_QUALIFICATION_PACKAGE_RESULT=FAIL reason=%s\n' "$1" >&2
  exit 1
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM HUP
  if ((COMPLETED == 0)) && [[ -n "$STAGING_DIR" ]]; then
    rm -rf -- "$STAGING_DIR" >/dev/null 2>&1 || true
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

while (($# > 0)); do
  case "$1" in
    --validator-report)
      (($# >= 2)) || fail missing_validator_report
      VALIDATOR_REPORT="$2"
      shift 2
      ;;
    --harness-report)
      (($# >= 2)) || fail missing_harness_report
      HARNESS_REPORT="$2"
      shift 2
      ;;
    --output-dir)
      (($# >= 2)) || fail missing_output_directory
      OUTPUT_DIR="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail unknown_argument
      ;;
  esac
done

for command_name in awk chmod date dirname find install jq mkdir mktemp mv realpath rm sha256sum stat wc; do
  command -v "$command_name" >/dev/null 2>&1 || fail required_command_missing
done

validate_private_input() {
  local input="$1" maximum_bytes="$2" bytes mode
  [[ "$input" == /* && -f "$input" && ! -L "$input" ]] || return 1
  [[ "$(realpath -e -- "$input" 2>/dev/null)" == "$input" ]] || return 1
  mode="$(stat -c '%a' -- "$input" 2>/dev/null)" || return 1
  [[ "$mode" == 600 ]] || return 1
  bytes="$(stat -c '%s' -- "$input" 2>/dev/null)" || return 1
  [[ "$bytes" =~ ^[0-9]+$ ]] || return 1
  ((bytes > 0 && bytes <= maximum_bytes))
}

[[ -n "$VALIDATOR_REPORT" ]] || fail missing_validator_report
[[ -n "$HARNESS_REPORT" ]] || fail missing_harness_report
[[ -n "$OUTPUT_DIR" ]] || fail missing_output_directory
validate_private_input "$VALIDATOR_REPORT" 1048576 || fail validator_report_invalid
validate_private_input "$HARNESS_REPORT" 16777216 || fail harness_report_invalid
[[ "$VALIDATOR_REPORT" != "$HARNESS_REPORT" ]] || fail input_reports_not_distinct

[[ "$OUTPUT_DIR" == /* && "$OUTPUT_DIR" != / && ! -e "$OUTPUT_DIR" && ! -L "$OUTPUT_DIR" ]] \
  || fail output_directory_invalid
OUTPUT_PARENT="$(dirname -- "$OUTPUT_DIR")"
[[ "$OUTPUT_PARENT" == /* && -d "$OUTPUT_PARENT" && ! -L "$OUTPUT_PARENT" ]] \
  || fail output_parent_invalid
[[ "$(realpath -e -- "$OUTPUT_PARENT" 2>/dev/null)" == "$OUTPUT_PARENT" ]] \
  || fail output_parent_invalid
[[ "$(stat -c '%a' -- "$OUTPUT_PARENT" 2>/dev/null)" == 700 ]] \
  || fail output_parent_not_private

# Snapshot both private inputs before parsing so every emitted digest and field
# comes from one immutable view. Hashing each source before and after the copy
# rejects concurrent mutation instead of packaging a partially changed report.
VALIDATOR_SOURCE="$VALIDATOR_REPORT"
HARNESS_SOURCE="$HARNESS_REPORT"
VALIDATOR_SOURCE_SHA_BEFORE="$(sha256sum -- "$VALIDATOR_SOURCE" 2>/dev/null | awk '{print $1}')" \
  || fail validator_report_digest_failed
HARNESS_SOURCE_SHA_BEFORE="$(sha256sum -- "$HARNESS_SOURCE" 2>/dev/null | awk '{print $1}')" \
  || fail harness_report_digest_failed
[[ "$VALIDATOR_SOURCE_SHA_BEFORE" =~ ^[0-9a-f]{64}$ ]] || fail validator_report_digest_failed
[[ "$HARNESS_SOURCE_SHA_BEFORE" =~ ^[0-9a-f]{64}$ ]] || fail harness_report_digest_failed

STAGING_DIR="$(mktemp -d "$OUTPUT_PARENT/.physical-qualification.XXXXXX" 2>/dev/null)" \
  || fail staging_directory_create_failed
[[ -d "$STAGING_DIR" && ! -L "$STAGING_DIR" ]] || fail staging_directory_create_failed
chmod 700 "$STAGING_DIR" 2>/dev/null || fail staging_directory_mode_failed
VALIDATOR_REPORT="$STAGING_DIR/release-validator-v11.json"
HARNESS_REPORT="$STAGING_DIR/.raw-harness-v11.txt"
install -m 600 -- "$VALIDATOR_SOURCE" "$VALIDATOR_REPORT" 2>/dev/null \
  || fail validator_report_copy_failed
install -m 600 -- "$HARNESS_SOURCE" "$HARNESS_REPORT" 2>/dev/null \
  || fail harness_report_snapshot_failed
VALIDATOR_REPORT_SHA="$(sha256sum -- "$VALIDATOR_REPORT" 2>/dev/null | awk '{print $1}')" \
  || fail validator_report_digest_failed
RAW_HARNESS_REPORT_SHA="$(sha256sum -- "$HARNESS_REPORT" 2>/dev/null | awk '{print $1}')" \
  || fail harness_report_digest_failed
VALIDATOR_SOURCE_SHA_AFTER="$(sha256sum -- "$VALIDATOR_SOURCE" 2>/dev/null | awk '{print $1}')" \
  || fail validator_report_digest_failed
HARNESS_SOURCE_SHA_AFTER="$(sha256sum -- "$HARNESS_SOURCE" 2>/dev/null | awk '{print $1}')" \
  || fail harness_report_digest_failed
[[ "$VALIDATOR_SOURCE_SHA_BEFORE" == "$VALIDATOR_REPORT_SHA" \
  && "$VALIDATOR_SOURCE_SHA_AFTER" == "$VALIDATOR_REPORT_SHA" ]] \
  || fail validator_report_changed_during_snapshot
[[ "$HARNESS_SOURCE_SHA_BEFORE" == "$RAW_HARNESS_REPORT_SHA" \
  && "$HARNESS_SOURCE_SHA_AFTER" == "$RAW_HARNESS_REPORT_SHA" ]] \
  || fail harness_report_changed_during_snapshot

# Accept only the canonical, release-eligible validator-v11/schema-v2 shape.
# This deliberately validates more than the schema's general pass/fail union:
# packaging is permitted only after every physical-release gate has passed.
jq -e '
  def artifact:
    type == "object"
    and (keys == ["bytes","elf","sha256","version"])
    and (.sha256 | type == "string" and test("^[0-9a-f]{64}$"))
    and (.bytes | type == "number" and floor == . and . >= 1 and . <= 67108864)
    and (.version | type == "string" and test("^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$"))
    and .elf == "aarch64-android-elf";
  def passed($code):
    any(.results[]; .code == $code and .outcome == "pass");
  (type == "object")
  and (keys == ["artifacts","completedAt","deploymentCandidate","environment","failureCode","phases","releaseEligible","repository","requestedPhase","results","schemaVersion","startedAt","status","sustainedObservation","validatorVersion"])
  and .schemaVersion == 2
  and .validatorVersion == "11"
  and .status == "pass"
  and .failureCode == null
  and .releaseEligible == true
  and (.startedAt | type == "string" and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"))
  and (.completedAt | type == "string" and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"))
  and (.repository | type == "object" and keys == ["androidRunId","ciRunId","commit","securityRunId","version"])
  and (.repository.commit | type == "string" and test("^[0-9a-f]{40}$"))
  and (.repository.version | type == "string" and test("^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$"))
  and (.repository.ciRunId | type == "string" and test("^[1-9][0-9]*$"))
  and (.repository.securityRunId | type == "string" and test("^[1-9][0-9]*$"))
  and (.repository.androidRunId | type == "string" and test("^[1-9][0-9]*$"))
  and (.environment | type == "object" and keys == ["architecture","fixtureMode","tools"])
  and (.environment.architecture == "aarch64" or .environment.architecture == "arm64")
  and .environment.fixtureMode == false
  and (.environment.tools | type == "object" and keys == ["bash","curl","file","jq"])
  and all(.environment.tools[];
    type == "string" and length >= 1 and length <= 256
    and (contains("\u0000") | not)
    and (contains("\n") | not)
    and (contains("\r") | not))
  and (.environment.tools.jq | length <= 64)
  and .requestedPhase == "all"
  and (.artifacts | type == "object" and keys == ["androidVolumeControl","baseline","default","fullSuite","mcpRuntime"])
  and (.artifacts.default | artifact)
  and (.artifacts.mcpRuntime | artifact)
  and (.artifacts.androidVolumeControl | artifact)
  and (.artifacts.fullSuite | artifact)
  and (.artifacts.baseline | artifact)
  and .artifacts.default.version == .repository.version
  and .artifacts.mcpRuntime.version == .repository.version
  and .artifacts.androidVolumeControl.version == .repository.version
  and .artifacts.fullSuite.version == .repository.version
  and .artifacts.baseline.version != .repository.version
  and ([
    .artifacts.default.sha256,
    .artifacts.mcpRuntime.sha256,
    .artifacts.androidVolumeControl.sha256,
    .artifacts.fullSuite.sha256,
    .artifacts.baseline.sha256
  ] | unique | length == 5)
  and (.deploymentCandidate | type == "object" and keys == ["posture","productionAction"])
  and .deploymentCandidate.posture == "full-suite"
  and .deploymentCandidate.productionAction == null
  and .phases == {preflight:"pass",runtime:"pass",deployment:"pass"}
  and (.results | type == "array" and length >= 1 and length <= 256)
  and all(.results[];
    type == "object"
    and (keys == ["check","code","outcome","phase"])
    and (.phase == "preflight" or .phase == "runtime" or .phase == "deployment")
    and (.check | type == "string" and length <= 96 and test("^[a-z0-9_]+$"))
    and (.outcome == "pass" or .outcome == "fail" or .outcome == "info")
    and (.code | type == "string" and length <= 96 and test("^[a-z0-9_]+$")))
  and all(.results[]; .outcome != "fail")
  and passed("full_suite_default_disabled_17_tool_posture_verified")
  and passed("full_suite_battery_runtime_gate_independence_verified")
  and passed("full_suite_volume_status_runtime_gate_independence_verified")
  and passed("full_suite_volume_control_runtime_gate_independence_verified")
  and passed("full_suite_command_runtime_gate_independence_verified")
  and passed("full_suite_enabled_21_tool_posture_verified")
  and passed("full_suite_optional_provider_success_verified")
  and passed("full_suite_volume_preview_and_grant_boundary_verified")
  and passed("full_suite_command_basename_and_profile_verified")
  and passed("full_suite_filesystem_mutations_independently_disabled")
  and passed("full_suite_deployment_candidate_selected")
  and (.sustainedObservation | type == "object" and keys == ["minimumMinutes","minutes","operatorSupplied","reasonCode","status"])
  and .sustainedObservation.operatorSupplied == true
  and .sustainedObservation.status == "pass"
  and (.sustainedObservation.minutes | type == "number" and floor == . and . >= 60 and . <= 10080)
  and .sustainedObservation.reasonCode == "stable"
  and .sustainedObservation.minimumMinutes == 60
' "$VALIDATOR_REPORT" >/dev/null 2>&1 || fail validator_report_contract_invalid

STARTED_AT="$(jq -r '.startedAt' "$VALIDATOR_REPORT" 2>/dev/null)" \
  || fail validator_report_read_failed
COMPLETED_AT="$(jq -r '.completedAt' "$VALIDATOR_REPORT" 2>/dev/null)" \
  || fail validator_report_read_failed
STARTED_EPOCH="$(date -u -d "$STARTED_AT" '+%s' 2>/dev/null)" \
  || fail validator_report_timestamp_invalid
COMPLETED_EPOCH="$(date -u -d "$COMPLETED_AT" '+%s' 2>/dev/null)" \
  || fail validator_report_timestamp_invalid
[[ "$(date -u -d "@$STARTED_EPOCH" '+%Y-%m-%dT%H:%M:%SZ' 2>/dev/null)" == "$STARTED_AT" ]] \
  || fail validator_report_timestamp_invalid
[[ "$(date -u -d "@$COMPLETED_EPOCH" '+%Y-%m-%dT%H:%M:%SZ' 2>/dev/null)" == "$COMPLETED_AT" ]] \
  || fail validator_report_timestamp_invalid
((COMPLETED_EPOCH >= STARTED_EPOCH)) || fail validator_report_timestamp_invalid

extract_harness_fact() {
  local key="$1"
  awk -v key="$key" '
    index($0, key "=") == 1 {
      value = substr($0, length(key) + 2)
      if (found && value != first) conflict = 1
      if (!found) first = value
      found = 1
    }
    END {
      if (!found || conflict) exit 1
      print first
    }
  ' "$HARNESS_REPORT" 2>/dev/null
}

HARNESS_VERSION="$(extract_harness_fact harness_version)" \
  || fail harness_report_contract_invalid
HARNESS_ARCHITECTURE="$(extract_harness_fact architecture)" \
  || fail harness_report_contract_invalid
HARNESS_VERSION_VALUE="$(extract_harness_fact candidate_version)" \
  || fail harness_report_contract_invalid
HARNESS_COMMIT="$(extract_harness_fact exact_head)" \
  || fail harness_report_contract_invalid
HARNESS_CANDIDATE_SHA="$(extract_harness_fact candidate_sha256)" \
  || fail harness_report_contract_invalid
HARNESS_MCP_SHA="$(extract_harness_fact mcp_runtime_sha256)" \
  || fail harness_report_contract_invalid
HARNESS_VOLUME_SHA="$(extract_harness_fact volume_control_sha256)" \
  || fail harness_report_contract_invalid
NATIVE_FULL_SUITE_SHA="$(extract_harness_fact full_suite_sha256)" \
  || fail harness_report_contract_invalid
HARNESS_RESULT="$(extract_harness_fact TERMUX_MCP_DEVICE_RESULT)" \
  || fail harness_report_contract_invalid
HARNESS_CLEANUP="$(extract_harness_fact cleanup_complete)" \
  || fail harness_report_contract_invalid
HARNESS_FINAL_STATUS="$(extract_harness_fact final_status)" \
  || fail harness_report_contract_invalid

[[ "$HARNESS_VERSION" == 11 ]] || fail harness_version_invalid
case "$HARNESS_ARCHITECTURE" in
  aarch64|arm64) ;;
  *) fail harness_architecture_invalid ;;
esac
[[ "$HARNESS_VERSION_VALUE" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] \
  || fail harness_candidate_version_invalid
[[ "$HARNESS_COMMIT" =~ ^[0-9a-f]{40}$ ]] || fail harness_commit_invalid
for digest in "$HARNESS_CANDIDATE_SHA" "$HARNESS_MCP_SHA" "$HARNESS_VOLUME_SHA" "$NATIVE_FULL_SUITE_SHA"; do
  [[ "$digest" =~ ^[0-9a-f]{64}$ ]] || fail harness_digest_invalid
done
[[ "$HARNESS_CANDIDATE_SHA" == "$NATIVE_FULL_SUITE_SHA" ]] \
  || fail harness_candidate_digest_mismatch
[[ "$HARNESS_MCP_SHA" != "$HARNESS_VOLUME_SHA" \
  && "$HARNESS_MCP_SHA" != "$NATIVE_FULL_SUITE_SHA" \
  && "$HARNESS_VOLUME_SHA" != "$NATIVE_FULL_SUITE_SHA" ]] \
  || fail harness_posture_digests_not_distinct
[[ "$HARNESS_RESULT" == PASS ]] || fail harness_result_not_pass
[[ "$HARNESS_CLEANUP" == true ]] || fail harness_cleanup_unconfirmed
[[ "$HARNESS_FINAL_STATUS" == PASS ]] || fail harness_final_status_not_pass

VALIDATOR_COMMIT="$(jq -r '.repository.commit' "$VALIDATOR_REPORT" 2>/dev/null)" \
  || fail validator_report_read_failed
VALIDATOR_VERSION_VALUE="$(jq -r '.repository.version' "$VALIDATOR_REPORT" 2>/dev/null)" \
  || fail validator_report_read_failed
VALIDATOR_ARCHITECTURE="$(jq -r '.environment.architecture' "$VALIDATOR_REPORT" 2>/dev/null)" \
  || fail validator_report_read_failed
[[ "$HARNESS_COMMIT" == "$VALIDATOR_COMMIT" ]] || fail report_commit_mismatch
[[ "$HARNESS_VERSION_VALUE" == "$VALIDATOR_VERSION_VALUE" ]] || fail report_version_mismatch
case "$VALIDATOR_ARCHITECTURE:$HARNESS_ARCHITECTURE" in
  aarch64:aarch64|aarch64:arm64|arm64:aarch64|arm64:arm64) ;;
  *) fail report_architecture_mismatch ;;
esac

WORKFLOW_FULL_SUITE_SHA="$(jq -r '.artifacts.fullSuite.sha256' "$VALIDATOR_REPORT" 2>/dev/null)" \
  || fail validator_report_read_failed
[[ "$VALIDATOR_REPORT_SHA" =~ ^[0-9a-f]{64}$ ]] || fail validator_report_digest_failed
[[ "$RAW_HARNESS_REPORT_SHA" =~ ^[0-9a-f]{64}$ ]] || fail harness_report_digest_failed
[[ "$WORKFLOW_FULL_SUITE_SHA" =~ ^[0-9a-f]{64}$ ]] || fail workflow_digest_invalid

CI_RUN_ID="$(jq -r '.repository.ciRunId' "$VALIDATOR_REPORT" 2>/dev/null)" \
  || fail validator_report_read_failed
SECURITY_RUN_ID="$(jq -r '.repository.securityRunId' "$VALIDATOR_REPORT" 2>/dev/null)" \
  || fail validator_report_read_failed
ANDROID_RUN_ID="$(jq -r '.repository.androidRunId' "$VALIDATOR_REPORT" 2>/dev/null)" \
  || fail validator_report_read_failed

jq -cn \
  --arg repository "CyberBASSLord-666/termux-mcp-edge" \
  --arg commit "$VALIDATOR_COMMIT" \
  --arg version "$VALIDATOR_VERSION_VALUE" \
  --arg ci_run_id "$CI_RUN_ID" \
  --arg security_run_id "$SECURITY_RUN_ID" \
  --arg android_run_id "$ANDROID_RUN_ID" \
  --arg validator_report_sha "$VALIDATOR_REPORT_SHA" \
  --arg raw_harness_report_sha "$RAW_HARNESS_REPORT_SHA" \
  --arg workflow_full_suite_sha "$WORKFLOW_FULL_SUITE_SHA" \
  --arg native_full_suite_sha "$NATIVE_FULL_SUITE_SHA" '
  {
    schemaVersion: 1,
    envelopeVersion: "1",
    status: "pass",
    failureCode: null,
    releaseEligible: true,
    repository: $repository,
    commit: $commit,
    version: $version,
    ciRunId: $ci_run_id,
    securityRunId: $security_run_id,
    androidRunId: $android_run_id,
    validatorVersion: "11",
    harnessVersion: "11",
    architecture: "aarch64",
    validatorReportSha256: $validator_report_sha,
    rawHarnessReportSha256: $raw_harness_report_sha,
    workflowFullSuiteSha256: $workflow_full_suite_sha,
    nativeFullSuiteSha256: $native_full_suite_sha,
    harnessPassed: true,
    cleanupConfirmed: true
  }' >"$STAGING_DIR/physical-qualification-v1.json" 2>/dev/null \
  || fail envelope_write_failed
chmod 600 "$STAGING_DIR/physical-qualification-v1.json" 2>/dev/null \
  || fail envelope_mode_failed
ENVELOPE_BYTES="$(stat -c '%s' -- "$STAGING_DIR/physical-qualification-v1.json" 2>/dev/null)" \
  || fail envelope_size_invalid
[[ "$ENVELOPE_BYTES" =~ ^[0-9]+$ ]] || fail envelope_size_invalid
((ENVELOPE_BYTES > 0 && ENVELOPE_BYTES <= 65536)) || fail envelope_size_invalid

jq -e \
  --arg validator_sha "$VALIDATOR_REPORT_SHA" \
  --arg harness_sha "$RAW_HARNESS_REPORT_SHA" '
  (keys == ["androidRunId","architecture","ciRunId","cleanupConfirmed","commit","envelopeVersion","failureCode","harnessPassed","harnessVersion","nativeFullSuiteSha256","rawHarnessReportSha256","releaseEligible","repository","schemaVersion","securityRunId","status","validatorReportSha256","validatorVersion","version","workflowFullSuiteSha256"])
  and .schemaVersion == 1
  and .envelopeVersion == "1"
  and .status == "pass"
  and .failureCode == null
  and .releaseEligible == true
  and .validatorReportSha256 == $validator_sha
  and .rawHarnessReportSha256 == $harness_sha
  and .harnessPassed == true
  and .cleanupConfirmed == true
' "$STAGING_DIR/physical-qualification-v1.json" >/dev/null 2>&1 \
  || fail envelope_verification_failed

validate_private_input "$VALIDATOR_SOURCE" 1048576 || fail validator_report_changed_before_publication
validate_private_input "$HARNESS_SOURCE" 16777216 || fail harness_report_changed_before_publication
[[ "$(sha256sum -- "$VALIDATOR_SOURCE" 2>/dev/null | awk '{print $1}')" == "$VALIDATOR_REPORT_SHA" ]] \
  || fail validator_report_changed_before_publication
[[ "$(sha256sum -- "$HARNESS_SOURCE" 2>/dev/null | awk '{print $1}')" == "$RAW_HARNESS_REPORT_SHA" ]] \
  || fail harness_report_changed_before_publication
rm -f -- "$HARNESS_REPORT" 2>/dev/null || fail harness_snapshot_removal_failed
[[ ! -e "$HARNESS_REPORT" && ! -L "$HARNESS_REPORT" ]] || fail harness_snapshot_removal_failed
[[ "$(find "$STAGING_DIR" -mindepth 1 -maxdepth 1 -type f 2>/dev/null | wc -l)" == 2 ]] \
  || fail bundle_file_contract_invalid
[[ -z "$(find "$STAGING_DIR" -mindepth 1 -maxdepth 1 ! -type f -print -quit 2>/dev/null)" ]] \
  || fail bundle_file_contract_invalid
mv -Tn -- "$STAGING_DIR" "$OUTPUT_DIR" 2>/dev/null || fail bundle_publication_failed
[[ ! -e "$STAGING_DIR" && ! -L "$STAGING_DIR" && -d "$OUTPUT_DIR" && ! -L "$OUTPUT_DIR" ]] \
  || fail bundle_publication_failed
STAGING_DIR=""
COMPLETED=1
printf 'PHYSICAL_QUALIFICATION_PACKAGE_RESULT=PASS\n'
