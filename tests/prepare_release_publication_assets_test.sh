#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C

ROOT="$(mktemp -d)"
trap 'rm -rf -- "$ROOT"' EXIT INT TERM
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
SCRIPT="$REPO_ROOT/scripts/prepare_release_publication_assets.sh"
REAL_PATH="$PATH"
REAL_CP="$(command -v cp)"
REAL_MV="$(command -v mv)"
REPOSITORY="CyberBASSLord-666/termux-mcp-edge"
COMMIT="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
VERSION="0.6.0"
CI_RUN_ID="5101"
SECURITY_RUN_ID="5102"
ANDROID_RUN_ID="5103"
STAGE_NAME="termux-mcp-server-v${VERSION}-release-stage-${COMMIT:0:12}.tar"
RECEIPT_NAME="release-publication-receipt-v1.json"

postures=(
  default
  mcp-runtime
  android-battery-status
  android-volume-status
  android-volume-control
  command-execution
  full-suite
)
features_json=(
  '[]'
  '["mcp-runtime"]'
  '["android-battery-status"]'
  '["android-volume-status"]'
  '["android-volume-control"]'
  '["command-execution"]'
  '["full-suite"]'
)
workflow_artifacts=(
  termux-mcp-server-aarch64-linux-android-default
  termux-mcp-server-aarch64-linux-android-mcp-runtime
  termux-mcp-server-aarch64-linux-android-android-battery-status
  termux-mcp-server-aarch64-linux-android-android-volume-status
  termux-mcp-server-aarch64-linux-android-android-volume-control
  termux-mcp-server-aarch64-linux-android-command-execution
  termux-mcp-server-aarch64-linux-android-full-suite
)

fail_test() {
  printf 'FAIL: %s\n' "$1" >&2
  exit 1
}

assert_fails() {
  local expected_code="$1" stage_tar="$2" output_parent="$3" digest="${4:-}"
  mkdir -m 700 -- "$output_parent"
  if [[ -z "$digest" ]]; then
    digest="$(sha256sum -- "$stage_tar" | awk '{print $1}')"
  fi
  if PREP_TEST_PATH="${PREP_TEST_PATH:-}" run_prepare "$stage_tar" "$output_parent" "$digest" \
    >"$ROOT/last.stdout" 2>"$ROOT/last.stderr"; then
    fail_test "command unexpectedly succeeded; expected $expected_code"
  fi
  grep -Fq -- "$expected_code" "$ROOT/last.stderr" \
    || { sed -n '1,120p' "$ROOT/last.stderr" >&2; fail_test "expected error code $expected_code was absent"; }
}

mkdir -m 700 -- "$ROOT/fake-bin"
cat >"$ROOT/fake-bin/file" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
target="${*: -1}"
if grep -Fq 'wrong-arch' "$target"; then
  printf '%s\n' 'ELF 64-bit LSB executable, x86-64, for GNU/Linux'
else
  printf '%s\n' 'ELF 64-bit LSB pie executable, ARM aarch64, interpreter /system/bin/linker64, for Android 24'
fi
EOF
chmod 700 "$ROOT/fake-bin/file"

BASE="$ROOT/base"
PAYLOAD="$BASE/payload"
mkdir -m 700 -p -- "$PAYLOAD/evidence" "$BASE/work" "$BASE/stage"
ARTIFACT_RECORDS="$BASE/work/artifacts.jsonl"
: >"$ARTIFACT_RECORDS"
: >"$PAYLOAD/SHA256SUMS"

binary_sha=()
binary_bytes=()
manifest_sha=()
release_names=()

for index in "${!postures[@]}"; do
  posture="${postures[$index]}"
  release_name="termux-mcp-server-v${VERSION}-aarch64-linux-android-${posture}"
  checksum_name="$release_name.sha256"
  workflow_manifest_name="$release_name.workflow-manifest.json"
  release_names+=("$release_name")
  printf '%s\n' '#!/system/bin/sh' "# fixture-aarch64-android-$posture" 'exit 0' \
    >"$PAYLOAD/$release_name"
  chmod 755 "$PAYLOAD/$release_name"
  digest="$(sha256sum "$PAYLOAD/$release_name" | awk '{print $1}')"
  bytes="$(stat -c '%s' "$PAYLOAD/$release_name")"
  binary_sha+=("$digest")
  binary_bytes+=("$bytes")
  printf '%s  %s\n' "$digest" "$release_name" >"$PAYLOAD/$checksum_name"
  printf '%s  %s\n' "$digest" "$release_name" >>"$PAYLOAD/SHA256SUMS"
  jq -S -n \
    --arg repository "$REPOSITORY" --arg commit "$COMMIT" \
    --arg run "$ANDROID_RUN_ID" --arg artifact "${workflow_artifacts[$index]}" \
    --arg posture "$posture" --argjson features "${features_json[$index]}" \
    --arg version "$VERSION" --arg digest "$digest" --argjson bytes "$bytes" '
      {
        schemaVersion:1,
        repository:$repository,
        commit:$commit,
        workflowRunId:$run,
        artifactName:$artifact,
        posture:$posture,
        features:$features,
        target:"aarch64-linux-android",
        fileName:"termux-mcp-server",
        version:$version,
        sha256:$digest,
        bytes:$bytes,
        elf:"aarch64-android-elf",
        createdAt:"2026-07-22T00:00:00Z"
      }
    ' >"$PAYLOAD/$workflow_manifest_name"
  current_manifest_sha="$(sha256sum "$PAYLOAD/$workflow_manifest_name" | awk '{print $1}')"
  manifest_sha+=("$current_manifest_sha")
  jq -c -n \
    --arg posture "$posture" --argjson features "${features_json[$index]}" \
    --arg workflow_artifact "${workflow_artifacts[$index]}" \
    --arg workflow_manifest "$workflow_manifest_name" \
    --arg workflow_manifest_sha "$current_manifest_sha" \
    --arg release_name "$release_name" --arg checksum_name "$checksum_name" \
    --arg digest "$digest" --argjson bytes "$bytes" '
      {
        posture:$posture,
        features:$features,
        workflowArtifactName:$workflow_artifact,
        workflowFileName:"termux-mcp-server",
        workflowManifestFileName:$workflow_manifest,
        workflowManifestSha256:$workflow_manifest_sha,
        releaseFileName:$release_name,
        checksumFileName:$checksum_name,
        sha256:$digest,
        bytes:$bytes,
        elf:"aarch64-android-elf"
      }
    ' >>"$ARTIFACT_RECORDS"
done

printf '%s\n' 'MIT License' 'publication fixture' >"$PAYLOAD/LICENSE"

jq -S -n \
  --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg default_sha "${binary_sha[0]}" --argjson default_bytes "${binary_bytes[0]}" \
  --arg mcp_sha "${binary_sha[1]}" --argjson mcp_bytes "${binary_bytes[1]}" \
  --arg volume_sha "${binary_sha[4]}" --argjson volume_bytes "${binary_bytes[4]}" \
  --arg full_sha "${binary_sha[6]}" --argjson full_bytes "${binary_bytes[6]}" \
  --arg full_manifest_sha "${manifest_sha[6]}" '
    {
      schemaVersion:3,
      gateVersion:"3",
      status:"pass",
      failureCode:null,
      releaseQualificationEligible:false,
      startedAt:"2026-07-22T00:00:00Z",
      completedAt:"2026-07-22T00:02:00Z",
      candidate:{
        commit:$commit,version:$version,ciRunId:$ci,securityRunId:$security,androidRunId:$android,
        defaultArtifact:{sha256:$default_sha,bytes:$default_bytes},
        mcpRuntimeArtifact:{sha256:$mcp_sha,bytes:$mcp_bytes},
        androidVolumeControlArtifact:{sha256:$volume_sha,bytes:$volume_bytes},
        fullSuiteArtifact:{
          sha256:$full_sha,bytes:$full_bytes,manifestSha256:$full_manifest_sha,
          artifactName:"termux-mcp-server-aarch64-linux-android-full-suite",
          posture:"full-suite",features:["full-suite"],fileName:"termux-mcp-server"
        }
      },
      environment:{
        executionMode:"official-termux-docker-native-arm64",architecture:"aarch64",
        image:"termux/termux-docker:aarch64",imageDigest:("sha256:" + ("c" * 64)),androidLinker:true
      },
      runtimeValidation:{
        status:"pass",reportSha256:("d" * 64),resultCount:20,
        phases:{preflight:"pass",runtime:"pass",deployment:"not_run"}
      },
      aggregateValidation:{
        status:"pass",requests:20,
        defaultDisabled:{
          toolCount:17,exactToolOrder:true,optionalFeaturesCompiled:true,
          optionalToolsHidden:true,runtimeFlagsOmitted:true
        },
        fullyEnabled:{
          toolCount:21,exactToolOrder:true,allOptionalToolsExposed:true,providerSuccesses:true,
          volumePreviewNoMutation:true,volumeGrantIsolation:true,commandExecutableIdentityPinned:true
        },
        independentRuntimeGates:true,filesystemMutationsDisabled:true,boundedCleanup:true,
        directPhysicalObservationRequired:true
      },
      stress:{
        status:"pass",samples:64,requests:128,servicePidStable:true,healthReadyStable:true,
        sessionLifecycle:true,exactToolAllowlist:true,safeRootIdentityPinned:true,
        safeRootAncestorIdentityPinned:true,copyFileMutationDisabled:true,highImpactDisabled:true,
        longObservationRequired:false
      }
    }
  ' >"$PAYLOAD/evidence/emulated-release-v3.json"

make_specialized_evidence() {
  local output="$1" schema_version="$2" gate_version="$3" artifact_index="$4" mode="$5"
  local related_index="${6:--1}" related_sha="" related_bytes=0
  if ((related_index >= 0)); then
    related_sha="${binary_sha[$related_index]}"
    related_bytes="${binary_bytes[$related_index]}"
  fi
  jq -S -n \
    --argjson schema "$schema_version" --arg gate "$gate_version" --arg mode "$mode" \
    --arg commit "$COMMIT" --arg version "$VERSION" \
    --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
    --arg artifact_sha "${binary_sha[$artifact_index]}" \
    --argjson artifact_bytes "${binary_bytes[$artifact_index]}" \
    --arg related_sha "$related_sha" --argjson related_bytes "$related_bytes" '
      {
        schemaVersion:$schema,gateVersion:$gate,status:"pass",failureCode:null,
        releaseQualificationEligible:false,startedAt:"2026-07-22T00:00:00Z",
        completedAt:"2026-07-22T00:01:00Z",
        candidate:(
          {
            commit:$commit,version:$version,ciRunId:$ci,securityRunId:$security,androidRunId:$android,
            artifact:{sha256:$artifact_sha,bytes:$artifact_bytes}
          }
          + (if $mode == "volume-control" then
               {incompatibleArtifact:{sha256:$related_sha,bytes:$related_bytes}}
             elif $mode == "command" then
               {defaultArtifact:{sha256:$related_sha,bytes:$related_bytes}}
             else {} end)
        ),
        environment:{
          architecture:"aarch64",executionMode:"official-termux-docker-native-arm64",
          image:"termux/termux-docker:aarch64",imageDigest:("sha256:" + ("b" * 64)),androidLinker:true
        },
        validation:(
          if $mode == "battery" then {
            status:"pass",requests:29,exactArtifact:true,compileGate:true,runtimeDefaultDisabled:true,
            disabledDiscovery:true,fixedProgram:true,fixedWorkingDirectory:true,noArguments:true,
            inheritedEnvironmentCleared:true,normalizedAllowlist:true,sensitiveFieldsRedacted:true,
            boundedOutput:true,immediateOverflowTermination:true,processGroupIsolation:true,
            pipeHoldingDescendantCleanup:true,callerCancellationCleanup:true,boundedSupervisorCleanup:true,
            stableErrors:true,androidDeviceControlDisabled:true,commandExecutionDisabled:true,
            highImpactToolsDisabled:true
          } elif $mode == "volume" then {
            status:"pass",requests:29,exactArtifact:true,compileGate:true,runtimeDefaultDisabled:true,
            disabledDiscovery:true,fixedProgram:true,fixedWorkingDirectory:true,noArguments:true,
            inheritedEnvironmentCleared:true,normalizedAllowlist:true,canonicalStreamOrdering:true,
            unrecognizedFieldsRejected:true,boundedOutput:true,immediateOverflowTermination:true,
            processGroupIsolation:true,pipeHoldingDescendantCleanup:true,callerCancellationCleanup:true,
            boundedSupervisorCleanup:true,stableErrors:true,androidDeviceControlDisabled:true,
            commandExecutionDisabled:true,highImpactToolsDisabled:true
          } elif $mode == "volume-control" then {
            status:"pass",requests:29,exactArtifact:true,compileGate:true,runtimeDefaultDisabled:true,
            disabledDiscovery:true,staticTokenRequired:true,capabilityKeyRequired:true,closedInputSchema:true,
            previewNoMutation:true,previewDoesNotConsumeGrant:true,headerContextEnforced:true,
            exactGrantBinding:true,singleUseReplay:true,freshMaximum:true,fixedProgram:true,
            exactTwoArguments:true,fixedWorkingDirectory:true,inheritedEnvironmentCleared:true,nullStdin:true,
            nonQueueingConcurrency:true,mutationVerified:true,rollbackConfirmed:true,rollbackUnconfirmed:true,
            cancellationIndependentRecovery:true,boundedSupervisor:true,auditCounters:true,
            redactedResponses:true,arbitraryCommandExecutionDisabled:true,broaderAndroidControlDisabled:true,
            longObservationRequired:false
          } else {
            status:"pass",requests:29,exactArtifact:true,compileGate:true,runtimeDefaultDisabled:true,
            disabledDiscovery:true,fixedCurrentExecutable:true,wrongExecutableNameFailsClosed:true,
            wrongExecutableNameRejectedBeforeServing:true,runningInodePinned:true,
            workingDirectoryDescriptorPinned:true,fixedArgvProfiles:true,closedInputSchema:true,
            overrideFieldsRejected:true,unknownProfileRejected:true,fixedWorkingDirectory:true,
            inheritedEnvironmentCleared:true,nullStdin:true,boundedOutput:true,utf8Output:true,
            versionProfile:true,helpProfile:true,boundaryProfile:true,auditCounters:true,stableErrors:true,
            arbitraryCommandExecutionDisabled:true,androidDeviceControlDisabled:true,
            highImpactToolsDisabled:true,longObservationRequired:false
          } end
        )
      }
    ' >"$output"
}

make_specialized_evidence "$PAYLOAD/evidence/android-battery-emulated-v2.json" 2 2 2 battery
make_specialized_evidence "$PAYLOAD/evidence/android-volume-emulated-v1.json" 1 1 3 volume
make_specialized_evidence "$PAYLOAD/evidence/android-volume-control-emulated-v1.json" 1 1 4 volume-control 3
make_specialized_evidence "$PAYLOAD/evidence/command-emulated-v2.json" 2 2 5 command 0

aggregate_sha="$(sha256sum "$PAYLOAD/evidence/emulated-release-v3.json" | awk '{print $1}')"
jq -S -n \
  --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg full_sha "${binary_sha[6]}" --arg full_manifest_sha "${manifest_sha[6]}" \
  --arg aggregate_sha "$aggregate_sha" '
    {
      schemaVersion:2,classifierVersion:"2",status:"pass",failureCode:null,
      releaseQualificationEligible:false,createdAt:"2026-07-22T00:03:00Z",
      evidenceMode:"physical_observation_required",
      reasonCode:"full_suite_direct_physical_observation_required",inheritanceCandidate:false,
      source:{commit:("f" * 40)},
      candidate:{
        commit:$commit,version:$version,ciRunId:$ci,securityRunId:$security,androidRunId:$android,
        fullSuiteArtifactSha256:$full_sha,fullSuiteManifestSha256:$full_manifest_sha
      },
      emulation:{
        reportSha256:$aggregate_sha,executionMode:"official-termux-docker-native-arm64",
        imageDigest:("sha256:" + ("e" * 64)),status:"pass",samples:64
      },
      protectedInputComparison:{
        runtimeAndDeploymentInputsUnchanged:false,
        cargoAndDependencyInputsUnchangedExceptRootVersion:false
      },
      changedInputClasses:["full_suite_artifact"],nextGate:"direct_physical_device_observation"
    }
  ' >"$PAYLOAD/evidence/release-observation-requirement-v2.json"

jq -S -n \
  --arg repository "$REPOSITORY" --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg default_sha "${binary_sha[0]}" --argjson default_bytes "${binary_bytes[0]}" \
  --arg mcp_sha "${binary_sha[1]}" --argjson mcp_bytes "${binary_bytes[1]}" \
  --arg volume_sha "${binary_sha[4]}" --argjson volume_bytes "${binary_bytes[4]}" \
  --arg full_sha "${binary_sha[6]}" --argjson full_bytes "${binary_bytes[6]}" '
    {
      schemaVersion:2,
      validatorVersion:"11",
      status:"pass",
      failureCode:null,
      releaseEligible:true,
      startedAt:"2026-07-22T00:00:00Z",
      completedAt:"2026-07-22T01:05:00Z",
      repository:{commit:$commit,version:$version,ciRunId:$ci,securityRunId:$security,androidRunId:$android},
      environment:{architecture:"aarch64",fixtureMode:false,tools:{bash:"bash",curl:"curl",file:"file",jq:"jq"}},
      requestedPhase:"all",
      artifacts:{
        default:{sha256:$default_sha,bytes:$default_bytes,version:$version,elf:"aarch64-android-elf"},
        mcpRuntime:{sha256:$mcp_sha,bytes:$mcp_bytes,version:$version,elf:"aarch64-android-elf"},
        androidVolumeControl:{sha256:$volume_sha,bytes:$volume_bytes,version:$version,elf:"aarch64-android-elf"},
        fullSuite:{sha256:$full_sha,bytes:$full_bytes,version:$version,elf:"aarch64-android-elf"},
        baseline:{sha256:("9" * 64),bytes:900,version:"0.5.1",elf:"aarch64-android-elf"}
      },
      deploymentCandidate:{posture:"full-suite",productionAction:null},
      phases:{preflight:"pass",runtime:"pass",deployment:"pass"},
      results:[
        {phase:"runtime",check:"default_posture",outcome:"pass",code:"full_suite_default_disabled_17_tool_posture_verified"},
        {phase:"runtime",check:"battery_gate",outcome:"pass",code:"full_suite_battery_runtime_gate_independence_verified"},
        {phase:"runtime",check:"volume_status_gate",outcome:"pass",code:"full_suite_volume_status_runtime_gate_independence_verified"},
        {phase:"runtime",check:"volume_control_gate",outcome:"pass",code:"full_suite_volume_control_runtime_gate_independence_verified"},
        {phase:"runtime",check:"command_gate",outcome:"pass",code:"full_suite_command_runtime_gate_independence_verified"},
        {phase:"runtime",check:"enabled_posture",outcome:"pass",code:"full_suite_enabled_21_tool_posture_verified"},
        {phase:"runtime",check:"provider_success",outcome:"pass",code:"full_suite_optional_provider_success_verified"},
        {phase:"runtime",check:"volume_boundary",outcome:"pass",code:"full_suite_volume_preview_and_grant_boundary_verified"},
        {phase:"runtime",check:"command_profile",outcome:"pass",code:"full_suite_command_basename_and_profile_verified"},
        {phase:"runtime",check:"filesystem_posture",outcome:"pass",code:"full_suite_filesystem_mutations_independently_disabled"},
        {phase:"deployment",check:"candidate",outcome:"pass",code:"full_suite_deployment_candidate_selected"}
      ],
      sustainedObservation:{operatorSupplied:true,status:"pass",minutes:65,reasonCode:"stable",minimumMinutes:60}
    }
  ' >"$PAYLOAD/evidence/release-validator-v11.json"

validator_sha="$(sha256sum "$PAYLOAD/evidence/release-validator-v11.json" | awk '{print $1}')"
jq -S -n \
  --arg repository "$REPOSITORY" --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg validator_sha "$validator_sha" --arg full_sha "${binary_sha[6]}" '
    {
      schemaVersion:1,
      envelopeVersion:"1",
      status:"pass",
      failureCode:null,
      releaseEligible:true,
      repository:$repository,
      commit:$commit,
      version:$version,
      ciRunId:$ci,
      securityRunId:$security,
      androidRunId:$android,
      validatorVersion:"11",
      harnessVersion:"11",
      architecture:"aarch64",
      validatorReportSha256:$validator_sha,
      rawHarnessReportSha256:("8" * 64),
      workflowFullSuiteSha256:$full_sha,
      nativeFullSuiteSha256:("7" * 64),
      harnessPassed:true,
      cleanupConfirmed:true
    }
  ' >"$PAYLOAD/evidence/physical-qualification-v1.json"

file_record() {
  local path="$1" name="$2" output="$3" digest bytes
  digest="$(sha256sum "$path" | awk '{print $1}')"
  bytes="$(stat -c '%s' "$path")"
  jq -c -n --arg name "$name" --arg digest "$digest" --argjson bytes "$bytes" \
    '{fileName:$name,sha256:$digest,bytes:$bytes}' >"$output"
}

file_record "$PAYLOAD/LICENSE" LICENSE "$BASE/work/license.json"
file_record "$PAYLOAD/evidence/emulated-release-v3.json" evidence/emulated-release-v3.json "$BASE/work/aggregate.json"
file_record "$PAYLOAD/evidence/release-validator-v11.json" evidence/release-validator-v11.json "$BASE/work/validator.json"
file_record "$PAYLOAD/evidence/physical-qualification-v1.json" evidence/physical-qualification-v1.json "$BASE/work/physical.json"
: >"$BASE/work/specialized.jsonl"
for name in \
  evidence/android-battery-emulated-v2.json \
  evidence/android-volume-emulated-v1.json \
  evidence/android-volume-control-emulated-v1.json \
  evidence/command-emulated-v2.json \
  evidence/release-observation-requirement-v2.json
do
  file_record "$PAYLOAD/$name" "$name" "$BASE/work/specialized-one.json"
  jq -c . "$BASE/work/specialized-one.json" >>"$BASE/work/specialized.jsonl"
done

jq -S -n \
  --arg repository "$REPOSITORY" --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --slurpfile artifacts "$ARTIFACT_RECORDS" \
  --slurpfile license "$BASE/work/license.json" \
  --slurpfile aggregate "$BASE/work/aggregate.json" \
  --slurpfile validator "$BASE/work/validator.json" \
  --slurpfile physical "$BASE/work/physical.json" \
  --slurpfile specialized "$BASE/work/specialized.jsonl" '
    {
      schemaVersion:1,
      publicationState:"staged_not_released",
      releaseEligible:false,
      repository:$repository,
      commit:$commit,
      version:$version,
      target:"aarch64-linux-android",
      workflowRuns:{ci:$ci,security:$security,android:$android},
      checksums:{algorithm:"sha256",combinedFileName:"SHA256SUMS"},
      license:$license[0],
      evidence:{aggregate:$aggregate[0],validator:$validator[0],physicalQualification:$physical[0],specialized:$specialized},
      artifacts:$artifacts
    }
  ' >"$PAYLOAD/release-staging-manifest-v1.json"

chmod 644 "$PAYLOAD"/*.sha256 "$PAYLOAD/SHA256SUMS" "$PAYLOAD/LICENSE" \
  "$PAYLOAD"/*.workflow-manifest.json "$PAYLOAD/release-staging-manifest-v1.json" \
  "$PAYLOAD/evidence"/*.json

make_stage_tar() {
  local payload="$1" output="$2"
  mkdir -m 700 -p -- "$(dirname "$output")"
  tar --format=gnu --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner \
    --mode='u+rwX,go+rX,go-w' -C "$payload" -cf "$output" .
  chmod 600 "$output"
}

BASE_STAGE="$BASE/stage/$STAGE_NAME"
make_stage_tar "$PAYLOAD" "$BASE_STAGE"
BASE_STAGE_SHA="$(sha256sum "$BASE_STAGE" | awk '{print $1}')"

run_prepare() {
  local stage_tar="$1" output_parent="$2" digest="${3:-}"
  [[ -n "$digest" ]] || digest="$(sha256sum -- "$stage_tar" | awk '{print $1}')"
  PATH="${PREP_TEST_PATH:-$ROOT/fake-bin:$REAL_PATH}" bash "$SCRIPT" \
    --stage-tar "$stage_tar" \
    --staged-artifact-sha256 "$digest" \
    --assets-dir "$output_parent/assets" \
    --receipt "$output_parent/$RECEIPT_NAME" \
    --repository "$REPOSITORY" \
    --commit "$COMMIT" \
    --version "$VERSION"
}

make_case() {
  local name="$1" case_root
  case_root="$ROOT/cases/$name"
  mkdir -m 700 -p "$case_root"
  cp -a "$PAYLOAD" "$case_root/payload"
  printf '%s\n' "$case_root"
}

repack_case() {
  local case_root="$1" output
  output="$case_root/$STAGE_NAME"
  make_stage_tar "$case_root/payload" "$output"
  printf '%s\n' "$output"
}

refresh_evidence_record() {
  local case_root="$1" evidence_path="$2" filter="$3" digest bytes
  digest="$(sha256sum "$case_root/payload/$evidence_path" | awk '{print $1}')"
  bytes="$(stat -c '%s' "$case_root/payload/$evidence_path")"
  jq --arg sha "$digest" --argjson bytes "$bytes" "$filter" \
    "$case_root/payload/release-staging-manifest-v1.json" >"$case_root/stage.next"
  mv "$case_root/stage.next" "$case_root/payload/release-staging-manifest-v1.json"
}

bash -n "$SCRIPT"
bash -n "$0"

mkdir -m 700 "$ROOT/results-one" "$ROOT/results-two"
run_prepare "$BASE_STAGE" "$ROOT/results-one" "$BASE_STAGE_SHA" >"$ROOT/happy.stdout"
grep -Fq 'assets=16' "$ROOT/happy.stdout" || fail_test 'success summary omitted exact asset count'
ASSETS_ONE="$ROOT/results-one/assets"
RECEIPT_ONE="$ROOT/results-one/$RECEIPT_NAME"
[[ -d "$ASSETS_ONE" && ! -L "$ASSETS_ONE" ]] || fail_test 'publication asset directory missing'
[[ -f "$RECEIPT_ONE" && ! -L "$RECEIPT_ONE" ]] || fail_test 'publication receipt missing'
[[ "$(find "$ASSETS_ONE" -mindepth 1 -maxdepth 1 -type f | wc -l)" == 16 ]] \
  || fail_test 'publication asset file count is not exact'
[[ -z "$(find "$ASSETS_ONE" -mindepth 1 -maxdepth 1 ! -type f -print -quit)" ]] \
  || fail_test 'publication asset directory contains a non-file'
cmp -s "$BASE_STAGE" "$ASSETS_ONE/$STAGE_NAME" || fail_test 'raw stage tar bytes changed'
cmp -s "$PAYLOAD/SHA256SUMS" "$ASSETS_ONE/SHA256SUMS" || fail_test 'combined checksum bytes changed'
for release_name in "${release_names[@]}"; do
  cmp -s "$PAYLOAD/$release_name" "$ASSETS_ONE/$release_name" \
    || fail_test 'governed binary bytes changed'
  cmp -s "$PAYLOAD/$release_name.sha256" "$ASSETS_ONE/$release_name.sha256" \
    || fail_test 'per-file checksum bytes changed'
  [[ "$(stat -c '%a' "$ASSETS_ONE/$release_name")" == 755 ]] || fail_test 'published binary mode changed'
done

jq -e \
  --arg repository "$REPOSITORY" --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg stage_name "$STAGE_NAME" --arg stage_sha "$BASE_STAGE_SHA" \
  --argjson stage_size "$(stat -c '%s' "$BASE_STAGE")" '
    (keys == ["assets","commit","repository","schemaVersion","stageTar","version"])
    and .schemaVersion == 1
    and .repository == $repository
    and .commit == $commit
    and .version == $version
    and .stageTar == {name:$stage_name,sha256:$stage_sha,size:$stage_size}
    and (.assets | length == 16)
    and ([.assets[].name] == ([.assets[].name] | sort))
    and all(.assets[]; keys == ["name","sha256","size","sourceStageMember"])
    and ([.assets[] | select(.name == $stage_name and .sourceStageMember == null)] | length == 1)
    and all(.assets[] | select(.name != $stage_name); .sourceStageMember == .name)
  ' "$RECEIPT_ONE" >/dev/null || fail_test 'receipt contract is invalid'

while IFS=$'\t' read -r name size digest; do
  path="$ASSETS_ONE/$name"
  [[ -f "$path" && ! -L "$path" ]] || fail_test 'receipt names a missing asset'
  [[ "$(stat -c '%s' "$path")" == "$size" ]] || fail_test 'receipt asset size mismatch'
  [[ "$(sha256sum "$path" | awk '{print $1}')" == "$digest" ]] || fail_test 'receipt asset digest mismatch'
done < <(jq -r '.assets[] | [.name,(.size|tostring),.sha256] | @tsv' "$RECEIPT_ONE")

run_prepare "$BASE_STAGE" "$ROOT/results-two" "$BASE_STAGE_SHA" >/dev/null
cmp -s "$RECEIPT_ONE" "$ROOT/results-two/$RECEIPT_NAME" \
  || fail_test 'identical stage inputs produced different receipts'
for path in "$ASSETS_ONE"/*; do
  cmp -s "$path" "$ROOT/results-two/assets/$(basename "$path")" \
    || fail_test 'identical stage inputs produced different asset bytes'
done

# Exercise the production handoff, not only the independently assembled tar
# fixture: feed the same governed inputs through the canonical staging script,
# then consume its unchanged output with this publication verifier.
integration_root="$ROOT/canonical-integration"
mkdir -m 700 -p "$integration_root/bundles" "$integration_root/emulated" \
  "$integration_root/stage" "$integration_root/publication"
for index in "${!postures[@]}"; do
  bundle="$integration_root/bundles/${postures[$index]}"
  mkdir -m 700 "$bundle"
  cp "$PAYLOAD/${release_names[$index]}" "$bundle/termux-mcp-server"
  cp "$PAYLOAD/${release_names[$index]}.workflow-manifest.json" "$bundle/artifact-manifest.json"
  printf '%s  termux-mcp-server\n' "${binary_sha[$index]}" >"$bundle/SHA256SUMS"
  chmod 700 "$bundle/termux-mcp-server"
  chmod 600 "$bundle/artifact-manifest.json" "$bundle/SHA256SUMS"
done
cp "$PAYLOAD/evidence/emulated-release-v3.json" \
  "$integration_root/emulated/termux-emulated-evidence.json"
cp "$PAYLOAD/evidence/android-battery-emulated-v2.json" \
  "$integration_root/emulated/termux-battery-emulated-evidence.json"
cp "$PAYLOAD/evidence/android-volume-emulated-v1.json" \
  "$integration_root/emulated/termux-volume-emulated-evidence.json"
cp "$PAYLOAD/evidence/android-volume-control-emulated-v1.json" \
  "$integration_root/emulated/termux-volume-control-emulated-evidence.json"
cp "$PAYLOAD/evidence/command-emulated-v2.json" \
  "$integration_root/emulated/termux-command-emulated-evidence.json"
cp "$PAYLOAD/evidence/release-observation-requirement-v2.json" \
  "$integration_root/emulated/termux-observation-requirement.json"
cp "$PAYLOAD/evidence/release-validator-v11.json" "$integration_root/validator.json"
cp "$PAYLOAD/evidence/physical-qualification-v1.json" "$integration_root/physical.json"
cp "$PAYLOAD/LICENSE" "$integration_root/LICENSE"
chmod 600 "$integration_root/emulated"/*.json "$integration_root/validator.json" \
  "$integration_root/physical.json" "$integration_root/LICENSE"

canonical_stage="$integration_root/stage/$STAGE_NAME"
PATH="$ROOT/fake-bin:$REAL_PATH" bash "$REPO_ROOT/scripts/stage_release_assets.sh" \
  --default-dir "$integration_root/bundles/default" \
  --mcp-runtime-dir "$integration_root/bundles/mcp-runtime" \
  --android-battery-status-dir "$integration_root/bundles/android-battery-status" \
  --android-volume-status-dir "$integration_root/bundles/android-volume-status" \
  --android-volume-control-dir "$integration_root/bundles/android-volume-control" \
  --command-execution-dir "$integration_root/bundles/command-execution" \
  --full-suite-dir "$integration_root/bundles/full-suite" \
  --emulated-evidence-dir "$integration_root/emulated" \
  --validator-evidence "$integration_root/validator.json" \
  --physical-qualification "$integration_root/physical.json" \
  --license "$integration_root/LICENSE" \
  --repository "$REPOSITORY" --commit "$COMMIT" --version "$VERSION" \
  --ci-run-id "$CI_RUN_ID" --security-run-id "$SECURITY_RUN_ID" --android-run-id "$ANDROID_RUN_ID" \
  --output "$canonical_stage" >/dev/null
canonical_stage_sha="$(sha256sum "$canonical_stage" | awk '{print $1}')"
run_prepare "$canonical_stage" "$integration_root/publication" "$canonical_stage_sha" >/dev/null
[[ "$(find "$integration_root/publication/assets" -mindepth 1 -maxdepth 1 -type f | wc -l)" == 16 ]] \
  || fail_test 'canonical staging-to-publication integration asset count is not exact'
cmp -s "$canonical_stage" "$integration_root/publication/assets/$STAGE_NAME" \
  || fail_test 'canonical staging-to-publication integration changed the stage tar'

assert_fails stage_digest_mismatch "$BASE_STAGE" "$ROOT/fail-digest" "$(printf '0%.0s' {1..64})"

ln -s "$BASE_STAGE" "$ROOT/$STAGE_NAME"
assert_fails stage_tar_path_invalid "$ROOT/$STAGE_NAME" "$ROOT/fail-stage-link" "$BASE_STAGE_SHA"

case_root="$(make_case extra-member)"
printf '%s\n' unexpected >"$case_root/payload/extra"
assert_fails archive_members_invalid "$(repack_case "$case_root")" "$ROOT/fail-extra"

case_root="$(make_case missing-member)"
rm "$case_root/payload/${release_names[0]}.sha256"
assert_fails archive_members_invalid "$(repack_case "$case_root")" "$ROOT/fail-missing"

case_root="$(make_case linked-member)"
rm "$case_root/payload/${release_names[0]}.sha256"
ln -s "${release_names[0]}" "$case_root/payload/${release_names[0]}.sha256"
assert_fails archive_link_or_special_file "$(repack_case "$case_root")" "$ROOT/fail-link"

case_root="$(make_case special-member)"
rm "$case_root/payload/${release_names[0]}.sha256"
mkfifo "$case_root/payload/${release_names[0]}.sha256"
assert_fails archive_link_or_special_file "$(repack_case "$case_root")" "$ROOT/fail-special"

case_root="$(make_case traversal-member)"
python3 - "$case_root/$STAGE_NAME" <<'PY'
import io
import sys
import tarfile
with tarfile.open(sys.argv[1], "w", format=tarfile.GNU_FORMAT) as archive:
    member = tarfile.TarInfo("../escape")
    member.size = 1
    member.mode = 0o644
    member.uid = member.gid = member.mtime = 0
    archive.addfile(member, io.BytesIO(b"x"))
PY
chmod 600 "$case_root/$STAGE_NAME"
assert_fails archive_members_invalid "$case_root/$STAGE_NAME" "$ROOT/fail-traversal"

case_root="$(make_case nonzero-member-padding)"
padding_stage="$(repack_case "$case_root")"
python3 - "$padding_stage" <<'PY'
import sys
import tarfile

stage = sys.argv[1]
with tarfile.open(stage, "r:") as archive:
    member = next(item for item in archive.getmembers() if item.isfile() and item.size % 512)
    padding_offset = member.offset_data + member.size
with open(stage, "r+b") as handle:
    handle.seek(padding_offset)
    handle.write(b"X")
PY
assert_fails archive_layout_not_canonical "$padding_stage" "$ROOT/fail-nonzero-padding"

oversized_parent="$ROOT/oversized"
mkdir -m 700 "$oversized_parent"
truncate -s 536870913 "$oversized_parent/$STAGE_NAME"
chmod 600 "$oversized_parent/$STAGE_NAME"
assert_fails stage_tar_invalid "$oversized_parent/$STAGE_NAME" "$ROOT/fail-oversized" "$(printf '0%.0s' {1..64})"

case_root="$(make_case binary-tamper)"
printf '%s\n' tampered >>"$case_root/payload/${release_names[2]}"
assert_fails artifact_record_digest_mismatch "$(repack_case "$case_root")" "$ROOT/fail-binary-tamper"

case_root="$(make_case aggregate-checksum-tamper)"
printf '%s\n' unexpected >>"$case_root/payload/SHA256SUMS"
assert_fails combined_checksum_mismatch "$(repack_case "$case_root")" "$ROOT/fail-combined-checksum"

case_root="$(make_case evidence-link-tamper)"
printf '%s\n' tampered >>"$case_root/payload/evidence/emulated-release-v3.json"
assert_fails evidence_record_mismatch "$(repack_case "$case_root")" "$ROOT/fail-evidence-link"

case_root="$(make_case aggregate-semantic-tamper)"
aggregate="$case_root/payload/evidence/emulated-release-v3.json"
jq '.aggregateValidation.defaultDisabled.toolCount = 18' "$aggregate" >"$case_root/aggregate.next"
mv "$case_root/aggregate.next" "$aggregate"
refresh_evidence_record "$case_root" evidence/emulated-release-v3.json \
  '.evidence.aggregate.sha256 = $sha | .evidence.aggregate.bytes = $bytes'
assert_fails aggregate_evidence_mismatch "$(repack_case "$case_root")" "$ROOT/fail-aggregate-semantic"

case_root="$(make_case aggregate-invalid-json)"
printf '%s\n' '{' >"$case_root/payload/evidence/emulated-release-v3.json"
refresh_evidence_record "$case_root" evidence/emulated-release-v3.json \
  '.evidence.aggregate.sha256 = $sha | .evidence.aggregate.bytes = $bytes'
assert_fails aggregate_evidence_mismatch "$(repack_case "$case_root")" "$ROOT/fail-aggregate-json"

case_root="$(make_case aggregate-duplicate-json-key)"
aggregate="$case_root/payload/evidence/emulated-release-v3.json"
sed '1s/^{/{"schemaVersion":3,/' "$aggregate" >"$case_root/aggregate.next"
mv "$case_root/aggregate.next" "$aggregate"
refresh_evidence_record "$case_root" evidence/emulated-release-v3.json \
  '.evidence.aggregate.sha256 = $sha | .evidence.aggregate.bytes = $bytes'
assert_fails aggregate_evidence_mismatch "$(repack_case "$case_root")" "$ROOT/fail-aggregate-duplicate-key"

case_root="$(make_case specialized-semantic-tamper)"
battery="$case_root/payload/evidence/android-battery-emulated-v2.json"
jq '.candidate.androidRunId = "9999"' "$battery" >"$case_root/battery.next"
mv "$case_root/battery.next" "$battery"
refresh_evidence_record "$case_root" evidence/android-battery-emulated-v2.json \
  '.evidence.specialized[0].sha256 = $sha | .evidence.specialized[0].bytes = $bytes'
assert_fails specialized_evidence_mismatch "$(repack_case "$case_root")" "$ROOT/fail-specialized-semantic"

case_root="$(make_case observation-semantic-tamper)"
observation="$case_root/payload/evidence/release-observation-requirement-v2.json"
jq '.candidate.fullSuiteManifestSha256 = ("0" * 64)' "$observation" >"$case_root/observation.next"
mv "$case_root/observation.next" "$observation"
refresh_evidence_record "$case_root" evidence/release-observation-requirement-v2.json \
  '.evidence.specialized[4].sha256 = $sha | .evidence.specialized[4].bytes = $bytes'
assert_fails observation_requirement_mismatch "$(repack_case "$case_root")" "$ROOT/fail-observation-semantic"

case_root="$(make_case observation-conditional-tamper)"
observation="$case_root/payload/evidence/release-observation-requirement-v2.json"
jq '.evidenceMode = "observation_inheritance_candidate"' "$observation" >"$case_root/observation.next"
mv "$case_root/observation.next" "$observation"
refresh_evidence_record "$case_root" evidence/release-observation-requirement-v2.json \
  '.evidence.specialized[4].sha256 = $sha | .evidence.specialized[4].bytes = $bytes'
assert_fails observation_requirement_mismatch "$(repack_case "$case_root")" "$ROOT/fail-observation-conditional"

case_root="$(make_case schema-boolean-const-confusion)"
jq '.schemaVersion = true' "$case_root/payload/release-staging-manifest-v1.json" >"$case_root/stage.next"
mv "$case_root/stage.next" "$case_root/payload/release-staging-manifest-v1.json"
assert_fails staging_manifest_schema_mismatch "$(repack_case "$case_root")" "$ROOT/fail-schema-boolean"

case_root="$(make_case workflow-manifest-identity)"
workflow_manifest="$case_root/payload/${release_names[0]}.workflow-manifest.json"
jq '.commit = ("b" * 40)' "$workflow_manifest" >"$case_root/workflow.next"
mv "$case_root/workflow.next" "$workflow_manifest"
new_manifest_sha="$(sha256sum "$workflow_manifest" | awk '{print $1}')"
jq --arg sha "$new_manifest_sha" '.artifacts[0].workflowManifestSha256 = $sha' \
  "$case_root/payload/release-staging-manifest-v1.json" >"$case_root/stage.next"
mv "$case_root/stage.next" "$case_root/payload/release-staging-manifest-v1.json"
assert_fails workflow_manifest_identity_mismatch "$(repack_case "$case_root")" "$ROOT/fail-workflow-identity"

case_root="$(make_case validator-ineligible)"
validator="$case_root/payload/evidence/release-validator-v11.json"
jq '.releaseEligible = false' "$validator" >"$case_root/validator.next"
mv "$case_root/validator.next" "$validator"
new_sha="$(sha256sum "$validator" | awk '{print $1}')"
new_bytes="$(stat -c '%s' "$validator")"
jq --arg sha "$new_sha" --argjson bytes "$new_bytes" \
  '.evidence.validator.sha256 = $sha | .evidence.validator.bytes = $bytes' \
  "$case_root/payload/release-staging-manifest-v1.json" >"$case_root/stage.next"
mv "$case_root/stage.next" "$case_root/payload/release-staging-manifest-v1.json"
assert_fails validator_eligibility_mismatch "$(repack_case "$case_root")" "$ROOT/fail-validator"

case_root="$(make_case physical-ineligible)"
physical="$case_root/payload/evidence/physical-qualification-v1.json"
jq '.cleanupConfirmed = false' "$physical" >"$case_root/physical.next"
mv "$case_root/physical.next" "$physical"
new_sha="$(sha256sum "$physical" | awk '{print $1}')"
new_bytes="$(stat -c '%s' "$physical")"
jq --arg sha "$new_sha" --argjson bytes "$new_bytes" \
  '.evidence.physicalQualification.sha256 = $sha | .evidence.physicalQualification.bytes = $bytes' \
  "$case_root/payload/release-staging-manifest-v1.json" >"$case_root/stage.next"
mv "$case_root/stage.next" "$case_root/payload/release-staging-manifest-v1.json"
assert_fails physical_qualification_eligibility_mismatch "$(repack_case "$case_root")" "$ROOT/fail-physical"

case_root="$(make_case wrong-architecture)"
index=2
binary="$case_root/payload/${release_names[$index]}"
printf '%s\n' '#!/system/bin/sh' '# wrong-arch' 'exit 0' >"$binary"
chmod 755 "$binary"
new_sha="$(sha256sum "$binary" | awk '{print $1}')"
new_bytes="$(stat -c '%s' "$binary")"
printf '%s  %s\n' "$new_sha" "${release_names[$index]}" \
  >"$case_root/payload/${release_names[$index]}.sha256"
: >"$case_root/payload/SHA256SUMS"
for current_index in "${!release_names[@]}"; do
  current_binary="$case_root/payload/${release_names[$current_index]}"
  current_sha="$(sha256sum "$current_binary" | awk '{print $1}')"
  printf '%s  %s\n' "$current_sha" "${release_names[$current_index]}" >>"$case_root/payload/SHA256SUMS"
done
workflow_manifest="$case_root/payload/${release_names[$index]}.workflow-manifest.json"
jq --arg sha "$new_sha" --argjson bytes "$new_bytes" '.sha256 = $sha | .bytes = $bytes' \
  "$workflow_manifest" >"$case_root/workflow.next"
mv "$case_root/workflow.next" "$workflow_manifest"
new_manifest_sha="$(sha256sum "$workflow_manifest" | awk '{print $1}')"
jq --arg sha "$new_sha" --argjson bytes "$new_bytes" --arg manifest_sha "$new_manifest_sha" \
  '.artifacts[2].sha256 = $sha | .artifacts[2].bytes = $bytes | .artifacts[2].workflowManifestSha256 = $manifest_sha' \
  "$case_root/payload/release-staging-manifest-v1.json" >"$case_root/stage.next"
mv "$case_root/stage.next" "$case_root/payload/release-staging-manifest-v1.json"
assert_fails binary_architecture_mismatch "$(repack_case "$case_root")" "$ROOT/fail-architecture"

case_root="$(make_case snapshot-symlink-race)"
race_stage="$(repack_case "$case_root")"
external_target="$ROOT/snapshot-race-external"
printf '%s\n' external-owner >"$external_target"
chmod 644 "$external_target"
mkdir -m 700 "$ROOT/racing-cp-symlink"
cat >"$ROOT/racing-cp-symlink/cp" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
source_path="${@: -2:1}"
if [[ "$(basename "$source_path")" == termux-mcp-server-v*-release-stage-*.tar ]]; then
  "$PREP_REAL_MV" -- "$source_path" "$source_path.real"
  ln -s -- "$PREP_TEST_EXTERNAL_TARGET" "$source_path"
fi
"$PREP_REAL_CP" "$@"
EOF
chmod 700 "$ROOT/racing-cp-symlink/cp"
PREP_REAL_CP="$REAL_CP" PREP_REAL_MV="$REAL_MV" PREP_TEST_EXTERNAL_TARGET="$external_target" \
PREP_TEST_PATH="$ROOT/racing-cp-symlink:$ROOT/fake-bin:$REAL_PATH" \
  assert_fails stage_snapshot_failed "$race_stage" "$ROOT/fail-snapshot-symlink-race"
[[ "$(stat -c '%a' "$external_target")" == 644 ]] \
  || fail_test 'snapshot symlink race changed external target permissions'
[[ "$(<"$external_target")" == external-owner ]] \
  || fail_test 'snapshot symlink race changed external target contents'

case_root="$(make_case snapshot-race)"
race_stage="$(repack_case "$case_root")"
race_stage_sha="$(sha256sum "$race_stage" | awk '{print $1}')"
mkdir -m 700 "$ROOT/racing-cp" "$ROOT/result-snapshot-race"
cat >"$ROOT/racing-cp/cp" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
source_path="${@: -2:1}"
"$PREP_REAL_CP" "$@"
if [[ "$(basename "$source_path")" == termux-mcp-server-v*-release-stage-*.tar ]]; then
  printf 'source replaced after snapshot\n' >>"$source_path"
fi
EOF
chmod 700 "$ROOT/racing-cp/cp"
PREP_REAL_CP="$REAL_CP" PREP_TEST_PATH="$ROOT/racing-cp:$ROOT/fake-bin:$REAL_PATH" \
  run_prepare "$race_stage" "$ROOT/result-snapshot-race" "$race_stage_sha" >/dev/null
[[ "$(sha256sum "$ROOT/result-snapshot-race/assets/$STAGE_NAME" | awk '{print $1}')" == "$race_stage_sha" ]] \
  || fail_test 'source replacement changed the validated stage snapshot'

case_root="$(make_case publication-race)"
race_stage="$(repack_case "$case_root")"
mkdir -m 700 "$ROOT/racing-mv"
cat >"$ROOT/racing-mv/mv" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
target="${@: -1}"
if [[ "$target" == "$PREP_TEST_ASSETS_DIR" && ! -e "$target" ]]; then
  mkdir -m 700 "$target"
  printf '%s\n' concurrent-owner >"$target/owner"
fi
"$PREP_REAL_MV" "$@"
EOF
chmod 700 "$ROOT/racing-mv/mv"
PREP_REAL_MV="$REAL_MV" \
PREP_TEST_ASSETS_DIR="$ROOT/result-publication-race/assets" \
PREP_TEST_PATH="$ROOT/racing-mv:$ROOT/fake-bin:$REAL_PATH" \
  assert_fails assets_publication_failed "$race_stage" "$ROOT/result-publication-race"
[[ "$(<"$ROOT/result-publication-race/assets/owner")" == concurrent-owner ]] \
  || fail_test 'no-clobber publication changed a concurrent output owner'
[[ -z "$(find "$ROOT/result-publication-race" -maxdepth 1 -name 'assets.staging.*' -print -quit)" ]] \
  || fail_test 'publication race leaked owned staging state'

case_root="$(make_case receipt-publication-race)"
race_stage="$(repack_case "$case_root")"
mkdir -m 700 "$ROOT/racing-mv-receipt"
cat >"$ROOT/racing-mv-receipt/mv" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
target="${@: -1}"
if [[ "$target" == "$PREP_TEST_RECEIPT" && ! -e "$target" ]]; then
  printf '%s\n' concurrent-owner >"$target"
fi
"$PREP_REAL_MV" "$@"
EOF
chmod 700 "$ROOT/racing-mv-receipt/mv"
PREP_REAL_MV="$REAL_MV" \
PREP_TEST_RECEIPT="$ROOT/result-receipt-publication-race/$RECEIPT_NAME" \
PREP_TEST_PATH="$ROOT/racing-mv-receipt:$ROOT/fake-bin:$REAL_PATH" \
  assert_fails receipt_publication_failed "$race_stage" "$ROOT/result-receipt-publication-race"
[[ "$(<"$ROOT/result-receipt-publication-race/$RECEIPT_NAME")" == concurrent-owner ]] \
  || fail_test 'receipt no-clobber race changed a concurrent output owner'
[[ -d "$ROOT/result-receipt-publication-race/assets" ]] \
  || fail_test 'receipt race did not preserve the validated fail-closed asset set'
[[ "$(find "$ROOT/result-receipt-publication-race/assets" -mindepth 1 -maxdepth 1 -type f | wc -l)" == 16 ]] \
  || fail_test 'receipt race left an incomplete validated asset set'
[[ -z "$(find "$ROOT/result-receipt-publication-race" -maxdepth 1 -name 'assets.staging.*' -print -quit)" ]] \
  || fail_test 'receipt race leaked owned staging state'

case_root="$(make_case post-publication-tamper)"
race_stage="$(repack_case "$case_root")"
mkdir -m 700 "$ROOT/racing-mv-tamper"
cat >"$ROOT/racing-mv-tamper/mv" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
target="${@: -1}"
"$PREP_REAL_MV" "$@"
if [[ "$target" == "$PREP_TEST_ASSETS_DIR" && -d "$target" ]]; then
  printf 'concurrent tamper\n' >>"$target/SHA256SUMS"
fi
EOF
chmod 700 "$ROOT/racing-mv-tamper/mv"
PREP_REAL_MV="$REAL_MV" \
PREP_TEST_ASSETS_DIR="$ROOT/result-post-publication-tamper/assets" \
PREP_TEST_PATH="$ROOT/racing-mv-tamper:$ROOT/fake-bin:$REAL_PATH" \
  assert_fails published_asset_receipt_mismatch "$race_stage" "$ROOT/result-post-publication-tamper"

printf 'Release publication asset preparation tests passed\n'
