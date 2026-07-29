#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

ROOT="$(mktemp -d)"
trap 'rm -rf -- "$ROOT"' EXIT INT TERM
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/stage_release_assets.sh"
PACKAGER="$REPO_ROOT/scripts/package_automated_qualification.sh"
POLICY="$REPO_ROOT/docs/release-qualification-policy-v1.json"
SCENARIO_SET="$REPO_ROOT/docs/automated-native-deployment-scenarios-v1.json"
SCHEMA="$REPO_ROOT/docs/release-staging-manifest-schema-v2.json"
REAL_PATH="$PATH"
REAL_CP="$(command -v cp)"
REAL_MV="$(command -v mv)"
REAL_PYTHON3="$(command -v python3)"
COMMIT=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
VERSION=0.6.0
CI_RUN_ID=4101
SECURITY_RUN_ID=4102
ANDROID_RUN_ID=4103
QUALIFICATION_RUN_ID=4104
OUTPUT_NAME="termux-mcp-server-v${VERSION}-release-stage-${COMMIT:0:12}.tar"

fail_test() {
  printf 'FAIL: %s\n' "$1" >&2
  exit 1
}

assert_fails() {
  local expected_code="$1"
  shift
  if "$@" >"$ROOT/last.stdout" 2>"$ROOT/last.stderr"; then
    fail_test "command unexpectedly succeeded; expected $expected_code"
  fi
  grep -Fq "$expected_code" "$ROOT/last.stderr" \
    || fail_test "expected error code $expected_code was absent"
}

mkdir -m 700 "$ROOT/fake-bin" "$ROOT/outputs"
cat >"$ROOT/fake-bin/file" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
target="${*: -1}"
if grep -Fq wrong-arch "$target"; then
  printf '%s\n' 'ELF 64-bit LSB executable, x86-64, for GNU/Linux'
else
  printf '%s\n' 'ELF 64-bit LSB pie executable, ARM aarch64, for Android 24'
fi
EOF
chmod 700 "$ROOT/fake-bin/file"

postures=(
  default
  mcp-runtime
  android-battery-status
  android-volume-status
  android-volume-control
  command-execution
  full-suite
)
bundle_names=(
  default
  mcp-runtime
  android-battery-status
  android-volume-status
  android-volume-control
  command-execution
  full-suite
)
artifact_names=(
  termux-mcp-server-aarch64-linux-android-default
  termux-mcp-server-aarch64-linux-android-mcp-runtime
  termux-mcp-server-aarch64-linux-android-android-battery-status
  termux-mcp-server-aarch64-linux-android-android-volume-status
  termux-mcp-server-aarch64-linux-android-android-volume-control
  termux-mcp-server-aarch64-linux-android-command-execution
  termux-mcp-server-aarch64-linux-android-full-suite
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

make_bundle() {
  local root="$1" index="$2" digest bytes
  mkdir -p "$root"
  printf '%s\n' '#!/system/bin/sh' "# posture-${postures[$index]}" 'exit 0' >"$root/termux-mcp-server"
  chmod 700 "$root/termux-mcp-server"
  digest="$(sha256sum "$root/termux-mcp-server" | awk '{print $1}')"
  bytes="$(stat -c '%s' "$root/termux-mcp-server")"
  printf '%s  termux-mcp-server\n' "$digest" >"$root/SHA256SUMS"
  jq -n \
    --arg artifact_name "${artifact_names[$index]}" \
    --arg posture "${postures[$index]}" \
    --argjson features "${features_json[$index]}" \
    --arg digest "$digest" --argjson bytes "$bytes" \
    --arg commit "$COMMIT" --arg run_id "$ANDROID_RUN_ID" --arg version "$VERSION" '
      {
        schemaVersion:1,
        repository:"CyberBASSLord-666/termux-mcp-edge",
        commit:$commit,
        workflowRunId:$run_id,
        artifactName:$artifact_name,
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
    ' >"$root/artifact-manifest.json"
  chmod 600 "$root/SHA256SUMS" "$root/artifact-manifest.json"
}

refresh_bundle_binary_identity() {
  local root="$1" digest bytes temporary
  digest="$(sha256sum "$root/termux-mcp-server" | awk '{print $1}')"
  bytes="$(stat -c '%s' "$root/termux-mcp-server")"
  printf '%s  termux-mcp-server\n' "$digest" >"$root/SHA256SUMS"
  temporary="$root/artifact-manifest.json.next"
  jq --arg digest "$digest" --argjson bytes "$bytes" \
    '.sha256 = $digest | .bytes = $bytes' "$root/artifact-manifest.json" >"$temporary"
  mv "$temporary" "$root/artifact-manifest.json"
}

make_specialized_evidence() {
  local output="$1" schema_version="$2" gate_version="$3" artifact_index="$4" mode="$5"
  local related_index="${6:--1}" artifact_sha artifact_bytes related_sha="" related_bytes=0
  artifact_sha="$(jq -r .sha256 "$BASE/bundles/${bundle_names[$artifact_index]}/artifact-manifest.json")"
  artifact_bytes="$(jq -r .bytes "$BASE/bundles/${bundle_names[$artifact_index]}/artifact-manifest.json")"
  if ((related_index >= 0)); then
    related_sha="$(jq -r .sha256 "$BASE/bundles/${bundle_names[$related_index]}/artifact-manifest.json")"
    related_bytes="$(jq -r .bytes "$BASE/bundles/${bundle_names[$related_index]}/artifact-manifest.json")"
  fi
  jq -n \
    --argjson schema "$schema_version" --arg gate "$gate_version" --arg mode "$mode" \
    --arg commit "$COMMIT" --arg version "$VERSION" \
    --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
    --arg artifact_sha "$artifact_sha" --argjson artifact_bytes "$artifact_bytes" \
    --arg related_sha "$related_sha" --argjson related_bytes "$related_bytes" '
      {
        schemaVersion:$schema,
        gateVersion:$gate,
        status:"pass",
        failureCode:null,
        releaseQualificationEligible:false,
        startedAt:"2026-07-22T00:00:00Z",
        completedAt:"2026-07-22T00:01:00Z",
        candidate: (
          {
            commit:$commit,
            version:$version,
            ciRunId:$ci,
            securityRunId:$security,
            androidRunId:$android,
            artifact:{sha256:$artifact_sha,bytes:$artifact_bytes}
          }
          + (if $mode == "volume-control" then
               {incompatibleArtifact:{sha256:$related_sha,bytes:$related_bytes}}
             elif $mode == "command" then
               {defaultArtifact:{sha256:$related_sha,bytes:$related_bytes}}
             else {} end)
        ),
        environment:{
          architecture:"aarch64",
          executionMode:"official-termux-docker-native-arm64",
          image:"termux/termux-docker:aarch64",
          imageDigest:("sha256:" + ("c" * 64)),
          rootfsImageId:("sha256:" + ("b" * 64)),
          runtimeImageDigest:("sha256:" + ("d" * 64)),
          androidLinker:true
        },
        validation:(
          if $mode == "battery" then {
            status:"pass",requests:29,exactArtifact:true,compileGate:true,
            runtimeDefaultDisabled:true,disabledDiscovery:true,fixedProgram:true,
            fixedWorkingDirectory:true,noArguments:true,inheritedEnvironmentCleared:true,
            normalizedAllowlist:true,sensitiveFieldsRedacted:true,boundedOutput:true,
            immediateOverflowTermination:true,processGroupIsolation:true,
            pipeHoldingDescendantCleanup:true,callerCancellationCleanup:true,
            boundedSupervisorCleanup:true,stableErrors:true,androidDeviceControlDisabled:true,
            commandExecutionDisabled:true,highImpactToolsDisabled:true
          } elif $mode == "volume" then {
            status:"pass",requests:29,exactArtifact:true,compileGate:true,
            runtimeDefaultDisabled:true,disabledDiscovery:true,fixedProgram:true,
            fixedWorkingDirectory:true,noArguments:true,inheritedEnvironmentCleared:true,
            normalizedAllowlist:true,canonicalStreamOrdering:true,unrecognizedFieldsRejected:true,
            boundedOutput:true,immediateOverflowTermination:true,processGroupIsolation:true,
            pipeHoldingDescendantCleanup:true,callerCancellationCleanup:true,
            boundedSupervisorCleanup:true,stableErrors:true,androidDeviceControlDisabled:true,
            commandExecutionDisabled:true,highImpactToolsDisabled:true
          } elif $mode == "volume-control" then {
            status:"pass",requests:29,exactArtifact:true,compileGate:true,
            runtimeDefaultDisabled:true,disabledDiscovery:true,staticTokenRequired:true,
            capabilityKeyRequired:true,closedInputSchema:true,previewNoMutation:true,
            previewDoesNotConsumeGrant:true,headerContextEnforced:true,exactGrantBinding:true,
            singleUseReplay:true,freshMaximum:true,fixedProgram:true,exactTwoArguments:true,
            fixedWorkingDirectory:true,inheritedEnvironmentCleared:true,nullStdin:true,
            nonQueueingConcurrency:true,mutationVerified:true,rollbackConfirmed:true,
            rollbackUnconfirmed:true,cancellationIndependentRecovery:true,boundedSupervisor:true,
            auditCounters:true,redactedResponses:true,arbitraryCommandExecutionDisabled:true,
            broaderAndroidControlDisabled:true,longObservationRequired:false
          } else {
            status:"pass",requests:29,exactArtifact:true,compileGate:true,
            runtimeDefaultDisabled:true,disabledDiscovery:true,fixedCurrentExecutable:true,
            wrongExecutableNameFailsClosed:true,wrongExecutableNameRejectedBeforeServing:true,
            runningInodePinned:true,workingDirectoryDescriptorPinned:true,fixedArgvProfiles:true,
            closedInputSchema:true,overrideFieldsRejected:true,unknownProfileRejected:true,
            fixedWorkingDirectory:true,inheritedEnvironmentCleared:true,nullStdin:true,
            boundedOutput:true,utf8Output:true,versionProfile:true,helpProfile:true,
            boundaryProfile:true,auditCounters:true,stableErrors:true,
            arbitraryCommandExecutionDisabled:true,androidDeviceControlDisabled:true,
            highImpactToolsDisabled:true,longObservationRequired:false
          } end
        )
      }
    ' >"$output"
}

BASE="$ROOT/base"
mkdir -p "$BASE/bundles" "$BASE/emulated"
for index in "${!postures[@]}"; do
  make_bundle "$BASE/bundles/${bundle_names[$index]}" "$index"
done
printf '%s\n' 'MIT License' 'fixture license text' >"$BASE/LICENSE"

bundle_sha() { jq -r .sha256 "$BASE/bundles/$1/artifact-manifest.json"; }
bundle_bytes() { jq -r .bytes "$BASE/bundles/$1/artifact-manifest.json"; }
full_manifest_sha="$(sha256sum "$BASE/bundles/full-suite/artifact-manifest.json" | awk '{print $1}')"

jq -n \
  --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg default_sha "$(bundle_sha default)" --argjson default_bytes "$(bundle_bytes default)" \
  --arg mcp_sha "$(bundle_sha mcp-runtime)" --argjson mcp_bytes "$(bundle_bytes mcp-runtime)" \
  --arg volume_control_sha "$(bundle_sha android-volume-control)" --argjson volume_control_bytes "$(bundle_bytes android-volume-control)" \
  --arg full_suite_sha "$(bundle_sha full-suite)" --argjson full_suite_bytes "$(bundle_bytes full-suite)" \
  --arg full_manifest_sha "$full_manifest_sha" '
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
        androidVolumeControlArtifact:{sha256:$volume_control_sha,bytes:$volume_control_bytes},
        fullSuiteArtifact:{
          sha256:$full_suite_sha,bytes:$full_suite_bytes,manifestSha256:$full_manifest_sha,
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
      runtimeValidation:{status:"pass",reportSha256:("d" * 64),resultCount:20,phases:{preflight:"pass",runtime:"pass",deployment:"not_run"}},
      aggregateValidation:{
        status:"pass",requests:20,
        defaultDisabled:{toolCount:17,exactToolOrder:true,optionalFeaturesCompiled:true,optionalToolsHidden:true,runtimeFlagsOmitted:true},
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
        safeRootAncestorIdentityPinned:true,copyFileMutationDisabled:true,
        highImpactDisabled:true,longObservationRequired:false
      }
    }
  ' >"$BASE/emulated/termux-native-aggregate-evidence-v4.json"

make_specialized_evidence "$BASE/emulated/termux-battery-emulated-evidence.json" 3 3 2 battery
make_specialized_evidence "$BASE/emulated/termux-volume-emulated-evidence.json" 2 2 3 volume
make_specialized_evidence "$BASE/emulated/termux-volume-control-emulated-evidence.json" 2 2 4 volume-control 3
make_specialized_evidence "$BASE/emulated/termux-command-emulated-evidence.json" 3 3 5 command 0

aggregate_sha="$(sha256sum "$BASE/emulated/termux-native-aggregate-evidence-v4.json" | awk '{print $1}')"
jq -n \
  --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg full_suite_sha "$(bundle_sha full-suite)" --arg full_manifest_sha "$full_manifest_sha" \
  --arg aggregate_sha "$aggregate_sha" '
    {
      schemaVersion:3,classifierVersion:"3",status:"pass",failureCode:null,releaseQualificationEligible:false,
      createdAt:"2026-07-22T00:03:00Z",evidenceMode:"automated_release_qualification",
      reasonCode:"automated_native_termux_evidence_required",inheritanceCandidate:false,
      source:{commit:("f" * 40)},
      candidate:{
        commit:$commit,version:$version,ciRunId:$ci,securityRunId:$security,androidRunId:$android,
        fullSuiteArtifactSha256:$full_suite_sha,fullSuiteManifestSha256:$full_manifest_sha
      },
      emulation:{
        reportSha256:$aggregate_sha,executionMode:"official-termux-docker-native-arm64",
        imageDigest:("sha256:" + ("c" * 64)),status:"pass",samples:64
      },
      claimBoundary:{
        physicalDeviceObserved:false,androidFrameworkObserved:false,
        sustainedPhysicalSoak:false,physicalCertification:"not_run"
      },
      protectedInputComparison:{runtimeAndDeploymentInputsUnchanged:false,cargoAndDependencyInputsUnchangedExceptRootVersion:false},
      changedInputClasses:["runtime_or_deployment","cargo_or_dependency"],
      nextGate:"assemble_automated_release_qualification"
    }
  ' >"$BASE/emulated/termux-observation-requirement-v3.json"

scenario_sha="$(sha256sum "$SCENARIO_SET" | awk '{print $1}')"
jq -n \
  --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg full_suite_sha "$(bundle_sha full-suite)" \
  --argjson full_suite_bytes "$(bundle_bytes full-suite)" \
  --arg full_manifest_sha "$full_manifest_sha" --arg scenario_sha "$scenario_sha" '
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
          posture:"full-suite",features:["full-suite"],sha256:$full_suite_sha,
          manifestSha256:$full_manifest_sha,bytes:$full_suite_bytes,
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
  ' >"$BASE/automated-native-deployment-v1.json"
chmod 600 "$BASE/automated-native-deployment-v1.json" "$BASE/emulated"/*.json
RUNTIME_DIR="$BASE/runtime"
mkdir -m 700 "$RUNTIME_DIR"
RUNTIME_ARCHIVE="$RUNTIME_DIR/termux-qualified-runtime-image-v1.tar.gz"
RUNTIME_PACKAGE_LOCK="$RUNTIME_DIR/termux-runtime-package-lock-v1.json"
RUNTIME_SNAPSHOT="$RUNTIME_DIR/termux-runtime-snapshot-v1.json"
RUNTIME_REPLAY="$RUNTIME_DIR/termux-runtime-snapshot-replay-v1.json"
printf 'retained qualified runtime archive fixture\n' >"$RUNTIME_ARCHIVE"
chmod 600 "$RUNTIME_ARCHIVE"
runtime_archive_sha="$(sha256sum "$RUNTIME_ARCHIVE" | awk '{print $1}')"
runtime_archive_bytes="$(stat -c '%s' "$RUNTIME_ARCHIVE")"
jq -n \
  --arg commit "$COMMIT" --arg android "$ANDROID_RUN_ID" '
  {
    schemaVersion:1,lockVersion:"1",
    repository:"CyberBASSLord-666/termux-mcp-edge",
    commit:$commit,androidRunId:$android,
    base:{
      image:"termux/termux-docker:aarch64",
      digest:("sha256:" + ("c" * 64)),
      imageId:("sha256:" + ("b" * 64))
    },
    requestedPackages:["file","jq","python","termux-services"],
    resolution:{
      resolver:"termux-apt-download-only",
      repositoryMetadataAuthenticated:true,
      packageBytesFrozenBeforeBuild:true,
      finalImageBuildNetwork:"none"
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
' >"$RUNTIME_PACKAGE_LOCK"
chmod 600 "$RUNTIME_PACKAGE_LOCK"
runtime_package_lock_sha="$(sha256sum "$RUNTIME_PACKAGE_LOCK" | awk '{print $1}')"
runtime_package_lock_bytes="$(stat -c '%s' "$RUNTIME_PACKAGE_LOCK")"
runtime_inventory_sha="$(
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
  --arg lock_sha "$runtime_package_lock_sha" \
  --argjson lock_bytes "$runtime_package_lock_bytes" \
  --arg inventory_sha "$runtime_inventory_sha" \
  --arg archive_sha "$runtime_archive_sha" \
  --argjson archive_bytes "$runtime_archive_bytes" '
  {
    schemaVersion:1,snapshotVersion:"1",status:"pass",failureCode:null,
    releaseQualificationEligible:false,
    repository:"CyberBASSLord-666/termux-mcp-edge",
    commit:$commit,androidRunId:$android,
    base:{
      image:"termux/termux-docker:aarch64",
      digest:("sha256:" + ("c" * 64)),
      imageId:("sha256:" + ("b" * 64))
    },
    runtimeImageId:("sha256:" + ("d" * 64)),
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
runtime_snapshot_sha="$(sha256sum "$RUNTIME_SNAPSHOT" | awk '{print $1}')"
runtime_snapshot_bytes="$(stat -c '%s' "$RUNTIME_SNAPSHOT")"
jq -n \
  --arg commit "$COMMIT" \
  --arg snapshot_sha "$runtime_snapshot_sha" \
  --argjson snapshot_bytes "$runtime_snapshot_bytes" \
  --arg archive_sha "$runtime_archive_sha" \
  --argjson archive_bytes "$runtime_archive_bytes" \
  --arg lock_sha "$runtime_package_lock_sha" \
  --argjson lock_bytes "$runtime_package_lock_bytes" \
  --arg inventory_sha "$runtime_inventory_sha" '
  {
    schemaVersion:1,replayVersion:"1",status:"pass",failureCode:null,
    releaseQualificationEligible:false,
    repository:"CyberBASSLord-666/termux-mcp-edge",
    commit:$commit,runtimeImageId:("sha256:" + ("d" * 64)),
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
    androidLinker:{path:"/system/bin/linker64",sha256:("a"*64),bytes:4096},
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

mkdir -m 700 "$BASE/qualification-parent"
PATH="$ROOT/fake-bin:$REAL_PATH" bash "$PACKAGER" \
  --policy "$POLICY" \
  --scenario-set "$SCENARIO_SET" \
  --aggregate-evidence "$BASE/emulated/termux-native-aggregate-evidence-v4.json" \
  --deployment-evidence "$BASE/automated-native-deployment-v1.json" \
  --classifier-evidence "$BASE/emulated/termux-observation-requirement-v3.json" \
  --battery-evidence "$BASE/emulated/termux-battery-emulated-evidence.json" \
  --volume-evidence "$BASE/emulated/termux-volume-emulated-evidence.json" \
  --volume-control-evidence "$BASE/emulated/termux-volume-control-emulated-evidence.json" \
  --command-evidence "$BASE/emulated/termux-command-emulated-evidence.json" \
  --runtime-archive "$RUNTIME_ARCHIVE" \
  --runtime-package-lock "$RUNTIME_PACKAGE_LOCK" \
  --runtime-snapshot "$RUNTIME_SNAPSHOT" \
  --runtime-replay "$RUNTIME_REPLAY" \
  --default-dir "$BASE/bundles/default" \
  --mcp-runtime-dir "$BASE/bundles/mcp-runtime" \
  --battery-dir "$BASE/bundles/android-battery-status" \
  --volume-dir "$BASE/bundles/android-volume-status" \
  --volume-control-dir "$BASE/bundles/android-volume-control" \
  --command-dir "$BASE/bundles/command-execution" \
  --full-suite-dir "$BASE/bundles/full-suite" \
  --qualification-run-id "$QUALIFICATION_RUN_ID" \
  --output "$BASE/qualification-parent/automated-qualification-v1.json" >/dev/null
cp "$BASE/qualification-parent/automated-qualification-v1.json" \
  "$BASE/automated-qualification-v1.json"
chmod 600 "$BASE/automated-qualification-v1.json"
rm -rf "$BASE/qualification-parent"

make_case() {
  local name="$1" destination
  destination="$ROOT/cases/$name"
  mkdir -p "$ROOT/cases"
  cp -a "$BASE" "$destination"
  printf '%s\n' "$destination"
}

run_stage() {
  local case_root="$1" output_dir="$2" output_name="${3:-$OUTPUT_NAME}"
  local selected_path="${STAGE_TEST_PATH:-$ROOT/fake-bin:$REAL_PATH}"
  mkdir -p "$output_dir"
  PATH="$selected_path" bash "$SCRIPT" \
    --default-dir "$case_root/bundles/default" \
    --mcp-runtime-dir "$case_root/bundles/mcp-runtime" \
    --android-battery-status-dir "$case_root/bundles/android-battery-status" \
    --android-volume-status-dir "$case_root/bundles/android-volume-status" \
    --android-volume-control-dir "$case_root/bundles/android-volume-control" \
    --command-execution-dir "$case_root/bundles/command-execution" \
    --full-suite-dir "$case_root/bundles/full-suite" \
    --emulated-evidence-dir "$case_root/emulated" \
    --deployment-evidence "$case_root/automated-native-deployment-v1.json" \
    --automated-qualification "$case_root/automated-qualification-v1.json" \
    --runtime-archive "$case_root/runtime/termux-qualified-runtime-image-v1.tar.gz" \
    --runtime-package-lock "$case_root/runtime/termux-runtime-package-lock-v1.json" \
    --runtime-snapshot "$case_root/runtime/termux-runtime-snapshot-v1.json" \
    --runtime-replay "$case_root/runtime/termux-runtime-snapshot-replay-v1.json" \
    --license "$case_root/LICENSE" \
    --repository CyberBASSLord-666/termux-mcp-edge \
    --commit "$COMMIT" \
    --version "$VERSION" \
    --ci-run-id "$CI_RUN_ID" \
    --security-run-id "$SECURITY_RUN_ID" \
    --android-run-id "$ANDROID_RUN_ID" \
    --qualification-run-id "$QUALIFICATION_RUN_ID" \
    --output "$output_dir/$output_name"
}

bash -n "$SCRIPT"
grep -Fq 'runtime_limit=1610612736' "$SCRIPT" \
  || fail_test 'runtime archive snapshot cap changed'
grep -Fq 'require_regular_file "$RUNTIME_ARCHIVE" 1610612736 runtime_archive_invalid' "$SCRIPT" \
  || fail_test 'runtime archive validation cap changed'
grep -Fq '((archive_size > 0 && archive_size <= 2147483647))' "$SCRIPT" \
  || fail_test 'staged tar size boundary changed'
jq -e '
  .type == "object"
  and .additionalProperties == false
  and .properties.publicationState.const == "staged_not_released"
  and .properties.releaseEligible.const == false
  and .properties.qualificationClass.const == "official_termux_native_automated_v1"
  and .properties.artifacts.minItems == 7
  and .properties.artifacts.maxItems == 7
  and (.properties.evidence.required | sort) == [
    "aggregate","classifier","deployment","qualification","runtime","specialized"
  ]
  and .properties.evidence.properties.runtime.additionalProperties == false
  and (.properties.evidence.properties.runtime.required | sort)
    == ["archive","packageLock","replay","snapshot"]
  and ."$defs".runtimeArchiveRecord.properties.bytes.maximum == 1610612736
  and ."$defs".runtimeJsonRecord.properties.bytes.maximum == 16777216
  and .properties.evidence.properties.specialized.minItems == 4
  and .properties.evidence.properties.specialized.maxItems == 4
' "$SCHEMA" >/dev/null

SUCCESS_CASE="$(make_case success)"
run_stage "$SUCCESS_CASE" "$ROOT/outputs/first" >"$ROOT/success.stdout"
FIRST_TAR="$ROOT/outputs/first/$OUTPUT_NAME"
[[ -f "$FIRST_TAR" && ! -L "$FIRST_TAR" ]] || fail_test 'staging archive missing'
grep -Fq 'publicationState=staged_not_released releaseEligible=false' "$ROOT/success.stdout" \
  || fail_test 'staging result did not remain non-published'
[[ -z "$(find "$ROOT/outputs/first" -maxdepth 1 -name '*.staging.*' -print -quit)" ]] \
  || fail_test 'successful staging leaked temporary state'

SECOND_CASE="$(make_case deterministic)"
run_stage "$SECOND_CASE" "$ROOT/outputs/second" >/dev/null
SECOND_TAR="$ROOT/outputs/second/$OUTPUT_NAME"
[[ "$(sha256sum "$FIRST_TAR" | awk '{print $1}')" == "$(sha256sum "$SECOND_TAR" | awk '{print $1}')" ]] \
  || fail_test 'identical inputs did not produce an identical tar digest'

mkdir "$ROOT/extracted"
tar -xf "$FIRST_TAR" -C "$ROOT/extracted"
[[ "$(find "$ROOT/extracted" -type f | wc -l)" == 36 ]] || fail_test 'staging archive file set is not exact'
[[ -z "$(find "$ROOT/extracted" -type l -print -quit)" ]] || fail_test 'staging archive contains a link'
[[ "$(stat -c '%a' "$ROOT/extracted")" == 755 ]] || fail_test 'archive root mode is not normalized'
[[ "$(stat -c '%a' "$ROOT/extracted/evidence")" == 755 ]] || fail_test 'evidence directory mode is not normalized'
[[ "$(stat -c '%a' "$ROOT/extracted/evidence/runtime")" == 755 ]] \
  || fail_test 'runtime evidence directory mode is not normalized'

for index in "${!postures[@]}"; do
  release_name="termux-mcp-server-v${VERSION}-aarch64-linux-android-${postures[$index]}"
  [[ "$(stat -c '%a' "$ROOT/extracted/$release_name")" == 755 ]] || fail_test 'binary mode is not normalized'
  [[ "$(stat -c '%a' "$ROOT/extracted/$release_name.sha256")" == 644 ]] || fail_test 'checksum mode is not normalized'
  cmp -s "$SUCCESS_CASE/bundles/${bundle_names[$index]}/termux-mcp-server" "$ROOT/extracted/$release_name" \
    || fail_test 'qualified binary bytes changed during staging'
  cmp -s "$SUCCESS_CASE/bundles/${bundle_names[$index]}/artifact-manifest.json" \
    "$ROOT/extracted/$release_name.workflow-manifest.json" \
    || fail_test 'workflow manifest bytes changed during staging'
  (cd "$ROOT/extracted" && sha256sum -c "$release_name.sha256" >/dev/null) \
    || fail_test 'per-binary checksum failed'
done
(cd "$ROOT/extracted" && sha256sum -c SHA256SUMS >/dev/null) || fail_test 'combined checksum failed'

cmp -s "$SUCCESS_CASE/emulated/termux-native-aggregate-evidence-v4.json" \
  "$ROOT/extracted/evidence/termux-native-aggregate-evidence-v4.json" \
  || fail_test 'aggregate evidence bytes changed'
cmp -s "$SUCCESS_CASE/automated-native-deployment-v1.json" \
  "$ROOT/extracted/evidence/automated-native-deployment-v1.json" \
  || fail_test 'deployment evidence bytes changed'
cmp -s "$SUCCESS_CASE/automated-qualification-v1.json" \
  "$ROOT/extracted/evidence/automated-qualification-v1.json" \
  || fail_test 'automated qualification bytes changed'
for runtime_name in \
  termux-qualified-runtime-image-v1.tar.gz \
  termux-runtime-package-lock-v1.json \
  termux-runtime-snapshot-v1.json \
  termux-runtime-snapshot-replay-v1.json
do
  cmp -s "$SUCCESS_CASE/runtime/$runtime_name" \
    "$ROOT/extracted/evidence/runtime/$runtime_name" \
    || fail_test "retained runtime bytes changed: $runtime_name"
  [[ "$(stat -c '%a' "$ROOT/extracted/evidence/runtime/$runtime_name")" == 644 ]] \
    || fail_test "retained runtime mode is not normalized: $runtime_name"
done
cmp -s "$SUCCESS_CASE/LICENSE" "$ROOT/extracted/LICENSE" || fail_test 'license bytes changed'

jq -e \
  --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg qualification "$QUALIFICATION_RUN_ID" \
  --arg runtime_archive_sha "$(sha256sum "$SUCCESS_CASE/runtime/termux-qualified-runtime-image-v1.tar.gz" | awk '{print $1}')" \
  --argjson runtime_archive_bytes "$(stat -c '%s' "$SUCCESS_CASE/runtime/termux-qualified-runtime-image-v1.tar.gz")" \
  --arg runtime_lock_sha "$(sha256sum "$SUCCESS_CASE/runtime/termux-runtime-package-lock-v1.json" | awk '{print $1}')" \
  --argjson runtime_lock_bytes "$(stat -c '%s' "$SUCCESS_CASE/runtime/termux-runtime-package-lock-v1.json")" \
  --arg runtime_snapshot_sha "$(sha256sum "$SUCCESS_CASE/runtime/termux-runtime-snapshot-v1.json" | awk '{print $1}')" \
  --argjson runtime_snapshot_bytes "$(stat -c '%s' "$SUCCESS_CASE/runtime/termux-runtime-snapshot-v1.json")" \
  --arg runtime_replay_sha "$(sha256sum "$SUCCESS_CASE/runtime/termux-runtime-snapshot-replay-v1.json" | awk '{print $1}')" \
  --argjson runtime_replay_bytes "$(stat -c '%s' "$SUCCESS_CASE/runtime/termux-runtime-snapshot-replay-v1.json")" '
    (keys == ["artifacts","checksums","claimBoundary","commit","evidence","license","publicationState","qualificationClass","releaseEligible","repository","schemaVersion","target","version","workflowRuns"])
    and .schemaVersion == 2
    and .publicationState == "staged_not_released"
    and .releaseEligible == false
    and .qualificationClass == "official_termux_native_automated_v1"
    and .claimBoundary == {
      physicalDeviceObserved:false,androidFrameworkObserved:false,
      sustainedPhysicalSoak:false,physicalCertification:"not_run"
    }
    and .repository == "CyberBASSLord-666/termux-mcp-edge"
    and .commit == $commit
    and .version == $version
    and .workflowRuns == {
      ci:$ci,
      security:$security,
      android:$android,
      qualification:$qualification
    }
    and (.artifacts | length == 7)
    and ([.artifacts[].posture] == ["default","mcp-runtime","android-battery-status","android-volume-status","android-volume-control","command-execution","full-suite"])
    and .evidence.aggregate.fileName == "evidence/termux-native-aggregate-evidence-v4.json"
    and .evidence.deployment.fileName == "evidence/automated-native-deployment-v1.json"
    and .evidence.classifier.fileName == "evidence/termux-observation-requirement-v3.json"
    and .evidence.qualification.fileName == "evidence/automated-qualification-v1.json"
    and (.evidence | keys == ["aggregate","classifier","deployment","qualification","runtime","specialized"])
    and (.evidence.runtime | keys == ["archive","packageLock","replay","snapshot"])
    and .evidence.runtime.archive == {
      fileName:"evidence/runtime/termux-qualified-runtime-image-v1.tar.gz",
      sha256:$runtime_archive_sha,
      bytes:$runtime_archive_bytes
    }
    and .evidence.runtime.packageLock == {
      fileName:"evidence/runtime/termux-runtime-package-lock-v1.json",
      sha256:$runtime_lock_sha,
      bytes:$runtime_lock_bytes
    }
    and .evidence.runtime.snapshot == {
      fileName:"evidence/runtime/termux-runtime-snapshot-v1.json",
      sha256:$runtime_snapshot_sha,
      bytes:$runtime_snapshot_bytes
    }
    and .evidence.runtime.replay == {
      fileName:"evidence/runtime/termux-runtime-snapshot-replay-v1.json",
      sha256:$runtime_replay_sha,
      bytes:$runtime_replay_bytes
    }
    and (.evidence.specialized | length == 4)
  ' "$ROOT/extracted/release-staging-manifest-v2.json" >/dev/null \
  || fail_test 'staging manifest content is invalid'

mapfile -t archive_members < <(tar -tf "$FIRST_TAR")
mapfile -t sorted_archive_members < <(printf '%s\n' "${archive_members[@]}" | sort)
[[ "$(printf '%s\n' "${archive_members[@]}")" == "$(printf '%s\n' "${sorted_archive_members[@]}")" ]] \
  || fail_test 'archive members are not deterministically sorted'
if LC_ALL=C TZ=XST8 tar --utc --full-time -tvf "$FIRST_TAR" \
  | awk '$4 != "1970-01-01" || $5 != "00:00:00" {exit 1}'; then
  :
else
  fail_test 'archive timestamps are not normalized'
fi

case_root="$(make_case bundle-link)"
rm "$case_root/bundles/default/SHA256SUMS"
ln -s termux-mcp-server "$case_root/bundles/default/SHA256SUMS"
assert_fails bundle_checksum_invalid run_stage "$case_root" "$ROOT/outputs/bundle-link"

case_root="$(make_case bundle-extra)"
printf 'unexpected\n' >"$case_root/bundles/default/extra"
assert_fails bundle_members_invalid run_stage "$case_root" "$ROOT/outputs/bundle-extra"

case_root="$(make_case checksum-mismatch)"
printf '%064d  termux-mcp-server\n' 0 >"$case_root/bundles/default/SHA256SUMS"
assert_fails bundle_checksum_mismatch run_stage "$case_root" "$ROOT/outputs/checksum-mismatch"

case_root="$(make_case manifest-commit)"
jq '.commit = ("b" * 40)' "$case_root/bundles/default/artifact-manifest.json" >"$case_root/manifest.next"
mv "$case_root/manifest.next" "$case_root/bundles/default/artifact-manifest.json"
assert_fails bundle_manifest_mismatch run_stage "$case_root" "$ROOT/outputs/manifest-commit"

case_root="$(make_case manifest-coherent-duplicate-key)"
manifest="$case_root/bundles/default/artifact-manifest.json"
sed '1s/^{/{"schemaVersion":1,/' "$manifest" >"$case_root/manifest.next"
mv "$case_root/manifest.next" "$manifest"
manifest_sha="$(sha256sum "$manifest" | awk '{print $1}')"
jq --arg sha "$manifest_sha" '.artifacts[0].manifestSha256 = $sha' \
  "$case_root/automated-qualification-v1.json" >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$case_root/automated-qualification-v1.json"
assert_fails automated_qualification_mismatch \
  run_stage "$case_root" "$ROOT/outputs/manifest-coherent-duplicate-key"

case_root="$(make_case wrong-architecture)"
printf '%s\n' '#!/system/bin/sh' '# wrong-arch' 'exit 0' >"$case_root/bundles/default/termux-mcp-server"
refresh_bundle_binary_identity "$case_root/bundles/default"
assert_fails bundle_binary_architecture_mismatch run_stage "$case_root" "$ROOT/outputs/wrong-architecture"

case_root="$(make_case duplicate-digest)"
cp "$case_root/bundles/default/termux-mcp-server" "$case_root/bundles/mcp-runtime/termux-mcp-server"
refresh_bundle_binary_identity "$case_root/bundles/mcp-runtime"
assert_fails bundle_posture_digests_not_distinct run_stage "$case_root" "$ROOT/outputs/duplicate-digest"

case_root="$(make_case evidence-extra)"
printf '{}\n' >"$case_root/emulated/termux-observation-inheritance.json"
assert_fails emulated_evidence_members_invalid run_stage "$case_root" "$ROOT/outputs/evidence-extra"

case_root="$(make_case aggregate-digest)"
jq '.candidate.fullSuiteArtifact.sha256 = ("0" * 64)' \
  "$case_root/emulated/termux-native-aggregate-evidence-v4.json" >"$case_root/aggregate.next"
mv "$case_root/aggregate.next" "$case_root/emulated/termux-native-aggregate-evidence-v4.json"
assert_fails aggregate_evidence_mismatch run_stage "$case_root" "$ROOT/outputs/aggregate-digest"

case_root="$(make_case specialized-run)"
jq '.candidate.androidRunId = "9999"' "$case_root/emulated/termux-battery-emulated-evidence.json" >"$case_root/battery.next"
mv "$case_root/battery.next" "$case_root/emulated/termux-battery-emulated-evidence.json"
assert_fails specialized_evidence_mismatch run_stage "$case_root" "$ROOT/outputs/specialized-run"

case_root="$(make_case classifier-lineage)"
jq '.candidate.fullSuiteManifestSha256 = ("0" * 64)' \
  "$case_root/emulated/termux-observation-requirement-v3.json" >"$case_root/classifier.next"
mv "$case_root/classifier.next" "$case_root/emulated/termux-observation-requirement-v3.json"
assert_fails observation_requirement_mismatch run_stage "$case_root" "$ROOT/outputs/classifier-lineage"

case_root="$(make_case classifier-change-list)"
jq '.changedInputClasses = []' \
  "$case_root/emulated/termux-observation-requirement-v3.json" >"$case_root/classifier.next"
mv "$case_root/classifier.next" "$case_root/emulated/termux-observation-requirement-v3.json"
assert_fails observation_requirement_mismatch \
  run_stage "$case_root" "$ROOT/outputs/classifier-change-list"

case_root="$(make_case qualification-class)"
jq '.qualificationClass = "physical_v1"' "$case_root/automated-qualification-v1.json" \
  >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$case_root/automated-qualification-v1.json"
assert_fails automated_qualification_mismatch run_stage "$case_root" "$ROOT/outputs/qualification-class"

case_root="$(make_case qualification-claim)"
jq '.claimBoundary.physicalDeviceObserved = true' "$case_root/automated-qualification-v1.json" \
  >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$case_root/automated-qualification-v1.json"
assert_fails automated_qualification_mismatch run_stage "$case_root" "$ROOT/outputs/qualification-claim"

case_root="$(make_case qualification-run-substitution)"
jq '.qualificationRun.runId = "9999"' "$case_root/automated-qualification-v1.json" \
  >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$case_root/automated-qualification-v1.json"
assert_fails automated_qualification_mismatch \
  run_stage "$case_root" "$ROOT/outputs/qualification-run-substitution"

case_root="$(make_case qualification-extra-key)"
jq '.operatorSuppliedMinutes = 60' "$case_root/automated-qualification-v1.json" \
  >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$case_root/automated-qualification-v1.json"
assert_fails automated_qualification_mismatch run_stage "$case_root" "$ROOT/outputs/qualification-extra-key"

case_root="$(make_case qualification-duplicate-key)"
sed '1s/^{/{"schemaVersion":1,/' "$case_root/automated-qualification-v1.json" \
  >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$case_root/automated-qualification-v1.json"
assert_fails automated_qualification_json_invalid run_stage "$case_root" "$ROOT/outputs/qualification-duplicate-key"

case_root="$(make_case physical-substitution)"
printf '%s\n' \
  '{"schemaVersion":1,"envelopeVersion":"1","status":"pass","releaseEligible":true}' \
  >"$case_root/automated-qualification-v1.json"
assert_fails automated_qualification_mismatch run_stage "$case_root" "$ROOT/outputs/physical-substitution"

case_root="$(make_case deployment-digest)"
jq '.validation.boundedCleanup = false' "$case_root/automated-native-deployment-v1.json" \
  >"$case_root/deployment.next"
mv "$case_root/deployment.next" "$case_root/automated-native-deployment-v1.json"
assert_fails automated_qualification_mismatch run_stage "$case_root" "$ROOT/outputs/deployment-digest"

case_root="$(make_case deployment-coherent-claim-tamper)"
jq '.environment.physicalDeviceObserved = true' \
  "$case_root/automated-native-deployment-v1.json" >"$case_root/deployment.next"
mv "$case_root/deployment.next" "$case_root/automated-native-deployment-v1.json"
deployment_sha="$(sha256sum "$case_root/automated-native-deployment-v1.json" | awk '{print $1}')"
deployment_bytes="$(stat -c '%s' "$case_root/automated-native-deployment-v1.json")"
jq --arg sha "$deployment_sha" --argjson bytes "$deployment_bytes" \
  '.evidence.deployment.sha256 = $sha | .evidence.deployment.bytes = $bytes' \
  "$case_root/automated-qualification-v1.json" >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$case_root/automated-qualification-v1.json"
assert_fails automated_qualification_mismatch \
  run_stage "$case_root" "$ROOT/outputs/deployment-coherent-claim-tamper"

case_root="$(make_case qualification-nested-extra-key)"
jq '.environment.rootfsUserland.operatorSupplied = true' \
  "$case_root/automated-qualification-v1.json" >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$case_root/automated-qualification-v1.json"
assert_fails automated_qualification_mismatch \
  run_stage "$case_root" "$ROOT/outputs/qualification-nested-extra-key"

runtime_names=(
  termux-qualified-runtime-image-v1.tar.gz
  termux-runtime-package-lock-v1.json
  termux-runtime-snapshot-v1.json
  termux-runtime-snapshot-replay-v1.json
)
runtime_labels=(archive package-lock snapshot replay)
for index in "${!runtime_names[@]}"; do
  case_root="$(make_case "runtime-${runtime_labels[$index]}-bytes")"
  printf '\n' >>"$case_root/runtime/${runtime_names[$index]}"
  assert_fails automated_qualification_mismatch \
    run_stage "$case_root" "$ROOT/outputs/runtime-${runtime_labels[$index]}-bytes"
done

case_root="$(make_case runtime-package-lock-duplicate-key)"
sed '1s/^{/{"schemaVersion":1,/' \
  "$case_root/runtime/termux-runtime-package-lock-v1.json" >"$case_root/runtime.next"
mv "$case_root/runtime.next" "$case_root/runtime/termux-runtime-package-lock-v1.json"
assert_fails runtime_evidence_json_invalid \
  run_stage "$case_root" "$ROOT/outputs/runtime-package-lock-duplicate-key"

case_root="$(make_case retained-runtime-record-tamper)"
jq '.retainedRuntime.archive.sha256 = ("0" * 64)' \
  "$case_root/automated-qualification-v1.json" >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$case_root/automated-qualification-v1.json"
assert_fails automated_qualification_mismatch \
  run_stage "$case_root" "$ROOT/outputs/retained-runtime-record-tamper"

case_root="$(make_case retained-runtime-verification-tamper)"
jq '.retainedRuntime.verification.runtimeNetworkAccess = true' \
  "$case_root/automated-qualification-v1.json" >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$case_root/automated-qualification-v1.json"
assert_fails automated_qualification_mismatch \
  run_stage "$case_root" "$ROOT/outputs/retained-runtime-verification-tamper"

case_root="$(make_case retained-runtime-physical-claim)"
jq '.retainedRuntime.claimBoundary.physicalDeviceObserved = true' \
  "$case_root/automated-qualification-v1.json" >"$case_root/qualification.next"
mv "$case_root/qualification.next" "$case_root/automated-qualification-v1.json"
assert_fails automated_qualification_mismatch \
  run_stage "$case_root" "$ROOT/outputs/retained-runtime-physical-claim"

case_root="$(make_case runtime-archive-link)"
mv "$case_root/runtime/termux-qualified-runtime-image-v1.tar.gz" \
  "$case_root/runtime/runtime-archive.real"
ln -s runtime-archive.real "$case_root/runtime/termux-qualified-runtime-image-v1.tar.gz"
assert_fails runtime_evidence_invalid \
  run_stage "$case_root" "$ROOT/outputs/runtime-archive-link"

case_root="$(make_case runtime-archive-over-limit)"
truncate -s 1610612737 "$case_root/runtime/termux-qualified-runtime-image-v1.tar.gz"
assert_fails runtime_evidence_invalid \
  run_stage "$case_root" "$ROOT/outputs/runtime-archive-over-limit"

case_root="$(make_case runtime-json-over-limit)"
truncate -s 16777217 "$case_root/runtime/termux-runtime-package-lock-v1.json"
assert_fails runtime_evidence_invalid \
  run_stage "$case_root" "$ROOT/outputs/runtime-json-over-limit"

case_root="$(make_case license-link)"
mv "$case_root/LICENSE" "$case_root/LICENSE.real"
ln -s LICENSE.real "$case_root/LICENSE"
assert_fails license_invalid run_stage "$case_root" "$ROOT/outputs/license-link"

case_root="$(make_case qualification-link)"
mv "$case_root/automated-qualification-v1.json" "$case_root/qualification.real.json"
ln -s qualification.real.json "$case_root/automated-qualification-v1.json"
assert_fails automated_qualification_invalid run_stage "$case_root" "$ROOT/outputs/qualification-link"

case_root="$(make_case qualification-oversized)"
dd if=/dev/zero bs=1048576 count=1 status=none | tr '\0' ' ' \
  >>"$case_root/automated-qualification-v1.json"
assert_fails automated_qualification_invalid run_stage "$case_root" "$ROOT/outputs/qualification-oversized"

case_root="$(make_case existing-output)"
mkdir -p "$ROOT/outputs/existing-output"
touch "$ROOT/outputs/existing-output/$OUTPUT_NAME"
assert_fails output_invalid run_stage "$case_root" "$ROOT/outputs/existing-output"

case_root="$(make_case noncanonical-output)"
assert_fails output_name_invalid run_stage "$case_root" "$ROOT/outputs/noncanonical-output" preflight-release-stage.tar

case_root="$(make_case snapshot-race)"
mkdir -m 700 "$ROOT/racing-cp"
cat >"$ROOT/racing-cp/cp" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
source_path="${@: -2:1}"
"$STAGE_REAL_CP" "$@"
if [[ "$source_path" == */termux-native-aggregate-evidence-v4.json ]]; then
  printf '{}\n' >"$source_path"
fi
EOF
chmod 700 "$ROOT/racing-cp/cp"
aggregate_sha_before="$(sha256sum "$case_root/emulated/termux-native-aggregate-evidence-v4.json" | awk '{print $1}')"
STAGE_REAL_CP="$REAL_CP" STAGE_TEST_PATH="$ROOT/racing-cp:$ROOT/fake-bin:$REAL_PATH" \
  run_stage "$case_root" "$ROOT/outputs/snapshot-race" >/dev/null
mkdir "$ROOT/snapshot-race-extracted"
tar -xf "$ROOT/outputs/snapshot-race/$OUTPUT_NAME" -C "$ROOT/snapshot-race-extracted"
[[ "$(sha256sum "$ROOT/snapshot-race-extracted/evidence/termux-native-aggregate-evidence-v4.json" | awk '{print $1}')" == "$aggregate_sha_before" ]] \
  || fail_test 'source replacement changed the validated snapshot'

case_root="$(make_case runtime-snapshot-race)"
mkdir -m 700 "$ROOT/runtime-racing-cp"
cat >"$ROOT/runtime-racing-cp/cp" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
source_path="${@: -2:1}"
"$STAGE_REAL_CP" "$@"
if [[ "$source_path" == */termux-qualified-runtime-image-v1.tar.gz ]]; then
  printf 'foreign replacement\n' >"$source_path"
fi
EOF
chmod 700 "$ROOT/runtime-racing-cp/cp"
STAGE_REAL_CP="$REAL_CP" \
STAGE_TEST_PATH="$ROOT/runtime-racing-cp:$ROOT/fake-bin:$REAL_PATH" \
  assert_fails input_snapshot_failed \
    run_stage "$case_root" "$ROOT/outputs/runtime-snapshot-race"
[[ ! -e "$ROOT/outputs/runtime-snapshot-race/$OUTPUT_NAME" ]] \
  || fail_test 'runtime source replacement published an output'

case_root="$(make_case publication-race)"
mkdir -m 700 "$ROOT/racing-python"
cat >"$ROOT/racing-python/python3" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
target="${@: -1}"
source="${@: -2:1}"
if [[ "$target" == "$STAGE_TEST_OUTPUT" && "$source" == *.staging.* ]]; then
  printf 'concurrent-owner\n' >"$target"
fi
exec "$STAGE_REAL_PYTHON3" "$@"
EOF
chmod 700 "$ROOT/racing-python/python3"
STAGE_TEST_OUTPUT="$ROOT/outputs/publication-race/$OUTPUT_NAME" \
STAGE_REAL_PYTHON3="$REAL_PYTHON3" \
STAGE_TEST_PATH="$ROOT/racing-python:$ROOT/fake-bin:$REAL_PATH" \
  assert_fails archive_publication_failed run_stage "$case_root" "$ROOT/outputs/publication-race"
[[ "$(<"$ROOT/outputs/publication-race/$OUTPUT_NAME")" == concurrent-owner ]] \
  || fail_test 'no-clobber publication changed a concurrent output'
[[ -z "$(find "$ROOT/outputs/publication-race" -maxdepth 1 -name "$OUTPUT_NAME.staging.*" -print -quit)" ]] \
  || fail_test 'publication conflict leaked owned temporary state'

case_root="$(make_case hard-link-publication-race)"
mkdir -m 700 "$ROOT/hard-link-racing-python"
cat >"$ROOT/hard-link-racing-python/python3" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
target="${@: -1}"
source="${@: -2:1}"
if [[ "$target" == "$STAGE_TEST_OUTPUT" && "$source" == *.staging.* ]]; then
  "$STAGE_REAL_LN" -- "$source" "$target"
fi
exec "$STAGE_REAL_PYTHON3" "$@"
EOF
chmod 700 "$ROOT/hard-link-racing-python/python3"
STAGE_TEST_OUTPUT="$ROOT/outputs/hard-link-publication-race/$OUTPUT_NAME" \
STAGE_REAL_LN="$(command -v ln)" STAGE_REAL_PYTHON3="$REAL_PYTHON3" \
STAGE_TEST_PATH="$ROOT/hard-link-racing-python:$ROOT/fake-bin:$REAL_PATH" \
  assert_fails archive_publication_failed run_stage "$case_root" \
    "$ROOT/outputs/hard-link-publication-race"
[[ -f "$ROOT/outputs/hard-link-publication-race/$OUTPUT_NAME" ]] \
  || fail_test 'hard-link collision cleanup removed the concurrent output'
[[ "$(sha256sum "$ROOT/outputs/hard-link-publication-race/$OUTPUT_NAME" | awk '{print $1}')" =~ ^[0-9a-f]{64}$ ]] \
  || fail_test 'hard-link collision did not retain the complete archive'
[[ -z "$(find "$ROOT/outputs/hard-link-publication-race" -maxdepth 1 \
  -name "$OUTPUT_NAME.staging.*" -print -quit)" ]] \
  || fail_test 'hard-link collision leaked owned staging state'

case_root="$(make_case pre-link-source-symlink-race)"
mkdir -m 700 "$ROOT/symlink-racing-python"
printf 'unvalidated replacement\n' >"$ROOT/unvalidated-stage-replacement"
cat >"$ROOT/symlink-racing-python/python3" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
target="${@: -1}"
source="${@: -2:1}"
if [[ "$target" == "$STAGE_TEST_OUTPUT" && "$source" == *.staging.* ]]; then
  "$STAGE_REAL_MV" -- "$source" "$source.validated"
  "$STAGE_REAL_LN" -s -- "$STAGE_REPLACEMENT" "$source"
fi
exec "$STAGE_REAL_PYTHON3" "$@"
EOF
chmod 700 "$ROOT/symlink-racing-python/python3"
STAGE_TEST_OUTPUT="$ROOT/outputs/pre-link-source-symlink-race/$OUTPUT_NAME" \
STAGE_REPLACEMENT="$ROOT/unvalidated-stage-replacement" \
STAGE_REAL_LN="$(command -v ln)" STAGE_REAL_MV="$REAL_MV" \
STAGE_REAL_PYTHON3="$REAL_PYTHON3" \
STAGE_TEST_PATH="$ROOT/symlink-racing-python:$ROOT/fake-bin:$REAL_PATH" \
  assert_fails archive_publication_failed run_stage "$case_root" \
    "$ROOT/outputs/pre-link-source-symlink-race"
[[ ! -e "$ROOT/outputs/pre-link-source-symlink-race/$OUTPUT_NAME" \
  && ! -L "$ROOT/outputs/pre-link-source-symlink-race/$OUTPUT_NAME" ]] \
  || fail_test 'pre-link source replacement published an output'
if grep -Eq 'rm[^#\n]*\\$OUTPUT|PUBLISH_(LINKED|IDENTITY)' "$SCRIPT"; then
  fail_test 'failure cleanup may still unlink or identity-check the public output'
fi

case_root="$(make_case staged-tar-over-limit)"
mkdir -p "$ROOT/oversized-tar"
cat >"$ROOT/oversized-tar/tar" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
output=""
while (($# > 0)); do
  if [[ "$1" == -cf && $# -ge 2 ]]; then
    output="$2"
    break
  fi
  shift
done
[[ -n "$output" ]]
truncate -s 2147483648 "$output"
EOF
chmod 700 "$ROOT/oversized-tar/tar"
STAGE_TEST_PATH="$ROOT/oversized-tar:$ROOT/fake-bin:$REAL_PATH" \
  assert_fails archive_size_invalid \
    run_stage "$case_root" "$ROOT/outputs/staged-tar-over-limit"
[[ ! -e "$ROOT/outputs/staged-tar-over-limit/$OUTPUT_NAME" ]] \
  || fail_test 'oversized staged tar published an output'
[[ -z "$(find "$ROOT/outputs/staged-tar-over-limit" -maxdepth 1 \
  -name "$OUTPUT_NAME.staging.*" -print -quit)" ]] \
  || fail_test 'oversized staged tar leaked owned staging state'

case_root="$(make_case tar-failure)"
mkdir -p "$ROOT/bad-tar" "$ROOT/outputs/tar-failure"
cat >"$ROOT/bad-tar/tar" <<'EOF'
#!/usr/bin/env bash
exit 42
EOF
chmod 700 "$ROOT/bad-tar/tar"
touch "$ROOT/outputs/tar-failure/$OUTPUT_NAME.staging.unrelated"
STAGE_TEST_PATH="$ROOT/bad-tar:$ROOT/fake-bin:$REAL_PATH" \
  assert_fails deterministic_archive_failed run_stage "$case_root" "$ROOT/outputs/tar-failure"
[[ -e "$ROOT/outputs/tar-failure/$OUTPUT_NAME.staging.unrelated" ]] \
  || fail_test 'failure cleanup removed unrelated state'
[[ "$(find "$ROOT/outputs/tar-failure" -maxdepth 1 -name "$OUTPUT_NAME.staging.*" | wc -l)" == 1 ]] \
  || fail_test 'failure cleanup left owned temporary state'
[[ ! -e "$ROOT/outputs/tar-failure/$OUTPUT_NAME" ]] || fail_test 'failed archive creation published output'

printf 'Deterministic release staging asset tests passed\n'
