#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077

ROOT="$(mktemp -d)"
trap 'rm -rf -- "$ROOT"' EXIT INT TERM
chmod 700 "$ROOT"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VALIDATOR="$REPO_ROOT/scripts/validate_rish_physical_identity_evidence.py"
EVIDENCE_SCHEMA="$REPO_ROOT/docs/android-rish-physical-identity-evidence-schema-v1.json"
POLICY_SCHEMA="$REPO_ROOT/docs/android-rish-physical-identity-policy-schema-v1.json"
POLICY="$REPO_ROOT/docs/android-rish-physical-identity-policy-v1.json"
EVIDENCE_FILE_NAME=android-rish-physical-identity-evidence-v1.json

COMMIT=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
VERSION=0.7.0
CARGO_LOCK_SHA=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
RAW_REPORT_SHA=cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc
WORKFLOW_DEFINITION_SHA=dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd
CONTROLLER_CHALLENGE_SHA=eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee
ARTIFACT_SHA=ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
DEVICE_PROFILE_COMMITMENT=1111111111111111111111111111111111111111111111111111111111111111
BUILD_FINGERPRINT_SHA=2222222222222222222222222222222222222222222222222222222222222222
TERMUX_SIGNER_SHA=3333333333333333333333333333333333333333333333333333333333333333
SHIZUKU_SIGNER_SHA=4444444444444444444444444444444444444444444444444444444444444444
DEX_SHA=5555555555555555555555555555555555555555555555555555555555555555
WORKFLOW_RUN_ID=1001
CI_RUN_ID=1002
SECURITY_RUN_ID=1003
ANDROID_RUN_ID=1004
ARTIFACT_BYTES=123456
DEX_BYTES=1234
API_LEVEL=35
SECURITY_PATCH=2026-07-01
TERMUX_VERSION=0.118.3
SHIZUKU_VERSION=13.5.4
PRIVATE_SENTINEL=private-device-value-never-reflect

fail_test() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

sha256() {
  sha256sum -- "$1" | awk '{print $1}'
}

POLICY_SHA="$(sha256 "$POLICY")"
test "$(jq -er '.environment.requiredShizukuStartMode' "$POLICY")" = "adb" \
  || fail_test "policy does not retain the ADB-start requirement"
if jq -e '.environment | has("shizukuStartModeObserved")' "$POLICY" >/dev/null; then
  fail_test "policy conflates the ADB-start requirement with evidence observability"
fi
EVIDENCE="$ROOT/$EVIDENCE_FILE_NAME"

jq -n \
  --arg commit "$COMMIT" \
  --arg version "$VERSION" \
  --arg policy_sha "$POLICY_SHA" \
  --arg cargo_lock_sha "$CARGO_LOCK_SHA" \
  --arg raw_report_sha "$RAW_REPORT_SHA" \
  --arg workflow_definition_sha "$WORKFLOW_DEFINITION_SHA" \
  --arg workflow_run_id "$WORKFLOW_RUN_ID" \
  --arg challenge_sha "$CONTROLLER_CHALLENGE_SHA" \
  --arg ci_run_id "$CI_RUN_ID" \
  --arg security_run_id "$SECURITY_RUN_ID" \
  --arg android_run_id "$ANDROID_RUN_ID" \
  --arg artifact_sha "$ARTIFACT_SHA" \
  --arg device_profile "$DEVICE_PROFILE_COMMITMENT" \
  --arg build_fingerprint_sha "$BUILD_FINGERPRINT_SHA" \
  --arg security_patch "$SECURITY_PATCH" \
  --arg termux_version "$TERMUX_VERSION" \
  --arg termux_signer_sha "$TERMUX_SIGNER_SHA" \
  --arg shizuku_version "$SHIZUKU_VERSION" \
  --arg shizuku_signer_sha "$SHIZUKU_SIGNER_SHA" \
  --arg dex_sha "$DEX_SHA" \
  --argjson artifact_bytes "$ARTIFACT_BYTES" \
  --argjson api_level "$API_LEVEL" \
  --argjson dex_bytes "$DEX_BYTES" '
  {
    schemaVersion: 1,
    gateVersion: "1",
    status: "pass",
    failureCode: null,
    releaseEligible: false,
    productionControlQualified: false,
    qualificationClass: "physical_shizuku_rish_identity_development_v1",
    scope: "s2_5_uid_probe_only",
    startedAt: "2026-07-31T12:00:00Z",
    completedAt: "2026-07-31T12:10:00Z",
    repository: "CyberBASSLord-666/termux-mcp-edge",
    commit: $commit,
    version: $version,
    policySha256: $policy_sha,
    cargoLockSha256: $cargo_lock_sha,
    rawReportSha256: $raw_report_sha,
    workflow: {
      name: "Android Rish Physical Identity",
      definitionSha256: $workflow_definition_sha,
      runId: $workflow_run_id,
      runAttempt: 1,
      event: "workflow_dispatch",
      protectedEnvironment: "android-rish-physical-development",
      controllerChallengeSha256: $challenge_sha,
      ciRunId: $ci_run_id,
      securityRunId: $security_run_id,
      androidRunId: $android_run_id
    },
    artifact: {
      artifactName: "termux-mcp-server-aarch64-linux-android-android-rish-development",
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
      adbShellUid: 2000,
      shizukuStartModeObserved: false,
      termuxVersion: $termux_version,
      termuxSignerSha256: $termux_signer_sha,
      shizukuVersion: $shizuku_version,
      shizukuSignerSha256: $shizuku_signer_sha
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
      state: "verified_shell_uid",
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
      controllerOfflinePosturePreCandidate: true,
      trustedDirectRishProbePreCandidate: true,
      trustedDirectRishProbePostCandidate: true,
      controllerOfflinePosturePostCandidate: true,
      dexTamperRejected: true,
      dexModeRejected: true,
      dexSymlinkRejected: true,
      scenarioResults: [
        {id:"controller_offline_posture_pre_candidate",execution:"physical",outcome:"pass"},
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
        {id:"controller_offline_posture_post_candidate",execution:"physical",outcome:"pass"},
        {id:"bounded_test_fixture_cleanup",execution:"physical",outcome:"pass"},
        {id:"device_slot_quarantined_after_candidate",execution:"physical",outcome:"pass"}
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
      deviceFixtureStateRemoved: true,
      controllerTransportRemoved: true,
      deviceSlotQuarantinedAfterCandidate: true
    }
  }
' >"$EVIDENCE"
chmod 600 "$EVIDENCE"

validator_command=(
  python3 "$VALIDATOR"
  --policy "$POLICY"
  --expected-commit "$COMMIT"
  --expected-version "$VERSION"
  --expected-policy-sha256 "$POLICY_SHA"
  --expected-cargo-lock-sha256 "$CARGO_LOCK_SHA"
  --expected-raw-report-sha256 "$RAW_REPORT_SHA"
  --expected-workflow-definition-sha256 "$WORKFLOW_DEFINITION_SHA"
  --expected-workflow-run-id "$WORKFLOW_RUN_ID"
  --expected-ci-run-id "$CI_RUN_ID"
  --expected-security-run-id "$SECURITY_RUN_ID"
  --expected-android-run-id "$ANDROID_RUN_ID"
  --expected-controller-challenge-sha256 "$CONTROLLER_CHALLENGE_SHA"
  --expected-artifact-sha256 "$ARTIFACT_SHA"
  --expected-artifact-bytes "$ARTIFACT_BYTES"
  --expected-api-level "$API_LEVEL"
  --expected-security-patch "$SECURITY_PATCH"
  --expected-device-profile-commitment "$DEVICE_PROFILE_COMMITMENT"
  --expected-build-fingerprint-sha256 "$BUILD_FINGERPRINT_SHA"
  --expected-termux-version "$TERMUX_VERSION"
  --expected-termux-signer-sha256 "$TERMUX_SIGNER_SHA"
  --expected-shizuku-version "$SHIZUKU_VERSION"
  --expected-shizuku-signer-sha256 "$SHIZUKU_SIGNER_SHA"
  --expected-dex-sha256 "$DEX_SHA"
  --expected-dex-bytes "$DEX_BYTES"
)

run_validator() {
  "${validator_command[@]}" --evidence "$1"
}

assert_fails() {
  local evidence="$1" expected_reason="$2"
  if run_validator "$evidence" >"$ROOT/last.stdout" 2>"$ROOT/last.stderr"; then
    fail_test "validator unexpectedly accepted $expected_reason"
  fi
  grep -Fxq \
    "RISH_PHYSICAL_IDENTITY_VALIDATION_RESULT=FAIL reason=$expected_reason" \
    "$ROOT/last.stderr" \
    || fail_test "validator did not report $expected_reason"
  [[ ! -s "$ROOT/last.stdout" ]] \
    || fail_test "failed validation wrote standard output"
  if grep -Fq "$PRIVATE_SENTINEL" "$ROOT/last.stdout" "$ROOT/last.stderr"; then
    fail_test "failed validation reflected private device data"
  fi
}

mutate() {
  local name="$1" filter="$2" destination
  destination="$ROOT/$name/$EVIDENCE_FILE_NAME"
  mkdir -m 700 "$ROOT/$name"
  jq "$filter" "$EVIDENCE" >"$destination"
  chmod 600 "$destination"
  printf '%s\n' "$destination"
}

bash -n "${BASH_SOURCE[0]}"
python3 - "$EVIDENCE_SCHEMA" "$POLICY_SCHEMA" "$POLICY" "$EVIDENCE" <<'PY'
import json
import pathlib
import sys

def closed_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError("duplicate key")
        value[key] = item
    return value

def walk(value):
    if isinstance(value, dict):
        if value.get("type") == "object" and value.get("additionalProperties") is not False:
            raise SystemExit("object schema is not closed")
        for item in value.values():
            walk(item)
    elif isinstance(value, list):
        for item in value:
            walk(item)

values = []
for raw_path in sys.argv[1:]:
    with pathlib.Path(raw_path).open("r", encoding="utf-8") as source:
        value = json.load(source, object_pairs_hook=closed_object)
    values.append(value)
    walk(value)

try:
    import jsonschema
except ImportError:
    pass
else:
    jsonschema.Draft202012Validator.check_schema(values[0])
    jsonschema.Draft202012Validator.check_schema(values[1])
    jsonschema.Draft202012Validator(values[1]).validate(values[2])
    jsonschema.Draft202012Validator(values[0]).validate(values[3])
PY

jq -e '
  .type == "object"
  and .additionalProperties == false
  and .properties.releaseEligible.const == false
  and .properties.productionControlQualified.const == false
  and .properties.qualificationClass.const == "physical_shizuku_rish_identity_development_v1"
  and .properties.scope.const == "s2_5_uid_probe_only"
  and ."$defs".validation.properties.scenarioResults.maxItems == 14
' "$EVIDENCE_SCHEMA" >/dev/null
jq -e '
  .releaseEligible == false
  and .productionControlQualified == false
  and .qualificationClass == "physical_shizuku_rish_identity_development_v1"
  and .scope == "s2_5_uid_probe_only"
  and .maximumEvidenceBytes == 65536
  and .maximumRawReportBytes == 16777216
  and .maximumObservationSeconds == 1800
  and (.validation.scenarioIds | length) == 14
' "$POLICY" >/dev/null

run_validator "$EVIDENCE" >"$ROOT/pass.stdout" 2>"$ROOT/pass.stderr"
[[ "$(<"$ROOT/pass.stdout")" == RISH_PHYSICAL_IDENTITY_VALIDATION_RESULT=PASS ]] \
  || fail_test "valid evidence did not produce the success contract"
[[ ! -s "$ROOT/pass.stderr" ]] || fail_test "valid evidence wrote standard error"

assert_fails "$(mutate extra-root '.unexpected = true')" evidence_contract_invalid
assert_fails "$(mutate extra-nested '.backend.path = "/private/path"')" backend_identity_invalid
assert_fails "$(mutate fixture '.status = "fixture"')" evidence_contract_invalid
assert_fails "$(mutate release-overclaim '.releaseEligible = true')" evidence_contract_invalid
assert_fails \
  "$(mutate production-overclaim '.productionControlQualified = true')" \
  evidence_contract_invalid
assert_fails "$(mutate scope-overclaim '.scope = "s4_mutation_ready"')" evidence_contract_invalid
assert_fails "$(mutate wrong-commit '.commit = ("9" * 40)')" candidate_identity_mismatch
assert_fails "$(mutate wrong-policy '.policySha256 = ("8" * 64)')" candidate_identity_mismatch
assert_fails \
  "$(mutate wrong-workflow-definition '.workflow.definitionSha256 = ("7" * 64)')" \
  workflow_identity_invalid
assert_fails "$(mutate rerun '.workflow.runAttempt = 2')" workflow_identity_invalid
assert_fails "$(mutate wrong-ci-run '.workflow.ciRunId = "2002"')" workflow_identity_invalid
assert_fails \
  "$(mutate no-physical-device '.environment.physicalDeviceObserved = false')" \
  environment_identity_invalid
assert_fails \
  "$(mutate wrong-signer '.environment.shizukuSignerSha256 = ("6" * 64)')" \
  environment_identity_invalid
assert_fails "$(mutate root-backend '.backend.uid = 0')" backend_identity_invalid
assert_fails "$(mutate writable-dex '.backend.dexMode = "0600"')" backend_identity_invalid
assert_fails \
  "$(mutate scenario-reordered '.validation.scenarioResults |= reverse')" \
  validation_contract_invalid
assert_fails \
  "$(mutate mutation-overclaim '.claimBoundary.deviceMutation = true')" \
  claim_boundary_invalid
assert_fails \
  "$(mutate same-uid-overclaim '.claimBoundary.sameUidPersistenceExcluded = true')" \
  claim_boundary_invalid
assert_fails \
  "$(mutate continuous-isolation-overclaim '.claimBoundary.continuousNetworkIsolation = true')" \
  claim_boundary_invalid
assert_fails \
  "$(mutate adversarial-isolation-overclaim '.claimBoundary.adversarialNetworkIsolation = true')" \
  claim_boundary_invalid
assert_fails \
  "$(mutate offline-posture-failed '.validation.controllerOfflinePosturePostCandidate = false')" \
  validation_contract_invalid
assert_fails \
  "$(mutate cleanup-failed '.cleanup.candidateProcessGroupStopped = false')" \
  cleanup_unconfirmed
assert_fails \
  "$(mutate obsolete-pid-cleanup '.cleanup.candidateServerPidStopped = true | del(.cleanup.candidateProcessGroupStopped)')" \
  cleanup_unconfirmed
assert_fails \
  "$(mutate slot-not-quarantined '.cleanup.deviceSlotQuarantinedAfterCandidate = false')" \
  cleanup_unconfirmed
assert_fails \
  "$(mutate invalid-time '.completedAt = "2026-07-31T11:59:59Z"')" \
  observation_time_invalid
assert_fails \
  "$(mutate long-time '.completedAt = "2026-07-31T13:00:01Z"')" \
  observation_time_invalid
assert_fails \
  "$(mutate future-patch '.environment.securityPatch = "2026-08-01"')" \
  environment_identity_invalid

mkdir -m 700 "$ROOT/duplicate-root"
DUPLICATE_ROOT="$ROOT/duplicate-root/$EVIDENCE_FILE_NAME"
printf '{"schemaVersion":1,"schemaVersion":1}\n' >"$DUPLICATE_ROOT"
chmod 600 "$DUPLICATE_ROOT"
assert_fails "$DUPLICATE_ROOT" duplicate_json_key

mkdir -m 700 "$ROOT/duplicate-nested"
DUPLICATE_NESTED="$ROOT/duplicate-nested/$EVIDENCE_FILE_NAME"
printf '{"backend":{"uid":2000,"uid":2000}}\n' >"$DUPLICATE_NESTED"
chmod 600 "$DUPLICATE_NESTED"
assert_fails "$DUPLICATE_NESTED" duplicate_json_key

mkdir -m 700 "$ROOT/non-finite"
NON_FINITE="$ROOT/non-finite/$EVIDENCE_FILE_NAME"
printf '{"value":NaN}\n' >"$NON_FINITE"
chmod 600 "$NON_FINITE"
assert_fails "$NON_FINITE" non_finite_json_number

mkdir -m 700 "$ROOT/oversized"
OVERSIZED="$ROOT/oversized/$EVIDENCE_FILE_NAME"
head -c 65537 /dev/zero | tr '\0' 'x' >"$OVERSIZED"
chmod 600 "$OVERSIZED"
assert_fails "$OVERSIZED" evidence_input_invalid_size

mkdir -m 700 "$ROOT/malformed"
MALFORMED="$ROOT/malformed/$EVIDENCE_FILE_NAME"
printf '\377\n' >"$MALFORMED"
chmod 600 "$MALFORMED"
assert_fails "$MALFORMED" evidence_input_invalid

mkdir -m 700 "$ROOT/symlink"
SYMLINK="$ROOT/symlink/$EVIDENCE_FILE_NAME"
ln -s "$EVIDENCE" "$SYMLINK"
assert_fails "$SYMLINK" evidence_input_invalid

mkdir -m 700 "$ROOT/private-sentinel"
PRIVATE_CASE="$ROOT/private-sentinel/$EVIDENCE_FILE_NAME"
jq --arg private "$PRIVATE_SENTINEL" '.backend.path = $private' \
  "$EVIDENCE" >"$PRIVATE_CASE"
chmod 600 "$PRIVATE_CASE"
assert_fails "$PRIVATE_CASE" backend_identity_invalid

assert_policy_fails() {
  local policy="$1" expected_reason="$2"
  if "${validator_command[@]}" --policy "$policy" --evidence "$EVIDENCE" \
    >"$ROOT/last.stdout" 2>"$ROOT/last.stderr"; then
    fail_test "validator unexpectedly accepted policy case $expected_reason"
  fi
  grep -Fxq \
    "RISH_PHYSICAL_IDENTITY_VALIDATION_RESULT=FAIL reason=$expected_reason" \
    "$ROOT/last.stderr" \
    || fail_test "validator did not report policy case $expected_reason"
  [[ ! -s "$ROOT/last.stdout" ]] \
    || fail_test "failed policy validation wrote standard output"
}

mkdir -m 700 "$ROOT/policy-extra"
POLICY_EXTRA="$ROOT/policy-extra/android-rish-physical-identity-policy-v1.json"
jq '.unexpected = true' "$POLICY" >"$POLICY_EXTRA"
chmod 600 "$POLICY_EXTRA"
assert_policy_fails "$POLICY_EXTRA" policy_contract_invalid

mkdir -m 700 "$ROOT/policy-duplicate"
POLICY_DUPLICATE="$ROOT/policy-duplicate/android-rish-physical-identity-policy-v1.json"
printf '{"schemaVersion":1,"schemaVersion":1}\n' >"$POLICY_DUPLICATE"
chmod 600 "$POLICY_DUPLICATE"
assert_policy_fails "$POLICY_DUPLICATE" duplicate_json_key

mkdir -m 700 "$ROOT/policy-symlink"
POLICY_SYMLINK="$ROOT/policy-symlink/android-rish-physical-identity-policy-v1.json"
ln -s "$POLICY" "$POLICY_SYMLINK"
assert_policy_fails "$POLICY_SYMLINK" policy_input_invalid

printf '%s\n' 'rish physical identity evidence tests passed'
