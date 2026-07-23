#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

ROOT="$(mktemp -d)"
trap 'rm -rf -- "$ROOT"' EXIT INT TERM
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/stage_release_assets.sh"
PACKAGER="$REPO_ROOT/scripts/package_physical_qualification.sh"
SCHEMA="$REPO_ROOT/docs/release-staging-manifest-schema-v1.json"
REAL_PATH="$PATH"
REAL_CP="$(command -v cp)"
REAL_MV="$(command -v mv)"
COMMIT=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
VERSION=0.6.0
CI_RUN_ID=4101
SECURITY_RUN_ID=4102
ANDROID_RUN_ID=4103
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
          imageDigest:("sha256:" + ("b" * 64)),
          androidLinker:true
        },
        validation:({status:"pass",requests:29,exactArtifact:true,compileGate:true}
          + (if $mode == "volume-control" or $mode == "command" then {longObservationRequired:false} else {} end))
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
        androidVolumeControlArtifact:{sha256:$volume_control_sha,bytes:$volume_control_bytes},
        fullSuiteArtifact:{
          sha256:$full_suite_sha,bytes:$full_suite_bytes,manifestSha256:$full_manifest_sha,
          artifactName:"termux-mcp-server-aarch64-linux-android-full-suite",
          posture:"full-suite",features:["full-suite"],fileName:"termux-mcp-server"
        }
      },
      environment:{
        executionMode:"official-termux-docker-native-arm64",architecture:"aarch64",
        image:"termux/termux-docker:aarch64",imageDigest:("sha256:" + ("c" * 64)),androidLinker:true
      },
      runtimeValidation:{status:"pass",reportSha256:("d" * 64),resultCount:20,phases:{preflight:"pass",runtime:"pass",deployment:"not_run"}},
      aggregateValidation:{
        status:"pass",requests:20,
        defaultDisabled:{toolCount:17,exactToolOrder:true,optionalFeaturesCompiled:true,optionalToolsHidden:true,runtimeFlagsOmitted:true},
        fullyEnabled:{
          toolCount:21,exactToolOrder:true,allOptionalToolsExposed:true,providerSuccesses:true,
          volumePreviewNoMutation:true,volumeGrantIsolation:true,commandExecutableIdentityPinned:true
        },
        independentRuntimeGates:true,filesystemMutationsDisabled:true,boundedCleanup:true,directPhysicalObservationRequired:true
      },
      stress:{status:"pass",samples:64,requests:128,servicePidStable:true,healthReadyStable:true,longObservationRequired:false}
    }
  ' >"$BASE/emulated/termux-emulated-evidence.json"

make_specialized_evidence "$BASE/emulated/termux-battery-emulated-evidence.json" 2 2 2 battery
make_specialized_evidence "$BASE/emulated/termux-volume-emulated-evidence.json" 1 1 3 volume
make_specialized_evidence "$BASE/emulated/termux-volume-control-emulated-evidence.json" 1 1 4 volume-control 3
make_specialized_evidence "$BASE/emulated/termux-command-emulated-evidence.json" 2 2 5 command 0

aggregate_sha="$(sha256sum "$BASE/emulated/termux-emulated-evidence.json" | awk '{print $1}')"
jq -n \
  --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg full_suite_sha "$(bundle_sha full-suite)" --arg full_manifest_sha "$full_manifest_sha" \
  --arg aggregate_sha "$aggregate_sha" '
    {
      schemaVersion:2,classifierVersion:"2",status:"pass",failureCode:null,releaseQualificationEligible:false,
      createdAt:"2026-07-22T00:03:00Z",evidenceMode:"physical_observation_required",
      reasonCode:"full_suite_direct_physical_observation_required",inheritanceCandidate:false,
      source:{},
      candidate:{
        commit:$commit,version:$version,ciRunId:$ci,securityRunId:$security,androidRunId:$android,
        fullSuiteArtifactSha256:$full_suite_sha,fullSuiteManifestSha256:$full_manifest_sha
      },
      emulation:{
        reportSha256:$aggregate_sha,executionMode:"official-termux-docker-native-arm64",
        imageDigest:("sha256:" + ("e" * 64)),status:"pass",samples:64
      },
      protectedInputComparison:{runtimeAndDeploymentInputsUnchanged:false,cargoAndDependencyInputsUnchangedExceptRootVersion:false},
      changedInputClasses:["full_suite_artifact"],nextGate:"direct_physical_device_observation"
    }
  ' >"$BASE/emulated/termux-observation-requirement.json"

jq -n \
  --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" \
  --arg default_sha "$(bundle_sha default)" --argjson default_bytes "$(bundle_bytes default)" \
  --arg mcp_sha "$(bundle_sha mcp-runtime)" --argjson mcp_bytes "$(bundle_bytes mcp-runtime)" \
  --arg volume_control_sha "$(bundle_sha android-volume-control)" --argjson volume_control_bytes "$(bundle_bytes android-volume-control)" \
  --arg full_suite_sha "$(bundle_sha full-suite)" --argjson full_suite_bytes "$(bundle_bytes full-suite)" '
    {
      schemaVersion:2,validatorVersion:"11",status:"pass",failureCode:null,releaseEligible:true,
      startedAt:"2026-07-22T00:00:00Z",completedAt:"2026-07-22T01:05:00Z",
      repository:{commit:$commit,version:$version,ciRunId:$ci,securityRunId:$security,androidRunId:$android},
      environment:{architecture:"aarch64",fixtureMode:false,tools:{bash:"bash",curl:"curl",file:"file",jq:"jq"}},
      requestedPhase:"all",
      artifacts:{
        default:{sha256:$default_sha,bytes:$default_bytes,version:$version,elf:"aarch64-android-elf"},
        mcpRuntime:{sha256:$mcp_sha,bytes:$mcp_bytes,version:$version,elf:"aarch64-android-elf"},
        androidVolumeControl:{sha256:$volume_control_sha,bytes:$volume_control_bytes,version:$version,elf:"aarch64-android-elf"},
        fullSuite:{sha256:$full_suite_sha,bytes:$full_suite_bytes,version:$version,elf:"aarch64-android-elf"},
        baseline:{sha256:("9" * 64),bytes:900,version:"0.5.1",elf:"aarch64-android-elf"}
      },
      deploymentCandidate:{posture:"full-suite",productionAction:null},
      phases:{preflight:"pass",runtime:"pass",deployment:"pass"},
      results:[
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
      sustainedObservation:{operatorSupplied:true,status:"pass",minutes:65,reasonCode:"stable",minimumMinutes:60}
    }
  ' >"$BASE/validator.json"
chmod 600 "$BASE/validator.json"
printf '%s\n' \
  'harness_version=11' \
  'architecture=aarch64' \
  "candidate_version=$VERSION" \
  "exact_head=$COMMIT" \
  "candidate_sha256=$(printf '8%.0s' {1..64})" \
  "mcp_runtime_sha256=$(printf '6%.0s' {1..64})" \
  "volume_control_sha256=$(printf '7%.0s' {1..64})" \
  "full_suite_sha256=$(printf '8%.0s' {1..64})" \
  'TERMUX_MCP_DEVICE_RESULT=PASS' \
  'cleanup_complete=true' \
  'final_status=PASS' >"$BASE/harness.txt"
chmod 600 "$BASE/harness.txt"
mkdir -m 700 "$BASE/physical-parent"
bash "$PACKAGER" \
  --validator-report "$BASE/validator.json" \
  --harness-report "$BASE/harness.txt" \
  --output-dir "$BASE/physical-parent/bundle" >/dev/null
cmp -s "$BASE/validator.json" "$BASE/physical-parent/bundle/release-validator-v11.json" \
  || fail_test 'physical packager changed validator fixture bytes'
cp "$BASE/physical-parent/bundle/physical-qualification-v1.json" "$BASE/physical.json"
rm -rf "$BASE/physical-parent" "$BASE/harness.txt"

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
    --validator-evidence "$case_root/validator.json" \
    --physical-qualification "$case_root/physical.json" \
    --license "$case_root/LICENSE" \
    --repository CyberBASSLord-666/termux-mcp-edge \
    --commit "$COMMIT" \
    --version "$VERSION" \
    --ci-run-id "$CI_RUN_ID" \
    --security-run-id "$SECURITY_RUN_ID" \
    --android-run-id "$ANDROID_RUN_ID" \
    --output "$output_dir/$output_name"
}

bash -n "$SCRIPT"
jq -e '
  .type == "object"
  and .additionalProperties == false
  and .properties.publicationState.const == "staged_not_released"
  and .properties.releaseEligible.const == false
  and .properties.artifacts.minItems == 7
  and .properties.artifacts.maxItems == 7
  and .properties.evidence.properties.specialized.minItems == 5
  and .properties.evidence.properties.specialized.maxItems == 5
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
[[ "$(find "$ROOT/extracted" -type f | wc -l)" == 32 ]] || fail_test 'staging archive file set is not exact'
[[ -z "$(find "$ROOT/extracted" -type l -print -quit)" ]] || fail_test 'staging archive contains a link'
[[ "$(stat -c '%a' "$ROOT/extracted")" == 755 ]] || fail_test 'archive root mode is not normalized'
[[ "$(stat -c '%a' "$ROOT/extracted/evidence")" == 755 ]] || fail_test 'evidence directory mode is not normalized'

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

cmp -s "$SUCCESS_CASE/emulated/termux-emulated-evidence.json" "$ROOT/extracted/evidence/emulated-release-v3.json" \
  || fail_test 'aggregate evidence bytes changed'
cmp -s "$SUCCESS_CASE/validator.json" "$ROOT/extracted/evidence/release-validator-v11.json" \
  || fail_test 'validator evidence bytes changed'
cmp -s "$SUCCESS_CASE/physical.json" "$ROOT/extracted/evidence/physical-qualification-v1.json" \
  || fail_test 'physical qualification bytes changed'
cmp -s "$SUCCESS_CASE/LICENSE" "$ROOT/extracted/LICENSE" || fail_test 'license bytes changed'

jq -e \
  --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg ci "$CI_RUN_ID" --arg security "$SECURITY_RUN_ID" --arg android "$ANDROID_RUN_ID" '
    (keys == ["artifacts","checksums","commit","evidence","license","publicationState","releaseEligible","repository","schemaVersion","target","version","workflowRuns"])
    and .schemaVersion == 1
    and .publicationState == "staged_not_released"
    and .releaseEligible == false
    and .repository == "CyberBASSLord-666/termux-mcp-edge"
    and .commit == $commit
    and .version == $version
    and .workflowRuns == {ci:$ci,security:$security,android:$android}
    and (.artifacts | length == 7)
    and ([.artifacts[].posture] == ["default","mcp-runtime","android-battery-status","android-volume-status","android-volume-control","command-execution","full-suite"])
    and .evidence.aggregate.fileName == "evidence/emulated-release-v3.json"
    and .evidence.validator.fileName == "evidence/release-validator-v11.json"
    and .evidence.physicalQualification.fileName == "evidence/physical-qualification-v1.json"
    and (.evidence.specialized | length == 5)
  ' "$ROOT/extracted/release-staging-manifest-v1.json" >/dev/null \
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
jq '.candidate.fullSuiteArtifact.sha256 = ("0" * 64)' "$case_root/emulated/termux-emulated-evidence.json" >"$case_root/aggregate.next"
mv "$case_root/aggregate.next" "$case_root/emulated/termux-emulated-evidence.json"
assert_fails aggregate_evidence_mismatch run_stage "$case_root" "$ROOT/outputs/aggregate-digest"

case_root="$(make_case specialized-run)"
jq '.candidate.androidRunId = "9999"' "$case_root/emulated/termux-battery-emulated-evidence.json" >"$case_root/battery.next"
mv "$case_root/battery.next" "$case_root/emulated/termux-battery-emulated-evidence.json"
assert_fails specialized_evidence_mismatch run_stage "$case_root" "$ROOT/outputs/specialized-run"

case_root="$(make_case classifier-lineage)"
jq '.candidate.fullSuiteManifestSha256 = ("0" * 64)' "$case_root/emulated/termux-observation-requirement.json" >"$case_root/classifier.next"
mv "$case_root/classifier.next" "$case_root/emulated/termux-observation-requirement.json"
assert_fails observation_requirement_mismatch run_stage "$case_root" "$ROOT/outputs/classifier-lineage"

case_root="$(make_case validator-eligibility)"
jq '.releaseEligible = false' "$case_root/validator.json" >"$case_root/validator.next"
mv "$case_root/validator.next" "$case_root/validator.json"
assert_fails validator_evidence_mismatch run_stage "$case_root" "$ROOT/outputs/validator-eligibility"

case_root="$(make_case validator-baseline-missing)"
jq '.artifacts.baseline = null' "$case_root/validator.json" >"$case_root/validator.next"
mv "$case_root/validator.next" "$case_root/validator.json"
assert_fails validator_evidence_mismatch run_stage "$case_root" "$ROOT/outputs/validator-baseline-missing"

case_root="$(make_case validator-required-result-missing)"
jq 'del(.results[] | select(.code == "full_suite_enabled_21_tool_posture_verified"))' \
  "$case_root/validator.json" >"$case_root/validator.next"
mv "$case_root/validator.next" "$case_root/validator.json"
assert_fails validator_evidence_mismatch run_stage "$case_root" "$ROOT/outputs/validator-required-result-missing"

case_root="$(make_case physical-cleanup)"
jq '.cleanupConfirmed = false' "$case_root/physical.json" >"$case_root/physical.next"
mv "$case_root/physical.next" "$case_root/physical.json"
assert_fails physical_qualification_mismatch run_stage "$case_root" "$ROOT/outputs/physical-cleanup"

case_root="$(make_case physical-validator-digest)"
jq '.validatorReportSha256 = ("0" * 64)' "$case_root/physical.json" >"$case_root/physical.next"
mv "$case_root/physical.next" "$case_root/physical.json"
assert_fails physical_qualification_mismatch run_stage "$case_root" "$ROOT/outputs/physical-validator-digest"

case_root="$(make_case license-link)"
mv "$case_root/LICENSE" "$case_root/LICENSE.real"
ln -s LICENSE.real "$case_root/LICENSE"
assert_fails license_invalid run_stage "$case_root" "$ROOT/outputs/license-link"

case_root="$(make_case validator-link)"
mv "$case_root/validator.json" "$case_root/validator.real.json"
ln -s validator.real.json "$case_root/validator.json"
assert_fails validator_evidence_invalid run_stage "$case_root" "$ROOT/outputs/validator-link"

case_root="$(make_case validator-oversized)"
dd if=/dev/zero bs=1048576 count=1 status=none | tr '\0' ' ' >>"$case_root/validator.json"
assert_fails validator_evidence_invalid run_stage "$case_root" "$ROOT/outputs/validator-oversized"

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
if [[ "$source_path" == */termux-emulated-evidence.json ]]; then
  printf '{}\n' >"$source_path"
fi
EOF
chmod 700 "$ROOT/racing-cp/cp"
aggregate_sha_before="$(sha256sum "$case_root/emulated/termux-emulated-evidence.json" | awk '{print $1}')"
STAGE_REAL_CP="$REAL_CP" STAGE_TEST_PATH="$ROOT/racing-cp:$ROOT/fake-bin:$REAL_PATH" \
  run_stage "$case_root" "$ROOT/outputs/snapshot-race" >/dev/null
mkdir "$ROOT/snapshot-race-extracted"
tar -xf "$ROOT/outputs/snapshot-race/$OUTPUT_NAME" -C "$ROOT/snapshot-race-extracted"
[[ "$(sha256sum "$ROOT/snapshot-race-extracted/evidence/emulated-release-v3.json" | awk '{print $1}')" == "$aggregate_sha_before" ]] \
  || fail_test 'source replacement changed the validated snapshot'

case_root="$(make_case publication-race)"
mkdir -m 700 "$ROOT/racing-mv"
cat >"$ROOT/racing-mv/mv" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
target="${@: -1}"
printf 'concurrent-owner\n' >"$target"
"$STAGE_REAL_MV" "$@"
EOF
chmod 700 "$ROOT/racing-mv/mv"
STAGE_REAL_MV="$REAL_MV" STAGE_TEST_PATH="$ROOT/racing-mv:$ROOT/fake-bin:$REAL_PATH" \
  assert_fails archive_publication_failed run_stage "$case_root" "$ROOT/outputs/publication-race"
[[ "$(<"$ROOT/outputs/publication-race/$OUTPUT_NAME")" == concurrent-owner ]] \
  || fail_test 'no-clobber publication changed a concurrent output'
[[ -z "$(find "$ROOT/outputs/publication-race" -maxdepth 1 -name "$OUTPUT_NAME.staging.*" -print -quit)" ]] \
  || fail_test 'publication conflict leaked owned temporary state'

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
