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
SOURCE_REPORT="$ROOT/docs/release-evidence/v0.5.1-physical-fe5f7b80.json"
FIXTURE_ROOT="$(mktemp -d)"
trap 'rm -rf -- "$FIXTURE_ROOT"' EXIT INT TERM

fail_test() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
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
  .properties.schemaVersion.const == 3
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
  .properties.schemaVersion.const == 2
  and .properties.gateVersion.const == "2"
  and .properties.releaseQualificationEligible.const == false
  and .properties.environment."$ref" == "#/$defs/environment"
  and ."$defs".environment.properties.executionMode.const == "official-termux-docker-native-arm64"
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
' "$ROOT/docs/android-battery-emulated-evidence-schema-v2.json" >/dev/null

jq -e '
  .properties.schemaVersion.const == 1
  and .properties.gateVersion.const == "1"
  and .properties.releaseQualificationEligible.const == false
  and .properties.environment."$ref" == "#/$defs/environment"
  and ."$defs".environment.properties.executionMode.const == "official-termux-docker-native-arm64"
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
' "$ROOT/docs/android-volume-emulated-evidence-schema-v1.json" >/dev/null

jq -e '
  .properties.schemaVersion.const == 1
  and .properties.gateVersion.const == "1"
  and .properties.releaseQualificationEligible.const == false
  and ."$defs".candidate.required == ["commit","version","ciRunId","securityRunId","androidRunId","artifact","incompatibleArtifact"]
  and ."$defs".environment.properties.executionMode.const == "official-termux-docker-native-arm64"
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
' "$ROOT/docs/android-volume-control-emulated-evidence-schema-v1.json" >/dev/null

jq -e '
  .properties.schemaVersion.const == 2
  and .properties.gateVersion.const == "2"
  and .properties.releaseQualificationEligible.const == false
  and .properties.candidate."$ref" == "#/$defs/candidate"
  and ."$defs".candidate.required == ["commit","version","ciRunId","securityRunId","androidRunId","artifact","defaultArtifact"]
  and ."$defs".environment.properties.executionMode.const == "official-termux-docker-native-arm64"
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
' "$ROOT/docs/command-emulated-evidence-schema-v2.json" >/dev/null
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
    schemaVersion: 3,
    gateVersion: "3",
    status: "pass",
    failureCode: null,
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
    runtimeValidation: {status: "pass"},
    aggregateValidation: {
      status: "pass",
      defaultDisabled: {toolCount: 17},
      fullyEnabled: {toolCount: 21},
      directPhysicalObservationRequired: true
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
  and .schemaVersion == 2
  and .classifierVersion == "2"
  and .releaseQualificationEligible == false
  and .evidenceMode == "physical_observation_required"
  and .reasonCode == "full_suite_direct_physical_observation_required"
  and .changedInputClasses == ["full_suite_artifact"]
  and .nextGate == "direct_physical_device_observation"
  and .candidate.fullSuiteArtifactSha256 == "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  and .candidate.fullSuiteManifestSha256 == "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
' "$FIXTURE_ROOT/output/equivalent.json" >/dev/null
[[ "$(stat -c %a "$FIXTURE_ROOT/output/equivalent.json")" == 600 ]] || fail_test 'classifier output is not private'

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
  and .evidenceMode == "physical_observation_required"
  and .reasonCode == "full_suite_direct_physical_observation_required"
  and .changedInputClasses == ["runtime_or_deployment", "cargo_or_dependency", "full_suite_artifact"]
  and .nextGate == "direct_physical_device_observation"
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
[[ "$(grep -Fc -- '- ".github/workflows/*"' "$CI_WORKFLOW")" == 2 ]] || fail_test 'workflow changes do not trigger CI for both push and pull requests'
[[ "$(grep -Fc -- '- ".github/workflows/*"' "$SECURITY_WORKFLOW")" == 2 ]] || fail_test 'workflow changes do not trigger Security for both push and pull requests'
[[ "$(grep -Fc -- '- "src/**"' "$SECURITY_WORKFLOW")" == 2 ]] || fail_test 'runtime source changes do not trigger Security for both push and pull requests'
[[ "$(grep -Fc -- '- "tests/**"' "$SECURITY_WORKFLOW")" == 2 ]] || fail_test 'test changes do not trigger Security for both push and pull requests'
[[ "$(grep -Fc -- '- "scripts/termux_release_validate.sh"' "$SECURITY_WORKFLOW")" == 2 ]] || fail_test 'release validator changes do not trigger Security for both push and pull requests'
[[ "$(grep -Fc -- '- "scripts/termux_device_smoke.sh"' "$SECURITY_WORKFLOW")" == 2 ]] || fail_test 'device smoke changes do not trigger Security for both push and pull requests'
[[ "$(grep -Fc -- '- "scripts/termux_deploy.sh"' "$SECURITY_WORKFLOW")" == 2 ]] || fail_test 'deployment changes do not trigger Security for both push and pull requests'
grep -Fq 'scripts/termux_volume_emulated_gate.sh' "$SECURITY_WORKFLOW" || fail_test 'volume native gate does not trigger Security'
grep -Fq 'docs/android-volume-emulated-evidence-schema-v*.json' "$SECURITY_WORKFLOW" || fail_test 'volume evidence schema does not trigger Security'
grep -Fq 'scripts/termux_volume_control_emulated_gate.sh' "$SECURITY_WORKFLOW" || fail_test 'volume control native gate does not trigger Security'
grep -Fq 'docs/android-volume-control-emulated-evidence-schema-v*.json' "$SECURITY_WORKFLOW" || fail_test 'volume control evidence schema does not trigger Security'
grep -Fq 'scripts/termux_command_emulated_gate.sh' "$SECURITY_WORKFLOW" || fail_test 'command native gate does not trigger Security'
grep -Fq 'docs/command-emulated-evidence-schema-v*.json' "$SECURITY_WORKFLOW" || fail_test 'command evidence schema does not trigger Security'
grep -Fq 'classify_observation_requirement.sh' "$ANDROID_WORKFLOW" || fail_test 'observation requirement classifier missing'
grep -Fq "if jq -e '.inheritanceCandidate == true'" "$ANDROID_WORKFLOW" || fail_test 'inheritance verifier is not conditionally gated'
grep -Fq '.evidenceMode == "physical_observation_required"' "$ANDROID_WORKFLOW" || fail_test 'runtime-change observation evidence path missing'
grep -Fq '.reasonCode == "full_suite_direct_physical_observation_required"' "$ANDROID_WORKFLOW" || fail_test 'full-suite direct-observation requirement missing'
grep -Fq "chmod 755 \"\$root/termux-mcp-server\"" "$ANDROID_WORKFLOW" || fail_test 'container-readable artifact binary mode missing'
grep -Fq "chmod 644 \"\$root/SHA256SUMS\" \"\$root/artifact-manifest.json\"" "$ANDROID_WORKFLOW" || fail_test 'container-readable artifact metadata mode missing'
grep -Fq 'export TERMUX_MCP_EMULATED_ENVIRONMENT=official-termux-docker-native-arm64' "$ANDROID_WORKFLOW" || fail_test 'Termux entrypoint-safe environment attestation missing'
grep -Fq "export TERMUX_MCP_TERMUX_IMAGE_DIGEST='\$TERMUX_IMAGE_DIGEST'" "$ANDROID_WORKFLOW" || fail_test 'Termux entrypoint-safe image digest missing'
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
