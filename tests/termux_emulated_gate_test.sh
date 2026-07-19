#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE="$ROOT/scripts/termux_emulated_gate.sh"
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
  .properties.status.const == "pass"
  and .properties.environment.properties.executionMode.const == "official-termux-docker-native-arm64"
  and .properties.environment.properties.androidLinker.const == true
  and .properties.candidate.properties.androidVolumeControlArtifact."$ref" == "#/$defs/artifact"
  and .properties.stress.properties.samples.minimum == 32
  and .properties.stress.properties.highImpactDisabled.const == true
' "$ROOT/docs/emulated-release-evidence-schema-v1.json" >/dev/null

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
  .properties.schemaVersion.const == 1
  and .properties.gateVersion.const == "1"
  and .properties.releaseQualificationEligible.const == false
  and .properties.candidate."$ref" == "#/$defs/candidate"
  and ."$defs".candidate.required == ["commit","version","ciRunId","securityRunId","androidRunId","artifact","defaultArtifact"]
  and ."$defs".environment.properties.executionMode.const == "official-termux-docker-native-arm64"
  and ."$defs".validation.properties.compileGate.const == true
  and ."$defs".validation.properties.runtimeDefaultDisabled.const == true
  and ."$defs".validation.properties.fixedCurrentExecutable.const == true
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
' "$ROOT/docs/command-emulated-evidence-schema-v1.json" >/dev/null

jq -e '
  .properties.releaseQualificationEligible.const == false
  and (.properties.evidenceMode.enum | index("physical_observation_required") != null)
  and (.properties.reasonCode.enum | index("runtime_and_build_inputs_changed") != null)
  and ."$defs".emulation.properties.executionMode.const == "official-termux-docker-native-arm64"
  and .allOf[0].then.properties.changedInputClasses.maxItems == 0
  and .allOf[0].else.properties.changedInputClasses.minItems == 1
' "$ROOT/docs/release-observation-requirement-schema-v1.json" >/dev/null

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
    schemaVersion: 1,
    gateVersion: "1",
    status: "pass",
    failureCode: null,
    candidate: {
      commit: $commit,
      version: "0.6.0",
      ciRunId: "1001",
      securityRunId: "1002",
      androidRunId: "1003"
    },
    environment: {
      executionMode: "official-termux-docker-native-arm64",
      androidLinker: true,
      imageDigest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    },
    runtimeValidation: {status: "pass"},
    stress: {status: "pass", samples: 32}
  }' >"$EQUIVALENT_EMULATED"

bash "$CLASSIFIER" \
  --repository-root "$FIXTURE_REPOSITORY" \
  --source-commit "$FIXTURE_SOURCE" \
  --candidate-commit "$EQUIVALENT_CANDIDATE" \
  --emulated-report "$EQUIVALENT_EMULATED" \
  --output "$FIXTURE_ROOT/output/equivalent.json" >/dev/null
jq -e '
  .inheritanceCandidate == true
  and .releaseQualificationEligible == false
  and .evidenceMode == "observation_inheritance_candidate"
  and .changedInputClasses == []
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
  and .reasonCode == "runtime_and_build_inputs_changed"
  and .changedInputClasses == ["runtime_or_deployment", "cargo_or_dependency"]
  and .nextGate == "direct_physical_device_observation"
' "$FIXTURE_ROOT/output/changed.json" >/dev/null

grep -Fq 'runs-on: ubuntu-24.04-arm' "$ANDROID_WORKFLOW" || fail_test 'native ARM64 runner missing'
grep -Fq 'termux/termux-docker:aarch64@sha256:926e5c08aebc6df89f1cb3d9558c3b56b6246e59305fcd707bdf68f2584493b3' "$ANDROID_WORKFLOW" || fail_test 'pinned official Termux image missing'
grep -Fq 'uses: actions/download-artifact@70fc10c6e5e1ce46ad2ea6f2b72d43f7d47b13c3' "$ANDROID_WORKFLOW" || fail_test 'download action is not pinned'
grep -Fq 'posture: android-battery-status' "$ANDROID_WORKFLOW" || fail_test 'battery feature build posture missing'
grep -Fq 'posture: android-volume-status' "$ANDROID_WORKFLOW" || fail_test 'volume feature build posture missing'
grep -Fq 'posture: android-volume-control' "$ANDROID_WORKFLOW" || fail_test 'volume control feature build posture missing'
grep -Fq 'posture: command-execution' "$ANDROID_WORKFLOW" || fail_test 'command feature build posture missing'
grep -Fq 'termux_battery_emulated_gate.sh' "$ANDROID_WORKFLOW" || fail_test 'battery native emulation gate missing'
grep -Fq 'termux_volume_emulated_gate.sh' "$ANDROID_WORKFLOW" || fail_test 'volume native emulation gate missing'
grep -Fq 'termux_volume_control_emulated_gate.sh' "$ANDROID_WORKFLOW" || fail_test 'volume control native emulation gate missing'
grep -Fq -- '--volume-control-dir /workspace/artifacts/android-volume-control' "$ANDROID_WORKFLOW" || fail_test 'canonical runtime validator is missing the volume control artifact'
grep -Fq 'termux_command_emulated_gate.sh' "$ANDROID_WORKFLOW" || fail_test 'command native emulation gate missing'
grep -Fq 'docs/android-volume-emulated-evidence-schema-v*.json' "$CI_WORKFLOW" || fail_test 'volume evidence schema does not trigger CI'
grep -Fq 'docs/android-volume-control-emulated-evidence-schema-v*.json' "$CI_WORKFLOW" || fail_test 'volume control evidence schema does not trigger CI'
grep -Fq 'docs/command-emulated-evidence-schema-v*.json' "$CI_WORKFLOW" || fail_test 'command evidence schema does not trigger CI'
[[ "$(grep -Fc -- '- ".github/workflows/*.yml"' "$CI_WORKFLOW")" == 2 ]] || fail_test 'workflow changes do not trigger CI for both push and pull requests'
[[ "$(grep -Fc -- '- ".github/workflows/*.yml"' "$SECURITY_WORKFLOW")" == 2 ]] || fail_test 'workflow changes do not trigger Security for both push and pull requests'
grep -Fq 'scripts/termux_volume_emulated_gate.sh' "$SECURITY_WORKFLOW" || fail_test 'volume native gate does not trigger Security'
grep -Fq 'docs/android-volume-emulated-evidence-schema-v*.json' "$SECURITY_WORKFLOW" || fail_test 'volume evidence schema does not trigger Security'
grep -Fq 'scripts/termux_volume_control_emulated_gate.sh' "$SECURITY_WORKFLOW" || fail_test 'volume control native gate does not trigger Security'
grep -Fq 'docs/android-volume-control-emulated-evidence-schema-v*.json' "$SECURITY_WORKFLOW" || fail_test 'volume control evidence schema does not trigger Security'
grep -Fq 'scripts/termux_command_emulated_gate.sh' "$SECURITY_WORKFLOW" || fail_test 'command native gate does not trigger Security'
grep -Fq 'docs/command-emulated-evidence-schema-v*.json' "$SECURITY_WORKFLOW" || fail_test 'command evidence schema does not trigger Security'
grep -Fq 'classify_observation_requirement.sh' "$ANDROID_WORKFLOW" || fail_test 'observation requirement classifier missing'
grep -Fq "if jq -e '.inheritanceCandidate == true'" "$ANDROID_WORKFLOW" || fail_test 'inheritance verifier is not conditionally gated'
grep -Fq '.evidenceMode == "physical_observation_required"' "$ANDROID_WORKFLOW" || fail_test 'runtime-change observation evidence path missing'
grep -Fq "chmod 755 \"\$root/termux-mcp-server\"" "$ANDROID_WORKFLOW" || fail_test 'container-readable artifact binary mode missing'
grep -Fq "chmod 644 \"\$root/SHA256SUMS\" \"\$root/artifact-manifest.json\"" "$ANDROID_WORKFLOW" || fail_test 'container-readable artifact metadata mode missing'
grep -Fq 'export TERMUX_MCP_EMULATED_ENVIRONMENT=official-termux-docker-native-arm64' "$ANDROID_WORKFLOW" || fail_test 'Termux entrypoint-safe environment attestation missing'
grep -Fq "export TERMUX_MCP_TERMUX_IMAGE_DIGEST='\$TERMUX_IMAGE_DIGEST'" "$ANDROID_WORKFLOW" || fail_test 'Termux entrypoint-safe image digest missing'
grep -Fq 'battery_feature_not_compiled' "$GATE" || fail_test 'standard runtime feature-disabled battery contract missing'
grep -Fq 'volume_feature_not_compiled' "$GATE" || fail_test 'standard runtime feature-disabled volume contract missing'
grep -Fq 'volume_control_posture_verified' "$GATE" || fail_test 'canonical runtime validator does not verify volume control posture'
grep -Fq 'androidVolumeControlArtifact' "$GATE" || fail_test 'canonical evidence omits the volume control artifact'
grep -Fq '.error.code == -32600' "$VOLUME_CONTROL_GATE" || fail_test 'volume control grant context does not assert the MCP invalid-request envelope'
grep -Fq 'A request-scoped capability grant is accepted only for an exact grant-authorized tool call.' "$VOLUME_CONTROL_GATE" || fail_test 'volume control grant context does not assert the stable transport detail'

chmod_line="$(grep -nF "chmod 700 \"\$output_root\"" "$ANDROID_WORKFLOW" | cut -d: -f1)"
chown_line="$(grep -nF "sudo chown 1000:1000 \"\$output_root\"" "$ANDROID_WORKFLOW" | cut -d: -f1)"
[[ "$chmod_line" =~ ^[0-9]+$ && "$chown_line" =~ ^[0-9]+$ ]] || fail_test 'private output ownership sequence missing'
((chmod_line < chown_line)) || fail_test 'output mode must be set before ownership transfers to the container user'

printf 'Native ARM64 Termux, battery/volume/volume-control/command postures, and observation evidence contract tests passed\n'
