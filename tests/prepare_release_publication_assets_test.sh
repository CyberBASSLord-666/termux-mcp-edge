#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C

ROOT="$(mktemp -d)"
trap 'rm -rf -- "$ROOT"' EXIT INT TERM
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
SCRIPT="$REPO_ROOT/scripts/prepare_release_publication_assets.sh"
STAGE_SCRIPT="$REPO_ROOT/scripts/stage_release_assets.sh"
PACKAGER="$REPO_ROOT/scripts/package_automated_qualification.sh"
POLICY="$REPO_ROOT/docs/release-qualification-policy-v1.json"
SCENARIO_SET="$REPO_ROOT/docs/automated-native-deployment-scenarios-v1.json"
REAL_PATH="$PATH"
REAL_CP="$(command -v cp)"
REAL_MV="$(command -v mv)"
REAL_PYTHON3="$(command -v python3)"
REPOSITORY="CyberBASSLord-666/termux-mcp-edge"
COMMIT="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
VERSION="0.6.0"
CI_RUN_ID="5101"
SECURITY_RUN_ID="5102"
ANDROID_RUN_ID="5103"
QUALIFICATION_RUN_ID="5104"
ROOTFS_DIGEST="sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
ROOTFS_IMAGE_ID="sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
RUNTIME_IMAGE_ID="sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
LINKER_SHA="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
STAGE_NAME="termux-mcp-server-v${VERSION}-release-stage-${COMMIT:0:12}.tar"
RECEIPT_NAME="release-publication-receipt-v1.json"
BUNDLE_NAME="publication-inputs"

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
install -d -m 700 -- "$PAYLOAD/evidence" "$BASE/work" "$BASE/stage"
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
      schemaVersion:4,
      gateVersion:"4",
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
        image:"termux/termux-docker:aarch64",imageDigest:("sha256:" + ("c" * 64)),
        rootfsImageId:("sha256:" + ("b" * 64)),
        runtimeImageDigest:("sha256:" + ("d" * 64)),androidLinker:true
      },
      claimBoundary:{
        physicalDeviceObserved:false,androidFrameworkObserved:false,
        sustainedPhysicalSoak:false,physicalCertification:"not_run"
      },
      coverage:{
        covered:[
          "exact_android_artifacts","official_termux_userland_native_arm64",
          "android_bionic_linker","deterministic_provider_simulation",
          "runtime_gate_composition","bounded_native_stress"
        ],
        notCovered:[
          "physical_device","android_framework","oem_policy","battery_aging",
          "thermal_soak","radio","doze"
        ]
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
        automatedQualificationComponent:true
      },
      stress:{
        status:"pass",samples:64,requests:256,servicePidStable:true,healthReadyStable:true,
        sessionLifecycle:true,exactToolAllowlist:true,safeRootIdentityPinned:true,
        safeRootAncestorIdentityPinned:true,copyFileMutationDisabled:true,highImpactDisabled:true,
        longObservationRequired:false
      }
    }
  ' >"$PAYLOAD/evidence/termux-native-aggregate-evidence-v4.json"

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
          image:"termux/termux-docker:aarch64",imageDigest:("sha256:" + ("c" * 64)),
          rootfsImageId:("sha256:" + ("b" * 64)),
          runtimeImageDigest:("sha256:" + ("d" * 64)),androidLinker:true
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

make_specialized_evidence "$PAYLOAD/evidence/termux-battery-emulated-evidence.json" 3 3 2 battery
make_specialized_evidence "$PAYLOAD/evidence/termux-volume-emulated-evidence.json" 2 2 3 volume
make_specialized_evidence "$PAYLOAD/evidence/termux-volume-control-emulated-evidence.json" 2 2 4 volume-control 3
make_specialized_evidence "$PAYLOAD/evidence/termux-command-emulated-evidence.json" 3 3 5 command 0

aggregate_sha="$(sha256sum "$PAYLOAD/evidence/termux-native-aggregate-evidence-v4.json" | awk '{print $1}')"
jq -S -n \
  --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg full_sha "${binary_sha[6]}" --arg full_manifest_sha "${manifest_sha[6]}" \
  --arg aggregate_sha "$aggregate_sha" '
    {
      schemaVersion:3,classifierVersion:"3",status:"pass",failureCode:null,
      releaseQualificationEligible:false,createdAt:"2026-07-22T00:03:00Z",
      evidenceMode:"automated_release_qualification",
      reasonCode:"automated_native_termux_evidence_required",inheritanceCandidate:false,
      source:{commit:("f" * 40)},
      candidate:{
        commit:$commit,version:$version,ciRunId:$ci,securityRunId:$security,androidRunId:$android,
        fullSuiteArtifactSha256:$full_sha,fullSuiteManifestSha256:$full_manifest_sha
      },
      emulation:{
        reportSha256:$aggregate_sha,executionMode:"official-termux-docker-native-arm64",
        imageDigest:("sha256:" + ("c" * 64)),status:"pass",samples:64
      },
      claimBoundary:{
        physicalDeviceObserved:false,androidFrameworkObserved:false,
        sustainedPhysicalSoak:false,physicalCertification:"not_run"
      },
      protectedInputComparison:{
        runtimeAndDeploymentInputsUnchanged:false,
        cargoAndDependencyInputsUnchangedExceptRootVersion:false
      },
      changedInputClasses:["runtime_or_deployment","cargo_or_dependency"],
      nextGate:"assemble_automated_release_qualification"
    }
  ' >"$PAYLOAD/evidence/termux-observation-requirement-v3.json"

scenario_sha="$(sha256sum "$SCENARIO_SET" | awk '{print $1}')"
jq -S -n \
  --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg full_sha "${binary_sha[6]}" --argjson full_bytes "${binary_bytes[6]}" \
  --arg full_manifest_sha "${manifest_sha[6]}" --arg scenario_sha "$scenario_sha" '
    {
      schemaVersion:1,gateVersion:"1",status:"pass",failureCode:null,
      releaseQualificationEligible:false,
      qualificationClass:"official_termux_native_automated_v1",
      startedAt:"2026-07-22T00:04:00Z",completedAt:"2026-07-22T00:05:00Z",
      candidate:{
        repository:"CyberBASSLord-666/termux-mcp-edge",commit:$commit,version:$version,
        ciRunId:$ci,securityRunId:$security,nativeRunId:$android,
        artifact:{
          artifactName:"termux-mcp-server-aarch64-linux-android-full-suite",
          posture:"full-suite",features:["full-suite"],sha256:$full_sha,
          manifestSha256:$full_manifest_sha,bytes:$full_bytes,
          target:"aarch64-linux-android",elf:"aarch64-android-elf"
        }
      },
      scenarioSet:{
        fileName:"automated-native-deployment-scenarios-v1.json",schemaVersion:1,
        scenarioSetVersion:"1",sha256:$scenario_sha,scenarioCount:6,
        scenarioIds:[
          "isolated_fresh_deploy","failed_upgrade_recovery","supervised_restart",
          "rollback_recovery","uninstall","bounded_cleanup"
        ]
      },
      environment:{
        architecture:"aarch64",executionMode:"official-termux-docker-native-arm64",
        rootfsImage:"termux/termux-docker:aarch64",rootfsDigest:("sha256:" + ("c" * 64)),
        rootfsImageId:("sha256:" + ("b" * 64)),
        runtimeImageDigest:("sha256:" + ("d" * 64)),
        termuxPrefix:"/data/data/com.termux/files/usr",
        androidLinker:{observed:true,path:"/system/bin/linker64",sha256:("a" * 64),bytes:4096},
        supervisor:"runit",runitSupervisorObserved:true,androidFrameworkObserved:false,
        physicalHardwareObserved:false,physicalDeviceObserved:false,sustainedPhysicalSoak:false
      },
      validation:{
        status:"pass",
        scenarioResults:[
          {id:"isolated_fresh_deploy",execution:"native",outcome:"pass",faultBoundary:"none"},
          {id:"failed_upgrade_recovery",execution:"native",outcome:"recovered",faultBoundary:"target_readiness_probe"},
          {id:"supervised_restart",execution:"native",outcome:"restarted",faultBoundary:"supervised_process"},
          {id:"rollback_recovery",execution:"native",outcome:"recovered",faultBoundary:"target_readiness_probe"},
          {id:"uninstall",execution:"native",outcome:"removed",faultBoundary:"none"},
          {id:"bounded_cleanup",execution:"native",outcome:"clean",faultBoundary:"none"}
        ],
        artifactManifestStrict:true,scenarioSetStrict:true,nativeArtifactExecuted:true,
        isolatedFreshDeploy:true,failedUpgradeRecovery:true,supervisedRestart:true,
        rollbackRecovery:true,uninstall:true,boundedCleanup:true,exactArtifact:true,
        isolatedServiceRoot:true,runitSupervisorObserved:true,realLoopbackProbes:true,
        probeFaultInjectionBounded:true,outputNoClobber:true,workspaceRemoved:true,
        serviceRemoved:true,runsvdirTerminated:true,physicalCertification:"not_run"
      }
    }
  ' >"$PAYLOAD/evidence/automated-native-deployment-v1.json"

RUNTIME_INPUT="$BASE/runtime-input"
mkdir -m 700 "$RUNTIME_INPUT"
RUNTIME_ARCHIVE="$RUNTIME_INPUT/termux-qualified-runtime-image-v1.tar.gz"
RUNTIME_LOCK="$RUNTIME_INPUT/termux-runtime-package-lock-v1.json"
RUNTIME_SNAPSHOT="$RUNTIME_INPUT/termux-runtime-snapshot-v1.json"
RUNTIME_REPLAY="$RUNTIME_INPUT/termux-runtime-snapshot-replay-v1.json"
printf 'retained qualified runtime archive fixture\n' >"$RUNTIME_ARCHIVE"
chmod 600 "$RUNTIME_ARCHIVE"
RUNTIME_ARCHIVE_SHA="$(sha256sum "$RUNTIME_ARCHIVE" | awk '{print $1}')"
RUNTIME_ARCHIVE_BYTES="$(stat -c '%s' "$RUNTIME_ARCHIVE")"
jq -n \
  --arg commit "$COMMIT" --arg android "$ANDROID_RUN_ID" \
  --arg base_digest "$ROOTFS_DIGEST" --arg base_id "$ROOTFS_IMAGE_ID" '
  {
    schemaVersion:1,lockVersion:"1",
    repository:"CyberBASSLord-666/termux-mcp-edge",
    commit:$commit,androidRunId:$android,
    base:{
      image:"termux/termux-docker:aarch64",
      digest:$base_digest,imageId:$base_id
    },
    requestedPackages:["file","jq","python","termux-services"],
    resolution:{
      resolver:"termux-apt-download-only",
      repositoryMetadataAuthenticated:true,
      packageBytesFrozenBeforeBuild:true,
      finalImageBuildNetwork:"none"
    },
    installation:{
      method:"termux-dpkg-unpack-configure",
      dependencyRepair:"none",
      runtimeUser:"1000:1000"
    },
    repositoryIndexes:[
      {fileName:"packages.termux.dev_InRelease",sha256:("1"*64),bytes:101}
    ],
    packages:[
      {
        package:"file",version:"1.0",architecture:"aarch64",
        fileName:"file_1.0_aarch64.deb",sha256:("2"*64),bytes:102
      },
      {
        package:"jq",version:"1.0",architecture:"aarch64",
        fileName:"jq_1.0_aarch64.deb",sha256:("3"*64),bytes:103
      },
      {
        package:"python",version:"1.0",architecture:"aarch64",
        fileName:"python_1.0_aarch64.deb",sha256:("4"*64),bytes:104
      },
      {
        package:"termux-services",version:"1.0",architecture:"aarch64",
        fileName:"termux-services_1.0_aarch64.deb",sha256:("5"*64),bytes:105
      }
    ]
  }
' >"$RUNTIME_LOCK"
chmod 600 "$RUNTIME_LOCK"
RUNTIME_LOCK_SHA="$(sha256sum "$RUNTIME_LOCK" | awk '{print $1}')"
RUNTIME_LOCK_BYTES="$(stat -c '%s' "$RUNTIME_LOCK")"
RUNTIME_INVENTORY_SHA="$(
  printf '%s\n' \
    $'file\t1.0\taarch64' \
    $'jq\t1.0\taarch64' \
    $'python\t1.0\taarch64' \
    $'termux-services\t1.0\taarch64' |
    sha256sum |
    awk '{print $1}'
)"
jq -n \
  --arg commit "$COMMIT" --arg android "$ANDROID_RUN_ID" \
  --arg base_digest "$ROOTFS_DIGEST" --arg base_id "$ROOTFS_IMAGE_ID" \
  --arg runtime_id "$RUNTIME_IMAGE_ID" \
  --arg lock_sha "$RUNTIME_LOCK_SHA" --argjson lock_bytes "$RUNTIME_LOCK_BYTES" \
  --arg inventory_sha "$RUNTIME_INVENTORY_SHA" \
  --arg archive_sha "$RUNTIME_ARCHIVE_SHA" \
  --argjson archive_bytes "$RUNTIME_ARCHIVE_BYTES" '
  {
    schemaVersion:1,snapshotVersion:"1",status:"pass",failureCode:null,
    releaseQualificationEligible:false,
    repository:"CyberBASSLord-666/termux-mcp-edge",
    commit:$commit,androidRunId:$android,
    base:{
      image:"termux/termux-docker:aarch64",
      digest:$base_digest,imageId:$base_id
    },
    runtimeImageId:$runtime_id,
    platform:{os:"linux",architecture:"arm64"},
    rootfsLayers:[("sha256:" + ("6"*64))],
    packageLock:{
      fileName:"termux-runtime-package-lock-v1.json",
      sha256:$lock_sha,bytes:$lock_bytes
    },
    installedPackages:{
      sha256:$inventory_sha,count:4,
      packages:[
        {package:"file",version:"1.0",architecture:"aarch64"},
        {package:"jq",version:"1.0",architecture:"aarch64"},
        {package:"python",version:"1.0",architecture:"aarch64"},
        {package:"termux-services",version:"1.0",architecture:"aarch64"}
      ]
    },
    archive:{
      fileName:"termux-qualified-runtime-image-v1.tar.gz",
      format:"docker-image-archive-v1",compression:"gzip-no-name",
      sha256:$archive_sha,bytes:$archive_bytes
    },
    claimBoundary:{
      physicalDeviceObserved:false,androidFrameworkObserved:false,
      sustainedPhysicalSoak:false,physicalCertification:"not_run"
    },
    rebuildReproducibilityClaim:false
  }
' >"$RUNTIME_SNAPSHOT"
chmod 600 "$RUNTIME_SNAPSHOT"
RUNTIME_SNAPSHOT_SHA="$(sha256sum "$RUNTIME_SNAPSHOT" | awk '{print $1}')"
RUNTIME_SNAPSHOT_BYTES="$(stat -c '%s' "$RUNTIME_SNAPSHOT")"
jq -n \
  --arg commit "$COMMIT" --arg runtime_id "$RUNTIME_IMAGE_ID" \
  --arg snapshot_sha "$RUNTIME_SNAPSHOT_SHA" \
  --argjson snapshot_bytes "$RUNTIME_SNAPSHOT_BYTES" \
  --arg archive_sha "$RUNTIME_ARCHIVE_SHA" \
  --argjson archive_bytes "$RUNTIME_ARCHIVE_BYTES" \
  --arg lock_sha "$RUNTIME_LOCK_SHA" --argjson lock_bytes "$RUNTIME_LOCK_BYTES" \
  --arg inventory_sha "$RUNTIME_INVENTORY_SHA" --arg linker_sha "$LINKER_SHA" '
  {
    schemaVersion:1,replayVersion:"1",status:"pass",failureCode:null,
    releaseQualificationEligible:false,
    repository:"CyberBASSLord-666/termux-mcp-edge",
    commit:$commit,runtimeImageId:$runtime_id,
    snapshot:{
      manifest:{
        fileName:"termux-runtime-snapshot-v1.json",
        sha256:$snapshot_sha,bytes:$snapshot_bytes
      },
      archive:{
        fileName:"termux-qualified-runtime-image-v1.tar.gz",
        format:"docker-image-archive-v1",compression:"gzip-no-name",
        sha256:$archive_sha,bytes:$archive_bytes
      }
    },
    packageLock:{
      fileName:"termux-runtime-package-lock-v1.json",
      sha256:$lock_sha,bytes:$lock_bytes
    },
    installedPackages:{sha256:$inventory_sha,count:4},
    androidLinker:{path:"/system/bin/linker64",sha256:$linker_sha,bytes:4096},
    verification:{
      archiveDigestVerified:true,singleImageArchive:true,
      loadedImageIdVerified:true,platformVerified:true,
      runtimeUserVerified:true,
      rootfsLayersVerified:true,packageLockVerified:true,
      packageInputBytesVerified:true,repositoryIndexBytesVerified:true,
      installedPackageInventoryVerified:true,
      requiredRuntimeCommandsVerified:true,androidLinkerVerified:true,
      runtimeNetworkAccess:false
    },
    claimBoundary:{
      physicalDeviceObserved:false,androidFrameworkObserved:false,
      sustainedPhysicalSoak:false,physicalCertification:"not_run"
    },
    rebuildReproducibilityClaim:false
  }
' >"$RUNTIME_REPLAY"
chmod 600 "$RUNTIME_REPLAY"

chmod 600 "$PAYLOAD/evidence"/*.json
install -d -m 700 "$BASE/bundles"
for index in "${!postures[@]}"; do
  bundle="$BASE/bundles/${postures[$index]}"
  mkdir -m 700 "$bundle"
  cp "$PAYLOAD/${release_names[$index]}" "$bundle/termux-mcp-server"
  cp "$PAYLOAD/${release_names[$index]}.workflow-manifest.json" "$bundle/artifact-manifest.json"
  printf '%s  termux-mcp-server\n' "${binary_sha[$index]}" >"$bundle/SHA256SUMS"
  chmod 700 "$bundle/termux-mcp-server"
  chmod 600 "$bundle/artifact-manifest.json" "$bundle/SHA256SUMS"
done

PATH="$ROOT/fake-bin:$REAL_PATH" bash "$PACKAGER" \
  --policy "$POLICY" \
  --scenario-set "$SCENARIO_SET" \
  --aggregate-evidence "$PAYLOAD/evidence/termux-native-aggregate-evidence-v4.json" \
  --deployment-evidence "$PAYLOAD/evidence/automated-native-deployment-v1.json" \
  --classifier-evidence "$PAYLOAD/evidence/termux-observation-requirement-v3.json" \
  --battery-evidence "$PAYLOAD/evidence/termux-battery-emulated-evidence.json" \
  --volume-evidence "$PAYLOAD/evidence/termux-volume-emulated-evidence.json" \
  --volume-control-evidence "$PAYLOAD/evidence/termux-volume-control-emulated-evidence.json" \
  --command-evidence "$PAYLOAD/evidence/termux-command-emulated-evidence.json" \
  --default-dir "$BASE/bundles/default" \
  --mcp-runtime-dir "$BASE/bundles/mcp-runtime" \
  --battery-dir "$BASE/bundles/android-battery-status" \
  --volume-dir "$BASE/bundles/android-volume-status" \
  --volume-control-dir "$BASE/bundles/android-volume-control" \
  --command-dir "$BASE/bundles/command-execution" \
  --full-suite-dir "$BASE/bundles/full-suite" \
  --runtime-archive "$RUNTIME_ARCHIVE" \
  --runtime-package-lock "$RUNTIME_LOCK" \
  --runtime-snapshot "$RUNTIME_SNAPSHOT" \
  --runtime-replay "$RUNTIME_REPLAY" \
  --qualification-run-id "$QUALIFICATION_RUN_ID" \
  --output "$PAYLOAD/evidence/automated-qualification-v1.json" >/dev/null

make_stage_tar() {
  local payload="$1" output="$2"
  install -d -m 700 -- "$(dirname "$output")"
  tar --format=gnu --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner \
    --mode='u+rwX,go+rX,go-w' -C "$payload" -cf "$output" .
  chmod 600 "$output"
}

BASE_STAGE="$BASE/stage/$STAGE_NAME"
mkdir -m 700 "$BASE/emulated-input"
for evidence_name in \
  termux-native-aggregate-evidence-v4.json \
  termux-battery-emulated-evidence.json \
  termux-volume-emulated-evidence.json \
  termux-volume-control-emulated-evidence.json \
  termux-command-emulated-evidence.json \
  termux-observation-requirement-v3.json
do
  cp "$PAYLOAD/evidence/$evidence_name" "$BASE/emulated-input/$evidence_name"
done
chmod 600 "$BASE/emulated-input"/*.json
PATH="$ROOT/fake-bin:$REAL_PATH" bash "$STAGE_SCRIPT" \
  --default-dir "$BASE/bundles/default" \
  --mcp-runtime-dir "$BASE/bundles/mcp-runtime" \
  --android-battery-status-dir "$BASE/bundles/android-battery-status" \
  --android-volume-status-dir "$BASE/bundles/android-volume-status" \
  --android-volume-control-dir "$BASE/bundles/android-volume-control" \
  --command-execution-dir "$BASE/bundles/command-execution" \
  --full-suite-dir "$BASE/bundles/full-suite" \
  --emulated-evidence-dir "$BASE/emulated-input" \
  --deployment-evidence "$PAYLOAD/evidence/automated-native-deployment-v1.json" \
  --automated-qualification "$PAYLOAD/evidence/automated-qualification-v1.json" \
  --runtime-archive "$RUNTIME_ARCHIVE" \
  --runtime-package-lock "$RUNTIME_LOCK" \
  --runtime-snapshot "$RUNTIME_SNAPSHOT" \
  --runtime-replay "$RUNTIME_REPLAY" \
  --license "$PAYLOAD/LICENSE" \
  --repository "$REPOSITORY" --commit "$COMMIT" --version "$VERSION" \
  --ci-run-id "$CI_RUN_ID" --security-run-id "$SECURITY_RUN_ID" --android-run-id "$ANDROID_RUN_ID" \
  --qualification-run-id "$QUALIFICATION_RUN_ID" \
  --output "$BASE_STAGE" >/dev/null
BASE_STAGE_SHA="$(sha256sum "$BASE_STAGE" | awk '{print $1}')"

SOURCE_PAYLOAD="$BASE/source-payload"
mv "$PAYLOAD" "$SOURCE_PAYLOAD"
mkdir -m 700 "$PAYLOAD"
tar -xf "$BASE_STAGE" -C "$PAYLOAD"

run_prepare() {
  local stage_tar="$1" output_parent="$2" digest="${3:-}"
  [[ -n "$digest" ]] || digest="$(sha256sum -- "$stage_tar" | awk '{print $1}')"
  PATH="${PREP_TEST_PATH:-$ROOT/fake-bin:$REAL_PATH}" bash "$SCRIPT" \
    --stage-tar "$stage_tar" \
    --staged-artifact-sha256 "$digest" \
    --assets-dir "$output_parent/$BUNDLE_NAME/assets" \
    --receipt "$output_parent/$BUNDLE_NAME/$RECEIPT_NAME" \
    --repository "$REPOSITORY" \
    --commit "$COMMIT" \
    --version "$VERSION"
}

make_case() {
  local name="$1" case_root
  case_root="$ROOT/cases/$name"
  install -d -m 700 "$case_root"
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
    "$case_root/payload/release-staging-manifest-v2.json" >"$case_root/stage.next"
  mv "$case_root/stage.next" "$case_root/payload/release-staging-manifest-v2.json"
}

refresh_qualification_record() {
  local case_root="$1" qualification digest bytes
  qualification="$case_root/payload/evidence/automated-qualification-v1.json"
  digest="$(sha256sum "$qualification" | awk '{print $1}')"
  bytes="$(stat -c '%s' "$qualification")"
  jq --arg sha "$digest" --argjson bytes "$bytes" \
    '.evidence.qualification.sha256 = $sha | .evidence.qualification.bytes = $bytes' \
    "$case_root/payload/release-staging-manifest-v2.json" >"$case_root/stage.next"
  mv "$case_root/stage.next" "$case_root/payload/release-staging-manifest-v2.json"
}

bash -n "$SCRIPT"
bash -n "$0"
grep -Fq 'max_sizes[name] = 16_777_216' "$SCRIPT" \
  || fail_test 'retained runtime JSON limit changed'
grep -Fq 'max_sizes[runtime_names[0]] = 1_610_612_736' "$SCRIPT" \
  || fail_test 'retained runtime archive limit changed'

mkdir -m 700 "$ROOT/results-one" "$ROOT/results-two"
run_prepare "$BASE_STAGE" "$ROOT/results-one" "$BASE_STAGE_SHA" >"$ROOT/happy.stdout"
grep -Fq 'assets=16' "$ROOT/happy.stdout" || fail_test 'success summary omitted exact asset count'
BUNDLE_ONE="$ROOT/results-one/$BUNDLE_NAME"
ASSETS_ONE="$BUNDLE_ONE/assets"
RECEIPT_ONE="$BUNDLE_ONE/$RECEIPT_NAME"
[[ -d "$BUNDLE_ONE" && ! -L "$BUNDLE_ONE" ]] || fail_test 'publication bundle missing'
[[ "$(find "$BUNDLE_ONE" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)" == \
  $'assets\nrelease-publication-receipt-v1.json' ]] \
  || fail_test 'publication bundle inventory is not exact'
[[ -d "$ASSETS_ONE" && ! -L "$ASSETS_ONE" ]] || fail_test 'publication asset directory missing'
[[ -f "$RECEIPT_ONE" && ! -L "$RECEIPT_ONE" ]] || fail_test 'publication receipt missing'
if [[ -n "${PREP_FIXTURE_EXPORT_DIR:-}" ]]; then
  [[ "$PREP_FIXTURE_EXPORT_DIR" == /* \
    && ! -e "$PREP_FIXTURE_EXPORT_DIR" \
    && ! -L "$PREP_FIXTURE_EXPORT_DIR" ]] \
    || fail_test 'fixture export destination is invalid'
  mkdir -m 700 -- "$PREP_FIXTURE_EXPORT_DIR"
  cp -a -- "$ASSETS_ONE" "$PREP_FIXTURE_EXPORT_DIR/assets"
  cp -p -- "$RECEIPT_ONE" \
    "$PREP_FIXTURE_EXPORT_DIR/release-publication-receipt-v1.json"
  printf 'Release publication fixture exported\n'
  exit 0
fi
[[ "$(find "$ASSETS_ONE" -mindepth 1 -maxdepth 1 -type f | wc -l)" == 16 ]] \
  || fail_test 'publication asset file count is not exact'
[[ -z "$(find "$ASSETS_ONE" -mindepth 1 -maxdepth 1 ! -type f -print -quit)" ]] \
  || fail_test 'publication asset directory contains a non-file'
cmp -s "$BASE_STAGE" "$ASSETS_ONE/$STAGE_NAME" || fail_test 'raw stage tar bytes changed'
cmp -s "$PAYLOAD/SHA256SUMS" "$ASSETS_ONE/SHA256SUMS" || fail_test 'combined checksum bytes changed'
for runtime_name in \
  termux-qualified-runtime-image-v1.tar.gz \
  termux-runtime-package-lock-v1.json \
  termux-runtime-snapshot-v1.json \
  termux-runtime-snapshot-replay-v1.json
do
  [[ ! -e "$ASSETS_ONE/$runtime_name" && ! -L "$ASSETS_ONE/$runtime_name" ]] \
    || fail_test 'retained runtime member escaped the unchanged stage tar'
  tar -xOf "$BASE_STAGE" "./evidence/runtime/$runtime_name" \
    >"$ROOT/extracted-$runtime_name"
  cmp -s "$PAYLOAD/evidence/runtime/$runtime_name" "$ROOT/extracted-$runtime_name" \
    || fail_test 'retained runtime member bytes changed in the stage'
done
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
cmp -s "$RECEIPT_ONE" "$ROOT/results-two/$BUNDLE_NAME/$RECEIPT_NAME" \
  || fail_test 'identical stage inputs produced different receipts'
for path in "$ASSETS_ONE"/*; do
  cmp -s "$path" "$ROOT/results-two/$BUNDLE_NAME/assets/$(basename "$path")" \
    || fail_test 'identical stage inputs produced different asset bytes'
done

# Exercise the production handoff, not only the independently assembled tar
# fixture: feed the same governed inputs through the canonical staging script,
# then consume its unchanged output with this publication verifier.
integration_root="$ROOT/canonical-integration"
install -d -m 700 "$integration_root/bundles" "$integration_root/emulated" \
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
cp "$PAYLOAD/evidence/termux-native-aggregate-evidence-v4.json" \
  "$integration_root/emulated/termux-native-aggregate-evidence-v4.json"
cp "$PAYLOAD/evidence/termux-battery-emulated-evidence.json" \
  "$integration_root/emulated/termux-battery-emulated-evidence.json"
cp "$PAYLOAD/evidence/termux-volume-emulated-evidence.json" \
  "$integration_root/emulated/termux-volume-emulated-evidence.json"
cp "$PAYLOAD/evidence/termux-volume-control-emulated-evidence.json" \
  "$integration_root/emulated/termux-volume-control-emulated-evidence.json"
cp "$PAYLOAD/evidence/termux-command-emulated-evidence.json" \
  "$integration_root/emulated/termux-command-emulated-evidence.json"
cp "$PAYLOAD/evidence/termux-observation-requirement-v3.json" \
  "$integration_root/emulated/termux-observation-requirement-v3.json"
cp "$PAYLOAD/evidence/automated-native-deployment-v1.json" \
  "$integration_root/automated-native-deployment-v1.json"
cp "$PAYLOAD/evidence/automated-qualification-v1.json" \
  "$integration_root/automated-qualification-v1.json"
cp "$PAYLOAD/LICENSE" "$integration_root/LICENSE"
chmod 600 "$integration_root/emulated"/*.json \
  "$integration_root/automated-native-deployment-v1.json" \
  "$integration_root/automated-qualification-v1.json" "$integration_root/LICENSE"

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
  --deployment-evidence "$integration_root/automated-native-deployment-v1.json" \
  --automated-qualification "$integration_root/automated-qualification-v1.json" \
  --runtime-archive "$PAYLOAD/evidence/runtime/termux-qualified-runtime-image-v1.tar.gz" \
  --runtime-package-lock "$PAYLOAD/evidence/runtime/termux-runtime-package-lock-v1.json" \
  --runtime-snapshot "$PAYLOAD/evidence/runtime/termux-runtime-snapshot-v1.json" \
  --runtime-replay "$PAYLOAD/evidence/runtime/termux-runtime-snapshot-replay-v1.json" \
  --license "$integration_root/LICENSE" \
  --repository "$REPOSITORY" --commit "$COMMIT" --version "$VERSION" \
  --ci-run-id "$CI_RUN_ID" --security-run-id "$SECURITY_RUN_ID" --android-run-id "$ANDROID_RUN_ID" \
  --qualification-run-id "$QUALIFICATION_RUN_ID" \
  --output "$canonical_stage" >/dev/null
canonical_stage_sha="$(sha256sum "$canonical_stage" | awk '{print $1}')"
run_prepare "$canonical_stage" "$integration_root/publication" "$canonical_stage_sha" >/dev/null
[[ "$(find "$integration_root/publication/$BUNDLE_NAME/assets" -mindepth 1 -maxdepth 1 -type f | wc -l)" == 16 ]] \
  || fail_test 'canonical staging-to-publication integration asset count is not exact'
cmp -s "$canonical_stage" "$integration_root/publication/$BUNDLE_NAME/assets/$STAGE_NAME" \
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

stage_limit_parent="$ROOT/stage-limit"
mkdir -m 700 "$stage_limit_parent" "$ROOT/failing-cp"
truncate -s 2147483647 "$stage_limit_parent/$STAGE_NAME"
chmod 600 "$stage_limit_parent/$STAGE_NAME"
cat >"$ROOT/failing-cp/cp" <<'EOF'
#!/usr/bin/env bash
exit 1
EOF
chmod 700 "$ROOT/failing-cp/cp"
PREP_TEST_PATH="$ROOT/failing-cp:$ROOT/fake-bin:$REAL_PATH" \
  assert_fails stage_snapshot_failed "$stage_limit_parent/$STAGE_NAME" \
  "$ROOT/fail-stage-exact-limit" "$(printf '0%.0s' {1..64})"

oversized_parent="$ROOT/oversized"
mkdir -m 700 "$oversized_parent"
truncate -s 2147483648 "$oversized_parent/$STAGE_NAME"
chmod 600 "$oversized_parent/$STAGE_NAME"
assert_fails stage_tar_invalid "$oversized_parent/$STAGE_NAME" \
  "$ROOT/fail-oversized" "$(printf '0%.0s' {1..64})"

case_root="$(make_case binary-tamper)"
printf '%s\n' tampered >>"$case_root/payload/${release_names[2]}"
assert_fails artifact_record_digest_mismatch "$(repack_case "$case_root")" "$ROOT/fail-binary-tamper"

case_root="$(make_case aggregate-checksum-tamper)"
printf '%s\n' unexpected >>"$case_root/payload/SHA256SUMS"
assert_fails combined_checksum_mismatch "$(repack_case "$case_root")" "$ROOT/fail-combined-checksum"

case_root="$(make_case evidence-link-tamper)"
printf '%s\n' tampered >>"$case_root/payload/evidence/termux-native-aggregate-evidence-v4.json"
assert_fails evidence_record_mismatch "$(repack_case "$case_root")" "$ROOT/fail-evidence-link"

case_root="$(make_case runtime-archive-link-tamper)"
printf '%s\n' tampered \
  >>"$case_root/payload/evidence/runtime/termux-qualified-runtime-image-v1.tar.gz"
assert_fails runtime_evidence_record_mismatch \
  "$(repack_case "$case_root")" "$ROOT/fail-runtime-archive-link"

case_root="$(make_case runtime-network-claim-tamper)"
runtime_replay="$case_root/payload/evidence/runtime/termux-runtime-snapshot-replay-v1.json"
jq '.verification.runtimeNetworkAccess = true' "$runtime_replay" \
  >"$case_root/runtime-replay.next"
mv "$case_root/runtime-replay.next" "$runtime_replay"
refresh_evidence_record "$case_root" \
  evidence/runtime/termux-runtime-snapshot-replay-v1.json \
  '.evidence.runtime.replay.sha256 = $sha | .evidence.runtime.replay.bytes = $bytes'
assert_fails runtime_replay_mismatch \
  "$(repack_case "$case_root")" "$ROOT/fail-runtime-network-claim"

case_root="$(make_case runtime-rebuild-claim-tamper)"
runtime_snapshot="$case_root/payload/evidence/runtime/termux-runtime-snapshot-v1.json"
jq '.rebuildReproducibilityClaim = true' "$runtime_snapshot" \
  >"$case_root/runtime-snapshot.next"
mv "$case_root/runtime-snapshot.next" "$runtime_snapshot"
refresh_evidence_record "$case_root" \
  evidence/runtime/termux-runtime-snapshot-v1.json \
  '.evidence.runtime.snapshot.sha256 = $sha | .evidence.runtime.snapshot.bytes = $bytes'
assert_fails runtime_snapshot_mismatch \
  "$(repack_case "$case_root")" "$ROOT/fail-runtime-rebuild-claim"

case_root="$(make_case runtime-missing-installed-package)"
runtime_snapshot="$case_root/payload/evidence/runtime/termux-runtime-snapshot-v1.json"
python3 - "$runtime_snapshot" <<'PY'
import hashlib
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
snapshot = json.loads(path.read_text(encoding="utf-8"))
packages = [
    item
    for item in snapshot["installedPackages"]["packages"]
    if item["package"] != "termux-services"
]
if len(packages) + 1 != snapshot["installedPackages"]["count"]:
    raise SystemExit("missing-package fixture did not remove exactly one package")
inventory_raw = "".join(
    f"{item['package']}\t{item['version']}\t{item['architecture']}\n"
    for item in packages
).encode()
snapshot["installedPackages"] = {
    "sha256": hashlib.sha256(inventory_raw).hexdigest(),
    "count": len(packages),
    "packages": packages,
}
path.write_text(
    json.dumps(snapshot, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
PY
refresh_evidence_record "$case_root" \
  evidence/runtime/termux-runtime-snapshot-v1.json \
  '.evidence.runtime.snapshot.sha256 = $sha | .evidence.runtime.snapshot.bytes = $bytes'
assert_fails runtime_package_installation_mismatch \
  "$(repack_case "$case_root")" "$ROOT/fail-runtime-missing-installed-package"

case_root="$(make_case qualification-runtime-byte-substitution)"
qualification="$case_root/payload/evidence/automated-qualification-v1.json"
jq '.retainedRuntime.archive.sha256 = ("0" * 64)' "$qualification" \
  >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$qualification"
refresh_qualification_record "$case_root"
assert_fails automated_qualification_mismatch \
  "$(repack_case "$case_root")" "$ROOT/fail-qualification-runtime-byte-substitution"

case_root="$(make_case aggregate-semantic-tamper)"
aggregate="$case_root/payload/evidence/termux-native-aggregate-evidence-v4.json"
jq '.aggregateValidation.defaultDisabled.toolCount = 18' "$aggregate" >"$case_root/aggregate.next"
mv "$case_root/aggregate.next" "$aggregate"
refresh_evidence_record "$case_root" evidence/termux-native-aggregate-evidence-v4.json \
  '.evidence.aggregate.sha256 = $sha | .evidence.aggregate.bytes = $bytes'
assert_fails aggregate_evidence_mismatch "$(repack_case "$case_root")" "$ROOT/fail-aggregate-semantic"

case_root="$(make_case aggregate-invalid-json)"
printf '%s\n' '{' >"$case_root/payload/evidence/termux-native-aggregate-evidence-v4.json"
refresh_evidence_record "$case_root" evidence/termux-native-aggregate-evidence-v4.json \
  '.evidence.aggregate.sha256 = $sha | .evidence.aggregate.bytes = $bytes'
assert_fails aggregate_evidence_mismatch "$(repack_case "$case_root")" "$ROOT/fail-aggregate-json"

case_root="$(make_case aggregate-duplicate-json-key)"
aggregate="$case_root/payload/evidence/termux-native-aggregate-evidence-v4.json"
sed '1s/^{/{"schemaVersion":4,/' "$aggregate" >"$case_root/aggregate.next"
mv "$case_root/aggregate.next" "$aggregate"
refresh_evidence_record "$case_root" evidence/termux-native-aggregate-evidence-v4.json \
  '.evidence.aggregate.sha256 = $sha | .evidence.aggregate.bytes = $bytes'
assert_fails aggregate_evidence_mismatch "$(repack_case "$case_root")" "$ROOT/fail-aggregate-duplicate-key"

case_root="$(make_case specialized-semantic-tamper)"
battery="$case_root/payload/evidence/termux-battery-emulated-evidence.json"
jq '.candidate.androidRunId = "9999"' "$battery" >"$case_root/battery.next"
mv "$case_root/battery.next" "$battery"
refresh_evidence_record "$case_root" evidence/termux-battery-emulated-evidence.json \
  '.evidence.specialized[0].sha256 = $sha | .evidence.specialized[0].bytes = $bytes'
assert_fails specialized_evidence_mismatch "$(repack_case "$case_root")" "$ROOT/fail-specialized-semantic"

case_root="$(make_case observation-semantic-tamper)"
observation="$case_root/payload/evidence/termux-observation-requirement-v3.json"
jq '.candidate.fullSuiteManifestSha256 = ("0" * 64)' "$observation" >"$case_root/observation.next"
mv "$case_root/observation.next" "$observation"
refresh_evidence_record "$case_root" evidence/termux-observation-requirement-v3.json \
  '.evidence.classifier.sha256 = $sha | .evidence.classifier.bytes = $bytes'
assert_fails observation_requirement_mismatch "$(repack_case "$case_root")" "$ROOT/fail-observation-semantic"

case_root="$(make_case observation-conditional-tamper)"
observation="$case_root/payload/evidence/termux-observation-requirement-v3.json"
jq '.evidenceMode = "observation_inheritance_candidate"' "$observation" >"$case_root/observation.next"
mv "$case_root/observation.next" "$observation"
refresh_evidence_record "$case_root" evidence/termux-observation-requirement-v3.json \
  '.evidence.classifier.sha256 = $sha | .evidence.classifier.bytes = $bytes'
assert_fails observation_requirement_mismatch "$(repack_case "$case_root")" "$ROOT/fail-observation-conditional"

case_root="$(make_case observation-change-list-tamper)"
observation="$case_root/payload/evidence/termux-observation-requirement-v3.json"
jq '.changedInputClasses = []' "$observation" >"$case_root/observation.next"
mv "$case_root/observation.next" "$observation"
refresh_evidence_record "$case_root" evidence/termux-observation-requirement-v3.json \
  '.evidence.classifier.sha256 = $sha | .evidence.classifier.bytes = $bytes'
assert_fails observation_requirement_mismatch "$(repack_case "$case_root")" \
  "$ROOT/fail-observation-change-list"

case_root="$(make_case schema-boolean-const-confusion)"
jq '.schemaVersion = true' "$case_root/payload/release-staging-manifest-v2.json" >"$case_root/stage.next"
mv "$case_root/stage.next" "$case_root/payload/release-staging-manifest-v2.json"
assert_fails staging_manifest_schema_mismatch "$(repack_case "$case_root")" "$ROOT/fail-schema-boolean"

case_root="$(make_case schema-cross-posture-name)"
jq '.artifacts[0].workflowArtifactName = .artifacts[1].workflowArtifactName' \
  "$case_root/payload/release-staging-manifest-v2.json" >"$case_root/stage.next"
mv "$case_root/stage.next" "$case_root/payload/release-staging-manifest-v2.json"
assert_fails staging_manifest_schema_mismatch "$(repack_case "$case_root")" \
  "$ROOT/fail-schema-cross-posture-name"

case_root="$(make_case schema-path-traversal)"
jq '.artifacts[0].workflowManifestFileName = "../artifact-manifest.json"' \
  "$case_root/payload/release-staging-manifest-v2.json" >"$case_root/stage.next"
mv "$case_root/stage.next" "$case_root/payload/release-staging-manifest-v2.json"
assert_fails staging_manifest_schema_mismatch "$(repack_case "$case_root")" \
  "$ROOT/fail-schema-path-traversal"

case_root="$(make_case qualification-run-substitution)"
jq '.workflowRuns.qualification = "9999"' \
  "$case_root/payload/release-staging-manifest-v2.json" >"$case_root/stage.next"
mv "$case_root/stage.next" "$case_root/payload/release-staging-manifest-v2.json"
assert_fails automated_qualification_mismatch \
  "$(repack_case "$case_root")" "$ROOT/fail-qualification-run-substitution"

case_root="$(make_case workflow-manifest-identity)"
workflow_manifest="$case_root/payload/${release_names[0]}.workflow-manifest.json"
jq '.commit = ("b" * 40)' "$workflow_manifest" >"$case_root/workflow.next"
mv "$case_root/workflow.next" "$workflow_manifest"
new_manifest_sha="$(sha256sum "$workflow_manifest" | awk '{print $1}')"
jq --arg sha "$new_manifest_sha" '.artifacts[0].workflowManifestSha256 = $sha' \
  "$case_root/payload/release-staging-manifest-v2.json" >"$case_root/stage.next"
mv "$case_root/stage.next" "$case_root/payload/release-staging-manifest-v2.json"
assert_fails workflow_manifest_identity_mismatch "$(repack_case "$case_root")" "$ROOT/fail-workflow-identity"

case_root="$(make_case qualification-class-downgrade)"
qualification="$case_root/payload/evidence/automated-qualification-v1.json"
jq '.qualificationClass = "physical_device_v1"' "$qualification" >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$qualification"
refresh_evidence_record "$case_root" evidence/automated-qualification-v1.json \
  '.evidence.qualification.sha256 = $sha | .evidence.qualification.bytes = $bytes'
assert_fails automated_qualification_mismatch "$(repack_case "$case_root")" "$ROOT/fail-qualification-class"

case_root="$(make_case qualification-claim-lie)"
qualification="$case_root/payload/evidence/automated-qualification-v1.json"
jq '.claimBoundary.physicalDeviceObserved = true' "$qualification" >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$qualification"
refresh_evidence_record "$case_root" evidence/automated-qualification-v1.json \
  '.evidence.qualification.sha256 = $sha | .evidence.qualification.bytes = $bytes'
assert_fails automated_qualification_mismatch "$(repack_case "$case_root")" "$ROOT/fail-qualification-claim"

case_root="$(make_case qualification-extra-key)"
qualification="$case_root/payload/evidence/automated-qualification-v1.json"
jq '.operatorPhysicalReport = "forbidden"' "$qualification" >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$qualification"
refresh_evidence_record "$case_root" evidence/automated-qualification-v1.json \
  '.evidence.qualification.sha256 = $sha | .evidence.qualification.bytes = $bytes'
assert_fails automated_qualification_mismatch "$(repack_case "$case_root")" "$ROOT/fail-qualification-extra"

case_root="$(make_case qualification-duplicate-key)"
qualification="$case_root/payload/evidence/automated-qualification-v1.json"
sed '1s/^{/{"schemaVersion":1,/' "$qualification" >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$qualification"
refresh_evidence_record "$case_root" evidence/automated-qualification-v1.json \
  '.evidence.qualification.sha256 = $sha | .evidence.qualification.bytes = $bytes'
assert_fails qualification_json_invalid "$(repack_case "$case_root")" "$ROOT/fail-qualification-duplicate"

case_root="$(make_case physical-v1-substitution)"
qualification="$case_root/payload/evidence/automated-qualification-v1.json"
jq -S -n \
  --arg commit "$COMMIT" --arg version "$VERSION" \
  '{schemaVersion:1,envelopeVersion:"1",status:"pass",failureCode:null,releaseEligible:true,
    qualificationClass:"physical_device_v1",commit:$commit,version:$version,
    physicalDeviceObserved:true,physicalCertification:"pass"}' >"$qualification"
refresh_evidence_record "$case_root" evidence/automated-qualification-v1.json \
  '.evidence.qualification.sha256 = $sha | .evidence.qualification.bytes = $bytes'
assert_fails automated_qualification_mismatch "$(repack_case "$case_root")" "$ROOT/fail-physical-substitution"

case_root="$(make_case qualification-mixed-identity)"
qualification="$case_root/payload/evidence/automated-qualification-v1.json"
jq '.commit = ("b" * 40)' "$qualification" >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$qualification"
refresh_evidence_record "$case_root" evidence/automated-qualification-v1.json \
  '.evidence.qualification.sha256 = $sha | .evidence.qualification.bytes = $bytes'
assert_fails automated_qualification_mismatch "$(repack_case "$case_root")" "$ROOT/fail-qualification-identity"

case_root="$(make_case deployment-coherent-claim-tamper)"
deployment="$case_root/payload/evidence/automated-native-deployment-v1.json"
qualification="$case_root/payload/evidence/automated-qualification-v1.json"
jq '.environment.physicalDeviceObserved = true' "$deployment" >"$case_root/deployment.next"
mv "$case_root/deployment.next" "$deployment"
deployment_sha="$(sha256sum "$deployment" | awk '{print $1}')"
deployment_bytes="$(stat -c '%s' "$deployment")"
jq --arg sha "$deployment_sha" --argjson bytes "$deployment_bytes" \
  '.evidence.deployment.sha256 = $sha | .evidence.deployment.bytes = $bytes' \
  "$qualification" >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$qualification"
qualification_sha="$(sha256sum "$qualification" | awk '{print $1}')"
qualification_bytes="$(stat -c '%s' "$qualification")"
jq --arg deployment_sha "$deployment_sha" --argjson deployment_bytes "$deployment_bytes" \
  --arg qualification_sha "$qualification_sha" --argjson qualification_bytes "$qualification_bytes" '
    .evidence.deployment.sha256 = $deployment_sha
    | .evidence.deployment.bytes = $deployment_bytes
    | .evidence.qualification.sha256 = $qualification_sha
    | .evidence.qualification.bytes = $qualification_bytes
  ' "$case_root/payload/release-staging-manifest-v2.json" >"$case_root/stage.next"
mv "$case_root/stage.next" "$case_root/payload/release-staging-manifest-v2.json"
assert_fails deployment_evidence_mismatch "$(repack_case "$case_root")" "$ROOT/fail-deployment-claim"

case_root="$(make_case specialized-coherent-environment-mix)"
battery="$case_root/payload/evidence/termux-battery-emulated-evidence.json"
qualification="$case_root/payload/evidence/automated-qualification-v1.json"
jq '.environment.imageDigest = ("sha256:" + ("d" * 64))' "$battery" >"$case_root/battery.next"
mv "$case_root/battery.next" "$battery"
battery_sha="$(sha256sum "$battery" | awk '{print $1}')"
battery_bytes="$(stat -c '%s' "$battery")"
jq --arg sha "$battery_sha" --argjson bytes "$battery_bytes" \
  '.evidence.specialized[0].sha256 = $sha | .evidence.specialized[0].bytes = $bytes' \
  "$qualification" >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$qualification"
refresh_evidence_record "$case_root" evidence/termux-battery-emulated-evidence.json \
  '.evidence.specialized[0].sha256 = $sha | .evidence.specialized[0].bytes = $bytes'
refresh_qualification_record "$case_root"
assert_fails specialized_evidence_mismatch "$(repack_case "$case_root")" \
  "$ROOT/fail-specialized-environment"

case_root="$(make_case classifier-coherent-environment-mix)"
classifier="$case_root/payload/evidence/termux-observation-requirement-v3.json"
qualification="$case_root/payload/evidence/automated-qualification-v1.json"
jq '.emulation.imageDigest = ("sha256:" + ("e" * 64))' "$classifier" >"$case_root/classifier.next"
mv "$case_root/classifier.next" "$classifier"
classifier_sha="$(sha256sum "$classifier" | awk '{print $1}')"
classifier_bytes="$(stat -c '%s' "$classifier")"
jq --arg sha "$classifier_sha" --argjson bytes "$classifier_bytes" \
  '.evidence.classifier.sha256 = $sha | .evidence.classifier.bytes = $bytes' \
  "$qualification" >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$qualification"
refresh_evidence_record "$case_root" evidence/termux-observation-requirement-v3.json \
  '.evidence.classifier.sha256 = $sha | .evidence.classifier.bytes = $bytes'
refresh_qualification_record "$case_root"
assert_fails observation_requirement_mismatch "$(repack_case "$case_root")" \
  "$ROOT/fail-classifier-environment"

case_root="$(make_case deployment-coherent-environment-mix)"
deployment="$case_root/payload/evidence/automated-native-deployment-v1.json"
qualification="$case_root/payload/evidence/automated-qualification-v1.json"
jq '.environment.rootfsDigest = ("sha256:" + ("f" * 64))' \
  "$deployment" >"$case_root/deployment.next"
mv "$case_root/deployment.next" "$deployment"
deployment_sha="$(sha256sum "$deployment" | awk '{print $1}')"
deployment_bytes="$(stat -c '%s' "$deployment")"
jq --arg sha "$deployment_sha" --argjson bytes "$deployment_bytes" '
  .evidence.deployment.sha256 = $sha
  | .evidence.deployment.bytes = $bytes
  | .environment.rootfsUserland.digest = ("sha256:" + ("f" * 64))
' "$qualification" >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$qualification"
refresh_evidence_record "$case_root" evidence/automated-native-deployment-v1.json \
  '.evidence.deployment.sha256 = $sha | .evidence.deployment.bytes = $bytes'
refresh_qualification_record "$case_root"
assert_fails deployment_evidence_mismatch "$(repack_case "$case_root")" \
  "$ROOT/fail-deployment-environment"

case_root="$(make_case qualification-coherent-linker-mix)"
qualification="$case_root/payload/evidence/automated-qualification-v1.json"
jq '.environment.androidRuntime.linkerSha256 = ("b" * 64)' \
  "$qualification" >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$qualification"
refresh_qualification_record "$case_root"
assert_fails automated_qualification_mismatch "$(repack_case "$case_root")" \
  "$ROOT/fail-qualification-linker"

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
  "$case_root/payload/release-staging-manifest-v2.json" >"$case_root/stage.next"
mv "$case_root/stage.next" "$case_root/payload/release-staging-manifest-v2.json"
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
[[ "$(sha256sum "$ROOT/result-snapshot-race/$BUNDLE_NAME/assets/$STAGE_NAME" | awk '{print $1}')" == "$race_stage_sha" ]] \
  || fail_test 'source replacement changed the validated stage snapshot'

case_root="$(make_case publication-bundle-race)"
race_stage="$(repack_case "$case_root")"
race_stage_sha="$(sha256sum "$race_stage" | awk '{print $1}')"
race_output="$ROOT/result-publication-bundle-race"
race_control="$ROOT/publication-bundle-race-control"
mkdir -m 700 "$race_output" "$race_control" "$ROOT/racing-file"
cat >"$ROOT/racing-file/file" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
if [[ ! -e "$PREP_TEST_CONTROL/ready" ]]; then
  : >"$PREP_TEST_CONTROL/ready"
  for _ in $(seq 1 400); do
    [[ -e "$PREP_TEST_CONTROL/continue" ]] && break
    sleep 0.01
  done
  [[ -e "$PREP_TEST_CONTROL/continue" ]]
fi
exec "$PREP_TEST_BASE_FILE" "$@"
EOF
chmod 700 "$ROOT/racing-file/file"
PREP_TEST_CONTROL="$race_control" \
PREP_TEST_BASE_FILE="$ROOT/fake-bin/file" \
PREP_TEST_PATH="$ROOT/racing-file:$ROOT/fake-bin:$REAL_PATH" \
  run_prepare "$race_stage" "$race_output" "$race_stage_sha" \
  >"$ROOT/bundle-race.stdout" 2>"$ROOT/bundle-race.stderr" &
race_pid=$!
for _ in $(seq 1 400); do
  [[ -e "$race_control/ready" ]] && break
  sleep 0.01
done
[[ -e "$race_control/ready" ]] || fail_test 'bundle race did not reach private validation'
mkdir -m 700 "$race_output/$BUNDLE_NAME"
printf '%s\n' concurrent-owner >"$race_output/$BUNDLE_NAME/owner"
foreign_identity="$(stat -c '%d:%i' "$race_output/$BUNDLE_NAME")"
: >"$race_control/continue"
if wait "$race_pid"; then
  fail_test 'concurrent bundle owner was overwritten'
fi
grep -Fq bundle_publication_conflict "$ROOT/bundle-race.stderr" \
  || fail_test 'bundle no-replace race did not fail with the expected code'
[[ "$(stat -c '%d:%i' "$race_output/$BUNDLE_NAME")" == "$foreign_identity" \
  && "$(<"$race_output/$BUNDLE_NAME/owner")" == concurrent-owner ]] \
  || fail_test 'atomic bundle publication changed a concurrent owner'
[[ ! -e "$race_output/$BUNDLE_NAME/assets" \
  && ! -e "$race_output/$BUNDLE_NAME/$RECEIPT_NAME" ]] \
  || fail_test 'bundle conflict exposed a partial publication'
[[ -z "$(find "$race_output" -maxdepth 1 -name '.release-publication-assets.*' -print -quit)" ]] \
  || fail_test 'bundle conflict leaked private staging state'

case_root="$(make_case private-bundle-tamper)"
race_stage="$(repack_case "$case_root")"
tamper_output="$ROOT/result-private-bundle-tamper"
tamper_control="$ROOT/private-bundle-tamper-control"
mkdir -m 700 "$tamper_control" "$ROOT/racing-python-bundle"
cat >"$ROOT/racing-python-bundle/python3" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
count=0
[[ ! -e "$PREP_TEST_CONTROL/count" ]] || count="$(<"$PREP_TEST_CONTROL/count")"
count=$((count + 1))
printf '%s\n' "$count" >"$PREP_TEST_CONTROL/count"
if [[ "$count" == 2 ]]; then
  target="$(find "$PREP_TEST_OUTPUT_PARENT" -path '*/bundle/assets/SHA256SUMS' -type f -print -quit)"
  [[ -n "$target" ]]
  printf 'concurrent private mutation\n' >>"$target"
fi
exec "$PREP_REAL_PYTHON3" "$@"
EOF
chmod 700 "$ROOT/racing-python-bundle/python3"
PREP_REAL_PYTHON3="$REAL_PYTHON3" \
PREP_TEST_CONTROL="$tamper_control" \
PREP_TEST_OUTPUT_PARENT="$tamper_output" \
PREP_TEST_PATH="$ROOT/racing-python-bundle:$ROOT/fake-bin:$REAL_PATH" \
  assert_fails private_bundle_validation_failed "$race_stage" "$tamper_output"
[[ ! -e "$tamper_output/$BUNDLE_NAME" && ! -L "$tamper_output/$BUNDLE_NAME" ]] \
  || fail_test 'private bundle mutation exposed a completion bundle'
[[ -z "$(find "$tamper_output" -maxdepth 1 -name '.release-publication-assets.*' -print -quit)" ]] \
  || fail_test 'private bundle mutation leaked staging state'

printf 'Release publication asset preparation tests passed\n'
