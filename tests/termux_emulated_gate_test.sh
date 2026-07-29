#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE="$ROOT/scripts/termux_emulated_gate.sh"
VALIDATOR="$ROOT/scripts/termux_release_validate.sh"
BATTERY_GATE="$ROOT/scripts/termux_battery_emulated_gate.sh"
VOLUME_GATE="$ROOT/scripts/termux_volume_emulated_gate.sh"
VOLUME_CONTROL_GATE="$ROOT/scripts/termux_volume_control_emulated_gate.sh"
COMMAND_GATE="$ROOT/scripts/termux_command_emulated_gate.sh"
CLASSIFIER="$ROOT/scripts/classify_observation_requirement.sh"
INHERITANCE="$ROOT/scripts/verify_observation_inheritance.sh"
ANDROID_WORKFLOW="$ROOT/.github/workflows/android-cross-compile.yml"
CI_WORKFLOW="$ROOT/.github/workflows/ci.yml"
SECURITY_WORKFLOW="$ROOT/.github/workflows/security.yml"
LATEST_RUN_SELECTOR="$ROOT/scripts/latest_workflow_run.jq"
COMMIT_HELPER="$ROOT/scripts/commit_verified_file.py"
SOURCE_REPORT="$ROOT/docs/release-evidence/v0.5.1-physical-fe5f7b80.json"
FIXTURE_ROOT="$(mktemp -d)"
trap 'rm -rf -- "$FIXTURE_ROOT"' EXIT INT TERM

fail_test() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

assert_unconditional_main_push() {
  local workflow="$1"
  awk '
    /^  push:$/ { in_push = 1; next }
    in_push && /^  [a-zA-Z_][a-zA-Z_-]*:$/ { exit }
    in_push && /branches: \[ main \]/ { main_branch = 1 }
    in_push && /^[[:space:]]+paths:/ { path_filter = 1 }
    END { exit !(main_branch && !path_filter) }
  ' "$workflow" || fail_test "main push is not unconditional: $workflow"
}

assert_publication_cleanup_contract() {
  local publisher="$1" name case_root cleanup_source staged output
  name="$(basename "$publisher" .sh)"
  case_root="$(mktemp -d "$FIXTURE_ROOT/${name}.publication.XXXXXX")"
  cleanup_source="$case_root/cleanup.sh"
  sed -n '/^cleanup() {$/,/^}$/p' "$publisher" >"$cleanup_source"
  if grep -Eq 'OUTPUT_REPORT|PUBLISH_(LINKED|IDENTITY)' "$cleanup_source"; then
    fail_test "failure cleanup may inspect or unlink the public output: $publisher"
  fi

  staged="$case_root/staged"
  output="$case_root/output"
  printf 'private staging\n' >"$staged"
  printf 'concurrent owner\n' >"$output"
  if (
    set +e
    # shellcheck source=/dev/null
    source "$cleanup_source"
    # The sourced cleanup function consumes these globals.
    # shellcheck disable=SC2034
    SERVER_PID='' \
      BACKGROUND_CURL_PID='' \
      BATTERY_PROGRAM_CREATED=false \
      BATTERY_PROGRAM='' \
      VOLUME_PROGRAM_CREATED=false \
      VOLUME_PROGRAM='' \
      PUBLISH_NEXT="$staged" \
      REPORT_NEXT="$staged" \
      OUTPUT_REPORT="$output" \
      WORK_ROOT=''
    false
    cleanup
  ); then
    fail_test "injected publication failure unexpectedly succeeded: $publisher"
  fi
  [[ ! -e "$staged" && ! -L "$staged" ]] \
    || fail_test "failure cleanup retained private staging: $publisher"
  [[ "$(<"$output")" == 'concurrent owner' ]] \
    || fail_test "failure cleanup removed or changed a public output: $publisher"
}

for script in "$GATE" "$BATTERY_GATE" "$VOLUME_GATE" "$VOLUME_CONTROL_GATE" "$COMMAND_GATE" "$CLASSIFIER" "$INHERITANCE"; do
  bash -n "$script"
  bash "$script" --help | grep -Fq 'Usage:' || fail_test "help output missing for $(basename "$script")"
done

for script in "$BATTERY_GATE" "$VOLUME_GATE" "$COMMAND_GATE"; do
  [[ "$(grep -Fc '"read_text_range",' "$script")" == 2 ]] \
    || fail_test "enabled/disabled UTF-8 range allowlist parity missing for $(basename "$script")"
done
grep -Fq '"read_text_range","search_text"' "$GATE" \
  || fail_test 'baseline native gate UTF-8 range allowlist parity missing'
grep -Fq '"read_text_range","search_text"' "$VOLUME_CONTROL_GATE" \
  || fail_test 'volume-control native gate UTF-8 range allowlist parity missing'

for script in "$GATE" "$BATTERY_GATE" "$VOLUME_GATE" "$VOLUME_CONTROL_GATE" "$COMMAND_GATE"; do
  grep -Fq 'fileWriteMutationEnabled == false' "$script" \
    || fail_test "default-disabled write status missing for $(basename "$script")"
  grep -Fq 'write_file_mutation_disabled' "$script" \
    || fail_test "live write denial missing for $(basename "$script")"
  grep -Fq 'inputSchema.properties.dry_run.const' "$script" \
    || fail_test "write discovery const missing for $(basename "$script")"
  grep -Fq 'MCP__FILE__TRASH_FILE_MUTATION_ENABLED=false' "$script" \
    || fail_test "trash_file mutation is not pinned disabled for $(basename "$script")"
  grep -Fq 'dedicated trash mutation gate is disabled' "$script" \
    || fail_test "trash_file disabled discovery schema is not asserted for $(basename "$script")"
  grep -Fq 'inputSchema.properties | keys) == ["dry_run","path"]' "$script" \
    || fail_test "trash_file closed discovery properties are not asserted for $(basename "$script")"
  grep -Fq 'inputSchema.required == ["path"]' "$script" \
    || fail_test "trash_file discovery required path is not asserted for $(basename "$script")"
  grep -Fq 'inputSchema.additionalProperties == false' "$script" \
    || fail_test "trash_file discovery additional properties are not rejected for $(basename "$script")"
  grep -Fq 'trashFileMutationEnabled == false' "$script" \
    || fail_test "trash_file disabled runtime status is not asserted for $(basename "$script")"
  grep -Fq 'trashFileMode == "dry_run_only_mutation_disabled"' "$script" \
    || fail_test "trash_file disabled runtime mode is not asserted for $(basename "$script")"
  grep -Fq 'trashFileGrantRequired == false' "$script" \
    || fail_test "trash_file disabled grant status is not asserted for $(basename "$script")"
  grep -Fq 'trashFileQuarantineMaxArtifacts == 32' "$script" \
    || fail_test "trash_file bounded quarantine status is not asserted for $(basename "$script")"
  grep -Fq 'params:{name:"trash_file"' "$script" \
    || fail_test "trash_file disabled direct call is not exercised for $(basename "$script")"
  grep -Fq 'trash_file_mutation_disabled' "$script" \
    || fail_test "trash_file live disabled denial is not asserted for $(basename "$script")"
  grep -Fq '.termux-mcp-trash-quarantine' "$script" \
    || fail_test "trash_file disabled quarantine non-mutation is not asserted for $(basename "$script")"
  grep -Fq 'target_mutated' "$script" \
    || fail_test "trash_file disabled target non-mutation is not asserted for $(basename "$script")"
done
grep -Fq 'MCP__FILE__COPY_FILE_MUTATION_ENABLED=false' "$GATE" \
  || fail_test 'baseline native gate does not pin copy_file mutation disabled'
grep -Fq 'stress_copy_file_disabled_status_invalid' "$GATE" \
  || fail_test 'baseline native gate omits live copy_file disabled denial'
grep -Fq 'copyFileMutationDisabled: true' "$GATE" \
  || fail_test 'baseline native gate evidence omits copy_file disabled posture'
grep -Fq 'stress_root_identity_redirected' "$GATE" \
  || fail_test 'baseline native gate omits safe-root replacement attack'
grep -Fq 'stress_ancestor_identity_redirected' "$GATE" \
  || fail_test 'baseline native gate omits safe-root ancestor replacement attack'
grep -Fq 'write-key-isolation' "$VOLUME_CONTROL_GATE" \
  || fail_test 'shared volume capability key is not isolated from write_file'
grep -Fq '"${payload:128:2}" == 03' "$VOLUME_CONTROL_GATE" \
  || fail_test 'volume-control native gate does not pin signed capability byte 3'
for code in \
  expanded_body_posture_verified \
  safe_root_file_create_replace_verified \
  request_scoped_single_use_write_grant_enforced \
  exact_write_file_byte_limit_verified \
  bounded_write_file_response_preflight_verified \
  request_scoped_single_use_copy_grant_enforced \
  source_content_destination_binding_enforced \
  exact_binary_copy_verified \
  copy_file_boundary_denials_verified \
  copy_file_private_audit_verified \
  copy_file_disabled_posture_verified \
  safe_root_file_trash_verified \
  request_scoped_trash_grant_enforced \
  trash_identity_content_binding_enforced \
  bounded_trash_file_response_preflight_verified \
  exact_trash_file_byte_limit_verified \
  trash_recovery_quarantine_verified \
  trash_file_private_audit_verified \
  trash_file_disabled_posture_verified \
  full_suite_default_disabled_17_tool_posture_verified \
  full_suite_enabled_21_tool_posture_verified \
  full_suite_optional_provider_success_verified \
  full_suite_volume_preview_and_grant_boundary_verified \
  full_suite_command_basename_and_profile_verified \
  full_suite_filesystem_mutations_independently_disabled
do
  grep -Fq "$code" "$GATE" \
    || fail_test "canonical emulation gate omits required validator evidence: $code"
  grep -Fq "$code" "$VALIDATOR" \
    || fail_test "release validator cannot emit canonical emulation evidence: $code"
done
grep -Fq '.validatorVersion == "11"' "$GATE" \
  || fail_test 'canonical emulation gate does not require release validator v11'
grep -Fq 'readonly VALIDATOR_VERSION="11"' "$VALIDATOR" \
  || fail_test 'release validator version does not match canonical emulation gate requirement'

if bash "$GATE" >"$ROOT/.termux-emulated-test.stdout" 2>"$ROOT/.termux-emulated-test.stderr"; then
  fail_test 'gate without required arguments unexpectedly succeeded'
fi
grep -Fq 'reason=expected_commit_invalid' "$ROOT/.termux-emulated-test.stderr" || fail_test 'gate missing deterministic argument failure'

if bash "$BATTERY_GATE" >"$ROOT/.termux-battery-test.stdout" 2>"$ROOT/.termux-battery-test.stderr"; then
  fail_test 'battery gate without required arguments unexpectedly succeeded'
fi
grep -Fq 'reason=expected_commit_invalid' "$ROOT/.termux-battery-test.stderr" || fail_test 'battery gate missing deterministic argument failure'

if bash "$VOLUME_GATE" >"$ROOT/.termux-volume-test.stdout" 2>"$ROOT/.termux-volume-test.stderr"; then
  fail_test 'volume gate without required arguments unexpectedly succeeded'
fi
grep -Fq 'reason=expected_commit_invalid' "$ROOT/.termux-volume-test.stderr" || fail_test 'volume gate missing deterministic argument failure'

if bash "$VOLUME_CONTROL_GATE" >"$ROOT/.termux-volume-control-test.stdout" 2>"$ROOT/.termux-volume-control-test.stderr"; then
  fail_test 'volume control gate without required arguments unexpectedly succeeded'
fi
grep -Fq 'reason=expected_commit_invalid' "$ROOT/.termux-volume-control-test.stderr" || fail_test 'volume control gate missing deterministic argument failure'

if bash "$COMMAND_GATE" >"$ROOT/.termux-command-test.stdout" 2>"$ROOT/.termux-command-test.stderr"; then
  fail_test 'command gate without required arguments unexpectedly succeeded'
fi
grep -Fq 'reason=expected_commit_invalid' "$ROOT/.termux-command-test.stderr" || fail_test 'command gate missing deterministic argument failure'

if bash "$CLASSIFIER" >"$ROOT/.termux-classifier-test.stdout" 2>"$ROOT/.termux-classifier-test.stderr"; then
  fail_test 'observation classifier without required arguments unexpectedly succeeded'
fi
grep -Fq 'reason=commit_invalid' "$ROOT/.termux-classifier-test.stderr" || fail_test 'observation classifier missing deterministic argument failure'

if bash "$INHERITANCE" >"$ROOT/.termux-inheritance-test.stdout" 2>"$ROOT/.termux-inheritance-test.stderr"; then
  fail_test 'inheritance verifier without required arguments unexpectedly succeeded'
fi
grep -Fq 'reason=commit_invalid' "$ROOT/.termux-inheritance-test.stderr" || fail_test 'inheritance verifier missing deterministic argument failure'

rm -f -- \
  "$ROOT/.termux-emulated-test.stdout" "$ROOT/.termux-emulated-test.stderr" \
  "$ROOT/.termux-battery-test.stdout" "$ROOT/.termux-battery-test.stderr" \
  "$ROOT/.termux-volume-test.stdout" "$ROOT/.termux-volume-test.stderr" \
  "$ROOT/.termux-volume-control-test.stdout" "$ROOT/.termux-volume-control-test.stderr" \
  "$ROOT/.termux-command-test.stdout" "$ROOT/.termux-command-test.stderr" \
  "$ROOT/.termux-classifier-test.stdout" "$ROOT/.termux-classifier-test.stderr" \
  "$ROOT/.termux-inheritance-test.stdout" "$ROOT/.termux-inheritance-test.stderr"

printf '%s  %s\n' \
  ed86dbee150a42f4dd2775c79f9c358ec1b0a4420661ecc62b47b7be9d741568 "$ROOT/docs/android-battery-emulated-evidence-schema-v2.json" \
  506a7f5ef3cf3fa0a1e83777d8edc722c132e99f9baf36bf3e5bdf0b448dab63 "$ROOT/docs/android-volume-emulated-evidence-schema-v1.json" \
  8a90efefbffc5ee5d37660f7dce375084c3f42be2ee74b806fa10ee7a382d8ab "$ROOT/docs/android-volume-control-emulated-evidence-schema-v1.json" \
  e2fb1da72669351805d360259f4ce3c209ce5c32c882d408988247edd2436fc0 "$ROOT/docs/command-emulated-evidence-schema-v2.json" \
  | sha256sum -c - >/dev/null \
  || fail_test 'historical specialized evidence schema bytes changed'

python3 - \
  "$ROOT/docs/android-battery-emulated-evidence-schema-v3.json" \
  "$ROOT/docs/android-volume-emulated-evidence-schema-v2.json" \
  "$ROOT/docs/android-volume-control-emulated-evidence-schema-v2.json" \
  "$ROOT/docs/command-emulated-evidence-schema-v3.json" <<'PY'
import json
import pathlib
import sys


def closed_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError(f"duplicate JSON key: {key}")
        value[key] = item
    return value


for raw_path in sys.argv[1:]:
    with pathlib.Path(raw_path).open("r", encoding="utf-8") as source:
        json.load(source, object_pairs_hook=closed_object)
PY

jq -e '
  .properties.schemaVersion.const == 2
  and .properties.gateVersion.const == "2"
  and .properties.status.const == "pass"
  and .properties.releaseQualificationEligible.const == false
  and .properties.environment.properties.executionMode.const == "official-termux-docker-native-arm64"
  and .properties.environment.properties.androidLinker.const == true
  and .properties.candidate.properties.androidVolumeControlArtifact."$ref" == "#/$defs/artifact"
  and .properties.stress.properties.samples.minimum == 32
  and .properties.stress.properties.highImpactDisabled.const == true
  and .properties.stress.properties.copyFileMutationDisabled.const == true
  and .properties.stress.properties.safeRootIdentityPinned.const == true
  and .properties.stress.properties.safeRootAncestorIdentityPinned.const == true
  and .properties.stress.properties.longObservationRequired.const == false
' "$ROOT/docs/emulated-release-evidence-schema-v2.json" >/dev/null

jq -e '
  ."$id" == "https://github.com/CyberBASSLord-666/termux-mcp-edge/blob/main/docs/emulated-release-evidence-schema-v3.json"
  and .title == "Termux MCP Edge emulated exact-artifact evidence v3"
  and .properties.schemaVersion.const == 3
  and .properties.gateVersion.const == "3"
  and .properties.status.const == "pass"
  and .properties.releaseQualificationEligible.const == false
  and .properties.candidate.properties.fullSuiteArtifact."$ref" == "#/$defs/fullSuiteArtifact"
  and ."$defs".fullSuiteArtifact.properties.artifactName.const == "termux-mcp-server-aarch64-linux-android-full-suite"
  and ."$defs".fullSuiteArtifact.properties.posture.const == "full-suite"
  and ."$defs".fullSuiteArtifact.properties.features.const == ["full-suite"]
  and ."$defs".fullSuiteArtifact.properties.fileName.const == "termux-mcp-server"
  and .properties.aggregateValidation.properties.defaultDisabled."$ref" == "#/$defs/defaultDisabled"
  and .properties.aggregateValidation.properties.fullyEnabled."$ref" == "#/$defs/fullyEnabled"
  and .properties.aggregateValidation.properties.independentRuntimeGates.const == true
  and ."$defs".defaultDisabled.properties.toolCount.const == 17
  and ."$defs".defaultDisabled.properties.runtimeFlagsOmitted.const == true
  and ."$defs".fullyEnabled.properties.toolCount.const == 21
  and ."$defs".fullyEnabled.properties.volumePreviewNoMutation.const == true
  and ."$defs".fullyEnabled.properties.volumeGrantIsolation.const == true
  and ."$defs".fullyEnabled.properties.commandExecutableIdentityPinned.const == true
  and .properties.aggregateValidation.properties.filesystemMutationsDisabled.const == true
  and .properties.aggregateValidation.properties.boundedCleanup.const == true
  and .properties.aggregateValidation.properties.directPhysicalObservationRequired.const == true
' "$ROOT/docs/emulated-release-evidence-schema-v3.json" >/dev/null

jq -e '
  .properties.schemaVersion.const == 4
  and .properties.gateVersion.const == "4"
  and .properties.status.const == "pass"
  and .properties.releaseQualificationEligible.const == false
  and .properties.claimBoundary."$ref" == "#/$defs/claimBoundary"
  and .properties.environment.properties.imageDigest.pattern == "^sha256:[0-9a-f]{64}$"
  and .properties.environment.properties.rootfsImageId.pattern == "^sha256:[0-9a-f]{64}$"
  and .properties.environment.properties.runtimeImageDigest.pattern == "^sha256:[0-9a-f]{64}$"
  and ."$defs".claimBoundary.properties.physicalDeviceObserved.const == false
  and ."$defs".claimBoundary.properties.androidFrameworkObserved.const == false
  and ."$defs".claimBoundary.properties.sustainedPhysicalSoak.const == false
  and ."$defs".claimBoundary.properties.physicalCertification.const == "not_run"
  and .properties.coverage.properties.covered.const == [
    "exact_android_artifacts",
    "official_termux_userland_native_arm64",
    "android_bionic_linker",
    "deterministic_provider_simulation",
    "runtime_gate_composition",
    "bounded_native_stress"
  ]
  and .properties.aggregateValidation.properties.automatedQualificationComponent.const == true
' "$ROOT/docs/emulated-release-evidence-schema-v4.json" >/dev/null

jq -e '
  .properties.schemaVersion.const == 3
  and .properties.gateVersion.const == "3"
  and .properties.releaseQualificationEligible.const == false
  and .properties.environment."$ref" == "#/$defs/environment"
  and ."$defs".environment.properties.executionMode.const == "official-termux-docker-native-arm64"
  and (."$defs".environment.required | index("rootfsImageId") != null)
  and (."$defs".environment.required | index("runtimeImageDigest") != null)
  and ."$defs".environment.properties.rootfsImageId."$ref" == "#/$defs/ociDigest"
  and ."$defs".environment.properties.runtimeImageDigest."$ref" == "#/$defs/ociDigest"
  and ."$defs".validation.properties.runtimeDefaultDisabled.const == true
  and ."$defs".validation.properties.fixedProgram.const == true
  and ."$defs".validation.properties.fixedWorkingDirectory.const == true
  and ."$defs".validation.properties.inheritedEnvironmentCleared.const == true
  and ."$defs".validation.properties.boundedOutput.const == true
  and ."$defs".validation.properties.immediateOverflowTermination.const == true
  and ."$defs".validation.properties.processGroupIsolation.const == true
  and ."$defs".validation.properties.pipeHoldingDescendantCleanup.const == true
  and ."$defs".validation.properties.callerCancellationCleanup.const == true
  and ."$defs".validation.properties.boundedSupervisorCleanup.const == true
  and ."$defs".validation.properties.androidDeviceControlDisabled.const == true
' "$ROOT/docs/android-battery-emulated-evidence-schema-v3.json" >/dev/null

jq -e '
  ."$id" == "https://github.com/CyberBASSLord-666/termux-mcp-edge/blob/main/docs/android-volume-emulated-evidence-schema-v2.json"
  and .title == "Termux MCP Android volume native emulation evidence v2"
  and .properties.schemaVersion.const == 2
  and .properties.gateVersion.const == "2"
  and .properties.releaseQualificationEligible.const == false
  and .properties.environment."$ref" == "#/$defs/environment"
  and ."$defs".environment.properties.executionMode.const == "official-termux-docker-native-arm64"
  and (."$defs".environment.required | index("rootfsImageId") != null)
  and (."$defs".environment.required | index("runtimeImageDigest") != null)
  and ."$defs".environment.properties.rootfsImageId."$ref" == "#/$defs/ociDigest"
  and ."$defs".environment.properties.runtimeImageDigest."$ref" == "#/$defs/ociDigest"
  and ."$defs".validation.properties.runtimeDefaultDisabled.const == true
  and ."$defs".validation.properties.fixedProgram.const == true
  and ."$defs".validation.properties.fixedWorkingDirectory.const == true
  and ."$defs".validation.properties.noArguments.const == true
  and ."$defs".validation.properties.inheritedEnvironmentCleared.const == true
  and ."$defs".validation.properties.normalizedAllowlist.const == true
  and ."$defs".validation.properties.canonicalStreamOrdering.const == true
  and ."$defs".validation.properties.unrecognizedFieldsRejected.const == true
  and ."$defs".validation.properties.boundedOutput.const == true
  and ."$defs".validation.properties.immediateOverflowTermination.const == true
  and ."$defs".validation.properties.processGroupIsolation.const == true
  and ."$defs".validation.properties.pipeHoldingDescendantCleanup.const == true
  and ."$defs".validation.properties.callerCancellationCleanup.const == true
  and ."$defs".validation.properties.boundedSupervisorCleanup.const == true
  and ."$defs".validation.properties.androidDeviceControlDisabled.const == true
' "$ROOT/docs/android-volume-emulated-evidence-schema-v2.json" >/dev/null

jq -e '
  ."$id" == "https://github.com/CyberBASSLord-666/termux-mcp-edge/blob/main/docs/android-volume-control-emulated-evidence-schema-v2.json"
  and .title == "Termux MCP Android volume control native emulation evidence v2"
  and .properties.schemaVersion.const == 2
  and .properties.gateVersion.const == "2"
  and .properties.releaseQualificationEligible.const == false
  and ."$defs".candidate.required == ["commit","version","ciRunId","securityRunId","androidRunId","artifact","incompatibleArtifact"]
  and ."$defs".environment.properties.executionMode.const == "official-termux-docker-native-arm64"
  and (."$defs".environment.required | index("rootfsImageId") != null)
  and (."$defs".environment.required | index("runtimeImageDigest") != null)
  and ."$defs".environment.properties.rootfsImageId."$ref" == "#/$defs/ociDigest"
  and ."$defs".environment.properties.runtimeImageDigest."$ref" == "#/$defs/ociDigest"
  and ."$defs".validation.properties.compileGate.const == true
  and ."$defs".validation.properties.runtimeDefaultDisabled.const == true
  and ."$defs".validation.properties.previewDoesNotConsumeGrant.const == true
  and ."$defs".validation.properties.headerContextEnforced.const == true
  and ."$defs".validation.properties.exactGrantBinding.const == true
  and ."$defs".validation.properties.singleUseReplay.const == true
  and ."$defs".validation.properties.exactTwoArguments.const == true
  and ."$defs".validation.properties.nonQueueingConcurrency.const == true
  and ."$defs".validation.properties.mutationVerified.const == true
  and ."$defs".validation.properties.rollbackConfirmed.const == true
  and ."$defs".validation.properties.rollbackUnconfirmed.const == true
  and ."$defs".validation.properties.cancellationIndependentRecovery.const == true
  and ."$defs".validation.properties.longObservationRequired.const == false
' "$ROOT/docs/android-volume-control-emulated-evidence-schema-v2.json" >/dev/null

jq -e '
  ."$id" == "https://github.com/CyberBASSLord-666/termux-mcp-edge/blob/main/docs/command-emulated-evidence-schema-v3.json"
  and .title == "Termux MCP fixed command profile native emulation evidence v3"
  and .properties.schemaVersion.const == 3
  and .properties.gateVersion.const == "3"
  and .properties.releaseQualificationEligible.const == false
  and .properties.candidate."$ref" == "#/$defs/candidate"
  and ."$defs".candidate.required == ["commit","version","ciRunId","securityRunId","androidRunId","artifact","defaultArtifact"]
  and ."$defs".environment.properties.executionMode.const == "official-termux-docker-native-arm64"
  and (."$defs".environment.required | index("rootfsImageId") != null)
  and (."$defs".environment.required | index("runtimeImageDigest") != null)
  and ."$defs".environment.properties.rootfsImageId."$ref" == "#/$defs/ociDigest"
  and ."$defs".environment.properties.runtimeImageDigest."$ref" == "#/$defs/ociDigest"
  and ."$defs".validation.properties.requests.const == 29
  and ."$defs".validation.properties.compileGate.const == true
  and ."$defs".validation.properties.runtimeDefaultDisabled.const == true
  and ."$defs".validation.properties.fixedCurrentExecutable.const == true
  and ."$defs".validation.properties.wrongExecutableNameFailsClosed.const == true
  and ."$defs".validation.properties.wrongExecutableNameRejectedBeforeServing.const == true
  and ."$defs".validation.properties.runningInodePinned.const == true
  and ."$defs".validation.properties.workingDirectoryDescriptorPinned.const == true
  and (."$defs".validation.required | index("wrongExecutableNameFailsClosed") != null)
  and (."$defs".validation.required | index("wrongExecutableNameRejectedBeforeServing") != null)
  and (."$defs".validation.required | index("workingDirectoryDescriptorPinned") != null)
  and ."$defs".validation.properties.fixedArgvProfiles.const == true
  and ."$defs".validation.properties.closedInputSchema.const == true
  and ."$defs".validation.properties.overrideFieldsRejected.const == true
  and ."$defs".validation.properties.fixedWorkingDirectory.const == true
  and ."$defs".validation.properties.inheritedEnvironmentCleared.const == true
  and ."$defs".validation.properties.nullStdin.const == true
  and ."$defs".validation.properties.boundedOutput.const == true
  and ."$defs".validation.properties.auditCounters.const == true
  and ."$defs".validation.properties.arbitraryCommandExecutionDisabled.const == true
  and ."$defs".validation.properties.longObservationRequired.const == false
' "$ROOT/docs/command-emulated-evidence-schema-v3.json" >/dev/null
grep -Fq 'EXPECTED_REQUEST_COUNT=29' "$COMMAND_GATE" \
  || fail_test 'command gate omits its exact request-count contract'
grep -Fq '((REQUEST_COUNT == EXPECTED_REQUEST_COUNT)) || fail request_count_invalid' "$COMMAND_GATE" \
  || fail_test 'command gate omits its runtime exact-count assertion'
grep -Fq "validating loaded executable and working-directory inode replacement isolation" "$COMMAND_GATE" \
  || fail_test 'command gate omits combined executable/cwd inode isolation'
grep -Fq 'start_server true "$PINNED_ARTIFACT" /' "$COMMAND_GATE" \
  || fail_test 'combined inode phase does not launch from filesystem root'
grep -Fq "printf '%s' \"\$SAFE_ROOT_REPLACEMENT_CONTENT\" >\"\$SAFE_ROOT\"" "$COMMAND_GATE" \
  || fail_test 'combined inode phase does not replace the cwd pathname with a file'
grep -Fq '"profile":"execution_boundary"' "$COMMAND_GATE" \
  || fail_test 'combined inode phase does not exercise the cwd boundary self-check'
grep -Fq 'executable_path_replacement_ran' "$COMMAND_GATE" \
  || fail_test 'command gate omits executable replacement marker assertion'
grep -Fq 'working_directory_path_replacement_used' "$COMMAND_GATE" \
  || fail_test 'command gate omits cwd replacement-content assertion'
grep -Fq 'wrongExecutableNameFailsClosed: true' "$COMMAND_GATE" \
  || fail_test 'command report omits precise wrong-name fail-closed evidence'
grep -Fq 'wrongExecutableNameRejectedBeforeServing: true' "$COMMAND_GATE" \
  || fail_test 'command report omits pre-service wrong-name rejection evidence'
grep -Fq 'workingDirectoryDescriptorPinned: true' "$COMMAND_GATE" \
  || fail_test 'command report omits descriptor-pinned cwd evidence'
grep -Fq "validating wrong executable name is rejected before serving" "$COMMAND_GATE" \
  || fail_test 'command gate omits wrong-name pre-service rejection posture'
grep -Fq 'the command execution client could not be initialized' "$COMMAND_GATE" \
  || fail_test 'command gate omits the typed command-client construction error'
grep -Fq 'wrong_name_construction_error_leaked_token' "$COMMAND_GATE" \
  || fail_test 'command gate omits wrong-name token-redaction evidence'
grep -Fq 'wrong_name_construction_error_leaked_path' "$COMMAND_GATE" \
  || fail_test 'command gate omits wrong-name path-redaction evidence'
grep -Fq 'wrong_name_service_announced' "$COMMAND_GATE" \
  || fail_test 'command gate omits pre-service log evidence'
grep -Fq 'wrong_name_service_reachable' "$COMMAND_GATE" \
  || fail_test 'command gate omits pre-service reachability evidence'
grep -Fq 'wrong_name_reachable=false' "$COMMAND_GATE" \
  || fail_test 'command gate omits the bounded live reachability probe'
grep -Fq 'wrong_name_reachable=true' "$COMMAND_GATE" \
  || fail_test 'command gate cannot record a live service failure'
if grep -Fq '"id":"wrong-name-' "$COMMAND_GATE"; then
  fail_test 'command gate still treats invalid command-client initialization as a live MCP posture'
fi

jq -e '
  .properties.releaseQualificationEligible.const == false
  and (.properties.evidenceMode.enum | index("physical_observation_required") != null)
  and (.properties.reasonCode.enum | index("runtime_and_build_inputs_changed") != null)
  and ."$defs".emulation.properties.executionMode.const == "official-termux-docker-native-arm64"
  and .allOf[0].then.properties.changedInputClasses.maxItems == 0
  and .allOf[0].else.properties.changedInputClasses.minItems == 1
' "$ROOT/docs/release-observation-requirement-schema-v1.json" >/dev/null

jq -e '
  .properties.schemaVersion.const == 2
  and .properties.classifierVersion.const == "2"
  and .properties.releaseQualificationEligible.const == false
  and ."$defs".candidate.properties.fullSuiteArtifactSha256."$ref" == "#/$defs/sha256"
  and ."$defs".candidate.properties.fullSuiteManifestSha256."$ref" == "#/$defs/sha256"
  and ."$defs".emulation.properties.executionMode.const == "official-termux-docker-native-arm64"
  and (.properties.reasonCode.enum | index("full_suite_direct_physical_observation_required") != null)
  and (.properties.changedInputClasses.items.enum | index("full_suite_artifact") != null)
  and .allOf[0].then.properties.changedInputClasses.maxItems == 0
  and .allOf[0].else.properties.changedInputClasses.minItems == 1
' "$ROOT/docs/release-observation-requirement-schema-v2.json" >/dev/null

jq -e '
  .properties.schemaVersion.const == 3
  and .properties.classifierVersion.const == "3"
  and .properties.releaseQualificationEligible.const == false
  and .properties.evidenceMode.const == "automated_release_qualification"
  and .properties.reasonCode.const == "automated_native_termux_evidence_required"
  and .properties.inheritanceCandidate.const == false
  and .properties.nextGate.const == "assemble_automated_release_qualification"
  and .properties.changedInputClasses.maxItems == 2
  and (.allOf | length) == 2
  and .allOf[0].if.properties.protectedInputComparison.properties.runtimeAndDeploymentInputsUnchanged.const == true
  and .allOf[0].then.properties.changedInputClasses.not.contains.const == "runtime_or_deployment"
  and .allOf[0].else.properties.changedInputClasses.contains.const == "runtime_or_deployment"
  and .allOf[1].if.properties.protectedInputComparison.properties.cargoAndDependencyInputsUnchangedExceptRootVersion.const == true
  and .allOf[1].then.properties.changedInputClasses.not.contains.const == "cargo_or_dependency"
  and .allOf[1].else.properties.changedInputClasses.contains.const == "cargo_or_dependency"
  and .properties.claimBoundary."$ref" == "#/$defs/claimBoundary"
  and ."$defs".claimBoundary.properties.physicalDeviceObserved.const == false
  and ."$defs".claimBoundary.properties.androidFrameworkObserved.const == false
  and ."$defs".claimBoundary.properties.sustainedPhysicalSoak.const == false
  and ."$defs".claimBoundary.properties.physicalCertification.const == "not_run"
' "$ROOT/docs/release-observation-requirement-schema-v3.json" >/dev/null

jq -e '
  .properties.releaseQualificationEligible.const == true
  and .properties.evidenceMode.const == "inherited_physical_observation"
  and .properties.sourceObservation.properties.physicalDevice.const == true
  and .properties.sourceObservation.properties.minutes.minimum == 60
  and .properties.equivalence.properties.runtimeSourceUnchanged.const == true
  and .properties.equivalence.properties.candidateArtifactsMatchBridge.const == true
' "$ROOT/docs/release-observation-inheritance-schema-v1.json" >/dev/null

test "$(sha256sum "$SOURCE_REPORT" | awk '{print $1}')" = 677796015065eb193ac78b2dd200de64efccb95a226837a4545c85021cb9283c

FIXTURE_REPOSITORY="$FIXTURE_ROOT/repository"
mkdir -p "$FIXTURE_REPOSITORY/src" "$FIXTURE_ROOT/output"
chmod 700 "$FIXTURE_ROOT/output"
git -C "$FIXTURE_REPOSITORY" init -q
git -C "$FIXTURE_REPOSITORY" config user.name 'Termux MCP Test'
git -C "$FIXTURE_REPOSITORY" config user.email 'termux-mcp-test@example.invalid'
cat >"$FIXTURE_REPOSITORY/Cargo.toml" <<'EOF'
[package]
name = "termux-mcp-server"
version = "0.5.1"
edition = "2021"
EOF
cat >"$FIXTURE_REPOSITORY/Cargo.lock" <<'EOF'
version = 4

[[package]]
name = "termux-mcp-server"
version = "0.5.1"
EOF
printf '%s\n' 'pub fn baseline() {}' >"$FIXTURE_REPOSITORY/src/lib.rs"
git -C "$FIXTURE_REPOSITORY" add Cargo.toml Cargo.lock src/lib.rs
git -C "$FIXTURE_REPOSITORY" commit -q -m baseline
FIXTURE_SOURCE="$(git -C "$FIXTURE_REPOSITORY" rev-parse HEAD)"

printf '%s\n' 'documentation only' >"$FIXTURE_REPOSITORY/README.md"
git -C "$FIXTURE_REPOSITORY" add README.md
git -C "$FIXTURE_REPOSITORY" commit -q -m documentation
EQUIVALENT_CANDIDATE="$(git -C "$FIXTURE_REPOSITORY" rev-parse HEAD)"
EQUIVALENT_EMULATED="$FIXTURE_ROOT/equivalent-emulated.json"
jq -n \
  --arg commit "$EQUIVALENT_CANDIDATE" '
  {
    schemaVersion: 4,
    gateVersion: "4",
    status: "pass",
    failureCode: null,
    releaseQualificationEligible: false,
    startedAt: "2026-07-23T00:00:00Z",
    completedAt: "2026-07-23T00:01:00Z",
    candidate: {
      commit: $commit,
      version: "0.6.0",
      ciRunId: "1001",
      securityRunId: "1002",
      androidRunId: "1003",
      fullSuiteArtifact: {
        sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        manifestSha256: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        artifactName: "termux-mcp-server-aarch64-linux-android-full-suite",
        posture: "full-suite",
        features: ["full-suite"],
        fileName: "termux-mcp-server"
      }
    },
    environment: {
      executionMode: "official-termux-docker-native-arm64",
      androidLinker: true,
      imageDigest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    },
    claimBoundary: {
      physicalDeviceObserved: false,
      androidFrameworkObserved: false,
      sustainedPhysicalSoak: false,
      physicalCertification: "not_run"
    },
    coverage: {
      covered: [
        "exact_android_artifacts",
        "official_termux_userland_native_arm64",
        "android_bionic_linker",
        "deterministic_provider_simulation",
        "runtime_gate_composition",
        "bounded_native_stress"
      ],
      notCovered: [
        "physical_device",
        "android_framework",
        "oem_policy",
        "battery_aging",
        "thermal_soak",
        "radio",
        "doze"
      ]
    },
    runtimeValidation: {status: "pass"},
    aggregateValidation: {
      status: "pass",
      defaultDisabled: {toolCount: 17},
      fullyEnabled: {toolCount: 21},
      automatedQualificationComponent: true
    },
    stress: {
      status: "pass",
      samples: 32,
      safeRootIdentityPinned: true,
      safeRootAncestorIdentityPinned: true,
      longObservationRequired: false
    }
  }' >"$EQUIVALENT_EMULATED"

bash "$CLASSIFIER" \
  --repository-root "$FIXTURE_REPOSITORY" \
  --source-commit "$FIXTURE_SOURCE" \
  --candidate-commit "$EQUIVALENT_CANDIDATE" \
  --emulated-report "$EQUIVALENT_EMULATED" \
  --output "$FIXTURE_ROOT/output/equivalent.json" >/dev/null
jq -e '
  .inheritanceCandidate == false
  and .schemaVersion == 3
  and .classifierVersion == "3"
  and .releaseQualificationEligible == false
  and .evidenceMode == "automated_release_qualification"
  and .reasonCode == "automated_native_termux_evidence_required"
  and .changedInputClasses == []
  and .nextGate == "assemble_automated_release_qualification"
  and .claimBoundary.physicalDeviceObserved == false
  and .claimBoundary.androidFrameworkObserved == false
  and .claimBoundary.sustainedPhysicalSoak == false
  and .claimBoundary.physicalCertification == "not_run"
  and .candidate.fullSuiteArtifactSha256 == "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  and .candidate.fullSuiteManifestSha256 == "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
' "$FIXTURE_ROOT/output/equivalent.json" >/dev/null
[[ "$(stat -c %a "$FIXTURE_ROOT/output/equivalent.json")" == 600 ]] || fail_test 'classifier output is not private'

RACE_BIN="$FIXTURE_ROOT/race-bin"
RACE_OUTPUT="$FIXTURE_ROOT/output/classifier-race.json"
RACE_LOG="$FIXTURE_ROOT/classifier-race.log"
REAL_PYTHON3="$(command -v python3)"
mkdir -m 700 "$RACE_BIN"
cat >"$RACE_BIN/python3" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
destination=''
previous=''
for argument in "$@"; do
  if [[ "$previous" == --destination ]]; then
    destination="$argument"
    break
  fi
  previous="$argument"
done
if [[ -n "$destination" && "$destination" == "${TERMUX_MCP_RACE_OUTPUT:-}" ]]; then
  printf '%s\n' 'race-sentinel' >"$destination"
fi
exec "$TERMUX_MCP_REAL_PYTHON3" "$@"
EOF
chmod 700 "$RACE_BIN/python3"
set +e
PATH="$RACE_BIN:$PATH" \
  TERMUX_MCP_REAL_PYTHON3="$REAL_PYTHON3" \
  TERMUX_MCP_RACE_OUTPUT="$RACE_OUTPUT" \
  bash "$CLASSIFIER" \
    --repository-root "$FIXTURE_REPOSITORY" \
    --source-commit "$FIXTURE_SOURCE" \
    --candidate-commit "$EQUIVALENT_CANDIDATE" \
    --emulated-report "$EQUIVALENT_EMULATED" \
    --output "$RACE_OUTPUT" >"$RACE_LOG" 2>&1
race_rc=$?
set -e
((race_rc != 0)) || fail_test 'classifier output publication race did not fail closed'
grep -Fq 'reason=output_already_exists' "$RACE_LOG" \
  || fail_test 'classifier output publication race returned the wrong failure'
[[ "$(<"$RACE_OUTPUT")" == race-sentinel ]] \
  || fail_test 'classifier output publication race clobbered the competing file'
if find "$FIXTURE_ROOT/output" -maxdepth 1 -name '.observation-requirement.*' -print -quit | grep -q .; then
  fail_test 'classifier output publication race left a temporary file'
fi

printf '%s\n' 'pub fn changed_runtime() {}' >"$FIXTURE_REPOSITORY/src/lib.rs"
cat >>"$FIXTURE_REPOSITORY/Cargo.toml" <<'EOF'

[features]
android-battery-status = []
EOF
git -C "$FIXTURE_REPOSITORY" add Cargo.toml src/lib.rs
git -C "$FIXTURE_REPOSITORY" commit -q -m runtime-change
CHANGED_CANDIDATE="$(git -C "$FIXTURE_REPOSITORY" rev-parse HEAD)"
CHANGED_EMULATED="$FIXTURE_ROOT/changed-emulated.json"
jq --arg commit "$CHANGED_CANDIDATE" '.candidate.commit = $commit' \
  "$EQUIVALENT_EMULATED" >"$CHANGED_EMULATED"

bash "$CLASSIFIER" \
  --repository-root "$FIXTURE_REPOSITORY" \
  --source-commit "$FIXTURE_SOURCE" \
  --candidate-commit "$CHANGED_CANDIDATE" \
  --emulated-report "$CHANGED_EMULATED" \
  --output "$FIXTURE_ROOT/output/changed.json" >/dev/null
jq -e '
  .inheritanceCandidate == false
  and .releaseQualificationEligible == false
  and .evidenceMode == "automated_release_qualification"
  and .reasonCode == "automated_native_termux_evidence_required"
  and .changedInputClasses == ["runtime_or_deployment", "cargo_or_dependency"]
  and .nextGate == "assemble_automated_release_qualification"
' "$FIXTURE_ROOT/output/changed.json" >/dev/null

grep -Fq 'runs-on: ubuntu-24.04-arm' "$ANDROID_WORKFLOW" || fail_test 'native ARM64 runner missing'
grep -Fq 'termux/termux-docker:aarch64@sha256:926e5c08aebc6df89f1cb3d9558c3b56b6246e59305fcd707bdf68f2584493b3' "$ANDROID_WORKFLOW" || fail_test 'pinned official Termux image missing'
grep -Fq 'uses: actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c' "$ANDROID_WORKFLOW" || fail_test 'download action is not pinned'
grep -Fq 'posture: android-battery-status' "$ANDROID_WORKFLOW" || fail_test 'battery feature build posture missing'
grep -Fq 'posture: android-volume-status' "$ANDROID_WORKFLOW" || fail_test 'volume feature build posture missing'
grep -Fq 'posture: android-volume-control' "$ANDROID_WORKFLOW" || fail_test 'volume control feature build posture missing'
grep -Fq 'posture: command-execution' "$ANDROID_WORKFLOW" || fail_test 'command feature build posture missing'
grep -Fq 'posture: full-suite' "$ANDROID_WORKFLOW" || fail_test 'full-suite build posture missing'
grep -Fq 'termux-mcp-server-aarch64-linux-android-full-suite' "$ANDROID_WORKFLOW" || fail_test 'full-suite artifact name missing'
grep -Fq 'termux_battery_emulated_gate.sh' "$ANDROID_WORKFLOW" || fail_test 'battery native emulation gate missing'
grep -Fq 'termux_volume_emulated_gate.sh' "$ANDROID_WORKFLOW" || fail_test 'volume native emulation gate missing'
grep -Fq 'termux_volume_control_emulated_gate.sh' "$ANDROID_WORKFLOW" || fail_test 'volume control native emulation gate missing'
grep -Fq -- '--volume-control-dir /workspace/artifacts/android-volume-control' "$ANDROID_WORKFLOW" || fail_test 'canonical runtime validator is missing the volume control artifact'
grep -Fq -- '--full-suite-dir /workspace/artifacts/full-suite' "$ANDROID_WORKFLOW" || fail_test 'aggregate native gate is missing the full-suite artifact'
grep -Fq 'termux_command_emulated_gate.sh' "$ANDROID_WORKFLOW" || fail_test 'command native emulation gate missing'
for publisher in \
  "$GATE" \
  "$BATTERY_GATE" \
  "$VOLUME_GATE" \
  "$VOLUME_CONTROL_GATE" \
  "$COMMAND_GATE"
do
  grep -Fq 'PUBLISH_NEXT="$(mktemp "$OUTPUT_PARENT/' "$publisher" \
    || fail_test "evidence publisher lacks a private output-parent temporary file: $publisher"
  grep -Fq 'python3 "$COMMIT_HELPER"' "$publisher" \
    || fail_test "evidence publisher does not use the held-FD commit helper: $publisher"
  if grep -Fq 'install -m 600 "$REPORT_NEXT" "$OUTPUT_REPORT"' "$publisher"; then
    fail_test "evidence publisher can still clobber its destination: $publisher"
  fi
  assert_publication_cleanup_contract "$publisher"
done
grep -Fq 'python3 "$COMMIT_HELPER"' "$CLASSIFIER" \
  || fail_test 'classifier does not use the held-FD commit helper'
assert_publication_cleanup_contract "$CLASSIFIER"
[[ -f "$COMMIT_HELPER" && ! -L "$COMMIT_HELPER" ]] \
  || fail_test 'held-FD commit helper is missing or linked'
if grep -En 'PUBLISH_(LINKED|IDENTITY)' \
  "$GATE" "$BATTERY_GATE" "$VOLUME_GATE" "$VOLUME_CONTROL_GATE" \
  "$COMMAND_GATE" "$CLASSIFIER" >/dev/null
then
  fail_test 'unsafe public-output rollback state remains in an evidence publisher'
fi
for specialized_gate in \
  "$BATTERY_GATE" \
  "$VOLUME_GATE" \
  "$VOLUME_CONTROL_GATE" \
  "$COMMAND_GATE"
do
  case "$(basename "$specialized_gate")" in
    termux_battery_emulated_gate.sh|termux_command_emulated_gate.sh)
      expected_gate_version=3
      ;;
    termux_volume_emulated_gate.sh|termux_volume_control_emulated_gate.sh)
      expected_gate_version=2
      ;;
    *)
      fail_test "unknown specialized gate: $specialized_gate"
      ;;
  esac
  grep -Fxq "GATE_VERSION=$expected_gate_version" "$specialized_gate" \
    || fail_test "specialized gate version is not the immutable schema pair: $specialized_gate"
  grep -Fq 'TERMUX_MCP_TERMUX_ROOTFS_IMAGE_ID' "$specialized_gate" \
    || fail_test "specialized gate omits the base image config identity: $specialized_gate"
  grep -Fq 'TERMUX_MCP_TERMUX_RUNTIME_IMAGE_DIGEST' "$specialized_gate" \
    || fail_test "specialized gate omits the derived runtime image identity: $specialized_gate"
  grep -Fq 'runtime_image_digest_not_derived' "$specialized_gate" \
    || fail_test "specialized gate does not compare base/runtime image IDs: $specialized_gate"
  grep -Fq 'rootfsImageId:' "$specialized_gate" \
    || fail_test "specialized report omits rootfsImageId: $specialized_gate"
  grep -Fq 'runtimeImageDigest:' "$specialized_gate" \
    || fail_test "specialized report omits runtimeImageDigest: $specialized_gate"
done
grep -Fq 'REPORT_NEXT="$(mktemp "$OUTPUT_PARENT/.observation-requirement.XXXXXX")"' "$CLASSIFIER" \
  || fail_test 'classifier report temporary file remains predictable or outside its output parent'
grep -Fq 'python3 "$COMMIT_HELPER"' "$CLASSIFIER" \
  || fail_test 'classifier report publication is not held-FD atomic no-clobber'
for contract in \
  '.failureCode == null' \
  '.candidate.version == $version' \
  '.candidate.ciRunId == $ci' \
  '.candidate.securityRunId == $security' \
  '.candidate.artifact.bytes >= 1' \
  '.candidate.artifact.bytes <= 67108864' \
  '.candidate.defaultArtifact.bytes >= 1' \
  '.candidate.defaultArtifact.bytes <= 67108864' \
  '.candidate.androidRunId == $android' \
  '.environment.architecture == "aarch64"' \
  '.environment.executionMode == "official-termux-docker-native-arm64"' \
  '.environment.image == "termux/termux-docker:aarch64"' \
  '.environment.imageDigest == $digest' \
  '.environment.rootfsImageId == $rootfs_image_id' \
  '.environment.runtimeImageDigest == $runtime_digest' \
  '.environment.androidLinker == true' \
  '.validation.requests == 29' \
  '.validation.exactArtifact == true' \
  '.validation.compileGate == true' \
  '.validation.wrongExecutableNameFailsClosed == true' \
  '.validation.wrongExecutableNameRejectedBeforeServing == true' \
  '.validation.runningInodePinned == true' \
  '.validation.workingDirectoryDescriptorPinned == true'; do
  grep -Fq "$contract" "$ANDROID_WORKFLOW" || fail_test "command evidence workflow omits: $contract"
done
grep -Fq 'docs/android-volume-emulated-evidence-schema-v*.json' "$CI_WORKFLOW" || fail_test 'volume evidence schema does not trigger CI'
grep -Fq 'docs/android-volume-control-emulated-evidence-schema-v*.json' "$CI_WORKFLOW" || fail_test 'volume control evidence schema does not trigger CI'
grep -Fq 'docs/command-emulated-evidence-schema-v*.json' "$CI_WORKFLOW" || fail_test 'command evidence schema does not trigger CI'
[[ "$(grep -Fc -- '- "docs/release-automated-qualification-schema-v*.json"' "$CI_WORKFLOW")" == 1 ]] || fail_test 'automated qualification schema is absent from the CI pull-request filter'
[[ "$(grep -Fc -- '- "docs/release-qualification-policy*.json"' "$CI_WORKFLOW")" == 1 ]] || fail_test 'qualification policy is absent from the CI pull-request filter'
[[ "$(grep -Fc -- '- "docs/automated-native-deployment-*.json"' "$CI_WORKFLOW")" == 1 ]] || fail_test 'automated deployment contracts are absent from the CI pull-request filter'
grep -Fq 'bash tests/termux_automated_deployment_gate_test.sh' "$CI_WORKFLOW" || fail_test 'automated deployment suite is absent from CI'
grep -Fq 'bash tests/package_automated_qualification_test.sh' "$CI_WORKFLOW" || fail_test 'automated qualification suite is absent from CI'
[[ "$(grep -Fc -- '- ".github/workflows/*"' "$CI_WORKFLOW")" == 1 ]] || fail_test 'workflow changes are absent from the CI pull-request filter'
[[ "$(grep -Fc -- '- ".github/workflows/*"' "$SECURITY_WORKFLOW")" == 1 ]] || fail_test 'workflow changes are absent from the Security pull-request filter'
[[ "$(grep -Fc -- '- ".github/workflows/*"' "$ANDROID_WORKFLOW")" == 1 ]] || fail_test 'workflow changes are absent from the Android pull-request filter'
[[ "$(grep -Fc -- '- "build.rs"' "$CI_WORKFLOW")" == 1 ]] || fail_test 'build script changes are absent from the CI pull-request filter'
[[ "$(grep -Fc -- '- "build.rs"' "$SECURITY_WORKFLOW")" == 1 ]] || fail_test 'build script changes are absent from the Security pull-request filter'
[[ "$(grep -Fc -- '- "build.rs"' "$ANDROID_WORKFLOW")" == 1 ]] || fail_test 'build script changes are absent from the Android pull-request filter'
[[ "$(grep -Fc -- '- ".cargo/**"' "$CI_WORKFLOW")" == 1 ]] || fail_test 'Cargo configuration changes are absent from the CI pull-request filter'
[[ "$(grep -Fc -- '- ".cargo/**"' "$SECURITY_WORKFLOW")" == 1 ]] || fail_test 'Cargo configuration changes are absent from the Security pull-request filter'
[[ "$(grep -Fc -- '- ".cargo/**"' "$ANDROID_WORKFLOW")" == 1 ]] || fail_test 'Cargo configuration changes are absent from the Android pull-request filter'
[[ "$(grep -Fc -- '- "src/**"' "$SECURITY_WORKFLOW")" == 1 ]] || fail_test 'runtime source changes are absent from the Security pull-request filter'
[[ "$(grep -Fc -- '- "tests/**"' "$SECURITY_WORKFLOW")" == 1 ]] || fail_test 'test changes are absent from the Security pull-request filter'
[[ "$(grep -Fc -- '- "scripts/termux_release_validate.sh"' "$SECURITY_WORKFLOW")" == 1 ]] || fail_test 'release validator changes are absent from the Security pull-request filter'
[[ "$(grep -Fc -- '- "scripts/termux_device_smoke.sh"' "$SECURITY_WORKFLOW")" == 1 ]] || fail_test 'device smoke changes are absent from the Security pull-request filter'
[[ "$(grep -Fc -- '- "scripts/termux_deploy.sh"' "$SECURITY_WORKFLOW")" == 1 ]] || fail_test 'deployment changes are absent from the Security pull-request filter'
grep -Fq 'scripts/termux_volume_emulated_gate.sh' "$SECURITY_WORKFLOW" || fail_test 'volume native gate does not trigger Security'
grep -Fq 'docs/android-volume-emulated-evidence-schema-v*.json' "$SECURITY_WORKFLOW" || fail_test 'volume evidence schema does not trigger Security'
grep -Fq 'scripts/termux_volume_control_emulated_gate.sh' "$SECURITY_WORKFLOW" || fail_test 'volume control native gate does not trigger Security'
grep -Fq 'docs/android-volume-control-emulated-evidence-schema-v*.json' "$SECURITY_WORKFLOW" || fail_test 'volume control evidence schema does not trigger Security'
grep -Fq 'scripts/termux_command_emulated_gate.sh' "$SECURITY_WORKFLOW" || fail_test 'command native gate does not trigger Security'
grep -Fq 'docs/command-emulated-evidence-schema-v*.json' "$SECURITY_WORKFLOW" || fail_test 'command evidence schema does not trigger Security'
[[ "$(grep -Fc -- '- "scripts/termux_automated_deployment_gate.sh"' "$SECURITY_WORKFLOW")" == 1 ]] || fail_test 'automated deployment gate is absent from the Security pull-request filter'
[[ "$(grep -Fc -- '- "scripts/package_automated_qualification.sh"' "$SECURITY_WORKFLOW")" == 1 ]] || fail_test 'automated qualification packager is absent from the Security pull-request filter'
[[ "$(grep -Fc -- '- "docs/release-automated-qualification-schema-v*.json"' "$SECURITY_WORKFLOW")" == 1 ]] || fail_test 'automated qualification schema is absent from the Security pull-request filter'
[[ "$(grep -Fc -- '- "docs/release-qualification-policy*.json"' "$SECURITY_WORKFLOW")" == 1 ]] || fail_test 'qualification policy is absent from the Security pull-request filter'
[[ "$(grep -Fc -- '- "docs/automated-native-deployment-*.json"' "$SECURITY_WORKFLOW")" == 1 ]] || fail_test 'automated deployment contracts are absent from the Security pull-request filter'
assert_unconditional_main_push "$CI_WORKFLOW"
assert_unconditional_main_push "$SECURITY_WORKFLOW"
assert_unconditional_main_push "$ANDROID_WORKFLOW"
python3 - "$ANDROID_WORKFLOW" <<'PY'
import sys
import yaml

workflow = yaml.safe_load(open(sys.argv[1], encoding="utf-8"))
workflow_defaults = workflow.get("defaults")
if (
    isinstance(workflow_defaults, dict)
    and isinstance(workflow_defaults.get("run"), dict)
    and "shell" in workflow_defaults["run"]
):
    raise SystemExit("workflow may not override the runner shell")
for job_name in ("android-aarch64", "termux-emulated"):
    job = workflow["jobs"][job_name]
    if "continue-on-error" in job:
        raise SystemExit(f"{job_name} job may not ignore failure")
    job_defaults = job.get("defaults")
    if (
        isinstance(job_defaults, dict)
        and isinstance(job_defaults.get("run"), dict)
        and "shell" in job_defaults["run"]
    ):
        raise SystemExit(f"{job_name} job may not override the runner shell")
    for index, item in enumerate(job["steps"]):
        if "continue-on-error" in item:
            raise SystemExit(
                f"{job_name} step {item.get('name', index)} may not ignore failure"
            )
        if "if" in item:
            raise SystemExit(
                f"{job_name} step {item.get('name', index)} may not bypass success ordering"
            )
        if "shell" in item:
            raise SystemExit(
                f"{job_name} step {item.get('name', index)} may not override the runner shell"
            )
steps = workflow["jobs"]["termux-emulated"]["steps"]
names = [item.get("name") for item in steps]
validation_index = names.index("Validate emulated evidence")
if names[validation_index : validation_index + 3] != [
    "Validate emulated evidence",
    "Freeze native qualification components",
    "Upload native qualification components",
]:
    raise SystemExit("validate/freeze/upload qualification ordering changed")
upload = steps[validation_index + 2]
if upload.get("uses") != "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a":
    raise SystemExit("qualification component upload action pin changed")
expected_upload = {
    "name": "termux-mcp-native-qualification-components",
    "path": "${{ steps.components.outputs.root }}/",
    "if-no-files-found": "error",
    "include-hidden-files": False,
    "compression-level": 0,
    "overwrite": False,
    "retention-days": 30,
}
if upload.get("with") != expected_upload:
    raise SystemExit("qualification component upload options changed")
step = next(
    item for item in steps
    if item.get("name") == "Resolve exact companion workflow evidence"
)
script = step["run"]
for name, path in (
    ("CI", ".github/workflows/ci.yml"),
    ("Security", ".github/workflows/security.yml"),
):
    start = script.index(f'.name == "{name}"')
    end = script.index(")]", start)
    selection = script[start:end]
    if f'.path == "{path}"' not in selection:
        raise SystemExit(f"{name} latest-run selection is not workflow-specific")
    if ".run_attempt == 1" in selection:
        raise SystemExit(f"{name} first-attempt filtering occurs before latest selection")
for marker in (
    'include "latest_workflow_run"',
    "complete_workflow_run_page",
    "latest_workflow_run_or_null",
    '(.run_attempt | tostring)',
    'latest exact-head CI is not a first attempt',
    'latest exact-head Security is not a first attempt',
    'IFS=: read -r _ _ ci_run_id _ <<<"$ci_state"',
    'IFS=: read -r _ _ security_run_id _ <<<"$security_state"',
):
    if marker not in script:
        raise SystemExit(f"companion latest/rerun contract missing: {marker}")
PY
[[ -f "$LATEST_RUN_SELECTOR" && ! -L "$LATEST_RUN_SELECTOR" ]] \
  || fail_test 'shared latest-run selector is missing or linked'
[[ "$(grep -Fc -- '- "scripts/latest_workflow_run.jq"' "$ANDROID_WORKFLOW")" == 1 ]] \
  || fail_test 'latest-run selector changes are absent from the Android pull-request filter'
[[ "$(grep -Fc -- '- "scripts/latest_workflow_run.jq"' "$SECURITY_WORKFLOW")" == 1 ]] \
  || fail_test 'latest-run selector changes are absent from the Security pull-request filter'
grep -Fq 'classify_observation_requirement.sh' "$ANDROID_WORKFLOW" || fail_test 'observation requirement classifier missing'
grep -Fq '.evidenceMode == "automated_release_qualification"' "$ANDROID_WORKFLOW" || fail_test 'automated qualification classifier route missing'
grep -Fq '.reasonCode == "automated_native_termux_evidence_required"' "$ANDROID_WORKFLOW" || fail_test 'automated qualification classifier reason missing'
if grep -Fq 'bash scripts/package_automated_qualification.sh' "$ANDROID_WORKFLOW"; then
  fail_test 'Android workflow still assembles its own post-run qualification conclusion'
fi
grep -Fq 'name: termux-mcp-native-qualification-components' "$ANDROID_WORKFLOW" \
  || fail_test 'Android seven-component snapshot artifact missing'
grep -Fq 'test "$(find "$snapshot" -mindepth 1 -maxdepth 1 -type f | wc -l)" = 7' "$ANDROID_WORKFLOW" \
  || fail_test 'Android snapshot does not enforce the exact seven-file boundary'
grep -Fq "chmod 755 \"\$root/termux-mcp-server\"" "$ANDROID_WORKFLOW" || fail_test 'container-readable artifact binary mode missing'
grep -Fq "chmod 644 \"\$root/SHA256SUMS\" \"\$root/artifact-manifest.json\"" "$ANDROID_WORKFLOW" || fail_test 'container-readable artifact metadata mode missing'
grep -Fq 'export TERMUX_MCP_EMULATED_ENVIRONMENT=official-termux-docker-native-arm64' "$ANDROID_WORKFLOW" || fail_test 'Termux entrypoint-safe environment attestation missing'
grep -Fq "export TERMUX_MCP_TERMUX_IMAGE_DIGEST='\$TERMUX_IMAGE_DIGEST'" "$ANDROID_WORKFLOW" || fail_test 'Termux entrypoint-safe image digest missing'
grep -Fq "export TERMUX_MCP_TERMUX_ROOTFS_IMAGE_ID='\$rootfs_image_id'" "$ANDROID_WORKFLOW" || fail_test 'base rootfs config/image ID attestation missing'
grep -Fq "export TERMUX_MCP_TERMUX_RUNTIME_IMAGE_DIGEST='\$runtime_image_id'" "$ANDROID_WORKFLOW" || fail_test 'derived runtime image digest attestation missing'
grep -Fq 'test "$runtime_image_id" != "$rootfs_image_id"' "$ANDROID_WORKFLOW" || fail_test 'derived runtime image is not distinguished from the materialized base image'
[[ "$(grep -Fc -- '--arg rootfs_image_id "$TERMUX_ROOTFS_IMAGE_ID"' "$ANDROID_WORKFLOW")" == 6 ]] \
  || fail_test 'base rootfs image ID is not bound across aggregate, specialized, and deployment evidence'
[[ "$(grep -Fc -- '--arg runtime_digest "$TERMUX_RUNTIME_IMAGE_DIGEST"' "$ANDROID_WORKFLOW")" == 6 ]] \
  || fail_test 'runtime image digest is not bound across aggregate, specialized, and deployment evidence'
[[ "$(grep -Fc -- '.environment.runtimeImageDigest == $runtime_digest' "$ANDROID_WORKFLOW")" == 6 ]] \
  || fail_test 'aggregate, specialized, and deployment runtime image digest checks are incomplete'
grep -Fq 'battery_feature_not_compiled' "$GATE" || fail_test 'standard runtime feature-disabled battery contract missing'
grep -Fq 'volume_feature_not_compiled' "$GATE" || fail_test 'standard runtime feature-disabled volume contract missing'
grep -Fq 'volume_control_posture_verified' "$GATE" || fail_test 'canonical runtime validator does not verify volume control posture'
grep -Fq 'androidVolumeControlArtifact' "$GATE" || fail_test 'canonical evidence omits the volume control artifact'
grep -Fq 'fullSuiteArtifact' "$GATE" || fail_test 'canonical evidence omits the full-suite artifact'
grep -Fq 'full_suite_manifest_sha' "$GATE" || fail_test 'canonical evidence omits the full-suite manifest digest'
grep -Fq 'aggregate_default_tool_allowlist_invalid' "$GATE" || fail_test 'aggregate default-disabled 17-tool contract missing'
grep -Fq 'aggregate_enabled_tool_allowlist_invalid' "$GATE" || fail_test 'aggregate fully-enabled 21-tool contract missing'
grep -Fq 'aggregate_volume_preview_mutated' "$GATE" || fail_test 'aggregate volume preview non-mutation contract missing'
grep -Fq 'capability_grant_binding_mismatch' "$GATE" || fail_test 'aggregate volume grant isolation contract missing'
grep -Fq 'aggregate_command_override_status_invalid' "$GATE" || fail_test 'aggregate command denial contract missing'
grep -Fq 'aggregate_command_executable_replacement_ran' "$GATE" || fail_test 'aggregate command inode replacement contract missing'
grep -Fq 'aggregate_shutdown_not_bounded' "$GATE" || fail_test 'aggregate bounded cleanup contract missing'
grep -Fq 'terminate_server_pid_bounded()' "$GATE" || fail_test 'shared bounded server cleanup helper missing'
grep -Fq 'kill -TERM "$pid"' "$GATE" || fail_test 'bounded server cleanup omits TERM'
grep -Fq 'kill -KILL "$pid"' "$GATE" || fail_test 'bounded server cleanup omits KILL fallback'
grep -Fq 'for ((attempt = 0; attempt < 50; attempt++))' "$GATE" || fail_test 'bounded server cleanup omits TERM deadline'
grep -Fq 'for ((attempt = 0; attempt < 20; attempt++))' "$GATE" || fail_test 'bounded server cleanup omits KILL deadline'
[[ "$(grep -Fc 'if ! kill -0 "$pid"' "$GATE")" == 2 ]] || fail_test 'bounded server cleanup can wait before proving child exit'
grep -Fq 'terminate_server_pid_bounded "$SERVER_PID" || status=1' "$GATE" || fail_test 'failure trap does not use bounded server cleanup'
grep -Fq "trap '' INT TERM HUP" "$GATE" || fail_test 'failure cleanup can be interrupted before fixture removal'
if grep -Fq 'wait "$SERVER_PID"' "$GATE"; then
  fail_test 'native gate retains an unbounded direct server wait'
fi
cleanup_stop_line="$(grep -nF 'terminate_server_pid_bounded "$SERVER_PID" || status=1' "$GATE" | head -n1 | cut -d: -f1)"
battery_fixture_cleanup_line="$(grep -nF 'rm -f -- "$BATTERY_PROGRAM"' "$GATE" | head -n1 | cut -d: -f1)"
volume_fixture_cleanup_line="$(grep -nF 'rm -f -- "$VOLUME_PROGRAM"' "$GATE" | head -n1 | cut -d: -f1)"
[[ "$cleanup_stop_line" =~ ^[0-9]+$ && "$battery_fixture_cleanup_line" =~ ^[0-9]+$ && "$volume_fixture_cleanup_line" =~ ^[0-9]+$ ]] \
  || fail_test 'bounded process and provider fixture cleanup sequence missing'
((cleanup_stop_line < battery_fixture_cleanup_line && battery_fixture_cleanup_line < volume_fixture_cleanup_line)) \
  || fail_test 'provider fixtures are not removed after bounded process cleanup'

grep -Fq 'validate_single_optional_gate_posture()' "$GATE" || fail_test 'aggregate single-gate validator missing'
grep -Fq 'aggregate_execution_copy_digest_mismatch' "$GATE" || fail_test 'each aggregate launch is not rebound to the exact full-suite digest'
grep -Fq "command curl --disable --proto '=http' --noproxy '*' --connect-timeout 2 --max-time 10" "$GATE" \
  || fail_test 'aggregate selected-tool calls are not bounded by the hardened HTTP client'
grep -Fq 'env "${cleared_runtime_environment[@]}" "${posture_environment[@]}"' "$GATE" \
  || fail_test 'aggregate postures do not clear all optional runtime gates before selecting one'
grep -Fq 'for aggregate_posture in battery-only volume-status-only volume-control-only command-only; do' "$GATE" \
  || fail_test 'aggregate four-posture independence matrix missing'
for posture in battery-only volume-status-only volume-control-only command-only; do
  grep -Fq "$posture)" "$GATE" || fail_test "aggregate posture is not configured: $posture"
  grep -Fq "full-suite-$posture-success" "$GATE" || fail_test "aggregate posture omits bounded selected-tool success: $posture"
done
[[ "$(grep -Fc '$base_tools + [$selected_tool]' "$GATE")" == 2 ]] \
  || fail_test 'aggregate discovery and runtime status do not both assert exact 18-tool order'
grep -Fq '($status.availableTools | length) == 18' "$GATE" || fail_test 'aggregate single-gate runtime tool count is not exact'
grep -Fq '$status.androidBatteryStatusEnabled == $battery_enabled' "$GATE" || fail_test 'aggregate battery gate independence status missing'
grep -Fq '$status.androidVolumeStatusEnabled == $volume_status_enabled' "$GATE" || fail_test 'aggregate volume-status gate independence status missing'
grep -Fq '$status.androidVolumeControlEnabled == $volume_control_enabled' "$GATE" || fail_test 'aggregate volume-control gate independence status missing'
grep -Fq '$status.commandExecution == $command_enabled' "$GATE" || fail_test 'aggregate command gate independence status missing'
grep -Fq 'AGGREGATE_CLEANUPS" == 6' "$GATE" || fail_test 'aggregate six-server cleanup count missing'
grep -Fq 'independentRuntimeGates: true' "$GATE" || fail_test 'aggregate report omits independent runtime-gate evidence'
grep -Fq '.aggregateValidation.independentRuntimeGates == true' "$GATE" || fail_test 'generated aggregate report does not verify runtime-gate evidence'

for mutation_contract in \
  'name:"create_directory",arguments:{path:$path,dry_run:false}' \
  'name:"copy_file",arguments:{source_path:$source,destination_path:$destination,dry_run:false}' \
  'name:"trash_file",arguments:{path:$path,dry_run:false}' \
  'name:"write_file",arguments:{path:$path,content:"inert",dry_run:false}' \
  'create_directory_mutation_disabled' \
  'copy_file_mutation_disabled' \
  'trash_file_mutation_disabled' \
  'write_file_mutation_disabled' \
  'aggregate_create_directory_disabled_contract_invalid' \
  'aggregate_copy_file_disabled_contract_invalid' \
  'aggregate_trash_file_disabled_contract_invalid' \
  'aggregate_write_disabled_contract_invalid' \
  'aggregate_copy_file_disabled_source_state_mutated' \
  'aggregate_copy_file_disabled_destination_mutated' \
  'aggregate_trash_file_disabled_target_state_mutated' \
  'aggregate_trash_file_disabled_quarantine_mutated' \
  'aggregate_write_disabled_destination_mutated'; do
  grep -Fq "$mutation_contract" "$GATE" || fail_test "aggregate filesystem dispatch contract missing: $mutation_contract"
done
grep -Fq '.candidate.fullSuiteArtifactSha256' "$CLASSIFIER" || fail_test 'observation classification omits full-suite digest binding'
grep -Fq '.error.code == -32600' "$VOLUME_CONTROL_GATE" || fail_test 'volume control grant context does not assert the MCP invalid-request envelope'
grep -Fq 'A request-scoped capability grant is accepted only for an exact grant-authorized tool call.' "$VOLUME_CONTROL_GATE" || fail_test 'volume control grant context does not assert the stable transport detail'

chmod_line="$(grep -nF "chmod 700 \"\$output_root\"" "$ANDROID_WORKFLOW" | cut -d: -f1)"
chown_line="$(grep -nF "sudo chown 1000:1000 \"\$output_root\"" "$ANDROID_WORKFLOW" | cut -d: -f1)"
[[ "$chmod_line" =~ ^[0-9]+$ && "$chown_line" =~ ^[0-9]+$ ]] || fail_test 'private output ownership sequence missing'
((chmod_line < chown_line)) || fail_test 'output mode must be set before ownership transfers to the container user'

printf 'Native ARM64 Termux, battery/volume/volume-control/command postures, and observation evidence contract tests passed\n'
