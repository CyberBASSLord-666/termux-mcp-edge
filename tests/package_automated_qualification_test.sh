#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077

ROOT="$(mktemp -d)"
trap 'rm -rf -- "$ROOT"' EXIT INT TERM
chmod 700 "$ROOT"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/package_automated_qualification.sh"
POLICY="$REPO_ROOT/docs/release-qualification-policy-v1.json"
SCENARIO_SET="$REPO_ROOT/docs/automated-native-deployment-scenarios-v1.json"
SCHEMA="$REPO_ROOT/docs/release-automated-qualification-schema-v1.json"
EVIDENCE="$ROOT/evidence"
BUNDLES="$ROOT/bundles"
OUTPUTS="$ROOT/outputs"
MOCK_BIN="$ROOT/bin"
mkdir -m 700 "$EVIDENCE" "$BUNDLES" "$OUTPUTS" "$MOCK_BIN"

COMMIT=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
OTHER_COMMIT=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
VERSION=0.6.0
CI_RUN_ID=1001
SECURITY_RUN_ID=1002
ANDROID_RUN_ID=1003
QUALIFICATION_RUN_ID=1004
ROOTFS_DIGEST=sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd
ROOTFS_IMAGE_ID=sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
RUNTIME_IMAGE_DIGEST=sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc
LINKER_SHA=eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee
POLICY_SHA=22ae15bb36fcb6c76c9341f4546dc38397e69f885f8da4a357b90d61c567c5ed
SCENARIO_SHA=dd31d4f89f9f25dba1a1bb1c492fd796f5a2619b215e2d57f3b0e60f9f24b3bb

postures=(
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
features=(
  '[]'
  '["mcp-runtime"]'
  '["android-battery-status"]'
  '["android-volume-status"]'
  '["android-volume-control"]'
  '["command-execution"]'
  '["full-suite"]'
)
binary_sha=()
binary_bytes=()
manifest_sha=()

fail_test() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

sha() {
  sha256sum -- "$1" | awk '{print $1}'
}

help_output="$(bash "$SCRIPT" --help)"
grep -Fq -- \
  '--policy /absolute/path/release-qualification-policy-v1.json' \
  <<<"$help_output" \
  || fail_test "help does not show absolute policy path"
grep -Fq -- \
  '--runtime-archive /absolute/path/termux-qualified-runtime-image-v1.tar.gz' \
  <<<"$help_output" \
  || fail_test "help does not show absolute runtime archive path"
grep -Fq -- \
  'All file and directory arguments, including --output, must be normalized' \
  <<<"$help_output" \
  || fail_test "help does not explain the absolute-path contract"
if grep -Fq -- '--policy release-qualification-policy-v1.json' <<<"$help_output"; then
  fail_test "help still advertises a rejected relative policy path"
fi

printf '%s\n' '#!/usr/bin/env bash' \
  'printf "%s\n" "ELF 64-bit LSB pie executable, ARM aarch64, Android, interpreter /system/bin/linker64"' \
  >"$MOCK_BIN/file"
chmod 700 "$MOCK_BIN/file"
export PATH="$MOCK_BIN:$PATH"

for index in "${!postures[@]}"; do
  bundle="$BUNDLES/${postures[$index]}"
  mkdir -m 700 "$bundle"
  printf '#!/data/data/com.termux/files/usr/bin/bash\n# posture=%s\n' \
    "${postures[$index]}" >"$bundle/termux-mcp-server"
  chmod 700 "$bundle/termux-mcp-server"
  binary_sha[$index]="$(sha "$bundle/termux-mcp-server")"
  binary_bytes[$index]="$(stat -c '%s' "$bundle/termux-mcp-server")"
  printf '%s  termux-mcp-server\n' "${binary_sha[$index]}" >"$bundle/SHA256SUMS"
  jq -n \
    --arg artifact_name "${artifact_names[$index]}" \
    --arg posture "${postures[$index]}" \
    --argjson features "${features[$index]}" \
    --arg commit "$COMMIT" \
    --arg version "$VERSION" \
    --arg workflow_run_id "$ANDROID_RUN_ID" \
    --arg sha "${binary_sha[$index]}" \
    --argjson bytes "${binary_bytes[$index]}" '
    {
      schemaVersion: 1,
      repository: "CyberBASSLord-666/termux-mcp-edge",
      commit: $commit,
      workflowRunId: $workflow_run_id,
      artifactName: $artifact_name,
      posture: $posture,
      features: $features,
      target: "aarch64-linux-android",
      fileName: "termux-mcp-server",
      version: $version,
      sha256: $sha,
      bytes: $bytes,
      elf: "aarch64-android-elf",
      createdAt: "2026-07-23T00:00:00Z"
    }
  ' >"$bundle/artifact-manifest.json"
  manifest_sha[$index]="$(sha "$bundle/artifact-manifest.json")"
done

AGGREGATE="$EVIDENCE/termux-native-aggregate-evidence-v4.json"
jq -n \
  --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg default_sha "${binary_sha[0]}" --argjson default_bytes "${binary_bytes[0]}" \
  --arg mcp_sha "${binary_sha[1]}" --argjson mcp_bytes "${binary_bytes[1]}" \
  --arg control_sha "${binary_sha[4]}" --argjson control_bytes "${binary_bytes[4]}" \
  --arg full_sha "${binary_sha[6]}" --argjson full_bytes "${binary_bytes[6]}" \
  --arg full_manifest "${manifest_sha[6]}" --arg image_digest "$ROOTFS_DIGEST" \
  --arg rootfs_image_id "$ROOTFS_IMAGE_ID" \
  --arg runtime_image_digest "$RUNTIME_IMAGE_DIGEST" '
  {
    schemaVersion:4, gateVersion:"4", status:"pass", failureCode:null,
    releaseQualificationEligible:false,
    startedAt:"2026-07-23T00:00:00Z", completedAt:"2026-07-23T00:01:00Z",
    candidate:{
      commit:$commit,version:$version,ciRunId:$ci,securityRunId:$security,androidRunId:$android,
      defaultArtifact:{sha256:$default_sha,bytes:$default_bytes},
      mcpRuntimeArtifact:{sha256:$mcp_sha,bytes:$mcp_bytes},
      androidVolumeControlArtifact:{sha256:$control_sha,bytes:$control_bytes},
      fullSuiteArtifact:{
        sha256:$full_sha,bytes:$full_bytes,manifestSha256:$full_manifest,
        artifactName:"termux-mcp-server-aarch64-linux-android-full-suite",
        posture:"full-suite",features:["full-suite"],fileName:"termux-mcp-server"
      }
    },
    environment:{
      executionMode:"official-termux-docker-native-arm64",architecture:"aarch64",
      image:"termux/termux-docker:aarch64",imageDigest:$image_digest,
      rootfsImageId:$rootfs_image_id,
      runtimeImageDigest:$runtime_image_digest,androidLinker:true
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
      status:"pass",reportSha256:("1"*64),resultCount:20,
      phases:{preflight:"pass",runtime:"pass",deployment:"not_run"}
    },
    aggregateValidation:{
      status:"pass",requests:24,
      defaultDisabled:{
        toolCount:17,exactToolOrder:true,optionalFeaturesCompiled:true,
        optionalToolsHidden:true,runtimeFlagsOmitted:true
      },
      fullyEnabled:{
        toolCount:21,exactToolOrder:true,allOptionalToolsExposed:true,
        providerSuccesses:true,volumePreviewNoMutation:true,
        volumeGrantIsolation:true,commandExecutableIdentityPinned:true
      },
      independentRuntimeGates:true,filesystemMutationsDisabled:true,
      boundedCleanup:true,automatedQualificationComponent:true
    },
    stress:{
      status:"pass",samples:32,requests:100,servicePidStable:true,
      healthReadyStable:true,sessionLifecycle:true,exactToolAllowlist:true,
      safeRootIdentityPinned:true,safeRootAncestorIdentityPinned:true,
      copyFileMutationDisabled:true,highImpactDisabled:true,
      longObservationRequired:false
    }
  }
' >"$AGGREGATE"
chmod 600 "$AGGREGATE"
AGGREGATE_SHA="$(sha "$AGGREGATE")"

make_specialized() {
  local kind="$1" schema="$2" gate="$3" artifact_index="$4" output="$5"
  local companion_index="${6:-}"
  local schema_path candidate_extra validation
  case "$kind" in
    battery) schema_path="$REPO_ROOT/docs/android-battery-emulated-evidence-schema-v3.json" ;;
    volume) schema_path="$REPO_ROOT/docs/android-volume-emulated-evidence-schema-v2.json" ;;
    volume-control) schema_path="$REPO_ROOT/docs/android-volume-control-emulated-evidence-schema-v2.json" ;;
    command) schema_path="$REPO_ROOT/docs/command-emulated-evidence-schema-v3.json" ;;
    *) fail_test "unknown specialized kind" ;;
  esac
  validation="$(jq -c --arg kind "$kind" '
    ."$defs".validation.properties
    | reduce (keys[]) as $key ({};
        .[$key] = (
          if $key == "status" then "pass"
          elif $key == "requests" then
            if $kind == "battery" then 18
            elif $kind == "volume" then 19
            elif $kind == "volume-control" then 20
            else 29 end
          elif $key == "longObservationRequired" then false
          else true end
        )
      )
  ' "$schema_path")"
  candidate_extra='{}'
  if [[ "$kind" == volume-control ]]; then
    candidate_extra="$(jq -cn \
      --arg sha "${binary_sha[$companion_index]}" \
      --argjson bytes "${binary_bytes[$companion_index]}" \
      '{incompatibleArtifact:{sha256:$sha,bytes:$bytes}}')"
  elif [[ "$kind" == command ]]; then
    candidate_extra="$(jq -cn \
      --arg sha "${binary_sha[$companion_index]}" \
      --argjson bytes "${binary_bytes[$companion_index]}" \
      '{defaultArtifact:{sha256:$sha,bytes:$bytes}}')"
  fi
  jq -n \
    --argjson schema "$schema" --arg gate "$gate" \
    --arg commit "$COMMIT" --arg version "$VERSION" \
    --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
    --arg sha "${binary_sha[$artifact_index]}" \
    --argjson bytes "${binary_bytes[$artifact_index]}" \
    --arg image_digest "$ROOTFS_DIGEST" \
    --arg rootfs_image_id "$ROOTFS_IMAGE_ID" \
    --arg runtime_image_digest "$RUNTIME_IMAGE_DIGEST" \
    --argjson extra "$candidate_extra" --argjson validation "$validation" '
    {
      schemaVersion:$schema,gateVersion:$gate,status:"pass",failureCode:null,
      releaseQualificationEligible:false,
      startedAt:"2026-07-23T00:00:00Z",completedAt:"2026-07-23T00:01:00Z",
      candidate:({
        commit:$commit,version:$version,ciRunId:$ci,securityRunId:$security,
        androidRunId:$android,artifact:{sha256:$sha,bytes:$bytes}
      } + $extra),
      environment:{
        architecture:"aarch64",executionMode:"official-termux-docker-native-arm64",
        image:"termux/termux-docker:aarch64",imageDigest:$image_digest,
        rootfsImageId:$rootfs_image_id,runtimeImageDigest:$runtime_image_digest,
        androidLinker:true
      },
      validation:$validation
    }
  ' >"$output"
  chmod 600 "$output"
}

BATTERY="$EVIDENCE/termux-battery-emulated-evidence.json"
VOLUME="$EVIDENCE/termux-volume-emulated-evidence.json"
VOLUME_CONTROL="$EVIDENCE/termux-volume-control-emulated-evidence.json"
COMMAND="$EVIDENCE/termux-command-emulated-evidence.json"
make_specialized battery 3 3 2 "$BATTERY"
make_specialized volume 2 2 3 "$VOLUME"
make_specialized volume-control 2 2 4 "$VOLUME_CONTROL" 3
make_specialized command 3 3 5 "$COMMAND" 0

CLASSIFIER="$EVIDENCE/termux-observation-requirement-v3.json"
jq -n \
  --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg full_sha "${binary_sha[6]}" --arg full_manifest "${manifest_sha[6]}" \
  --arg aggregate_sha "$AGGREGATE_SHA" --arg image_digest "$ROOTFS_DIGEST" '
  {
    schemaVersion:3,classifierVersion:"3",status:"pass",failureCode:null,
    releaseQualificationEligible:false,createdAt:"2026-07-23T00:02:00Z",
    evidenceMode:"automated_release_qualification",
    reasonCode:"automated_native_termux_evidence_required",
    inheritanceCandidate:false,source:{commit:$commit},
    candidate:{
      commit:$commit,version:$version,ciRunId:$ci,securityRunId:$security,
      androidRunId:$android,fullSuiteArtifactSha256:$full_sha,
      fullSuiteManifestSha256:$full_manifest
    },
    emulation:{
      reportSha256:$aggregate_sha,executionMode:"official-termux-docker-native-arm64",
      imageDigest:$image_digest,status:"pass",samples:32
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
' >"$CLASSIFIER"
chmod 600 "$CLASSIFIER"

DEPLOYMENT="$EVIDENCE/automated-native-deployment-v1.json"
jq -n \
  --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg native "$ANDROID_RUN_ID" \
  --arg full_sha "${binary_sha[6]}" --arg full_manifest "${manifest_sha[6]}" \
  --arg rootfs_digest "$ROOTFS_DIGEST" --arg rootfs_image_id "$ROOTFS_IMAGE_ID" \
  --arg runtime_image_digest "$RUNTIME_IMAGE_DIGEST" \
  --arg linker_sha "$LINKER_SHA" \
  --arg scenario_sha "$SCENARIO_SHA" '
  {
    schemaVersion:1,gateVersion:"1",status:"pass",failureCode:null,
    releaseQualificationEligible:false,
    qualificationClass:"official_termux_native_automated_v1",
    startedAt:"2026-07-23T00:03:00Z",completedAt:"2026-07-23T00:04:00Z",
    candidate:{
      repository:"CyberBASSLord-666/termux-mcp-edge",commit:$commit,version:$version,
      ciRunId:$ci,securityRunId:$security,nativeRunId:$native,
      artifact:{
        artifactName:"termux-mcp-server-aarch64-linux-android-full-suite",
        posture:"full-suite",features:["full-suite"],sha256:$full_sha,
        manifestSha256:$full_manifest,bytes:55,target:"aarch64-linux-android",
        elf:"aarch64-android-elf"
      }
    },
    scenarioSet:{
      fileName:"automated-native-deployment-scenarios-v1.json",
      schemaVersion:1,scenarioSetVersion:"1",sha256:$scenario_sha,scenarioCount:6,
      scenarioIds:[
        "isolated_fresh_deploy","failed_upgrade_recovery","supervised_restart",
        "rollback_recovery","uninstall","bounded_cleanup"
      ]
    },
    environment:{
      architecture:"aarch64",executionMode:"official-termux-docker-native-arm64",
      rootfsImage:"termux/termux-docker:aarch64",rootfsDigest:$rootfs_digest,
      rootfsImageId:$rootfs_image_id,
      runtimeImageDigest:$runtime_image_digest,
      termuxPrefix:"/data/data/com.termux/files/usr",
      androidLinker:{observed:true,path:"/system/bin/linker64",sha256:$linker_sha,bytes:4096},
      supervisor:"runit",runitSupervisorObserved:true,
      androidFrameworkObserved:false,physicalHardwareObserved:false,
      physicalDeviceObserved:false,sustainedPhysicalSoak:false
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
' >"$DEPLOYMENT"
# Replace fixture literal with the exact generated binary byte count.
jq --argjson bytes "${binary_bytes[6]}" '.candidate.artifact.bytes = $bytes' \
  "$DEPLOYMENT" >"$DEPLOYMENT.next"
mv "$DEPLOYMENT.next" "$DEPLOYMENT"
chmod 600 "$DEPLOYMENT"

RUNTIME_DIR="$EVIDENCE/runtime"
mkdir -m 700 "$RUNTIME_DIR"
RUNTIME_ARCHIVE="$RUNTIME_DIR/termux-qualified-runtime-image-v1.tar.gz"
RUNTIME_LOCK="$RUNTIME_DIR/termux-runtime-package-lock-v1.json"
RUNTIME_SNAPSHOT="$RUNTIME_DIR/termux-runtime-snapshot-v1.json"
RUNTIME_REPLAY="$RUNTIME_DIR/termux-runtime-snapshot-replay-v1.json"
printf 'retained qualified runtime archive fixture\n' >"$RUNTIME_ARCHIVE"
chmod 600 "$RUNTIME_ARCHIVE"
RUNTIME_ARCHIVE_SHA="$(sha "$RUNTIME_ARCHIVE")"
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
RUNTIME_LOCK_SHA="$(sha "$RUNTIME_LOCK")"
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
  --arg runtime_id "$RUNTIME_IMAGE_DIGEST" \
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
RUNTIME_SNAPSHOT_SHA="$(sha "$RUNTIME_SNAPSHOT")"
RUNTIME_SNAPSHOT_BYTES="$(stat -c '%s' "$RUNTIME_SNAPSHOT")"
jq -n \
  --arg commit "$COMMIT" --arg runtime_id "$RUNTIME_IMAGE_DIGEST" \
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
RUNTIME_REPLAY_SHA="$(sha "$RUNTIME_REPLAY")"

run_package() {
  local output="$1"
  local policy="${2:-$POLICY}" scenario="${3:-$SCENARIO_SET}"
  local aggregate="${4:-$AGGREGATE}" deployment="${5:-$DEPLOYMENT}"
  local classifier="${6:-$CLASSIFIER}" battery="${7:-$BATTERY}"
  local volume="${8:-$VOLUME}" volume_control="${9:-$VOLUME_CONTROL}"
  local command="${10:-$COMMAND}"
  local runtime_archive="${RUNTIME_ARCHIVE_OVERRIDE:-$RUNTIME_ARCHIVE}"
  local runtime_lock="${RUNTIME_LOCK_OVERRIDE:-$RUNTIME_LOCK}"
  local runtime_snapshot="${RUNTIME_SNAPSHOT_OVERRIDE:-$RUNTIME_SNAPSHOT}"
  local runtime_replay="${RUNTIME_REPLAY_OVERRIDE:-$RUNTIME_REPLAY}"
  bash "$SCRIPT" \
    --policy "$policy" --scenario-set "$scenario" \
    --aggregate-evidence "$aggregate" --deployment-evidence "$deployment" \
    --classifier-evidence "$classifier" --battery-evidence "$battery" \
    --volume-evidence "$volume" --volume-control-evidence "$volume_control" \
    --command-evidence "$command" \
    --runtime-archive "$runtime_archive" \
    --runtime-package-lock "$runtime_lock" \
    --runtime-snapshot "$runtime_snapshot" \
    --runtime-replay "$runtime_replay" \
    --default-dir "$BUNDLES/default" --mcp-runtime-dir "$BUNDLES/mcp-runtime" \
    --battery-dir "$BUNDLES/android-battery-status" \
    --volume-dir "$BUNDLES/android-volume-status" \
    --volume-control-dir "$BUNDLES/android-volume-control" \
    --command-dir "$BUNDLES/command-execution" \
    --full-suite-dir "$BUNDLES/full-suite" \
    --qualification-run-id "$QUALIFICATION_RUN_ID" \
    --output "$output"
}

assert_fails() {
  local reason="$1" output="$2"
  shift 2
  if run_package "$output" "$@" >"$ROOT/last.stdout" 2>"$ROOT/last.stderr"; then
    fail_test "package unexpectedly passed for $reason"
  fi
  grep -Fq "reason=$reason" "$ROOT/last.stderr" \
    || fail_test "expected $reason, got $(<"$ROOT/last.stderr")"
  [[ ! -e "$output" && ! -L "$output" ]] \
    || fail_test "$reason published output"
}

make_variant() {
  local source="$1" destination_dir="$2" file_name="$3" filter="$4"
  mkdir -m 700 "$destination_dir"
  jq "$filter" "$source" >"$destination_dir/$file_name"
  chmod 600 "$destination_dir/$file_name"
  printf '%s\n' "$destination_dir/$file_name"
}

bash -n "$SCRIPT"
bash -n "${BASH_SOURCE[0]}"
jq -e . "$POLICY" "$SCENARIO_SET" "$SCHEMA" >/dev/null
[[ "$(sha "$POLICY")" == "$POLICY_SHA" ]] || fail_test "policy digest drifted"
[[ "$(sha "$SCENARIO_SET")" == "$SCENARIO_SHA" ]] || fail_test "scenario digest drifted"

PASS_OUTPUT="$OUTPUTS/pass/automated-qualification-v1.json"
mkdir -m 700 "$(dirname "$PASS_OUTPUT")"
run_package "$PASS_OUTPUT" >"$ROOT/pass.stdout" 2>"$ROOT/pass.stderr"
[[ "$(<"$ROOT/pass.stdout")" == AUTOMATED_QUALIFICATION_PACKAGE_RESULT=PASS ]] \
  || fail_test "success output contract changed"
[[ ! -s "$ROOT/pass.stderr" ]] || fail_test "success wrote stderr"
[[ "$(stat -c '%a' "$PASS_OUTPUT")" == 600 ]] || fail_test "output mode changed"
jq -e \
  --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg policy_sha "$POLICY_SHA" --arg scenario_sha "$SCENARIO_SHA" \
  --arg aggregate_sha "$AGGREGATE_SHA" \
  --arg archive_sha "$RUNTIME_ARCHIVE_SHA" \
  --arg lock_sha "$RUNTIME_LOCK_SHA" \
  --arg snapshot_sha "$RUNTIME_SNAPSHOT_SHA" \
  --arg replay_sha "$RUNTIME_REPLAY_SHA" \
  --arg qualification_run "$QUALIFICATION_RUN_ID" \
  --arg android_run "$ANDROID_RUN_ID" '
  (keys == [
    "artifacts","claimBoundary","commit","envelopeVersion","environment","evidence",
    "failureCode","gates","policy","qualificationClass","qualificationRun",
    "releaseEligible","repository","retainedRuntime","scenarioSet","schemaVersion",
    "status","version","workflowRuns"
  ])
  and .schemaVersion == 1 and .envelopeVersion == "1" and .status == "pass"
  and .failureCode == null and .releaseEligible == true
  and .qualificationClass == "official_termux_native_automated_v1"
  and .commit == $commit and .version == $version
  and .workflowRuns.ci.attempt == 1 and .workflowRuns.security.attempt == 1
  and .workflowRuns.android.attempt == 1
  and all(.workflowRuns[]; .event == "push" and .ref == "refs/heads/main"
    and .headCommit == $commit and .conclusion == "success")
  and .qualificationRun == {
    runId:$qualification_run,
    attempt:1,
    event:"workflow_run",
    sourceWorkflow:"Android Cross Compile",
    sourceRunId:$android_run
  }
  and .claimBoundary == {
    physicalDeviceObserved:false,androidFrameworkObserved:false,
    sustainedPhysicalSoak:false,physicalCertification:"not_run"
  }
  and .policy.sha256 == $policy_sha and .scenarioSet.sha256 == $scenario_sha
  and .scenarioSet.scenarioCount == 6
  and (.artifacts | length == 7)
  and ([.artifacts[].sha256] | unique | length == 7)
  and ([.artifacts[].manifestSha256] | unique | length == 7)
  and .evidence.aggregate.sha256 == $aggregate_sha
  and (.evidence.specialized | length == 4)
  and ([
    .evidence.aggregate,
    .evidence.deployment,
    .evidence.classifier,
    .evidence.specialized[]
  ] | all(keys == ["bytes","fileName","sha256"]))
  and all(.gates[]; . == "pass")
  and .environment.rootfsUserland.digest == "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
  and .environment.rootfsUserland.imageId == "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  and .environment.runtimeImageDigest == "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
  and .environment.androidRuntime.linkerIdentity == "aarch64-android-bionic-elf"
  and (.retainedRuntime | keys) == [
    "androidLinker","archive","base","claimBoundary","installedPackages",
    "packageLock","rebuildReproducibilityClaim","replay","runtimeImageId",
    "snapshot","verification"
  ]
  and .retainedRuntime.runtimeImageId
    == "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
  and .retainedRuntime.archive.sha256 == $archive_sha
  and .retainedRuntime.packageLock.sha256 == $lock_sha
  and .retainedRuntime.snapshot.sha256 == $snapshot_sha
  and .retainedRuntime.replay.sha256 == $replay_sha
  and .retainedRuntime.verification.runtimeNetworkAccess == false
  and .retainedRuntime.claimBoundary == .claimBoundary
  and .retainedRuntime.rebuildReproducibilityClaim == false
' "$PASS_OUTPUT" >/dev/null || fail_test "qualified envelope contract invalid"
if jq -r 'paths(scalars) as $p | ($p | map(tostring) | join("."))' "$PASS_OUTPUT" \
  | grep -Eiq 'duration|elapsed|equivalent|minute|time_dilation'; then
  fail_test "forbidden time-equivalence field entered envelope"
fi

mkdir -m 700 "$ROOT/runtime-archive-substitution"
cp -- "$RUNTIME_ARCHIVE" \
  "$ROOT/runtime-archive-substitution/termux-qualified-runtime-image-v1.tar.gz"
printf X >>"$ROOT/runtime-archive-substitution/termux-qualified-runtime-image-v1.tar.gz"
chmod 600 \
  "$ROOT/runtime-archive-substitution/termux-qualified-runtime-image-v1.tar.gz"
RUNTIME_ARCHIVE_OVERRIDE="$ROOT/runtime-archive-substitution/termux-qualified-runtime-image-v1.tar.gz" \
  assert_fails runtime_snapshot_archive_mismatch \
  "$OUTPUTS/runtime-archive-substitution/automated-qualification-v1.json"

mkdir -m 700 "$ROOT/runtime-lock-duplicate"
sed '0,/{/s//{"schemaVersion":1,/' "$RUNTIME_LOCK" \
  >"$ROOT/runtime-lock-duplicate/termux-runtime-package-lock-v1.json"
chmod 600 "$ROOT/runtime-lock-duplicate/termux-runtime-package-lock-v1.json"
RUNTIME_LOCK_OVERRIDE="$ROOT/runtime-lock-duplicate/termux-runtime-package-lock-v1.json" \
  assert_fails duplicate_json_key \
  "$OUTPUTS/runtime-lock-duplicate/automated-qualification-v1.json"

INVALID_INSTALL_LOCK="$(make_variant \
  "$RUNTIME_LOCK" "$ROOT/runtime-installation-method" \
  termux-runtime-package-lock-v1.json \
  '.installation.method = "termux-apt-local-archives"')"
RUNTIME_LOCK_OVERRIDE="$INVALID_INSTALL_LOCK" \
  assert_fails runtime_package_lock_contract_invalid \
  "$OUTPUTS/runtime-installation-method/automated-qualification-v1.json"

FALSE_REBUILD_SNAPSHOT="$(make_variant \
  "$RUNTIME_SNAPSHOT" "$ROOT/runtime-false-rebuild" \
  termux-runtime-snapshot-v1.json '.rebuildReproducibilityClaim = true')"
RUNTIME_SNAPSHOT_OVERRIDE="$FALSE_REBUILD_SNAPSHOT" \
  assert_fails runtime_snapshot_contract_invalid \
  "$OUTPUTS/runtime-false-rebuild/automated-qualification-v1.json"

PHYSICAL_REPLAY="$(make_variant \
  "$RUNTIME_REPLAY" "$ROOT/runtime-physical-claim" \
  termux-runtime-snapshot-replay-v1.json '.claimBoundary.physicalDeviceObserved = true')"
RUNTIME_REPLAY_OVERRIDE="$PHYSICAL_REPLAY" \
  assert_fails runtime_replay_contract_invalid \
  "$OUTPUTS/runtime-physical-claim/automated-qualification-v1.json"

SKIPPED_REPLAY="$(make_variant \
  "$RUNTIME_REPLAY" "$ROOT/runtime-skipped-verification" \
  termux-runtime-snapshot-replay-v1.json '.verification.rootfsLayersVerified = false')"
RUNTIME_REPLAY_OVERRIDE="$SKIPPED_REPLAY" \
  assert_fails runtime_replay_verification_invalid \
  "$OUTPUTS/runtime-skipped-verification/automated-qualification-v1.json"

MISSING_INSTALL_ROOT="$ROOT/runtime-missing-installed-package"
mkdir -m 700 "$MISSING_INSTALL_ROOT"
python3 - \
  "$RUNTIME_SNAPSHOT" \
  "$RUNTIME_REPLAY" \
  "$MISSING_INSTALL_ROOT/termux-runtime-snapshot-v1.json" \
  "$MISSING_INSTALL_ROOT/termux-runtime-snapshot-replay-v1.json" <<'PY'
import hashlib
import json
import pathlib
import sys

source_snapshot = pathlib.Path(sys.argv[1])
source_replay = pathlib.Path(sys.argv[2])
output_snapshot = pathlib.Path(sys.argv[3])
output_replay = pathlib.Path(sys.argv[4])
snapshot = json.loads(source_snapshot.read_text(encoding="utf-8"))
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
snapshot_raw = (
    json.dumps(snapshot, sort_keys=True, separators=(",", ":")) + "\n"
).encode()
output_snapshot.write_bytes(snapshot_raw)
replay = json.loads(source_replay.read_text(encoding="utf-8"))
replay["snapshot"]["manifest"]["sha256"] = hashlib.sha256(snapshot_raw).hexdigest()
replay["snapshot"]["manifest"]["bytes"] = len(snapshot_raw)
replay["installedPackages"] = {
    "sha256": snapshot["installedPackages"]["sha256"],
    "count": snapshot["installedPackages"]["count"],
}
output_replay.write_text(
    json.dumps(replay, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
PY
chmod 600 \
  "$MISSING_INSTALL_ROOT/termux-runtime-snapshot-v1.json" \
  "$MISSING_INSTALL_ROOT/termux-runtime-snapshot-replay-v1.json"
RUNTIME_SNAPSHOT_OVERRIDE="$MISSING_INSTALL_ROOT/termux-runtime-snapshot-v1.json" \
RUNTIME_REPLAY_OVERRIDE="$MISSING_INSTALL_ROOT/termux-runtime-snapshot-replay-v1.json" \
  assert_fails runtime_package_installation_mismatch \
  "$OUTPUTS/runtime-missing-installed-package/automated-qualification-v1.json"

mkdir -m 700 "$ROOT/runtime-snapshot-substitution"
cp -- "$RUNTIME_SNAPSHOT" \
  "$ROOT/runtime-snapshot-substitution/termux-runtime-snapshot-v1.json"
printf ' ' >>"$ROOT/runtime-snapshot-substitution/termux-runtime-snapshot-v1.json"
chmod 600 "$ROOT/runtime-snapshot-substitution/termux-runtime-snapshot-v1.json"
RUNTIME_SNAPSHOT_OVERRIDE="$ROOT/runtime-snapshot-substitution/termux-runtime-snapshot-v1.json" \
  assert_fails runtime_replay_snapshot_mismatch \
  "$OUTPUTS/runtime-snapshot-substitution/automated-qualification-v1.json"

MIXED_RUNTIME_LINKER="$(make_variant \
  "$RUNTIME_REPLAY" "$ROOT/runtime-linker-mismatch" \
  termux-runtime-snapshot-replay-v1.json '.androidLinker.sha256 = ("0" * 64)')"
RUNTIME_REPLAY_OVERRIDE="$MIXED_RUNTIME_LINKER" \
  assert_fails retained_runtime_cross_document_mismatch \
  "$OUTPUTS/runtime-linker-mismatch/automated-qualification-v1.json"

mkdir -m 700 "$ROOT/runtime-public-mode"
cp -- "$RUNTIME_REPLAY" \
  "$ROOT/runtime-public-mode/termux-runtime-snapshot-replay-v1.json"
chmod 644 "$ROOT/runtime-public-mode/termux-runtime-snapshot-replay-v1.json"
RUNTIME_REPLAY_OVERRIDE="$ROOT/runtime-public-mode/termux-runtime-snapshot-replay-v1.json" \
  assert_fails runtime_replay_file_invalid \
  "$OUTPUTS/runtime-public-mode/automated-qualification-v1.json"

mkdir -m 700 "$ROOT/runtime-symlink"
ln -s -- "$RUNTIME_LOCK" \
  "$ROOT/runtime-symlink/termux-runtime-package-lock-v1.json"
RUNTIME_LOCK_OVERRIDE="$ROOT/runtime-symlink/termux-runtime-package-lock-v1.json" \
  assert_fails runtime_packageLock_file_invalid \
  "$OUTPUTS/runtime-symlink/automated-qualification-v1.json"

mkdir -m 700 "$ROOT/runtime-empty-archive"
: >"$ROOT/runtime-empty-archive/termux-qualified-runtime-image-v1.tar.gz"
chmod 600 "$ROOT/runtime-empty-archive/termux-qualified-runtime-image-v1.tar.gz"
RUNTIME_ARCHIVE_OVERRIDE="$ROOT/runtime-empty-archive/termux-qualified-runtime-image-v1.tar.gz" \
  assert_fails runtime_archive_file_invalid \
  "$OUTPUTS/runtime-empty-archive/automated-qualification-v1.json"

if run_package "$PASS_OUTPUT" >"$ROOT/last.stdout" 2>"$ROOT/last.stderr"; then
  fail_test "existing output was overwritten"
fi
grep -Fq 'reason=output_already_exists' "$ROOT/last.stderr" \
  || fail_test "existing output rejection changed"

STALE_POLICY="$(make_variant "$POLICY" "$ROOT/stale-policy" \
  release-qualification-policy-v1.json '.branch = "release"')"
assert_fails policy_stale_or_modified "$OUTPUTS/stale-policy/automated-qualification-v1.json" \
  "$STALE_POLICY"

STALE_SCENARIO="$(make_variant "$SCENARIO_SET" "$ROOT/stale-scenario" \
  automated-native-deployment-scenarios-v1.json '.scenarios[0].expectedOutcome = "clean"')"
assert_fails scenario_set_stale_or_modified "$OUTPUTS/stale-scenario/automated-qualification-v1.json" \
  "$POLICY" "$STALE_SCENARIO"

EXTRA_AGGREGATE="$(make_variant "$AGGREGATE" "$ROOT/extra-aggregate" \
  termux-native-aggregate-evidence-v4.json '.unexpected = true')"
assert_fails aggregate_evidence_invalid "$OUTPUTS/extra-aggregate/automated-qualification-v1.json" \
  "$POLICY" "$SCENARIO_SET" "$EXTRA_AGGREGATE"

TYPE_AGGREGATE="$(make_variant "$AGGREGATE" "$ROOT/type-aggregate" \
  termux-native-aggregate-evidence-v4.json '.schemaVersion = "4"')"
assert_fails aggregate_evidence_invalid "$OUTPUTS/type-aggregate/automated-qualification-v1.json" \
  "$POLICY" "$SCENARIO_SET" "$TYPE_AGGREGATE"

MISSING_GATE="$(make_variant "$AGGREGATE" "$ROOT/missing-gate" \
  termux-native-aggregate-evidence-v4.json 'del(.aggregateValidation.boundedCleanup)')"
assert_fails aggregate_gate_missing "$OUTPUTS/missing-gate/automated-qualification-v1.json" \
  "$POLICY" "$SCENARIO_SET" "$MISSING_GATE"

REVERSED_TIME="$(make_variant "$AGGREGATE" "$ROOT/reversed-time" \
  termux-native-aggregate-evidence-v4.json \
  '.startedAt = "2026-07-23T00:02:00Z" | .completedAt = "2026-07-23T00:01:00Z"')"
assert_fails aggregate_timestamp_invalid "$OUTPUTS/reversed-time/automated-qualification-v1.json" \
  "$POLICY" "$SCENARIO_SET" "$REVERSED_TIME"

MIXED_CLASSIFIER="$(make_variant "$CLASSIFIER" "$ROOT/mixed-classifier" \
  termux-observation-requirement-v3.json \
  ".candidate.commit = \"$OTHER_COMMIT\"")"
assert_fails classifier_candidate_mismatch "$OUTPUTS/mixed-classifier/automated-qualification-v1.json" \
  "$POLICY" "$SCENARIO_SET" "$AGGREGATE" "$DEPLOYMENT" "$MIXED_CLASSIFIER"

INCONSISTENT_CLASSIFIER="$(make_variant "$CLASSIFIER" "$ROOT/inconsistent-classifier" \
  termux-observation-requirement-v3.json '.changedInputClasses = []')"
assert_fails classifier_evidence_invalid \
  "$OUTPUTS/inconsistent-classifier/automated-qualification-v1.json" \
  "$POLICY" "$SCENARIO_SET" "$AGGREGATE" "$DEPLOYMENT" "$INCONSISTENT_CLASSIFIER"

FIXTURE_DEPLOYMENT="$(make_variant "$DEPLOYMENT" "$ROOT/fixture-deployment" \
  automated-native-deployment-v1.json '.status = "fixture"')"
assert_fails deployment_evidence_invalid "$OUTPUTS/fixture-deployment/automated-qualification-v1.json" \
  "$POLICY" "$SCENARIO_SET" "$AGGREGATE" "$FIXTURE_DEPLOYMENT"

MISSING_DEPLOYMENT_GATE="$(make_variant "$DEPLOYMENT" "$ROOT/missing-deployment-gate" \
  automated-native-deployment-v1.json 'del(.validation.rollbackRecovery)')"
assert_fails deployment_gate_missing "$OUTPUTS/missing-deployment-gate/automated-qualification-v1.json" \
  "$POLICY" "$SCENARIO_SET" "$AGGREGATE" "$MISSING_DEPLOYMENT_GATE"

MIXED_SPECIALIZED="$(make_variant "$BATTERY" "$ROOT/mixed-specialized" \
  termux-battery-emulated-evidence.json \
  ".candidate.commit = \"$OTHER_COMMIT\"")"
assert_fails specialized_candidate_mismatch "$OUTPUTS/mixed-specialized/automated-qualification-v1.json" \
  "$POLICY" "$SCENARIO_SET" "$AGGREGATE" "$DEPLOYMENT" "$CLASSIFIER" "$MIXED_SPECIALIZED"

MIXED_SPECIALIZED_ENV="$(make_variant "$BATTERY" "$ROOT/mixed-specialized-environment" \
  termux-battery-emulated-evidence.json \
  '.environment.rootfsImageId = ("sha256:" + ("f" * 64))')"
assert_fails specialized_environment_invalid \
  "$OUTPUTS/mixed-specialized-environment/automated-qualification-v1.json" \
  "$POLICY" "$SCENARIO_SET" "$AGGREGATE" "$DEPLOYMENT" "$CLASSIFIER" \
  "$MIXED_SPECIALIZED_ENV"

SPECIALIZED_DUPLICATE_DIR="$ROOT/specialized-duplicate"
mkdir -m 700 "$SPECIALIZED_DUPLICATE_DIR"
sed '0,/{/s//{"schemaVersion":3,/' "$BATTERY" \
  >"$SPECIALIZED_DUPLICATE_DIR/termux-battery-emulated-evidence.json"
chmod 600 "$SPECIALIZED_DUPLICATE_DIR/termux-battery-emulated-evidence.json"
assert_fails duplicate_json_key \
  "$OUTPUTS/specialized-duplicate/automated-qualification-v1.json" \
  "$POLICY" "$SCENARIO_SET" "$AGGREGATE" "$DEPLOYMENT" "$CLASSIFIER" \
  "$SPECIALIZED_DUPLICATE_DIR/termux-battery-emulated-evidence.json"

DUPLICATE_DIR="$ROOT/duplicate"
mkdir -m 700 "$DUPLICATE_DIR"
sed '0,/{/s//{\"schemaVersion\":4,/' "$AGGREGATE" \
  >"$DUPLICATE_DIR/termux-native-aggregate-evidence-v4.json"
chmod 600 "$DUPLICATE_DIR/termux-native-aggregate-evidence-v4.json"
assert_fails duplicate_json_key "$OUTPUTS/duplicate/automated-qualification-v1.json" \
  "$POLICY" "$SCENARIO_SET" "$DUPLICATE_DIR/termux-native-aggregate-evidence-v4.json"

TRAILING_DIR="$ROOT/trailing"
mkdir -m 700 "$TRAILING_DIR"
cp "$AGGREGATE" "$TRAILING_DIR/termux-native-aggregate-evidence-v4.json"
printf '{}\n' >>"$TRAILING_DIR/termux-native-aggregate-evidence-v4.json"
chmod 600 "$TRAILING_DIR/termux-native-aggregate-evidence-v4.json"
assert_fails aggregate_evidence_json_invalid "$OUTPUTS/trailing/automated-qualification-v1.json" \
  "$POLICY" "$SCENARIO_SET" "$TRAILING_DIR/termux-native-aggregate-evidence-v4.json"

NONFINITE_DIR="$ROOT/nonfinite"
mkdir -m 700 "$NONFINITE_DIR"
sed '0,/"schemaVersion": 4/s//"schemaVersion": NaN/' "$AGGREGATE" \
  >"$NONFINITE_DIR/termux-native-aggregate-evidence-v4.json"
chmod 600 "$NONFINITE_DIR/termux-native-aggregate-evidence-v4.json"
assert_fails non_finite_json_number "$OUTPUTS/nonfinite/automated-qualification-v1.json" \
  "$POLICY" "$SCENARIO_SET" "$NONFINITE_DIR/termux-native-aggregate-evidence-v4.json"

TAMPERED="$BUNDLES/full-suite/termux-mcp-server"
cp "$TAMPERED" "$ROOT/full-suite.original"
printf '# tampered\n' >>"$TAMPERED"
assert_fails artifact_checksum_mismatch "$OUTPUTS/tampered/automated-qualification-v1.json"
mv "$ROOT/full-suite.original" "$TAMPERED"
chmod 700 "$TAMPERED"

grep -Fq 'COMMIT_HELPER="$SCRIPT_DIR/commit_verified_file.py"' "$SCRIPT" \
  || fail_test "packager does not resolve the shared held-fd commit helper"
grep -Fq 'not os.path.isabs(commit_helper)' "$SCRIPT" \
  || fail_test "packager does not require an absolute commit-helper path"
grep -Fq '"--source",' "$SCRIPT" \
  || fail_test "packager does not pass its private candidate to the commit helper"
grep -Fq '"--destination",' "$SCRIPT" \
  || fail_test "packager does not pass the public output to the commit helper"
grep -Fq '"--sha256",' "$SCRIPT" \
  || fail_test "packager does not bind the candidate digest at commit"
grep -Fq '"--mode",' "$SCRIPT" \
  || fail_test "packager does not bind the candidate mode at commit"
if grep -Eq 'published_identity|publication_complete|output[.]unlink[(]' "$SCRIPT"; then
  fail_test "packager retains retired post-publication rollback logic"
fi
grep -Fq 'signal.SIGINT, signal.SIGTERM, signal.SIGHUP' "$SCRIPT" \
  || fail_test "packager publication cleanup is not signal-aware"

# Model both sides of an ambiguous helper result without weakening the real
# held-fd helper. A successful no-replace link followed by a nonzero wrapper
# result must leave the valid public inode in place, while a foreign
# destination inserted before the link must be preserved byte-for-byte.
WRAPPER_ROOT="$ROOT/commit-helper-wrapper"
mkdir -m 700 "$WRAPPER_ROOT"
cp -- "$SCRIPT" "$WRAPPER_ROOT/package_automated_qualification.sh"
printf '%s\n' \
  '#!/usr/bin/env python3' \
  'import os' \
  'import pathlib' \
  'import subprocess' \
  'import sys' \
  'real = os.environ["REAL_COMMIT_HELPER"]' \
  'destination = pathlib.Path(sys.argv[sys.argv.index("--destination") + 1])' \
  'if "foreign-race" in str(destination):' \
  '    descriptor = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)' \
  '    with os.fdopen(descriptor, "wb") as handle:' \
  '        handle.write(b"foreign destination\n")' \
  '        handle.flush()' \
  '        os.fsync(handle.fileno())' \
  'result = subprocess.run([sys.executable, real, *sys.argv[1:]], check=False)' \
  'if "ambiguous-success" in str(destination) and result.returncode == 0:' \
  '    raise SystemExit(1)' \
  'raise SystemExit(result.returncode)' \
  >"$WRAPPER_ROOT/commit_verified_file.py"
chmod 600 "$WRAPPER_ROOT/commit_verified_file.py"
ORIGINAL_SCRIPT="$SCRIPT"
export REAL_COMMIT_HELPER="$REPO_ROOT/scripts/commit_verified_file.py"
SCRIPT="$WRAPPER_ROOT/package_automated_qualification.sh"

AMBIGUOUS_OUTPUT="$OUTPUTS/ambiguous-success/automated-qualification-v1.json"
mkdir -m 700 "$(dirname "$AMBIGUOUS_OUTPUT")"
if run_package "$AMBIGUOUS_OUTPUT" >"$ROOT/ambiguous.stdout" 2>"$ROOT/ambiguous.stderr"; then
  fail_test "ambiguous post-link helper result unexpectedly passed"
fi
grep -Fq 'reason=output_publication_failed' "$ROOT/ambiguous.stderr" \
  || fail_test "ambiguous post-link helper result reported the wrong failure"
cmp -s -- "$PASS_OUTPUT" "$AMBIGUOUS_OUTPUT" \
  || fail_test "ambiguous post-link helper result removed or changed the valid output"
[[ "$(stat -c '%a' "$AMBIGUOUS_OUTPUT")" == 600 ]] \
  || fail_test "ambiguous post-link output mode changed"

FOREIGN_OUTPUT="$OUTPUTS/foreign-race/automated-qualification-v1.json"
mkdir -m 700 "$(dirname "$FOREIGN_OUTPUT")"
if run_package "$FOREIGN_OUTPUT" >"$ROOT/foreign.stdout" 2>"$ROOT/foreign.stderr"; then
  fail_test "foreign destination race unexpectedly passed"
fi
grep -Fq 'reason=output_publication_failed' "$ROOT/foreign.stderr" \
  || fail_test "foreign destination race reported the wrong failure"
[[ "$(<"$FOREIGN_OUTPUT")" == "foreign destination" ]] \
  || fail_test "foreign destination race did not preserve foreign bytes"
[[ "$(stat -c '%a' "$FOREIGN_OUTPUT")" == 600 ]] \
  || fail_test "foreign destination race changed the foreign mode"

SCRIPT="$ORIGINAL_SCRIPT"
unset REAL_COMMIT_HELPER

printf 'Automated qualification package tests passed\n'
