#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077

ROOT="$(mktemp -d)"
chmod 700 "$ROOT"
declare -a SUPERVISOR_PID_FILES=()

pid_identity_running() {
  local pid="$1" proc_id="$2" expected="$3" stat suffix current state namespace_pid
  [[ "$pid" =~ ^[1-9][0-9]*$ && "$proc_id" =~ ^[1-9][0-9]*$ \
    && -r "/proc/$proc_id/stat" && -r "/proc/$proc_id/status" ]] || return 1
  namespace_pid="$(awk '/^NSpid:/{print $NF}' "/proc/$proc_id/status")"
  [[ "$namespace_pid" == "$pid" ]] || return 1
  IFS= read -r stat <"/proc/$proc_id/stat" || return 1
  [[ "$stat" == *") "* ]] || return 1
  suffix="${stat##*) }"
  current="$(awk '{print $20}' <<<"$suffix")"
  state="$(awk '{print $1}' <<<"$suffix")"
  [[ "$current" == "$expected" && "$state" != Z && "$state" != X ]]
}

cleanup_test() {
  local status=$? pid_file role pid proc_id start _
  trap - EXIT INT TERM
  for pid_file in "${SUPERVISOR_PID_FILES[@]}"; do
    [[ -f "$pid_file" ]] || continue
    while IFS=' ' read -r role pid proc_id start; do
      [[ -n "$role" ]] || continue
      pid_identity_running "$pid" "$proc_id" "$start" \
        && kill -TERM "$pid" >/dev/null 2>&1 || true
    done <"$pid_file"
  done
  for _ in $(seq 1 20); do
    local found=0
    for pid_file in "${SUPERVISOR_PID_FILES[@]}"; do
      [[ -f "$pid_file" ]] || continue
      while IFS=' ' read -r role pid proc_id start; do
        [[ -n "$role" ]] || continue
        pid_identity_running "$pid" "$proc_id" "$start" && found=1
      done <"$pid_file"
    done
    ((found == 0)) && break
    sleep 0.05
  done
  for pid_file in "${SUPERVISOR_PID_FILES[@]}"; do
    [[ -f "$pid_file" ]] || continue
    while IFS=' ' read -r role pid proc_id start; do
      [[ -n "$role" ]] || continue
      pid_identity_running "$pid" "$proc_id" "$start" \
        && kill -KILL "$pid" >/dev/null 2>&1 || true
    done <"$pid_file"
  done
  rm -rf -- "$ROOT"
  exit "$status"
}
trap cleanup_test EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/termux_automated_deployment_gate.sh"
SCHEMA="$REPO_ROOT/docs/automated-native-deployment-evidence-schema-v1.json"
SCENARIOS="$REPO_ROOT/docs/automated-native-deployment-scenarios-v1.json"
SCENARIO_SCHEMA="$REPO_ROOT/docs/automated-native-deployment-scenarios-schema-v1.json"
COMMIT=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
VERSION=0.6.0
CI_RUN_ID=1001
SECURITY_RUN_ID=1002
NATIVE_RUN_ID=1003
SCENARIO_SHA=dd31d4f89f9f25dba1a1bb1c492fd796f5a2619b215e2d57f3b0e60f9f24b3bb

fail_test() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

sha() {
  sha256sum -- "$1" | awk '{print $1}'
}

make_case() {
  local name="$1" case_root
  case_root="$ROOT/$name"
  mkdir -m 700 "$case_root" "$case_root/home" "$case_root/output" "$case_root/bundle"
  printf '%s\n' "$case_root"
}

write_bundle() {
  local bundle="$1"
  cat >"$bundle/termux-mcp-server" <<EOF
#!/bin/sh
if [ "\${1:-}" = "--version" ]; then
  printf '%s\n' 'termux-mcp-server $VERSION'
  exit 0
fi
exit 0
EOF
  chmod 700 "$bundle/termux-mcp-server"
  local artifact_sha artifact_bytes
  artifact_sha="$(sha "$bundle/termux-mcp-server")"
  artifact_bytes="$(stat -c %s "$bundle/termux-mcp-server")"
  printf '%s  %s\n' "$artifact_sha" termux-mcp-server >"$bundle/SHA256SUMS"
  jq -n \
    --arg commit "$COMMIT" \
    --arg run_id "$NATIVE_RUN_ID" \
    --arg version "$VERSION" \
    --arg sha256 "$artifact_sha" \
    --argjson bytes "$artifact_bytes" '
    {
      schemaVersion: 1,
      repository: "CyberBASSLord-666/termux-mcp-edge",
      commit: $commit,
      workflowRunId: $run_id,
      artifactName: "termux-mcp-server-aarch64-linux-android-full-suite",
      posture: "full-suite",
      features: ["full-suite"],
      target: "aarch64-linux-android",
      fileName: "termux-mcp-server",
      version: $version,
      sha256: $sha256,
      bytes: $bytes,
      elf: "aarch64-android-elf",
      createdAt: "2026-07-23T00:00:00Z"
    }
  ' >"$bundle/artifact-manifest.json"
  chmod 600 "$bundle/SHA256SUMS" "$bundle/artifact-manifest.json"
}

run_gate() {
  local case_root="$1"
  env \
    HOME="$case_root/home" \
    TERMUX_MCP_AUTOMATED_DEPLOYMENT_FIXTURE_MODE=1 \
    bash "$SCRIPT" \
      --artifact-dir "$case_root/bundle" \
      --expected-commit "$COMMIT" \
      --expected-version "$VERSION" \
      --ci-run-id "$CI_RUN_ID" \
      --security-run-id "$SECURITY_RUN_ID" \
      --native-run-id "$NATIVE_RUN_ID" \
      --output "$case_root/output/automated-native-deployment-v1.json"
}

run_gate_with_supervisor() {
  local case_root="$1" pid_file="$2" pause_path="${3:-}"
  env \
    HOME="$case_root/home" \
    TERMUX_MCP_AUTOMATED_DEPLOYMENT_FIXTURE_MODE=1 \
    TERMUX_MCP_AUTOMATED_DEPLOYMENT_TEST_RUNSVDIR="$FAKE_RUNSVDIR" \
    TERMUX_MCP_AUTOMATED_DEPLOYMENT_TEST_SUPERVISOR_PID_FILE="$pid_file" \
    TERMUX_MCP_AUTOMATED_DEPLOYMENT_TEST_PAUSE_AFTER_SUPERVISOR_START="$pause_path" \
    bash "$SCRIPT" \
      --artifact-dir "$case_root/bundle" \
      --expected-commit "$COMMIT" \
      --expected-version "$VERSION" \
      --ci-run-id "$CI_RUN_ID" \
      --security-run-id "$SECURITY_RUN_ID" \
      --native-run-id "$NATIVE_RUN_ID" \
      --output "$case_root/output/automated-native-deployment-v1.json"
}

assert_supervisor_pids_exited() {
  local pid_file="$1" role pid proc_id start count=0
  [[ -s "$pid_file" ]] || fail_test "fixture supervisor did not publish identities"
  while IFS=' ' read -r role pid proc_id start; do
    if [[ "$role" != runsvdir && "$role" != runsv && "$role" != service ]]; then
      sed -n '1,20p' "$pid_file" >&2
      fail_test "fixture supervisor published an unknown role"
    fi
    [[ "$pid" =~ ^[1-9][0-9]*$ && "$proc_id" =~ ^[1-9][0-9]*$ \
      && "$start" =~ ^[1-9][0-9]*$ ]] \
      || fail_test "fixture supervisor published an invalid identity"
    if pid_identity_running "$pid" "$proc_id" "$start"; then
      fail_test "isolated $role process survived bounded cleanup"
    fi
    count=$((count + 1))
  done <"$pid_file"
  [[ "$count" == 3 ]] || fail_test "fixture supervisor identity set was incomplete"
}

assert_case_clean() {
  local case_root="$1"
  [[ -z "$(find "$case_root/home" -mindepth 1 -maxdepth 1 \
    \( -name '.termux-mcp-automated.*' -o -name '.termux-mcp-automated-deployment-gate.lock' \) \
    -print -quit)" ]] || fail_test "gate left private work or lock state"
  if [[ -d "$case_root/home"/.termux-prefix/var/tmp ]]; then
    [[ -z "$(find "$case_root/home"/.termux-prefix/var/tmp -mindepth 1 -maxdepth 1 \
      -name 'termux-mcp-automated.*' -print -quit)" ]] \
      || fail_test "gate left service sandbox state"
  fi
}

assert_gate_fails() {
  local case_root="$1" reason="$2"
  if run_gate "$case_root" >"$case_root/stdout" 2>"$case_root/stderr"; then
    fail_test "gate unexpectedly succeeded for $reason"
  fi
  grep -Fq "reason=$reason" "$case_root/stderr" \
    || fail_test "failure reason $reason was absent"
  [[ ! -e "$case_root/output/automated-native-deployment-v1.json" ]] \
    || fail_test "failed gate published evidence for $reason"
  [[ -z "$(find "$case_root/output" -mindepth 1 -maxdepth 1 \
    -name '.automated-native-deployment.*' -print -quit)" ]] \
    || fail_test "failed gate left report staging for $reason"
  assert_case_clean "$case_root"
}

bash -n "$SCRIPT"
bash -n "${BASH_SOURCE[0]}"
jq empty "$SCHEMA"
jq empty "$SCENARIOS"
python3 - "$SCRIPT" <<'PY'
from pathlib import Path
import sys

lines = Path(sys.argv[1]).read_text(encoding="utf-8").splitlines()
for name in (
    "track_runsv_children",
    "collect_isolated_processes",
    "signal_tracked_processes",
):
    start = lines.index(f"{name}() {{")
    end = next(index for index in range(start + 1, len(lines)) if lines[index] == "}")
    if lines[end - 1] != "  return 0":
        raise SystemExit(f"{name} must return success explicitly")
PY
grep -Fq 'kill -HUP "$RUNSVDIR_PID"' "$SCRIPT" \
  || fail_test "runit shutdown does not use the runsvdir HUP contract"
if grep -Eq '(^|[[:space:]])pkill([[:space:]]|$)|terminate_pid_bounded' "$SCRIPT"; then
  fail_test "broad or parent-only supervisor cleanup remains"
fi
[[ "$(sha "$SCENARIOS")" == "$SCENARIO_SHA" ]] \
  || fail_test "committed scenario digest changed"

FAKE_RUNSVDIR="$ROOT/fake-runsvdir"
cat >"$FAKE_RUNSVDIR" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'

service_root="$1"
work_root="$2"
pid_file="$3"

process_identity() {
  local role="$1" pid="$2" namespace proc proc_id namespace_pid stat suffix start
  namespace="$(readlink /proc/self/ns/pid)"
  for proc in /proc/[1-9]*; do
    [[ -d "$proc" ]] || continue
    proc_id="${proc##*/}"
    [[ "$(readlink "/proc/$proc_id/ns/pid" 2>/dev/null || true)" == "$namespace" ]] \
      || continue
    namespace_pid="$(awk '/^NSpid:/{print $NF}' "/proc/$proc_id/status" 2>/dev/null || true)"
    [[ "$namespace_pid" == "$pid" ]] || continue
    IFS= read -r stat <"/proc/$proc_id/stat"
    suffix="${stat##*) }"
    start="$(awk '{print $20}' <<<"$suffix")"
    printf '%s %s %s %s\n' "$role" "$pid" "$proc_id" "$start"
    return 0
  done
  return 1
}

idle_in() {
  local directory="$1"
  cd "$directory"
  trap 'exit 0' TERM
  trap '' HUP
  while :; do
    if read -r -t 1 _; then
      :
    fi
  done
}

idle_in "$service_root" &
runsv_pid=$!
idle_in "$work_root" &
service_pid=$!

shutdown() {
  trap - HUP TERM
  kill -TERM "$service_pid" "$runsv_pid" >/dev/null 2>&1 || true
  wait "$service_pid" 2>/dev/null || true
  wait "$runsv_pid" 2>/dev/null || true
  exit 0
}
trap shutdown HUP
# This deliberately ignores TERM. A parent-only TERM/KILL cleanup orphans both
# isolated children and therefore fails the gate's adversarial cleanup test.
trap ':' TERM

identity_next="$pid_file.next"
rm -f -- "$identity_next"
{
  process_identity runsvdir "$$"
  process_identity runsv "$runsv_pid"
  process_identity service "$service_pid"
} >"$identity_next"
chmod 600 "$identity_next"
mv -T "$identity_next" "$pid_file"

while :; do
  if read -r -t 1 _; then
    :
  fi
done
EOF
chmod 700 "$FAKE_RUNSVDIR"

jq -e '
  ."$schema" == "https://json-schema.org/draft/2020-12/schema"
  and .type == "object"
  and .additionalProperties == false
  and (.required | length) == 12
  and (.required | unique | length) == 12
  and (.properties | keys == [
    "candidate",
    "completedAt",
    "environment",
    "failureCode",
    "gateVersion",
    "qualificationClass",
    "releaseQualificationEligible",
    "scenarioSet",
    "schemaVersion",
    "startedAt",
    "status",
    "validation"
  ])
  and .properties.releaseQualificationEligible.const == false
  and .properties.qualificationClass.const == "official_termux_native_automated_v1"
  and .properties.scenarioSet.properties.fileName.const == "automated-native-deployment-scenarios-v1.json"
  and .properties.scenarioSet.properties.scenarioCount.const == 6
  and .properties.environment.additionalProperties == false
  and (.properties.environment.required | index("runtimeImageDigest")) != null
  and (.properties.environment.required | index("rootfsImageId")) != null
  and .properties.environment.properties.rootfsImageId.oneOf[1]."$ref" == "#/$defs/imageDigest"
  and .properties.environment.properties.runtimeImageDigest.oneOf[1]."$ref" == "#/$defs/imageDigest"
  and .allOf[0].then.properties.environment.properties.rootfsImageId."$ref" == "#/$defs/imageDigest"
  and .allOf[1].then.properties.environment.properties.rootfsImageId.const == null
  and .allOf[0].then.properties.environment.properties.runtimeImageDigest."$ref" == "#/$defs/imageDigest"
  and .allOf[1].then.properties.environment.properties.runtimeImageDigest.const == null
  and .properties.environment.properties.androidFrameworkObserved.const == false
  and .properties.environment.properties.physicalHardwareObserved.const == false
  and .properties.environment.properties.physicalDeviceObserved.const == false
  and .properties.environment.properties.sustainedPhysicalSoak.const == false
  and .properties.validation.properties.physicalCertification.const == "not_run"
  and ."$defs".failedUpgradeResult.allOf[1].properties.faultBoundary.const == "target_readiness_probe"
  and ."$defs".rollbackRecoveryResult.allOf[1].properties.faultBoundary.const == "target_readiness_probe"
' "$SCHEMA" >/dev/null

jq -e '
  ."$schema" == "https://json-schema.org/draft/2020-12/schema"
  and .type == "object"
  and .additionalProperties == false
  and .required == [
    "schemaVersion",
    "scenarioSetVersion",
    "qualificationClass",
    "scenarios"
  ]
  and .properties.schemaVersion.const == 1
  and .properties.scenarioSetVersion.const == "1"
  and .properties.qualificationClass.const == "official_termux_native_automated_v1"
  and .properties.scenarios.minItems == 6
  and .properties.scenarios.maxItems == 6
  and .properties.scenarios.uniqueItems == true
  and .properties.scenarios.items == false
  and ([.properties.scenarios.prefixItems[].const.id] == [
    "isolated_fresh_deploy",
    "failed_upgrade_recovery",
    "supervised_restart",
    "rollback_recovery",
    "uninstall",
    "bounded_cleanup"
  ])
  and .properties.scenarios.prefixItems[1].const.faultInjection
    == "target_scoped_readiness_probe_failure"
  and .properties.scenarios.prefixItems[2].const.faultInjection
    == "supervised_process_termination"
  and .properties.scenarios.prefixItems[3].const.faultInjection
    == "target_scoped_readiness_probe_failure"
' "$SCENARIO_SCHEMA" >/dev/null

jq -e '
  keys == ["qualificationClass","scenarioSetVersion","scenarios","schemaVersion"]
  and .schemaVersion == 1
  and .scenarioSetVersion == "1"
  and .qualificationClass == "official_termux_native_automated_v1"
  and (.scenarios | length) == 6
  and ([.scenarios[].id] == [
    "isolated_fresh_deploy",
    "failed_upgrade_recovery",
    "supervised_restart",
    "rollback_recovery",
    "uninstall",
    "bounded_cleanup"
  ])
  and ([.scenarios[].id] | unique | length) == 6
  and .scenarios[1].faultInjection == "target_scoped_readiness_probe_failure"
  and .scenarios[3].faultInjection == "target_scoped_readiness_probe_failure"
' "$SCENARIOS" >/dev/null

HELP_ROOT="$(make_case help)"
bash "$SCRIPT" --help >"$HELP_ROOT/stdout" 2>"$HELP_ROOT/stderr"
grep -Fq 'automated-native-deployment-v1.json' "$HELP_ROOT/stdout" \
  || fail_test "help omitted canonical output filename"
grep -Fq 'six-scenario deployment contract' "$HELP_ROOT/stdout" \
  || fail_test "help omitted scenario contract"
grep -Fq 'never standalone release authority' "$HELP_ROOT/stdout" \
  || fail_test "help omitted authority boundary"
grep -Fq 'does not' "$HELP_ROOT/stdout" \
  || fail_test "help omitted negative observation boundary"
[[ ! -s "$HELP_ROOT/stderr" ]] || fail_test "help wrote stderr"

PASS_ROOT="$(make_case pass)"
write_bundle "$PASS_ROOT/bundle"
ARTIFACT_SHA="$(sha "$PASS_ROOT/bundle/termux-mcp-server")"
MANIFEST_SHA="$(sha "$PASS_ROOT/bundle/artifact-manifest.json")"
if ! run_gate "$PASS_ROOT" >"$PASS_ROOT/stdout" 2>"$PASS_ROOT/stderr"; then
  sed -n '1,120p' "$PASS_ROOT/stderr" >&2
  fail_test "fixture success case failed"
fi
[[ "$(<"$PASS_ROOT/stdout")" == *"TERMUX_MCP_AUTOMATED_DEPLOYMENT_RESULT=FIXTURE"* ]] \
  || fail_test "fixture success output contract changed"
if [[ -s "$PASS_ROOT/stderr" ]]; then
  sed -n '1,120p' "$PASS_ROOT/stderr" >&2
  fail_test "successful fixture wrote stderr"
fi
REPORT="$PASS_ROOT/output/automated-native-deployment-v1.json"
[[ -f "$REPORT" && ! -L "$REPORT" && "$(stat -c %a "$REPORT")" == 600 ]] \
  || fail_test "fixture report publication contract invalid"
jq -e \
  --arg commit "$COMMIT" \
  --arg version "$VERSION" \
  --arg artifact_sha "$ARTIFACT_SHA" \
  --arg manifest_sha "$MANIFEST_SHA" \
  --arg scenario_sha "$SCENARIO_SHA" '
  (keys == ["candidate","completedAt","environment","failureCode","gateVersion","qualificationClass","releaseQualificationEligible","scenarioSet","schemaVersion","startedAt","status","validation"])
  and .schemaVersion == 1
  and .gateVersion == "1"
  and .status == "fixture"
  and .failureCode == null
  and .releaseQualificationEligible == false
  and .qualificationClass == "official_termux_native_automated_v1"
  and .candidate.repository == "CyberBASSLord-666/termux-mcp-edge"
  and .candidate.commit == $commit
  and .candidate.version == $version
  and .candidate.ciRunId == "1001"
  and .candidate.securityRunId == "1002"
  and .candidate.nativeRunId == "1003"
  and .candidate.artifact.sha256 == $artifact_sha
  and .candidate.artifact.manifestSha256 == $manifest_sha
  and .scenarioSet.fileName == "automated-native-deployment-scenarios-v1.json"
  and .scenarioSet.sha256 == $scenario_sha
  and .scenarioSet.scenarioCount == 6
  and ([.validation.scenarioResults[].id] == .scenarioSet.scenarioIds)
  and ([.validation.scenarioResults[].execution] | all(. == "fixture"))
  and .validation.artifactManifestStrict == true
  and .validation.scenarioSetStrict == true
  and .validation.exactArtifact == true
  and .validation.isolatedServiceRoot == true
  and .validation.probeFaultInjectionBounded == true
  and .validation.outputNoClobber == true
  and .validation.workspaceRemoved == true
  and .validation.serviceRemoved == true
  and .validation.runsvdirTerminated == true
  and .validation.nativeArtifactExecuted == false
  and .validation.isolatedFreshDeploy == false
  and .validation.failedUpgradeRecovery == false
  and .validation.supervisedRestart == false
  and .validation.rollbackRecovery == false
  and .validation.uninstall == false
  and .validation.boundedCleanup == false
  and .validation.runitSupervisorObserved == false
  and .validation.realLoopbackProbes == false
  and .validation.physicalCertification == "not_run"
  and .environment.executionMode == "fixture-host-test"
  and .environment.rootfsImage == null
  and .environment.rootfsDigest == null
  and .environment.rootfsImageId == null
  and .environment.runtimeImageDigest == null
  and .environment.androidLinker == {
    observed: false,
    path: null,
    sha256: null,
    bytes: null
  }
  and .environment.runitSupervisorObserved == false
  and .environment.androidFrameworkObserved == false
  and .environment.physicalHardwareObserved == false
  and .environment.physicalDeviceObserved == false
  and .environment.sustainedPhysicalSoak == false
' "$REPORT" >/dev/null
grep -Fq 'TERMUX_MCP_TERMUX_RUNTIME_IMAGE_DIGEST' "$SCRIPT" \
  || fail_test "native gate does not consume the derived runtime image digest"
grep -Fq 'TERMUX_MCP_TERMUX_ROOTFS_IMAGE_ID' "$SCRIPT" \
  || fail_test "native gate does not consume the rootfs image config identity"
grep -Fq 'runtime_image_digest_not_derived' "$SCRIPT" \
  || fail_test "native gate does not require distinct base and runtime image digests"
if grep -Fq "$PASS_ROOT" "$REPORT" \
  || grep -Eq 'automated-native-deployment-gate-token|MCP__AUTH__|safe-root|deploy-home' "$REPORT"; then
  fail_test "fixture report leaked private runtime state"
fi
assert_case_clean "$PASS_ROOT"
[[ -z "$(find "$PASS_ROOT/output" -mindepth 1 -maxdepth 1 \
  -name '.automated-native-deployment.*' -print -quit)" ]] \
  || fail_test "successful gate left report staging"

SUPERVISOR_SUCCESS_ROOT="$(make_case supervisor-success-cleanup)"
write_bundle "$SUPERVISOR_SUCCESS_ROOT/bundle"
SUPERVISOR_SUCCESS_PIDS="$SUPERVISOR_SUCCESS_ROOT/supervisor.pids"
SUPERVISOR_PID_FILES+=("$SUPERVISOR_SUCCESS_PIDS")
if run_gate_with_supervisor \
  "$SUPERVISOR_SUCCESS_ROOT" "$SUPERVISOR_SUCCESS_PIDS" \
  >"$SUPERVISOR_SUCCESS_ROOT/stdout" 2>"$SUPERVISOR_SUCCESS_ROOT/stderr"
then
  supervisor_success_status=0
else
  supervisor_success_status=$?
  sed -n '1,120p' "$SUPERVISOR_SUCCESS_ROOT/stderr" >&2
  fail_test "supervisor success cleanup fixture failed with status $supervisor_success_status"
fi
assert_supervisor_pids_exited "$SUPERVISOR_SUCCESS_PIDS"
jq -e '
  .status == "fixture"
  and .validation.serviceRemoved == true
  and .validation.runsvdirTerminated == true
' "$SUPERVISOR_SUCCESS_ROOT/output/automated-native-deployment-v1.json" >/dev/null \
  || fail_test "supervisor success cleanup claims changed"
assert_case_clean "$SUPERVISOR_SUCCESS_ROOT"

SUPERVISOR_TRAP_ROOT="$(make_case supervisor-trap-cleanup)"
write_bundle "$SUPERVISOR_TRAP_ROOT/bundle"
SUPERVISOR_TRAP_PIDS="$SUPERVISOR_TRAP_ROOT/supervisor.pids"
SUPERVISOR_TRAP_PAUSE="$SUPERVISOR_TRAP_ROOT/pause"
SUPERVISOR_PID_FILES+=("$SUPERVISOR_TRAP_PIDS")
env \
  HOME="$SUPERVISOR_TRAP_ROOT/home" \
  TERMUX_MCP_AUTOMATED_DEPLOYMENT_FIXTURE_MODE=1 \
  TERMUX_MCP_AUTOMATED_DEPLOYMENT_TEST_RUNSVDIR="$FAKE_RUNSVDIR" \
  TERMUX_MCP_AUTOMATED_DEPLOYMENT_TEST_SUPERVISOR_PID_FILE="$SUPERVISOR_TRAP_PIDS" \
  TERMUX_MCP_AUTOMATED_DEPLOYMENT_TEST_PAUSE_AFTER_SUPERVISOR_START="$SUPERVISOR_TRAP_PAUSE" \
  bash "$SCRIPT" \
    --artifact-dir "$SUPERVISOR_TRAP_ROOT/bundle" \
    --expected-commit "$COMMIT" \
    --expected-version "$VERSION" \
    --ci-run-id "$CI_RUN_ID" \
    --security-run-id "$SECURITY_RUN_ID" \
    --native-run-id "$NATIVE_RUN_ID" \
    --output "$SUPERVISOR_TRAP_ROOT/output/automated-native-deployment-v1.json" \
    >"$SUPERVISOR_TRAP_ROOT/stdout" 2>"$SUPERVISOR_TRAP_ROOT/stderr" &
SUPERVISOR_TRAP_GATE_PID=$!
for _ in $(seq 1 1000); do
  [[ -f "$SUPERVISOR_TRAP_PAUSE.ready" && -s "$SUPERVISOR_TRAP_PIDS" ]] && break
  kill -0 "$SUPERVISOR_TRAP_GATE_PID" >/dev/null 2>&1 || break
  sleep 0.02
done
[[ -f "$SUPERVISOR_TRAP_PAUSE.ready" && -s "$SUPERVISOR_TRAP_PIDS" ]] \
  || fail_test "trap cleanup fixture did not reach supervisor boundary"
kill -TERM "$SUPERVISOR_TRAP_GATE_PID" \
  || fail_test "could not interrupt trap cleanup fixture"
if wait "$SUPERVISOR_TRAP_GATE_PID"; then
  fail_test "interrupted trap cleanup fixture unexpectedly succeeded"
fi
assert_supervisor_pids_exited "$SUPERVISOR_TRAP_PIDS"
[[ ! -e "$SUPERVISOR_TRAP_ROOT/output/automated-native-deployment-v1.json" ]] \
  || fail_test "interrupted supervisor fixture published evidence"
assert_case_clean "$SUPERVISOR_TRAP_ROOT"

EXTRA_ROOT="$(make_case manifest-extra-key)"
write_bundle "$EXTRA_ROOT/bundle"
jq '.unexpected = true' "$EXTRA_ROOT/bundle/artifact-manifest.json" \
  >"$EXTRA_ROOT/manifest.next"
mv "$EXTRA_ROOT/manifest.next" "$EXTRA_ROOT/bundle/artifact-manifest.json"
chmod 600 "$EXTRA_ROOT/bundle/artifact-manifest.json"
assert_gate_fails "$EXTRA_ROOT" artifact_manifest_invalid

DUPLICATE_ROOT="$(make_case manifest-duplicate-key)"
write_bundle "$DUPLICATE_ROOT/bundle"
{
  printf '%s\n' '{' '  "schemaVersion": 1,'
  sed -n '2,$p' "$DUPLICATE_ROOT/bundle/artifact-manifest.json"
} >"$DUPLICATE_ROOT/manifest.next"
mv "$DUPLICATE_ROOT/manifest.next" "$DUPLICATE_ROOT/bundle/artifact-manifest.json"
chmod 600 "$DUPLICATE_ROOT/bundle/artifact-manifest.json"
assert_gate_fails "$DUPLICATE_ROOT" artifact_manifest_duplicate_key

NESTED_DUPLICATE_ROOT="$(make_case manifest-disjoint-nested-duplicate-key)"
write_bundle "$NESTED_DUPLICATE_ROOT/bundle"
{
  printf '%s\n' '{' \
    '  "unexpected": {"left": 1},' \
    '  "unexpected": {"right": 2},'
  sed -n '2,$p' "$NESTED_DUPLICATE_ROOT/bundle/artifact-manifest.json"
} >"$NESTED_DUPLICATE_ROOT/manifest.next"
mv "$NESTED_DUPLICATE_ROOT/manifest.next" \
  "$NESTED_DUPLICATE_ROOT/bundle/artifact-manifest.json"
chmod 600 "$NESTED_DUPLICATE_ROOT/bundle/artifact-manifest.json"
assert_gate_fails "$NESTED_DUPLICATE_ROOT" artifact_manifest_duplicate_key

CHECKSUM_ROOT="$(make_case bad-checksum)"
write_bundle "$CHECKSUM_ROOT/bundle"
printf '%s\n' '# changed after checksum publication' >>"$CHECKSUM_ROOT/bundle/termux-mcp-server"
assert_gate_fails "$CHECKSUM_ROOT" artifact_checksum_invalid

LOCK_ROOT="$(make_case active-lock)"
write_bundle "$LOCK_ROOT/bundle"
mkdir -m 700 "$LOCK_ROOT/home/.termux-mcp-automated-deployment-gate.lock"
printf '%s\n' "$$" >"$LOCK_ROOT/home/.termux-mcp-automated-deployment-gate.lock/owner.pid"
if run_gate "$LOCK_ROOT" >"$LOCK_ROOT/stdout" 2>"$LOCK_ROOT/stderr"; then
  fail_test "gate unexpectedly ignored active lock"
fi
grep -Fq 'reason=gate_lock_held' "$LOCK_ROOT/stderr" \
  || fail_test "active lock reason absent"
[[ -d "$LOCK_ROOT/home/.termux-mcp-automated-deployment-gate.lock" ]] \
  || fail_test "gate removed a lock it did not own"
[[ ! -e "$LOCK_ROOT/output/automated-native-deployment-v1.json" ]] \
  || fail_test "locked gate published evidence"
rm -rf "$LOCK_ROOT/home/.termux-mcp-automated-deployment-gate.lock"
assert_case_clean "$LOCK_ROOT"

PREEXISTING_ROOT="$(make_case preexisting-output)"
write_bundle "$PREEXISTING_ROOT/bundle"
printf '%s' sentinel >"$PREEXISTING_ROOT/output/automated-native-deployment-v1.json"
if run_gate "$PREEXISTING_ROOT" >"$PREEXISTING_ROOT/stdout" 2>"$PREEXISTING_ROOT/stderr"; then
  fail_test "gate overwrote a preexisting report"
fi
grep -Fq 'reason=output_already_exists' "$PREEXISTING_ROOT/stderr" \
  || fail_test "preexisting report reason absent"
[[ "$(<"$PREEXISTING_ROOT/output/automated-native-deployment-v1.json")" == sentinel ]] \
  || fail_test "preexisting report changed"
assert_case_clean "$PREEXISTING_ROOT"

SOURCE_RACE_ROOT="$(make_case source-replacement-race)"
write_bundle "$SOURCE_RACE_ROOT/bundle"
SOURCE_RACE_PAUSE="$SOURCE_RACE_ROOT/pause"
env \
  HOME="$SOURCE_RACE_ROOT/home" \
  TERMUX_MCP_AUTOMATED_DEPLOYMENT_FIXTURE_MODE=1 \
  TERMUX_MCP_AUTOMATED_DEPLOYMENT_TEST_PAUSE_AFTER_SNAPSHOT="$SOURCE_RACE_PAUSE" \
  bash "$SCRIPT" \
    --artifact-dir "$SOURCE_RACE_ROOT/bundle" \
    --expected-commit "$COMMIT" \
    --expected-version "$VERSION" \
    --ci-run-id "$CI_RUN_ID" \
    --security-run-id "$SECURITY_RUN_ID" \
    --native-run-id "$NATIVE_RUN_ID" \
    --output "$SOURCE_RACE_ROOT/output/automated-native-deployment-v1.json" \
    >"$SOURCE_RACE_ROOT/stdout" 2>"$SOURCE_RACE_ROOT/stderr" &
SOURCE_RACE_PID=$!
for _ in $(seq 1 200); do
  [[ -f "$SOURCE_RACE_PAUSE.ready" ]] && break
  kill -0 "$SOURCE_RACE_PID" >/dev/null 2>&1 || break
  sleep 0.02
done
[[ -f "$SOURCE_RACE_PAUSE.ready" ]] || fail_test "source race did not reach snapshot boundary"
printf '%s\n' '# source replaced after snapshot' >>"$SOURCE_RACE_ROOT/bundle/termux-mcp-server"
: >"$SOURCE_RACE_PAUSE.continue"
if wait "$SOURCE_RACE_PID"; then
  fail_test "source replacement race unexpectedly succeeded"
fi
grep -Fq 'reason=artifact_source_changed' "$SOURCE_RACE_ROOT/stderr" \
  || fail_test "source replacement race reason absent"
[[ ! -e "$SOURCE_RACE_ROOT/output/automated-native-deployment-v1.json" ]] \
  || fail_test "source replacement race published evidence"
assert_case_clean "$SOURCE_RACE_ROOT"

OUTPUT_RACE_ROOT="$(make_case output-publication-race)"
write_bundle "$OUTPUT_RACE_ROOT/bundle"
OUTPUT_RACE_PAUSE="$OUTPUT_RACE_ROOT/pause"
env \
  HOME="$OUTPUT_RACE_ROOT/home" \
  TERMUX_MCP_AUTOMATED_DEPLOYMENT_FIXTURE_MODE=1 \
  TERMUX_MCP_AUTOMATED_DEPLOYMENT_TEST_PAUSE_AFTER_SNAPSHOT="$OUTPUT_RACE_PAUSE" \
  bash "$SCRIPT" \
    --artifact-dir "$OUTPUT_RACE_ROOT/bundle" \
    --expected-commit "$COMMIT" \
    --expected-version "$VERSION" \
    --ci-run-id "$CI_RUN_ID" \
    --security-run-id "$SECURITY_RUN_ID" \
    --native-run-id "$NATIVE_RUN_ID" \
    --output "$OUTPUT_RACE_ROOT/output/automated-native-deployment-v1.json" \
    >"$OUTPUT_RACE_ROOT/stdout" 2>"$OUTPUT_RACE_ROOT/stderr" &
OUTPUT_RACE_PID=$!
for _ in $(seq 1 200); do
  [[ -f "$OUTPUT_RACE_PAUSE.ready" ]] && break
  kill -0 "$OUTPUT_RACE_PID" >/dev/null 2>&1 || break
  sleep 0.02
done
[[ -f "$OUTPUT_RACE_PAUSE.ready" ]] || fail_test "output race did not reach snapshot boundary"
printf '%s' competing-writer >"$OUTPUT_RACE_ROOT/output/automated-native-deployment-v1.json"
: >"$OUTPUT_RACE_PAUSE.continue"
if wait "$OUTPUT_RACE_PID"; then
  fail_test "output publication race unexpectedly succeeded"
fi
grep -Fq 'reason=report_publication_race' "$OUTPUT_RACE_ROOT/stderr" \
  || fail_test "output publication race reason absent"
[[ "$(<"$OUTPUT_RACE_ROOT/output/automated-native-deployment-v1.json")" == competing-writer ]] \
  || fail_test "output publication race overwrote competitor"
[[ -z "$(find "$OUTPUT_RACE_ROOT/output" -mindepth 1 -maxdepth 1 \
  -name '.automated-native-deployment.*' -print -quit)" ]] \
  || fail_test "output publication race left report staging"
assert_case_clean "$OUTPUT_RACE_ROOT"

PUBLISH_SIGNAL_ROOT="$(make_case post-publication-signal)"
write_bundle "$PUBLISH_SIGNAL_ROOT/bundle"
PUBLISH_SIGNAL_PAUSE="$PUBLISH_SIGNAL_ROOT/publish-pause"
env \
  HOME="$PUBLISH_SIGNAL_ROOT/home" \
  TERMUX_MCP_AUTOMATED_DEPLOYMENT_FIXTURE_MODE=1 \
  TERMUX_MCP_AUTOMATED_DEPLOYMENT_TEST_PAUSE_AFTER_PUBLISH="$PUBLISH_SIGNAL_PAUSE" \
  bash "$SCRIPT" \
    --artifact-dir "$PUBLISH_SIGNAL_ROOT/bundle" \
    --expected-commit "$COMMIT" \
    --expected-version "$VERSION" \
    --ci-run-id "$CI_RUN_ID" \
    --security-run-id "$SECURITY_RUN_ID" \
    --native-run-id "$NATIVE_RUN_ID" \
    --output "$PUBLISH_SIGNAL_ROOT/output/automated-native-deployment-v1.json" \
    >"$PUBLISH_SIGNAL_ROOT/stdout" 2>"$PUBLISH_SIGNAL_ROOT/stderr" &
PUBLISH_SIGNAL_PID=$!
for _ in $(seq 1 200); do
  [[ -f "$PUBLISH_SIGNAL_PAUSE.ready" ]] && break
  kill -0 "$PUBLISH_SIGNAL_PID" >/dev/null 2>&1 || break
  sleep 0.02
done
[[ -f "$PUBLISH_SIGNAL_PAUSE.ready" ]] \
  || fail_test "post-publication signal fixture did not reach linked-output boundary"
[[ -f "$PUBLISH_SIGNAL_ROOT/output/automated-native-deployment-v1.json" ]] \
  || fail_test "post-publication signal fixture did not create its owned output"
PUBLISH_SIGNAL_SHA="$(sha "$PUBLISH_SIGNAL_ROOT/output/automated-native-deployment-v1.json")"
kill -TERM "$PUBLISH_SIGNAL_PID" \
  || fail_test "could not interrupt post-publication signal fixture"
if wait "$PUBLISH_SIGNAL_PID"; then
  fail_test "post-publication signal fixture unexpectedly succeeded"
fi
[[ -f "$PUBLISH_SIGNAL_ROOT/output/automated-native-deployment-v1.json" ]] \
  || fail_test "post-publication signal removed an already committed output"
[[ "$(sha "$PUBLISH_SIGNAL_ROOT/output/automated-native-deployment-v1.json")" == "$PUBLISH_SIGNAL_SHA" ]] \
  || fail_test "post-publication signal changed an already committed output"
[[ -z "$(find "$PUBLISH_SIGNAL_ROOT/output" -mindepth 1 -maxdepth 1 \
  -name '.automated-native-deployment.*' -print -quit)" ]] \
  || fail_test "post-publication signal retained its staging link"
assert_case_clean "$PUBLISH_SIGNAL_ROOT"

WRONG_NAME_ROOT="$(make_case wrong-output-name)"
write_bundle "$WRONG_NAME_ROOT/bundle"
if env \
  HOME="$WRONG_NAME_ROOT/home" \
  TERMUX_MCP_AUTOMATED_DEPLOYMENT_FIXTURE_MODE=1 \
  bash "$SCRIPT" \
    --artifact-dir "$WRONG_NAME_ROOT/bundle" \
    --expected-commit "$COMMIT" \
    --expected-version "$VERSION" \
    --ci-run-id "$CI_RUN_ID" \
    --security-run-id "$SECURITY_RUN_ID" \
    --native-run-id "$NATIVE_RUN_ID" \
    --output "$WRONG_NAME_ROOT/output/wrong.json" \
    >"$WRONG_NAME_ROOT/stdout" 2>"$WRONG_NAME_ROOT/stderr"; then
  fail_test "wrong output filename unexpectedly succeeded"
fi
grep -Fq 'reason=output_filename_invalid' "$WRONG_NAME_ROOT/stderr" \
  || fail_test "wrong output filename reason absent"
[[ ! -e "$WRONG_NAME_ROOT/output/wrong.json" ]] \
  || fail_test "wrong output filename was published"
assert_case_clean "$WRONG_NAME_ROOT"

if grep -Fq 'first_readiness_probe' "$SCRIPT" \
  || grep -Fq 'first_readiness_probe' "$SCHEMA" \
  || grep -Fq 'first_readiness_probe' "$SCENARIOS"; then
  fail_test "stale first-probe fault literal remains"
fi
grep -Fq 'python3 "$COMMIT_HELPER"' "$SCRIPT" \
  || fail_test "deployment evidence does not use the held-FD commit helper"
rollback_state_pattern='PUBLISH_(LINKED|IDENTITY)|OUTPUT_PUBLISHED'
matcher_fixture="$ROOT/unsafe-publication-state.fixture"
printf 'OUTPUT_PUBLISHED=1\n' >"$matcher_fixture"
grep -Eq -- "$rollback_state_pattern" "$matcher_fixture" \
  || fail_test "unsafe public-output matcher did not execute"
if grep -Eq -- "$rollback_state_pattern" "$SCRIPT"; then
  fail_test "deployment evidence retains unsafe public-output rollback state"
fi

printf 'automated native deployment gate tests passed\n'
